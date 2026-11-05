//! Transit legs on the scheduled timetable: the journeys of a trip, each a chain of its set.
//!
//! **What a transit leg is.** A trip whose mode is stated as transit leaves its origin at its
//! departure, walks on the walk layer to a stop, rides one or more runs of the timetable, with a
//! wait before each boarding and perhaps a walk between two stops in a change, and walks from the
//! last stop to its destination. The whole of it is one [`Leg`] of mode transit between the trip's
//! ends snapped to the walk layer, its [rides](Ride) listed, using no link (the walks are timed on
//! the walk layer but their links are not kept).
//!
//! **The journeys.** The stops a trip can walk to are those within
//! [`TransitDefaults::access_walk_max_s`] of its origin on the walk layer, and the stops it can
//! walk from those within the same limit of its destination (each walk counted in whole seconds,
//! with [`TransitDefaults::stop_transfer_s`] added at the stop). A stop is reached on foot only
//! if the walk layer has a node within [`TransitDefaults::stop_walk_snap_m`] of it; walks between
//! stops in a change are those within [`TransitDefaults::transfer_walk_max_s`] (the feed's
//! minimum transfer times replace them where it gives them). The journeys are RAPTOR's Pareto
//! set from those stops at the trip's departure (`core-transit`'s [`Raptor::pareto_before`]):
//! for each number of vehicles, the earliest arrival, kept if it is earlier than every journey
//! with fewer vehicles. **Every one of them is a chain of the trip's set**, except:
//!
//! - a journey that leaves from and returns to the same stop (a ride out and back, which RAPTOR
//!   can offer where both ends are near one stop; nobody takes it);
//! - a journey longer door to door than [`TransitDefaults::journey_max_s`];
//! - a journey whose walking breaks a walking limit ([`walking_within_limits`]: at most the
//!   access limit from the origin, the same to the destination, and at most
//!   [`Transit::walk_total_max_seconds`] in all, the walks between stops included).
//!
//! The timetable is static (no bus rides the roads here), so a trip's journeys at its departure
//! never change: they are planned **once**, before the first iteration, as bike and walking legs
//! are, and never re-priced to other numbers.
//!
//! **Attributes.** A journey's door-to-door time is its arrival less the trip's departure, in
//! whole seconds, and splits exactly into walking (to the first stop, between stops, from the
//! last), waiting (from reaching a stop to the departure of the run boarded there, the boarding
//! slack included) and in-vehicle time (each run from boarding to alighting). Its transfers are
//! its rides less one; its money its fare ([`Fares`]: a price per journey, and per ride its
//! kind of service's price per kilometre times the kilometres from its boarding stop to its
//! alighting stop as the crow flies), 0 for a traveller holding a transit pass; its length the
//! rides' distances between their stops as the crow flies.
//!
//! **Fare as a third criterion.** RAPTOR's Pareto set is over arrival and vehicles: where the
//! kinds of service have different prices per kilometre, a slower or less direct journey on
//! cheaper services can be worth offering. The search is not changed; for a traveller who pays,
//! the trip has **one more query per cheaper price level**: for each price per kilometre below
//! the dearest (among the kinds the timetable's lines have), the Pareto set on the view of the
//! timetable that keeps only the runs of the kinds priced at most that much. The journeys of
//! every query together, each with its exact fare, are kept where no other journey is as early,
//! with as few vehicles and as cheap, and better in one of the three (of journeys equal in all
//! three, the first found: the whole timetable's, then the cheaper views' in order). When every
//! kind has the same price per kilometre (by default every price is 0), and for a traveller with
//! a pass, there is no other query and the set is RAPTOR's.
//!
//! **Path size.** The chains of one trip share rides, not links: [`ride_path_sizes`] gives each
//! transit chain its path-size factor over the trip's transit chains by time share, as the other
//! engine documents for its itineraries: each ride (a line between two stops) is an element the
//! chains may share, weighed by its seconds; walking and waiting are each the chain's own.
//!
//! **The load stage.** A transit leg loads nothing on the road network: it is ridden on the
//! timetable's times, exactly as planned (the runs keep to the timetable), so the trip ends at
//! its journey's arrival. [`boardings`] counts the travellers boarding and alighting at each call
//! of the timetable from the chains taken.
//!
//! **Cost:** per stop a walk-layer node and its walks to the stops within the transfer limit;
//! per distinct origin and destination node one bounded search on the walk layer; per transit
//! trip one RAPTOR query, once per run (and with fares by kind of service, one more per cheaper
//! price level, on a view of the timetable built once).

