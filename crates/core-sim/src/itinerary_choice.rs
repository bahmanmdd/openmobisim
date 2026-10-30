//! Itinerary choice: which itinerary a transit, park-and-ride or
//! bike-and-ride trip takes, from a choice set of the competitive ones, by the
//! run's choice model.
//!
//! **The rule**: the competitive options are in the choice
//! set, and the choice model selects the whole mode/route combination, legs
//! through different hubs included, from the attributes that matter. An
//! **alternative** here is a door-to-door itinerary:
//!
//! | Trip | Alternative |
//! |---|---|
//! | `transit` | walk · ride(s) · walk: one of RAPTOR's Pareto journeys (arrival against vehicles) |
//! | `car_transit` or `bike_transit`, vehicle at the origin (**out**) | drive or ride to a parking, park, walk to a stop, ride(s), walk: parking × route to it × journey from it |
//! | the same, vehicle at a parking (**back**) | walk, ride(s) to that parking's stops, walk, fetch the vehicle, drive or ride home: journey × route from it. **The hub is fixed: it is where the vehicle is** |
//!
//! **Candidates** (out): the parkings of the vehicle's kind within reach
//! ([`crate::parking::ParkingDefaults::reach_car_s`],
//! [`crate::parking::ParkingDefaults::reach_bike_s`]); the `K` nearest and the `K` best by a rough
//! estimate of the whole trip are tried (`K` =
//! [`crate::parking::ParkingDefaults::candidates`]), each with one RAPTOR query; the
//! alternatives of at most `K` parkings are kept. **Car parks** are picked on the
//! free-flow drive to each (one backward search per car park, once per run), and only
//! the picked ones' routes are then searched on the expected times. **Bike
//! parkings** are found by one search from the origin on the bike layer, whose costs
//! are static, so each trip's are found once per run and kept, pruned to those that can
//! ever be tried. **The choice-set limit** (the run's `choice_detour_limit`) then
//! drops the alternatives much slower than the best, as it does for car routes.
//!
//! **The attributes** — one vocabulary for every alternative, routes included
//! ([`ATTRIBUTES`]): `time_min` (door to door), `car_min`, `bike_min`,
//! `walk_min`, `wait_min` (first wait and transfer waits, boarding slack
//! included), `ride_min`, `transfers`, `parking_min` (parking or fetching the
//! vehicle), `ln_path_size` (over the itinerary by time share: the vehicle leg
//! and each ride are the elements alternatives can share), and, as for routes,
//! `length_km` (the vehicle leg's), `detour`, `overlap` (0) and `n_links` (the
//! vehicle leg's); and for mode choice `nest` (the mode's index) and the 0/1
//! `mode_walk`, `mode_bike`, `mode_car`, `mode_transit`, `mode_car_transit` and
//! `mode_bike_transit`; and money, `cost_eur` and its parts ([`crate::prices`]).
//!
//! **Identity** is a hash of the parking, the vehicle leg's links and the rides
//! as (line, boarding stop, alighting stop), **not the runs**, so a bus a
//! minute late keeps the alternative and its draws.
//!
//! **Choosing and executing**: the whole itinerary is chosen on expected
//! costs (the last iteration's link times and parking availability; free flow and the
//! start-of-day parkings at first; the timetable always, with the disruptions travellers
//! know of);
//! a run executes it on the realised times, boarding the first run of each
//! chosen line that can be caught, and falls back to the earliest journey from
//! where the traveller is when a line cannot be followed (counted). **Later**:
//! re-check the choice against the realised service.
//!
//! **Rounds:** a trip back to a parked vehicle depends on where the trip before
//! it left the vehicle, so each traveller's vehicle itinerary trips are chosen in
//! order, one round each; plain transit trips in the first round.
//!
//! **Cost:** per transit trip one RAPTOR query (its walks to and from stops are searched
//! once per run, [`crate::transit::TransitSetup`]); per out trip one search, stopped at the
//! picked car parks or, by bike, once per run, and up to `2K` queries; per back trip one
//! query and a search per journey. Parallel in fixed chunks, so results never depend on
//! the thread count.
//!
//! **Timetable searches are kept** (`KeptSearches`): the timetable a run plans on is the
//! same in every choice, so a trip's plain transit journeys are searched once per run, its
//! journeys back to a parking once per parking, and its journeys out of a parking again only
//! when it reaches the parking at another second (the drive or the time to park changed) or
//! with a looser bound on the arrival than the kept search had. Exact: a kept search answers
//! only a query it answers bit for bit. Memory: the kept journeys, a few hundred bytes a trip.

use std::collections::HashMap;
use std::sync::OnceLock;

use openmobisim_core_choice::{ChoiceBatch, ChoiceError, ChoiceModel};
use openmobisim_core_demand::{Mode, Travellers, Trips};
use openmobisim_core_graph::defaults::RoadClass;
use openmobisim_core_graph::geometry::{LonLat, ground_distance_metres};
use openmobisim_core_graph::hubs::ParkingKind;
use openmobisim_core_graph::layers::{BikeInfrastructure, StaticLayer};
use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_routes::{NodeSnapper, Reach, RouteKey, RouteSets, Search, SearchContext};
use openmobisim_core_transit::{Journey, JourneyLeg, Raptor, RaptorData, ServiceKind};
use openmobisim_core_types::hash::Fnv1a;
use openmobisim_core_types::ids::{EntityId, LinkId, NULL_ID, NodeId, TravellerId, TripId};
use openmobisim_core_types::rng::{DrawAddress, StreamRng};

use crate::demand_values::DemandValues;
use crate::equilibration::Equilibration;
use crate::gap::{Currency, Ended, GapTrips, RouteGapSums};
use crate::generalised_cost::RouteCost;
use crate::layers::{LayerSetup, StaticLayers, StaticRoutes};
use crate::link_times::LinkTimes;
use crate::link_values::{PreparedLinkValues, ValueLayer};
use crate::parking::{ExpectedAvailability, ParkingSetup};
use crate::prices::{Cost, PreparedPrices};
use crate::services::ServiceSetup;
use crate::services::costs::DockCosts;
use crate::transit::{TransitSetup, par_map};
use crate::trees::{DriveKey, TreeDrive};

/// The attributes every alternative carries, routes and itineraries alike, in
/// the order a batch holds them.
///
/// After `parking_min`: the bike leg's kilometres by facility
/// (`bike_separated_km`, `bike_lane_km`, `bike_mixed_km`), and a transit itinerary's walks,
/// waits and rides split — the walk to the first stop, from the last and between stops
/// (`walk_access_min`, `walk_egress_min`, `walk_transfer_min`), the wait for the first
/// vehicle and for the others (`wait_first_min`, `wait_transfer_min`), and the minutes on
/// board by kind of service (`ride_rail_min` … `ride_other_min`). Each is a part of a total
/// above it, so a model weighs the parts on top of the totals, or instead of them.
///
/// After those: money, in euros — `cost_eur` and its parts
/// `cost_running_eur`, `cost_toll_eur`, `cost_parking_eur`, `cost_fare_eur` ([`crate::prices`]);
/// and the trip's departure in hours after midnight, `trip_departure_h` (U6;
/// [`crate::demand_values`]).
pub const ATTRIBUTES: [&str; 40] = [
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
    "trip_departure_h",
    "nest",
    "mode_walk",
    "mode_bike",
    "mode_car",
    "mode_transit",
    "mode_car_transit",
    "mode_bike_transit",
];

/// The attributes a shared-bike alternative adds, in a run with a docked service (S302), after
/// [`ATTRIBUTES`]: `mode_bike_share` (1 for a shared bike, 0 for the rest: its constant),
/// `dock_min`, the expected minutes walked to other docks when the pickup dock is empty or
/// the return dock full ([`crate::services::costs::DockCosts`]; part of `walk_min`, so weighed as
/// walking by default; 0 for every other mode), and `mode_bike_share_transit` (1 for a shared
/// bike to a stop's hub then transit, 0 for the rest). A run without a service offers none, so
/// its choice sets are what they were before services existed.
pub const SERVICE_ATTRIBUTES: [&str; 3] =
    ["mode_bike_share", "dock_min", "mode_bike_share_transit"];

/// The attributes a taxi alternative adds, in a run with a taxi fleet (S302), after
/// [`ATTRIBUTES`] (and [`SERVICE_ATTRIBUTES`] with a docked service too): `mode_taxi` (1 for a
/// taxi, 0 for the rest: its constant) and `fail_prob`, the run's expected share of requests that
/// get no vehicle within the fleet's longest wait
/// ([`crate::services::ServiceExpectations::taxi_fail_share`], the same for every trip; 0 for
/// every other mode). A taxi's `wait_min` is its expected wait (the run's
/// [`crate::services::ServiceExpectations::taxi_wait_s`]; the longest wait for the share
/// expected to fail), its `car_min` its drive and its `time_min` the door-to-door expectation,
/// the walk of the share expected to fail included. A run without a taxi fleet offers neither.
pub const TAXI_ATTRIBUTES: [&str; 2] = ["mode_taxi", "fail_prob"];

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
    /// A shared bike (S302): walk to a dock, ride on the bike layer, return the bike at a dock,
    /// walk; [`Alternative::share_docks`] says which docks.
    Share,
    /// A shared bike to a transit stop's hub, then transit (S302): walk to a dock, ride, return
    /// the bike at a dock of a hub with a stop, walk to the stop, ride(s), walk;
    /// [`Alternative::share_docks`] says which docks.
    ShareTransit,
    /// A taxi of the run's taxi fleet (S302), door to door: its drive is the car's fastest
    /// route at the expected link times.
    Taxi,
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
    /// The path size's log fixed from elsewhere (a car route's, over the car routes offered:
    /// by length, [`openmobisim_core_routes::path_sizes`]), or `None` to work it out over the
    /// alternatives (by time share, the project's extension to itineraries).
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
    /// [`BikeInfrastructure`]'s order; zeros without a bike leg. A ferry crossing counts
    /// in none.
    pub bike_m: [f64; 3],
    /// The wait for the first vehicle, in seconds (part of [`Self::wait_s`]).
    pub wait_first_s: u32,
    /// Seconds on board by kind of service, in [`ServiceKind`]'s order (part of
    /// [`Self::ride_s`]).
    pub ride_kind_s: [u32; 6],
    /// The rides' metres from each boarding stop to its alighting stop as the crow flies: what a
    /// distance fare charges.
    pub ride_m: f64,
    /// A car trip offered only because the trip had no alternative at all: it
    /// takes no car of the traveller's (a lift or a taxi for one without), and is counted apart.
    pub fallback: bool,
    /// A shared bike's pickup and return docks, by the docked service's dock index ([`NO_PARKING`]
    /// for any other alternative).
    pub share_docks: (u32, u32),
    /// A shared bike's expected seconds walked to other docks, the pickup dock empty or the
    /// return dock full (part of [`Self::walk_s`]); 0 for any other alternative.
    pub dock_s: f64,
    /// A taxi's expected share of requests that fail (`fail_prob`); 0 for any other
    /// alternative.
    pub taxi_fail: f64,
    /// A taxi's expected wait should a vehicle come, in seconds; 0 for any other alternative.
    pub taxi_wait_s: f64,
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
            Shape::Share => {
                h.write_u32(100);
                h.write_u32(self.share_docks.0);
                h.write_u32(self.share_docks.1);
            }
            Shape::ShareTransit => {
                h.write_u32(101);
                h.write_u32(self.share_docks.0);
                h.write_u32(self.share_docks.1);
            }
            Shape::Taxi => h.write_u32(102),
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
        h.finish32()
    }

    /// Whether `other` is the same itinerary: what [`Self::identity_hash`] hashes, compared.
    fn same_itinerary(&self, other: &Self) -> bool {
        self.shape == other.shape
            && (self.shape != Shape::Direct || self.mode == other.mode)
            && self.parking == other.parking
            && self.share_docks == other.share_docks
            && self.vehicle_links == other.vehicle_links
            && self.rides == other.rides
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
    /// The longest walk to or from a stop, in seconds: the class's or the run's.
    pub access_walk_s: f64,
    /// The trip's user class index: its coefficient vector under the route cost (S299).
    pub class: u32,
    /// The origin and destination on the run's walk layer, for a shared bike (null without a
    /// docked service, or for a trip not offered one).
    pub share_o: NodeId,
    /// See [`Self::share_o`].
    pub share_d: NodeId,
    /// The walk from origin to destination on the walk layer (each snapped to its nearest walk
    /// node), in seconds, whatever its length: the way on of a taxi request that fails (infinite
    /// without a walk path, or for a trip not offered a taxi).
    pub walk_all_s: f64,
}

/// What a trip may choose from: each mode offered to it where it is.
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
    /// Out through this parking only: a kept itinerary re-costed, not re-planned.
    pub parking: Option<u32>,
    /// A shared bike, dock to dock (S302).
    pub bike_share: bool,
    /// A shared bike to a stop's hub, then transit (S302).
    pub bike_share_transit: bool,
    /// A taxi (S302).
    pub taxi: bool,
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
            Mode::BikeShare => out.bike_share = self.bike_share,
            Mode::BikeShareTransit => out.bike_share_transit = self.bike_share_transit,
            Mode::Taxi => out.taxi = self.taxi,
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
    /// At most this many car routes offered to a trip choosing its mode — the fastest at its
    /// departure, the earlier first among equals (0: every route of the set). The cap stated
    /// car trips get (`choice_max_routes`), applied here too: under a
    /// flat logit a mode's share grew with how many routes its set held.
    pub max_routes: u32,
    /// A car trip's time before its drive (reaching the car) and after it (parking and walking
    /// from it), in seconds (`ModeDefaults::car_access_s`, `car_egress_s`): in a car
    /// alternative's door-to-door time, before a park-and-ride drive and when the car is
    /// fetched, and wherever the car is left (a car park adds its occupancy's time on top).
    pub car_access_s: f64,
    /// See [`Self::car_access_s`].
    pub car_egress_s: f64,
    /// A bike trip's time before its ride and after it, likewise (`ModeDefaults::bike_access_s`,
    /// `bike_egress_s`).
    pub bike_access_s: f64,
    /// See [`Self::bike_access_s`].
    pub bike_egress_s: f64,
    /// The car's route sets, when the run offers mode choice.
    pub car_routes: Option<CarRoutes<'a>>,
    /// The run's route cost for car searches (S299): what the gap's network search, the
    /// park-and-ride drives and the car-park reach minimise per coefficient vector. `None` (or a
    /// cost of time alone): travel time.
    pub car_cost: Option<&'a RouteCost>,
    /// Each coefficient vector's extras prepared for [`Self::car`] (their A* bounds), by vector;
    /// empty where the route cost is time alone.
    pub car_extras: &'a [Option<openmobisim_core_routes::Extras>],
    /// The run's docked service and its docks' expected costs, for shared bikes (S302); `None`
    /// without one.
    pub share: Option<ShareContext<'a>>,
    /// The run's taxi fleet and its expected costs (S302); `None` without one.
    pub taxi: Option<TaxiContext<'a>>,
}

/// A run's taxi fleet as the choice sees it (S302): its defaults and what travellers expect of
/// it (S303).
#[derive(Clone, Copy)]
pub(crate) struct TaxiContext<'a> {
    pub defaults: &'a crate::services::TaxiDefaults,
    pub costs: crate::services::costs::TaxiExpectation,
}

/// A run's docked bike service as the choice sees it (S302): the service, the walk layer its
/// docks are on, which walk nodes hold a dock, and the docks' expected costs.
#[derive(Clone, Copy)]
pub(crate) struct ShareContext<'a> {
    pub setup: &'a ServiceSetup,
    pub walk: &'a LayerSetup,
    pub index: &'a DockIndex,
    pub costs: &'a DockCosts,
    /// The docks at transit stops' hubs, when the run offers [`Mode::BikeShareTransit`].
    pub hubs: Option<&'a HubDocks>,
}

/// Which walk nodes hold a dock of a docked service, and the docks at each: made once per run.
#[derive(Clone, Debug, Default)]
pub(crate) struct DockIndex {
    /// Per walk node, whether a dock is there.
    marks: Vec<bool>,
    /// The docks at a walk node, in dock order.
    at: HashMap<u32, Vec<u32>>,
}

impl DockIndex {
    /// The docks of `setup` (a docked service) on its walk layer of `walk_nodes` nodes.
    pub(crate) fn new(setup: &ServiceSetup, walk_nodes: u32) -> Self {
        let mut out = Self { marks: vec![false; walk_nodes as usize], at: HashMap::new() };
        if let crate::services::ServiceParts::BikeDocked(docks, _) = setup.parts() {
            for (d, w) in docks.walk_node.iter().enumerate() {
                out.marks[w.index()] = true;
                out.at.entry(w.raw()).or_default().push(u32::try_from(d).expect("docks fit u32"));
            }
        }
        out
    }

    /// The docks within `bound` seconds' walk of `node` (`back`: the walks *to* it), nearest
    /// first, ties by dock index, with the walk's seconds.
    fn near(&self, reach: &mut Reach<'_>, node: NodeId, bound: f64, back: bool) -> Vec<(u32, f64)> {
        let found = if back {
            reach.backward(node, bound, &self.marks)
        } else {
            reach.forward(node, bound, &self.marks)
        };
        let mut out = Vec::new();
        for (at, secs) in found {
            for &d in self.at.get(&at.raw()).map_or(&[][..], Vec::as_slice) {
                out.push((d, secs));
            }
        }
        out.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
        out
    }
}

/// A shared-bike trip's dock pair, routed: the walks, the ride and its route. Static (the walk
/// and bike layers' costs are), so found once per run; the docks' expected costs are added
/// per choice.
#[derive(Clone, Debug)]
pub(crate) struct SharePlan {
    pickup: u32,
    ret: u32,
    walk_o_s: f64,
    walk_d_s: f64,
    ride_s: f64,
    links: Box<[LinkId]>,
    metres: f64,
}

