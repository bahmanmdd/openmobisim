//! Docked bikes on the fleet engine: docks snapped to the walk and bike layers, their
//! neighbours, pickups and returns with counted bikes, a traveller redirected from an empty or
//! full dock, a failed pickup, overflow, and the docks' expected costs.
//!
//! The network: a 5 × 5 two-way grid of 200 m blocks; three docks on its first row, at 0, 200
//! and 600 m (A, B, C). Rides go at 4 m/s as the crow flies; every walk to a destination from a
//! dock not planned is 500 s; the planned one's is 60 s.

#![allow(clippy::float_cmp, reason = "times here are exact sums of the docks' own walks")]

use std::sync::Arc;

use openmobisim_core_graph::geometry::{LonLat, ground_distance_metres};
use openmobisim_core_graph::layers::{BikeCost, StaticLayer, StaticLayerDefaults, StaticNetwork};
use openmobisim_core_graph::manhattan_grid;
use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_sim::services::costs::DockCosts;
use openmobisim_core_sim::services::engine::{FleetDay, FleetEngine, Leg, NoRoads, ServiceLegs};
use openmobisim_core_sim::services::outcome::{FleetOutcome, LegStatus};
use openmobisim_core_sim::services::{
    DockDefaults, DockRow, Docks, ServiceError, ServiceExpectations, ServiceParts, ServiceSetup,
};
use openmobisim_core_sim::{LayerSetup, StaticLayers};
use openmobisim_core_types::diagnostics::Diagnostics;
use openmobisim_core_types::ids::{EntityId, NodeId};

/// A 5 × 5 two-way grid of 200 m blocks, and its bike and walk layers.
fn grid() -> (RoadNetwork, StaticLayers) {
    let road = manhattan_grid(5, 200.0, false).0;
    let d = StaticLayerDefaults::SHIPPED;
    let derive = |layer| {
        Arc::new(StaticNetwork::derive(&road, layer, d, &mut Diagnostics::new()).expect("derives"))
    };
    let layers = StaticLayers {
        bike: Some(LayerSetup::new(derive(StaticLayer::Bike), BikeCost::Dedicated, d)),
        walk: Some(LayerSetup::new(derive(StaticLayer::Walk), BikeCost::Dedicated, d)),
    };
    (road, layers)
}

fn at(road: &RoadNetwork, node: usize) -> LonLat {
    road.node_lonlat(NodeId::from_index(node))
}

fn row(id: &str, position: LonLat, capacity: u32, bikes: Option<u32>) -> DockRow {
    DockRow { dock_id: id.to_string(), name: None, hub_id: None, position, capacity, bikes }
}

fn setup(rows: &[DockRow], layers: &StaticLayers) -> Result<ServiceSetup, ServiceError> {
    ServiceSetup::bike_docked(
        "bike_share",
        rows,
        layers.walk.as_ref().expect("walk"),
        layers.bike.as_ref().expect("bike"),
        None,
        DockDefaults::SHIPPED,
    )
}

fn docks(s: &ServiceSetup) -> &Docks {
    match s.parts() {
        ServiceParts::BikeDocked(docks, _) => docks,
        ServiceParts::Taxi(..) => panic!("a docked service"),
    }
}

/// Three docks on the grid's first row, at 0, 200 and 600 m: A, B, C.
fn three(bikes: [u32; 3], capacity: u32) -> ServiceSetup {
    let (road, layers) = grid();
    let rows = vec![
        row("C", at(&road, 3), capacity, Some(bikes[2])),
        row("A", at(&road, 0), capacity, Some(bikes[0])),
        row("B", at(&road, 1), capacity, Some(bikes[1])),
    ];
    setup(&rows, &layers).expect("set up")
}

/// Rides at 4 m/s as the crow flies between docks, and a fixed walk to every destination.
#[derive(Debug)]
struct Legs(Vec<LonLat>);

impl Legs {
    fn of(s: &ServiceSetup) -> Self {
        Self(docks(s).position.clone())
    }
}

impl ServiceLegs for Legs {
    fn ride_s(&self, _: u16, from: u32, to: u32) -> f64 {
        ground_distance_metres(self.0[from as usize], self.0[to as usize]) / 4.0
    }
    fn walk_to_destination_s(&self, _: usize, _: u32) -> f64 {
        500.0
    }
}

