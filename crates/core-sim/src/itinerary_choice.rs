//! Itinerary choice (M4, S201): which itinerary a transit, park-and-ride or
//! bike-and-ride trip takes, from a choice set of the competitive ones, by the
//! run's choice model.
//!
//! **The rule** (the user, S201, D9): the competitive options are in the choice
//! set, and the choice model selects the whole mode/route combination, legs
//! through different hubs included, from the attributes that matter. An
//! **alternative** here is a door-to-door itinerary:
//!
//! | Trip | Alternative |
//! |---|---|
//! | `transit` | walk · ride(s) · walk: one of RAPTOR's Pareto journeys (arrival against vehicles) |
//! | `car_transit` or `bike_transit`, vehicle at the origin (**out**) | drive or ride to a parking, park, walk to a stop, ride(s), walk: parking × route to it × journey from it |
//! | the same, vehicle at a parking (**back**) | walk, ride(s) to that parking's stops, walk, fetch the vehicle, drive or ride home: journey × route from it. **The hub is fixed: it is where the vehicle is** (design §23.2) |
//!
//! **Candidates** (out): the parkings of the vehicle's kind within reach
//! ([`crate::parking::ParkingDefaults::reach_car_s`],
//! [`crate::parking::ParkingDefaults::reach_bike_s`]); the `K` nearest and the `K` best by a rough
//! estimate of the whole trip are tried (`K` =
//! [`crate::parking::ParkingDefaults::candidates`]), each with one RAPTOR query; the
//! alternatives of at most `K` parkings are kept. **Car parks** are picked on the
//! free-flow drive to each (one backward search per car park, once per run), and only
//! the picked ones' routes are then searched on the expected times (S209). **Bike
//! parkings** are found by one search from the origin on the bike layer, whose costs
//! are static, so each trip's are found once per run and kept, pruned to those that can
//! ever be tried (S209). **The choice-set limit** (the run's `choice_detour_limit`) then
//! drops the alternatives much slower than the best, as it does for car routes.
//!
//! **The attributes** — one vocabulary for every alternative, routes included
//! ([`ATTRIBUTES`]): `time_min` (door to door), `car_min`, `bike_min`,
//! `walk_min`, `wait_min` (first wait and transfer waits, boarding slack
//! included), `ride_min`, `transfers`, `parking_min` (parking or fetching the
//! vehicle), `ln_path_size` (over the itinerary by time share: the vehicle leg
//! and each ride are the elements alternatives can share), and, as for routes,
//! `length_km` (the vehicle leg's), `detour`, `overlap` (0) and `n_links` (the
//! vehicle leg's); and for mode choice (M5) `nest` (the mode's index) and the 0/1
//! `mode_walk`, `mode_bike`, `mode_car`, `mode_transit`, `mode_car_transit` and
//! `mode_bike_transit`; and money (S248), `cost_eur` and its parts ([`crate::prices`]).
//!
//! **Identity** is a hash of the parking, the vehicle leg's links and the rides
//! as (line, boarding stop, alighting stop), **not the runs**, so a bus a
//! minute late keeps the alternative and its draws.
//!
//! **Choosing and executing** (A11): the whole itinerary is chosen on expected
//! costs (the last iteration's link times, realised timetable and parking
//! availability; free flow, the schedule and the start-of-day parkings at first);
//! a run executes it on the realised times, boarding the first run of each
//! chosen line that can be caught, and falls back to the earliest journey from
//! where the traveller is when a line cannot be followed (counted). **Later**
//! (S202, roadmap I-af): re-check the choice against the realised service.
//!
//! **Rounds:** a trip back to a parked vehicle depends on where the trip before
//! it left the vehicle, so each traveller's vehicle itinerary trips are chosen in
//! order, one round each (A13); plain transit trips in the first round.
//!
//! **Cost:** per transit trip one RAPTOR query (its walks to and from stops are searched
//! once per run, [`crate::transit::TransitSetup`]); per out trip one search, stopped at the
//! picked car parks or, by bike, once per run, and up to `2K` queries; per back trip one
//! query and a search per journey. Parallel in fixed chunks, so results never depend on
//! the thread count.

use std::sync::OnceLock;

use openmobisim_core_choice::{ChoiceBatch, ChoiceError, ChoiceModel};
use openmobisim_core_demand::{Mode, Travellers, Trips};
use openmobisim_core_graph::defaults::RoadClass;
use openmobisim_core_graph::geometry::{LonLat, ground_distance_metres};
use openmobisim_core_graph::hubs::ParkingKind;
use openmobisim_core_graph::layers::{BikeInfrastructure, StaticLayer};
use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_routes::{
    NodeSnapper, Reach, RouteAttributes, RouteKey, RouteSets, Search, SearchContext,
};
use openmobisim_core_transit::{Journey, JourneyLeg, Raptor, RaptorData, ServiceKind};
use openmobisim_core_types::hash::Fnv1a;
use openmobisim_core_types::ids::{EntityId, LinkId, NULL_ID, NodeId, TripId};
use openmobisim_core_types::rng::{DrawAddress, StreamRng};

use crate::equilibration::Equilibration;
use crate::layers::{LayerSetup, StaticLayers, StaticRoutes};
use crate::link_times::LinkTimes;
use crate::link_values::{PreparedLinkValues, ValueLayer};
use crate::parking::{ExpectedAvailability, ParkingSetup};
use crate::prices::{Cost, PreparedPrices};
use crate::transit::{TransitSetup, par_map};

/// The attributes every alternative carries, routes and itineraries alike, in
/// the order a batch holds them (A8).
///
/// After `parking_min` (S236, roadmap I-bb): the bike leg's kilometres by facility
/// (`bike_separated_km`, `bike_lane_km`, `bike_mixed_km`), and a transit itinerary's walks,
/// waits and rides split — the walk to the first stop, from the last and between stops
/// (`walk_access_min`, `walk_egress_min`, `walk_transfer_min`), the wait for the first
/// vehicle and for the others (`wait_first_min`, `wait_transfer_min`), and the minutes on
/// board by kind of service (`ride_rail_min` … `ride_other_min`). Each is a part of a total
/// above it, so a model weighs the parts on top of the totals, or instead of them.
///
/// After those (S248, roadmap I-bb U5): money, in euros — `cost_eur` and its parts
/// `cost_running_eur`, `cost_toll_eur`, `cost_parking_eur`, `cost_fare_eur` ([`crate::prices`]).
pub const ATTRIBUTES: [&str; 39] = [
    "time_min",
    "length_km",
    "detour",
    "overlap",
    "ln_path_size",
    "n_links",
    "car_min",
    "bike_min",
    "walk_min",
    "wait_min",
    "ride_min",
    "transfers",
    "parking_min",
    "bike_separated_km",
    "bike_lane_km",
    "bike_mixed_km",
    "walk_access_min",
    "walk_egress_min",
    "walk_transfer_min",
    "wait_first_min",
    "wait_transfer_min",
    "ride_rail_min",
    "ride_metro_min",
    "ride_tram_min",
    "ride_bus_min",
    "ride_ferry_min",
    "ride_other_min",
    "cost_eur",
    "cost_running_eur",
    "cost_toll_eur",
    "cost_parking_eur",
    "cost_fare_eur",
    "nest",
    "mode_walk",
    "mode_bike",
    "mode_car",
    "mode_transit",
    "mode_car_transit",
    "mode_bike_transit",
];

/// The value of a mode attribute for an alternative of `mode`: `nest` is the mode's index
/// ([`Mode::index`], the nest of a nested logit), `mode_<name>` is 1 for that mode's
/// alternatives and 0 for the rest (so `beta_mode_bike` is the bike's constant, M5).
pub(crate) fn mode_attribute(name: &str, mode: Mode) -> Option<f64> {
    if name == "nest" {
        #[allow(clippy::cast_precision_loss, reason = "a small index")]
        return Some(mode.index() as f64);
    }
    let rest = name.strip_prefix("mode_")?;
    Some(if rest == mode.as_str() { 1.0 } else { 0.0 })
}

/// No parking: a plain transit alternative.
pub const NO_PARKING: u32 = u32::MAX;

/// How many trips are asked about at once (as for routes).
const CHUNK: usize = 32_768;

/// Trips planned per scratch: a scratch holds searches over whole networks, so
/// a thread makes one per chunk it takes, not one per few trips. Results do not
/// depend on it.
pub(crate) const SCRATCH_CHUNK: usize = 64;

/// The longest drive or ride home from a parking a search looks for, in seconds.
const BACK_BOUND_S: f64 = 6.0 * 3600.0;

/// Set in the iteration half of the gap sample's draw address, so its draws are apart from the
/// reselection's (`(traveller, iteration)`, iterations below 2³¹).
const GAP_SAMPLE_SALT: u32 = 1 << 31;

/// The shape of an itinerary trip, given where its vehicle is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    /// Walk, ride, walk.
    Transit,
    /// The vehicle first, then transit.
    Out,
    /// Transit first, then the vehicle parked at this parking.
    Back(u32),
    /// One leg on one layer (M5, mode choice): a car route, a bike route or a walk; the
    /// alternative's [`Alternative::mode`] says which.
    Direct,
}

/// A ride of a chosen journey: the line and the stops.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlanRide {
    /// The line: the timetable's route.
    pub route: u32,
    /// Where it is boarded.
    pub board: NodeId,
    /// Where it is left.
    pub alight: NodeId,
}

/// One alternative: see the [module docs](self).
#[derive(Clone, Debug, PartialEq)]
pub struct Alternative {
    /// Its shape.
    pub shape: Shape,
    /// The mode it is: [`Mode::Transit`], [`Mode::CarTransit`] or [`Mode::BikeTransit`]
    /// for an itinerary, [`Mode::Car`], [`Mode::Bike`] or [`Mode::Walk`] for a direct leg.
    pub mode: Mode,
    /// The path size's log fixed from elsewhere (a car route's, from its route set), or
    /// `None` to work it out over the alternatives.
    pub fixed_ln_path_size: Option<f64>,
    /// The parking used ([`NO_PARKING`] for plain transit).
    pub parking: u32,
    /// The vehicle leg's links (empty for plain transit): to the parking out, from
    /// it back.
    pub vehicle_links: Vec<LinkId>,
    /// The vehicle leg's expected seconds.
    pub vehicle_s: f64,
    /// The expected time to park (out) or fetch the vehicle (back).
    pub parking_s: f64,
    /// The first stop, and the walk to it (from the origin, or from the parking
    /// out), stop transfer included, in seconds.
    pub first_stop: NodeId,
    /// See [`Self::first_stop`].
    pub access_walk_s: u32,
    /// The rides.
    pub rides: Vec<PlanRide>,
    /// Each ride's expected seconds.
    pub ride_seconds: Vec<u32>,
    /// Before each ride after the first, the walk from the last stop left, in seconds.
    pub transfer_walks: Vec<u32>,
    /// The last stop, and the walk from it (to the destination, or to the parking
    /// back), stop transfer included.
    pub last_stop: NodeId,
    /// See [`Self::last_stop`].
    pub egress_walk_s: u32,
    /// Expected seconds walking, waiting and riding.
    pub walk_s: f64,
    /// See [`Self::walk_s`].
    pub wait_s: f64,
    /// See [`Self::walk_s`].
    pub ride_s: f64,
    /// Expected seconds door to door.
    pub total_s: f64,
    /// The second the vehicle leaves the parking (back), expected.
    pub vehicle_departure: f64,
    /// The vehicle leg's length in metres.
    pub vehicle_m: f64,
    /// Its identity.
    pub identity: u32,
    /// Its path size's natural log.
    pub ln_path_size: f64,
    /// Its bike leg's metres on separated tracks, painted lanes and in mixed traffic, in
    /// [`BikeInfrastructure`]'s order (S236); zeros without a bike leg. A ferry crossing counts
    /// in none.
    pub bike_m: [f64; 3],
    /// The wait for the first vehicle, in seconds (part of [`Self::wait_s`]).
    pub wait_first_s: u32,
    /// Seconds on board by kind of service, in [`ServiceKind`]'s order (part of
    /// [`Self::ride_s`]).
    pub ride_kind_s: [u32; 6],
    /// The rides' metres from each boarding stop to its alighting stop as the crow flies: what a
    /// distance fare charges (S248).
    pub ride_m: f64,
}

