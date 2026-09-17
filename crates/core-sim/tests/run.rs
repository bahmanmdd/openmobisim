//! `Run`: the Phase 1 vertical slice — demand through S133's placeholder
//! routing through level-0 loading to one KPI and S57's completion stats.
//!
//! Fixtures are hand-built directly (`RoadNetworkBuilder` for the network,
//! `RawTrip`/`RawPerson` structs for demand — no Parquet file needed,
//! `core_demand::build_travellers` takes them straight), the same
//! hand-authored-fixture convention `core-loading`'s and `core-demand`'s own
//! tests use.

use std::sync::Arc;

use openmobisim_core_demand::{ClassDefaults, Ownership, RawTrip, build_travellers};
use openmobisim_core_graph::defaults::{GlobalMultipliers, RoadClass, SignalDefaults};
use openmobisim_core_graph::geometry::LonLat;
use openmobisim_core_graph::network::{LinkSpec, RoadNetwork, RoadNetworkBuilder};
use openmobisim_core_sim::Run;
use openmobisim_core_types::diagnostics::Diagnostics;
use openmobisim_core_types::ids::LinkId;
use openmobisim_core_types::time::Second;

/// Three nodes in a line, `a -> b -> c`, each leg its own two-way street:
/// `RoadNetworkBuilder::add_link` is directed and makes exactly one link per
/// call, so a two-way leg needs both directions added explicitly.
fn line_network() -> RoadNetwork {
    let mut b = RoadNetworkBuilder::new();
    b.add_node("a", LonLat::new(4.800, 45.700));
    b.add_node("b", LonLat::new(4.801, 45.700));
    b.add_node("c", LonLat::new(4.802, 45.700));
    b.add_link("ab", "a", "b", LinkSpec::new(RoadClass::Primary));
    b.add_link("ba", "b", "a", LinkSpec::new(RoadClass::Primary));
    b.add_link("bc", "b", "c", LinkSpec::new(RoadClass::Primary));
    b.add_link("cb", "c", "b", LinkSpec::new(RoadClass::Primary));
    let mut diagnostics = Diagnostics::new();
    b.build(GlobalMultipliers::default(), SignalDefaults::SHIPPED, &mut diagnostics)
        .expect("buildable")
}

fn trip(
    traveller_id: &str,
    trip_seq: u32,
    origin: (f64, f64),
    destination: (f64, f64),
    departure: u32,
) -> RawTrip {
    RawTrip {
        traveller_id: traveller_id.to_string(),
        trip_seq,
        origin: LonLat::new(origin.0, origin.1),
        destination: LonLat::new(destination.0, destination.1),
        departure_time: Second(departure),
        user_class: "commuter".to_string(),
        weight: None,
    }
}

/// The same three nodes, plus an island far away: `z` and `y`, joined by a
/// two-way street of their own and linked to nothing else — to make
/// `shortest_path` fail on purpose. (The island has a road: a node with no
/// car link at all is not a place a car trip starts or ends.)
fn network_with_an_unreachable_node() -> RoadNetwork {
    let mut b = RoadNetworkBuilder::new();
    b.add_node("a", LonLat::new(4.800, 45.700));
    b.add_node("b", LonLat::new(4.801, 45.700));
    b.add_node("c", LonLat::new(4.802, 45.700));
    b.add_node("z", LonLat::new(5.500, 46.500));
    b.add_node("y", LonLat::new(5.501, 46.500));
    b.add_link("zy", "z", "y", LinkSpec::new(RoadClass::Primary));
    b.add_link("yz", "y", "z", LinkSpec::new(RoadClass::Primary));
    b.add_link("ab", "a", "b", LinkSpec::new(RoadClass::Primary));
    b.add_link("ba", "b", "a", LinkSpec::new(RoadClass::Primary));
    b.add_link("bc", "b", "c", LinkSpec::new(RoadClass::Primary));
    b.add_link("cb", "c", "b", LinkSpec::new(RoadClass::Primary));
    let mut diagnostics = Diagnostics::new();
    b.build(GlobalMultipliers::default(), SignalDefaults::SHIPPED, &mut diagnostics)
        .expect("buildable")
}

const A: (f64, f64) = (4.800, 45.700);
const C: (f64, f64) = (4.802, 45.700);
const Z: (f64, f64) = (5.500, 46.500);

fn car_owning_defaults() -> ClassDefaults {
    ClassDefaults::new().with_default("commuter", Ownership { car: true, ..Ownership::NONE })
}