/// One trip's dock pairs, kept once found.
pub(crate) type SharePlans = OnceLock<Box<[SharePlan]>>;

/// The docks of a docked service at a transit stop's hub, for a shared bike to transit (S302):
/// a dock joins a stop's hub in the run's one hub set ([`crate::services::hubs::RunHubs`]: by its
/// row's `hub_id`, or within `hub_join_m` of the stop); from such a dock a traveller walks to
/// the hub's stops. Made once per run.
#[derive(Debug)]
pub(crate) struct HubDocks {
    /// Per dock, its hub's stops within the service's `walk_max_s` on foot, with the walk's
    /// whole seconds; empty for a dock at no stop's hub.
    stops: Vec<Vec<(NodeId, u32)>>,
    /// Per bike-layer node, whether a dock with stops is there.
    marks: Vec<bool>,
    /// The docks with stops at a bike-layer node, in dock order.
    at: HashMap<u32, Vec<u32>>,
    /// Per dock, the docks with stops a shared bike reaches from it within `reach_s` on the bike
    /// layer, with the ride's seconds (before `ride_factor`): found on first use, kept for the
    /// run (the bike layer's costs are static).
    reach: Vec<OnceLock<HubReach>>,
    /// The longest ride to a hub's dock, in seconds of the bike layer: bike-and-ride's
    /// `reach_bike_s`.
    reach_s: f64,
    /// The speed the rest of the trip is estimated at, in metres per second, to rank the hubs:
    /// bike-and-ride's `rank_speed_km_h`.
    rank_speed_m_s: f64,
}

impl HubDocks {
    /// The docks of `setup` at the stop hubs of `hubs` (built with `setup`'s
    /// [`ServiceSetup::resource_rows`] as its first rows), with the walks from each to its hub's
    /// stops on `walk` (the run's walk layer, the transit's own), within the service's
    /// `walk_max_s`; `bike_nodes` is the bike layer's node count; `reach_s` and `rank_km_h` are
    /// bike-and-ride's `reach_bike_s` and `rank_speed_km_h`.
    pub(crate) fn new(
        setup: &ServiceSetup,
        hubs: &crate::services::hubs::RunHubs,
        transit: &TransitSetup,
        walk: &LayerSetup,
        bike_nodes: u32,
        (reach_s, rank_km_h): (f64, f64),
    ) -> Self {
        let crate::services::ServiceParts::BikeDocked(docks, d) = setup.parts() else {
            return Self {
                stops: Vec::new(),
                marks: vec![false; bike_nodes as usize],
                at: HashMap::new(),
                reach: Vec::new(),
                reach_s,
                rank_speed_m_s: (rank_km_h / 3.6).max(0.1),
            };
        };
        // Each hub's stops, by hub id.
        let mut hub_stops: HashMap<u32, Vec<NodeId>> = HashMap::new();
        for s in 0..transit.timetable().stop_count() {
            let stop = NodeId::new(s);
            hub_stops.entry(hubs.stop_hub(stop).raw()).or_default().push(stop);
        }
        let graph = walk.network().network();
        let mut reach = Reach::new(graph, walk.seconds());
        let mut wanted = vec![false; graph.node_count() as usize];
        let mut out = Self {
            stops: Vec::with_capacity(docks.len()),
            marks: vec![false; bike_nodes as usize],
            at: HashMap::new(),
            reach: (0..docks.len()).map(|_| OnceLock::new()).collect(),
            reach_s,
            rank_speed_m_s: (rank_km_h / 3.6).max(0.1),
        };
        for dock in 0..docks.len() {
            let i = u32::try_from(dock).expect("docks fit u32");
            let hub = hubs.set().resource_hub(hubs.row_resource(dock)).raw();
            let stops: Vec<NodeId> = hub_stops.get(&hub).cloned().unwrap_or_default();
            let from = docks.walk_node[dock];
            let nodes: Vec<(NodeId, NodeId)> = stops
                .iter()
                .filter_map(|&s| transit.stop_walk_node(s).map(|w| (s, w)))
                .filter(|(_, w)| !w.is_null())
                .collect();
            for &(_, w) in &nodes {
                wanted[w.index()] = true;
            }
            let found: HashMap<u32, f64> = if nodes.is_empty() {
                HashMap::new()
            } else {
                reach
                    .forward(from, d.walk_max_s, &wanted)
                    .into_iter()
                    .map(|(n, t)| (n.raw(), t))
                    .collect()
            };
            for &(_, w) in &nodes {
                wanted[w.index()] = false;
            }
            let mut within: Vec<(NodeId, u32)> = nodes
                .iter()
                .filter_map(|&(s, w)| {
                    let t = if w == from { 0.0 } else { *found.get(&w.raw())? };
                    Some((s, floor_seconds(t)))
                })
                .collect();
            within.sort_unstable();
            if !within.is_empty() {
                let b = docks.bike_node[dock];
                out.marks[b.index()] = true;
                out.at.entry(b.raw()).or_default().push(i);
            }
            out.stops.push(within);
        }
        out
    }

    /// The stops a traveller walks to from `dock`, with the walk's seconds.
    fn stops(&self, dock: u32) -> &[(NodeId, u32)] {
        self.stops.get(dock as usize).map_or(&[][..], Vec::as_slice)
    }

    /// The docks with stops a shared bike reaches from `dock` (at bike-layer node `from`), with
    /// the ride's seconds on the bike layer, nearest first.
    fn reach_from(&self, dock: u32, from: NodeId, bike: &LayerSetup) -> &[(u32, f64)] {
        self.reach[dock as usize].get_or_init(|| {
            let mut reach = Reach::new(bike.network().network(), bike.seconds());
            let mut out: Vec<(u32, f64)> = Vec::new();
            for (node, secs) in reach.forward(from, self.reach_s, &self.marks) {
                for &h in self.at.get(&node.raw()).map_or(&[][..], Vec::as_slice) {
                    out.push((h, secs));
                }
            }
            if self.marks[from.index()] {
                for &h in self.at.get(&from.raw()).map_or(&[][..], Vec::as_slice) {
                    if !out.iter().any(|&(x, _)| x == h) {
                        out.push((h, 0.0));
                    }
                }
            }
            out.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
            out.into_boxed_slice()
        })
    }
}

/// The docks with stops a shared bike reaches from one dock, with the ride's seconds.
type HubReach = Box<[(u32, f64)]>;

/// A candidate pair of a shared bike to a hub: the walk and the ride, the estimate of the whole
/// trip, the pickup dock, the hub's dock, the walk, the ride.
type ChainPair = (f64, f64, u32, u32, f64, f64);

/// A shared bike to a hub's dock, for a trip (S302): the pickup dock and its walk from the
/// origin, the hub's dock, and the ride between them, routed. Static, so found once per run.
#[derive(Clone, Debug)]
pub(crate) struct ChainPlan {
    pickup: u32,
    ret: u32,
    walk_o_s: f64,
    ride_s: f64,
    links: Box<[LinkId]>,
    metres: f64,
}

/// One trip's shared bikes to hubs, kept once found.
pub(crate) type ChainPlans = OnceLock<Box<[ChainPlan]>>;

/// One thread's scratch for planning.
pub(crate) struct Scratch<'a> {
    /// Walks to and from stops, and RAPTOR; `None` without a timetable.
    transit: Option<TransitScratch<'a>>,
    /// The car and bike searches, made on first use: a plain transit trip needs
    /// neither, and each holds a few bytes per link of its whole network.
    car: Option<Search<'a>>,
    bike: Option<Search<'a>>,
    /// The walks to and from docks, made on first use (S302).
    docks: Option<Reach<'a>>,
    /// One flag per road node: the car parks a park-and-ride search looks for. Made on
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
/// metres, and the route.
#[derive(Clone, Debug)]
pub(crate) struct ReachCand {
    node: NodeId,
    seconds: f64,
    links: Box<[LinkId]>,
    metres: f64,
}

/// One trip's bike parkings in reach, kept once found: see [`Itineraries::bike_reach`].
pub(crate) type BikeReach = OnceLock<Box<[ReachCand]>>;

/// Park-and-ride's shared drives ([`crate::parking::ParkingDefaults::reach_car_step_s`]),
/// by road node and step of the day. Searched on one pass's times.
pub(crate) type CarReach = HashMap<DriveKey, SharedDrive>;

/// The shared drives the search trees answered (S299), as a pass reads them.
pub(crate) fn car_reach_of(drives: Vec<TreeDrive>) -> CarReach {
    drives.into_iter().map(|(key, sought, routes)| (key, SharedDrive { sought, routes })).collect()
}

/// The choice sets a choice planned that the gap after it would plan again on the same
/// costs, by trip position: see [`Itineraries::choose`]'s `gap_plans`.
pub(crate) type GapPlans = Vec<Option<Vec<Alternative>>>;

/// One trip's timetable searches, kept for the run (S299): the run plans on one timetable,
/// so a RAPTOR query with the same stops at the same seconds and the same bounds has the
/// same answer whenever it is asked. See [`Itineraries`]' `kept`.
#[derive(Debug, Default)]
pub(crate) struct KeptSearches {
    /// Plain transit: the journeys from the origin at the trip's departure to the destination.
    plain: Option<Box<[Journey]>>,
    /// Through a parking: the last search out of each parking, and the search back to each.
    at: Vec<KeptAt>,
}

/// One kept search through a parking.
#[derive(Debug)]
struct KeptAt {
    /// Back (transit to the parking) or out (from it).
    back: bool,
    parking: u32,
    /// Out: the second the traveller had parked (the access walks start from it); back: 0.
    start: u32,
    /// The arrival bound searched under (journeys arriving before it), `u32::MAX` for none.
    bound: u32,
    journeys: Box<[Journey]>,
}

/// One trip's kept searches, behind a lock (a trip is planned by one thread at a time; the
/// lock is never contended).
pub(crate) type Kept = std::sync::Mutex<KeptSearches>;

/// How the timetable searches of a run's planning went, by kind: plain transit, out of a
/// parking, back to one.
#[derive(Debug, Default)]
pub(crate) struct SearchCounts {
    searched: [std::sync::atomic::AtomicU64; 3],
    reused: [std::sync::atomic::AtomicU64; 3],
}

/// The kinds of [`SearchCounts`].
const PLAIN: usize = 0;
const OUT: usize = 1;
const BACK: usize = 2;

impl SearchCounts {
    fn count(&self, kind: usize, reused: bool) {
        let c = if reused { &self.reused[kind] } else { &self.searched[kind] };
        c.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
}

/// A trip's kept searches and the run's counts, as a plan reads and fills them.
#[derive(Clone, Copy)]
pub(crate) struct KeptFor<'k> {
    kept: &'k Kept,
    counts: &'k SearchCounts,
}

impl KeptFor<'_> {
    fn lock(&self) -> std::sync::MutexGuard<'_, KeptSearches> {
        self.kept.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The plain transit journeys, searched by `search` the first time.
    fn plain(&self, search: impl FnOnce() -> Vec<Journey>) -> Vec<Journey> {
        if let Some(j) = &self.lock().plain {
            self.counts.count(PLAIN, true);
            return j.to_vec();
        }
        let found = search();
        self.counts.count(PLAIN, false);
        self.lock().plain = Some(found.clone().into_boxed_slice());
        found
    }

    /// The journeys through `parking` (`back`, or out of it from `start`) arriving before
    /// `bound`: a kept search with the same parking and start and a bound at least as loose,
    /// its journeys arriving before `bound` (exact: RAPTOR's bound prunes only labels at or
    /// after it, and nothing at or after it leads to an arrival before it); else searched by
    /// `search`, and kept in place of the parking's last.
    fn at(
        &self,
        back: bool,
        parking: u32,
        start: u32,
        bound: u32,
        search: impl FnOnce() -> Vec<Journey>,
    ) -> Vec<Journey> {
        let kind = if back { BACK } else { OUT };
        {
            let kept = self.lock();
            if let Some(k) = kept.at.iter().find(|k| {
                k.back == back && k.parking == parking && k.start == start && k.bound >= bound
            }) {
                self.counts.count(kind, true);
                return k.journeys.iter().filter(|j| j.arrival < bound).cloned().collect();
            }
        }
        let found = search();
        self.counts.count(kind, false);
        let mut kept = self.lock();
        let entry =
            KeptAt { back, parking, start, bound, journeys: found.clone().into_boxed_slice() };
        match kept.at.iter_mut().find(|k| k.back == back && k.parking == parking) {
            Some(k) => *k = entry,
            None => kept.at.push(entry),
        }
        found
    }
}

/// What the kept searches of a run came to: see `Itineraries::timetable_searches`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TimetableSearches {
    /// RAPTOR queries the planning made, by kind: plain transit, out of a parking, back to one.
    pub searched: [u64; 3],
    /// Queries answered from a kept search instead, by kind.
    pub reused: [u64; 3],
    /// Searches kept at the end of the run.
    pub kept: u64,
    /// The bytes they hold, the empty slots of the trips without one included.
    pub kept_bytes: u64,
}

/// One shared drive: leaving a road node at the middle of a step, to the car parks the
/// trips leaving there within the step pick.
pub(crate) struct SharedDrive {
    /// The car-park nodes searched for, ascending.
    sought: Box<[u32]>,
    /// The route to each of them reached within the reach, by node ascending.
    routes: Box<[(u32, Box<[LinkId]>)]>,
}

/// A link's time from a clock time: the wait at an origin or the time to cross a link.
type LinkClock<'f> = dyn Fn(u32, f64) -> f64 + 'f;

/// The car's routes from `origin` to the nodes `targets` marks (at most `wanted` of them) leaving
/// at `departure`, within `bound_s` seconds of driving, on `wait` and `seconds`: `(node, seconds
/// after departure, links)` in the order reached. By time without `extras` (the fastest routes,
/// [`Search::fastest_routes_to`], to the bit); with them, the cheapest by the travel time plus the
/// extras (S299), searched within the bound times the largest share of extras to time a link has
/// and kept within `bound_s` of driving.
#[allow(clippy::too_many_arguments, reason = "the time-dependent search's own arguments")]
fn car_search_to(
    search: &mut Search<'_>,
    extras: Option<&openmobisim_core_routes::Extras>,
    origin: NodeId,
    targets: &[bool],
    wanted: usize,
    departure: f64,
    bound_s: f64,
    (wait, seconds): (&LinkClock<'_>, &LinkClock<'_>),
) -> Vec<(NodeId, f64, Vec<LinkId>)> {
    let Some(e) = extras else {
        return search
            .fastest_routes_to(origin, targets, wanted, departure, bound_s, wait, seconds);
    };
    search.clear_route_state();
    search.set_extras(Some(e.clone()));
    let found = search
        .cheapest_routes_to(
            origin,
            targets,
            wanted,
            departure,
            bound_s * (1.0 + e.max_ratio()),
            wait,
            seconds,
        )
        .into_iter()
        .filter(|(_, _, t, _)| *t <= bound_s)
        .map(|(node, _, t, links)| (node, t, links))
        .collect();
    search.clear_route_state();
    found
}