use std::collections::HashMap;

use openmobisim_core_demand::{Mode, Travellers, Trips};
use openmobisim_core_graph::geometry::{LonLat, ground_distance_metres};
use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_routes::{NodeSnapper, Reach};
use openmobisim_core_transit::{
    Footpaths, Journey, JourneyLeg, Raptor, RaptorData, ServiceKind, Timetable, TransitDefaults,
};
use openmobisim_core_types::ids::{EntityId, NULL_ID, NodeId, TripId, UserClassId};

use crate::attributes::Attributes;
use crate::chain::{Chain, FixedTimes, Leg, Place, Ride};
use crate::error::AssignError;
use crate::layer_planner::Layer;
use crate::planner::{CostSource, LegPlanner};
use crate::state::State;

/// The timetable a run's transit trips ride, and its limits.
#[derive(Clone, Copy, Debug)]
pub struct Transit<'a> {
    /// The timetable (its scheduled times are the times ridden).
    pub timetable: &'a Timetable,
    /// Boarding slack, walking limits, the longest journey and the most vehicles.
    pub defaults: TransitDefaults,
    /// The most walking a journey may have in all, in seconds: to the first stop, between stops
    /// and from the last (3 600 by default, the user's ruling of 2026-11-05: *an assumption*,
    /// a choice-set limit, not a preference; the walking minutes are weighed by the choice model).
    pub walk_total_max_seconds: f64,
    /// What a journey costs a traveller without a transit pass ([`Fares::FREE`] by default).
    pub fares: Fares,
}

/// The longest walk in a journey in all, by default, in seconds.
pub const WALK_TOTAL_MAX_SECONDS: f64 = 3600.0;

impl<'a> Transit<'a> {
    /// `timetable` with `defaults`, the default longest walk in all and no fare.
    #[must_use]
    pub fn new(timetable: &'a Timetable, defaults: TransitDefaults) -> Self {
        Self {
            timetable,
            defaults,
            walk_total_max_seconds: WALK_TOTAL_MAX_SECONDS,
            fares: Fares::FREE,
        }
    }
}

/// A journey's fare, in euros, for a traveller without a transit pass: [`Self::base_eur`] once
/// per journey, and for each ride the price per kilometre of its line's kind of service
/// ([`Self::km_eur`]) times the ride's kilometres, from its boarding stop to its alighting stop
/// as the crow flies. A traveller holding a transit pass pays 0.
///
/// *Uncalibrated: every price 0 by default (paying, like holding a pass, is access, which the
/// traveller classes give). A city's own: the Netherlands charge one base fare and one rate per
/// kilometre for bus, tram and metro and another for trains; Amsterdam's GVB a base of 1.16 € and
/// 0.217 € a kilometre in 2026 (Vervoerregio Amsterdam, note BBV/2025/15361, p. 2,
/// <https://vervoerregio.bestuurlijkeinformatie.nl/Document/View/abddeeb0-be54-4eae-9622-4ac0935df151>).*
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fares {
    /// The price of a journey, once, in euros.
    pub base_eur: f64,
    /// The price of a kilometre ridden, in euros, by kind of service, in the order of
    /// [`ServiceKind::ALL`] (rail, metro, tram, bus, ferry, other).
    pub km_eur: [f64; 6],
}

impl Fares {
    /// No fare at all.
    pub const FREE: Self = Self { base_eur: 0.0, km_eur: [0.0; 6] };

