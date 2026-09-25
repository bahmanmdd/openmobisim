//! `Run`: the Phase 1 vertical slice — demand through route sets (S165; S133's placeholder before that)
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
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_loading::FidelityLevel;
use openmobisim_core_sim::{FlowMotor, Run};
use openmobisim_core_types::diagnostics::Diagnostics;
use openmobisim_core_types::ids::LinkId;
use openmobisim_core_types::time::Second;
use openmobisim_core_types::units::Duration;

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
        mode: None,
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

/// The free-flow time of `a -> b -> c`, floored once (S88; S161, F1) — the
/// exact number a completed a-to-c trip's travel time must equal at weight 1.
fn expected_a_to_c_seconds(network: &RoadNetwork) -> u32 {
    let ab = network.link_external_ids().typed_id_of::<LinkId>("ab").expect("known link");
    let bc = network.link_external_ids().typed_id_of::<LinkId>("bc").expect("known link");
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "test assertion")]
    let seconds = (network.free_flow_time(ab).get() + network.free_flow_time(bc).get()) as u32;
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
    let by_road =
        (network.free_flow_time(id("ab")).get() + network.free_flow_time(id("bc")).get()) as u32;
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

// --- Route sets (S165) --------------------------------------------------------------

/// A -> B -> D on top (200 m) and A -> C -> D below (240 m).
fn diamond_network() -> RoadNetwork {
    let mut b = RoadNetworkBuilder::new();
    for (n, x, y) in [("a", 0.0, 0.0), ("b", 100.0, 60.0), ("c", 100.0, -60.0), ("d", 200.0, 0.0)] {
        b.add_node(n, LonLat::new(4.8 + x / 77_800.0, 45.7 + y / 110_574.0));
    }
    for (name, from, to, len) in [
        ("ab", "a", "b", 100.0),
        ("bd", "b", "d", 100.0),
        ("ac", "a", "c", 120.0),
        ("cd", "c", "d", 120.0),
    ] {
        let mut spec = LinkSpec::new(RoadClass::Residential);
        spec.length_m = Some(len);
        b.add_link(name, from, to, spec);
    }
    b.build(GlobalMultipliers::default(), SignalDefaults::SHIPPED, &mut Diagnostics::new())
        .expect("buildable")
}

fn diamond_run(
    generator: Option<std::sync::Arc<dyn openmobisim_core_routes::RouteSetGenerator>>,
) -> openmobisim_core_sim::RunResult {
    let network = Arc::new(diamond_network());
    let at = |x: f64, y: f64| (4.8 + x / 77_800.0, 45.7 + y / 110_574.0);
    let raw = vec![
        trip("alice", 0, at(0.0, 0.0), at(200.0, 0.0), 0),
        trip("bob", 0, at(0.0, 0.0), at(200.0, 0.0), 60),
    ];
    let mut d = Diagnostics::new();
    let (travellers, trips) =
        build_travellers(raw, Vec::new(), &car_owning_defaults(), 1, &mut d).expect("buildable");
    let mut run = Run::new(network, Arc::new(travellers), Arc::new(trips), Second(3600));
    if let Some(g) = generator {
        run = run.with_route_generator(g);
    }
    run.execute(&mut Diagnostics::new())
}

#[test]
fn a_run_keeps_the_route_sets_it_routed_from_and_takes_each_pairs_best_route() {
    let result = diamond_run(None);
    let sets = result.route_sets.expect("kept");
    assert_eq!(sets.keys().len(), 1, "both trips share one origin-destination pair");
    assert_eq!(sets.routes(0).count(), 2, "the top road and the bottom road");
    assert_eq!(sets.method(), "penalty");
    // The trips took the top road: 200 m at 30 km/h.
    let top = sets.routes(0).next().expect("best").cost;
    assert_eq!(result.completion.completed, 2);
    assert!((result.total_travel_time.get() - 2.0 * f64::from(top).floor()).abs() <= 2.0);
}

#[test]
fn the_method_is_selectable_and_changes_nothing_while_only_the_best_route_is_used() {
    let shortest =
        openmobisim_core_routes::generator("shortest", &Default::default()).expect("built in");
    let plain = diamond_run(Some(std::sync::Arc::from(shortest)));
    let default = diamond_run(None);
    assert_eq!(plain.route_sets.as_ref().map(|s| s.routes(0).count()), Some(1));
    assert_eq!(default.route_sets.as_ref().map(|s| s.routes(0).count()), Some(2));
    assert_eq!(plain.completion, default.completion);
    assert_eq!(plain.total_travel_time, default.total_travel_time);
}

// --- the run's identity: seed and fingerprint (S168) ---------------------------------