/// A drive along `links` leaving at `dep`, in seconds on `times`: as the search times it.
fn drive_seconds(times: &LinkTimes, links: &[LinkId], dep: f64) -> f64 {
    let Some(first) = links.first() else { return 0.0 };
    let mut t = dep + times.origin_wait_seconds(first.raw(), dep);
    for l in links {
        t += times.link_seconds(l.raw(), t);
    }
    t - dep
}

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
            docks: None,
            car_mask: Vec::new(),
        }
    }

    /// Every alternative `offer` holds for a trip, finalised: deduplicated, within the
    /// choice-set limit, at most `K` parkings, path sizes set. `legs` are its bike and walk
    /// routes and `car_key` its car pair, for mode choice.
    ///
    /// **A vehicle parked at a parking is fetched** when that can be done: the trip is then
    /// offered only the ways back to it (M4's rule, kept for mode choice: nothing yet weighs
    /// leaving a car at a car park overnight). Only if there is none are the
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
        legs: (&[StaticLeg], Option<&StaticLeg>),
        car_key: Option<RouteKey>,
        bike_reach: Option<&BikeReach>,
        car_reach: &CarReach,
        kept: Option<KeptFor<'_>>,
        (share, chains): (Option<&SharePlans>, Option<&ChainPlans>),
    ) -> Vec<Alternative> {
        let k = self.parking.map_or(1, |p| p.defaults().candidate_count());
        let vehicles =
            [(offer.car_transit, ParkingKind::Car), (offer.bike_transit, ParkingKind::Bike)];
        let mut alts = Vec::new();
        for (shape, kind) in vehicles {
            if let Some(Shape::Back(p)) = shape {
                alts.extend(self.plan_back(s, n, departure, p, kind, kept));
            }
        }
        if !alts.is_empty() {
            self.measure_bike_legs(&mut alts);
            return finalise(alts, self.detour_limit, k, self.journey_max_s());
        }
        let (bike_legs, walk_leg) = legs;
        if let (true, Some(leg)) = (offer.walk, walk_leg) {
            alts.push(direct(Mode::Walk, leg, 0.0, 0.0));
        }
        if offer.bike && !bike_legs.is_empty() {
            // Several bike routes (S298): each an alternative, with the path size over them by
            // length, as the car's routes have theirs, so overlapping routes do not count twice.
            let from = alts.len();
            for leg in bike_legs {
                alts.push(direct(Mode::Bike, leg, self.bike_access_s, self.bike_egress_s));
            }
            if let (true, Some((layer, _))) = (bike_legs.len() > 1, self.bike) {
                let graph = layer.network().network();
                let links: Vec<Vec<u32>> =
                    bike_legs.iter().map(|l| l.links.iter().map(|l| l.raw()).collect()).collect();
                let refs: Vec<&[u32]> = links.iter().map(Vec::as_slice).collect();
                let sizes = openmobisim_core_routes::path_sizes(&refs, |l| {
                    graph.link_length(LinkId::new(l)).get()
                });
                for (a, size) in alts[from..].iter_mut().zip(sizes) {
                    a.fixed_ln_path_size = Some(size.max(1e-9).ln());
                }
            }
        }
        if let (true, Some(key), Some(routes)) = (offer.car, car_key, &self.car_routes) {
            alts.extend(self.plan_car(routes, key, departure));
        }
        if offer.transit {
            alts.extend(self.plan_transit(s, n, departure, kept));
        }
        if let (true, Some(memo)) = (offer.bike_share, share) {
            alts.extend(self.plan_share(s, n, departure, memo));
        }
        if let (true, Some(memo)) = (offer.bike_share_transit, chains) {
            alts.extend(self.plan_share_transit(s, n, departure, destination, memo));
        }
        if let (true, Some(key), Some(routes)) = (offer.taxi, car_key, &self.car_routes) {
            alts.extend(self.plan_taxi(routes, key, n, departure));
        }
        let far_enough = self.far_enough(&offer, origin, destination);
        for (shape, kind) in vehicles {
            if let (Some(Shape::Out), true) = (shape, far_enough) {
                alts.extend(self.plan_out(
                    s,
                    n,
                    departure,
                    destination,
                    kind,
                    (bike_reach, car_reach),
                    offer.parking,
                    kept,
                ));
            }
        }
        self.measure_bike_legs(&mut alts);
        finalise(alts, self.detour_limit, k, self.journey_max_s())
    }

    /// A trip's shared-bike alternatives (S302): its dock pairs, found and routed once per run
    /// ([`Self::share_plans`]), each costed with the docks' expected costs at the seconds it
    /// would reach them.
    fn plan_share(
        &self,
        s: &mut Scratch<'a>,
        n: &TripNodes,
        departure: u32,
        memo: &SharePlans,
    ) -> Vec<Alternative> {
        let Some(share) = self.share else { return Vec::new() };
        let crate::services::ServiceParts::BikeDocked(_, d) = share.setup.parts() else {
            return Vec::new();
        };
        let plans = memo.get_or_init(|| self.share_plans(s, n));
        plans
            .iter()
            .map(|p| {
                let mut a = blank(Mode::BikeShare, Shape::Share);
                a.share_docks = (p.pickup, p.ret);
                a.vehicle_links = p.links.to_vec();
                a.vehicle_s = p.ride_s;
                a.vehicle_m = p.metres;
                a.access_walk_s = floor_seconds(p.walk_o_s);
                a.egress_walk_s = floor_seconds(p.walk_d_s);
                let at_pickup = f64::from(departure) + p.walk_o_s;
                let at_return = at_pickup + d.pickup_s + p.ride_s;
                a.dock_s = share.costs.pickup_s(p.pickup, at_pickup)
                    + share.costs.return_s(p.ret, at_return);
                a.parking_s = d.pickup_s + d.return_s;
                a.walk_s = p.walk_o_s + p.walk_d_s + a.dock_s;
                a.total_s = a.walk_s + a.parking_s + a.vehicle_s;
                a
            })
            .collect()
    }

    /// A trip's dock pairs: of the docks within the service's `walk_max_s` of its origin and of
    /// its destination on the walk layer, the `3K` nearest of each (`K` the service's
    /// `candidates`); of their pairs (two docks apart), the `K` best by an estimate (the walks
    /// and the distance as the crow flies at 4 m/s), each routed on the bike layer by the bike's
    /// cost, as one's own bike is, and timed at the service's `ride_factor`. None without a pair.
    fn share_plans(&self, s: &mut Scratch<'a>, n: &TripNodes) -> Box<[SharePlan]> {
        let (Some(share), Some((bike, bike_ctx))) = (self.share, self.bike) else {
            return Box::new([]);
        };
        let crate::services::ServiceParts::BikeDocked(docks, d) = share.setup.parts() else {
            return Box::new([]);
        };
        if n.share_o.is_null() || n.share_d.is_null() {
            return Box::new([]);
        }
        let k = d.candidate_count();
        let walk = share.walk;
        let reach =
            s.docks.get_or_insert_with(|| Reach::new(walk.network().network(), walk.seconds()));
        let mut from = share.index.near(reach, n.share_o, d.walk_max_s, false);
        let mut to = share.index.near(reach, n.share_d, d.walk_max_s, true);
        from.truncate(3 * k);
        to.truncate(3 * k);
        let mut pairs: Vec<(f64, u32, f64, u32, f64)> = Vec::new();
        for &(po, wo) in &from {
            for &(pd, wd) in &to {
                if po == pd {
                    continue;
                }
                let metres = ground_distance_metres(
                    docks.position[po as usize],
                    docks.position[pd as usize],
                );
                pairs.push((wo + wd + metres / 4.0 * d.ride_factor, po, wo, pd, wd));
            }
        }
        pairs.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)).then(a.3.cmp(&b.3)));
        let seconds = bike.seconds();
        let net = bike.network().network();
        let search = s.bike(bike_ctx);
        let mut out = Vec::new();
        for &(_, po, wo, pd, wd) in pairs.iter().take(k) {
            let Some(route) =
                search.shortest(docks.bike_node[po as usize], docks.bike_node[pd as usize])
            else {
                continue;
            };
            let ride: f64 = route.links.iter().map(|l| seconds[l.index()]).sum();
            let metres: f64 = route.links.iter().map(|&l| net.link_length(l).get()).sum();
            out.push(SharePlan {
                pickup: po,
                ret: pd,
                walk_o_s: wo,
                walk_d_s: wd,
                ride_s: ride * d.ride_factor,
                links: route.links.into_boxed_slice(),
                metres,
            });
        }
        out.into_boxed_slice()
    }

    /// A trip's shared bikes to a stop's hub, then transit (S302), as bike-and-ride out with the
    /// hub's dock for the bike parking: per pair of a pickup dock and a hub's dock
    /// ([`Self::chain_plans`], found once per run), the docks' expected costs at the seconds it
    /// would reach them, then one timetable query from the hub's stops once the bike is
    /// returned, each Pareto journey an alternative. Each query stops at the choice-set limit
    /// past the fastest so far, as bike-and-ride's do.
    fn plan_share_transit(
        &self,
        s: &mut Scratch<'a>,
        n: &TripNodes,
        departure: u32,
        destination: LonLat,
        memo: &ChainPlans,
    ) -> Vec<Alternative> {
        let Some(share) = self.share else { return Vec::new() };
        let Some(hubs) = share.hubs else { return Vec::new() };
        let crate::services::ServiceParts::BikeDocked(_, d) = share.setup.parts() else {
            return Vec::new();
        };
        let plans = memo.get_or_init(|| self.chain_plans(s, n, destination));
        if plans.is_empty() || n.walk_d.is_null() {
            return Vec::new();
        }
        let (Some((transit, _)), Some(ts)) = (self.transit, s.transit.as_mut()) else {
            return Vec::new();
        };
        let egress = transit.egress(&mut ts.into, n.walk_d, n.access_walk_s);
        if egress.is_empty() {
            return Vec::new();
        }
        let transfer = transit.transfer_s();
        let dep = f64::from(departure);
        let mut alts = Vec::new();
        let mut best = f64::INFINITY;
        for p in plans.iter() {
            let at_pickup = dep + p.walk_o_s;
            let at_return = at_pickup + d.pickup_s + p.ride_s;
            let dock_s =
                share.costs.pickup_s(p.pickup, at_pickup) + share.costs.return_s(p.ret, at_return);
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "a second of the day"
            )]
            let returned = (at_return + d.return_s + dock_s).floor() as u32;
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "a second of the day"
            )]
            let before = if self.detour_limit > 0.0 && best.is_finite() {
                (dep + best * (1.0 + self.detour_limit)).floor().min(f64::from(u32::MAX - 2)) as u32
                    + 2
            } else {
                u32::MAX
            };
            let access: Vec<(NodeId, u32)> = hubs
                .stops(p.ret)
                .iter()
                .map(|&(stop, w)| (stop, returned.saturating_add(w).saturating_add(transfer)))
                .collect();
            let journeys: Vec<Journey> = ts
                .raptor
                .pareto_last_walk(&access, &egress, before, last_walk_max(n.access_walk_s))
                .into_iter()
                .filter(|j| !is_loop(j))
                .collect();
            for j in journeys {
                let mut a = self.transit_part(&j, returned);
                a.shape = Shape::ShareTransit;
                a.mode = Mode::BikeShareTransit;
                a.share_docks = (p.pickup, p.ret);
                a.vehicle_links = p.links.to_vec();
                a.vehicle_s = p.ride_s;
                a.vehicle_m = p.metres;
                a.dock_s = dock_s;
                a.parking_s = d.pickup_s + d.return_s;
                a.walk_s += p.walk_o_s + dock_s;
                // The second the traveller reaches the pickup dock: the shared bike's request.
                a.vehicle_departure = at_pickup;
                a.total_s = f64::from(j.arrival) - dep;
                best = best.min(a.total_s);
                alts.push(a);
            }
        }
        alts
    }

    /// A trip's pairs of a pickup dock and a hub's dock (S302): of the docks within the
    /// service's `walk_max_s` of its origin on the walk layer, the `3K` nearest (`K` the
    /// service's `candidates`); from each, the docks at a stop's hub within bike-and-ride's
    /// `reach_bike_s` on the bike layer ([`HubDocks`]); of those pairs (two docks apart), the
    /// `K` nearest by the walk and the ride and the `K` best by an estimate of the whole trip
    /// (the walk, the docks' fixed seconds, the ride, the walk to the hub's nearest stop, and
    /// the rest as the crow flies at bike-and-ride's `rank_speed_km_h`), as bike-and-ride picks
    /// its parkings; each routed on the bike layer by the bike's cost, as one's own bike is,
    /// and timed at the service's `ride_factor`. None without a pair.
    fn chain_plans(
        &self,
        s: &mut Scratch<'a>,
        n: &TripNodes,
        destination: LonLat,
    ) -> Box<[ChainPlan]> {
        let (Some(share), Some((bike, bike_ctx))) = (self.share, self.bike) else {
            return Box::new([]);
        };
        let Some(hubs) = share.hubs else { return Box::new([]) };
        let crate::services::ServiceParts::BikeDocked(docks, d) = share.setup.parts() else {
            return Box::new([]);
        };
        if n.share_o.is_null() {
            return Box::new([]);
        }
        let k = d.candidate_count();
        let walk = share.walk;
        let reach =
            s.docks.get_or_insert_with(|| Reach::new(walk.network().network(), walk.seconds()));
        let mut from = share.index.near(reach, n.share_o, d.walk_max_s, false);
        from.truncate(3 * k);
        // (walk and ride, estimate, pickup, hub's dock, walk, ride)
        let mut pairs: Vec<ChainPair> = Vec::new();
        for &(po, wo) in &from {
            for &(h, secs) in hubs.reach_from(po, docks.bike_node[po as usize], bike) {
                if h == po {
                    continue;
                }
                let Some(&(_, to_stop)) = hubs.stops(h).iter().min_by_key(|x| x.1) else {
                    continue;
                };
                let ride = secs * d.ride_factor;
                let est = wo
                    + d.pickup_s
                    + ride
                    + d.return_s
                    + f64::from(to_stop)
                    + ground_distance_metres(docks.position[h as usize], destination)
                        / hubs.rank_speed_m_s;
                pairs.push((wo + ride, est, po, h, wo, ride));
            }
        }
        let order = |key: fn(&ChainPair) -> f64| {
            let mut idx: Vec<usize> = (0..pairs.len()).collect();
            idx.sort_by(|&a, &b| {
                key(&pairs[a])
                    .total_cmp(&key(&pairs[b]))
                    .then(pairs[a].2.cmp(&pairs[b].2))
                    .then(pairs[a].3.cmp(&pairs[b].3))
            });
            idx
        };
        let mut tried: Vec<usize> = order(|p| p.1).into_iter().take(k).collect();
        for c in order(|p| p.0).into_iter().take(k) {
            if !tried.contains(&c) {
                tried.push(c);
            }
        }
        let seconds = bike.seconds();
        let net = bike.network().network();
        let search = s.bike(bike_ctx);
        let mut out = Vec::new();
        for c in tried {
            let (_, _, po, h, wo, _) = pairs[c];
            let Some(route) =
                search.shortest(docks.bike_node[po as usize], docks.bike_node[h as usize])
            else {
                continue;
            };
            let ride: f64 = route.links.iter().map(|l| seconds[l.index()]).sum();
            let metres: f64 = route.links.iter().map(|&l| net.link_length(l).get()).sum();
            out.push(ChainPlan {
                pickup: po,
                ret: h,
                walk_o_s: wo,
                ride_s: ride * d.ride_factor,
                links: route.links.into_boxed_slice(),
                metres,
            });
        }
        out.into_boxed_slice()
    }

    /// The longest journey with transit offered: the timetable's limit, or none.
    fn journey_max_s(&self) -> f64 {
        self.transit.map_or(f64::INFINITY, |(t, _)| t.defaults().journey_max_s)
    }

    /// Whether a trip offered `offer` is offered the ways out through a parking at all: only
    /// one long enough when choosing (A18; nobody drives to a car park for a 1 km trip). A trip
    /// given the mode is offered it at any length.
    fn far_enough(&self, offer: &Offer, origin: LonLat, destination: LonLat) -> bool {
        !offer.choosing
            || self.parking.is_none_or(|p| {
                ground_distance_metres(origin, destination) >= p.defaults().pr_min_km * 1000.0
            })
    }

    /// The coefficient vector a trip of `n` drives by, and its extras where it has some (S299):
    /// `(0, None)` where the route cost is time alone.
    fn car_vector(&self, n: &TripNodes) -> (u32, Option<&openmobisim_core_routes::Extras>) {
        let Some(cost) = self.car_cost.filter(|c| !c.is_time_only()) else { return (0, None) };
        let v = cost.vector_of_class(n.class as usize);
        (v, self.car_extras.get(v as usize).and_then(Option::as_ref))
    }

    /// The step of the day a park-and-ride drive leaving at `dep` shares its search in
    /// ([`crate::parking::ParkingDefaults::reach_car_step_s`]); `None` when each drive
    /// is searched on its own.
    fn car_step(&self, dep: f64) -> Option<u64> {
        let step = self.parking?.defaults().reach_car_step_s;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "a step count")]
        (step > 0.0).then(|| (dep / step).floor() as u64)
    }

    /// The car parks a park-and-ride trip leaving road node `origin` at `dep` drives to: the
    /// one its kept itinerary went through (`only`), else its candidates
    /// ([`Self::car_candidates`]). Their nodes, ascending.
    fn car_picked(
        &self,
        parking: &ParkingSetup,
        origin: NodeId,
        dep: f64,
        destination: LonLat,
        only: Option<u32>,
    ) -> Vec<NodeId> {
        match only {
            Some(p) => vec![parking.node(p)],
            None => self.car_candidates(parking, origin, dep, destination),
        }
    }

    /// The shared drive a park-and-ride trip of `n` leaving at `dep` would read, and the
    /// car parks it needs from it: `None` when drives are not shared, or the trip drives to none.
    pub(crate) fn car_drive_wanted(
        &self,
        n: &TripNodes,
        dep: f64,
        destination: LonLat,
        only: Option<u32>,
    ) -> Option<(DriveKey, Vec<NodeId>)> {
        let parking = self.parking?;
        let slot = self.car_step(dep)?;
        if n.road_o.is_null() {
            return None;
        }
        let picked = self.car_picked(parking, n.road_o, dep, destination, only);
        let vector = self.car_vector(n).0;
        (!picked.is_empty()).then(|| ((n.road_o.raw(), slot, vector), picked))
    }

    /// Searches the shared drives `wanted` asks for (road node and step, and the car parks
    /// needed) that `reach` does not hold yet, in parallel: one search per road node and
    /// step, leaving at the step's middle, to every car park its trips need; a drive held
    /// already but for fewer car parks is searched again for them all.
    pub(crate) fn fill_car_reach(
        &self,
        wanted: Vec<(DriveKey, Vec<NodeId>)>,
        reach: &mut CarReach,
    ) {
        let Some(parking) = self.parking else { return };
        let mut by_key: std::collections::BTreeMap<DriveKey, Vec<u32>> =
            std::collections::BTreeMap::new();
        for (key, nodes) in wanted {
            by_key.entry(key).or_default().extend(nodes.iter().map(|n| n.raw()));
        }
        let jobs: Vec<(DriveKey, Vec<u32>)> = by_key
            .into_iter()
            .filter_map(|(key, mut nodes)| {
                nodes.sort_unstable();
                nodes.dedup();
                if let Some(held) = reach.get(&key) {
                    if nodes.iter().all(|n| held.sought.binary_search(n).is_ok()) {
                        return None;
                    }
                    nodes.extend_from_slice(&held.sought);
                    nodes.sort_unstable();
                    nodes.dedup();
                }
                Some((key, nodes))
            })
            .collect();
        if jobs.is_empty() {
            return;
        }
        let d = parking.defaults();
        let road = self.car.network;
        let times = self.times;
        let wait = |l: u32, t: f64| times.origin_wait_seconds(l, t);
        let secs = |l: u32, t: f64| times.link_seconds(l, t);
        let found = par_map(
            &jobs,
            16,
            || (Search::new(self.car), vec![false; road.node_count() as usize]),
            |(search, mask), ((node, slot, vector), sought)| {
                #[allow(clippy::cast_precision_loss, reason = "a step count")]
                let departure = (*slot as f64 + 0.5) * d.reach_car_step_s;
                for &n in sought {
                    mask[n as usize] = true;
                }
                let extras = self.car_extras.get(*vector as usize).and_then(Option::as_ref);
                let mut routes: Vec<(u32, Box<[LinkId]>)> = car_search_to(
                    search,
                    extras,
                    NodeId::new(*node),
                    mask,
                    sought.len(),
                    departure,
                    d.reach_car_s,
                    (&wait, &secs),
                )
                .into_iter()
                .map(|(to, _, links)| (to.raw(), links.into_boxed_slice()))
                .collect();
                for &n in sought {
                    mask[n as usize] = false;
                }
                routes.sort_unstable_by_key(|r| r.0);
                SharedDrive {
                    sought: sought.clone().into_boxed_slice(),
                    routes: routes.into_boxed_slice(),
                }
            },
        );
        reach.extend(jobs.into_iter().map(|(key, _)| key).zip(found));
    }

    /// The bike parkings a ride from bike node `from` reaches within the reach, as
    /// `(node, seconds, links, metres)` in the order reached: one static search.
    fn bike_reach_search(
        &self,
        search: &mut Search<'a>,
        from: NodeId,
    ) -> Vec<(NodeId, f64, Vec<LinkId>, f64)> {
        let (Some((layer, ctx)), Some(parking)) = (self.bike, self.parking) else {
            return Vec::new();
        };
        let (targets, count) = parking.targets(ParkingKind::Bike);
        let cost = |l: u32, _t: f64| ctx.link_cost(LinkId::new(l));
        let graph = layer.network().network();
        search
            .fastest_routes_to(
                from,
                targets,
                count,
                0.0,
                parking.defaults().reach_bike_s,
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

    /// Each bike leg's metres by facility: what a model sees of the route's quality. A
    /// ferry crossing is ridden on no facility, so it counts in none (its time is in the
    /// leg's).
    fn measure_bike_legs(&self, alts: &mut [Alternative]) {
        let Some((layer, _)) = self.bike else { return };
        let net = layer.network();
        for a in alts.iter_mut().filter(|a| {
            matches!(
                a.mode,
                Mode::Bike | Mode::BikeTransit | Mode::BikeShare | Mode::BikeShareTransit
            )
        }) {
            for &l in &a.vehicle_links {
                if net.network().link_class(l) != RoadClass::Ferry {
                    a.bike_m[net.infrastructure(l) as usize] += net.network().link_length(l).get();
                }
            }
        }
    }

    /// The car alternatives of a mode-choice trip: its pair's routes, costed on the
    /// expected times, with the path sizes over the routes offered (by length).
    /// The car routes of `key` for a trip that had no alternative, each marked
    /// [`Alternative::fallback`]; none without route sets for it.
    fn fallback_car(&self, key: RouteKey, departure: u32) -> Vec<Alternative> {
        let Some(routes) = &self.car_routes else { return Vec::new() };
        let mut alts = self.plan_car(routes, key, departure);
        for a in &mut alts {
            a.fallback = true;
        }
        // Numbered and trimmed as every choice set is.
        let k = self.parking.map_or(1, |p| p.defaults().candidate_count());
        finalise(alts, self.detour_limit, k, self.journey_max_s())
    }

    /// The seconds after a car trip's departure its car enters the roads: the loading's whole
    /// seconds of `car_access_s`. Every car route is costed from there.
    fn car_start_s(&self) -> f64 {
        self.car_access_s.max(0.0).round()
    }

    /// The time to reach a vehicle of `kind` before its leg.
    fn access_s(&self, kind: ParkingKind) -> f64 {
        match kind {
            ParkingKind::Car => self.car_access_s,
            ParkingKind::Bike => self.bike_access_s,
        }
    }

    /// The time to leave a vehicle of `kind` after its leg, anywhere.
    fn egress_s(&self, kind: ParkingKind) -> f64 {
        match kind {
            ParkingKind::Car => self.car_egress_s,
            ParkingKind::Bike => self.bike_egress_s,
        }
    }

    /// A trip's taxi alternative (S302): the fastest of its car pair's routes at the expected
    /// link times, leaving once the expected wait and boarding are over; the run's expected wait
    /// and failure share (S303); a failure is expected to cost the longest wait and the walk.
    /// None if a failure would leave it no way on (no walk path) while failures are expected,
    /// or without a route.
    fn plan_taxi(
        &self,
        routes: &CarRoutes<'_>,
        key: RouteKey,
        n: &TripNodes,
        departure: u32,
    ) -> Option<Alternative> {
        let taxi = self.taxi?;
        let d = taxi.defaults;
        let k = routes.sets.key_index(key)?;
        let (wait, fail) = taxi.costs.at(n.road_o, f64::from(departure));
        let leave = f64::from(departure) + wait + d.board_s;
        let (r, seconds) = routes
            .sets
            .route_range(k)
            .map(|r| (r, self.times.route_seconds(routes.sets.route(r).links, leave)))
            .min_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)))?;
        if !seconds.is_finite() || (fail > 0.0 && !n.walk_all_s.is_finite()) {
            return None;
        }
        let mut a = blank(Mode::Taxi, Shape::Taxi);
        a.vehicle_links = routes.sets.route(r).links.iter().map(|&l| LinkId::new(l)).collect();
        a.vehicle_s = seconds;
        a.vehicle_m = routes.sets.lengths_m()[r];
        a.vehicle_departure = leave;
        a.taxi_fail = fail;
        a.taxi_wait_s = wait;
        // Served (1 − p): the wait, boarding, the drive; failed (p): the longest wait, the walk.
        a.wait_s = (1.0 - fail) * wait + fail * d.max_wait_s;
        a.parking_s = (1.0 - fail) * d.board_s;
        a.walk_s = if fail > 0.0 { fail * n.walk_all_s } else { 0.0 };
        a.total_s = a.wait_s + a.parking_s + (1.0 - fail) * seconds + a.walk_s;
        // One taxi alternative: a path size of its own.
        a.fixed_ln_path_size = Some(0.0);
        Some(a)
    }

    fn plan_car(&self, routes: &CarRoutes<'_>, key: RouteKey, departure: u32) -> Vec<Alternative> {
        let Some(k) = routes.sets.key_index(key) else { return Vec::new() };
        // The car enters the roads once it is reached: its route is costed from there (S295).
        let at = f64::from(departure) + self.car_start_s();
        let mut alts: Vec<Alternative> = routes
            .sets
            .route_range(k)
            .map(|r| {
                let view = routes.sets.route(r);
                let seconds = self.times.route_seconds(view.links, at);
                let mut a = blank(Mode::Car, Shape::Direct);
                a.vehicle_links = view.links.iter().map(|&l| LinkId::new(l)).collect();
                a.vehicle_s = seconds;
                a.vehicle_m = routes.sets.lengths_m()[r];
                a.vehicle_departure = at;
                // Reaching the car; parking and walking from it (S279).
                a.walk_s = self.car_access_s;
                a.parking_s = self.car_egress_s;
                a.total_s = seconds + self.car_access_s + self.car_egress_s;
                a
            })
            .collect();
        // At most `max_routes` offered, the fastest, the earlier first among equals: the same
        // rule as `route_choice::Chooser::fill` applies to stated car trips (S268, S279).
        let cap = self.max_routes as usize;
        if cap > 0 && alts.len() > cap {
            let mut order: Vec<usize> = (0..alts.len()).collect();
            order.sort_by(|&x, &y| alts[x].vehicle_s.total_cmp(&alts[y].vehicle_s).then(x.cmp(&y)));
            let mut keep = vec![false; alts.len()];
            for &i in &order[..cap] {
                keep[i] = true;
            }
            let mut i = 0;
            alts.retain(|_| {
                i += 1;
                keep[i - 1]
            });
        }
        // The path size over the routes **offered**, by length (S279, CM-3), not over the whole
        // stored set. (An itinerary's is by time share, over the alternatives it competes
        // with: the project's extension, in `finalise`.)
        let road = self.car.network;
        let links: Vec<Vec<u32>> =
            alts.iter().map(|a| a.vehicle_links.iter().map(|l| l.raw()).collect()).collect();
        let refs: Vec<&[u32]> = links.iter().map(Vec::as_slice).collect();
        let sizes =
            openmobisim_core_routes::path_sizes(&refs, |l| road.link_length(LinkId::new(l)).get());
        for (a, size) in alts.iter_mut().zip(sizes) {
            a.fixed_ln_path_size = Some(size.max(1e-9).ln());
        }
        alts
    }

    fn plan_transit(
        &self,
        s: &mut Scratch<'a>,
        n: &TripNodes,
        departure: u32,
        kept: Option<KeptFor<'_>>,
    ) -> Vec<Alternative> {
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
        let mut search = || {
            ts.raptor
                .pareto_last_walk(&access, &egress, u32::MAX, last_walk_max(n.access_walk_s))
                .into_iter()
                .filter(|j| !is_loop(j))
                .collect::<Vec<Journey>>()
        };
        let journeys = match kept {
            Some(kept) => kept.plain(search),
            None => search(),
        };
        journeys
            .iter()
            .map(|j| {
                let mut a = self.transit_part(j, departure);
                a.shape = Shape::Transit;
                a.total_s = f64::from(j.arrival.saturating_sub(departure));
                a
            })
            .collect()
    }

    /// The out itineraries through the candidate parkings, or through `only` alone (a kept
    /// itinerary re-costed: the same search and query as for that parking among the
    /// candidates, so the same numbers).
    #[allow(clippy::too_many_arguments, reason = "a trip's facts and what it is offered")]
    fn plan_out(
        &self,
        s: &mut Scratch<'a>,
        n: &TripNodes,
        departure: u32,
        destination: LonLat,
        kind: ParkingKind,
        (bike_reach, car_reach): (Option<&BikeReach>, &CarReach),
        only: Option<u32>,
        kept: Option<KeptFor<'_>>,
    ) -> Vec<Alternative> {
        let Some(parking) = self.parking else { return Vec::new() };
        let d = parking.defaults();
        let dep = f64::from(departure);
        // The vehicle leaves once it is reached: its drive or ride is costed from there, and
        // the car park's room read when it arrives (S295).
        let leave = dep + self.access_s(kind);
        // The vehicle's reach: `(node, seconds, links, metres)`.
        let found: Vec<(NodeId, f64, Vec<LinkId>, f64)> = match kind {
            ParkingKind::Car => {
                if n.road_o.is_null() {
                    return Vec::new();
                }
                // The candidates are picked on free-flow times to every car park, searched once
                // per run (S209); only their routes are searched on the expected times,
                // by one search that stops once it has reached them all.
                let picked = self.car_picked(parking, n.road_o, leave, destination, only);
                if picked.is_empty() {
                    return Vec::new();
                }
                let road = self.car.network;
                // A drive shared by the trips leaving this node within a step (S267), if it was
                // searched for every car park picked here: its routes to them, each timed at
                // this trip's departure.
                let (vector, extras) = self.car_vector(n);
                let shared = self
                    .car_step(leave)
                    .and_then(|slot| car_reach.get(&(n.road_o.raw(), slot, vector)))
                    .filter(|drive| {
                        picked.iter().all(|node| drive.sought.binary_search(&node.raw()).is_ok())
                    });
                if let Some(drive) = shared {
                    let routes = &drive.routes;
                    picked
                        .iter()
                        .filter_map(|node| {
                            let i = routes.binary_search_by_key(&node.raw(), |r| r.0).ok()?;
                            let links = &routes[i].1;
                            let t = drive_seconds(self.times, links, leave);
                            let m = links.iter().map(|&l| road.link_length(l).get()).sum();
                            (t <= d.reach_car_s).then(|| (*node, t, links.to_vec(), m))
                        })
                        .collect()
                } else {
                    let mut mask = std::mem::take(&mut s.car_mask);
                    mask.resize(road.node_count() as usize, false);
                    for node in &picked {
                        mask[node.index()] = true;
                    }
                    let times = self.times;
                    let wait = |l: u32, t: f64| times.origin_wait_seconds(l, t);
                    let secs = |l: u32, t: f64| times.link_seconds(l, t);
                    let found = car_search_to(
                        s.car(self.car),
                        extras,
                        n.road_o,
                        &mask,
                        picked.len(),
                        leave,
                        d.reach_car_s,
                        (&wait, &secs),
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
            }
            ParkingKind::Bike => {
                let Some((_, ctx)) = self.bike else {
                    return Vec::new();
                };
                if n.bike_o.is_null() {
                    return Vec::new();
                }
                // The bike layer's costs are static, so the search's answer is the trip's for
                // the whole run: found once and kept (S209), pruned to the parkings that can
                // ever be tried, whatever the parkings' availability; trips from one node
                // share one search when `Itineraries::fill_bike_reach` found them first (S259).
                match bike_reach {
                    Some(cell) => cell
                        .get_or_init(|| {
                            let found = self.bike_reach_search(s.bike(ctx), n.bike_o);
                            reach_cands(&found, parking, destination, self.bike_egress_s)
                        })
                        .iter()
                        .map(|c| (c.node, c.seconds, c.links.to_vec(), c.metres))
                        .collect(),
                    None => self.bike_reach_search(s.bike(ctx), n.bike_o),
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
                let arrive = leave + t;
                let availability = self.availability.map_or(1.0, |a| a.at(p, arrive));
                let ps = d.parking_seconds(kind, availability, self.egress_s(kind));
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
        // Reaching the vehicle before its leg.
        let access_s = self.access_s(kind);
        let mut alts: Vec<Alternative> = Vec::new();
        // The fastest so far, in seconds door to door: the choice set drops any itinerary of
        // this mode more than the detour limit slower than its best ([`finalise`]), so each
        // station's search stops there (S267; a second later, against rounding).
        let mut best = f64::INFINITY;
        for c in tried {
            let c = &cands[c];
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "a second of the day"
            )]
            let before = if self.detour_limit > 0.0 && best.is_finite() {
                (dep + best * (1.0 + self.detour_limit)).floor().min(f64::from(u32::MAX - 2)) as u32
                    + 2
            } else {
                u32::MAX
            };
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "a second of the day"
            )]
            let parked = (leave + c.vehicle_s + c.parking_s).floor() as u32;
            let access: Vec<(NodeId, u32)> = parking
                .stops(c.parking)
                .iter()
                .map(|&(stop, w)| (stop, parked.saturating_add(w).saturating_add(transfer)))
                .collect();
            let mut search = || {
                ts.raptor
                    .pareto_last_walk(&access, &egress, before, last_walk_max(n.access_walk_s))
                    .into_iter()
                    .filter(|j| !is_loop(j))
                    .collect::<Vec<Journey>>()
            };
            let journeys = match kept {
                Some(kept) => kept.at(false, c.parking, parked, before, search),
                None => search(),
            };
            for j in journeys {
                let mut a = self.transit_part(&j, parked);
                a.shape = Shape::Out;
                a.mode = transit_mode(kind);
                a.parking = c.parking;
                a.vehicle_links.clone_from(&c.links);
                a.vehicle_s = c.vehicle_s;
                a.vehicle_m = c.metres;
                a.parking_s = c.parking_s;
                a.walk_s += access_s;
                a.vehicle_departure = leave;
                a.total_s = f64::from(j.arrival) - dep;
                best = best.min(a.total_s);
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
                    + d.parking_seconds(ParkingKind::Car, availability, self.car_egress_s)
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
        kept: Option<KeptFor<'_>>,
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
        // Fetching the vehicle at its parking, and leaving it at the destination.
        let (fetch, leave_s) = (self.access_s(kind), self.egress_s(kind));
        let dep = f64::from(departure);
        let from = parking.node(p);
        let mut search = || -> Vec<Journey> {
            ts.raptor.pareto(&access, &egress).into_iter().filter(|j| !is_loop(j)).collect()
        };
        let journeys = match kept {
            Some(kept) => kept.at(true, p, 0, u32::MAX, search),
            None => search(),
        };
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
                    let extras = self.car_vector(n).1;
                    let found = match extras {
                        None => s.car(self.car).fastest_route(
                            from,
                            n.road_d,
                            leave,
                            BACK_BOUND_S,
                            &wait,
                            &secs,
                        ),
                        // By the trip's vector's generalised cost (S299).
                        Some(_) => {
                            let mut mask = std::mem::take(&mut s.car_mask);
                            mask.resize(road.node_count() as usize, false);
                            mask[n.road_d.index()] = true;
                            let one = car_search_to(
                                s.car(self.car),
                                extras,
                                from,
                                &mask,
                                1,
                                leave,
                                BACK_BOUND_S,
                                (&wait, &secs),
                            )
                            .into_iter()
                            .next()
                            .map(|(_, t, links)| (t, links));
                            mask[n.road_d.index()] = false;
                            s.car_mask = mask;
                            one
                        }
                    };
                    found.map(|(t, links)| {
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
            a.parking_s = fetch + leave_s;
            a.vehicle_departure = leave;
            a.total_s = leave + t + leave_s - dep;
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

/// Which parkings of `found` (`(node, seconds, links, metres)`, as the bike-and-ride search
/// returns them) can ever be among the ones [`Planner::plan_out`] tries: the `K`
/// nearest by riding time, or one of the `K` best by its estimate of the whole trip,
/// whatever the parkings' availability. Availability moves an estimate only through
/// the time to park, between its floor (all room) and floor plus slope (full), so a
/// parking whose best estimate is above the `K`-th smallest worst estimate is never among
/// the `K` best, and one more than `K` others are nearer than is never among the nearest.
/// Exact: the kept list gives the same tried set as the whole list, at any availability.
fn prune_reach(
    found: &[(NodeId, f64, Vec<LinkId>, f64)],
    parking: &ParkingSetup,
    kind: ParkingKind,
    destination: LonLat,
    egress_s: f64,
) -> Vec<bool> {
    let d = parking.defaults();
    let k = d.candidate_count();
    let speed = (d.rank_speed_km_h / 3.6).max(0.1);
    let (all_room, full) =
        (d.parking_seconds(kind, 1.0, egress_s), d.parking_seconds(kind, 0.0, egress_s));
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
        return vec![true; found.len()];
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
    keep
}

/// A bike-and-ride trip's reach as kept for the run: the parkings of `found` that
/// [`prune_reach`] keeps for a trip to `destination`, in the order found.
fn reach_cands(
    found: &[(NodeId, f64, Vec<LinkId>, f64)],
    parking: &ParkingSetup,
    destination: LonLat,
    bike_egress_s: f64,
) -> Box<[ReachCand]> {
    let keep = prune_reach(found, parking, ParkingKind::Bike, destination, bike_egress_s);
    found
        .iter()
        .zip(keep)
        .filter(|&(_, k)| k)
        .map(|((node, seconds, links, metres), _)| ReachCand {
            node: *node,
            seconds: *seconds,
            links: links.clone().into_boxed_slice(),
            metres: *metres,
        })
        .collect()
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
        fallback: false,
        share_docks: (NO_PARKING, NO_PARKING),
        dock_s: 0.0,
        taxi_fail: 0.0,
        taxi_wait_s: 0.0,
    }
}

/// Seconds as whole seconds, rounded down (a walk's, for its attribute in minutes).
fn floor_seconds(s: f64) -> u32 {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "seconds")]
    let out = s.max(0.0).floor().min(f64::from(u32::MAX - 1)) as u32;
    out
}

/// A bike route or a walk as an alternative: a bike pays `access_s` before its ride (as walk
/// time) and `egress_s` after it (as parking time); a walk pays neither.
fn direct(mode: Mode, leg: &StaticLeg, access_s: f64, egress_s: f64) -> Alternative {
    let mut a = blank(mode, Shape::Direct);
    a.vehicle_links.clone_from(&leg.links);
    a.vehicle_m = leg.metres;
    if mode == Mode::Walk {
        a.walk_s = leg.seconds;
        a.total_s = leg.seconds;
    } else {
        a.vehicle_s = leg.seconds;
        a.walk_s = access_s;
        a.parking_s = egress_s;
        a.total_s = leg.seconds + access_s + egress_s;
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
/// The limits are per mode because the modes' times are not alike: a walk is always
/// slower than a drive, and whether it is a real alternative is for the model to weigh
/// (with its mode constants), not for a time limit.
fn finalise(
    mut alts: Vec<Alternative>,
    detour_limit: f64,
    parkings: usize,
    journey_max_s: f64,
) -> Vec<Alternative> {
    // A journey with transit longer than the limit is no alternative (S269).
    alts.retain(|a| a.rides.is_empty() || a.total_s <= journey_max_s);
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
    // A duplicate has the same identity and the same contents: two different itineraries whose
    // 32-bit identities collide are both kept (X-28).
    let mut unique: Vec<Alternative> = Vec::with_capacity(alts.len());
    for a in alts {
        if !unique.iter().any(|u| u.identity == a.identity && u.same_itinerary(&a)) {
            unique.push(a);
        }
    }
    let mut alts = unique;
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
    // leg is keyed by its links and its mode (the layers number their links each on their
    // own), not by the parking it ends at: two parkings at one node share one drive.
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
            e.push(((0, 0, hash, a.mode.index() as u32), a.vehicle_s));
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

/// The longest last walk of an itinerary, in whole seconds: the traveller's `access_walk_max_s`
/// bounds the transfer walk and the egress walk **together**. A walk limit that is
/// not finite or not positive leaves the search to its own reach (`u32::MAX`: none).
fn last_walk_max(access_walk_s: f64) -> u32 {
    if access_walk_s.is_finite() && access_walk_s > 0.0 {
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "seconds, clamped below u32::MAX"
        )]
        let s = access_walk_s.floor().min(f64::from(u32::MAX - 1)) as u32;
        s
    } else {
        u32::MAX
    }
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
        Mode::Bike | Mode::BikeTransit | Mode::BikeShare | Mode::BikeShareTransit => {
            Some(ValueLayer::Bike)
        }
        Mode::Walk => Some(ValueLayer::Walk),
        Mode::Transit => None,
        Mode::Taxi => Some(ValueLayer::Road),
    }
}

/// Whether `a` arrives at its trip's destination by car (a car route, or the drive home from a
/// park-and-ride), so pays the parking there.
fn arrives_by_car(a: &Alternative) -> bool {
    a.mode == Mode::Car || (a.mode == Mode::CarTransit && matches!(a.shape, Shape::Back(_)))
}

/// What `a`, an alternative of `trip`, costs, by part ([`crate::prices`]): its car or bike
/// leg's running cost, its car leg's tolls, the fee of the parking it leaves its vehicle at (out;
/// a trip back fetches a vehicle already paid for) or, arriving by car, the parking fee at the
/// trip's destination, and its rides' fare.
fn cost(a: &Alternative, inputs: &ChooseInputs<'_>, trip: TripId) -> Cost {
    let (values, prices) = (inputs.link_values, inputs.prices);
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
        } else if arrives_by_car(a) {
            inputs.demand_values.destination_parking(trip)
        } else {
            0.0
        },
        // A pass holder pays no fare (S279, CM-15): the pass is access, not money, and its own
        // price is not counted (Q-7).
        fare: if a.mode == Mode::Taxi {
            // A taxi's fare (S302): every traveller pays it, a transit pass covers none; the
            // share that fails pays nothing.
            (1.0 - a.taxi_fail) * prices.taxi(a.vehicle_m)
        } else if a.mode == Mode::BikeShare {
            // A shared bike's price (S302): every traveller pays it, a transit pass covers none.
            prices.bike_share(a.vehicle_s)
        } else if inputs.travellers.ownership(inputs.trips.traveller(trip)).transit_pass {
            // A shared bike to transit: the bike's price, and no fare (S302).
            if a.mode == Mode::BikeShareTransit { prices.bike_share(a.vehicle_s) } else { 0.0 }
        } else if a.mode == Mode::BikeShareTransit {
            prices.bike_share(a.vehicle_s) + prices.fare(a.rides.len(), a.ride_m)
        } else {
            prices.fare(a.rides.len(), a.ride_m)
        },
    }
}

/// The value of attribute `name` for `a`, an alternative of `traveller`'s `trip`, the best total
/// of its mode in its set being `best`; the user's link values, the run's prices and
/// the user's traveller and trip values are `inputs`'.
fn attribute(
    name: &str,
    a: &Alternative,
    best: f64,
    inputs: &ChooseInputs<'_>,
    traveller: TravellerId,
    trip: TripId,
) -> f64 {
    let values = inputs.link_values;
    if name.starts_with("cost_") {
        if let Some(v) = cost(a, inputs, trip).attribute(name) {
            return v;
        }
    }
    if let Some(source) = inputs.demand_values.lookup(name) {
        return inputs.demand_values.value(source, inputs.trips, traveller, trip);
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
    let kind = vehicle_of(a.mode);
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
        "car_min" if kind == Some(ParkingKind::Car) || a.mode == Mode::Taxi => vehicle_min,
        "fail_prob" => a.taxi_fail,
        "bike_min"
            if kind == Some(ParkingKind::Bike)
                || matches!(a.mode, Mode::BikeShare | Mode::BikeShareTransit) =>
        {
            vehicle_min
        }
        "dock_min" => a.dock_s / 60.0,
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

/// The attributes a model reads, checked against [`ATTRIBUTES`] and those of the user's link,
/// traveller and trip values (`extra`), in that order.
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
    /// The logsum of its choice set when it was last planned (`NaN` if the model gives
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
    /// Per mode: Σ w of those trips.
    pub weight: [f64; Mode::COUNT],
    /// Weights: all assessed, chose again, changed, had to choose again (their
    /// alternative had gone).
    pub w_all: f64,
    pub w_reselected: f64,
    pub w_changed: f64,
    pub w_forced: f64,
    /// Mode choice: the weight of the trips choosing their mode that had a choice
    /// before, and of those whose mode changed.
    pub w_choosing: f64,
    pub w_mode_changed: f64,
    /// Kept itineraries re-costed at their parking, not re-planned.
    pub recosted: u32,
    /// Per mode: Σ w · the time the choice model expects of the trip within the mode it kept
    /// (its probabilities over that mode's alternatives), for the trips in `paid`.
    pub expected_paid: [f64; Mode::COUNT],
    /// Whether the model gave probabilities for every batch (else `expected_paid` is not
    /// measured).
    pub expected_measured: bool,
    /// Σ w · (1 − the probability the model gives the mode kept) over the trips choosing their
    /// mode that chose again: the mode changes chance alone would make (I-al's floor).
    pub w_mode_floor: f64,
    /// The route gap's sums over the car routes of the trips choosing their mode that drove
    /// this loading and finished (S302; [`CarGap`]): each one's route against the car routes of
    /// its set, a route no longer in the set re-timed on these times. Empty without a
    /// [`CarGap`].
    pub car_routes: RouteGapSums,
}

/// What [`Itineraries::choose`] needs to measure the car routes of the trips choosing their
/// mode, for the route gap the stopping rule reads (S302).
#[derive(Clone, Copy)]
pub(crate) struct CarGap<'a> {
    /// Per trip of the run: whether it was still under way when the window ended (left out, as
    /// the route choice leaves its own).
    pub unfinished: &'a [bool],
    /// The currency of the generalised sums, when the stopping rule reads them.
    pub currency: Option<&'a Currency>,
}

impl Assessment {
    /// The itinerary disequilibrium over every mode: the gap less what the choice
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

    /// The floor of [`Self::mode_changed_share`]: the share whose mode chance alone
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

    /// The share of the itineraries' gap with transit that the planned-against-realised
    /// difference makes (S299): `e / (g + e)`, `e` the mean seconds a transit part ended later
    /// than on the timetable it was planned on (`plan_error_s`, over the trips that rode), `g`
    /// the mean seconds the transit, park-and-ride and bike-and-ride trips assessed paid above the
    /// best of their mode on the costs they planned with. 0 when `e` is 0 (the buses kept to
    /// the timetable); NaN when `e` is NaN or nothing with transit was assessed.
    pub(crate) fn plan_error_share(&self, plan_error_s: f64) -> f64 {
        let ms = [Mode::Transit, Mode::CarTransit, Mode::BikeTransit].map(Mode::index);
        let w: f64 = ms.iter().map(|&m| self.weight[m]).sum();
        if plan_error_s.is_nan() || w <= 0.0 {
            return f64::NAN;
        }
        if plan_error_s == 0.0 {
            return 0.0;
        }
        let g = ms.iter().map(|&m| self.paid[m] - self.least[m]).sum::<f64>() / w;
        plan_error_s / (g + plan_error_s)
    }

    /// The share of the trips choosing their mode whose mode changed, NaN if none chose
    /// before.
    pub(crate) fn mode_changed_share(&self) -> f64 {
        if self.w_choosing > 0.0 { self.w_mode_changed / self.w_choosing } else { f64::NAN }
    }
}

/// The itinerary trips of a run: which they are, their nodes, and the rounds
/// they are chosen in. With mode choice, also every trip that chooses its mode.
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
    /// Per itinerary trip: its bike routes as legs (several with bike route sets, S298) and its
    /// walk leg, for mode choice.
    legs: Vec<(Vec<StaticLeg>, Option<StaticLeg>)>,
    /// Per position, a choosing trip's car pair (`None`: origin and destination on one node,
    /// or not choosing).
    car_keys: Vec<Option<RouteKey>>,
    /// Per position, its bike parkings in reach, found by the first plan that needs them and
    /// kept for the run: the bike layer's costs are static.
    bike_reach: Vec<BikeReach>,
    /// Set once [`Self::fill_bike_reach`] has run.
    bike_reach_filled: OnceLock<()>,
    /// Per position, its shared-bike dock pairs (S302), found by the first plan that needs them
    /// and kept for the run: the walk and bike layers' costs are static. Empty without a docked
    /// service.
    share: Vec<SharePlans>,
    /// Per position, its shared bikes to a stop's hub (S302), likewise. Empty unless the run
    /// offers [`Mode::BikeShareTransit`].
    chains: Vec<ChainPlans>,
    /// Per position, its timetable searches, kept for the run ([`KeptSearches`]): valid on the
    /// one timetable the run plans on, which `kept_on` names (its address) once a plan used it;
    /// a planner on another timetable searches without them.
    kept: Vec<Kept>,
    kept_on: OnceLock<usize>,
    /// How the kept searches were used.
    search_counts: SearchCounts,
    /// Whether any trip may drive to a car park (stated park-and-ride, or offered it in mode
    /// choice): the car parks' free-flow table is then searched before planning.
    parks_cars: bool,
    /// The modes each traveller class may use, by class index; empty: every class all.
    class_modes: Vec<[bool; Mode::COUNT]>,
    /// Whether a trip with no alternative travels by car: then every trip has its
    /// car pair in `car_keys`, chooser or not.
    fallback_car: bool,
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
        Mode::Walk | Mode::Transit | Mode::BikeShare | Mode::BikeShareTransit | Mode::Taxi => None,
    }
}