    /// The price of a kilometre on a run of `kind`.
    #[must_use]
    pub fn km_eur_of(&self, kind: ServiceKind) -> f64 {
        self.km_eur[kind as usize]
    }

    /// Whether some kinds of `timetable`'s lines have another price per kilometre than others:
    /// only then does a journey's fare depend on more than its kilometres, and is it a third
    /// criterion of the journeys (see the [module docs](self)).
    #[must_use]
    pub fn differ_by_kind(&self, timetable: &Timetable) -> bool {
        self.levels(timetable).len() > 1
    }

    /// The distinct prices per kilometre of the kinds `timetable`'s lines have, cheapest first.
    fn levels(&self, timetable: &Timetable) -> Vec<f64> {
        let mut levels: Vec<f64> =
            (0..timetable.route_count()).map(|r| self.km_eur_of(timetable.route_kind(r))).collect();
        levels.sort_by(f64::total_cmp);
        levels.dedup();
        levels
    }
}

/// Whether a journey's walking is within the limits: the walk to the first stop (`access`) and
/// from the last (`egress`) each at most `access_max`, and every walk together (`transfers`, the
/// walks between stops, included) at most `total_max`; all in seconds. **The one rule on walking
/// in a journey** (see the [module docs](self)).
#[must_use]
pub fn walking_within_limits(
    access: f64,
    transfers: &[f64],
    egress: f64,
    access_max: f64,
    total_max: f64,
) -> bool {
    let total = access + transfers.iter().sum::<f64>() + egress;
    access <= access_max && egress <= access_max && total <= total_max
}

/// A stop and the walk to it (or from it), in whole seconds.
type StopWalk = (NodeId, u32);

/// Whole seconds of a walk, rounded down.
fn floor_seconds(s: f64) -> u32 {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "a walk's seconds")]
    let t = s.max(0.0).min(f64::from(u32::MAX - 1)).floor() as u32;
    t
}

/// The planner of transit legs: the timetable linked to the walk layer. See the
/// [module docs](self).
#[derive(Debug)]
pub struct TransitPlanner<'a> {
    timetable: &'a Timetable,
    transit: Transit<'a>,
    walk: &'a RoadNetwork,
    walk_seconds: Vec<f64>,
    snapper: NodeSnapper,
    /// Walk node → the stops there (CSR), and whether a node has a stop.
    node_stops_start: Vec<u32>,
    node_stops: Vec<NodeId>,
    has_stop: Vec<bool>,
    data: RaptorData,
    /// With fares by kind of service, per price per kilometre below the dearest (cheapest
    /// first), the timetable's runs of the kinds priced at most that much; empty otherwise.
    cheaper: Vec<RaptorData>,
}