/// A leg of trip `trip` at the pickup dock at `at_pickup_s`, planned from `pickup` to `ret`.
fn trip(trip: u32, at_pickup_s: f64, pickup: u32, ret: u32, s: &ServiceSetup) -> Leg {
    Leg {
        service: 0,
        trip,
        weight: 1,
        request_s: at_pickup_s,
        origin: NodeId::new(0),
        destination: NodeId::new(1),
        pickup_dock: pickup,
        return_dock: ret,
        ride_s: Legs::of(s).ride_s(0, pickup, ret),
        egress_s: 60.0,
    }
}

/// [`Legs`], with a walk of 100 s to the destination from one dock.
#[derive(Debug)]
struct Near(Legs, u32);

impl ServiceLegs for Near {
    fn ride_s(&self, service: u16, from: u32, to: u32) -> f64 {
        self.0.ride_s(service, from, to)
    }
    fn walk_to_destination_s(&self, _: usize, dock: u32) -> f64 {
        if dock == self.1 { 100.0 } else { 500.0 }
    }
}

/// One fleet day of `legs` over the window 0..`window` s.
fn load(s: &ServiceSetup, legs: &[Leg], window: f64) -> FleetOutcome {
    load_with(s, legs, window, Arc::new(Legs::of(s)))
}

/// [`load`], with the city `city`.
fn load_with(
    s: &ServiceSetup,
    legs: &[Leg],
    window: f64,
    city: Arc<dyn ServiceLegs>,
) -> FleetOutcome {
    let day = FleetDay { window: (0.0, window), legs: legs.to_vec(), seed: 1 };
    let out =
        FleetEngine::new(Arc::new(vec![s.clone()]), Arc::new(day), city, Arc::new(NoRoads), None)
            .run_day()
            .expect("no policy");
    assert_eq!(out.invariant_violations, 0, "bikes are conserved");
    out
}

fn sum(v: &[f64]) -> f64 {
    v.iter().sum()
}

const A: u32 = 0;
const B: u32 = 1;
const C: u32 = 2;

#[test]
fn docks_are_kept_in_id_order_with_their_neighbours_nearest_first() {
    let s = three([1, 0, 2], 2);
    let d = docks(&s);
    assert_eq!(d.len(), 3);
    assert_eq!(d.external, ["A", "B", "C"]);
    assert_eq!(d.fleet(), 3);
    let from_a: Vec<u32> = d.neighbours(A).iter().map(|n| n.0).collect();
    assert_eq!(from_a, [B, C], "both within 800 m, the nearer first");
    let (ab, ac) = (d.neighbours(A)[0].1, d.neighbours(A)[1].1);
    assert!(ab > 0.0 && (ac / ab - 3.0).abs() < 0.01, "a walk three times as long: {ab} {ac}");
    // Each dock touches both layers.
    assert_eq!(s.resource_rows()[B as usize].access_points.len(), 2);
}

#[test]
fn rows_off_the_layers_or_without_slots_are_left_out_and_ids_must_be_unique() {
    let (road, layers) = grid();
    let far = LonLat { lon: at(&road, 0).lon + 1.0, lat: at(&road, 0).lat };
    let rows = vec![
        row("far", far, 10, None),
        row("none", at(&road, 2), 0, None),
        row("kept", at(&road, 4), 1, None),
    ];
    let s = setup(&rows, &layers).expect("set up");
    let r = s.report();
    assert_eq!((r.rows, r.kept, r.off_layer, r.empty), (3, 1, 1, 1));
    let twice = vec![row("x", at(&road, 0), 1, None), row("x", at(&road, 1), 1, None)];
    let err = setup(&twice, &layers).unwrap_err();
    assert!(matches!(err, ServiceError::BadRow(_)), "{err}");
    // Without a count, a dock starts at its share of the slots.
    let half = vec![row("h", at(&road, 0), 9, None)];
    let s = setup(&half, &layers).expect("set up");
    assert_eq!(docks(&s).start[0], 5, "half of nine, rounded");
}

#[test]
fn a_trip_takes_a_bike_rides_and_returns_it() {
    let s = three([1, 0, 2], 2);
    let d = DockDefaults::SHIPPED;
    let t = trip(0, 100.0, A, B, &s);
    let out = load(&s, &[t], 3600.0);
    let o = out.legs[0];
    assert_eq!((o.picked, o.returned, o.redirections), (Some(A), Some(B), 0));
    assert_eq!(o.status, LegStatus::Served);
    assert_eq!(o.picked_s, 100.0);
    assert_eq!(o.requested_s, 100.0);
    assert_eq!(o.arrived_s, 100.0 + d.pickup_s + t.ride_s + d.return_s + t.egress_s);
    let bins = out.docks[0].as_ref().expect("a docked service's bins");
    assert_eq!(sum(&bins.pickups), 1.0);
    assert_eq!(sum(&bins.returns), 1.0);
    // A empty from 100 s to the end of its first bin and every bin after.
    assert_eq!(bins.empty_s[0], 800.0);
}