/// What choosing needs besides the planner.
pub(crate) struct ChooseInputs<'a> {
    pub trips: &'a Trips,
    pub travellers: &'a Travellers,
    pub model: &'a dyn ChoiceModel,
    pub rng: &'a StreamRng,
    pub wanted: &'a [String],
    /// The user's link values.
    pub link_values: &'a PreparedLinkValues,
    /// The run's prices.
    pub prices: &'a PreparedPrices,
    /// The user's traveller and trip values.
    pub demand_values: &'a DemandValues,
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
    /// How long a walk or ride mode choice offers.
    pub modes: &'a crate::layers::ModeDefaults,
    /// The modes each traveller class may use, by class index; empty: every class all.
    pub class_modes: &'a [[bool; Mode::COUNT]],
    /// Each traveller class's own limits, by class index; empty: the run's.
    pub class_limits: &'a [crate::layers::ClassLimits],
    /// The travellers, for each trip's class.
    pub travellers: &'a Travellers,
    /// The run's docked bike service (S302), if it has one.
    pub share: Option<&'a ServiceSetup>,
    /// The run's taxi fleet (S302), if it has one.
    pub taxi: Option<&'a ServiceSetup>,
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
            Mode::BikeShare => sim.share.is_some() && bike.is_some() && sim.layers.walk.is_some(),
            Mode::BikeShareTransit => {
                sim.share.is_some()
                    && transit.is_some()
                    && bike.is_some()
                    && sim.layers.walk.is_some()
            }
            // A taxi request that fails walks on: the walk layer is needed.
            Mode::Taxi => sim.taxi.is_some() && sim.layers.walk.is_some(),
        };
        let mut offered = [false; Mode::COUNT];
        for &m in sim.choice.unwrap_or(&[]) {
            offered[m.index()] = simulated(m);
        }
        let road_snapper = NodeSnapper::new(road);
        let bike_snapper = bike.map(|b| NodeSnapper::every_node(b.network().network()));
        // Origins and destinations on the walk layer, for a shared bike (S302).
        let share_walk = sim
            .layers
            .walk
            .as_ref()
            .filter(|_| offered[Mode::BikeShare.index()] || offered[Mode::BikeShareTransit.index()])
            .map(|w| (w, NodeSnapper::every_node(w.network().network())));
        // The walk of a taxi request that fails (S302): the walk layer's shortest time, whatever
        // its length (the trips' own walk routes stop at the longest walk offered), one search
        // per origin node to all its trips' destinations, once per run.
        let taxi_walk = sim
            .layers
            .walk
            .as_ref()
            .filter(|_| offered[Mode::Taxi.index()])
            .map(|w| (w, NodeSnapper::every_node(w.network().network())));
        let mut taxi_pairs: HashMap<u32, Vec<(usize, NodeId)>> = HashMap::new();
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
            bike_reach_filled: OnceLock::new(),
            share: Vec::new(),
            chains: Vec::new(),
            kept: Vec::new(),
            kept_on: OnceLock::new(),
            search_counts: SearchCounts::default(),
            parks_cars: false,
            class_modes: sim.class_modes.to_vec(),
            fallback_car: sim.modes.fallback_car,
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
                class: sim.travellers.user_class(trips.traveller(trip)).raw(),
                share_o: null,
                share_d: null,
                walk_all_s: f64::INFINITY,
            };
            if let (true, Some((w, snapper))) = (choosing, &taxi_walk) {
                let graph = w.network().network();
                let (from, to) = (snapper.nearest(graph, o), snapper.nearest(graph, d));
                taxi_pairs.entry(from.raw()).or_default().push((out.trips.len(), to));
            }
            if let (true, Some((w, snapper))) = (choosing, &share_walk) {
                let graph = w.network().network();
                nodes.share_o = snapper.nearest(graph, o);
                nodes.share_d = snapper.nearest(graph, d);
            }
            if choosing || mode == Mode::CarTransit || out.fallback_car {
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
            // The car pair: a chooser's, offered the car; with the fallback (S271), everyone's.
            let key = ((choosing && (offered[Mode::Car.index()] || offered[Mode::Taxi.index()]))
                || out.fallback_car)
                && nodes.road_o != nodes.road_d;
            let car_key = key.then(|| RouteKey::new(nodes.road_o, nodes.road_d));
            let legs = if choosing {
                let max = limits.modes(sim.modes);
                (
                    sim.static_routes.legs(
                        sim.layers,
                        trip,
                        StaticLayer::Bike,
                        max.max_seconds(StaticLayer::Bike),
                    ),
                    sim.static_routes.leg(
                        sim.layers,
                        trip,
                        StaticLayer::Walk,
                        max.max_seconds(StaticLayer::Walk),
                    ),
                )
            } else {
                (Vec::new(), None)
            };
            out.pos[i] = u32::try_from(out.trips.len()).expect("trips fit u32");
            out.trips.push(trip);
            out.nodes.push(nodes);
            out.choosing.push(choosing);
            out.legs.push(legs);
            out.car_keys.push(car_key);
        }
        if let Some((w, _)) = &taxi_walk {
            let graph = w.network().network();
            let mut reach = Reach::new(graph, w.seconds());
            let mut wanted = vec![false; graph.node_count() as usize];
            let mut origins: Vec<u32> = taxi_pairs.keys().copied().collect();
            origins.sort_unstable();
            for from in origins {
                let pairs = &taxi_pairs[&from];
                for &(_, to) in pairs {
                    wanted[to.index()] = true;
                }
                let found: HashMap<u32, f64> = reach
                    .forward(NodeId::from_raw(from), f64::INFINITY, &wanted)
                    .into_iter()
                    .map(|(n, s)| (n.raw(), s))
                    .collect();
                for &(p, to) in pairs {
                    wanted[to.index()] = false;
                    out.nodes[p].walk_all_s = if to.raw() == from {
                        0.0
                    } else {
                        found.get(&to.raw()).copied().unwrap_or(f64::INFINITY)
                    };
                }
            }
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
        if share_walk.is_some() {
            out.share = (0..out.trips.len()).map(|_| OnceLock::new()).collect();
        }
        if offered[Mode::BikeShareTransit.index()] {
            out.chains = (0..out.trips.len()).map(|_| OnceLock::new()).collect();
        }
        out.kept = (0..out.trips.len()).map(|_| Kept::default()).collect();
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

    /// Whether the trip at `position` chooses its mode, rather than was given it.
    pub(crate) fn chooses(&self, position: usize) -> bool {
        self.choosing[position]
    }

    /// A trip's nodes.
    pub(crate) fn nodes(&self, position: usize) -> &TripNodes {
        &self.nodes[position]
    }

    /// The kept searches of the trip at `position`, if `planner` plans on the timetable they
    /// were kept on (the first a plan used).
    fn kept_for(&self, planner: &Planner<'_>, position: usize) -> Option<KeptFor<'_>> {
        let (_, data) = planner.transit?;
        let id = std::ptr::from_ref(data) as usize;
        (*self.kept_on.get_or_init(|| id) == id)
            .then(|| KeptFor { kept: &self.kept[position], counts: &self.search_counts })
    }

    /// What the kept timetable searches came to so far.
    pub(crate) fn timetable_searches(&self) -> TimetableSearches {
        use std::sync::atomic::Ordering::Relaxed;
        let mut out = TimetableSearches::default();
        for k in 0..3 {
            out.searched[k] = self.search_counts.searched[k].load(Relaxed);
            out.reused[k] = self.search_counts.reused[k].load(Relaxed);
        }
        let journeys = |js: &[Journey]| -> usize {
            js.iter()
                .map(|j| {
                    std::mem::size_of::<Journey>()
                        + j.legs.capacity() * std::mem::size_of::<JourneyLeg>()
                })
                .sum()
        };
        let mut bytes = self.kept.len() * std::mem::size_of::<Kept>();
        for kept in &self.kept {
            let kept = kept.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(j) = &kept.plain {
                out.kept += 1;
                bytes += journeys(j);
            }
            out.kept += kept.at.len() as u64;
            bytes += kept.at.capacity() * std::mem::size_of::<KeptAt>();
            bytes += kept.at.iter().map(|k| journeys(&k.journeys)).sum::<usize>();
        }
        out.kept_bytes = bytes as u64;
        out
    }

    /// The car pair of the trip at `position` if it chooses its mode and may drive, for
    /// its route set.
    pub(crate) fn car_key(&self, position: usize) -> Option<RouteKey> {
        self.car_keys[position]
    }

    /// The trips the last choice (`chosen`) left with no alternative, or with only the car
    /// they fell back to, and why each mode gave them none. Run once at
    /// the end, over those trips alone: what they were offered is worked out again from their
    /// day's choices, and a transit trip's stops in reach are the walks its planning searched
    /// and kept (`TransitSetup::access`'s memo), so nothing is searched again.
    pub(crate) fn explain_none(
        &self,
        trips: &Trips,
        travellers: &Travellers,
        chosen: &Chosen,
        transit: Option<&TransitSetup>,
        parking: Option<&ParkingSetup>,
        car_sets: Option<&RouteSets>,
    ) -> Vec<NoAlternative> {
        use NoAlternativeReason as R;
        let mut reach = transit.map(|t| Reach::new(t.walk().network(), t.walk_link_seconds()));
        let mut out = Vec::new();
        for (p, &trip) in self.trips.iter().enumerate() {
            let fallback = match &chosen.alt[p] {
                None => false,
                Some(a) if a.fallback => true,
                Some(_) => continue,
            };
            let origin = trips.origin(trip);
            let shape = |kind| match self.vehicle_at(trips, travellers, trip, kind, &chosen.alt) {
                Loc::Parked(parking) => Some(Shape::Back(parking)),
                Loc::At(o) if o == origin => Some(Shape::Out),
                _ => None,
            };
            let n = &self.nodes[p];
            let departure = trips.departure(trip).get();
            // Transit's reason: no stop within the walk at either end, or no journey.
            let mut transit_reason = || match (transit, reach.as_mut()) {
                (Some(t), Some(r)) if !n.walk_o.is_null() && !n.walk_d.is_null() => {
                    let none = t.access(r, n.walk_o, departure, n.access_walk_s).is_empty()
                        || t.egress(r, n.walk_d, n.access_walk_s).is_empty();
                    if none { R::NoStopInReach } else { R::NoTransitJourney }
                }
                _ => R::NoStopInReach,
            };
            let mut reasons = [R::NotOffered; Mode::COUNT];
            let choosing = self.choosing[p];
            if choosing {
                let class = travellers.user_class(trips.traveller(trip)).index();
                let on = |m: Mode| {
                    self.offered[m.index()]
                        && self.class_modes.get(class).is_none_or(|c| c[m.index()])
                };
                let (bike_legs, walk_leg) = &self.legs[p];
                if on(Mode::Walk) {
                    reasons[Mode::Walk.index()] =
                        if walk_leg.is_none() { R::WalkTooLong } else { R::Unexplained };
                }
                if on(Mode::Bike) {
                    reasons[Mode::Bike.index()] =
                        match (shape(ParkingKind::Bike), bike_legs.is_empty()) {
                            (Some(Shape::Out), true) => R::RideTooLong,
                            (Some(Shape::Out), false) => R::Unexplained,
                            _ => R::NoVehicle,
                        };
                }
                if on(Mode::Car) {
                    let routes = |key: RouteKey| {
                        car_sets
                            .and_then(|s| s.key_index(key).map(|k| s.route_range(k)))
                            .is_some_and(|r| !r.is_empty())
                    };
                    reasons[Mode::Car.index()] = match (shape(ParkingKind::Car), self.car_keys[p]) {
                        (Some(Shape::Out), Some(key)) if routes(key) => R::Unexplained,
                        (Some(Shape::Out), _) => R::NoRoadRoute,
                        _ => R::NoVehicle,
                    };
                }
                if on(Mode::Transit) {
                    reasons[Mode::Transit.index()] = transit_reason();
                }
                // A shared bike gives nothing only when no two docks are within the walk of
                // the origin and of the destination (or no ride joins them) (S302).
                if on(Mode::BikeShare) {
                    reasons[Mode::BikeShare.index()] =
                        if self.share.get(p).and_then(OnceLock::get).is_none_or(|p| p.is_empty()) {
                            R::NoDockInReach
                        } else {
                            R::Unexplained
                        };
                }
                // A taxi gives nothing without a road route, or when a request is expected to
                // fail with no walk to fall back on (S302).
                if on(Mode::Taxi) {
                    reasons[Mode::Taxi.index()] =
                        if self.car_keys[p].is_none() { R::NoRoadRoute } else { R::Unexplained };
                }
                // A shared bike to transit: no dock in the walk with a hub's dock in the ride,
                // else no journey from the hub (S302).
                if on(Mode::BikeShareTransit) {
                    reasons[Mode::BikeShareTransit.index()] = if self
                        .chains
                        .get(p)
                        .and_then(OnceLock::get)
                        .is_none_or(|p| p.is_empty())
                    {
                        R::NoDockInReach
                    } else {
                        R::NoTransitJourney
                    };
                }
                let far_enough = parking.is_none_or(|pk| {
                    ground_distance_metres(origin, trips.destination(trip))
                        >= pk.defaults().pr_min_km * 1000.0
                });
                for (mode, kind) in
                    [(Mode::CarTransit, ParkingKind::Car), (Mode::BikeTransit, ParkingKind::Bike)]
                {
                    // A vehicle parked at a parking is offered back whatever the modes asked
                    // for (`offer_of`).
                    reasons[mode.index()] = match shape(kind) {
                        Some(Shape::Back(_)) => R::NoTransitJourney,
                        _ if !on(mode) => R::NotOffered,
                        Some(Shape::Out) if !far_enough => R::TooShort,
                        Some(Shape::Out) => R::NoTransitJourney,
                        _ => R::NoVehicle,
                    };
                }
            } else {
                let mode = trips.mode(trip);
                reasons[mode.index()] = match parking_kind_of(mode) {
                    None => transit_reason(),
                    Some(kind) => match shape(kind) {
                        Some(_) => R::NoTransitJourney,
                        None => R::NoVehicle,
                    },
                };
            }
            out.push(NoAlternative { trip, choosing, reasons, fallback_car: fallback });
        }
        out
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
                    // A fallback car trip takes no car of the traveller's (CM-4): theirs stays.
                    Some(a) if a.fallback => continue,
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
        let (bike_legs, walk_leg) = &self.legs[position];
        Offer {
            choosing: true,
            walk: on(Mode::Walk) && walk_leg.is_some(),
            bike: on(Mode::Bike) && bike == Some(Shape::Out) && !bike_legs.is_empty(),
            car: on(Mode::Car) && car == Some(Shape::Out) && self.car_keys[position].is_some(),
            transit: on(Mode::Transit),
            car_transit: via(car, Mode::CarTransit),
            bike_transit: via(bike, Mode::BikeTransit),
            parking: None,
            bike_share: on(Mode::BikeShare),
            bike_share_transit: on(Mode::BikeShareTransit),
            taxi: on(Mode::Taxi) && self.car_keys[position].is_some(),
        }
    }

    /// The choice set of the trip at `position` for `offer`, planned on `planner`'s costs with
    /// the shared drives `car_reach`: what it is offered, or, with nothing at all, the car
    /// (S271, W-1) if it has a road path.
    fn plan_trip<'a>(
        &self,
        planner: &Planner<'a>,
        trips: &Trips,
        s: &mut Scratch<'a>,
        position: usize,
        offer: Offer,
        car_reach: &CarReach,
    ) -> Vec<Alternative> {
        let p = position;
        let trip = self.trips[p];
        let (bike, walk) = &self.legs[p];
        let set = if offer.is_empty() {
            Vec::new()
        } else {
            planner.plan(
                s,
                &self.nodes[p],
                trips.departure(trip).get(),
                trips.origin(trip),
                trips.destination(trip),
                offer,
                (bike.as_slice(), walk.as_ref()),
                self.car_keys[p],
                self.bike_reach.get(p),
                car_reach,
                self.kept_for(planner, p),
                (self.share.get(p), self.chains.get(p)),
            )
        };
        // No alternative at all: the car (S271, W-1), if it has a road path.
        match (set.is_empty() && self.fallback_car, self.car_keys[p]) {
            (true, Some(key)) => planner.fallback_car(key, trips.departure(trip).get()),
            _ => set,
        }
    }

    /// The shared drives the trips `jobs` (position and offer) would read when planned:
    /// those going out through a car park.
    fn drives_wanted(
        &self,
        planner: &Planner<'_>,
        trips: &Trips,
        jobs: &[(usize, Offer)],
    ) -> Vec<(DriveKey, Vec<NodeId>)> {
        jobs.iter()
            .filter_map(|&(p, offer)| {
                let trip = self.trips[p];
                let (origin, destination) = (trips.origin(trip), trips.destination(trip));
                let out = matches!(offer.car_transit, Some(Shape::Out))
                    && planner.far_enough(&offer, origin, destination);
                if !out {
                    return None;
                }
                let dep = f64::from(trips.departure(trip).get()) + planner.car_start_s();
                planner.car_drive_wanted(&self.nodes[p], dep, destination, offer.parking)
            })
            .collect()
    }

    /// The shared drives a pass after the loading `current` was chosen for is expected to read
    /// (S299): those of every trip as `current` leaves its traveller's vehicles, offered all it
    /// may use or, keeping an itinerary out through a car park, that car park alone. A pass
    /// may read others (its earlier trips' choices can move a vehicle) and search them then.
    /// With `only` (by traveller, as [`Self::choose`]'s), the marked travellers' trips alone.
    pub(crate) fn drives_ahead(
        &self,
        planner: &Planner<'_>,
        trips: &Trips,
        travellers: &Travellers,
        current: &Chosen,
        only: Option<&[bool]>,
    ) -> Vec<(DriveKey, Vec<NodeId>)> {
        let mut jobs: Vec<(usize, Offer)> = Vec::new();
        for p in 0..self.trips.len() {
            if only.is_some_and(|o| !o[trips.traveller(self.trips[p]).index()]) {
                continue;
            }
            let offer = self.offer_of(trips, travellers, p, &current.alt);
            jobs.push((p, offer));
            if let Some(old) = current.alt[p].as_ref().filter(|old| old.shape == Shape::Out) {
                jobs.push((p, offer.only(old.mode).at_parking(old.parking)));
            }
        }
        self.drives_wanted(planner, trips, &jobs)
    }

    /// Finds, before the first planning, the bike parkings in reach of every trip that may ride
    /// out to one: one search per origin node, shared by the trips that start there
    /// (about four trips per bike node on Amsterdam's morning, which the free-flow loading's
    /// groups would otherwise search apart). Exact: a trip's reach depends on its origin node and
    /// destination alone, and a trip not found here is found when first planned.
    fn fill_bike_reach<'a>(&self, planner: &Planner<'a>, trips: &Trips, travellers: &Travellers) {
        let (Some((_, ctx)), Some(parking)) = (planner.bike, planner.parking) else {
            return;
        };
        let min_m = parking.defaults().pr_min_km * 1000.0;
        let bike_transit = Mode::BikeTransit.index();
        let mut wanted: Vec<(u32, usize)> = (0..self.trips.len())
            .filter(|&p| {
                let trip = self.trips[p];
                if self.nodes[p].bike_o.is_null() {
                    return false;
                }
                if !self.choosing[p] {
                    return trips.mode(trip) == Mode::BikeTransit;
                }
                // As `offer_of` and `Planner::plan` decide it, but for where the bike is.
                let class = travellers.user_class(trips.traveller(trip)).index();
                self.offered[bike_transit]
                    && self.class_modes.get(class).is_none_or(|c| c[bike_transit])
                    && ground_distance_metres(trips.origin(trip), trips.destination(trip)) >= min_m
            })
            .map(|p| (self.nodes[p].bike_o.raw(), p))
            .collect();
        wanted.sort_unstable();
        let groups: Vec<&[(u32, usize)]> = wanted.chunk_by(|a, b| a.0 == b.0).collect();
        par_map(
            &groups,
            64,
            || Search::new(ctx),
            |search, group| {
                let found = planner.bike_reach_search(search, NodeId::new(group[0].0));
                for &(_, p) in *group {
                    let destination = trips.destination(self.trips[p]);
                    let _ = self.bike_reach[p].set(reach_cands(
                        &found,
                        parking,
                        destination,
                        planner.bike_egress_s,
                    ));
                }
            },
        );
    }

    /// The trips [`Self::gap`] measures, with what each is offered: a trip choosing its mode
    /// that travelled, and one that drove a car, offered all that `current` leaves it.
    fn gap_jobs(
        &self,
        trips: &Trips,
        travellers: &Travellers,
        current: &Chosen,
    ) -> Vec<(usize, Offer)> {
        (0..self.trips.len())
            .filter(|&p| {
                current.alt[p].as_ref().is_some_and(|old| self.choosing[p] || old.mode == Mode::Car)
            })
            .map(|p| (p, self.offer_of(trips, travellers, p, &current.alt)))
            .collect()
    }

    /// Choose: every trip (`current` `None`, iteration 0), or, after a loading,
    /// the travellers `strategy` picks to choose again and those whose
    /// alternative is no longer on offer, the rest keeping theirs at the new
    /// costs. Returns the choices and what the pass found.
    ///
    /// **The gap is sampled**: with a `strategy`, the travellers it picks to
    /// choose again plan their whole choice set, and so does a keyed sample of the others
    /// ([`Equilibration::itinerary_gap_sample`] trips, drawn again each iteration); both are
    /// random samples, and the gap is measured on them. A trip outside both that keeps an
    /// itinerary out through a parking is **re-costed** at that parking (its search and query
    /// only), and is asked to choose again only if its itinerary is gone there. Without a
    /// `strategy` (the run's last assessment) every trip plans in full: the last gap is exact.
    ///
    /// **With `only`** (by traveller, a group of the free-flow loading's increments) the
    /// marked travellers choose again and the others are neither planned nor assessed: they
    /// keep their alternatives as they are.
    ///
    /// **`car_reach`** holds the shared drives searched on `planner`'s times so far (the search
    /// trees', S299, or none): those the pass needs and it lacks are searched and added.
    ///
    /// **With `gap_plans`** (when [`Self::gap`] follows on `planner` and `current`), each trip
    /// the gap measures that was planned here with the very offer the gap gives it leaves its
    /// set there (S299), so the gap plans it only once: same planner, same offer, same set.
    ///
    /// # Errors
    ///
    /// [`ChoiceError`] if the model fails or answers wrongly.
    #[allow(clippy::too_many_lines, reason = "one pass: plan, keep or choose, assess")]
    #[allow(clippy::too_many_arguments, reason = "the costs, the choices and who chooses")]
    pub(crate) fn choose<'a>(
        &self,
        planner: &Planner<'a>,
        inputs: &ChooseInputs<'_>,
        current: Option<&Chosen>,
        iteration: u32,
        strategy: Option<(&dyn Equilibration, &StreamRng, f64)>,
        only: Option<&[bool]>,
        car_reach: &mut CarReach,
        mut gap_plans: Option<&mut GapPlans>,
        car_gap: Option<CarGap<'_>>,
    ) -> Result<(Chosen, Assessment), ChoiceError> {
        let n = self.trips.len();
        let mut next = Chosen {
            alt: current.map_or_else(|| vec![None; n], |c| c.alt.clone()),
            probability: current.map_or_else(|| vec![f64::NAN; n], |c| c.probability.clone()),
            alternatives: current.map_or_else(|| vec![0; n], |c| c.alternatives.clone()),
            logsum: current.map_or_else(|| vec![f64::NAN; n], |c| c.logsum.clone()),
        };
        let mut assessment = Assessment {
            expected_measured: true,
            car_routes: RouteGapSums::new(car_gap.is_some_and(|c| c.currency.is_some())),
            ..Assessment::default()
        };
        let (trips, travellers) = (inputs.trips, inputs.travellers);
        // The car parks' free-flow table, searched here, outside the parallel planning (S209).
        if let (true, Some(parking)) = (self.parks_cars, planner.parking) {
            parking.prepare_car_free_flow();
        }
        // The bike-and-ride reaches, one search per origin node, also outside it (S259; not
        // inside a `OnceLock`'s initialisation, as `prepare_car_free_flow` says why).
        if self.bike_reach_filled.get().is_none() {
            self.fill_bike_reach(planner, trips, travellers);
            let _ = self.bike_reach_filled.set(());
        }
        // With `only` (S223: one group of the free-flow loading's increments), its travellers
        // choose again and no one else is planned or assessed.
        let marked = |p: usize| only.is_none_or(|o| o[trips.traveller(self.trips[p]).index()]);
        let reselects = |p: usize| {
            let traveller = trips.traveller(self.trips[p]).raw();
            only.is_some_and(|_| marked(p))
                || strategy.is_some_and(|(strategy, msa, share)| {
                    strategy.reselects(msa, traveller, iteration, share)
                })
        };
        // Who plans in full to measure the gap: everyone without a strategy, else a keyed
        // sample of travellers (its own draw address, apart from the reselection's).
        #[allow(clippy::cast_precision_loss, reason = "a share")]
        let share = strategy.map_or(1.0, |(strategy, _, _)| {
            f64::from(strategy.itinerary_gap_sample()) / n.max(1) as f64
        });
        let in_sample = |p: usize| {
            share >= 1.0
                || strategy.is_some_and(|(_, rng, _)| {
                    let traveller = trips.traveller(self.trips[p]).raw();
                    rng.unit(DrawAddress::from_pair(traveller, iteration | GAP_SAMPLE_SALT)) < share
                })
        };
        let plan = |s: &mut Scratch<'a>, p: usize, offer: Offer, car_reach: &CarReach| {
            self.plan_trip(planner, trips, s, p, offer, car_reach)
        };
        // The offer the gap after this pass gives each trip it measures.
        let gap_offer: Option<Vec<Option<Offer>>> =
            gap_plans.as_ref().zip(current).map(|(_, current)| {
                let mut offers = vec![None; n];
                for (p, offer) in self.gap_jobs(trips, travellers, current) {
                    offers[p] = Some(offer);
                }
                offers
            });
        if let (Some(kept), Some(_)) = (gap_plans.as_deref_mut(), &gap_offer) {
            kept.clear();
            kept.resize(n, None);
        }
        // The drives park-and-ride trips share (S267), searched on this pass's times as the
        // trips that need them come up, outside the parallel planning; those `car_reach` holds
        // already (the search trees', S299) are not searched again.
        let car_reach_keys = |jobs: &[(usize, Offer)]| -> Vec<(DriveKey, Vec<NodeId>)> {
            self.drives_wanted(planner, trips, jobs)
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
                    // A trip that fell back to the car (S271) looks at all it is offered again.
                    Some(old) if !old.fallback && !reselects(p) && !offer.fetches() => {
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
            planner.fill_car_reach(car_reach_keys(&jobs), car_reach);
            let reach: &CarReach = car_reach;
            let mut sets: Vec<Vec<Alternative>> = par_map(
                &jobs,
                SCRATCH_CHUNK,
                || planner.scratch(),
                |s, &(p, o)| plan(s, p, o, reach),
            );
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
            // The offer each set was planned with.
            let mut planned = first.clone();
            for &w in &again {
                planned[w] = work[w].1;
            }
            planner.fill_car_reach(car_reach_keys(&redo), car_reach);
            let reach: &CarReach = car_reach;
            let replanned = par_map(
                &redo,
                SCRATCH_CHUNK,
                || planner.scratch(),
                |s, &(p, o)| plan(s, p, o, reach),
            );
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
                // (situation, weight, the route taken's place in the set or its re-timed
                // alternative): the car routes of the trips choosing their mode (S302).
                let mut car_in_gap: Vec<(usize, f64, Result<usize, Alternative>)> = Vec::new();
                // Each alternative's door-to-door seconds, for the generalised sums.
                let mut alt_seconds: Vec<f64> = Vec::new();
                let car_currency = car_gap.and_then(|c| c.currency);
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
                            *slot =
                                attribute(name, a, best[a.mode.index()], inputs, traveller, trip);
                        }
                        batch.push_alternative(a.identity, &row);
                        if car_currency.is_some() {
                            alt_seconds.push(a.total_s);
                        }
                    }
                    let s = situation_of.len();
                    situation_of.push(w);
                    // A trip choosing its mode that drove and finished: its route in the route
                    // gap, against the car routes of its set (S302). A route no longer in the
                    // set (slower than the detour limit allows, or gone) is re-timed on these
                    // times, as the full gap does.
                    let drove = match (car_gap, old, self.choosing[p]) {
                        (Some(cg), Some(old), true) => {
                            old.mode == Mode::Car
                                && !cg.unfinished[trip.index()]
                                && set.iter().any(|a| a.mode == Mode::Car)
                        }
                        _ => false,
                    };
                    if let (true, Some(old)) = (drove, old) {
                        let taken = match set.iter().position(|a| a.identity == old.identity) {
                            Some(i) => Ok(i),
                            None => {
                                let mut a = old.clone();
                                let links: Vec<u32> =
                                    a.vehicle_links.iter().map(|l| l.raw()).collect();
                                let at =
                                    f64::from(trips.departure(trip).get()) + planner.car_start_s();
                                a.vehicle_s = planner.times.route_seconds(&links, at);
                                a.total_s = a.vehicle_s + a.walk_s + a.parking_s;
                                Err(a)
                            }
                        };
                        car_in_gap.push((s, weight, taken));
                    }
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
                        assessment.weight[m] += weight;
                        in_gap.push((s, m, weight));
                    } else {
                        assessment.recosted += 1;
                    }
                    assessment.w_all += weight;
                    let reselect = strategy.is_some_and(|(strategy, msa, share)| {
                        strategy.reselects(msa, traveller.raw(), iteration, share)
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
                // The batch is checked once, before the model's first look at it (X-30).
                let mut checked = false;
                if batch.situations() > 0 {
                    batch.validate()?;
                    checked = true;
                    if let Some(logsums) = inputs.model.logsums(&batch)? {
                        for (s, &w) in situation_of.iter().enumerate() {
                            // A trip that kept its alternative planned only its mode: its
                            // logsum stays the one of its last full choice set (S279, CM-1:
                            // before, the one-mode logsum overwrote it, and `logsum_od` mixed
                            // the two kinds).
                            if first[w] == work[w].1 {
                                next.logsum[work[w].0] = logsums[s];
                            }
                        }
                    }
                }
                if !(in_gap.is_empty() && floor.is_empty() && car_in_gap.is_empty()) {
                    if !checked {
                        batch.validate()?;
                        checked = true;
                    }
                    let probabilities = inputs.model.probabilities(&batch)?;
                    if !car_in_gap.is_empty() {
                        self.car_route_sums(
                            inputs,
                            &batch,
                            probabilities.as_deref(),
                            (&car_in_gap, &situation_of, &sets),
                            car_currency.map(|c| (c, alt_seconds.as_slice())),
                            &mut assessment.car_routes,
                        )?;
                    }
                    match probabilities {
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
                if !checked {
                    batch.validate()?;
                }
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
            // The sets the gap would plan again with the same offer, left for it.
            if let (Some(kept), Some(gap_offer)) = (gap_plans.as_deref_mut(), &gap_offer) {
                for (w, set) in sets.into_iter().enumerate() {
                    let p = work[w].0;
                    if gap_offer[p] == Some(planned[w]) {
                        kept[p] = Some(set);
                    }
                }
            }
        }
        Ok((next, assessment))
    }

    /// The route gap's sums over the car routes of the trips choosing their mode that drove
    /// (S302), for one chunk of [`Self::choose`]: `cars` holds each such trip's situation in
    /// `batch`, its weight and its route taken (its place in the set, or the route re-timed when
    /// the set no longer holds it); `situation_of` and `sets` give each situation's set. In
    /// seconds of driving (the route chosen, the least of the set's car routes, and what the
    /// model expects among them, its probabilities renormalised over the car routes; the least
    /// where it gives the car no probability, as an all-or-nothing model does); with
    /// `generalised`, also in generalised minutes, from the alternatives' door-to-door seconds.
    ///
    /// # Errors
    ///
    /// [`ChoiceError`] if the model fails or answers wrongly.
    #[allow(clippy::type_complexity, reason = "the chunk's own vectors, as they are")]
    fn car_route_sums(
        &self,
        inputs: &ChooseInputs<'_>,
        batch: &ChoiceBatch,
        probabilities: Option<&[f64]>,
        (cars, situation_of, sets): (
            &[(usize, f64, Result<usize, Alternative>)],
            &[usize],
            &[Vec<Alternative>],
        ),
        generalised: Option<(&Currency, &[f64])>,
        sums: &mut RouteGapSums,
    ) -> Result<(), ChoiceError> {
        let g = match generalised {
            Some((currency, seconds)) => Some(currency.minutes(inputs.model, batch, seconds)?),
            None => None,
        };
        // The routes no longer in their set, costed in generalised minutes one by one.
        let names: Vec<&str> = inputs.wanted.iter().map(String::as_str).collect();
        let mut single = ChoiceBatch::new(batch.iteration(), &names);
        let mut single_seconds: Vec<f64> = Vec::new();
        let mut row = vec![0.0; inputs.wanted.len()];
        // Per car trip: (weight, paid, least, expected, g_least, g_expected) and, for one
        // costed apart, its place in `single`.
        let mut rows: Vec<([f64; 6], Option<f64>)> = Vec::with_capacity(cars.len());
        for (s, weight, taken) in cars {
            let (s, weight) = (*s, *weight);
            let set = &sets[situation_of[s]];
            let start = batch.range(s).start;
            let car: Vec<usize> = (0..set.len()).filter(|&i| set[i].mode == Mode::Car).collect();
            let least = car.iter().map(|&i| set[i].vehicle_s).fold(f64::INFINITY, f64::min);
            let (paid, g_paid) = match taken {
                Ok(i) => (set[*i].vehicle_s, g.as_ref().map(|g| g[start + *i])),
                Err(a) => {
                    if g.is_some() {
                        let traveller = TravellerId::new(batch.travellers()[s]);
                        let trip = TripId::new(batch.trips()[s]);
                        let best =
                            car.iter().map(|&i| set[i].total_s).fold(f64::INFINITY, f64::min);
                        single.begin_situation_in(traveller.raw(), trip.raw(), batch.classes()[s]);
                        for (slot, name) in row.iter_mut().zip(inputs.wanted) {
                            *slot = attribute(name, a, best, inputs, traveller, trip);
                        }
                        single.push_alternative(a.identity, &row);
                        single_seconds.push(a.total_s);
                    }
                    (a.vehicle_s, None)
                }
            };
            let (mut expected, mut g_least, mut g_expected) = (f64::NAN, f64::NAN, f64::NAN);
            if let Some(pr) = probabilities {
                let p_car: f64 = car.iter().map(|&i| pr[start + i]).sum();
                expected = if p_car > 0.0 {
                    car.iter().map(|&i| pr[start + i] * set[i].vehicle_s).sum::<f64>() / p_car
                } else {
                    least
                };
                if let Some(g) = &g {
                    g_least = car.iter().map(|&i| g[start + i]).fold(f64::INFINITY, f64::min);
                    g_expected = if p_car > 0.0 {
                        car.iter().map(|&i| pr[start + i] * g[start + i]).sum::<f64>() / p_car
                    } else {
                        g_least
                    };
                }
            }
            rows.push(([weight, paid, least, expected, g_least, g_expected], g_paid));
        }
        let mut apart = Vec::new().into_iter();
        if !single_seconds.is_empty() {
            single.validate()?;
            if let Some((currency, _)) = generalised {
                apart = currency.minutes(inputs.model, &single, &single_seconds)?.into_iter();
            }
        }
        let measured = probabilities.is_some();
        sums.measured &= measured;
        sums.g_measured &= measured;
        for ([weight, paid, least, expected, g_least, g_expected], g_paid) in rows {
            sums.weight += weight;
            sums.paid += weight * paid;
            sums.least += weight * least;
            if measured {
                sums.expected += weight * expected;
            }
            if sums.g_measured {
                let g_paid = g_paid.or_else(|| apart.next()).unwrap_or(f64::NAN);
                sums.g_paid += weight * g_paid;
                sums.g_least += weight * g_least;
                sums.g_expected += weight * g_expected;
            }
        }
        Ok(())
    }

    /// **The full gap's itinerary trips** (S293, [`crate::gap`]), on `planner`'s costs (this
    /// loading's): every trip choosing its mode and every trip that drove a car alternative
    /// (keeping one, or with no alternative but the car) has its **whole choice set** planned,
    /// every mode it is offered where the choices `current` (those loaded) leave its
    /// traveller's vehicles, and costed in generalised minutes. Into `out`: the alternative
    /// `current` gives it; for a trip choosing its mode the best of the set and what the model
    /// expects over it; for one that drove a car alternative the best car route, what the model
    /// expects among the car routes (its probabilities renormalised over them) and the best over
    /// the whole network (the route a bounded time-dependent search finds, costed as a car
    /// alternative). A car trip still under way at the end is costed as what it paid and still
    /// had to pay ([`Ended::car_seconds`]) plus its walk and parking; any other keeps its cost
    /// on these costs. An alternative no longer in the set is costed as it was planned, a car
    /// route re-timed on these times. Every trip, every call; nothing is drawn, nothing changed.
    /// The shared drives are read from `car_reach` (those searched on `planner`'s times so far:
    /// a drive's routes to the car parks it sought are the same whoever asked first, S299), and
    /// those missing are searched and added.
    ///
    /// Costs one more planning of every such trip's whole set (in parallel), and one bounded
    /// search per car trip. A set in `kept` (left by [`Self::choose`] on the same planner and
    /// `current`) is taken as it is instead of being planned again.
    ///
    /// # Errors
    ///
    /// [`ChoiceError`] if the model fails or answers wrongly.
    #[allow(clippy::too_many_lines, reason = "one pass: plan, cost, search")]
    #[allow(clippy::too_many_arguments, reason = "the costs, the choices and what the end left")]
    pub(crate) fn gap<'a>(
        &self,
        planner: &Planner<'a>,
        inputs: &ChooseInputs<'_>,
        current: &Chosen,
        iteration: u32,
        ended: &Ended<'_>,
        currency: &Currency,
        out: &mut GapTrips,
        car_reach: &mut CarReach,
        kept: Option<GapPlans>,
    ) -> Result<(), ChoiceError> {
        let (trips, travellers, model) = (inputs.trips, inputs.travellers, inputs.model);
        if let (true, Some(parking)) = (self.parks_cars, planner.parking) {
            parking.prepare_car_free_flow();
        }
        if self.bike_reach_filled.get().is_none() {
            self.fill_bike_reach(planner, trips, travellers);
            let _ = self.bike_reach_filled.set(());
        }
        let jobs = self.gap_jobs(trips, travellers, current);
        let mut kept = kept.unwrap_or_default();
        let names: Vec<&str> = inputs.wanted.iter().map(String::as_str).collect();
        let mut row = vec![0.0; inputs.wanted.len()];
        let fill = |row: &mut [f64], a: &Alternative, best: f64, traveller, trip| {
            for (slot, name) in row.iter_mut().zip(inputs.wanted) {
                *slot = attribute(name, a, best, inputs, traveller, trip);
            }
        };
        for jobs in jobs.chunks(CHUNK) {
            // A set the choice before left (`kept`) is the one planning it again would give.
            let fresh: Vec<(usize, Offer)> = jobs
                .iter()
                .filter(|&&(p, _)| kept.get(p).is_none_or(Option::is_none))
                .copied()
                .collect();
            planner.fill_car_reach(self.drives_wanted(planner, trips, &fresh), car_reach);
            let reach: &CarReach = car_reach;
            let mut planned = par_map(
                &fresh,
                SCRATCH_CHUNK,
                || planner.scratch(),
                |s, &(p, offer)| self.plan_trip(planner, trips, s, p, offer, reach),
            )
            .into_iter();
            let sets: Vec<Vec<Alternative>> = jobs
                .iter()
                .map(|&(p, _)| match kept.get_mut(p).and_then(Option::take) {
                    Some(set) => set,
                    None => planned.next().expect("a plan for every job not kept"),
                })
                .collect();
            let mut batch = ChoiceBatch::new(iteration, &names);
            let mut seconds: Vec<f64> = Vec::new();
            // Per situation: its job, and the best door-to-door seconds of each mode in its set.
            let mut situation_of: Vec<(usize, [f64; Mode::COUNT])> = Vec::new();
            for (j, &(p, _)) in jobs.iter().enumerate() {
                if sets[j].is_empty() {
                    continue;
                }
                let trip = self.trips[p];
                let traveller = trips.traveller(trip);
                let best = best_by_mode(&sets[j]);
                batch.begin_situation_in(
                    traveller.raw(),
                    trip.raw(),
                    travellers.user_class(traveller).raw(),
                );
                for a in &sets[j] {
                    fill(&mut row, a, best[a.mode.index()], traveller, trip);
                    batch.push_alternative(a.identity, &row);
                    seconds.push(a.total_s);
                }
                situation_of.push((j, best));
            }
            if situation_of.is_empty() {
                continue;
            }
            batch.validate()?;
            let probabilities = model.probabilities(&batch)?;
            let g = currency.minutes(model, &batch, &seconds)?;
            // One situation per alternative costed on its own: a car trip still under way, an
            // alternative no longer in the set, a route the network search found.
            let mut single = ChoiceBatch::new(iteration, &names);
            let mut single_seconds: Vec<f64> = Vec::new();
            let mut pending: Vec<(usize, f64)> = Vec::new();
            // The car trips to search for: trip, its pair, departure, the best car drive's
            // seconds (the search's bound) and the best car alternative's door to door.
            let mut searches: Vec<(usize, RouteKey, f64, f64, f64, u32)> = Vec::new();
            let cost = planner.car_cost.filter(|c| !c.is_time_only());
            // Each vector's extras, prepared for the car's context (their A* bounds).
            let prepared = planner.car_extras;
            for (s, &(j, best)) in situation_of.iter().enumerate() {
                let (p, _) = jobs[j];
                let set = &sets[j];
                let range = batch.range(s);
                let start = range.start;
                let gs = &g[range.clone()];
                let trip = self.trips[p];
                let t = trip.index();
                let traveller = trips.traveller(trip);
                let old = current.alt[p].as_ref().expect("a measured trip travelled");
                if self.choosing[p] {
                    out.g_best_all[t] = gs.iter().copied().fold(f64::INFINITY, f64::min);
                    out.g_expected_all[t] = probabilities
                        .as_ref()
                        .map_or(f64::NAN, |pr| range.clone().map(|a| pr[a] * g[a]).sum());
                }
                let car: Vec<usize> =
                    (0..set.len()).filter(|&i| set[i].mode == Mode::Car).collect();
                let drove = old.mode == Mode::Car;
                if drove && !car.is_empty() {
                    out.g_best_set[t] = car.iter().map(|&i| gs[i]).fold(f64::INFINITY, f64::min);
                    out.g_expected_set[t] = probabilities.as_ref().map_or(f64::NAN, |pr| {
                        let (p_car, g_car) = car.iter().fold((0.0, 0.0), |(pc, gc), &i| {
                            (pc + pr[start + i], gc + pr[start + i] * gs[i])
                        });
                        if p_car > 0.0 { g_car / p_car } else { f64::NAN }
                    });
                    out.g_best_network[t] = out.g_best_set[t];
                    if let Some(key) = self.car_keys[p] {
                        // The best car drive's cost: its time, plus its extras under the trip's
                        // coefficient vector where the cost is not time alone (S299).
                        let vector = cost.map_or(0, |c| {
                            c.vector_of_class(travellers.user_class(traveller).index())
                        });
                        let drive_cost = |a: &Alternative| match cost {
                            None => a.vehicle_s,
                            Some(c) => {
                                let raw: Vec<u32> =
                                    a.vehicle_links.iter().map(|l| l.raw()).collect();
                                a.vehicle_s + c.route_extra(vector, &raw)
                            }
                        };
                        let bound =
                            car.iter().map(|&i| drive_cost(&set[i])).fold(f64::INFINITY, f64::min);
                        let departure =
                            f64::from(trips.departure(trip).get()) + planner.car_start_s();
                        searches.push((t, key, departure, bound, best[Mode::Car.index()], vector));
                    }
                }
                let kept = set.iter().position(|a| a.identity == old.identity);
                let unfinished_car = drove && ended.unfinished[t];
                match kept {
                    Some(i) if !unfinished_car => {
                        out.g_chosen[t] = gs[i];
                        out.g_remaining[t] = 0.0;
                    }
                    _ => {
                        let mut a = kept.map_or_else(|| old.clone(), |i| set[i].clone());
                        let mut remaining = 0.0;
                        if drove {
                            let links: Vec<u32> = a.vehicle_links.iter().map(|l| l.raw()).collect();
                            let departure =
                                f64::from(trips.departure(trip).get()) + planner.car_start_s();
                            if unfinished_car {
                                let (paid, rest) =
                                    ended.car_seconds(t, &links, departure, planner.times);
                                a.vehicle_s = paid + rest;
                                remaining = rest + a.walk_s + a.parking_s;
                            } else {
                                a.vehicle_s = planner.times.route_seconds(&links, departure);
                            }
                            a.total_s = a.vehicle_s + a.walk_s + a.parking_s;
                        }
                        single.begin_situation_in(traveller.raw(), trip.raw(), batch.classes()[s]);
                        fill(&mut row, &a, best[a.mode.index()], traveller, trip);
                        single.push_alternative(a.identity, &row);
                        single_seconds.push(a.total_s);
                        pending.push((t, remaining / 60.0));
                    }
                }
            }
            if !pending.is_empty() {
                single.validate()?;
                let g = currency.minutes(model, &single, &single_seconds)?;
                for ((t, remaining), g) in pending.into_iter().zip(g) {
                    out.g_chosen[t] = g;
                    out.g_remaining[t] = remaining;
                }
            }
            // The whole network for the car trips: one bounded search each, in parallel; each
            // route found costed as a car alternative.
            let times = planner.times;
            let road_nodes = planner.car.network.node_count() as usize;
            let found = par_map(
                &searches,
                crate::route_choice::GAP_CHUNK,
                || Search::new(planner.car),
                |search, &(_, key, departure, bound, _, vector)| {
                    let wait = |link: u32, at: f64| times.origin_wait_seconds(link, at);
                    let link_seconds = |link: u32, at: f64| times.link_seconds(link, at);
                    let (origin, destination) =
                        (NodeId::new(key.origin), NodeId::new(key.destination));
                    match prepared.get(vector as usize).and_then(Option::as_ref) {
                        // By time: the fastest within the best drive's time.
                        None => search.fastest_route(
                            origin,
                            destination,
                            departure,
                            bound,
                            &wait,
                            &link_seconds,
                        ),
                        // By the vector's generalised cost (S299): the cheapest within the best
                        // drive's cost, timed as it is driven.
                        Some(e) => {
                            search.clear_route_state();
                            search.set_extras(Some(e.clone()));
                            let mut target = vec![false; road_nodes];
                            target[destination.index()] = true;
                            let one = search
                                .cheapest_routes_to(
                                    origin,
                                    &target,
                                    1,
                                    departure,
                                    bound,
                                    &wait,
                                    &link_seconds,
                                )
                                .into_iter()
                                .next()
                                .filter(|(_, _, _, links)| !links.is_empty())
                                .map(|(_, _, seconds, links)| (seconds, links));
                            search.clear_route_state();
                            one
                        }
                    }
                },
            );
            single.clear();
            single_seconds.clear();
            let mut pending: Vec<usize> = Vec::new();
            let road = planner.car.network;
            for (&(t, _, _, _, best_car, _), found) in searches.iter().zip(found) {
                let Some((drive_s, links)) = found else { continue };
                let trip = TripId::from_index(t);
                let traveller = trips.traveller(trip);
                let mut a = blank(Mode::Car, Shape::Direct);
                a.vehicle_m = links.iter().map(|&l| road.link_length(l).get()).sum();
                a.vehicle_links = links;
                a.vehicle_s = drive_s;
                a.walk_s = planner.car_access_s;
                a.parking_s = planner.car_egress_s;
                a.total_s = drive_s + planner.car_access_s + planner.car_egress_s;
                a.identity = a.identity_hash();
                single.begin_situation_in(
                    traveller.raw(),
                    trip.raw(),
                    travellers.user_class(traveller).raw(),
                );
                fill(&mut row, &a, best_car, traveller, trip);
                single.push_alternative(a.identity, &row);
                single_seconds.push(a.total_s);
                pending.push(t);
            }
            if !pending.is_empty() {
                single.validate()?;
                let g = currency.minutes(model, &single, &single_seconds)?;
                for (t, g) in pending.into_iter().zip(g) {
                    out.g_best_network[t] = out.g_best_network[t].min(g);
                }
            }
        }
        Ok(())
    }
}