impl<'a> TransitPlanner<'a> {
    /// The timetable of `transit` linked to `walk`, the walk layer.
    ///
    /// # Errors
    ///
    /// [`AssignError::Input`] if a walking limit is not a number of at least 0.
    ///
    /// # Panics
    ///
    /// If the timetable has more than `u32::MAX` stops.
    pub fn new(transit: Transit<'a>, walk: Layer<'a>) -> Result<Self, AssignError> {
        let d = transit.defaults;
        for (name, v) in [
            ("access_walk_max_s", d.access_walk_max_s),
            ("transfer_walk_max_s", d.transfer_walk_max_s),
            ("walk_total_max_s", transit.walk_total_max_seconds),
            ("journey_max_s", d.journey_max_s),
        ] {
            if v.is_nan() || v < 0.0 {
                return Err(AssignError::Input(format!(
                    "transit's {name} must be a number of seconds of at least 0, not {v}"
                )));
            }
        }
        let timetable = transit.timetable;
        let graph = walk.network.network();
        let walk_seconds = walk.network.link_seconds();
        let snapper = NodeSnapper::every_node(graph);
        let n = timetable.stop_count() as usize;
        // Each stop at the walk layer's nearest node, if within the snapping distance.
        let stop_walk: Vec<NodeId> = (0..n)
            .map(|s| {
                let at = timetable.stop_position(NodeId::from_index(s));
                let node = snapper.nearest(graph, at);
                if ground_distance_metres(at, graph.node_lonlat(node)) <= d.stop_walk_snap_m {
                    node
                } else {
                    NodeId::from_raw(NULL_ID)
                }
            })
            .collect();
        let nodes = graph.node_count() as usize;
        let mut per_node: Vec<Vec<NodeId>> = vec![Vec::new(); nodes];
        for (s, node) in stop_walk.iter().enumerate() {
            if !node.is_null() {
                per_node[node.index()].push(NodeId::from_index(s));
            }
        }
        let mut node_stops_start = Vec::with_capacity(nodes + 1);
        node_stops_start.push(0_u32);
        let mut node_stops = Vec::new();
        for list in &per_node {
            node_stops.extend_from_slice(list);
            node_stops_start.push(u32::try_from(node_stops.len()).expect("stops fit u32"));
        }
        let has_stop: Vec<bool> = per_node.iter().map(|l| !l.is_empty()).collect();
        // The walks between stops within the transfer limit; the feed's minimum transfer times
        // replace them where it gives them.
        let mut walks: HashMap<(u32, u32), u32> = HashMap::new();
        {
            let mut reach = Reach::new(graph, &walk_seconds);
            for (s, &from) in stop_walk.iter().enumerate() {
                if from.is_null() {
                    continue;
                }
                for (node, secs) in reach.forward(from, d.transfer_walk_max_s, &has_stop) {
                    let (a, b) = (
                        node_stops_start[node.index()] as usize,
                        node_stops_start[node.index() + 1] as usize,
                    );
                    for &to in &node_stops[a..b] {
                        if to.index() != s {
                            let s32 = u32::try_from(s).expect("stops fit u32");
                            walks.insert((s32, to.raw()), floor_seconds(secs));
                        }
                    }
                }
            }
        }
        for t in timetable.transfers() {
            if !stop_walk[t.from.index()].is_null() && !stop_walk[t.to.index()].is_null() {
                walks.insert((t.from.raw(), t.to.raw()), t.seconds);
            }
        }
        let mut list: Vec<(NodeId, NodeId, u32)> =
            walks.into_iter().map(|((a, b), s)| (NodeId::new(a), NodeId::new(b), s)).collect();
        list.sort_unstable();
        let footpaths = Footpaths::new(timetable.stop_count(), list);
        // The cheaper views, only when the kinds' prices per kilometre differ.
        let fares = transit.fares;
        let mut levels = fares.levels(timetable);
        levels.pop();
        let cheaper = levels
            .into_iter()
            .map(|level| {
                RaptorData::new_keeping(
                    timetable,
                    timetable.scheduled(),
                    footpaths.clone(),
                    d.board_slack_s,
                    |run| fares.km_eur_of(timetable.run_kind(run)) <= level,
                )
            })
            .collect();
        let data = RaptorData::new(timetable, timetable.scheduled(), footpaths, d.board_slack_s);
        Ok(Self {
            timetable,
            transit,
            walk: graph,
            walk_seconds,
            snapper,
            node_stops_start,
            node_stops,
            has_stop,
            data,
            cheaper,
        })
    }

