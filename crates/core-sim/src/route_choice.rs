//! Route choice: which of its pair's routes each trip takes.
//!
//! The route sets say which alternatives exist for each origin-destination
//! pair; a [`ChoiceModel`] says which one each traveller takes. This module is
//! the join: it turns every trip's set into a situation (a
//! [`ChoiceBatch`]) whose alternatives
//! carry the route attributes below, asks the model in batches, and keeps the
//! answers.
//!
//! **The attributes** of a route, all per route and all read by name:
//!
//! | name | meaning |
//! |---|---|
//! | `time_min` | expected travel time in minutes at the trip's departure, at the last loading's link times (at free flow for the first choice; turns and signal delay included) |
//! | `length_km` | length in kilometres |
//! | `detour` | `time / best time − 1` within the set: 0 for the best route |
//! | `overlap` | the largest share of its cost shared with a route found before it (0 to 1) |
//! | `ln_path_size` | the natural log of the route's path size (Ben-Akiva and Bierlaire, by length) **over the routes offered** in this situation, not over the whole stored set (the itineraries' is by time share, the project's extension): 0 for a route that shares nothing, negative as it shares more |
//! | `n_links` | how many links |
//! | `car_min` | the same as `time_min`: a route is driven all the way |
//! | `bike_min`, `walk_min`, `wait_min`, `ride_min`, `transfers`, `parking_min` | 0: a route has none |
//! | `bike_mixed_km`, `bike_lane_km`, `bike_separated_km` | a bike route's kilometres in mixed traffic, on a painted lane, on a track of its own (a ferry crossing in none); 0 for a car route |
//! | `nest`, `mode_car` and the other `mode_*` | the car's nest (its mode index) and 1 for `mode_car`, 0 for the others |
//! | `cost_eur`, `cost_running_eur`, `cost_toll_eur`, `cost_parking_eur` | money ([`crate::prices`]): the route's kilometres at the car's price per km, its tolls, and the parking fee at the trip's destination if given; `cost_fare_eur` is 0 |
//! | `trip_departure_h`, `person_<name>`, `trip_<name>` | the trip's departure in hours, and the user's traveller and trip values ([`crate::demand_values`]) |
//!
//! The last seven are the itineraries' vocabulary (M4, A8;
//! [`crate::itinerary_choice`]): routes carry them too, so one set of
//! coefficients describes every choice in a run, and a coefficient on walking
//! or waiting changes nothing for a car route.
//!
//! **Alternative identity** is a hash of the route's links, so it does not
//! change when other routes are added to or removed from the set, and the
//! draws of every route that stays are exactly as they were.

use std::sync::Arc;

use openmobisim_core_choice::{ChoiceBatch, ChoiceError, ChoiceModel};
use openmobisim_core_demand::{Mode, Travellers, Trips};
use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_routes::{RouteKey, RouteSets};
use openmobisim_core_types::hash::Fnv1a;
use openmobisim_core_types::ids::{EntityId, LinkId, TravellerId, TripId};

use crate::demand_values::DemandValues;
use crate::equilibration::Equilibration;
use crate::gap::{Currency, Ended, GapTrips, RouteGapSums};
use crate::generalised_cost::RouteCost;
use crate::link_times::LinkTimes;
use crate::link_values::{PreparedLinkValues, ValueLayer};
use crate::prices::PreparedPrices;
use crate::trees::SearchTrees;
use openmobisim_core_graph::hubs::ParkingKind;
use openmobisim_core_types::rng::{DrawAddress, StreamRng};

/// Sampled trips per search scratch in the whole-network gap: a scratch holds a few bytes
/// per link of the network, so a thread makes one per chunk it takes. Results do not depend on it.
pub(crate) const GAP_CHUNK: usize = 16;

/// The attributes a route carries, in the order a batch holds them: the same
/// names as every itinerary's ([`crate::itinerary_choice::ATTRIBUTES`], A8).
pub const ROUTE_ATTRIBUTES: [&str; 40] = crate::itinerary_choice::ATTRIBUTES;

/// "No route": a trip with no set, or whose origin and destination are one node.
pub const NO_ROUTE: u32 = u32::MAX;

/// How many trips are asked about at once. Bounds the batch's memory and is the
/// granularity of a Python model's call; results do not depend on it.
const CHUNK: usize = 32_768;

/// Trips per piece of work when their routes are costed in parallel ([`Chooser::fill`]):
/// results do not depend on it.
const COSTING_CHUNK: usize = 256;

/// Each trip's route choice, indexed by trip.
#[derive(Clone, Debug, Default)]
pub struct RouteChoices {
    /// The route taken, as an index into the run's route sets, or [`NO_ROUTE`].
    pub route: Vec<u32>,
    /// How many routes the trip could choose from (0 for none).
    pub alternatives: Vec<u32>,
    /// The probability the model gave the route taken; `NaN` if it gave none.
    pub probability: Vec<f64>,
    /// The traveller's weight: how many people the trip stands for.
    pub weight: Vec<u32>,
}

/// Equal when every entry is, comparing probabilities by their bits so that a
/// model with no probabilities (`NaN`) equals itself.
impl PartialEq for RouteChoices {
    fn eq(&self, other: &Self) -> bool {
        self.route == other.route
            && self.alternatives == other.alternatives
            && self.weight == other.weight
            && self.probability.len() == other.probability.len()
            && self
                .probability
                .iter()
                .zip(&other.probability)
                .all(|(a, b)| a.to_bits() == b.to_bits())
    }
}

impl RouteChoices {
    /// Renumber after the sets grew: `shift` is what [`RouteSets::extended`] returned
    /// for the store `old`, `grown` is the new store and `trip_keys` each trip's pair. A trip
    /// keeps the route it had (now a different number where routes were added before it), and
    /// its number of alternatives is the size of its pair's grown set. Probabilities are what
    /// the model gave the route when it was taken, and stay so.
    pub(crate) fn grown(
        &mut self,
        old: &RouteSets,
        shift: &[u32],
        grown: &RouteSets,
        trip_keys: &[RouteKey],
    ) {
        for t in 0..self.route.len() {
            if self.route[t] == NO_ROUTE {
                continue;
            }
            self.route[t] += shift[old.key_of_route(self.route[t] as usize)];
            let key = grown.key_index(trip_keys[t]).expect("a routed trip has a set");
            self.alternatives[t] =
                u32::try_from(grown.route_range(key).len()).expect("few alternatives");
        }
    }