/// The free-flow time of `a -> b -> c`, floored per link the same way
/// `core-loading` does — the exact number a completed a-to-c trip's travel
/// time must equal at weight 1.
fn expected_a_to_c_seconds(network: &RoadNetwork) -> u32 {
    let ab = network.link_external_ids().typed_id_of::<LinkId>("ab").expect("known link");
    let bc = network.link_external_ids().typed_id_of::<LinkId>("bc").expect("known link");
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "test assertion")]
    let seconds = network.free_flow_time(ab).get() as u32 + network.free_flow_time(bc).get() as u32;
    seconds
}

#[test]
fn a_single_car_trip_completes_with_the_right_travel_time() {
    let network = Arc::new(line_network());
    let raw_trips = vec![trip("alice", 0, A, C, 0)];
    let mut build_diagnostics = Diagnostics::new();
    let (travellers, trips) =
        build_travellers(raw_trips, Vec::new(), &car_owning_defaults(), 1, &mut build_diagnostics)
            .expect("buildable");

    let mut run =
        Run::new(network.clone(), Arc::new(travellers), Arc::new(trips), Second(1_000_000));
    let mut diagnostics = Diagnostics::new();
    let result = run.execute(&mut diagnostics);

    assert_eq!(result.completion.total_trips, 1);
    assert_eq!(result.completion.completed, 1);
    assert_eq!(result.completion.truncated, 0);
    assert_eq!(result.completion.no_vehicle_available, 0);
    assert_eq!(result.completion.no_feasible_path, 0);
    #[allow(clippy::float_cmp, reason = "no arithmetic happened; both sides are exact")]
    {
        assert_eq!(result.completion.completion_rate(), 1.0);
        assert_eq!(result.total_travel_time.get(), f64::from(expected_a_to_c_seconds(&network)));
    }
}

/// S153: a car neither starts at a footway-only node nor routes over a
/// footway, even when the footway is the shorter way.
#[test]
fn a_car_trip_ignores_footways() {
    let mut b = RoadNetworkBuilder::new();
    b.add_node("a", LonLat::new(4.800, 45.700));
    b.add_node("b", LonLat::new(4.801, 45.701));
    b.add_node("c", LonLat::new(4.802, 45.700));
    b.add_node("p", LonLat::new(4.7999, 45.700)); // closest to A, on the footway only
    b.add_link("ab", "a", "b", LinkSpec::new(RoadClass::Primary));
    b.add_link("bc", "b", "c", LinkSpec::new(RoadClass::Primary));
    b.add_link("pa", "p", "a", LinkSpec::new(RoadClass::Footway));
    b.add_link("ac", "a", "c", LinkSpec::new(RoadClass::Footway));
    let network = Arc::new(
        b.build(GlobalMultipliers::default(), SignalDefaults::SHIPPED, &mut Diagnostics::new())
            .expect("buildable"),
    );
    let raw_trips = vec![trip("alice", 0, (4.7999, 45.700), C, 0)];
    let (travellers, trips) =
        build_travellers(raw_trips, Vec::new(), &car_owning_defaults(), 1, &mut Diagnostics::new())
            .expect("buildable");
    let mut run =
        Run::new(network.clone(), Arc::new(travellers), Arc::new(trips), Second(1_000_000));
    let result = run.execute(&mut Diagnostics::new());

    assert_eq!(result.completion.completed, 1);
    let id = |e: &str| network.link_external_ids().typed_id_of::<LinkId>(e).expect("link");
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "test assertion")]
    let by_road = network.free_flow_time(id("ab")).get() as u32
        + network.free_flow_time(id("bc")).get() as u32;
    #[allow(clippy::float_cmp, reason = "both sides are small integer seconds, exactly")]
    {
        assert_eq!(result.total_travel_time.get(), f64::from(by_road), "the car takes ab, bc");
    }
}

#[test]
fn a_second_trip_uses_the_vehicle_the_first_trip_relocated() {
    // Proves relocation actually happens: if it didn't, trip 2's origin "c"
    // would find no car there (the car would still show as being at "a").
    let network = Arc::new(line_network());
    let raw_trips = vec![trip("alice", 0, A, C, 0), trip("alice", 1, C, A, 10_000)];
    let mut build_diagnostics = Diagnostics::new();
    let (travellers, trips) =
        build_travellers(raw_trips, Vec::new(), &car_owning_defaults(), 1, &mut build_diagnostics)
            .expect("buildable");

    let mut run =
        Run::new(network.clone(), Arc::new(travellers), Arc::new(trips), Second(1_000_000));
    let mut diagnostics = Diagnostics::new();
    let result = run.execute(&mut diagnostics);

    assert_eq!(result.completion.total_trips, 2);
    assert_eq!(result.completion.completed, 2);
    assert_eq!(result.completion.no_vehicle_available, 0);
    #[allow(clippy::float_cmp, reason = "both sides are small integer seconds, exactly")]
    {
        assert_eq!(
            result.total_travel_time.get(),
            2.0 * f64::from(expected_a_to_c_seconds(&network)),
            "both legs cover the same links, so both cost the same travel time"
        );
    }
}