    /// The timetable.
    #[must_use]
    pub fn timetable(&self) -> &'a Timetable {
        self.timetable
    }

    /// The place on the walk layer a point is snapped to: its nearest node.
    #[must_use]
    pub fn place(&self, point: LonLat) -> Place {
        Place::Walk(self.snapper.nearest(self.walk, point))
    }

    /// A search scratch for [`Self::walks`].
    pub(crate) fn reach(&self) -> Reach<'_> {
        Reach::new(self.walk, &self.walk_seconds)
    }

    /// The RAPTOR scratches for [`Self::legs`]: one on the whole timetable, then one per cheaper
    /// view (with fares by kind of service).
    pub(crate) fn raptors(&self) -> Vec<Raptor<'_>> {
        let rides = self.transit.defaults.max_rides as usize;
        std::iter::once(&self.data).chain(&self.cheaper).map(|d| Raptor::new(d, rides)).collect()
    }

    /// The stops within the access limit of walk node `node` (`backward`: the stops from which
    /// `node` is within it), each with the walk in whole seconds plus the stop's transfer time.
    pub(crate) fn walks(
        &self,
        reach: &mut Reach<'_>,
        node: NodeId,
        backward: bool,
    ) -> Vec<(NodeId, u32)> {
        let limit = self.transit.defaults.access_walk_max_s;
        let transfer = floor_seconds(self.transit.defaults.stop_transfer_s);
        // Searched one second further, so every walk counted in whole seconds up to the limit
        // is there.
        let found = if backward {
            reach.backward(node, limit + 1.0, &self.has_stop)
        } else {
            reach.forward(node, limit + 1.0, &self.has_stop)
        };
        let mut out = Vec::new();
        for (at, secs) in found {
            let w = floor_seconds(secs);
            if f64::from(w) <= limit {
                let (a, b) = (
                    self.node_stops_start[at.index()] as usize,
                    self.node_stops_start[at.index() + 1] as usize,
                );
                out.extend(self.node_stops[a..b].iter().map(|&s| (s, w.saturating_add(transfer))));
            }
        }
        out
    }

    /// Every transit leg of a trip from `from` to `to` leaving at `departure` (whole seconds),
    /// given the walks from its origin (`access`) and to its destination (`egress`, both from
    /// [`Self::walks`]): one per journey of the Pareto set that is kept (see the
    /// [module docs](self)), fewest vehicles first; its money the fare if the traveller `pays`
    /// (holds no transit pass), else 0. With fares by kind of service, for a traveller who pays,
    /// the journeys of each cheaper view are added after them, and only those no other journey
    /// dominates on arrival, vehicles and fare are kept ([`dominated`]).
    pub(crate) fn legs(
        &self,
        raptors: &mut [Raptor<'_>],
        (from, to): (Place, Place),
        departure: u32,
        (access, egress): (&[StopWalk], &[StopWalk]),
        pays: bool,
    ) -> Vec<Leg> {
        if access.is_empty() || egress.is_empty() {
            return Vec::new();
        }
        let at: Vec<(NodeId, u32)> =
            access.iter().map(|&(s, w)| (s, departure.saturating_add(w))).collect();
        let d = self.transit.defaults;
        // The whole timetable's journeys; and each cheaper view's for a traveller who pays.
        let views = if pays { raptors.len() } else { 1 };
        let mut found: Vec<(u32, Leg)> = Vec::new();
        for raptor in &mut raptors[..views] {
            found.extend(
                raptor
                    .pareto_before(&at, egress, u32::MAX)
                    .iter()
                    .filter(|j| !is_loop(j))
                    .filter(|j| f64::from(j.arrival.saturating_sub(departure)) <= d.journey_max_s)
                    .filter_map(|j| Some((j.arrival, self.leg_of(j, from, to, departure, pays)?))),
            );
        }
        if views == 1 {
            return found.into_iter().map(|(_, leg)| leg).collect();
        }
        let keys: Vec<(u32, usize, f64)> =
            found.iter().map(|(a, leg)| (*a, leg.rides.len(), leg.attributes.money)).collect();
        found
            .into_iter()
            .enumerate()
            .filter(|&(i, _)| !dominated(&keys, i))
            .map(|(_, (_, leg))| leg)
            .collect()
    }

    /// The leg of journey `j` of a trip leaving at `departure`, its money the fare if the
    /// traveller `pays`, else 0; `None` if its walking breaks a limit ([`walking_within_limits`]).
    fn leg_of(
        &self,
        j: &Journey,
        from: Place,
        to: Place,
        departure: u32,
        pays: bool,
    ) -> Option<Leg> {
        let (mut access, mut egress) = (0_u32, 0_u32);
        let mut transfers: Vec<f64> = Vec::new();
        let (mut walk, mut wait, mut ride, mut metres) = (0_u32, 0_u32, 0_u32, 0.0);
        let fares = self.transit.fares;
        let mut fare = fares.base_eur;
        let mut clock = departure;
        let mut rides = Vec::new();
        for leg in &j.legs {
            match *leg {
                JourneyLeg::Access { arrival, .. } => {
                    access = arrival.saturating_sub(departure);
                    walk += access;
                    clock = arrival;
                }
                JourneyLeg::Ride {
                    run,
                    board_call,
                    alight_call,
                    board_stop,
                    alight_stop,
                    departure: leave,
                    arrival,
                } => {
                    wait += leave.saturating_sub(clock);
                    ride += arrival.saturating_sub(leave);
                    let m = ground_distance_metres(
                        self.timetable.stop_position(board_stop),
                        self.timetable.stop_position(alight_stop),
                    );
                    metres += m;
                    fare += fares.km_eur_of(self.timetable.run_kind(run)) * m / 1000.0;
                    rides.push(Ride {
                        line: self.timetable.run_route(run),
                        run,
                        board_stop,
                        alight_stop,
                        board_call,
                        alight_call,
                        departure: leave,
                        arrival,
                    });
                    clock = arrival;
                }
                JourneyLeg::Transfer { seconds, .. } => {
                    transfers.push(f64::from(seconds));
                    walk += seconds;
                    clock = clock.saturating_add(seconds);
                }
                JourneyLeg::Egress { seconds, .. } => {
                    egress = seconds;
                    walk += seconds;
                }
            }
        }
        let limits = (self.transit.defaults.access_walk_max_s, self.transit.walk_total_max_seconds);
        if !walking_within_limits(
            f64::from(access),
            &transfers,
            f64::from(egress),
            limits.0,
            limits.1,
        ) {
            return None;
        }
        let attributes = Attributes {
            in_vehicle_seconds: f64::from(ride),
            walk_seconds: f64::from(walk),
            wait_seconds: f64::from(wait),
            transfers: u32::try_from(rides.len().saturating_sub(1)).expect("few rides"),
            length_metres: metres,
            money: if pays { fare } else { 0.0 },
            ..Attributes::default()
        };
        Some(Leg {
            mode: Mode::Transit,
            from,
            to,
            departure: f64::from(departure),
            attributes,
            fixed: FixedTimes::NONE,
            links: Vec::new(),
            rides,
        })
    }
}