impl Alternative {
    /// Vehicles boarded, minus one.
    #[must_use]
    pub fn transfers(&self) -> u32 {
        u32::try_from(self.rides.len().saturating_sub(1)).unwrap_or(u32::MAX)
    }

    fn identity_hash(&self) -> u32 {
        let mut h = Fnv1a::new();
        match self.shape {
            Shape::Transit => h.write_u32(0),
            Shape::Out => h.write_u32(1),
            Shape::Back(_) => h.write_u32(2),
            Shape::Direct => h.write_u32(3 + u32::try_from(self.mode.index()).unwrap_or(0)),
        }
        h.write_u32(self.parking);
        for l in &self.vehicle_links {
            h.write_u32(l.raw());
        }
        for r in &self.rides {
            h.write_u32(r.route);
            h.write_u32(r.board.raw());
            h.write_u32(r.alight.raw());
        }
        let v = h.finish();
        #[allow(clippy::cast_possible_truncation, reason = "folding 64 bits into 32 on purpose")]
        let folded = (v ^ (v >> 32)) as u32;
        folded
    }
}

/// An itinerary trip's nodes on the layers it may use (null where absent), and how far its
/// traveller walks to and from a stop.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TripNodes {
    pub walk_o: NodeId,
    pub walk_d: NodeId,
    pub road_o: NodeId,
    pub road_d: NodeId,
    pub bike_o: NodeId,
    pub bike_d: NodeId,
    /// The longest walk to or from a stop, in seconds: the class's (S235) or the run's.
    pub access_walk_s: f64,
}

/// What a trip may choose from (M5): each mode offered to it where it is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Offer {
    /// Whether the trip chooses its mode (its mode was not stated).
    pub choosing: bool,
    pub walk: bool,
    pub bike: bool,
    pub car: bool,
    pub transit: bool,
    /// Park-and-ride out, or back to the car parked there.
    pub car_transit: Option<Shape>,
    /// Bike-and-ride, likewise.
    pub bike_transit: Option<Shape>,
    /// Out through this parking only: a kept itinerary re-costed, not re-planned (S210).
    pub parking: Option<u32>,
}

impl Offer {
    fn is_empty(&self) -> bool {
        Self { choosing: false, parking: None, ..*self } == Self::default()
    }

    /// This offer, out through `parking` only.
    fn at_parking(self, parking: u32) -> Self {
        Self { parking: Some(parking), ..self }
    }

    /// Whether a vehicle waits at a parking to be fetched.
    fn fetches(&self) -> bool {
        matches!(self.car_transit, Some(Shape::Back(_)))
            || matches!(self.bike_transit, Some(Shape::Back(_)))
    }

    /// This offer with only `mode` left in it.
    fn only(&self, mode: Mode) -> Self {
        let mut out = Self { choosing: self.choosing, ..Self::default() };
        match mode {
            Mode::Walk => out.walk = self.walk,
            Mode::Bike => out.bike = self.bike,
            Mode::Car => out.car = self.car,
            Mode::Transit => out.transit = self.transit,
            Mode::CarTransit => out.car_transit = self.car_transit,
            Mode::BikeTransit => out.bike_transit = self.bike_transit,
        }
        out
    }
}

/// A trip's route on a static layer (bike or walk), for mode choice.
#[derive(Clone, Debug)]
pub(crate) struct StaticLeg {
    pub links: Vec<LinkId>,
    pub seconds: f64,
    pub metres: f64,
}

/// The car's route sets, for the car alternatives of mode choice.
pub(crate) struct CarRoutes<'a> {
    pub sets: &'a RouteSets,
    pub attributes: &'a RouteAttributes,
}

/// What an itinerary choice set is made on: see the [module docs](self).
pub(crate) struct Planner<'a> {
    /// The timetable and the RAPTOR data to plan on; `None` in a run without a timetable
    /// (mode choice among walk, bike and car).
    pub transit: Option<(&'a TransitSetup, &'a RaptorData)>,
    pub parking: Option<&'a ParkingSetup>,
    pub car: &'a SearchContext<'a>,
    pub bike: Option<(&'a LayerSetup, &'a SearchContext<'a>)>,
    pub times: &'a LinkTimes,
    pub availability: Option<&'a ExpectedAvailability>,
    pub detour_limit: f64,
    /// The car's route sets, when the run offers mode choice.
    pub car_routes: Option<CarRoutes<'a>>,
}

/// One thread's scratch for planning.
pub(crate) struct Scratch<'a> {
    /// Walks to and from stops, and RAPTOR; `None` without a timetable.
    transit: Option<TransitScratch<'a>>,
    /// The car and bike searches, made on first use: a plain transit trip needs
    /// neither, and each holds a few bytes per link of its whole network.
    car: Option<Search<'a>>,
    bike: Option<Search<'a>>,
    /// One flag per road node: the car parks a park-and-ride search looks for (S209). Made on
    /// first use, and left all false between uses.
    car_mask: Vec<bool>,
}

/// The transit part of a [`Scratch`].
struct TransitScratch<'a> {
    out: Reach<'a>,
    into: Reach<'a>,
    raptor: Raptor<'a>,
}

impl<'a> Scratch<'a> {
    fn car(&mut self, ctx: &'a SearchContext<'a>) -> &mut Search<'a> {
        self.car.get_or_insert_with(|| Search::new(ctx))
    }

    fn bike(&mut self, ctx: &'a SearchContext<'a>) -> &mut Search<'a> {
        self.bike.get_or_insert_with(|| Search::new(ctx))
    }
}

/// A bike parking within a bike-and-ride trip's reach: its node, the ride's seconds and
/// metres, and the route (S209).
#[derive(Clone, Debug)]
pub(crate) struct ReachCand {
    node: NodeId,
    seconds: f64,
    links: Box<[LinkId]>,
    metres: f64,
}

/// One trip's bike parkings in reach, kept once found: see [`Itineraries::bike_reach`].
pub(crate) type BikeReach = OnceLock<Box<[ReachCand]>>;