/// Everything a run's fingerprint should notice, as knobs on the diamond run.
#[derive(Clone)]
struct Knobs {
    seed: u64,
    top_length_m: f64,
    departure: u32,
    weight: Option<u32>,
    car: bool,
    window: u32,
    step: Option<(f64, FidelityLevel)>,
    method: &'static str,
    bins: Option<u32>,
}

impl Knobs {
    fn base() -> Self {
        Self {
            seed: 0,
            top_length_m: 100.0,
            departure: 60,
            weight: None,
            car: true,
            window: 3600,
            step: None,
            method: "penalty",
            bins: None,
        }
    }

    fn run(&self) -> Run {
        let mut b = RoadNetworkBuilder::new();
        for (n, x, y) in
            [("a", 0.0, 0.0), ("b", 100.0, 60.0), ("c", 100.0, -60.0), ("d", 200.0, 0.0)]
        {
            b.add_node(n, LonLat::new(4.8 + x / 77_800.0, 45.7 + y / 110_574.0));
        }
        for (name, from, to, len) in [
            ("ab", "a", "b", self.top_length_m),
            ("bd", "b", "d", 100.0),
            ("ac", "a", "c", 120.0),
            ("cd", "c", "d", 120.0),
        ] {
            let mut spec = LinkSpec::new(RoadClass::Residential);
            spec.length_m = Some(len);
            b.add_link(name, from, to, spec);
        }
        let network = Arc::new(
            b.build(GlobalMultipliers::default(), SignalDefaults::SHIPPED, &mut Diagnostics::new())
                .expect("buildable"),
        );
        let at = |x: f64, y: f64| (4.8 + x / 77_800.0, 45.7 + y / 110_574.0);
        let mut alice = trip("alice", 0, at(0.0, 0.0), at(200.0, 0.0), self.departure);
        alice.weight = self.weight;
        let defaults = ClassDefaults::new()
            .with_default("commuter", Ownership { car: self.car, ..Ownership::NONE });
        let (travellers, trips) =
            build_travellers(vec![alice], Vec::new(), &defaults, 1, &mut Diagnostics::new())
                .expect("buildable");
        let mut run =
            Run::new(network.clone(), Arc::new(travellers), Arc::new(trips), Second(self.window))
                .with_master_seed(self.seed)
                .with_route_generator(Arc::from(
                    openmobisim_core_routes::generator(self.method, &Default::default())
                        .expect("built in"),
                ));
        if let Some((step, level)) = self.step {
            let turns = Arc::new(TurnTable::build(&network, SignalDefaults::SHIPPED));
            run = run.with_flow_motor(FlowMotor::Ltm { turns, step: Duration(step), level });
        }
        if let Some(bin) = self.bins {
            run = run.with_link_bins(bin);
        }
        run
    }

    fn fingerprint(&self) -> u64 {
        self.run().description().fingerprint
    }
}

#[test]
fn a_fingerprint_is_the_same_for_the_same_inputs_before_and_after_the_run() {
    let mut run = Knobs::base().run();
    let before = run.description();
    assert_eq!(before, Knobs::base().run().description());
    let _ = run.execute(&mut Diagnostics::new());
    assert_eq!(before, run.description(), "running does not change what went in");
    assert_eq!(before.fingerprint_hex().len(), 16);
    assert_eq!(before.master_seed, 0);
    assert_eq!(before.route_method, "penalty");
    assert!(before.route_descriptor.starts_with("penalty"), "{}", before.route_descriptor);
    assert_eq!(
        (before.flow_level, before.flow_step_seconds, before.link_bin_seconds),
        (0, None, None)
    );
}

#[test]
fn every_input_that_decides_a_run_changes_its_fingerprint() {
    let base = Knobs::base().fingerprint();
    let changed: Vec<(&str, Knobs)> = vec![
        ("seed", Knobs { seed: 1, ..Knobs::base() }),
        ("a link's length", Knobs { top_length_m: 101.0, ..Knobs::base() }),
        ("a departure", Knobs { departure: 61, ..Knobs::base() }),
        ("a weight", Knobs { weight: Some(3), ..Knobs::base() }),
        ("what a traveller owns", Knobs { car: false, ..Knobs::base() }),
        ("the window", Knobs { window: 7200, ..Knobs::base() }),
        ("the loading engine", Knobs { step: Some((300.0, FidelityLevel::Full)), ..Knobs::base() }),
        ("the route method", Knobs { method: "shortest", ..Knobs::base() }),
        ("the bin length", Knobs { bins: Some(300), ..Knobs::base() }),
    ];
    for (what, knobs) in &changed {
        assert_ne!(knobs.fingerprint(), base, "{what} must change the fingerprint");
    }
    // Within the loading engine: its step and its level each count.
    let ltm = |step, level| Knobs { step: Some((step, level)), ..Knobs::base() }.fingerprint();
    let reference = ltm(300.0, FidelityLevel::Full);
    assert_ne!(ltm(60.0, FidelityLevel::Full), reference, "the step");
    assert_ne!(ltm(300.0, FidelityLevel::PointQueue), reference, "the level");
    let bins = |b| Knobs { bins: Some(b), ..Knobs::base() }.fingerprint();
    assert_ne!(bins(300), bins(600), "the bin length itself");
    // Nothing changed, nothing moves.
    assert_eq!(Knobs::base().fingerprint(), base);
}