    /// After a set made room ([`RouteSets::revised`]): each trip's route renumbered by
    /// `map` (a removed route had no trip; were one on it, it would take its set's last route).
    pub(crate) fn revised(&mut self, map: &[u32], grown: &RouteSets, trip_keys: &[RouteKey]) {
        for t in 0..self.route.len() {
            if self.route[t] == NO_ROUTE {
                continue;
            }
            let key = grown.key_index(trip_keys[t]).expect("a routed trip has a set");
            let range = grown.route_range(key);
            let new = map[self.route[t] as usize];
            self.route[t] = if new == u32::MAX {
                u32::try_from(range.end - 1).expect("routes fit u32")
            } else {
                new
            };
            self.alternatives[t] = u32::try_from(range.len()).expect("few alternatives");
        }
    }

    /// How many trips.
    #[must_use]
    pub fn len(&self) -> usize {
        self.route.len()
    }

    /// Whether there are no trips.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.route.is_empty()
    }
}

/// Everything a route choice reads.
pub(crate) struct Inputs<'a> {
    pub network: &'a RoadNetwork,
    pub travellers: &'a Travellers,
    pub trips: &'a Trips,
    pub trip_keys: &'a [RouteKey],
    pub model: &'a dyn ChoiceModel,
    /// An alternative is offered only if its expected time is within this share of the best's
    /// (0: all of them).
    pub detour_limit: f64,
    /// At most this many of them, the fastest (0: no limit).
    pub max_routes: u32,
    /// The user's link values: a route's `road_<name>_km` and `road_<name>_sum`.
    pub link_values: &'a PreparedLinkValues,
    /// The attributes' names of the user's link, traveller and trip values.
    pub link_value_names: &'a [String],
    /// The run's prices: a route's `cost_eur` and its parts.
    pub prices: &'a PreparedPrices,
    /// The user's traveller and trip values.
    pub demand_values: &'a DemandValues,
    /// The whole seconds after a car trip's departure its car enters the roads (S295):
    /// `car_access_s` in a run whose trips pay the fixed times, else 0. Every reading of a car
    /// route's time-dependent cost starts there ([`car_departure`]), as the loading does.
    pub car_start_s: u32,
    /// The mode whose routes these are (S298): the car's by default (the road layer's link
    /// values, the car's running cost, `car_min`), or the bike's (the bike layer's values,
    /// `bike_min`, `bike_mixed_km`, no toll or parking fee).
    pub mode: Mode,
    /// The bike layer, when `mode` is the bike's: its infrastructure per link, for
    /// `bike_mixed_km`.
    pub bike_layer: Option<&'a openmobisim_core_graph::layers::StaticNetwork>,
    /// The run's route cost for these routes (S299): what the offered set and the gaps compare
    /// routes in, per coefficient vector, a trip's vector its class's. `None` (or a cost of time
    /// alone): travel time.
    pub cost: Option<&'a RouteCost>,
}

/// When trip `trip`'s car enters the roads: its departure plus `car_start_s`
/// ([`Inputs::car_start_s`]), in seconds.
fn car_departure(trips: &Trips, trip: TripId, car_start_s: u32) -> f64 {
    f64::from(trips.departure(trip).get() + car_start_s)
}

/// The identity of a route: a 32-bit hash of its links.
fn route_identity(links: &[u32]) -> u32 {
    let mut h = Fnv1a::new();
    for &l in links {
        h.write_u32(l);
    }
    h.finish32()
}

/// The first bit of every random key that marks a **floor** draw (below): a
/// fresh sample from the model's probabilities, independent of every draw the
/// run itself uses.
const FLOOR_KEY: u32 = 0x8000_0000;

/// What one update of the assignment found.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Assessment {
    pub reselected_share: f64,
    pub changed_share: f64,
    /// The relative gap to the least-cost route of the choice set, over the trips that
    /// finished, the gap the model itself expects, and the difference.
    pub gap: f64,
    pub gap_expected: f64,
    pub gap_excess: f64,
    /// The share of travellers (by weight) left out because their trip had not finished.
    pub incomplete_share: f64,
    pub gap_flow: f64,
    pub gap_flow_floor: f64,
    /// The same in-set gap's deterministic part in **generalised minutes** (S299): `(Σ w g_taken
    /// − Σ w g_expected) / Σ w g_least` over the finished trips whose route taken is offered, `g`
    /// an alternative's utility over its class's time coefficient. Measured only when asked
    /// ([`Chooser::update`]'s `currency`); `NaN` otherwise.
    pub gap_generalised_excess: f64,
    /// The sums `gap` to `gap_excess` and `gap_generalised_excess` are made of, for the run to
    /// add the car routes of the trips choosing their mode to them (S302).
    pub sums: RouteGapSums,
}

/// A trip that chose again, and what it chose.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Change {
    pub trip: usize,
    pub route: u32,
    pub probability: f64,
}

/// What [`Chooser::update`] found and would change.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Update {
    pub assessment: Assessment,
    pub changes: Vec<Change>,
    /// Each trip's expected travel time under the model at these times, in seconds (`NaN`
    /// for a trip with no set, or if the model gives no probabilities): what
    /// [`Chooser::network_gap`] needs to tell the model's dispersion from disequilibrium.
    pub expected_seconds: Vec<f64>,
}

/// What [`Chooser::fill`] leaves next to the batch: per situation (one trip's choice among its
/// pair's routes) and per alternative offered.
#[derive(Default)]
struct Filled {
    /// Per situation: its trip.
    trip_of: Vec<usize>,
    /// Per situation: the store index of its set's route 0, and how many routes the set has.
    set_first: Vec<usize>,
    set_len: Vec<usize>,
    /// Per situation: where its set's expected seconds start in `set_seconds`.
    set_at: Vec<usize>,
    /// The expected seconds of every route of every situation's set, offered or not, in order.
    set_seconds: Vec<f64>,
    /// Per alternative offered, as the batch holds them: its route's store index, and its expected seconds.
    alt_route: Vec<u32>,
    alt_seconds: Vec<f64>,
    /// A scratch per link for the path sizes, all zeros between situations
    /// ([`openmobisim_core_routes::path_sizes_counted`]).
    link_users: Vec<u32>,
}