impl<'a> Planner<'a> {
    pub(crate) fn scratch(&self) -> Scratch<'a> {
        Scratch {
            transit: self.transit.map(|(transit, data)| {
                let walk = transit.walk().network();
                let seconds = transit.walk_link_seconds();
                TransitScratch {
                    out: Reach::new(walk, seconds),
                    into: Reach::new(walk, seconds),
                    raptor: Raptor::new(data, transit.defaults().max_rides as usize),
                }
            }),
            car: None,
            bike: None,
            car_mask: Vec::new(),
        }
    }

    /// Every alternative `offer` holds for a trip, finalised: deduplicated, within the
    /// choice-set limit, at most `K` parkings, path sizes set. `legs` are its bike and walk
    /// routes and `car_key` its car pair, for mode choice.
    ///
    /// **A vehicle parked at a parking is fetched** when that can be done: the trip is then
    /// offered only the ways back to it (M4's rule, kept for mode choice: nothing yet weighs
    /// leaving a car at a car park overnight, design §23.4). Only if there is none are the
    /// other modes planned.
    #[allow(clippy::too_many_arguments, reason = "a trip's facts and what it is offered")]
    pub(crate) fn plan(
        &self,
        s: &mut Scratch<'a>,
        n: &TripNodes,
        departure: u32,
        origin: LonLat,
        destination: LonLat,
        offer: Offer,
        legs: (Option<&StaticLeg>, Option<&StaticLeg>),
        car_key: Option<RouteKey>,
        bike_reach: Option<&BikeReach>,
    ) -> Vec<Alternative> {
        let k = self.parking.map_or(1, |p| p.defaults().candidate_count());
        let vehicles =
            [(offer.car_transit, ParkingKind::Car), (offer.bike_transit, ParkingKind::Bike)];
        let mut alts = Vec::new();
        for (shape, kind) in vehicles {
            if let Some(Shape::Back(p)) = shape {
                alts.extend(self.plan_back(s, n, departure, p, kind));
            }
        }
        if !alts.is_empty() {
            self.measure_bike_legs(&mut alts);
            return finalise(alts, self.detour_limit, k);
        }
        let (bike_leg, walk_leg) = legs;
        if let (true, Some(leg)) = (offer.walk, walk_leg) {
            alts.push(direct(Mode::Walk, leg));
        }
        if let (true, Some(leg)) = (offer.bike, bike_leg) {
            alts.push(direct(Mode::Bike, leg));
        }
        if let (true, Some(key), Some(routes)) = (offer.car, car_key, &self.car_routes) {
            alts.extend(self.plan_car(routes, key, departure));
        }
        if offer.transit {
            alts.extend(self.plan_transit(s, n, departure));
        }
        // Out only for a trip long enough when choosing (A18): nobody drives to a car park for
        // a 1 km trip. A trip given the mode is offered it at any length.
        let far_enough = !offer.choosing
            || self.parking.is_none_or(|p| {
                ground_distance_metres(origin, destination) >= p.defaults().pr_min_km * 1000.0
            });
        for (shape, kind) in vehicles {
            if let (Some(Shape::Out), true) = (shape, far_enough) {
                alts.extend(self.plan_out(
                    s,
                    n,
                    departure,
                    destination,
                    kind,
                    bike_reach,
                    offer.parking,
                ));
            }
        }
        self.measure_bike_legs(&mut alts);
        finalise(alts, self.detour_limit, k)
    }

    /// Each bike leg's metres by facility (S236): what a model sees of the route's quality. A
    /// ferry crossing is ridden on no facility, so it counts in none (S241; its time is in the
    /// leg's).
    fn measure_bike_legs(&self, alts: &mut [Alternative]) {
        let Some((layer, _)) = self.bike else { return };
        let net = layer.network();
        for a in alts.iter_mut().filter(|a| matches!(a.mode, Mode::Bike | Mode::BikeTransit)) {
            for &l in &a.vehicle_links {
                if net.network().link_class(l) != RoadClass::Ferry {
                    a.bike_m[net.infrastructure(l) as usize] += net.network().link_length(l).get();
                }
            }
        }
    }

    /// The car alternatives of a mode-choice trip: its pair's routes, costed on the
    /// expected times, with their route set's path sizes.
    fn plan_car(&self, routes: &CarRoutes<'_>, key: RouteKey, departure: u32) -> Vec<Alternative> {
        let Some(k) = routes.sets.key_index(key) else { return Vec::new() };
        routes
            .sets
            .route_range(k)
            .map(|r| {
                let view = routes.sets.route(r);
                let seconds = self.times.route_seconds(view.links, f64::from(departure));
                let mut a = blank(Mode::Car, Shape::Direct);
                a.vehicle_links = view.links.iter().map(|&l| LinkId::new(l)).collect();
                a.vehicle_s = seconds;
                a.vehicle_m = routes.attributes.length_m[r];
                a.total_s = seconds;
                a.fixed_ln_path_size = Some(routes.attributes.path_size[r].max(1e-9).ln());
                a
            })
            .collect()
    }

    fn plan_transit(&self, s: &mut Scratch<'a>, n: &TripNodes, departure: u32) -> Vec<Alternative> {
        let (Some((transit, _)), Some(ts)) = (self.transit, s.transit.as_mut()) else {
            return Vec::new();
        };
        if n.walk_o.is_null() || n.walk_d.is_null() {
            return Vec::new();
        }
        let access = transit.access(&mut ts.out, n.walk_o, departure, n.access_walk_s);
        let egress = transit.egress(&mut ts.into, n.walk_d, n.access_walk_s);
        if access.is_empty() || egress.is_empty() {
            return Vec::new();
        }
        ts.raptor
            .pareto(&access, &egress)
            .iter()
            .filter(|j| !is_loop(j))
            .map(|j| {
                let mut a = self.transit_part(j, departure);
                a.shape = Shape::Transit;
                a.total_s = f64::from(j.arrival.saturating_sub(departure));
                a
            })
            .collect()
    }

    /// The out itineraries through the candidate parkings, or through `only` alone (a kept
    /// itinerary re-costed, S210: the same search and query as for that parking among the
    /// candidates, so the same numbers).
    #[allow(clippy::too_many_arguments, reason = "a trip's facts and what it is offered")]
    fn plan_out(
        &self,
        s: &mut Scratch<'a>,
        n: &TripNodes,
        departure: u32,
        destination: LonLat,
        kind: ParkingKind,
        bike_reach: Option<&BikeReach>,
        only: Option<u32>,
    ) -> Vec<Alternative> {
        let Some(parking) = self.parking else { return Vec::new() };
        let d = parking.defaults();
        let dep = f64::from(departure);
        let (targets, count) = parking.targets(kind);
        // The vehicle's reach: `(node, seconds, links, metres)`.
        let found: Vec<(NodeId, f64, Vec<LinkId>, f64)> = match kind {
            ParkingKind::Car => {
                if n.road_o.is_null() {
                    return Vec::new();
                }
                // The candidates are picked on free-flow times to every car park, searched once
                // per run (S209); only their routes are searched on the expected times,
                // by one search that stops once it has reached them all.
                let picked = match only {
                    Some(p) => vec![parking.node(p)],
                    None => self.car_candidates(parking, n.road_o, dep, destination),
                };
                if picked.is_empty() {
                    return Vec::new();
                }
                let road = self.car.network;
                let mut mask = std::mem::take(&mut s.car_mask);
                mask.resize(road.node_count() as usize, false);
                for node in &picked {
                    mask[node.index()] = true;
                }
                let times = self.times;
                let wait = |l: u32, t: f64| times.origin_wait_seconds(l, t);
                let secs = |l: u32, t: f64| times.link_seconds(l, t);
                let found = s
                    .car(self.car)
                    .fastest_routes_to(
                        n.road_o,
                        &mask,
                        picked.len(),
                        dep,
                        d.reach_car_s,
                        &wait,
                        &secs,
                    )
                    .into_iter()
                    .map(|(node, t, links)| {
                        let m = links.iter().map(|&l| road.link_length(l).get()).sum();
                        (node, t, links, m)
                    })
                    .collect();
                for node in &picked {
                    mask[node.index()] = false;
                }
                s.car_mask = mask;
                found
            }
            ParkingKind::Bike => {
                let Some((layer, ctx)) = self.bike else {
                    return Vec::new();
                };
                if n.bike_o.is_null() {
                    return Vec::new();
                }
                let mut search_all = || -> Vec<(NodeId, f64, Vec<LinkId>, f64)> {
                    let cost = |l: u32, _t: f64| ctx.link_cost(LinkId::new(l));
                    let graph = layer.network().network();
                    s.bike(ctx)
                        .fastest_routes_to(
                            n.bike_o,
                            targets,
                            count,
                            0.0,
                            d.reach_bike_s,
                            &|_, _| 0.0,
                            &cost,
                        )
                        .into_iter()
                        .map(|(node, _, links)| {
                            let t = links.iter().map(|l| layer.seconds()[l.index()]).sum();
                            let m = links.iter().map(|&l| graph.link_length(l).get()).sum();
                            (node, t, links, m)
                        })
                        .collect()
                };
                // The bike layer's costs are static, so the search's answer is the trip's for
                // the whole run: found once and kept (S209), pruned to the parkings that can
                // ever be tried, whatever the parkings' availability.
                match bike_reach {
                    Some(cell) => cell
                        .get_or_init(|| {
                            prune_reach(search_all(), parking, kind, destination)
                                .into_iter()
                                .map(|(node, seconds, links, metres)| ReachCand {
                                    node,
                                    seconds,
                                    links: links.into_boxed_slice(),
                                    metres,
                                })
                                .collect()
                        })
                        .iter()
                        .map(|c| (c.node, c.seconds, c.links.to_vec(), c.metres))
                        .collect(),
                    None => search_all(),
                }
            }
        };
        struct Cand {
            parking: u32,
            links: Vec<LinkId>,
            metres: f64,
            vehicle_s: f64,
            parking_s: f64,
            est: f64,
        }
        let speed = (d.rank_speed_km_h / 3.6).max(0.1);
        let mut cands: Vec<Cand> = Vec::new();
        for (node, t, links, metres) in found {
            for &p in parking.at(kind, node) {
                let arrive = dep + t;
                let availability = self.availability.map_or(1.0, |a| a.at(p, arrive));
                let ps = d.parking_seconds(kind, availability);
                let walk = parking.stops(p).iter().map(|x| x.1).min().unwrap_or(0);
                let est = t
                    + ps
                    + f64::from(walk)
                    + ground_distance_metres(parking.position(p), destination) / speed;
                cands.push(Cand {
                    parking: p,
                    links: links.clone(),
                    metres,
                    vehicle_s: t,
                    parking_s: ps,
                    est,
                });
            }
        }
        // The K nearest and the K best by the estimate, the best first.
        let k = d.candidate_count();
        let mut by_near: Vec<usize> = (0..cands.len()).collect();
        by_near.sort_by(|&a, &b| {
            cands[a]
                .vehicle_s
                .total_cmp(&cands[b].vehicle_s)
                .then(cands[a].parking.cmp(&cands[b].parking))
        });
        let mut by_est: Vec<usize> = (0..cands.len()).collect();
        by_est.sort_by(|&a, &b| {
            cands[a].est.total_cmp(&cands[b].est).then(cands[a].parking.cmp(&cands[b].parking))
        });
        let mut tried: Vec<usize> = by_est.iter().copied().take(k).collect();
        for &c in by_near.iter().take(k) {
            if !tried.contains(&c) {
                tried.push(c);
            }
        }
        if let Some(p) = only {
            tried = (0..cands.len()).filter(|&c| cands[c].parking == p).collect();
        }
        let (Some((transit, _)), Some(ts)) = (self.transit, s.transit.as_mut()) else {
            return Vec::new();
        };
        if tried.is_empty() || n.walk_d.is_null() {
            return Vec::new();
        }
        let egress = transit.egress(&mut ts.into, n.walk_d, n.access_walk_s);
        if egress.is_empty() {
            return Vec::new();
        }
        let transfer = transit.transfer_s();
        let mut alts = Vec::new();
        for c in tried {
            let c = &cands[c];
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "a second of the day"
            )]
            let parked = (dep + c.vehicle_s + c.parking_s).floor() as u32;
            let access: Vec<(NodeId, u32)> = parking
                .stops(c.parking)
                .iter()
                .map(|&(stop, w)| (stop, parked.saturating_add(w).saturating_add(transfer)))
                .collect();
            for j in ts.raptor.pareto(&access, &egress).into_iter().filter(|j| !is_loop(j)) {
                let mut a = self.transit_part(&j, parked);
                a.shape = Shape::Out;
                a.mode = transit_mode(kind);
                a.parking = c.parking;
                a.vehicle_links.clone_from(&c.links);
                a.vehicle_s = c.vehicle_s;
                a.vehicle_m = c.metres;
                a.parking_s = c.parking_s;
                a.vehicle_departure = dep;
                a.total_s = f64::from(j.arrival) - dep;
                alts.push(a);
            }
        }
        alts
    }

    /// The car parks a park-and-ride trip leaving road node `origin` at `dep` tries: the `K`
    /// nearest and the `K` best by a rough estimate of the whole trip (as [`Self::plan_out`]
    /// ranks them), both on the **free-flow** drive to each car park within reach
    /// ([`ParkingSetup::car_free_flow`]), the time to park at the expected availability, the
    /// walk to the nearest stop and the rest of the way at the ranking speed. Their nodes,
    /// ascending.
    fn car_candidates(
        &self,
        parking: &ParkingSetup,
        origin: NodeId,
        dep: f64,
        destination: LonLat,
    ) -> Vec<NodeId> {
        let d = parking.defaults();
        let k = d.candidate_count();
        let speed = (d.rank_speed_km_h / 3.6).max(0.1);
        let ff = parking.car_free_flow();
        // (free-flow seconds, estimate, parking, node)
        let mut each: Vec<(f64, f64, u32, NodeId)> = Vec::new();
        for (i, &node) in ff.nodes.iter().enumerate() {
            let t = ff.seconds(i, origin);
            if t.is_nan() || t > d.reach_car_s {
                continue;
            }
            for &p in parking.at(ParkingKind::Car, node) {
                let availability = self.availability.map_or(1.0, |a| a.at(p, dep + t));
                let walk = parking.stops(p).iter().map(|x| x.1).min().unwrap_or(0);
                let est = t
                    + d.parking_seconds(ParkingKind::Car, availability)
                    + f64::from(walk)
                    + ground_distance_metres(parking.position(p), destination) / speed;
                each.push((t, est, p, node));
            }
        }
        let mut near: Vec<usize> = (0..each.len()).collect();
        near.sort_by(|&a, &b| each[a].0.total_cmp(&each[b].0).then(each[a].2.cmp(&each[b].2)));
        let mut best: Vec<usize> = (0..each.len()).collect();
        best.sort_by(|&a, &b| each[a].1.total_cmp(&each[b].1).then(each[a].2.cmp(&each[b].2)));
        let mut nodes: Vec<NodeId> =
            near.iter().take(k).chain(best.iter().take(k)).map(|&i| each[i].3).collect();
        nodes.sort_unstable();
        nodes.dedup();
        nodes
    }

    fn plan_back(
        &self,
        s: &mut Scratch<'a>,
        n: &TripNodes,
        departure: u32,
        p: u32,
        kind: ParkingKind,
    ) -> Vec<Alternative> {
        let Some(parking) = self.parking else { return Vec::new() };
        let (Some((transit, _)), Some(ts)) = (self.transit, s.transit.as_mut()) else {
            return Vec::new();
        };
        if n.walk_o.is_null() {
            return Vec::new();
        }
        let access = transit.access(&mut ts.out, n.walk_o, departure, n.access_walk_s);
        let transfer = transit.transfer_s();
        let egress: Vec<(NodeId, u32)> =
            parking.stops(p).iter().map(|&(stop, w)| (stop, w.saturating_add(transfer))).collect();
        if access.is_empty() || egress.is_empty() {
            return Vec::new();
        }
        let fetch = parking.defaults().fetch_seconds(kind);
        let dep = f64::from(departure);
        let from = parking.node(p);
        let journeys: Vec<Journey> =
            ts.raptor.pareto(&access, &egress).into_iter().filter(|j| !is_loop(j)).collect();
        let mut alts = Vec::new();
        for j in journeys {
            let leave = f64::from(j.arrival) + fetch;
            let leg = match kind {
                ParkingKind::Car => {
                    if n.road_d.is_null() {
                        continue;
                    }
                    let times = self.times;
                    let wait = |l: u32, t: f64| times.origin_wait_seconds(l, t);
                    let secs = |l: u32, t: f64| times.link_seconds(l, t);
                    let road = self.car.network;
                    s.car(self.car)
                        .fastest_route(from, n.road_d, leave, BACK_BOUND_S, &wait, &secs)
                        .map(|(t, links)| {
                            let m = links.iter().map(|&l| road.link_length(l).get()).sum();
                            (t, links, m)
                        })
                }
                ParkingKind::Bike => match self.bike {
                    Some((layer, ctx)) if !n.bike_d.is_null() => {
                        s.bike(ctx).shortest(from, n.bike_d).map(|r| {
                            let graph = layer.network().network();
                            let t = r.links.iter().map(|l| layer.seconds()[l.index()]).sum();
                            let m = r.links.iter().map(|&l| graph.link_length(l).get()).sum();
                            (t, r.links, m)
                        })
                    }
                    _ => None,
                },
            };
            let Some((t, links, metres)) = leg else { continue };
            let mut a = self.transit_part(&j, departure);
            a.shape = Shape::Back(p);
            a.mode = transit_mode(kind);
            a.parking = p;
            a.vehicle_links = links;
            a.vehicle_s = t;
            a.vehicle_m = metres;
            a.parking_s = fetch;
            a.vehicle_departure = leave;
            a.total_s = leave + t - dep;
            alts.push(a);
        }
        alts
    }

    /// The transit part of an alternative from a RAPTOR journey the traveller
    /// starts walking into at `start`.
    fn transit_part(&self, j: &Journey, start: u32) -> Alternative {
        let (transit, _) = self.transit.expect("a journey comes from a timetable");
        let tt = transit.timetable();
        let mut a = blank(Mode::Transit, Shape::Transit);
        let mut clock = start;
        let mut walk_before = 0u32;
        for leg in &j.legs {
            match *leg {
                JourneyLeg::Access { stop, arrival } => {
                    a.first_stop = stop;
                    a.access_walk_s = arrival.saturating_sub(start);
                    a.walk_s += f64::from(a.access_walk_s);
                    clock = arrival;
                }
                JourneyLeg::Ride { run, board_stop, alight_stop, departure, arrival, .. } => {
                    let wait = departure.saturating_sub(clock);
                    a.wait_s += f64::from(wait);
                    if a.rides.is_empty() {
                        a.wait_first_s = wait;
                    }
                    let ride = arrival.saturating_sub(departure);
                    a.ride_s += f64::from(ride);
                    let kind = tt.route_kind(tt.run_route(run)) as usize;
                    a.ride_kind_s[kind] = a.ride_kind_s[kind].saturating_add(ride);
                    if !a.rides.is_empty() {
                        a.transfer_walks.push(walk_before);
                    }
                    walk_before = 0;
                    a.rides.push(PlanRide {
                        route: tt.run_route(run),
                        board: board_stop,
                        alight: alight_stop,
                    });
                    a.ride_seconds.push(ride);
                    a.ride_m += ground_distance_metres(
                        tt.stop_position(board_stop),
                        tt.stop_position(alight_stop),
                    );
                    a.last_stop = alight_stop;
                    clock = arrival;
                }
                JourneyLeg::Transfer { seconds, .. } => {
                    walk_before += seconds;
                    a.walk_s += f64::from(seconds);
                    clock = clock.saturating_add(seconds);
                }
                JourneyLeg::Egress { stop, seconds } => {
                    a.last_stop = stop;
                    a.egress_walk_s = seconds;
                    a.walk_s += f64::from(seconds);
                }
            }
        }
        a
    }
}