#[test]
fn a_traveller_at_an_empty_dock_walks_on_to_the_nearest_with_a_bike() {
    // A has one bike; the second traveller finds it empty, tries B (empty: the first bike is
    // still being ridden), then C, which has bikes (and room for the first one's return).
    let s = three([1, 0, 2], 3);
    let first = trip(0, 100.0, A, C, &s);
    let second = trip(1, 110.0, A, C, &s);
    let out = load(&s, &[first, second], 3600.0);
    let o = out.legs[1];
    let d = docks(&s);
    let (ab, bc) = (d.neighbours(A)[0].1, d.neighbours(B).iter().find(|n| n.0 == C).unwrap().1);
    assert_eq!((o.picked, o.redirections), (Some(C), 2));
    assert_eq!(o.extra_walk_s, ab + bc);
    assert_eq!(o.picked_s, 110.0 + ab + bc);
    assert_eq!(o.requested_s, 110.0, "the first request's second is kept");
    assert_eq!(sum(&out.docks[0].as_ref().unwrap().redirected), 2.0);
}

#[test]
fn a_pickup_fails_past_the_docks_near_the_planned_one_and_the_traveller_walks() {
    let s = three([0, 0, 0], 2);
    let t = trip(0, 100.0, A, C, &s);
    let out = load(&s, &[t], 3600.0);
    let o = out.legs[0];
    assert_eq!((o.picked, o.returned, o.redirections), (None, None, 2));
    assert_eq!(o.status, LegStatus::Failed);
    let walked = o.extra_walk_s;
    assert_eq!(o.arrived_s, 100.0 + walked + 500.0, "the walk on from the last dock tried");
    let bins = out.docks[0].as_ref().unwrap();
    assert_eq!(sum(&bins.failed), 1.0);
    assert_eq!(sum(&bins.pickups), 0.0);
}

#[test]
fn a_full_return_dock_sends_the_bike_on_and_with_no_room_near_it_overflows() {
    // B is full; the bike ridden from C to B goes on to A, which has room and from which the
    // destination is nearer (100 s on foot against 500 s from B).
    let s = three([0, 2, 2], 2);
    let t = trip(0, 100.0, C, B, &s);
    let out = load_with(&s, &[t], 3600.0, Arc::new(Near(Legs::of(&s), A)));
    let o = out.legs[0];
    assert_eq!((o.picked, o.returned, o.redirections, o.overflow), (Some(C), Some(A), 1, false));
    let ba = Legs::of(&s).ride_s(0, B, A);
    let d = DockDefaults::SHIPPED;
    assert_eq!(o.arrived_s, 100.0 + d.pickup_s + t.ride_s + ba + d.return_s + 100.0);
    // With the destination as near on foot from B as from A, riding on is not worth it: the
    // bike is left at B above capacity (S302).
    let out = load(&s, &[t], 3600.0);
    let o = out.legs[0];
    assert_eq!((o.returned, o.redirections, o.overflow), (Some(B), 0, true));
    // Never back to the dock the bike came from: B full, C (where it was taken) the nearest.
    let s = three([1, 2, 0], 2);
    let t = trip(0, 100.0, C, B, &s);
    let t = Leg { pickup_dock: A, ride_s: Legs::of(&s).ride_s(0, A, B), ..t };
    let out = load_with(&s, &[t], 3600.0, Arc::new(Near(Legs::of(&s), A)));
    assert_eq!(out.legs[0].picked, Some(A));
    assert_ne!(out.legs[0].returned, Some(A), "not back to its pickup dock");
    // A full dock with no other within 800 m: the bike is docked anyway, above its slots.
    let (road, layers) = grid();
    let rows = vec![row("A", at(&road, 0), 2, Some(2)), row("Z", at(&road, 24), 1, Some(1))];
    let s = setup(&rows, &layers).expect("set up");
    assert!(docks(&s).neighbours(1).is_empty(), "the far corner is 1.1 km away");
    let out = load(&s, &[trip(0, 100.0, 0, 1, &s)], 3600.0);
    let o = out.legs[0];
    assert_eq!((o.returned, o.overflow), (Some(1), true));
    assert_eq!(out.overflow_max[0], 1.0);
}