/// Each itinerary trip's choice and how it went, for the results.
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
    /// Whether the trip chose its mode, or was given it.
    pub choosing: Vec<bool>,
    /// The logsum of its choice set (`NaN` if the model gives none).
    pub logsum: Vec<f64>,
    /// How the run's planning searched the timetable, and what it kept.
    pub timetable_searches: TimetableSearches,
}

/// No mode: a trip that had no alternative.
pub const NO_MODE: u8 = u8::MAX;

/// Why a mode gave a trip left with no alternative nothing to choose.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NoAlternativeReason {
    /// Not offered to the trip: not among the run's modes, not one its class may use, or, for
    /// a trip whose mode was stated, not that mode.
    NotOffered,
    /// No walk within the walk limit (`walk_max_s`): a longer one, or none on the walk layer.
    WalkTooLong,
    /// No ride within the ride limit (`bike_max_s`): a longer one, or none on the bike layer.
    RideTooLong,
    /// The vehicle is not at the origin: the traveller has none, or left it elsewhere earlier
    /// in their day.
    NoVehicle,
    /// No road route: none between origin and destination, or both on one road node.
    NoRoadRoute,
    /// No stop within the walk to a stop (`access_walk_max_s`) of the origin or of the
    /// destination.
    NoStopInReach,
    /// Stops in reach, but no journey with transit arriving within the longest one offered
    /// (`journey_max_s`) on the window's timetable.
    NoTransitJourney,
    /// Shorter than the shortest trip offered a parking on the way (`pr_min_km`).
    TooShort,
    /// No two docks of the run's docked service within its walk (`walk_max_s`) of the origin
    /// and of the destination, or no ride on the bike layer between them (S302).
    NoDockInReach,
    /// None of the above: what the mode was offered should have given an alternative. A
    /// defect to report.
    Unexplained,
}