/// The parkings of `found` (`(node, seconds, links, metres)`, as the bike-and-ride search
/// returns them) that can ever be among the ones [`Planner::plan_out`] tries: the `K`
/// nearest by riding time, or one of the `K` best by its estimate of the whole trip,
/// whatever the parkings' availability (S209). Availability moves an estimate only through
/// the time to park, between its floor (all room) and floor plus slope (full), so a
/// parking whose best estimate is above the `K`-th smallest worst estimate is never among
/// the `K` best, and one more than `K` others are nearer than is never among the nearest.
/// Exact: the kept list gives the same tried set as the whole list, at any availability.
fn prune_reach(
    found: Vec<(NodeId, f64, Vec<LinkId>, f64)>,
    parking: &ParkingSetup,
    kind: ParkingKind,
    destination: LonLat,
) -> Vec<(NodeId, f64, Vec<LinkId>, f64)> {
    let d = parking.defaults();
    let k = d.candidate_count();
    let speed = (d.rank_speed_km_h / 3.6).max(0.1);
    let (all_room, full) = (d.parking_seconds(kind, 1.0), d.parking_seconds(kind, 0.0));
    // Per parking: (its node's index in `found`, riding seconds, best estimate, worst estimate).
    let mut each: Vec<(usize, f64, f64, f64)> = Vec::new();
    for (i, (node, t, _, _)) in found.iter().enumerate() {
        for &p in parking.at(kind, *node) {
            let walk = f64::from(parking.stops(p).iter().map(|x| x.1).min().unwrap_or(0));
            let rest = walk + ground_distance_metres(parking.position(p), destination) / speed;
            each.push((i, *t, t + all_room + rest, t + full + rest));
        }
    }
    if each.len() <= k {
        return found;
    }
    let kth = |mut v: Vec<f64>| {
        v.sort_by(f64::total_cmp);
        v[k - 1]
    };
    let near_k = kth(each.iter().map(|e| e.1).collect());
    let worst_k = kth(each.iter().map(|e| e.3).collect());
    let mut keep = vec![false; found.len()];
    for &(i, t, best, _) in &each {
        if t <= near_k || best <= worst_k {
            keep[i] = true;
        }
    }
    found.into_iter().zip(keep).filter_map(|(f, k)| k.then_some(f)).collect()
}

/// Whether a journey comes back to the stop it first boarded at: RAPTOR has no journey
/// without a ride, so where the start and the end share a stop it can offer a ride out and
/// back, which nobody takes (M5, found on the toy: `W → N1`, where the trip ends at a stop).
fn is_loop(j: &Journey) -> bool {
    let mut rides = j.legs.iter().filter_map(|l| match *l {
        JourneyLeg::Ride { board_stop, alight_stop, .. } => Some((board_stop, alight_stop)),
        _ => None,
    });
    let first = rides.next();
    let last = rides.next_back().or(first);
    matches!((first, last), (Some((board, _)), Some((_, alight))) if board == alight)
}

/// An alternative of `mode` and `shape` with nothing in it yet.
fn blank(mode: Mode, shape: Shape) -> Alternative {
    Alternative {
        shape,
        mode,
        fixed_ln_path_size: None,
        parking: NO_PARKING,
        vehicle_links: Vec::new(),
        vehicle_s: 0.0,
        parking_s: 0.0,
        first_stop: NodeId::from_raw(NULL_ID),
        access_walk_s: 0,
        rides: Vec::new(),
        ride_seconds: Vec::new(),
        transfer_walks: Vec::new(),
        last_stop: NodeId::from_raw(NULL_ID),
        egress_walk_s: 0,
        walk_s: 0.0,
        wait_s: 0.0,
        ride_s: 0.0,
        total_s: 0.0,
        vehicle_departure: 0.0,
        vehicle_m: 0.0,
        identity: 0,
        ln_path_size: 0.0,
        bike_m: [0.0; 3],
        wait_first_s: 0,
        ride_kind_s: [0; 6],
        ride_m: 0.0,
    }
}

/// A bike route or a walk as an alternative (M5).
fn direct(mode: Mode, leg: &StaticLeg) -> Alternative {
    let mut a = blank(mode, Shape::Direct);
    a.vehicle_links.clone_from(&leg.links);
    a.vehicle_m = leg.metres;
    a.total_s = leg.seconds;
    if mode == Mode::Walk {
        a.walk_s = leg.seconds;
    } else {
        a.vehicle_s = leg.seconds;
    }
    a
}

/// The itinerary mode of a vehicle kind.
fn transit_mode(kind: ParkingKind) -> Mode {
    match kind {
        ParkingKind::Car => Mode::CarTransit,
        ParkingKind::Bike => Mode::BikeTransit,
    }
}