impl Filled {
    fn clear(&mut self) {
        self.trip_of.clear();
        self.set_first.clear();
        self.set_len.clear();
        self.set_at.clear();
        self.set_seconds.clear();
        self.alt_route.clear();
        self.alt_seconds.clear();
    }
}

/// What a route's attributes are made from ([`Chooser::route_row`]).
struct RouteRow<'r> {
    links: &'r [u32],
    /// Its length in metres (0 if the model reads neither a length nor money).
    length_m: f64,
    overlap: f64,
    /// Its expected seconds, and the least of its set's.
    seconds: f64,
    best: f64,
    /// The natural log of its path size over the routes offered (read only if the model reads
    /// it).
    ln_path_size: f64,
}

/// The parts of route choice that do not change from one iteration to the next:
/// which attributes to fill and the routes' identities (their lengths are the
/// store's). **It belongs to one state of the route sets**: when they grow the
/// run makes a new one, which costs a pass over the store (~50 ms at 53 000 routes) and is
/// done only then.
pub(crate) struct Chooser<'a> {
    inputs: &'a Inputs<'a>,
    route_sets: Arc<RouteSets>,
    wanted: Vec<String>,
    /// Whether the model reads a length or money, so that routes carry their lengths.
    wants_length: bool,
    /// Whether the model reads `ln_path_size`, computed per situation over the offered routes.
    wants_path_size: bool,
    identity: Vec<u32>,
    weight: Vec<u32>,
    /// Each route's extra seconds under each coefficient vector of the route cost
    /// (`extra[vector][route]`, S299); empty where the cost is time alone.
    extra: Vec<Vec<f64>>,
}