/// Whether journey `i` of `keys` (each journey's arrival, vehicles and fare) is dominated: another
/// is no later, takes no more vehicles and costs no more, and is better in one of the three, or
/// equal in all three and listed before it (the first of equal journeys is kept).
fn dominated(keys: &[(u32, usize, f64)], i: usize) -> bool {
    let (a, v, f) = keys[i];
    keys.iter().enumerate().any(|(k, &(b, w, g))| {
        k != i && b <= a && w <= v && g <= f && (k < i || b < a || w < v || g < f)
    })
}

/// Whether a journey comes back to the stop it first boarded at (a ride out and back).
fn is_loop(j: &Journey) -> bool {
    let mut rides = j.legs.iter().filter_map(|l| match *l {
        JourneyLeg::Ride { board_stop, alight_stop, .. } => Some((board_stop, alight_stop)),
        _ => None,
    });
    let first = rides.next();
    let last = rides.next_back().or(first);
    matches!((first, last), (Some((board, _)), Some((_, alight))) if board == alight)
}

impl LegPlanner for TransitPlanner<'_> {
    fn mode(&self) -> Mode {
        Mode::Transit
    }

    /// The journey arriving first (the last of the Pareto set kept), its fare not priced (no
    /// traveller is known here); the class, the state and the cost source are not read (the
    /// timetable is static).
    fn plan(
        &self,
        from: Place,
        to: Place,
        departure: f64,
        _class: UserClassId,
        _state: &State,
        _cost_source: CostSource,
    ) -> Option<Leg> {
        let (Place::Walk(o), Place::Walk(d)) = (from, to) else { return None };
        let mut reach = self.reach();
        let access = self.walks(&mut reach, o, false);
        let egress = self.walks(&mut reach, d, true);
        let mut raptors = self.raptors();
        let departure = floor_seconds(departure);
        self.legs(&mut raptors, (from, to), departure, (&access, &egress), false).pop()
    }

    /// Nothing: a leg on the static timetable costs what it cost when planned.
    fn reprice(&self, _leg: &mut Leg, _state: &State) {}
}