/// Deduplicate by identity (the fastest first, fewest vehicles on a tie), keep
/// what is within `detour_limit` of the best **of its mode** (0: all), keep the
/// alternatives of at most `parkings` parkings per mode, and set the identities and
/// path sizes.
///
/// The limits are per mode (M5) because the modes' times are not alike: a walk is always
/// slower than a drive, and whether it is a real alternative is for the model to weigh
/// (with its mode constants), not for a time limit.
fn finalise(mut alts: Vec<Alternative>, detour_limit: f64, parkings: usize) -> Vec<Alternative> {
    for a in &mut alts {
        a.identity = a.identity_hash();
    }
    alts.sort_by(|a, b| {
        a.total_s
            .total_cmp(&b.total_s)
            .then(a.rides.len().cmp(&b.rides.len()))
            .then(a.parking.cmp(&b.parking))
            .then(a.identity.cmp(&b.identity))
    });
    let mut seen: Vec<u32> = Vec::with_capacity(alts.len());
    alts.retain(|a| {
        if seen.contains(&a.identity) {
            false
        } else {
            seen.push(a.identity);
            true
        }
    });
    if detour_limit > 0.0 {
        let best = best_by_mode(&alts);
        alts.retain(|a| a.total_s <= best[a.mode.index()] * (1.0 + detour_limit));
    }
    // At most `parkings` parkings per mode; alternatives without one (transit, a direct leg)
    // are not counted against it.
    let mut kept: Vec<(Mode, u32)> = Vec::new();
    alts.retain(|a| {
        if a.parking == NO_PARKING || kept.contains(&(a.mode, a.parking)) {
            true
        } else if kept.iter().filter(|k| k.0 == a.mode).count() < parkings {
            kept.push((a.mode, a.parking));
            true
        } else {
            false
        }
    });
    // Path size by time share: the vehicle leg and each ride are elements
    // alternatives can share; walking, waiting and parking are each its own. A vehicle
    // leg is keyed by its mode too: the layers number their links each on their own.
    type Element = (u8, u32, u32, u32);
    let elements = |a: &Alternative| -> Vec<(Element, f64)> {
        let mut e = Vec::new();
        if a.vehicle_s > 0.0 {
            let mut h = Fnv1a::new();
            for l in &a.vehicle_links {
                h.write_u32(l.raw());
            }
            #[allow(clippy::cast_possible_truncation, reason = "a hash")]
            let hash = h.finish() as u32;
            #[allow(clippy::cast_possible_truncation, reason = "six modes")]
            e.push(((0, a.parking, hash, a.mode.index() as u32), a.vehicle_s));
        }
        for (r, &secs) in a.rides.iter().zip(&a.ride_seconds) {
            e.push(((1, r.route, r.board.raw(), r.alight.raw()), f64::from(secs)));
        }
        e
    };
    let all: Vec<Vec<(Element, f64)>> = alts.iter().map(elements).collect();
    for (i, a) in alts.iter_mut().enumerate() {
        if let Some(fixed) = a.fixed_ln_path_size {
            a.ln_path_size = fixed;
            continue;
        }
        let total = a.total_s.max(1.0);
        let mut shared = 0.0;
        let mut size = 0.0;
        for &(key, secs) in &all[i] {
            let users = all.iter().filter(|other| other.iter().any(|&(k, _)| k == key)).count();
            #[allow(clippy::cast_precision_loss, reason = "a handful of alternatives")]
            let users = users.max(1) as f64;
            size += secs / users;
            shared += secs;
        }
        size += (total - shared).max(0.0);
        a.ln_path_size = (size / total).clamp(1e-9, 1.0).ln();
    }
    // Identities must differ within a set: nudge a (vanishingly rare) clash.
    for i in 1..alts.len() {
        while alts[..i].iter().any(|b| b.identity == alts[i].identity) {
            alts[i].identity = alts[i].identity.wrapping_add(1);
        }
    }
    alts
}

/// The least expected door-to-door time of each mode's alternatives (infinite for a mode
/// with none).
fn best_by_mode(alts: &[Alternative]) -> [f64; Mode::COUNT] {
    let mut best = [f64::INFINITY; Mode::COUNT];
    for a in alts {
        let b = &mut best[a.mode.index()];
        *b = b.min(a.total_s);
    }
    best
}

/// The layer an alternative's vehicle or walking leg ([`Alternative::vehicle_links`]) runs on.
fn leg_layer(mode: Mode) -> Option<ValueLayer> {
    match mode {
        Mode::Car | Mode::CarTransit => Some(ValueLayer::Road),
        Mode::Bike | Mode::BikeTransit => Some(ValueLayer::Bike),
        Mode::Walk => Some(ValueLayer::Walk),
        Mode::Transit => None,
    }
}

/// What `a` costs, by part (S248, [`crate::prices`]): its car or bike leg's running cost, its
/// car leg's tolls, the fee of the parking it leaves its vehicle at (out; a trip back fetches a
/// vehicle already paid for) and its rides' fare.
fn cost(a: &Alternative, values: &PreparedLinkValues, prices: &PreparedPrices) -> Cost {
    let vehicle = vehicle_of(a.mode);
    Cost {
        running: prices.running(vehicle, a.vehicle_m),
        toll: if vehicle == Some(ParkingKind::Car) {
            prices.toll(values, &a.vehicle_links)
        } else {
            0.0
        },
        parking: if a.shape == Shape::Out && a.parking != NO_PARKING {
            prices.parking(a.parking)
        } else {
            0.0
        },
        fare: prices.fare(a.rides.len(), a.ride_m),
    }
}

/// The value of attribute `name` for `a`, the best total of its mode in its set being
/// `best`, the user's link values being `values` (S236) and the run's prices `prices` (S248).
fn attribute(
    name: &str,
    a: &Alternative,
    best: f64,
    values: &PreparedLinkValues,
    prices: &PreparedPrices,
) -> f64 {
    if name.starts_with("cost_") {
        if let Some(v) = cost(a, values, prices).attribute(name) {
            return v;
        }
    }
    if let Some((column, aggregate)) = values.lookup(name) {
        return if leg_layer(a.mode) == Some(values.layer(column)) {
            values.total(column, aggregate, a.vehicle_links.iter().map(|l| l.index()))
        } else {
            0.0
        };
    }
    if let Some(v) = mode_attribute(name, a.mode) {
        return v;
    }
    let kind = match a.mode {
        Mode::Car | Mode::CarTransit => Some(ParkingKind::Car),
        Mode::Bike | Mode::BikeTransit => Some(ParkingKind::Bike),
        Mode::Walk | Mode::Transit => None,
    };
    let vehicle_min = a.vehicle_s / 60.0;
    match name {
        "time_min" => a.total_s / 60.0,
        "length_km" => a.vehicle_m / 1000.0,
        "detour" => {
            if best > 0.0 {
                (a.total_s / best - 1.0).max(0.0)
            } else {
                0.0
            }
        }
        "ln_path_size" => a.ln_path_size,
        "n_links" => {
            #[allow(clippy::cast_precision_loss, reason = "a count of links")]
            let n = a.vehicle_links.len() as f64;
            n
        }
        "car_min" if kind == Some(ParkingKind::Car) => vehicle_min,
        "bike_min" if kind == Some(ParkingKind::Bike) => vehicle_min,
        "walk_min" => a.walk_s / 60.0,
        "wait_min" => a.wait_s / 60.0,
        "ride_min" => a.ride_s / 60.0,
        "transfers" => f64::from(a.transfers()),
        "parking_min" => a.parking_s / 60.0,
        "bike_separated_km" => a.bike_m[BikeInfrastructure::Separated as usize] / 1000.0,
        "bike_lane_km" => a.bike_m[BikeInfrastructure::Lane as usize] / 1000.0,
        "bike_mixed_km" => a.bike_m[BikeInfrastructure::Mixed as usize] / 1000.0,
        "walk_access_min" => f64::from(a.access_walk_s) / 60.0,
        "walk_egress_min" => f64::from(a.egress_walk_s) / 60.0,
        "walk_transfer_min" => f64::from(a.transfer_walks.iter().sum::<u32>()) / 60.0,
        "wait_first_min" => f64::from(a.wait_first_s) / 60.0,
        "wait_transfer_min" => (a.wait_s - f64::from(a.wait_first_s)) / 60.0,
        other => other
            .strip_prefix("ride_")
            .and_then(|k| k.strip_suffix("_min"))
            .and_then(|k| ServiceKind::ALL.iter().find(|s| s.as_str() == k))
            .map_or(0.0, |&s| f64::from(a.ride_kind_s[s as usize]) / 60.0),
    }
}

/// The attributes a model reads, checked against [`ATTRIBUTES`] and the user's link values'
/// (`extra`, S236), in that order.
///
/// # Errors
///
/// [`ChoiceError::MissingAttribute`] for a name that is not one of them.
pub(crate) fn wanted(
    model: &dyn ChoiceModel,
    builtin: &[&str],
    extra: &[String],
) -> Result<Vec<String>, ChoiceError> {
    let offered = || builtin.iter().map(|s| (*s).to_string()).chain(extra.iter().cloned());
    match model.required_attributes() {
        None => Ok(offered().collect()),
        Some(names) => {
            for name in &names {
                if !offered().any(|o| &o == name) {
                    return Err(ChoiceError::MissingAttribute {
                        name: name.clone(),
                        offered: offered().collect(),
                    });
                }
            }
            Ok(offered().filter(|a| names.iter().any(|n| n == a)).collect())
        }
    }
}

/// Each itinerary trip's choice, by position among the run's itinerary trips (in trip order).
#[derive(Clone, Debug, Default)]
pub struct Chosen {
    /// The alternative taken, or `None` (no vehicle where needed, or no alternative).
    pub alt: Vec<Option<Alternative>>,
    /// The probability the model gave it (`NaN` if none).
    pub probability: Vec<f64>,
    /// How many alternatives the trip had when it last chose (a trip that keeps its
    /// alternative is not asked again).
    pub alternatives: Vec<u32>,
    /// The logsum of its choice set when it was last planned (S238; `NaN` if the model gives
    /// none): the expected utility of its best alternative, for utility-based accessibility.
    pub logsum: Vec<f64>,
}

/// What one choice pass found, for the convergence report.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Assessment {
    /// Per mode (`Mode::index`): Σ w · expected seconds of the alternative kept, and
    /// of the least in its set.
    pub paid: [f64; Mode::COUNT],
    pub least: [f64; Mode::COUNT],
    /// Weights: all assessed, chose again, changed, had to choose again (their
    /// alternative had gone).
    pub w_all: f64,
    pub w_reselected: f64,
    pub w_changed: f64,
    pub w_forced: f64,
    /// Mode choice (M5): the weight of the trips choosing their mode that had a choice
    /// before, and of those whose mode changed.
    pub w_choosing: f64,
    pub w_mode_changed: f64,
    /// Kept itineraries re-costed at their parking, not re-planned (S210).
    pub recosted: u32,
    /// Per mode: Σ w · the time the choice model expects of the trip within the mode it kept
    /// (its probabilities over that mode's alternatives), for the trips in `paid` (I-al, S227).
    pub expected_paid: [f64; Mode::COUNT],
    /// Whether the model gave probabilities for every batch (else `expected_paid` is not
    /// measured).
    pub expected_measured: bool,
    /// Σ w · (1 − the probability the model gives the mode kept) over the trips choosing their
    /// mode that chose again: the mode changes chance alone would make (I-al's floor, S227).
    pub w_mode_floor: f64,
}

impl Assessment {
    /// The itinerary disequilibrium over every mode (I-al, S227): the gap less what the choice
    /// model itself expects, `Σ_m (paid_m − expected_m) / Σ_m least_m`, each trip against the best
    /// of the mode it kept. The plain pooled gap if the model gave no probabilities; NaN if
    /// nothing was assessed.
    pub(crate) fn gap_excess_pooled(&self) -> f64 {
        let least: f64 = self.least.iter().sum();
        if least <= 0.0 {
            return f64::NAN;
        }
        let paid: f64 = self.paid.iter().sum();
        if self.expected_measured {
            (paid - self.expected_paid.iter().sum::<f64>()) / least
        } else {
            (paid - least) / least
        }
    }