#[test]
fn a_traveller_without_a_car_is_not_simulated() {
    let network = Arc::new(line_network());
    let raw_trips = vec![trip("bob", 0, A, C, 0)];
    let mut build_diagnostics = Diagnostics::new();
    // No class default declares ownership, and there is no persons.parquet.
    let (travellers, trips) =
        build_travellers(raw_trips, Vec::new(), &ClassDefaults::new(), 1, &mut build_diagnostics)
            .expect("buildable");

    let mut run = Run::new(network, Arc::new(travellers), Arc::new(trips), Second(1_000_000));
    let mut diagnostics = Diagnostics::new();
    let result = run.execute(&mut diagnostics);

    assert_eq!(result.completion.total_trips, 1);
    assert_eq!(result.completion.no_vehicle_available, 1);
    assert_eq!(result.completion.completed, 0);
    #[allow(clippy::float_cmp, reason = "no arithmetic happened; a zero KPI stays exactly zero")]
    {
        assert_eq!(result.total_travel_time.get(), 0.0);
    }
    assert_eq!(
        diagnostics.count_of(openmobisim_core_sim::run::local_codes::NO_VEHICLE_AVAILABLE),
        1
    );
}

#[test]
fn a_trip_past_the_window_is_truncated_not_completed() {
    let network = Arc::new(line_network());
    let raw_trips = vec![trip("carol", 0, A, C, 0)];
    let mut build_diagnostics = Diagnostics::new();
    let (travellers, trips) =
        build_travellers(raw_trips, Vec::new(), &car_owning_defaults(), 1, &mut build_diagnostics)
            .expect("buildable");

    // A window shorter than the trip's own travel time.
    let mut run = Run::new(network, Arc::new(travellers), Arc::new(trips), Second(1));
    let mut diagnostics = Diagnostics::new();
    let result = run.execute(&mut diagnostics);

    assert_eq!(result.completion.completed, 0);
    assert_eq!(result.completion.truncated, 1);
    #[allow(clippy::float_cmp, reason = "no arithmetic happened; a zero KPI stays exactly zero")]
    {
        assert_eq!(
            result.total_travel_time.get(),
            0.0,
            "a truncated trip must not contribute to the KPI"
        );
    }
    assert_eq!(diagnostics.count_of(openmobisim_core_types::diagnostics::codes::TRIP_TRUNCATED), 1);
}

#[test]
fn traveller_weight_scales_the_kpi() {
    let network = Arc::new(line_network());
    let mut heavy = trip("dana", 0, A, C, 0);
    heavy.weight = Some(3);
    let raw_trips = vec![trip("erin", 0, A, C, 0), heavy];
    let mut build_diagnostics = Diagnostics::new();
    let (travellers, trips) =
        build_travellers(raw_trips, Vec::new(), &car_owning_defaults(), 1, &mut build_diagnostics)
            .expect("buildable");

    let mut run =
        Run::new(network.clone(), Arc::new(travellers), Arc::new(trips), Second(1_000_000));
    let mut diagnostics = Diagnostics::new();
    let result = run.execute(&mut diagnostics);

    let one_leg = f64::from(expected_a_to_c_seconds(&network));
    #[allow(clippy::float_cmp, reason = "both sides are small integer seconds, exactly")]
    {
        assert_eq!(
            result.total_travel_time.get(),
            one_leg * 1.0 + one_leg * 3.0,
            "erin (weight 1) and dana (weight 3) each ride the same links"
        );
    }
}

#[test]
fn no_feasible_path_is_counted_and_strands_the_rest_of_the_day() {
    let network = Arc::new(network_with_an_unreachable_node());
    // Trip 1 targets the isolated node z (no path exists); trip 2 continues
    // from where trip 1 was *supposed* to end, per S128 — but the vehicle
    // never actually got there, so trip 2 must come up short too.
    let raw_trips = vec![trip("hank", 0, A, Z, 0), trip("hank", 1, Z, A, 10_000)];
    let mut build_diagnostics = Diagnostics::new();
    let (travellers, trips) =
        build_travellers(raw_trips, Vec::new(), &car_owning_defaults(), 1, &mut build_diagnostics)
            .expect("buildable");

    let mut run = Run::new(network, Arc::new(travellers), Arc::new(trips), Second(1_000_000));
    let mut diagnostics = Diagnostics::new();
    let result = run.execute(&mut diagnostics);

    assert_eq!(result.completion.total_trips, 2);
    assert_eq!(result.completion.no_feasible_path, 1, "trip 1 has nowhere to route to");
    assert_eq!(
        result.completion.no_vehicle_available, 1,
        "trip 2's car never arrived at z, so it is not available for trip 2 either"
    );
    assert_eq!(result.completion.completed, 0);
    assert_eq!(
        diagnostics.count_of(openmobisim_core_types::diagnostics::codes::NO_FEASIBLE_PATH),
        1
    );
}

