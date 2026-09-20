//! `Run` with `FlowMotor::Ltm`: the same orchestration as `tests/run.rs`,
//! loaded by S84/S85's iterative LTM instead of S133/S134's level-0
//! placeholder.
//!
//! `tests/run.rs` is untouched and still exercises `FlowMotor::Level0` (the
//! default) exactly as it always has — this file is additive (G4/S52: an
//! opt-in feature costs nothing when off, and is proven separately when on).

use std::sync::Arc;

use openmobisim_core_demand::{ClassDefaults, Ownership, RawTrip, build_travellers};
use openmobisim_core_graph::defaults::{GlobalMultipliers, RoadClass, SignalDefaults};
use openmobisim_core_graph::geometry::LonLat;
use openmobisim_core_graph::network::{LinkSpec, RoadNetwork, RoadNetworkBuilder};
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_loading::FidelityLevel;
use openmobisim_core_sim::{FlowMotor, Run};
use openmobisim_core_types::diagnostics::Diagnostics;
use openmobisim_core_types::ids::{EntityId, LinkId};
use openmobisim_core_types::time::Second;
use openmobisim_core_types::units::Duration;

/// Three nodes in a line, `a -> b -> c`, each leg its own two-way street.
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

const A: (f64, f64) = (4.800, 45.700);
const C: (f64, f64) = (4.802, 45.700);

fn car_owning_defaults() -> ClassDefaults {
    ClassDefaults::new().with_default("commuter", Ownership { car: true, ..Ownership::NONE })
}

fn ltm_motor(network: &RoadNetwork) -> FlowMotor {
    let turns = Arc::new(TurnTable::build(network, SignalDefaults::SHIPPED));
    FlowMotor::Ltm { turns, step: Duration(300.0), level: FidelityLevel::Full }
}

fn expected_a_to_c_seconds(network: &RoadNetwork) -> u32 {
    let ab = network.link_external_ids().typed_id_of::<LinkId>("ab").expect("known link");
    let bc = network.link_external_ids().typed_id_of::<LinkId>("bc").expect("known link");
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "test assertion")]
    let seconds = network.free_flow_time(ab).get() as u32 + network.free_flow_time(bc).get() as u32;
    seconds
}

#[test]
fn an_uncongested_trip_completes_close_to_free_flow_time() {
    let network = Arc::new(line_network());
    let raw_trips = vec![trip("alice", 0, A, C, 0)];
    let mut build_diagnostics = Diagnostics::new();
    let (travellers, trips) =
        build_travellers(raw_trips, Vec::new(), &car_owning_defaults(), 1, &mut build_diagnostics)
            .expect("buildable");

    let mut run =
        Run::new(network.clone(), Arc::new(travellers), Arc::new(trips), Second(1_000_000))
            .with_flow_motor(ltm_motor(&network));
    let mut diagnostics = Diagnostics::new();
    let result = run.execute(&mut diagnostics);

    assert_eq!(result.completion.total_trips, 1);
    assert_eq!(result.completion.completed, 1);
    assert_eq!(result.completion.truncated, 0);

    let free_flow = f64::from(expected_a_to_c_seconds(&network));
    // Lone vehicle, no interaction: the LTM must not report *less* than
    // free-flow time, and the step-grid coarsening (S88) cannot add more
    // than one loading step of slack.
    assert!(result.total_travel_time.get() >= free_flow);
    assert!(result.total_travel_time.get() <= free_flow + 300.0);
}

#[test]
fn a_trip_past_the_window_is_truncated_not_completed() {
    let network = Arc::new(line_network());
    let raw_trips = vec![trip("carol", 0, A, C, 0)];
    let mut build_diagnostics = Diagnostics::new();
    let (travellers, trips) =
        build_travellers(raw_trips, Vec::new(), &car_owning_defaults(), 1, &mut build_diagnostics)
            .expect("buildable");

    let mut run = Run::new(network.clone(), Arc::new(travellers), Arc::new(trips), Second(1))
        .with_flow_motor(ltm_motor(&network));
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
fn a_second_trip_uses_the_vehicle_the_first_trip_relocated() {
    let network = Arc::new(line_network());
    let raw_trips = vec![trip("alice", 0, A, C, 0), trip("alice", 1, C, A, 10_000)];
    let mut build_diagnostics = Diagnostics::new();
    let (travellers, trips) =
        build_travellers(raw_trips, Vec::new(), &car_owning_defaults(), 1, &mut build_diagnostics)
            .expect("buildable");

    let mut run =
        Run::new(network.clone(), Arc::new(travellers), Arc::new(trips), Second(1_000_000))
            .with_flow_motor(ltm_motor(&network));
    let mut diagnostics = Diagnostics::new();
    let result = run.execute(&mut diagnostics);

    assert_eq!(result.completion.total_trips, 2);
    assert_eq!(result.completion.completed, 2);
    assert_eq!(result.completion.no_vehicle_available, 0);
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
        let mut run =
            Run::new(network.clone(), Arc::new(travellers), Arc::new(trips), Second(1_000_000))
                .with_flow_motor(ltm_motor(&network));
        let mut diagnostics = Diagnostics::new();
        run.execute(&mut diagnostics)
    };

    let a = build();
    let b = build();
    assert_eq!(a, b);
}