    /// The floor of [`Self::mode_changed_share`] (I-al, S227): the share whose mode chance alone
    /// would change among those who chose again; NaN if none chose before.
    pub(crate) fn mode_changed_floor(&self) -> f64 {
        if self.w_choosing > 0.0 && self.expected_measured {
            self.w_mode_floor / self.w_choosing
        } else {
            f64::NAN
        }
    }

    /// The relative gap of `mode`'s itinerary trips, NaN if none was assessed.
    pub(crate) fn gap(&self, mode: Mode) -> f64 {
        let (paid, least) = (self.paid[mode.index()], self.least[mode.index()]);
        if least > 0.0 { (paid - least) / least } else { f64::NAN }
    }

    /// The share of the trips choosing their mode whose mode changed, NaN if none chose
    /// before.
    pub(crate) fn mode_changed_share(&self) -> f64 {
        if self.w_choosing > 0.0 { self.w_mode_changed / self.w_choosing } else { f64::NAN }
    }
}

/// The itinerary trips of a run: which they are, their nodes, and the rounds
/// they are chosen in. With mode choice (M5), also every trip that chooses its mode.
pub(crate) struct Itineraries {
    /// The itinerary trips, in trip order.
    pub trips: Vec<TripId>,
    /// Per trip of the run: its position in `trips`, or `u32::MAX`.
    pos: Vec<u32>,
    nodes: Vec<TripNodes>,
    /// Positions by round.
    rounds: Vec<Vec<usize>>,
    /// Per position: whether the trip chooses its mode.
    choosing: Vec<bool>,
    /// The modes a choosing trip may be offered, by [`Mode::index`]: those the run asked for
    /// and can simulate.
    offered: [bool; Mode::COUNT],
    /// Per position, a choosing trip's bike and walk routes.
    legs: Vec<[Option<StaticLeg>; 2]>,
    /// Per position, a choosing trip's car pair (`None`: origin and destination on one node,
    /// or not choosing).
    car_keys: Vec<Option<RouteKey>>,
    /// Per position, its bike parkings in reach, found by the first plan that needs them and
    /// kept for the run: the bike layer's costs are static (S209).
    bike_reach: Vec<BikeReach>,
    /// Whether any trip may drive to a car park (stated park-and-ride, or offered it in mode
    /// choice): the car parks' free-flow table is then searched before planning (S209).
    parks_cars: bool,
    /// The modes each traveller class may use, by class index (S231); empty: every class all.
    class_modes: Vec<[bool; Mode::COUNT]>,
}

/// Where a vehicle is expected to be, following a traveller's choices.
#[derive(Clone, Copy, PartialEq)]
enum Loc {
    At(LonLat),
    Parked(u32),
    Nowhere,
}

/// The vehicle kind a mode's itinerary uses.
pub(crate) fn parking_kind_of(mode: Mode) -> Option<ParkingKind> {
    match mode {
        Mode::CarTransit => Some(ParkingKind::Car),
        Mode::BikeTransit => Some(ParkingKind::Bike),
        _ => None,
    }
}

/// The vehicle kind a mode moves, directly or through a parking.
fn vehicle_of(mode: Mode) -> Option<ParkingKind> {
    match mode {
        Mode::Car | Mode::CarTransit => Some(ParkingKind::Car),
        Mode::Bike | Mode::BikeTransit => Some(ParkingKind::Bike),
        Mode::Walk | Mode::Transit => None,
    }
}

/// What choosing needs besides the planner.
pub(crate) struct ChooseInputs<'a> {
    pub trips: &'a Trips,
    pub travellers: &'a Travellers,
    pub model: &'a dyn ChoiceModel,
    pub rng: &'a StreamRng,
    pub wanted: &'a [String],
    /// The user's link values (S236).
    pub link_values: &'a PreparedLinkValues,
    /// The run's prices (S248).
    pub prices: &'a PreparedPrices,
}

/// What a run can simulate, for [`Itineraries::new`].
pub(crate) struct Simulated<'a> {
    pub transit: Option<&'a TransitSetup>,
    pub parking: Option<&'a ParkingSetup>,
    pub road: &'a RoadNetwork,
    pub layers: &'a StaticLayers,
    /// The modes offered to a trip whose mode was not stated; `None` without mode choice.
    pub choice: Option<&'a [Mode]>,
    /// Every trip's bike and walk routes, a choosing trip's included.
    pub static_routes: &'a StaticRoutes,
    /// How long a walk or ride mode choice offers (S209).
    pub modes: &'a crate::layers::ModeDefaults,
    /// The modes each traveller class may use, by class index (S231); empty: every class all.
    pub class_modes: &'a [[bool; Mode::COUNT]],
    /// Each traveller class's own limits, by class index (S235); empty: the run's.
    pub class_limits: &'a [crate::layers::ClassLimits],
    /// The travellers, for each trip's class.
    pub travellers: &'a Travellers,
}

impl Itineraries {
    /// The run's trips that are itinerary trips, given what it can simulate:
    /// transit trips if it has a timetable; park-and-ride and bike-and-ride if it
    /// also has parkings (and, for bikes, the bike layer); and, with mode choice, every
    /// trip whose mode was not stated.
    pub(crate) fn new(trips: &Trips, sim: &Simulated<'_>) -> Self {
        let total = trips.len() as usize;
        let (transit, parking, road) = (sim.transit, sim.parking, sim.road);
        let bike = sim.layers.bike.as_ref();
        let simulated = |mode: Mode| match mode {
            Mode::Transit => transit.is_some(),
            Mode::CarTransit => transit.is_some() && parking.is_some(),
            Mode::BikeTransit => transit.is_some() && parking.is_some() && bike.is_some(),
            Mode::Bike => bike.is_some(),
            Mode::Walk => sim.layers.walk.is_some(),
            Mode::Car => true,
        };
        let mut offered = [false; Mode::COUNT];
        for &m in sim.choice.unwrap_or(&[]) {
            offered[m.index()] = simulated(m);
        }
        let road_snapper = NodeSnapper::new(road);
        let bike_snapper = bike.map(|b| NodeSnapper::every_node(b.network().network()));
        let null = NodeId::from_raw(NULL_ID);
        let mut out = Self {
            trips: Vec::new(),
            pos: vec![u32::MAX; total],
            nodes: Vec::new(),
            rounds: Vec::new(),
            choosing: Vec::new(),
            offered,
            legs: Vec::new(),
            car_keys: Vec::new(),
            bike_reach: Vec::new(),
            parks_cars: false,
            class_modes: sim.class_modes.to_vec(),
        };
        for i in 0..total {
            let trip = TripId::from_index(i);
            let mode = trips.mode(trip);
            let choosing = sim.choice.is_some() && !trips.mode_given(trip);
            let itinerary = matches!(mode, Mode::Transit | Mode::CarTransit | Mode::BikeTransit);
            if !(choosing || itinerary && simulated(mode)) {
                continue;
            }
            let (o, d) = (trips.origin(trip), trips.destination(trip));
            // The traveller's class's limits (S235), the run's where it gives none.
            let limits = crate::layers::class_limits(
                sim.class_limits,
                sim.travellers.user_class(trips.traveller(trip)).index(),
            );
            let mut nodes = TripNodes {
                walk_o: transit.map_or(null, |t| t.walk_node(o)),
                walk_d: transit.map_or(null, |t| t.walk_node(d)),
                road_o: null,
                road_d: null,
                bike_o: null,
                bike_d: null,
                access_walk_s: limits
                    .access_walk_max_s
                    .unwrap_or_else(|| transit.map_or(0.0, |t| t.defaults().access_walk_max_s)),
            };
            if choosing || mode == Mode::CarTransit {
                nodes.road_o = road_snapper.nearest(road, o);
                nodes.road_d = road_snapper.nearest(road, d);
            }
            if choosing || mode == Mode::BikeTransit {
                if let (Some(b), Some(snapper)) = (bike, &bike_snapper) {
                    let graph = b.network().network();
                    nodes.bike_o = snapper.nearest(graph, o);
                    nodes.bike_d = snapper.nearest(graph, d);
                }
            }
            let (legs, car_key) = if choosing {
                let max = limits.modes(sim.modes);
                let leg =
                    |layer| sim.static_routes.leg(sim.layers, trip, layer, max.max_seconds(layer));
                let key = (offered[Mode::Car.index()] && nodes.road_o != nodes.road_d)
                    .then(|| RouteKey::new(nodes.road_o, nodes.road_d));
                ([leg(StaticLayer::Bike), leg(StaticLayer::Walk)], key)
            } else {
                ([None, None], None)
            };
            out.pos[i] = u32::try_from(out.trips.len()).expect("trips fit u32");
            out.trips.push(trip);
            out.nodes.push(nodes);
            out.choosing.push(choosing);
            out.legs.push(legs);
            out.car_keys.push(car_key);
        }
        // Rounds: plain transit trips first; each traveller's other itinerary trips one
        // round each, in order, since each may move a vehicle the next one needs.
        let mut rounds: Vec<Vec<usize>> = vec![Vec::new()];
        let mut per_traveller: std::collections::HashMap<u32, usize> =
            std::collections::HashMap::new();
        for (p, &trip) in out.trips.iter().enumerate() {
            if !out.choosing[p] && trips.mode(trip) == Mode::Transit {
                rounds[0].push(p);
            } else {
                let r = per_traveller.entry(trips.traveller(trip).raw()).or_insert(0);
                if rounds.len() <= *r {
                    rounds.push(Vec::new());
                }
                rounds[*r].push(p);
                *r += 1;
            }
        }
        out.rounds = rounds;
        out.bike_reach = (0..out.trips.len()).map(|_| OnceLock::new()).collect();
        out.parks_cars = parking.is_some()
            && out.trips.iter().zip(&out.choosing).any(|(&trip, &choosing)| {
                trips.mode(trip) == Mode::CarTransit
                    || (choosing && offered[Mode::CarTransit.index()])
            });
        out
    }

    /// Whether there are none.
    pub(crate) fn is_empty(&self) -> bool {
        self.trips.is_empty()
    }

    /// The position of a trip, if it is an itinerary trip.
    pub(crate) fn position(&self, trip: TripId) -> Option<usize> {
        let p = self.pos[trip.index()];
        (p != u32::MAX).then_some(p as usize)
    }

    /// A trip's nodes.
    pub(crate) fn nodes(&self, position: usize) -> &TripNodes {
        &self.nodes[position]
    }

    /// The car pair of the trip at `position` if it chooses its mode and may drive, for
    /// its route set.
    pub(crate) fn car_key(&self, position: usize) -> Option<RouteKey> {
        self.car_keys[position]
    }