#[test]
fn completion_counts_always_sum_to_total_trips() {
    // A mixed scenario exercising several buckets at once: alice completes
    // both her trips, hank cannot reach the isolated node z, and ivy is
    // given no car-owning class at all. The buckets must partition every
    // trip in the demand — nothing double-counted, nothing dropped.
    let network = Arc::new(network_with_an_unreachable_node());
    let mut ivy_trip = trip("ivy", 0, A, C, 0);
    ivy_trip.user_class = "pedestrian".to_string(); // no default declared for this class
    let raw_trips = vec![
        trip("alice", 0, A, C, 0),
        trip("alice", 1, C, A, 5000),
        trip("hank", 0, A, Z, 0),
        ivy_trip,
    ];
    let mut build_diagnostics = Diagnostics::new();
    let (travellers, trips) =
        build_travellers(raw_trips, Vec::new(), &car_owning_defaults(), 1, &mut build_diagnostics)
            .expect("buildable");

    let mut run = Run::new(network, Arc::new(travellers), Arc::new(trips), Second(1_000_000));
    let mut diagnostics = Diagnostics::new();
    let result = run.execute(&mut diagnostics);

    let c = &result.completion;
    assert_eq!(c.no_vehicle_available, 1, "ivy has no car-owning class");
    assert_eq!(c.no_feasible_path, 1, "hank targets the isolated node");
    assert_eq!(c.completed, 2, "alice's two trips");
    assert_eq!(
        c.completed + c.truncated + c.no_vehicle_available + c.no_feasible_path,
        c.total_trips,
        "every trip must land in exactly one bucket"
    );
}

#[test]
fn a_run_is_deterministic() {
    let build = || {
        let network = Arc::new(line_network());
        let raw_trips = vec![
            trip("alice", 0, A, C, 0),
            trip("alice", 1, C, A, 5000),
            trip("bob", 0, A, C, 100),
        ];
        let mut build_diagnostics = Diagnostics::new();
        let (travellers, trips) = build_travellers(
            raw_trips,
            Vec::new(),
            &car_owning_defaults(),
            1,
            &mut build_diagnostics,
        )
        .expect("buildable");
        let mut run = Run::new(network, Arc::new(travellers), Arc::new(trips), Second(1_000_000));
        let mut diagnostics = Diagnostics::new();
        run.execute(&mut diagnostics)
    };

    let a = build();
    let b = build();
    assert_eq!(a, b);
}

#[test]
fn events_are_one_row_per_trip_and_match_the_completion_buckets() {
    use openmobisim_core_sim::EventType;

    let network = Arc::new(network_with_an_unreachable_node());
    let mut no_car_trip = trip("ivy_no_car", 0, A, C, 0);
    no_car_trip.user_class = "pedestrian".to_string(); // no default declared for this class
    let raw_trips = vec![trip("alice", 0, A, C, 0), trip("hank", 0, A, Z, 0), no_car_trip];
    let mut build_diagnostics = Diagnostics::new();
    let (travellers, trips) =
        build_travellers(raw_trips, Vec::new(), &car_owning_defaults(), 1, &mut build_diagnostics)
            .expect("buildable");

    let mut run = Run::new(network, Arc::new(travellers), Arc::new(trips), Second(1_000_000));
    let mut diagnostics = Diagnostics::new();
    let result = run.execute(&mut diagnostics);

    assert_eq!(
        result.events.len(),
        result.completion.total_trips as usize,
        "one event row per trip, no more, no fewer"
    );
    let count = |t: EventType| result.events.iter().filter(|e| e.event_type == t).count();
    assert_eq!(count(EventType::TripCompleted), result.completion.completed as usize);
    assert_eq!(count(EventType::TripTruncated), result.completion.truncated as usize);
    assert_eq!(count(EventType::NoFeasiblePath), result.completion.no_feasible_path as usize);
    // ivy_no_car's class has no declared default, so she owns nothing.
    assert_eq!(
        count(EventType::NoVehicleAvailable),
        result.completion.no_vehicle_available as usize
    );
}