impl NoAlternativeReason {
    /// The stable snake_case name, as the results write it (`""` for [`Self::NotOffered`]).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotOffered => "",
            Self::WalkTooLong => "walk_too_long",
            Self::RideTooLong => "ride_too_long",
            Self::NoVehicle => "no_vehicle",
            Self::NoRoadRoute => "no_road_route",
            Self::NoStopInReach => "no_stop_in_reach",
            Self::NoTransitJourney => "no_transit_journey",
            Self::TooShort => "too_short",
            Self::NoDockInReach => "no_dock_in_reach",
            Self::Unexplained => "unexplained",
        }
    }
}

/// A trip left with no alternative in the run's last choice, and why each
/// mode gave it none.
#[derive(Clone, Debug, PartialEq)]
pub struct NoAlternative {
    /// The trip.
    pub trip: TripId,
    /// Whether it chose its mode; else its mode was stated (transit, park-and-ride or
    /// bike-and-ride) and only that mode was offered.
    pub choosing: bool,
    /// Per mode, by [`Mode::index`], why it gave the trip nothing.
    pub reasons: [NoAlternativeReason; Mode::COUNT],
    /// Whether the trip then travelled by car ([`crate::ModeDefaults::fallback_car`]).
    pub fallback_car: bool,
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
            mode: Vec::with_capacity(n),
            choosing: itineraries.choosing.clone(),
            logsum: chosen.logsum.clone(),
            timetable_searches: itineraries.timetable_searches(),
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

#[cfg(test)]
mod tests {
    use super::*;