#[test]
fn the_description_names_the_loading_engine_and_the_bins() {
    let knobs = Knobs {
        step: Some((120.0, FidelityLevel::SpatialQueue)),
        bins: Some(600),
        ..Knobs::base()
    };
    let d = knobs.run().description();
    assert_eq!(
        (d.flow_level, d.flow_step_seconds, d.link_bin_seconds),
        (3, Some(120.0), Some(600))
    );
    // The network fingerprint is of ids only: a longer link changes the run's fingerprint, not that.
    let longer = Knobs { top_length_m: 250.0, ..Knobs::base() }.run().description();
    assert_eq!(longer.network_fingerprint, Knobs::base().run().description().network_fingerprint);
    assert_ne!(longer.fingerprint, Knobs::base().run().description().fingerprint);
}

// --- route choice (S169) ---------------------------------------------------------------

/// `n` travellers, each with one trip from `a` to `d` across the diamond, on distinct departures.
fn choosing_run(
    n: u32,
    model: Option<Arc<dyn openmobisim_core_choice::ChoiceModel>>,
    seed: u64,
) -> Run {
    let network = Arc::new(diamond_network());
    let at = |x: f64, y: f64| (4.8 + x / 77_800.0, 45.7 + y / 110_574.0);
    let raw: Vec<RawTrip> =
        (0..n).map(|i| trip(&format!("p{i}"), 0, at(0.0, 0.0), at(200.0, 0.0), i % 3000)).collect();
    let (travellers, trips) =
        build_travellers(raw, Vec::new(), &car_owning_defaults(), 1, &mut Diagnostics::new())
            .expect("buildable");
    let mut run = Run::new(network, Arc::new(travellers), Arc::new(trips), Second(7200))
        .with_master_seed(seed);
    if let Some(m) = model {
        run = run.with_choice_model(m);
    }
    run
}

fn logit_with(options: &[(&str, f64)]) -> Arc<dyn openmobisim_core_choice::ChoiceModel> {
    let o: openmobisim_core_choice::Options =
        options.iter().map(|&(k, v)| (k.to_string(), v)).collect();
    Arc::from(openmobisim_core_choice::model("logit", &o).expect("built in"))
}

/// How many of the trips took route `k` of their (single) set.
fn took(result: &openmobisim_core_sim::RunResult, k: usize) -> u32 {
    let sets = result.route_sets.as_ref().expect("sets");
    let choices = result.route_choices.as_ref().expect("choices");
    let first = sets.route_range(0).start;
    u32::try_from(choices.route.iter().filter(|&&r| r as usize == first + k).count()).expect("few")
}

#[test]
fn by_default_everyone_takes_the_best_route_and_the_run_says_so() {
    let result = choosing_run(50, None, 0).execute(&mut Diagnostics::new());
    let choices = result.route_choices.expect("recorded");
    assert_eq!(choices.len(), 50);
    let sets = result.route_sets.expect("sets");
    assert!(choices.route.iter().all(|&r| r as usize == sets.route_range(0).start));
    assert!(choices.alternatives.iter().all(|&a| a == 2), "the top road and the bottom road");
    assert!(choices.probability.iter().all(|&p| (p - 1.0).abs() < 1e-15));
    assert!(choices.weight.iter().all(|&w| w == 1));
}

#[test]
fn a_logit_with_no_aversion_splits_travellers_evenly_and_the_loading_follows() {
    let n = 4_000;
    let even = logit_with(&[("beta_time_min", 0.0), ("beta_ln_path_size", 0.0)]);
    let result = choosing_run(n, Some(even), 11).execute(&mut Diagnostics::new());
    let (top, bottom) = (took(&result, 0), took(&result, 1));
    assert_eq!(top + bottom, n);
    let sigma = (f64::from(n) * 0.25).sqrt();
    assert!((f64::from(top) - f64::from(n) / 2.0).abs() < 5.0 * sigma, "{top} on the top road");
    let choices = result.route_choices.as_ref().expect("choices");
    assert!(choices.probability.iter().all(|&p| (p - 0.5).abs() < 1e-12));
    // The loading took each trip down the road it chose: total time is what those roads cost.
    let sets = result.route_sets.as_ref().expect("sets");
    let expected: f64 = choices.route.iter().map(|&r| f64::from(sets.route(r as usize).cost)).sum();
    assert!(
        (result.total_travel_time.get() - expected).abs() <= f64::from(n),
        "{} against {expected}",
        result.total_travel_time.get()
    );
    // ... which differs from everyone on the top road by thousands of seconds.
    let all_top = f64::from(n) * f64::from(sets.route(sets.route_range(0).start).cost);
    assert!(result.total_travel_time.get() > all_top + 5_000.0);
}

