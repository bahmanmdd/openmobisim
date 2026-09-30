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
//! **Candidates** (out): the parkings of the vehicle's kind within reach, found
//! by one search from the origin ([`crate::parking::ParkingDefaults::reach_car_s`],
//! [`crate::parking::ParkingDefaults::reach_bike_s`]); the `K` nearest and the `K` best by a rough
//! estimate of the whole trip are tried (`K` =
//! [`crate::parking::ParkingDefaults::candidates`]), each with one RAPTOR query; the
//! alternatives of at most `K` parkings are kept. **The choice-set limit** (the
//! run's `choice_detour_limit`) then drops the alternatives much slower than the
//! best, as it does for car routes.
//!
//! **The attributes** — one vocabulary for every alternative, routes included
//! ([`ATTRIBUTES`]): `time_min` (door to door), `car_min`, `bike_min`,
//! `walk_min`, `wait_min` (first wait and transfer waits, boarding slack
//! included), `ride_min`, `transfers`, `parking_min` (parking or fetching the
//! vehicle), `ln_path_size` (over the itinerary by time share: the vehicle leg
//! and each ride are the elements alternatives can share), and, as for routes,
//! `length_km` (the vehicle leg's), `detour`, `overlap` (0) and `n_links` (the
//! vehicle leg's).
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
//! **Cost:** per transit trip one RAPTOR query; per out trip one bounded search
//! and up to `2K` queries; per back trip one query and a search per journey.
//! Parallel in fixed chunks, so results never depend on the thread count.

use openmobisim_core_choice::{ChoiceBatch, ChoiceError, ChoiceModel};
use openmobisim_core_demand::{Mode, Travellers, Trips};
use openmobisim_core_graph::geometry::{LonLat, ground_distance_metres};
use openmobisim_core_graph::hubs::ParkingKind;
use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_routes::{NodeSnapper, Reach, Search, SearchContext};
use openmobisim_core_transit::{Journey, JourneyLeg, Raptor, RaptorData};
use openmobisim_core_types::hash::Fnv1a;
use openmobisim_core_types::ids::{EntityId, LinkId, NULL_ID, NodeId, TripId};
use openmobisim_core_types::rng::StreamRng;

use crate::equilibration::Equilibration;
use crate::layers::LayerSetup;
use crate::link_times::LinkTimes;
use crate::parking::{ExpectedAvailability, ParkingSetup};
use crate::transit::{TransitSetup, par_map};

/// The attributes every alternative carries, routes and itineraries alike, in
/// the order a batch holds them (A8).
pub const ATTRIBUTES: [&str; 13] = [
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
];

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

/// The shape of an itinerary trip, given where its vehicle is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    /// Walk, ride, walk.
    Transit,
    /// The vehicle first, then transit.
    Out,
    /// Transit first, then the vehicle parked at this parking.
    Back(u32),
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

/// An itinerary trip's nodes on the layers it may use (null where absent).
#[derive(Clone, Copy, Debug)]
pub(crate) struct TripNodes {
    pub walk_o: NodeId,
    pub walk_d: NodeId,
    pub road_o: NodeId,
    pub road_d: NodeId,
    pub bike_o: NodeId,
    pub bike_d: NodeId,
}

/// What an itinerary choice set is made on: see the [module docs](self).
pub(crate) struct Planner<'a> {
    pub transit: &'a TransitSetup,
    pub parking: Option<&'a ParkingSetup>,
    pub car: &'a SearchContext<'a>,
    pub bike: Option<(&'a LayerSetup, &'a SearchContext<'a>)>,
    pub raptor: &'a RaptorData,
    pub times: &'a LinkTimes,
    pub availability: Option<&'a ExpectedAvailability>,
    pub detour_limit: f64,
}

/// One thread's scratch for planning.
pub(crate) struct Scratch<'a> {
    out: Reach<'a>,
    into: Reach<'a>,
    raptor: Raptor<'a>,
    /// The car and bike searches, made on first use: a plain transit trip needs
    /// neither, and each holds a few bytes per link of its whole network.
    car: Option<Search<'a>>,
    bike: Option<Search<'a>>,
}

impl<'a> Scratch<'a> {
    fn car(&mut self, ctx: &'a SearchContext<'a>) -> &mut Search<'a> {
        self.car.get_or_insert_with(|| Search::new(ctx))
    }

    fn bike(&mut self, ctx: &'a SearchContext<'a>) -> &mut Search<'a> {
        self.bike.get_or_insert_with(|| Search::new(ctx))
    }
}