    /// **Park-and-ride drives by the route cost** (S299): on a two-road diamond, the drive to a
    /// car park at `D` is the top road by time; with 60 s of extras on its first link (a toll),
    /// the bottom road; and a drive past its time bound is not kept, however cheap.
    #[test]
    fn a_drive_to_a_car_park_follows_its_class_s_generalised_cost() {
        use openmobisim_core_graph::defaults::{GlobalMultipliers, RoadClass, SignalDefaults};
        use openmobisim_core_graph::geometry::LonLat;
        use openmobisim_core_graph::network::{LinkSpec, RoadNetworkBuilder};
        use openmobisim_core_graph::turns::TurnTable;
        use openmobisim_core_types::diagnostics::Diagnostics;
        let mut b = RoadNetworkBuilder::new();
        for (n, x, y) in
            [("A", 0.0, 0.0), ("B", 100.0, 60.0), ("C", 100.0, -60.0), ("D", 200.0, 0.0)]
        {
            b.add_node(n, LonLat::new(4.8 + x / 77_800.0, 45.7 + y / 110_574.0));
        }
        for (name, from, to, len) in [
            ("ab", "A", "B", 100.0),
            ("bd", "B", "D", 100.0),
            ("ac", "A", "C", 120.0),
            ("cd", "C", "D", 120.0),
        ] {
            let mut spec = LinkSpec::new(RoadClass::Residential);
            spec.length_m = Some(len);
            b.add_link(name, from, to, spec);
        }
        let net = b
            .build(GlobalMultipliers::default(), SignalDefaults::SHIPPED, &mut Diagnostics::new())
            .unwrap();
        let turns = TurnTable::build(&net, SignalDefaults::SHIPPED);
        let ctx = SearchContext::new(&net, &turns);
        let mut search = Search::new(&ctx);
        let id = |n: &str| net.link_external_ids().typed_id_of::<LinkId>(n).unwrap();
        let node = |n: &str| net.node_external_ids().typed_id_of::<NodeId>(n).unwrap();
        let mut targets = vec![false; net.node_count() as usize];
        targets[node("D").index()] = true;
        let times = LinkTimes::free_flow(&net);
        let wait = |l: u32, t: f64| times.origin_wait_seconds(l, t);
        let secs = |l: u32, t: f64| times.link_seconds(l, t);
        let mut extra = vec![0.0; net.link_count() as usize];
        extra[id("ab").index()] = 60.0;
        let extras = ctx.extras(extra);
        let drive =
            |e: Option<&openmobisim_core_routes::Extras>, bound: f64, s: &mut Search<'_>| {
                car_search_to(s, e, node("A"), &targets, 1, 0.0, bound, (&wait, &secs))
            };
        let by_time = drive(None, 600.0, &mut search);
        assert_eq!(by_time[0].2, vec![id("ab"), id("bd")]);
        let by_cost = drive(Some(&extras), 600.0, &mut search);
        assert_eq!(by_cost[0].2, vec![id("ac"), id("cd")], "the toll moves the drive");
        let bottom = by_cost[0].1;
        assert!(drive(Some(&extras), bottom - 1.0, &mut search).is_empty(), "past the bound");
    }