impl<'a> Chooser<'a> {
    /// The route cost, where it is not time alone.
    fn route_cost(&self) -> Option<&'a RouteCost> {
        self.inputs.cost.filter(|c| !c.is_time_only())
    }

    /// Trip `trip`'s coefficient vector under the route cost: 0 where it is time alone.
    fn vector_of(&self, trip: TripId) -> u32 {
        self.route_cost().map_or(0, |c| {
            let traveller = self.inputs.trips.traveller(trip);
            c.vector_of_class(self.inputs.travellers.user_class(traveller).index())
        })
    }

    /// The attributes filled, as a batch names them.
    pub(crate) fn wanted_names(&self) -> Vec<&str> {
        self.wanted.iter().map(String::as_str).collect()
    }

    /// Prepare to choose.
    ///
    /// # Errors
    ///
    /// [`ChoiceError::MissingAttribute`] if the model reads an attribute routes
    /// do not carry.
    pub(crate) fn new(
        inputs: &'a Inputs<'a>,
        route_sets: Arc<RouteSets>,
    ) -> Result<Self, ChoiceError> {
        let (trips, travellers) = (inputs.trips, inputs.travellers);
        // The attributes to fill: what the model reads, or all of them.
        let wanted = crate::itinerary_choice::wanted(
            inputs.model,
            &ROUTE_ATTRIBUTES,
            inputs.link_value_names,
        )?;
        let needs = |name: &str| wanted.iter().any(|w| w == name);
        // Lengths for `length_km` and a route's running cost (S248); the path size is taken
        // per situation over the routes offered (S279, CM-3), not from the store.
        let wants_path_size = needs("ln_path_size");
        let wants_length = needs("length_km") || needs("cost_eur") || needs("cost_running_eur");
        let mut identity: Vec<u32> = (0..route_sets.route_count())
            .map(|r| route_identity(route_sets.route(r).links))
            .collect();
        // Within a set, identities must differ: nudge a (vanishingly rare) clash.
        for key in 0..route_sets.keys().len() {
            let range = route_sets.route_range(key);
            for r in range.clone() {
                while identity[range.start..r].contains(&identity[r]) {
                    identity[r] = identity[r].wrapping_add(1);
                }
            }
        }
        let weight = (0..trips.len())
            .map(|i| travellers.weight(trips.traveller(TripId::from_index(i as usize))))
            .collect();
        // The routes' extras per vector, for the offered set and the gaps (S299): one pass over
        // the store per vector, made only where the cost is not time alone.
        let extra = match inputs.cost.filter(|c| !c.is_time_only()) {
            None => Vec::new(),
            #[allow(clippy::cast_possible_truncation, reason = "few vectors")]
            Some(cost) => (0..cost.vector_count())
                .map(|v| {
                    (0..route_sets.route_count())
                        .map(|r| cost.route_extra(v as u32, route_sets.route(r).links))
                        .collect()
                })
                .collect(),
        };
        Ok(Self {
            inputs,
            route_sets,
            wanted,
            wants_length,
            wants_path_size,
            identity,
            weight,
            extra,
        })
    }

    /// Route `r`'s length in metres, if the model reads a length or money; 0 otherwise.
    fn length_m(&self, r: usize) -> f64 {
        if self.wants_length { self.route_sets.lengths_m()[r] } else { 0.0 }
    }

    /// Fill `row` with the attributes of `route` for `traveller`'s `trip`, in the order the
    /// batch holds them ([`Chooser::wanted`]).
    fn route_row(
        &self,
        row: &mut [f64],
        route: &RouteRow<'_>,
        trip: TripId,
        traveller: TravellerId,
    ) {
        let trips = self.inputs.trips;
        for (slot, name) in row.iter_mut().zip(&self.wanted) {
            if let Some(source) = self.inputs.demand_values.lookup(name) {
                *slot = self.inputs.demand_values.value(source, trips, traveller, trip);
                continue;
            }
            if let Some((column, aggregate)) = self.inputs.link_values.lookup(name) {
                let layer = match self.inputs.mode {
                    Mode::Bike => ValueLayer::Bike,
                    Mode::Walk => ValueLayer::Walk,
                    _ => ValueLayer::Road,
                };
                *slot = if self.inputs.link_values.layer(column) == layer {
                    self.inputs.link_values.total(
                        column,
                        aggregate,
                        route.links.iter().map(|&l| l as usize),
                    )
                } else {
                    0.0
                };
                continue;
            }
            *slot = match name.as_str() {
                "time_min" => route.seconds / 60.0,
                "cost_eur" | "cost_running_eur" | "cost_toll_eur" | "cost_parking_eur" => {
                    let prices = self.inputs.prices;
                    let car = self.inputs.mode == Mode::Car;
                    let kind = if self.inputs.mode == Mode::Bike {
                        ParkingKind::Bike
                    } else {
                        ParkingKind::Car
                    };
                    let running = prices.running(Some(kind), route.length_m);
                    // Tolls and the parking fee at the destination (S249) are the car's.
                    let toll = || {
                        if car {
                            prices.toll_raw(self.inputs.link_values, route.links)
                        } else {
                            0.0
                        }
                    };
                    let parking = || {
                        if car { self.inputs.demand_values.destination_parking(trip) } else { 0.0 }
                    };
                    match name.as_str() {
                        "cost_running_eur" => running,
                        "cost_toll_eur" => toll(),
                        "cost_parking_eur" => parking(),
                        _ => running + toll() + parking(),
                    }
                }
                "length_km" => route.length_m / 1000.0,
                "detour" => (route.seconds / route.best - 1.0).max(0.0),
                "overlap" => route.overlap,
                "ln_path_size" => route.ln_path_size,
                "n_links" => f64::from(u32::try_from(route.links.len()).unwrap_or(u32::MAX)),
                "car_min" if self.inputs.mode == Mode::Car => route.seconds / 60.0,
                "bike_min" if self.inputs.mode == Mode::Bike => route.seconds / 60.0,
                "bike_mixed_km" | "bike_lane_km" | "bike_separated_km" => {
                    match (self.inputs.mode, self.inputs.bike_layer) {
                        (Mode::Bike, Some(layer)) => {
                            use openmobisim_core_graph::layers::BikeInfrastructure as Kind;
                            let graph = layer.network();
                            // A ferry crossing is ridden on no facility: it counts in none.
                            let counts = |l: LinkId| match name.as_str() {
                                "bike_mixed_km" => layer.is_mixed_traffic(l),
                                kind => {
                                    let wanted = if kind == "bike_lane_km" {
                                        Kind::Lane
                                    } else {
                                        Kind::Separated
                                    };
                                    layer.infrastructure(l) == wanted
                                        && graph.link_class(l)
                                            != openmobisim_core_graph::defaults::RoadClass::Ferry
                                }
                            };
                            route
                                .links
                                .iter()
                                .map(|&l| LinkId::new(l))
                                .filter(|&l| counts(l))
                                .map(|l| graph.link_length(l).get())
                                .sum::<f64>()
                                / 1000.0
                        }
                        _ => 0.0,
                    }
                }
                other => {
                    crate::itinerary_choice::mode_attribute(other, self.inputs.mode).unwrap_or(0.0)
                }
            };
        }
    }

    /// Put trips `from..to` that have a set into `batch`, with `times` as their cost (free flow
    /// if `None`), only those of the travellers `who` marks if it is given. Each situation offers
    /// the routes of its pair's set **whose expected cost is within [`Inputs::detour_limit`] of
    /// the least's** (all of them if the limit is 0: the least-cost route is always offered),
    /// **at most [`Inputs::max_routes`] of them, the cheapest**: the cost a route's expected time
    /// plus its extras under the trip's coefficient vector of the route cost ([`Inputs::cost`],
    /// S299), its expected time where the cost is time alone. What is left besides the batch is
    /// in `filled`.
    fn fill(
        &self,
        batch: &mut ChoiceBatch,
        filled: &mut Filled,
        range: core::ops::Range<usize>,
        times: Option<&LinkTimes>,
        who: Option<&[bool]>,
    ) {
        let Inputs { trips, trip_keys, detour_limit, max_routes, .. } = *self.inputs;
        let route_sets = &*self.route_sets;
        batch.clear();
        filled.clear();
        let mut row = vec![0.0; self.wanted.len()];
        // The trips asked, each with its pair's set of routes.
        let asked: Vec<(usize, core::ops::Range<usize>)> = range
            .filter_map(|i| {
                let key = trip_keys[i];
                if key.origin == key.destination {
                    return None;
                }
                let trip = TripId::from_index(i);
                if who.is_some_and(|w| !w[trips.traveller(trip).index()]) {
                    return None;
                }
                let set = route_sets.route_range(route_sets.key_index(key)?);
                (!set.is_empty()).then_some((i, set))
            })
            .collect();
        // Each route's expected time: at free flow the store's, later the link times of the
        // last loading walked from the trip's departure. Each trip's alone, so they are worked
        // out in parallel and the batch is the same for any number of threads (S294).
        let car_start_s = self.inputs.car_start_s;
        let costed: Vec<Vec<f64>> = match times {
            Some(t) => crate::transit::par_map(
                &asked,
                COSTING_CHUNK,
                || (),
                |(), (i, set)| {
                    let departure = car_departure(trips, TripId::from_index(*i), car_start_s);
                    set.clone()
                        .map(|r| t.route_seconds(route_sets.route(r).links, departure))
                        .collect()
                },
            ),
            None => asked
                .iter()
                .map(|(_, set)| set.clone().map(|r| f64::from(route_sets.route(r).cost)).collect())
                .collect(),
        };
        let mut generalised: Vec<f64> = Vec::new();
        for ((i, set), set_seconds) in asked.into_iter().zip(costed) {
            let trip = TripId::from_index(i);
            let traveller = trips.traveller(trip);
            let class = self.inputs.travellers.user_class(traveller).raw();
            batch.begin_situation_in(traveller.raw(), trip.raw(), class);
            let at = filled.set_seconds.len();
            filled.set_seconds.extend(set_seconds);
            let seconds = &filled.set_seconds[at..];
            // What the limit and the cap compare (S299): each route's expected time plus its
            // extras under the trip's coefficient vector; its time where the cost is time alone.
            let judged: &[f64] = if self.extra.is_empty() {
                seconds
            } else {
                let extra = &self.extra[self.vector_of(trip) as usize];
                generalised.clear();
                generalised.extend(set.clone().zip(seconds).map(|(r, s)| s + extra[r]));
                &generalised
            };
            let best = seconds.iter().copied().fold(f64::INFINITY, f64::min);
            let least = judged.iter().copied().fold(f64::INFINITY, f64::min);
            let reach =
                if detour_limit > 0.0 { least * (1.0 + detour_limit) } else { f64::INFINITY };
            // At most `max_routes` offered, the cheapest, the earlier first among equals (S268).
            let cap = max_routes as usize;
            let offered: Option<Vec<bool>> = (cap > 0 && judged.len() > cap).then(|| {
                let mut order: Vec<usize> = (0..judged.len()).collect();
                order.sort_by(|&x, &y| judged[x].total_cmp(&judged[y]).then(x.cmp(&y)));
                let mut keep = vec![false; judged.len()];
                for &a in &order[..cap] {
                    keep[a] = true;
                }
                keep
            });
            let is_offered = |a: usize| judged[a] <= reach && offered.as_ref().is_none_or(|k| k[a]);
            // The path size over the routes offered, by length (S279, CM-3), not over the
            // whole stored set: overlap with a route nobody is offered is no overlap.
            let path_size = if self.wants_path_size {
                let links: Vec<&[u32]> = set
                    .clone()
                    .enumerate()
                    .filter(|&(a, _)| is_offered(a))
                    .map(|(_, r)| route_sets.route(r).links)
                    .collect();
                let network = self.inputs.network;
                openmobisim_core_routes::path_sizes_counted(
                    &links,
                    |l| network.link_length(LinkId::new(l)).get(),
                    &mut filled.link_users,
                )
            } else {
                Vec::new()
            };
            let mut offered_so_far = 0;
            for (a, r) in set.clone().enumerate() {
                if !is_offered(a) {
                    continue;
                }
                let view = route_sets.route(r);
                let route = RouteRow {
                    links: view.links,
                    length_m: self.length_m(r),
                    overlap: f64::from(view.overlap),
                    seconds: seconds[a],
                    best,
                    // Read only if the model reads the path size, which is then computed.
                    ln_path_size: if self.wants_path_size {
                        path_size[offered_so_far].ln()
                    } else {
                        0.0
                    },
                };
                self.route_row(&mut row, &route, trip, traveller);
                batch.push_alternative(self.identity[r], &row);
                filled.alt_route.push(u32::try_from(r).expect("route ids fit in 32 bits"));
                filled.alt_seconds.push(seconds[a]);
                offered_so_far += 1;
            }
            filled.set_first.push(set.start);
            filled.set_len.push(set.len());
            filled.set_at.push(at);
            filled.trip_of.push(i);
        }
    }

    /// The gap against the **whole network**, over the routed trips that **finished**
    /// (`unfinished[t]` false): `Σ w t_chosen / Σ w t_fastest − 1`, where `t_fastest` is the
    /// least of the set's best and the route the run's shared search `trees` found for the
    /// trip's origin and departure bin ([`crate::trees`]), re-costed at the trip's own departure
    /// on `times`. Returns it with the **disequilibrium** version: the same less what the
    /// model itself expects inside the set, `Σ w (t_chosen − t_expected + t_least_in_set −
    /// t_fastest) / Σ w t_fastest`, where `expected_seconds[t]` is trip `t`'s expected time
    /// under the model (`NaN`: the model gave no probabilities; the second number is then `NaN`).
    ///
    /// Every finished trip counts (the shared trees cost no search; S294), summed in trip order,
    /// so the result is the same for any thread count. `NaN` if nothing is routed.
    pub(crate) fn network_gap(
        &self,
        current: &RouteChoices,
        times: &LinkTimes,
        trees: &SearchTrees,
        unfinished: &[bool],
        expected_seconds: &[f64],
    ) -> (f64, f64) {
        let Inputs { trips, trip_keys, car_start_s, .. } = *self.inputs;
        let route_sets = &*self.route_sets;
        let chosen: Vec<usize> = (0..current.route.len())
            .filter(|&t| current.route[t] != NO_ROUTE && !unfinished[t])
            .collect();
        if chosen.is_empty() {
            return (f64::NAN, f64::NAN);
        }
        let per_trip: Vec<(f64, f64, f64, f64)> = crate::transit::par_map(
            &chosen,
            GAP_CHUNK,
            || (),
            |(), &t: &usize| {
                let key = trip_keys[t];
                let departure = car_departure(trips, TripId::from_index(t), car_start_s);
                let k = route_sets.key_index(key).expect("a routed trip has a set");
                let mut in_set = f64::INFINITY;
                let mut taken = 0.0;
                for r in route_sets.route_range(k) {
                    let time = times.route_seconds(route_sets.route(r).links, departure);
                    in_set = in_set.min(time);
                    if r == current.route[t] as usize {
                        taken = time;
                    }
                }
                let vector = self.vector_of(TripId::from_index(t));
                let found = trees
                    .route_of(key.origin, vector, key.destination, departure)
                    .map(|found| times.route_seconds(&found.links, departure));
                (
                    f64::from(current.weight[t]),
                    taken,
                    in_set,
                    found.map_or(in_set, |f| f.min(in_set)),
                )
            },
        );
        let (mut paid, mut fastest, mut excess) = (0.0, 0.0, 0.0);
        for (&t, &(w, taken, in_set, best)) in chosen.iter().zip(&per_trip) {
            paid += w * taken;
            fastest += w * best;
            excess += w * (taken - expected_seconds[t] + in_set - best);
        }
        if fastest > 0.0 {
            ((paid - fastest) / fastest, excess / fastest)
        } else {
            (f64::NAN, f64::NAN)
        }
    }

    /// **The full gap's car routes** (S293, [`crate::gap`]): for every trip that drives a car
    /// route of the sets, at this loading's link `times`, the generalised minutes of the route
    /// `current` gives it, of the best of the routes it is offered (the batch [`Self::fill`]
    /// builds), what the model expects of it among them, and of the best over the whole network
    /// (the route the run's shared search `trees` found for the trip's origin and departure bin,
    /// [`crate::trees`], of the trip's coefficient vector, re-costed at the trip's own departure
    /// and costed as any route where it is no dearer than the set's least in the vector's
    /// generalised cost, its time where the cost is time alone), into `out`. A trip still under way at the end is costed
    /// as what it paid and still had to pay ([`Ended::car_seconds`]); a route taken but no longer
    /// offered, at its time on these times. Every trip, every call; nothing is drawn, nothing
    /// changed.
    ///
    /// Costs the batches built once more and a route costing per trip; no search (S294).
    ///
    /// # Errors
    ///
    /// [`ChoiceError`] as for [`Self::choose_all`].
    #[allow(
        clippy::too_many_arguments,
        reason = "the loading's times and trees, the ended trips, the currency and the output"
    )]
    pub(crate) fn gap(
        &self,
        current: &RouteChoices,
        times: &LinkTimes,
        trees: &SearchTrees,
        iteration: u32,
        ended: &Ended<'_>,
        currency: &Currency,
        out: &mut GapTrips,
    ) -> Result<(), ChoiceError> {
        let Inputs { network, trips, trip_keys, model, .. } = *self.inputs;
        let sets = &*self.route_sets;
        let total = trips.len() as usize;
        let names = self.wanted_names();
        let mut batch = ChoiceBatch::new(iteration, &names);
        let mut filled = Filled::default();
        // One situation per route costed on its own: a route taken and no longer offered, a trip
        // still under way, a route the network search found.
        let mut single = ChoiceBatch::new(iteration, &names);
        let mut row = vec![0.0; self.wanted.len()];
        // Each trip measured, the least expected seconds of its set, and the least in the trip's
        // vector's generalised cost (its seconds where the cost is time alone, S299).
        let mut measured: Vec<(usize, f64, f64)> = Vec::new();
        let mut i = 0;
        while i < total {
            let end = (i + CHUNK).min(total);
            self.fill(&mut batch, &mut filled, i..end, Some(times), None);
            i = end;
            if filled.trip_of.is_empty() {
                continue;
            }
            batch.validate()?;
            let probabilities = model.probabilities(&batch)?;
            let g = currency.minutes(model, &batch, &filled.alt_seconds)?;
            single.clear();
            // (trip, minutes still to pay) and the seconds of each situation of `single`.
            let mut pending: Vec<(usize, f64)> = Vec::new();
            let mut pending_seconds: Vec<f64> = Vec::new();
            for (s, &t) in filled.trip_of.iter().enumerate() {
                let range = batch.range(s);
                let set_seconds = &filled.set_seconds[filled.set_at[s]..][..filled.set_len[s]];
                let least = set_seconds.iter().copied().fold(f64::INFINITY, f64::min);
                out.g_best_set[t] = g[range.clone()].iter().copied().fold(f64::INFINITY, f64::min);
                out.g_expected_set[t] = probabilities
                    .as_ref()
                    .map_or(f64::NAN, |p| range.clone().map(|a| p[a] * g[a]).sum());
                let taken = current.route[t] as usize;
                let offered = range.clone().find(|&a| filled.alt_route[a] as usize == taken);
                match offered {
                    Some(a) if !ended.unfinished[t] => {
                        out.g_chosen[t] = g[a];
                        out.g_remaining[t] = 0.0;
                    }
                    _ => {
                        let trip = TripId::from_index(t);
                        let traveller = trips.traveller(trip);
                        let view = sets.route(taken);
                        let (seconds, remaining) = if ended.unfinished[t] {
                            let departure = car_departure(trips, trip, self.inputs.car_start_s);
                            let (paid, rest) = ended.car_seconds(t, view.links, departure, times);
                            (paid + rest, rest)
                        } else {
                            (set_seconds[taken - filled.set_first[s]], 0.0)
                        };
                        let route = RouteRow {
                            links: view.links,
                            length_m: self.length_m(taken),
                            overlap: f64::from(view.overlap),
                            seconds,
                            best: least,
                            ln_path_size: 0.0,
                        };
                        single.begin_situation_in(traveller.raw(), trip.raw(), batch.classes()[s]);
                        self.route_row(&mut row, &route, trip, traveller);
                        single.push_alternative(self.identity[taken], &row);
                        pending.push((t, remaining / 60.0));
                        pending_seconds.push(seconds);
                    }
                }
                let least_cost = if self.extra.is_empty() {
                    least
                } else {
                    let extra = &self.extra[self.vector_of(TripId::from_index(t)) as usize];
                    let first = filled.set_first[s];
                    set_seconds
                        .iter()
                        .enumerate()
                        .map(|(i, x)| x + extra[first + i])
                        .fold(f64::INFINITY, f64::min)
                };
                measured.push((t, least, least_cost));
            }
            if !pending.is_empty() {
                single.validate()?;
                let g = currency.minutes(model, &single, &pending_seconds)?;
                for ((t, remaining), g) in pending.into_iter().zip(g) {
                    out.g_chosen[t] = g;
                    out.g_remaining[t] = remaining;
                }
            }
        }
        // The whole network (S294): each trip's route read off the shared trees, re-costed at
        // the trip's own departure, and costed as any route of the sets where it is no slower than
        // the set's best.
        for chunk in measured.chunks(CHUNK) {
            single.clear();
            let mut pending: Vec<usize> = Vec::new();
            let mut pending_seconds: Vec<f64> = Vec::new();
            for &(t, least, least_cost) in chunk {
                out.g_best_network[t] = out.g_best_set[t];
                let key = trip_keys[t];
                let trip = TripId::from_index(t);
                let departure = car_departure(trips, trip, self.inputs.car_start_s);
                let vector = self.vector_of(trip);
                let Some(found) = trees.route_of(key.origin, vector, key.destination, departure)
                else {
                    continue;
                };
                let seconds = times.route_seconds(&found.links, departure);
                // Costed as a route where it is no dearer than the set's least, in the trip's
                // vector's generalised cost (its time where the cost is time alone).
                let cost = match self.route_cost() {
                    None => seconds,
                    Some(c) => seconds + c.route_extra(vector, &found.links),
                };
                if cost > least_cost {
                    continue;
                }
                let links = &found.links;
                let length_m = if self.wants_length {
                    links.iter().map(|&l| network.link_length(LinkId::new(l)).get()).sum()
                } else {
                    0.0
                };
                let route = RouteRow {
                    links,
                    length_m,
                    overlap: 0.0,
                    seconds,
                    best: least,
                    ln_path_size: 0.0,
                };
                let traveller = trips.traveller(trip);
                let class = self.inputs.travellers.user_class(traveller).raw();
                single.begin_situation_in(traveller.raw(), trip.raw(), class);
                self.route_row(&mut row, &route, trip, traveller);
                single.push_alternative(route_identity(links), &row);
                pending.push(t);
                pending_seconds.push(seconds);
            }
            if !pending.is_empty() {
                single.validate()?;
                let g = currency.minutes(model, &single, &pending_seconds)?;
                for (t, g) in pending.into_iter().zip(g) {
                    out.g_best_network[t] = out.g_best_network[t].min(g);
                }
            }
        }
        Ok(())
    }

    fn empty_choices(&self) -> RouteChoices {
        let total = self.inputs.trips.len() as usize;
        RouteChoices {
            route: vec![NO_ROUTE; total],
            alternatives: vec![0; total],
            probability: vec![f64::NAN; total],
            weight: self.weight.clone(),
        }
    }

    /// Every trip chooses, on free-flow costs (iteration 0).
    ///
    /// # Errors
    ///
    /// [`ChoiceError`] if the model is given a malformed batch, or fails or answers
    /// wrongly.
    pub(crate) fn choose_all(
        &self,
        rng: &StreamRng,
        iteration: u32,
        times: Option<&LinkTimes>,
    ) -> Result<RouteChoices, ChoiceError> {
        let total = self.inputs.trips.len() as usize;
        let mut out = self.empty_choices();
        let mut batch = ChoiceBatch::new(iteration, &self.wanted_names());
        let mut filled = Filled::default();
        let mut i = 0;
        while i < total {
            let end = (i + CHUNK).min(total);
            self.fill(&mut batch, &mut filled, i..end, times, None);
            i = end;
            if filled.trip_of.is_empty() {
                continue;
            }
            batch.validate()?;
            let choices = self.inputs.model.choose(&batch, rng)?;
            choices.validate(&batch)?;
            for (s, &t) in filled.trip_of.iter().enumerate() {
                out.route[t] = filled.alt_route[batch.range(s).start + choices.chosen[s] as usize];
                // How many routes its pair's set holds, however many were offered.
                out.alternatives[t] = u32::try_from(filled.set_len[s]).expect("few alternatives");
                out.probability[t] = choices.probability[s];
            }
        }
        Ok(out)
    }

    /// The travellers `who` marks (by traveller) choose again on the link `times`, drawing as
    /// [`Self::choose_all`] does at `iteration`; the others are not asked (one group of
    /// the free-flow loading's increments). Returns what they chose, to apply as
    /// [`Update::changes`] are. Nothing is assessed.
    ///
    /// # Errors
    ///
    /// [`ChoiceError`] as for [`Self::choose_all`].
    pub(crate) fn choose_some(
        &self,
        times: &LinkTimes,
        iteration: u32,
        who: &[bool],
        rng: &StreamRng,
    ) -> Result<Vec<Change>, ChoiceError> {
        let total = self.inputs.trips.len() as usize;
        let mut batch = ChoiceBatch::new(iteration, &self.wanted_names());
        let mut filled = Filled::default();
        let mut changes: Vec<Change> = Vec::new();
        let mut i = 0;
        while i < total {
            let end = (i + CHUNK).min(total);
            self.fill(&mut batch, &mut filled, i..end, Some(times), Some(who));
            i = end;
            if filled.trip_of.is_empty() {
                continue;
            }
            batch.validate()?;
            let choices = self.inputs.model.choose(&batch, rng)?;
            choices.validate(&batch)?;
            for (s, &t) in filled.trip_of.iter().enumerate() {
                changes.push(Change {
                    trip: t,
                    route: filled.alt_route[batch.range(s).start + choices.chosen[s] as usize],
                    probability: choices.probability[s],
                });
            }
        }
        Ok(changes)
    }

    /// Assess the assignment `current` against the link `times` its loading
    /// produced, and, if `strategy` is given, work out who would choose again and
    /// what they would choose. **Nothing is changed**: the caller applies
    /// [`Update::changes`] if the run goes on (it may stop first, having converged).
    ///
    /// The assessment (the flow and cost gaps against their floor)
    /// is of `current` as it was loaded. **The cost gaps leave out the trips still under
    /// way when the window ended** (`unfinished[t]`: their times are lower bounds), and
    /// say how many those are; the expected gap is what the model itself expects of the rest.
    ///
    /// # Errors
    ///
    /// [`ChoiceError`] as for [`Self::choose_all`].
    #[allow(
        clippy::too_many_arguments,
        reason = "the loading's times and draws, who finished, and the generalised gap's currency"
    )]
    pub(crate) fn update(
        &self,
        current: &RouteChoices,
        times: &LinkTimes,
        iteration: u32,
        strategy: Option<(&dyn Equilibration, &StreamRng, f64)>,
        rng: &StreamRng,
        unfinished: &[bool],
        currency: Option<&Currency>,
    ) -> Result<Update, ChoiceError> {
        let total = self.inputs.trips.len() as usize;
        let sets = &*self.route_sets;
        let mut batch = ChoiceBatch::new(iteration, &self.wanted_names());
        let mut filled = Filled::default();
        let routes = sets.route_count();
        let (mut observed, mut expected, mut sample) =
            (vec![0.0; routes], vec![0.0; routes], vec![0.0; routes]);
        let (mut w_all, mut w_reselected, mut w_changed, mut w_unfinished) = (0.0, 0.0, 0.0, 0.0);
        let (mut paid, mut least, mut expected_paid) = (0.0, 0.0, 0.0);
        // The same in generalised minutes, when asked (S299).
        let (mut g_paid, mut g_least, mut g_expected) = (0.0, 0.0, 0.0);
        let mut g_measured = currency.is_some();
        let mut expected_seconds = vec![f64::NAN; total];
        let mut measured = true;
        let mut changes: Vec<Change> = Vec::new();
        let mut i = 0;
        while i < total {
            let end = (i + CHUNK).min(total);
            self.fill(&mut batch, &mut filled, i..end, Some(times), None);
            i = end;
            if filled.trip_of.is_empty() {
                continue;
            }
            batch.validate()?;
            let probabilities = self.inputs.model.probabilities(&batch)?;
            measured &= probabilities.is_some();
            let generalised = match currency {
                Some(c) => Some(c.minutes(self.inputs.model, &batch, &filled.alt_seconds)?),
                None => None,
            };
            // Each situation's draws, pure functions of their keys, drawn in parallel and read in
            // situation order below (S294): the floor sample's, and whether its traveller chooses
            // again.
            let situations: Vec<usize> = (0..filled.trip_of.len()).collect();
            let draws: Vec<(f64, bool)> = crate::transit::par_map(
                &situations,
                COSTING_CHUNK,
                || (),
                |(), &s| {
                    let u = if probabilities.is_some() {
                        floor_draw(rng, &batch, s, iteration)
                    } else {
                        f64::NAN
                    };
                    let again = strategy.is_some_and(|(strategy, msa_rng, share)| {
                        strategy.reselects(msa_rng, batch.travellers()[s], iteration, share)
                    });
                    (u, again)
                },
            );
            let mut moving: Vec<usize> = Vec::new();
            for (s, &t) in filled.trip_of.iter().enumerate() {
                let (u, again) = draws[s];
                let w = f64::from(current.weight[t]);
                w_all += w;
                let range = batch.range(s);
                let taken = current.route[t] as usize;
                // The gap: what the route taken costs against the cheapest of the set, at the
                // times this loading produced (the whole set, offered or not).
                let set_seconds = &filled.set_seconds[filled.set_at[s]..][..filled.set_len[s]];
                let least_here = set_seconds.iter().copied().fold(f64::INFINITY, f64::min);
                let finished = !unfinished[t];
                if finished {
                    paid += w * set_seconds[taken - filled.set_first[s]];
                    least += w * least_here;
                } else {
                    w_unfinished += w;
                }
                if let (Some(g), Some(p), true) = (&generalised, &probabilities, finished) {
                    let range_g = &g[range.clone()];
                    let taken_at =
                        range.clone().position(|r| filled.alt_route[r] as usize == taken);
                    if let Some(a) = taken_at {
                        g_paid += w * range_g[a];
                        g_least += w * range_g.iter().copied().fold(f64::INFINITY, f64::min);
                        g_expected += w * range.clone().map(|r| p[r] * g[r]).sum::<f64>();
                    }
                } else if generalised.is_some() && probabilities.is_none() {
                    g_measured = false;
                }
                if let Some(p) = &probabilities {
                    observed[taken] += w;
                    let mut cumulative = 0.0;
                    let mut drawn = false;
                    let mut expected_here = 0.0;
                    for (a, r) in range.clone().enumerate() {
                        let route = filled.alt_route[r] as usize;
                        expected[route] += w * p[r];
                        expected_here += p[r] * filled.alt_seconds[r];
                        cumulative += p[r];
                        if !drawn && (u < cumulative || a + 1 == range.len()) {
                            sample[route] += w;
                            drawn = true;
                        }
                    }
                    expected_seconds[t] = expected_here;
                    if finished {
                        expected_paid += w * expected_here;
                    }
                }
                if again {
                    moving.push(s);
                }
            }
            if !moving.is_empty() {
                let sub = batch.subset(&moving);
                let choices = self.inputs.model.choose(&sub, rng)?;
                choices.validate(&sub)?;
                for (m, &s) in moving.iter().enumerate() {
                    let t = filled.trip_of[s];
                    let w = f64::from(current.weight[t]);
                    let new = filled.alt_route[batch.range(s).start + choices.chosen[m] as usize];
                    w_reselected += w;
                    if new != current.route[t] {
                        w_changed += w;
                    }
                    changes.push(Change {
                        trip: t,
                        route: new,
                        probability: choices.probability[m],
                    });
                }
            }
        }
        let share = |x: f64| if w_all > 0.0 { x / w_all } else { 0.0 };
        let tv = |a: &[f64]| 0.5 * a.iter().zip(&expected).map(|(x, e)| (x - e).abs()).sum::<f64>();
        let sums = RouteGapSums {
            weight: w_all - w_unfinished,
            paid,
            least,
            expected: expected_paid,
            measured,
            g_paid,
            g_least,
            g_expected,
            g_measured,
        };
        let (gap, gap_expected, gap_excess) = sums.gaps();
        let assessment = Assessment {
            reselected_share: share(w_reselected),
            changed_share: share(w_changed),
            gap,
            gap_expected,
            gap_excess,
            incomplete_share: share(w_unfinished),
            gap_flow: if measured { share(tv(&observed)) } else { f64::NAN },
            gap_flow_floor: if measured { share(tv(&sample)) } else { f64::NAN },
            gap_generalised_excess: sums.generalised_excess(),
            sums,
        };
        Ok(Update { assessment, changes, expected_seconds })
    }
}

/// The uniform draw that picks a route for the floor sample of situation `s`:
/// keyed on (traveller, trip, `FLOOR_KEY | iteration`, 0) from the choice stream,
/// so it is independent of every draw the run itself makes.
fn floor_draw(rng: &StreamRng, batch: &ChoiceBatch, s: usize, iteration: u32) -> f64 {
    rng.unit(DrawAddress::from_quad(
        batch.travellers()[s],
        batch.trips()[s],
        FLOOR_KEY | iteration,
        0,
    ))
}