#[test]
fn heavier_demand_never_completes_faster_than_lighter_demand() {
    // Not an exact number (spillback's exact magnitude depends on the fixture
    // in ways not worth hand-deriving here) — but genuine interaction between
    // vehicles sharing a link must never make things *faster* on average.
    let network = Arc::new(line_network());
    let build = |n: u32| {
        let raw_trips: Vec<RawTrip> = (0..n).map(|i| trip(&format!("t{i}"), 0, A, C, i)).collect();
        let mut build_diagnostics = Diagnostics::new();
        let (travellers, trips) = build_travellers(
            raw_trips,
            Vec::new(),
            &car_owning_defaults(),
            1,
            &mut build_diagnostics,
        )
        .expect("buildable");
        let mut run =
            Run::new(network.clone(), Arc::new(travellers), Arc::new(trips), Second(3600))
                .with_flow_motor(ltm_motor(&network));
        let mut diagnostics = Diagnostics::new();
        run.execute(&mut diagnostics)
    };

    let light = build(2);
    let heavy = build(200);

    assert!(light.completion.completed > 0, "the light scenario must produce a usable average");
    let light_avg = light.total_travel_time.get() / f64::from(light.completion.completed);
    let heavy_completed = heavy.completion.completed.max(1);
    let heavy_avg = heavy.total_travel_time.get() / f64::from(heavy_completed);

    assert!(
        heavy_avg >= light_avg,
        "average travel time under heavy demand ({heavy_avg}) must not be lower than under \
         light demand ({light_avg}) — real vehicles sharing real link capacity"
    );
}

// --- Per-link, per-time-bin results (S163) ------------------------------------------

/// Two cars A -> C: every link on the way has two crossings in the table, at
/// the free-flow time, under either loading engine.
#[test]
fn a_run_can_report_per_link_bins_under_either_engine() {
    for ltm in [false, true] {
        let network = Arc::new(line_network());
        let raw_trips = vec![trip("alice", 0, A, C, 0), trip("bob", 0, A, C, 600)];
        let mut d = Diagnostics::new();
        let (travellers, trips) =
            build_travellers(raw_trips, Vec::new(), &car_owning_defaults(), 1, &mut d)
                .expect("buildable");
        let mut run =
            Run::new(network.clone(), Arc::new(travellers), Arc::new(trips), Second(3600))
                .with_link_bins(300);
        if ltm {
            run = run.with_flow_motor(ltm_motor(&network));
        }
        let result = run.execute(&mut Diagnostics::new());
        let bins = result.link_bins.expect("asked for");
        let id = |e: &str| network.link_external_ids().typed_id_of::<LinkId>(e).expect("link");
        for name in ["ab", "bc"] {
            let link = id(name);
            let rows: Vec<usize> =
                (0..bins.len()).filter(|&r| bins.links()[r] == link.raw()).collect();
            assert_eq!(rows.iter().map(|&r| bins.crossings()[r]).sum::<u32>(), 2, "{name}");
            for r in rows {
                assert!(
                    (bins.mean_seconds(r) - network.free_flow_time(link).get()).abs() < 1e-6,
                    "{name}: mean {} s, free flow {} s (ltm {ltm})",
                    bins.mean_seconds(r),
                    network.free_flow_time(link).get()
                );
            }
        }
        assert!(bins.links().iter().all(|&l| l == id("ab").raw() || l == id("bc").raw()));
    }
}

/// Without `with_link_bins` nothing is recorded, and the result is unchanged.
#[test]
fn per_link_bins_are_off_unless_asked_for() {
    let network = Arc::new(line_network());
    let raw_trips = vec![trip("alice", 0, A, C, 0)];
    let mut d = Diagnostics::new();
    let (travellers, trips) =
        build_travellers(raw_trips, Vec::new(), &car_owning_defaults(), 1, &mut d)
            .expect("buildable");
    let (travellers, trips) = (Arc::new(travellers), Arc::new(trips));
    let plain = Run::new(network.clone(), travellers.clone(), trips.clone(), Second(3600))
        .execute(&mut Diagnostics::new());
    let binned = Run::new(network, travellers, trips, Second(3600))
        .with_link_bins(60)
        .execute(&mut Diagnostics::new());
    assert!(plain.link_bins.is_none());
    assert_eq!(
        (plain.total_travel_time, plain.completion, &plain.events),
        (binned.total_travel_time, binned.completion, &binned.events)
    );
}