impl<'a> Planner<'a> {
    pub(crate) fn scratch(&self) -> Scratch<'a> {
        let walk = self.transit.walk().network();
        let seconds = self.transit.walk_link_seconds();
        Scratch {
            out: Reach::new(walk, seconds),
            into: Reach::new(walk, seconds),
            raptor: Raptor::new(self.raptor, self.transit.defaults().max_rides as usize),
            car: None,
            bike: None,
        }
    }

    /// Every alternative of a trip of `shape` (for `kind`'s vehicle), finalised:
    /// deduplicated, within the choice-set limit, path sizes set.
    pub(crate) fn plan(
        &self,
        s: &mut Scratch<'a>,
        n: &TripNodes,
        departure: u32,
        destination: LonLat,
        shape: Shape,
        kind: ParkingKind,
    ) -> Vec<Alternative> {
        let (alts, cap) = match shape {
            Shape::Transit => (self.plan_transit(s, n, departure), usize::MAX),
            Shape::Out => {
                let k = self.parking.map_or(1, |p| p.defaults().candidate_count());
                (self.plan_out(s, n, departure, destination, kind), k)
            }
            Shape::Back(p) => (self.plan_back(s, n, departure, p, kind), usize::MAX),
        };
        finalise(alts, self.detour_limit, cap)
    }

    fn plan_transit(&self, s: &mut Scratch<'a>, n: &TripNodes, departure: u32) -> Vec<Alternative> {
        if n.walk_o.is_null() || n.walk_d.is_null() {
            return Vec::new();
        }
        let access = self.transit.access(&mut s.out, n.walk_o, departure);
        let egress = self.transit.egress(&mut s.into, n.walk_d);
        if access.is_empty() || egress.is_empty() {
            return Vec::new();
        }
        s.raptor
            .pareto(&access, &egress)
            .iter()
            .map(|j| {
                let mut a = self.transit_part(j, departure);
                a.shape = Shape::Transit;
                a.total_s = f64::from(j.arrival.saturating_sub(departure));
                a
            })
            .collect()
    }

    fn plan_out(
        &self,
        s: &mut Scratch<'a>,
        n: &TripNodes,
        departure: u32,
        destination: LonLat,
        kind: ParkingKind,
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
                let times = self.times;
                let wait = |l: u32, t: f64| times.origin_wait_seconds(l, t);
                let secs = |l: u32, t: f64| times.link_seconds(l, t);
                let road = self.car.network;
                s.car(self.car)
                    .fastest_routes_to(n.road_o, targets, count, dep, d.reach_car_s, &wait, &secs)
                    .into_iter()
                    .map(|(node, t, links)| {
                        let m = links.iter().map(|&l| road.link_length(l).get()).sum();
                        (node, t, links, m)
                    })
                    .collect()
            }
            ParkingKind::Bike => {
                let Some((layer, ctx)) = self.bike else {
                    return Vec::new();
                };
                let search = s.bike(ctx);
                if n.bike_o.is_null() {
                    return Vec::new();
                }
                let cost = |l: u32, _t: f64| ctx.link_cost(LinkId::new(l));
                let graph = layer.network().network();
                search
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
        if tried.is_empty() || n.walk_d.is_null() {
            return Vec::new();
        }
        let egress = self.transit.egress(&mut s.into, n.walk_d);
        if egress.is_empty() {
            return Vec::new();
        }
        let transfer = self.transit.transfer_s();
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
            for j in s.raptor.pareto(&access, &egress) {
                let mut a = self.transit_part(&j, parked);
                a.shape = Shape::Out;
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

    fn plan_back(
        &self,
        s: &mut Scratch<'a>,
        n: &TripNodes,
        departure: u32,
        p: u32,
        kind: ParkingKind,
    ) -> Vec<Alternative> {
        let Some(parking) = self.parking else { return Vec::new() };
        if n.walk_o.is_null() {
            return Vec::new();
        }
        let access = self.transit.access(&mut s.out, n.walk_o, departure);
        let transfer = self.transit.transfer_s();
        let egress: Vec<(NodeId, u32)> =
            parking.stops(p).iter().map(|&(stop, w)| (stop, w.saturating_add(transfer))).collect();
        if access.is_empty() || egress.is_empty() {
            return Vec::new();
        }
        let fetch = parking.defaults().fetch_seconds(kind);
        let dep = f64::from(departure);
        let from = parking.node(p);
        let mut alts = Vec::new();
        for j in s.raptor.pareto(&access, &egress) {
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
        let tt = self.transit.timetable();
        let mut a = Alternative {
            shape: Shape::Transit,
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
        };
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
                    a.wait_s += f64::from(departure.saturating_sub(clock));
                    let ride = arrival.saturating_sub(departure);
                    a.ride_s += f64::from(ride);
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

/// Deduplicate by identity (the fastest first, fewest vehicles on a tie), keep
/// what is within `detour_limit` of the best (0: all), keep the alternatives of
/// at most `parkings` parkings, and set the identities and path sizes.
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
    if let Some(best) = alts.first().map(|a| a.total_s) {
        if detour_limit > 0.0 {
            let reach = best * (1.0 + detour_limit);
            alts.retain(|a| a.total_s <= reach);
        }
    }
    if parkings < usize::MAX {
        let mut kept: Vec<u32> = Vec::new();
        alts.retain(|a| {
            if kept.contains(&a.parking) {
                true
            } else if kept.len() < parkings {
                kept.push(a.parking);
                true
            } else {
                false
            }
        });
    }
    // Path size by time share: the vehicle leg and each ride are elements
    // alternatives can share; walking, waiting and parking are each its own.
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
            e.push(((0, a.parking, hash, 0), a.vehicle_s));
        }
        for (r, &secs) in a.rides.iter().zip(&a.ride_seconds) {
            e.push(((1, r.route, r.board.raw(), r.alight.raw()), f64::from(secs)));
        }
        e
    };
    let all: Vec<Vec<(Element, f64)>> = alts.iter().map(elements).collect();
    for (i, a) in alts.iter_mut().enumerate() {
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

/// The value of attribute `name` for `a`, the best total in its set being `best`.
fn attribute(name: &str, a: &Alternative, best: f64, kind: Option<ParkingKind>) -> f64 {
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
        _ => 0.0,
    }
}

/// The attributes a model reads, checked against [`ATTRIBUTES`].
///
/// # Errors
///
/// [`ChoiceError::MissingAttribute`] for a name that is not one of them.
pub(crate) fn wanted(model: &dyn ChoiceModel) -> Result<Vec<&'static str>, ChoiceError> {
    match model.required_attributes() {
        None => Ok(ATTRIBUTES.to_vec()),
        Some(names) => {
            for name in &names {
                if !ATTRIBUTES.contains(&name.as_str()) {
                    return Err(ChoiceError::MissingAttribute {
                        name: name.clone(),
                        offered: ATTRIBUTES.iter().map(|s| (*s).to_string()).collect(),
                    });
                }
            }
            Ok(ATTRIBUTES.iter().copied().filter(|a| names.iter().any(|n| n == a)).collect())
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
    /// How many alternatives the trip had.
    pub alternatives: Vec<u32>,
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
}

impl Assessment {
    /// The relative gap of `mode`'s itinerary trips, NaN if none was assessed.
    pub(crate) fn gap(&self, mode: Mode) -> f64 {
        let (paid, least) = (self.paid[mode.index()], self.least[mode.index()]);
        if least > 0.0 { (paid - least) / least } else { f64::NAN }
    }
}

/// The itinerary trips of a run: which they are, their nodes, and the rounds
/// they are chosen in.
pub(crate) struct Itineraries {
    /// The itinerary trips, in trip order.
    pub trips: Vec<TripId>,
    /// Per trip of the run: its position in `trips`, or `u32::MAX`.
    pos: Vec<u32>,
    nodes: Vec<TripNodes>,
    /// Positions by round.
    rounds: Vec<Vec<usize>>,
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

/// What choosing needs besides the planner.
pub(crate) struct ChooseInputs<'a> {
    pub trips: &'a Trips,
    pub travellers: &'a Travellers,
    pub model: &'a dyn ChoiceModel,
    pub rng: &'a StreamRng,
    pub wanted: &'a [&'static str],
}

impl Itineraries {
    /// The run's trips that are itinerary trips, given what it can simulate:
    /// transit trips if it has a timetable; park-and-ride and bike-and-ride if it
    /// also has parkings (and, for bikes, the bike layer).
    pub(crate) fn new(
        trips: &Trips,
        travellers: &Travellers,
        transit: &TransitSetup,
        parking: Option<&ParkingSetup>,
        road: &RoadNetwork,
        bike: Option<&LayerSetup>,
    ) -> Self {
        let total = trips.len() as usize;
        let simulated = |mode: Mode| match mode {
            Mode::Transit => true,
            Mode::CarTransit => parking.is_some(),
            Mode::BikeTransit => parking.is_some() && bike.is_some(),
            _ => false,
        };
        let road_snapper = NodeSnapper::new(road);
        let bike_snapper = bike.map(|b| NodeSnapper::every_node(b.network().network()));
        let null = NodeId::from_raw(NULL_ID);
        let mut out = Self {
            trips: Vec::new(),
            pos: vec![u32::MAX; total],
            nodes: Vec::new(),
            rounds: Vec::new(),
        };
        for i in 0..total {
            let trip = TripId::from_index(i);
            let mode = trips.mode(trip);
            if !simulated(mode) {
                continue;
            }
            let (o, d) = (trips.origin(trip), trips.destination(trip));
            let mut nodes = TripNodes {
                walk_o: transit.walk_node(o),
                walk_d: transit.walk_node(d),
                road_o: null,
                road_d: null,
                bike_o: null,
                bike_d: null,
            };
            match mode {
                Mode::CarTransit => {
                    nodes.road_o = road_snapper.nearest(road, o);
                    nodes.road_d = road_snapper.nearest(road, d);
                }
                Mode::BikeTransit => {
                    if let (Some(b), Some(snapper)) = (bike, &bike_snapper) {
                        let graph = b.network().network();
                        nodes.bike_o = snapper.nearest(graph, o);
                        nodes.bike_d = snapper.nearest(graph, d);
                    }
                }
                _ => {}
            }
            out.pos[i] = u32::try_from(out.trips.len()).expect("trips fit u32");
            out.trips.push(trip);
            out.nodes.push(nodes);
        }
        // Rounds: plain transit trips first; each traveller's vehicle itinerary trips
        // one round each, in order.
        let mut rounds: Vec<Vec<usize>> = vec![Vec::new()];
        let mut per_traveller: std::collections::HashMap<u32, usize> =
            std::collections::HashMap::new();
        for (p, &trip) in out.trips.iter().enumerate() {
            if trips.mode(trip) == Mode::Transit {
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
        let _ = travellers;
        out.rounds = rounds;
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

    /// The shape of the trip at `position`, following the traveller's choices so
    /// far (`chosen`): `None` if its vehicle is not where it can be used.
    fn shape_of(
        &self,
        trips: &Trips,
        travellers: &Travellers,
        position: usize,
        chosen: &[Option<Alternative>],
    ) -> Option<Shape> {
        let trip = self.trips[position];
        let mode = trips.mode(trip);
        let Some(kind) = parking_kind_of(mode) else { return Some(Shape::Transit) };
        let traveller = trips.traveller(trip);
        let own = travellers.ownership(traveller);
        let owns = match kind {
            ParkingKind::Car => own.car,
            ParkingKind::Bike => own.bike,
        };
        if !owns {
            return None;
        }
        let mut first = true;
        let mut loc = Loc::Nowhere;
        for t in travellers.trips_of(traveller) {
            if first {
                loc = Loc::At(trips.origin(t));
                first = false;
            }
            if t == trip {
                break;
            }
            let m = trips.mode(t);
            let uses = match kind {
                ParkingKind::Car => matches!(m, Mode::Car | Mode::CarTransit),
                ParkingKind::Bike => matches!(m, Mode::Bike | Mode::BikeTransit),
            };
            if !uses {
                continue;
            }
            let at_origin = loc == Loc::At(trips.origin(t));
            match m {
                Mode::Car | Mode::Bike => {
                    if at_origin {
                        loc = Loc::At(trips.destination(t));
                    }
                }
                _ => {
                    let alt = self.position(t).and_then(|p| chosen[p].as_ref());
                    match alt.map(|a| a.shape) {
                        Some(Shape::Out) if at_origin => {
                            loc = Loc::Parked(alt.expect("some").parking)
                        }
                        Some(Shape::Back(p)) if loc == Loc::Parked(p) => {
                            loc = Loc::At(trips.destination(t));
                        }
                        _ => {}
                    }
                }
            }
        }
        match loc {
            Loc::Parked(p) => Some(Shape::Back(p)),
            Loc::At(o) if o == trips.origin(trip) => Some(Shape::Out),
            _ => None,
        }
    }

    /// Choose: every trip (`current` `None`, iteration 0), or, after a loading,
    /// the travellers `strategy` picks to choose again and those whose
    /// alternative is no longer on offer, the rest keeping theirs at the new
    /// costs. Returns the choices and what the pass found.
    ///
    /// # Errors
    ///
    /// [`ChoiceError`] if the model fails or answers wrongly.
    pub(crate) fn choose(
        &self,
        planner: &Planner<'_>,
        inputs: &ChooseInputs<'_>,
        current: Option<&Chosen>,
        iteration: u32,
        strategy: Option<(&dyn Equilibration, &StreamRng)>,
    ) -> Result<(Chosen, Assessment), ChoiceError> {
        let n = self.trips.len();
        let mut next = Chosen {
            alt: current.map_or_else(|| vec![None; n], |c| c.alt.clone()),
            probability: current.map_or_else(|| vec![f64::NAN; n], |c| c.probability.clone()),
            alternatives: current.map_or_else(|| vec![0; n], |c| c.alternatives.clone()),
        };
        let mut assessment = Assessment::default();
        let (trips, travellers) = (inputs.trips, inputs.travellers);
        for round in &self.rounds {
            // The shape of each trip, from the choices before it.
            let work: Vec<(usize, Option<Shape>)> = round
                .iter()
                .map(|&p| (p, self.shape_of(trips, travellers, p, &next.alt)))
                .collect();
            let sets: Vec<Vec<Alternative>> = par_map(
                &work,
                SCRATCH_CHUNK,
                || planner.scratch(),
                |s, &(p, shape)| {
                    let Some(shape) = shape else { return Vec::new() };
                    let trip = self.trips[p];
                    let kind = parking_kind_of(trips.mode(trip)).unwrap_or(ParkingKind::Car);
                    planner.plan(
                        s,
                        &self.nodes[p],
                        trips.departure(trip).get(),
                        trips.destination(trip),
                        shape,
                        kind,
                    )
                },
            );
            for chunk in (0..work.len()).collect::<Vec<_>>().chunks(CHUNK) {
                let mut batch = ChoiceBatch::new(iteration, inputs.wanted);
                let mut situation_of: Vec<usize> = Vec::new();
                let mut row = vec![0.0; inputs.wanted.len()];
                let mut movers: Vec<usize> = Vec::new();
                for &w in chunk {
                    let (p, _) = work[w];
                    let set = &sets[w];
                    let trip = self.trips[p];
                    if set.is_empty() {
                        next.alt[p] = None;
                        next.alternatives[p] = 0;
                        continue;
                    }
                    let traveller = trips.traveller(trip);
                    let kind = parking_kind_of(trips.mode(trip));
                    let best = set[0].total_s;
                    batch.begin_situation(traveller.raw(), trip.raw());
                    for a in set {
                        for (slot, name) in row.iter_mut().zip(inputs.wanted) {
                            *slot = attribute(name, a, best, kind);
                        }
                        batch.push_alternative(a.identity, &row);
                    }
                    let s = situation_of.len();
                    situation_of.push(w);
                    next.alternatives[p] = u32::try_from(set.len()).expect("few alternatives");
                    let weight = f64::from(travellers.weight(traveller));
                    // Keep, or choose again.
                    let kept = current
                        .and_then(|c| c.alt[p].as_ref())
                        .and_then(|old| set.iter().find(|a| a.identity == old.identity));
                    let Some(kept) = kept else {
                        if current.is_some() {
                            assessment.w_forced += weight;
                        }
                        movers.push(s);
                        continue;
                    };
                    let m = trips.mode(trip).index();
                    assessment.paid[m] += weight * kept.total_s;
                    assessment.least[m] += weight * best;
                    assessment.w_all += weight;
                    let reselect = strategy.is_some_and(|(strategy, msa)| {
                        strategy.reselects(msa, traveller.raw(), iteration)
                    });
                    if reselect {
                        assessment.w_reselected += weight;
                        movers.push(s);
                    } else {
                        next.alt[p] = Some(kept.clone());
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
                    let changed = current
                        .and_then(|c| c.alt[p].as_ref())
                        .is_some_and(|old| old.identity != alt.identity);
                    if changed {
                        let trip = self.trips[p];
                        assessment.w_changed += f64::from(travellers.weight(trips.traveller(trip)));
                    }
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
    /// How many alternatives it had (0: none; it did not travel).
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
}

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
        };
        for alt in &chosen.alt {
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