#[test]
fn a_strong_time_aversion_sends_almost_everyone_down_the_faster_road() {
    let keen = logit_with(&[("beta_time_min", -3.0)]);
    let result = choosing_run(1_000, Some(keen), 3).execute(&mut Diagnostics::new());
    // The bottom road is about 4.8 s (0.08 min) slower: e^-0.24 = 0.79, so it is still taken by
    // more than a third; a much stronger aversion is needed to empty it.
    assert!(took(&result, 1) > 300, "{}", took(&result, 1));
    let very = logit_with(&[("beta_time_min", -60.0)]);
    let result = choosing_run(1_000, Some(very), 3).execute(&mut Diagnostics::new());
    assert!(took(&result, 1) < 40, "{}", took(&result, 1));
}

#[test]
fn the_seed_decides_a_sampled_choice_and_nothing_else() {
    let even = || logit_with(&[("beta_time_min", 0.0), ("beta_ln_path_size", 0.0)]);
    let run = |seed| choosing_run(500, Some(even()), seed).execute(&mut Diagnostics::new());
    let (a, again, other) = (run(1), run(1), run(2));
    assert_eq!(a.route_choices, again.route_choices, "the same seed, the same choices");
    assert_ne!(a.route_choices, other.route_choices, "another seed, another draw");
    // The all-or-nothing default draws nothing: the seed cannot change who goes where.
    let det = |seed| choosing_run(500, None, seed).execute(&mut Diagnostics::new());
    assert_eq!(det(1).route_choices, det(2).route_choices);
}

#[test]
fn the_description_names_the_choice_model_and_the_streams_it_draws_from() {
    let default = choosing_run(5, None, 0).description();
    assert_eq!((default.choice_model.as_str(), default.live_streams.len()), ("deterministic", 0));
    let sampled = choosing_run(5, Some(logit_with(&[])), 0).description();
    assert_eq!(sampled.choice_model, "logit");
    assert_eq!(sampled.live_streams, ["choice"]);
    assert!(sampled.choice_descriptor.starts_with("logit;beta_ln_path_size=1"));
    assert_ne!(default.fingerprint, sampled.fingerprint);
    let tweaked = choosing_run(5, Some(logit_with(&[("beta_time_min", -0.3)])), 0).description();
    assert_ne!(tweaked.fingerprint, sampled.fingerprint, "a coefficient is an input");
}

#[test]
fn a_model_that_needs_an_attribute_routes_lack_stops_the_run_and_says_what_exists() {
    let mut run = choosing_run(5, Some(logit_with(&[("beta_comfort", 1.0)])), 0);
    let error = run.try_execute(&mut Diagnostics::new()).unwrap_err().to_string();
    assert!(error.contains("comfort") && error.contains("time_min"), "{error}");
}

/// A model written outside the crate: everyone takes the slowest route.
struct Slowest;

impl openmobisim_core_choice::ChoiceModel for Slowest {
    fn name(&self) -> &str {
        "slowest"
    }
    fn descriptor(&self) -> String {
        "slowest".to_string()
    }
    fn is_sampled(&self) -> bool {
        false
    }
    fn required_attributes(&self) -> Option<Vec<String>> {
        Some(vec!["time_min".to_string()])
    }
    fn choose(
        &self,
        batch: &openmobisim_core_choice::ChoiceBatch,
        _rng: &openmobisim_core_types::rng::StreamRng,
    ) -> Result<openmobisim_core_choice::Choices, openmobisim_core_choice::ChoiceError> {
        let chosen: Vec<u32> = (0..batch.situations())
            .map(|s| u32::try_from(batch.range(s).len() - 1).expect("few"))
            .collect();
        let probability = vec![f64::NAN; chosen.len()];
        Ok(openmobisim_core_choice::Choices { chosen, probability })
    }
}

#[test]
fn a_model_from_outside_the_crate_drives_the_run() {
    let result = choosing_run(30, Some(Arc::new(Slowest)), 0).execute(&mut Diagnostics::new());
    assert_eq!((took(&result, 0), took(&result, 1)), (0, 30), "everyone on the bottom road");
    let choices = result.route_choices.expect("recorded");
    assert!(choices.probability.iter().all(|p| p.is_nan()), "no probability was given");
    assert_eq!(result.completion.completed, 30);
}