    /// A park-and-ride alternative: `links` driven in 600 s to `parking`, 1 000 s in all.
    fn park_and_ride(parking: u32, links: &[u32]) -> Alternative {
        Alternative {
            shape: Shape::Out,
            mode: Mode::CarTransit,
            fixed_ln_path_size: None,
            parking,
            vehicle_links: links.iter().map(|&l| LinkId::new(l)).collect(),
            vehicle_s: 600.0,
            parking_s: 0.0,
            first_stop: NodeId::new(0),
            access_walk_s: 0,
            rides: Vec::new(),
            ride_seconds: Vec::new(),
            transfer_walks: Vec::new(),
            last_stop: NodeId::new(0),
            egress_walk_s: 0,
            walk_s: 400.0,
            wait_s: 0.0,
            ride_s: 0.0,
            total_s: 1000.0,
            vehicle_departure: 0.0,
            vehicle_m: 0.0,
            identity: 0,
            ln_path_size: 0.0,
            bike_m: [0.0; 3],
            wait_first_s: 0,
            ride_kind_s: [0; 6],
            ride_m: 0.0,
            fallback: false,
            share_docks: (NO_PARKING, NO_PARKING),
            dock_s: 0.0,
            taxi_fail: 0.0,
            taxi_wait_s: 0.0,
        }
    }

    #[test]
    fn two_parkings_at_one_node_share_their_drive_in_the_path_size() {
        // The same 600 s drive to two parkings at one node: one element used by both, so each
        // path size is (600 / 2 + 400) / 1 000; a drive on other links is its own.
        let alts = finalise(
            vec![park_and_ride(1, &[3, 4]), park_and_ride(2, &[3, 4]), park_and_ride(3, &[5, 6])],
            0.0,
            5,
            f64::INFINITY,
        );
        let ln = |p: u32| alts.iter().find(|a| a.parking == p).map(|a| a.ln_path_size).unwrap();
        assert!((ln(1) - 0.7_f64.ln()).abs() < 1e-12, "{}", ln(1));
        assert!((ln(2) - 0.7_f64.ln()).abs() < 1e-12, "{}", ln(2));
        assert!(ln(3).abs() < 1e-12, "a drive of its own: {}", ln(3));
    }

    /// **Property:** duplicates are dropped by their contents, not by their 32-bit identity
    /// alone: two different itineraries whose identities collide are both kept, a true duplicate
    /// once.
    #[test]
    fn itineraries_whose_identities_collide_are_both_kept() {
        // Drives over one link each: among a few hundred thousand, two identities collide (the
        // birthday bound of 32 bits); the search is deterministic.
        let mut seen = std::collections::HashMap::new();
        let (a, b) = (0u32..1_000_000)
            .find_map(|l| {
                let id = park_and_ride(1, &[l]).identity_hash();
                seen.insert(id, l).map(|other| (other, l))
            })
            .expect("a collision within a million");
        assert_ne!(a, b);
        let kept = finalise(
            vec![park_and_ride(1, &[a]), park_and_ride(1, &[b]), park_and_ride(1, &[a])],
            0.0,
            5,
            f64::INFINITY,
        );
        let mut links: Vec<u32> = kept.iter().map(|k| k.vehicle_links[0].raw()).collect();
        links.sort_unstable();
        assert_eq!(links, vec![a.min(b), a.max(b)], "both kept, the true duplicate dropped");
    }
}