#[test]
fn the_people_a_traveller_stands_for_take_as_many_bikes() {
    let s = three([2, 0, 0], 2);
    let heavy = Leg { weight: 3, ..trip(0, 100.0, A, B, &s) };
    let out = load(&s, &[heavy], 3600.0);
    assert_eq!(out.legs[0].picked, None, "two bikes do not serve three people");
    let pair = Leg { weight: 2, ..trip(0, 100.0, A, B, &s) };
    let out = load(&s, &[pair], 3600.0);
    assert_eq!(out.legs[0].picked, Some(A));
    assert_eq!(sum(&out.docks[0].as_ref().unwrap().pickups), 2.0);
}

#[test]
fn outcomes_do_not_depend_on_the_order_trips_are_given() {
    let s = three([1, 1, 1], 3);
    let trips: Vec<Leg> =
        (0..12).map(|k| trip(k, f64::from(k % 4) * 50.0, k % 3, (k + 1) % 3, &s)).collect();
    let forward = load(&s, &trips, 3600.0);
    let mut reversed = trips.clone();
    reversed.reverse();
    let backward = load(&s, &reversed, 3600.0);
    // Ties at one second go by trip id, not position: each leg fares the same.
    let n = trips.len();
    for i in 0..n {
        assert_eq!(
            format!("{:?}", forward.legs[i]),
            format!("{:?}", backward.legs[n - 1 - i]),
            "trip {i}"
        );
    }
    assert_eq!(forward.docks, backward.docks);
    let again = load(&s, &trips, 3600.0);
    assert_eq!(format!("{forward:?}"), format!("{again:?}"), "the same trips, the same loading");
}

#[test]
fn a_dock_empty_at_the_start_costs_the_walk_to_its_nearest_neighbour_with_bikes() {
    let s = three([0, 0, 2], 2);
    let costs = DockCosts::initial(&s, (0.0, 3600.0));
    let d = docks(&s);
    let to_c = d.neighbours(A).iter().find(|n| n.0 == C).unwrap().1;
    assert_eq!(costs.pickup_s(A, 0.0), to_c, "B is empty too, so the walk to C");
    assert_eq!(costs.pickup_s(C, 0.0), 0.0);
    // C is full: a return there costs the walk to B, the nearest with room.
    let to_b = d.neighbours(C).iter().find(|n| n.0 == B).unwrap().1;
    assert_eq!(costs.return_s(C, 1000.0), to_b);
    // The expectations scale them (S303): the default keeps them, the factors multiply each
    // side, and the worst case expects every dock empty and full: the walk to the nearest
    // neighbour at both ends.
    let e = ServiceExpectations::SHIPPED;
    assert_eq!(costs.clone().expected(&s, &e), costs);
    let half = ServiceExpectations { dock_pickup_factor: 0.5, dock_return_factor: 0.0, ..e };
    let scaled = costs.clone().expected(&s, &half);
    assert_eq!(scaled.pickup_s(A, 0.0), to_c / 2.0);
    assert_eq!(scaled.return_s(C, 1000.0), 0.0);
    let worst = costs.expected(&s, &ServiceExpectations::preset("pessimistic").unwrap());
    let nearest = |k: u32| d.neighbours(k)[0].1;
    for k in [A, B, C] {
        assert_eq!(worst.pickup_s(k, 0.0), nearest(k), "dock {k}");
        assert_eq!(worst.return_s(k, 3000.0), nearest(k), "dock {k}");
    }
    let optimistic = ServiceExpectations::preset("optimistic").unwrap();
    let none = DockCosts::initial(&s, (0.0, 3600.0)).expected(&s, &optimistic);
    assert_eq!((none.pickup_s(A, 0.0), none.return_s(C, 1000.0)), (0.0, 0.0));
}