    /// Where the traveller's vehicle of `kind` is expected as `trip` starts, following
    /// their stated modes and their choices so far (`chosen`).
    fn vehicle_at(
        &self,
        trips: &Trips,
        travellers: &Travellers,
        trip: TripId,
        kind: ParkingKind,
        chosen: &[Option<Alternative>],
    ) -> Loc {
        let traveller = trips.traveller(trip);
        let own = travellers.ownership(traveller);
        let owns = match kind {
            ParkingKind::Car => own.car,
            ParkingKind::Bike => own.bike,
        };
        if !owns {
            return Loc::Nowhere;
        }
        let mut loc = Loc::Nowhere;
        for (i, t) in travellers.trips_of(traveller).enumerate() {
            if i == 0 {
                loc = Loc::At(trips.origin(t));
            }
            if t == trip {
                break;
            }
            // What trip `t` did: its choice, or its stated car or bike trip.
            let (mode, shape, parking) = match self.position(t) {
                Some(p) => match &chosen[p] {
                    Some(a) => (a.mode, a.shape, a.parking),
                    None => continue,
                },
                None => match trips.mode(t) {
                    m @ (Mode::Car | Mode::Bike) => (m, Shape::Direct, NO_PARKING),
                    _ => continue,
                },
            };
            if vehicle_of(mode) != Some(kind) {
                continue;
            }
            let at_origin = loc == Loc::At(trips.origin(t));
            match shape {
                Shape::Direct | Shape::Transit if at_origin => loc = Loc::At(trips.destination(t)),
                Shape::Out if at_origin => loc = Loc::Parked(parking),
                Shape::Back(p) if loc == Loc::Parked(p) => loc = Loc::At(trips.destination(t)),
                _ => {}
            }
        }
        loc
    }

    /// What the trip at `position` is offered, following the traveller's choices so far
    /// (`chosen`): its stated mode where its vehicle is, or, choosing, every mode it can use
    /// there.
    fn offer_of(
        &self,
        trips: &Trips,
        travellers: &Travellers,
        position: usize,
        chosen: &[Option<Alternative>],
    ) -> Offer {
        let trip = self.trips[position];
        let origin = trips.origin(trip);
        let shape = |kind| match self.vehicle_at(trips, travellers, trip, kind, chosen) {
            Loc::Parked(p) => Some(Shape::Back(p)),
            Loc::At(o) if o == origin => Some(Shape::Out),
            _ => None,
        };
        if !self.choosing[position] {
            return match trips.mode(trip) {
                Mode::CarTransit => {
                    Offer { car_transit: shape(ParkingKind::Car), ..Offer::default() }
                }
                Mode::BikeTransit => {
                    Offer { bike_transit: shape(ParkingKind::Bike), ..Offer::default() }
                }
                _ => Offer { transit: true, ..Offer::default() },
            };
        }
        // A mode the run offers and the traveller's class may use (S231).
        let class = travellers.user_class(trips.traveller(trip)).index();
        let on = |m: Mode| {
            self.offered[m.index()] && self.class_modes.get(class).is_none_or(|c| c[m.index()])
        };
        let (car, bike) = (shape(ParkingKind::Car), shape(ParkingKind::Bike));
        // A vehicle parked at a parking is offered back whatever the modes asked for.
        let via = |at: Option<Shape>, m: Mode| match at {
            Some(Shape::Back(p)) => Some(Shape::Back(p)),
            Some(Shape::Out) if on(m) => Some(Shape::Out),
            _ => None,
        };
        let [bike_leg, walk_leg] = &self.legs[position];
        Offer {
            choosing: true,
            walk: on(Mode::Walk) && walk_leg.is_some(),
            bike: on(Mode::Bike) && bike == Some(Shape::Out) && bike_leg.is_some(),
            car: on(Mode::Car) && car == Some(Shape::Out) && self.car_keys[position].is_some(),
            transit: on(Mode::Transit),
            car_transit: via(car, Mode::CarTransit),
            bike_transit: via(bike, Mode::BikeTransit),
            parking: None,
        }
    }

    /// Choose: every trip (`current` `None`, iteration 0), or, after a loading,
    /// the travellers `strategy` picks to choose again and those whose
    /// alternative is no longer on offer, the rest keeping theirs at the new
    /// costs. Returns the choices and what the pass found.
    ///
    /// **The gap is sampled** (S210, design §11.2): with a `strategy`, the travellers it picks to
    /// choose again plan their whole choice set, and so does a keyed sample of the others
    /// ([`Equilibration::itinerary_gap_sample`] trips, drawn again each iteration); both are
    /// random samples, and the gap is measured on them. A trip outside both that keeps an
    /// itinerary out through a parking is **re-costed** at that parking (its search and query
    /// only), and is asked to choose again only if its itinerary is gone there. Without a
    /// `strategy` (the run's last assessment) every trip plans in full: the last gap is exact.
    ///
    /// **With `only`** (by traveller; S223, a group of the free-flow loading's increments) the
    /// marked travellers choose again and the others are neither planned nor assessed: they
    /// keep their alternatives as they are.
    ///
    /// # Errors
    ///
    /// [`ChoiceError`] if the model fails or answers wrongly.
    #[allow(clippy::too_many_lines, reason = "one pass: plan, keep or choose, assess")]
    pub(crate) fn choose<'a>(
        &self,
        planner: &Planner<'a>,
        inputs: &ChooseInputs<'_>,
        current: Option<&Chosen>,
        iteration: u32,
        strategy: Option<(&dyn Equilibration, &StreamRng)>,
        only: Option<&[bool]>,
    ) -> Result<(Chosen, Assessment), ChoiceError> {
        let n = self.trips.len();
        let mut next = Chosen {
            alt: current.map_or_else(|| vec![None; n], |c| c.alt.clone()),
            probability: current.map_or_else(|| vec![f64::NAN; n], |c| c.probability.clone()),
            alternatives: current.map_or_else(|| vec![0; n], |c| c.alternatives.clone()),
            logsum: current.map_or_else(|| vec![f64::NAN; n], |c| c.logsum.clone()),
        };
        let mut assessment = Assessment { expected_measured: true, ..Assessment::default() };
        let (trips, travellers) = (inputs.trips, inputs.travellers);
        // The car parks' free-flow table, searched here, outside the parallel planning (S209).
        if let (true, Some(parking)) = (self.parks_cars, planner.parking) {
            parking.prepare_car_free_flow();
        }
        // With `only` (S223: one group of the free-flow loading's increments), its travellers
        // choose again and no one else is planned or assessed.
        let marked = |p: usize| only.is_none_or(|o| o[trips.traveller(self.trips[p]).index()]);
        let reselects = |p: usize| {
            let traveller = trips.traveller(self.trips[p]).raw();
            only.is_some_and(|_| marked(p))
                || strategy
                    .is_some_and(|(strategy, msa)| strategy.reselects(msa, traveller, iteration))
        };
        // Who plans in full to measure the gap: everyone without a strategy, else a keyed
        // sample of travellers (its own draw address, apart from the reselection's).
        #[allow(clippy::cast_precision_loss, reason = "a share")]
        let share = strategy.map_or(1.0, |(strategy, _)| {
            f64::from(strategy.itinerary_gap_sample()) / n.max(1) as f64
        });
        let in_sample = |p: usize| {
            share >= 1.0
                || strategy.is_some_and(|(_, rng)| {
                    let traveller = trips.traveller(self.trips[p]).raw();
                    rng.unit(DrawAddress::from_pair(traveller, iteration | GAP_SAMPLE_SALT)) < share
                })
        };
        let plan = |s: &mut Scratch<'a>, p: usize, offer: Offer| {
            if offer.is_empty() {
                return Vec::new();
            }
            let trip = self.trips[p];
            let [bike, walk] = &self.legs[p];
            planner.plan(
                s,
                &self.nodes[p],
                trips.departure(trip).get(),
                trips.origin(trip),
                trips.destination(trip),
                offer,
                (bike.as_ref(), walk.as_ref()),
                self.car_keys[p],
                self.bike_reach.get(p),
            )
        };
        for round in &self.rounds {
            // What each trip is offered, from the choices before it. A trip that keeps its
            // alternative plans only the mode it took (M5, for compute: enough to find it
            // again and to measure it against the best of its mode); one that chooses, or
            // must fetch a vehicle, plans all.
            let work: Vec<(usize, Offer)> = round
                .iter()
                .filter(|&&p| marked(p))
                .map(|&p| (p, self.offer_of(trips, travellers, p, &next.alt)))
                .collect();
            let first: Vec<Offer> = work
                .iter()
                .map(|&(p, offer)| match current.and_then(|c| c.alt[p].as_ref()) {
                    Some(old) if !reselects(p) && !offer.fetches() => {
                        let only = offer.only(old.mode);
                        if old.shape == Shape::Out && !in_sample(p) {
                            only.at_parking(old.parking)
                        } else {
                            only
                        }
                    }
                    _ => offer,
                })
                .collect();
            let jobs: Vec<(usize, Offer)> =
                work.iter().zip(&first).map(|(&(p, _), &offer)| (p, offer)).collect();
            let mut sets: Vec<Vec<Alternative>> =
                par_map(&jobs, SCRATCH_CHUNK, || planner.scratch(), |s, &(p, o)| plan(s, p, o));
            // Those whose alternative is gone must choose again: among all they are offered.
            let again: Vec<usize> = (0..work.len())
                .filter(|&w| {
                    first[w] != work[w].1
                        && current
                            .and_then(|c| c.alt[work[w].0].as_ref())
                            .is_some_and(|old| !sets[w].iter().any(|a| a.identity == old.identity))
                })
                .collect();
            let redo: Vec<(usize, Offer)> = again.iter().map(|&w| work[w]).collect();
            let replanned =
                par_map(&redo, SCRATCH_CHUNK, || planner.scratch(), |s, &(p, o)| plan(s, p, o));
            for (w, set) in again.into_iter().zip(replanned) {
                sets[w] = set;
            }
            for chunk in (0..work.len()).collect::<Vec<_>>().chunks(CHUNK) {
                let names: Vec<&str> = inputs.wanted.iter().map(String::as_str).collect();
                let mut batch = ChoiceBatch::new(iteration, &names);
                let mut situation_of: Vec<usize> = Vec::new();
                let mut row = vec![0.0; inputs.wanted.len()];
                let mut movers: Vec<usize> = Vec::new();
                // (situation, mode index, weight): trips in the gap, and the choosing ones that
                // choose again, whose expectation and floor need the model's probabilities.
                let mut in_gap: Vec<(usize, usize, f64)> = Vec::new();
                let mut floor: Vec<(usize, usize, f64)> = Vec::new();
                for &w in chunk {
                    let (p, _) = work[w];
                    let set = &sets[w];
                    let trip = self.trips[p];
                    let traveller = trips.traveller(trip);
                    let weight = f64::from(travellers.weight(traveller));
                    let old = current.and_then(|c| c.alt[p].as_ref());
                    if self.choosing[p] && old.is_some() {
                        assessment.w_choosing += weight;
                    }
                    if set.is_empty() {
                        if self.choosing[p] && old.is_some() {
                            assessment.w_mode_changed += weight;
                        }
                        next.alt[p] = None;
                        next.alternatives[p] = 0;
                        continue;
                    }
                    let best = best_by_mode(set);
                    let class = travellers.user_class(traveller).raw();
                    batch.begin_situation_in(traveller.raw(), trip.raw(), class);
                    for a in set {
                        for (slot, name) in row.iter_mut().zip(inputs.wanted) {
                            *slot = attribute(
                                name,
                                a,
                                best[a.mode.index()],
                                inputs.link_values,
                                inputs.prices,
                            );
                        }
                        batch.push_alternative(a.identity, &row);
                    }
                    let s = situation_of.len();
                    situation_of.push(w);
                    // Keep, or choose again.
                    let kept = old.and_then(|old| set.iter().find(|a| a.identity == old.identity));
                    let Some(kept) = kept else {
                        if current.is_some() {
                            assessment.w_forced += weight;
                        }
                        movers.push(s);
                        continue;
                    };
                    // Against the best of its own mode: with mode constants, the fastest mode
                    // need not be the best (M5), so this is each mode's route gap. A trip
                    // re-costed at its parking saw no other: not in the gap's sample.
                    let m = kept.mode.index();
                    if first[w].parking.is_none() {
                        assessment.paid[m] += weight * kept.total_s;
                        assessment.least[m] += weight * best[m];
                        in_gap.push((s, m, weight));
                    } else {
                        assessment.recosted += 1;
                    }
                    assessment.w_all += weight;
                    let reselect = strategy.is_some_and(|(strategy, msa)| {
                        strategy.reselects(msa, traveller.raw(), iteration)
                    });
                    if reselect {
                        assessment.w_reselected += weight;
                        movers.push(s);
                        if self.choosing[p] {
                            floor.push((s, m, weight));
                        }
                    } else {
                        next.alt[p] = Some(kept.clone());
                    }
                }
                // Each planned trip's logsum, on this iteration's costs (S238).
                if batch.situations() > 0 {
                    batch.validate()?;
                    if let Some(logsums) = inputs.model.logsums(&batch)? {
                        for (s, &w) in situation_of.iter().enumerate() {
                            next.logsum[work[w].0] = logsums[s];
                        }
                    }
                }
                if !(in_gap.is_empty() && floor.is_empty()) {
                    batch.validate()?;
                    match inputs.model.probabilities(&batch)? {
                        Some(prob) => {
                            // A situation's alternatives are its set's, in order.
                            let within = |s: usize, m: usize| {
                                let start = batch.range(s).start;
                                let (mut p_m, mut t_m) = (0.0, 0.0);
                                for (i, a) in sets[situation_of[s]].iter().enumerate() {
                                    if a.mode.index() == m {
                                        p_m += prob[start + i];
                                        t_m += prob[start + i] * a.total_s;
                                    }
                                }
                                (p_m, t_m)
                            };
                            for &(s, m, weight) in &in_gap {
                                let (p_m, t_m) = within(s, m);
                                if p_m > 0.0 {
                                    assessment.expected_paid[m] += weight * t_m / p_m;
                                }
                            }
                            for &(s, m, weight) in &floor {
                                assessment.w_mode_floor += weight * (1.0 - within(s, m).0);
                            }
                        }
                        None => assessment.expected_measured = false,
                    }
                }
                if movers.is_empty() {
                    continue;
                }
                batch.validate()?;
                let sub = batch.subset(&movers);
                let choices = inputs.model.choose(&sub, inputs.rng)?;
                choices.validate(&sub)?;
                for (m, &s) in movers.iter().enumerate() {
                    let w = situation_of[s];
                    let (p, _) = work[w];
                    let alt = sets[w][choices.chosen[m] as usize].clone();
                    let old = current.and_then(|c| c.alt[p].as_ref());
                    let weight = f64::from(travellers.weight(trips.traveller(self.trips[p])));
                    if old.is_some_and(|old| old.identity != alt.identity) {
                        assessment.w_changed += weight;
                    }
                    if self.choosing[p] && old.is_some_and(|old| old.mode != alt.mode) {
                        assessment.w_mode_changed += weight;
                    }
                    next.alternatives[p] = u32::try_from(sets[w].len()).expect("few alternatives");
                    next.alt[p] = Some(alt);
                    next.probability[p] = choices.probability[m];
                }
            }
        }
        Ok((next, assessment))
    }
}