/// The logarithm of each chain's path-size factor among `chains` (one trip's transit chains), by
/// time share: `ln(Σ_r s_r / n_r + (T − Σ_r s_r)) − ln T`, with `s_r` the seconds of ride `r` (a
/// line between two stops), `n_r` the number of the chains that ride it, and `T` the chain's
/// door-to-door seconds (at least 1). See the [module docs](self).
#[must_use]
pub fn ride_path_sizes(chains: &[&Chain]) -> Vec<f64> {
    let key = |r: &Ride| (r.line, r.board_stop, r.alight_stop);
    let rides = |c: &Chain| -> Vec<Ride> {
        c.legs().iter().flat_map(|l| l.rides.iter().copied()).collect()
    };
    let all: Vec<Vec<Ride>> = chains.iter().map(|c| rides(c)).collect();
    chains
        .iter()
        .zip(&all)
        .map(|(c, own)| {
            let total = c.total().total_seconds().max(1.0);
            let (mut size, mut shared) = (0.0, 0.0);
            for r in own {
                let users = all.iter().filter(|o| o.iter().any(|x| key(x) == key(r))).count();
                let secs = f64::from(r.arrival - r.departure);
                #[allow(clippy::cast_precision_loss, reason = "a handful of chains")]
                let users = users.max(1) as f64;
                size += secs / users;
                shared += secs;
            }
            size += (total - shared).max(0.0);
            (size / total).clamp(1e-9, 1.0).ln()
        })
        .collect()
}

/// Per call of `timetable`, the travellers (by their weight) boarding there and alighting
/// there, from every trip's chain taken in `state`.
#[must_use]
pub fn boardings(
    timetable: &Timetable,
    state: &State,
    travellers: &Travellers,
    trips: &Trips,
) -> (Vec<f64>, Vec<f64>) {
    let n = timetable.call_count();
    let (mut on, mut off) = (vec![0.0; n], vec![0.0; n]);
    for (t, entry) in state.trips().iter().enumerate() {
        let Some(chain) = entry.current_chain() else { continue };
        let w = f64::from(travellers.weight(trips.traveller(TripId::from_index(t))));
        for r in chain.legs().iter().flat_map(|l| &l.rides) {
            on[r.board_call as usize] += w;
            off[r.alight_call as usize] += w;
        }
    }
    (on, off)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_journey_stands_unless_another_is_as_good_in_all_three_and_better_in_one() {
        // (arrival, vehicles, fare): a train (earlier, dearer); a bus (later, cheaper); a later
        // and dearer bus; the train again (equal in all three, listed after); two vehicles,
        // earlier and dearer still.
        let keys = [(900, 1, 4.6), (1500, 1, 2.2), (1600, 1, 3.0), (900, 1, 4.6), (800, 2, 9.0)];
        let stands: Vec<bool> = (0..keys.len()).map(|i| !dominated(&keys, i)).collect();
        assert_eq!(stands, [true, true, false, false, true]);
    }

    #[test]
    #[allow(clippy::float_cmp, reason = "exact zeros")]
    fn each_kind_reads_its_own_price_per_km() {
        let fares = Fares { base_eur: 1.0, km_eur: [0.3, 0.1, 0.1, 0.1, 0.5, 0.0] };
        assert!((fares.km_eur_of(ServiceKind::Rail) - 0.3).abs() < 1e-15);
        assert!((fares.km_eur_of(ServiceKind::Bus) - 0.1).abs() < 1e-15);
        assert!(Fares::FREE.km_eur.iter().all(|p| *p == 0.0) && Fares::FREE.base_eur == 0.0);
    }
}