#[test]
fn expectations_are_set_by_name_and_refused_with_the_names() {
    use std::collections::BTreeMap;
    let e = ServiceExpectations::SHIPPED
        .with_options(&BTreeMap::from([("taxi_wait_s".to_string(), 300.0)]))
        .unwrap();
    assert_eq!(e.taxi_wait_s, 300.0);
    let err = ServiceExpectations::SHIPPED
        .with_options(&BTreeMap::from([("taxi_wait".to_string(), 300.0)]))
        .unwrap_err();
    assert!(err.contains("taxi_wait_s") && err.contains("dock_worst_case"), "{err}");
    let err = ServiceExpectations::SHIPPED
        .with_options(&BTreeMap::from([("taxi_fail_share".to_string(), 1.5)]))
        .unwrap_err();
    assert!(err.contains("from 0 to 1"), "{err}");
    let err = ServiceExpectations::preset("cheerful").unwrap_err();
    assert!(err.contains("optimistic") && err.contains("pessimistic"), "{err}");
    assert_eq!(ServiceExpectations::preset("default").unwrap(), ServiceExpectations::SHIPPED);
    assert_eq!(ServiceExpectations::SHIPPED.taxi_wait_s, 600.0);
}

#[test]
fn costs_read_seconds_from_midnight_with_bins_from_the_window_s_start() {
    // The same docks over 07:00..08:00: a second of the window reads its bin.
    let s = three([0, 0, 2], 2);
    let costs = DockCosts::initial(&s, (25_200.0, 28_800.0));
    let to_c = docks(&s).neighbours(A).iter().find(|n| n.0 == C).unwrap().1;
    assert_eq!(costs.pickup_s(A, 25_200.0), to_c);
    assert_eq!(costs.pickup_s(A, 28_000.0), to_c);
    assert_eq!(costs.pickup_s(C, 25_200.0), 0.0);
}

#[test]
fn options_are_checked_by_name_and_range() {
    use std::collections::BTreeMap;
    let ok = DockDefaults::from_options(&BTreeMap::from([("redirect_max".to_string(), 5.0)]))
        .expect("a known option");
    assert_eq!(ok.redirect_count(), 5);
    let unknown = DockDefaults::from_options(&BTreeMap::from([("slots".to_string(), 1.0)]));
    assert!(unknown.unwrap_err().to_string().contains("redirect_max_m"));
    for (name, value) in [("start_share", 1.5), ("bin_s", 0.0), ("pickup_s", -1.0)] {
        let bad = DockDefaults::from_options(&BTreeMap::from([(name.to_string(), value)]));
        assert!(matches!(bad, Err(ServiceError::BadOption { .. })), "{name} = {value}");
    }
    assert_eq!(DockDefaults::NAMES.len(), DockDefaults::SHIPPED.values().len());
}

/// A small deterministic generator (no dependency): `x <- x * a + c` modulo 2^64.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self, n: u32) -> u32 {
        self.0 =
            self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        u32::try_from((self.0 >> 33) % u64::from(n)).expect("below n")
    }
}

#[test]
fn a_day_of_200_random_legs_conserves_bikes_and_replays_exactly() {
    let s = three([3, 1, 2], 4);
    let mut rng = Lcg(7);
    let legs: Vec<Leg> = (0..200)
        .map(|k| {
            let from = rng.next(3);
            let to = (from + 1 + rng.next(2)) % 3;
            let at = f64::from(rng.next(7_200));
            Leg { weight: 1 + rng.next(2), ..trip(k, at, from, to, &s) }
        })
        .collect();
    // `load` checks invariant_violations == 0 (and debug builds assert after every event).
    let out = load(&s, &legs, 7_200.0);
    let bins = out.docks[0].as_ref().unwrap();
    assert!(sum(&bins.pickups) > 0.0 && sum(&bins.failed) > 0.0, "a busy day: some fail");
    // Every leg requested is served, failed or still riding at the end.
    for o in &out.legs {
        assert!(
            matches!(o.status, LegStatus::Served | LegStatus::Failed | LegStatus::Truncated),
            "{o:?}"
        );
    }
    let again = load(&s, &legs, 7_200.0);
    assert_eq!(format!("{out:?}"), format!("{again:?}"), "the same day twice");
    let mut shuffled: Vec<(usize, Leg)> = legs.iter().copied().enumerate().collect();
    shuffled.reverse();
    shuffled.rotate_left(37);
    let given: Vec<Leg> = shuffled.iter().map(|x| x.1).collect();
    let other = load(&s, &given, 7_200.0);
    for (j, (i, _)) in shuffled.iter().enumerate() {
        assert_eq!(format!("{:?}", other.legs[j]), format!("{:?}", out.legs[*i]), "leg {i}");
    }
    assert_eq!(other.docks, out.docks);
}