/// Each itinerary trip's choice and how it went, for the results (M4).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ItineraryResult {
    /// The itinerary trips, in trip order.
    pub trip: Vec<u32>,
    /// The parking each used ([`NO_PARKING`] for plain transit, or with no choice).
    pub parking: Vec<u32>,
    /// Whether it went out (vehicle first), back (transit first) or neither: 1, 2, 0.
    pub direction: Vec<u8>,
    /// How many alternatives it had when it last chose (0: none; it did not travel).
    pub alternatives: Vec<u32>,
    /// The probability the model gave its choice (`NaN` if none).
    pub probability: Vec<f64>,
    /// Its expected door-to-door time at the choice, in seconds (`NaN` if none).
    pub expected_s: Vec<f64>,
    /// Vehicles it boarded in the last loading.
    pub rides: Vec<u32>,
    /// Trips whose chosen line could not be followed, in the last loading.
    pub replanned: u32,
    /// Trips back: the mean absolute difference between the expected and the
    /// realised arrival at the parking, in seconds (A15; `NaN` if none).
    pub return_mismatch_s: f64,
    /// The mode of the alternative taken ([`Mode::index`]; [`NO_MODE`] if none).
    pub mode: Vec<u8>,
    /// Whether the trip chose its mode (M5), or was given it.
    pub choosing: Vec<bool>,
    /// The logsum of its choice set (S238; `NaN` if the model gives none).
    pub logsum: Vec<f64>,
}

/// No mode: a trip that had no alternative.
pub const NO_MODE: u8 = u8::MAX;

impl ItineraryResult {
    pub(crate) fn of(
        itineraries: &Itineraries,
        chosen: &Chosen,
        rides: &[u32],
        replanned: u32,
        return_mismatch_s: f64,
    ) -> Self {
        let n = itineraries.trips.len();
        let mut out = Self {
            trip: itineraries.trips.iter().map(|t| t.raw()).collect(),
            parking: Vec::with_capacity(n),
            direction: Vec::with_capacity(n),
            alternatives: chosen.alternatives.clone(),
            probability: chosen.probability.clone(),
            expected_s: Vec::with_capacity(n),
            rides: if rides.len() == n { rides.to_vec() } else { vec![0; n] },
            replanned,
            return_mismatch_s,
            mode: Vec::with_capacity(n),
            choosing: itineraries.choosing.clone(),
            logsum: chosen.logsum.clone(),
        };
        for alt in &chosen.alt {
            #[allow(clippy::cast_possible_truncation, reason = "six modes")]
            out.mode.push(alt.as_ref().map_or(NO_MODE, |a| a.mode.index() as u8));
            out.parking.push(alt.as_ref().map_or(NO_PARKING, |a| a.parking));
            out.direction.push(match alt.as_ref().map(|a| a.shape) {
                Some(Shape::Out) => 1,
                Some(Shape::Back(_)) => 2,
                _ => 0,
            });
            out.expected_s.push(alt.as_ref().map_or(f64::NAN, |a| a.total_s));
        }
        out
    }
}

/// Where a walk of an executed itinerary starts or ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WalkEnd {
    /// The transit part's start: the origin, or the parking out.
    Start,
    /// Its end: the destination, or the parking back.
    End,
    /// A stop.
    Stop(NodeId),
}

/// One walk of an executed itinerary.
#[derive(Clone, Copy, Debug)]
pub(crate) struct WalkSeg {
    pub from: WalkEnd,
    pub to: WalkEnd,
    pub departure: u32,
}

/// An itinerary's transit part as executed.
#[derive(Clone, Debug)]
pub(crate) struct Followed {
    /// The second it ends (at the destination, or the parking back).
    pub end: u32,
    /// The rides taken.
    pub rides: Vec<JourneyLeg>,
    /// The walks.
    pub walks: Vec<WalkSeg>,
    /// Whether a chosen line could not be followed and the earliest journey from
    /// there was taken instead.
    pub replanned: bool,
}

/// Execute `alt`'s transit part on `data`, the traveller at its first stop at
/// `at_first_stop`: each chosen line's first catchable run; where one cannot be
/// followed, the earliest journey from there to `egress` (`egress()` makes it
/// only then). `None` if that fails too.
pub(crate) fn follow(
    data: &RaptorData,
    raptor: &mut Raptor<'_>,
    alt: &Alternative,
    at_first_stop: u32,
    egress: impl FnOnce() -> Vec<(NodeId, u32)>,
) -> Option<Followed> {
    let mut out = Followed {
        end: 0,
        rides: Vec::with_capacity(alt.rides.len()),
        walks: vec![WalkSeg {
            from: WalkEnd::Start,
            to: WalkEnd::Stop(alt.first_stop),
            departure: at_first_stop.saturating_sub(alt.access_walk_s),
        }],
        replanned: false,
    };
    let mut clock = at_first_stop;
    let mut at = alt.first_stop;
    for (i, ride) in alt.rides.iter().enumerate() {
        if i > 0 {
            let walk = alt.transfer_walks[i - 1];
            if ride.board != at {
                out.walks.push(WalkSeg {
                    from: WalkEnd::Stop(at),
                    to: WalkEnd::Stop(ride.board),
                    departure: clock,
                });
            }
            clock = clock.saturating_add(walk);
            at = ride.board;
        }
        if let Some(leg) = data.ride_line(ride.route, ride.board, ride.alight, clock) {
            if let JourneyLeg::Ride { arrival, .. } = leg {
                clock = arrival;
            }
            out.rides.push(leg);
            at = ride.alight;
            continue;
        }
        // The line cannot be followed: the earliest journey from here.
        let j = raptor.earliest(&[(at, clock)], &egress())?;
        out.replanned = true;
        let mut t = clock;
        for leg in &j.legs {
            match *leg {
                JourneyLeg::Ride { arrival, .. } => {
                    out.rides.push(*leg);
                    t = arrival;
                }
                JourneyLeg::Transfer { from, to, seconds } => {
                    out.walks.push(WalkSeg {
                        from: WalkEnd::Stop(from),
                        to: WalkEnd::Stop(to),
                        departure: t,
                    });
                    t = t.saturating_add(seconds);
                }
                JourneyLeg::Egress { stop, .. } => {
                    out.walks.push(WalkSeg {
                        from: WalkEnd::Stop(stop),
                        to: WalkEnd::End,
                        departure: t,
                    });
                }
                JourneyLeg::Access { .. } => {}
            }
        }
        out.end = j.arrival;
        return Some(out);
    }
    out.walks.push(WalkSeg {
        from: WalkEnd::Stop(alt.last_stop),
        to: WalkEnd::End,
        departure: clock,
    });
    out.end = clock.saturating_add(alt.egress_walk_s);
    Some(out)
}
