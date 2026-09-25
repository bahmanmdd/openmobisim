//! Bike and walk trips in a run (S195): routed on their own layer, traversed at
//! the layer's speeds, never part of the car's route sets, choice or gap, and
//! counted by mode.

#![allow(clippy::float_cmp, reason = "travel times here are exact sums of exact link times")]

use std::sync::Arc;

use openmobisim_core_demand::{
    ClassDefaults, Mode, Ownership, RawTrip, Travellers, Trips, build_travellers,
};
use openmobisim_core_graph::defaults::SignalDefaults;
use openmobisim_core_graph::layers::{BikeCost, StaticLayer, StaticLayerDefaults, StaticNetwork};
use openmobisim_core_graph::manhattan_grid;
use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_loading::FidelityLevel;
use openmobisim_core_sim::equilibration::strategy;
use openmobisim_core_sim::{EventType, FlowMotor, LayerSetup, Run, RunResult, StaticLayers};
use openmobisim_core_types::diagnostics::Diagnostics;
use openmobisim_core_types::time::Second;
use openmobisim_core_types::units::Duration;

/// A 5 × 5 two-way grid of 200 m blocks, unsignalised.
fn grid() -> Arc<RoadNetwork> {
    Arc::new(manhattan_grid(5, 200.0, false).0)
}

fn layers(road: &RoadNetwork) -> Arc<StaticLayers> {
    let d = StaticLayerDefaults::SHIPPED;
    let derive = |layer| {
        Arc::new(StaticNetwork::derive(road, layer, d, &mut Diagnostics::new()).expect("derives"))
    };
    Arc::new(StaticLayers {
        bike: Some(LayerSetup::new(derive(StaticLayer::Bike), BikeCost::Dedicated, d)),
        walk: Some(LayerSetup::new(derive(StaticLayer::Walk), BikeCost::Dedicated, d)),
    })
}

/// The range a trip of `blocks` grid blocks can take on `layer`, floored once
/// as a trip's arrival is (S88): the projected grid's blocks differ by up to a
/// metre, so the time lies between `blocks` of its shortest and of its longest.
fn seconds_range(road: &RoadNetwork, layer: StaticLayer, blocks: f64) -> (f64, f64) {
    let l = layers(road);
    let s = l.get(layer).expect("both layers").seconds();
    let (lo, hi) = s.iter().fold((f64::INFINITY, 0.0f64), |(a, b), &x| (a.min(x), b.max(x)));
    ((blocks * lo).floor(), (blocks * hi).floor())
}

/// A grid node's position, as `manhattan_grid` places it.
fn at(road: &RoadNetwork, row: u32, col: u32) -> (f64, f64) {
    let name = openmobisim_core_graph::examples::node_name(row, col);
    let id = road.node_external_ids().id_of(&name).expect("a grid node");
    let p = road.node_lonlat(openmobisim_core_types::ids::EntityId::new(id));
    (p.lon, p.lat)
}

fn trip(who: &str, seq: u32, from: (f64, f64), to: (f64, f64), t: u32, mode: Mode) -> RawTrip {
    RawTrip {
        traveller_id: who.to_string(),
        trip_seq: seq,
        origin: openmobisim_core_graph::LonLat::new(from.0, from.1),
        destination: openmobisim_core_graph::LonLat::new(to.0, to.1),
        departure_time: Second(t),
        user_class: "everyone".to_string(),
        weight: None,
        mode: Some(mode),
    }
}

fn owning(car: bool, bike: bool) -> ClassDefaults {
    ClassDefaults::new().with_default("everyone", Ownership { car, bike, transit_pass: false })
}

fn demand(rows: Vec<RawTrip>, defaults: &ClassDefaults) -> (Arc<Travellers>, Arc<Trips>) {
    let (t, trips) =
        build_travellers(rows, Vec::new(), defaults, 1, &mut Diagnostics::new()).expect("builds");
    (Arc::new(t), Arc::new(trips))
}

fn ltm(road: &RoadNetwork) -> FlowMotor {
    FlowMotor::Ltm {
        turns: Arc::new(TurnTable::build(road, SignalDefaults::SHIPPED)),
        step: Duration(300.0),
        level: FidelityLevel::Full,
    }
}

fn run(road: &Arc<RoadNetwork>, rows: Vec<RawTrip>, defaults: &ClassDefaults) -> RunResult {
    let (travellers, trips) = demand(rows, defaults);
    let mut run = Run::new(road.clone(), travellers, trips, Second(86_400))
        .with_flow_motor(ltm(road))
        .with_layers(layers(road))
        .with_link_bins(300);
    run.execute(&mut Diagnostics::new())
}

#[test]
fn a_bike_trip_takes_its_layer_s_time() {
    let road = grid();
    // Corner to corner: 8 blocks of 200 m, every link mixed traffic, 15 km/h.
    let result = run(
        &road,
        vec![trip("b", 0, at(&road, 0, 0), at(&road, 4, 4), 0, Mode::Bike)],
        &owning(false, true),
    );
    let bike = result.by_mode[Mode::Bike.index()];
    assert_eq!(bike.completion.completed, 1);
    // About 1600 m at 15 km/h: 384 s and a little.
    let (lo, hi) = seconds_range(&road, StaticLayer::Bike, 8.0);
    let t = bike.total_travel_time.get();
    assert!((384.0..=387.0).contains(&t) && (lo..=hi).contains(&t), "{t} not in {lo}..={hi}");
    assert_eq!(result.total_travel_time.get(), t, "the run total includes it");
    assert!(result.link_bins.is_none() || result.link_bins.as_ref().unwrap().is_empty());
    let bins = result.bike_link_bins.as_ref().expect("per-link results were asked for");
    assert_eq!(bins.crossings().iter().sum::<u32>(), 8, "one crossing per link of the route");
    assert!(result.walk_link_bins.is_none());
}

#[test]
fn a_walk_trip_needs_no_vehicle_and_walks_at_walking_speed() {
    let road = grid();
    let result = run(
        &road,
        vec![trip("w", 0, at(&road, 0, 0), at(&road, 0, 2), 0, Mode::Walk)],
        &owning(false, false),
    );
    let walk = result.by_mode[Mode::Walk.index()];
    assert_eq!(walk.completion.completed, 1);
    // About 400 m at 4.8 km/h: 300 s and a little.
    let (lo, hi) = seconds_range(&road, StaticLayer::Walk, 2.0);
    let t = walk.total_travel_time.get();
    assert!((300.0..=302.0).contains(&t) && (lo..=hi).contains(&t), "{t} not in {lo}..={hi}");
}

#[test]
fn bikes_leave_car_results_exactly_as_they_were() {
    let road = grid();
    let cars = |t| {
        vec![
            trip("c1", 0, at(&road, 0, 0), at(&road, 4, 4), t, Mode::Car),
            trip("c2", 0, at(&road, 4, 0), at(&road, 0, 4), t, Mode::Car),
            trip("c3", 0, at(&road, 2, 0), at(&road, 2, 4), t, Mode::Car),
        ]
    };
    let alone = run(&road, cars(0), &owning(true, true));
    let mut mixed_rows = cars(0);
    for i in 0..20 {
        mixed_rows.push(trip(&format!("b{i}"), 0, at(&road, 0, 0), at(&road, 4, 4), 0, Mode::Bike));
    }
    let mixed = run(&road, mixed_rows, &owning(true, true));
    let car = |r: &RunResult| r.by_mode[Mode::Car.index()];
    assert_eq!(car(&alone), car(&mixed), "bikes do not slow cars in the first version");
    assert_eq!(alone.link_bins, mixed.link_bins, "nor move a car link's results");
    assert_eq!(mixed.by_mode[Mode::Bike.index()].completion.completed, 20);
}

#[test]
fn a_car_only_run_is_the_same_run_with_or_without_layers() {
    let road = grid();
    let rows = || vec![trip("c", 0, at(&road, 0, 0), at(&road, 4, 4), 0, Mode::Car)];
    let (travellers, trips) = demand(rows(), &owning(true, false));
    let plain = Run::new(road.clone(), travellers.clone(), trips.clone(), Second(86_400));
    let with = Run::new(road.clone(), travellers, trips, Second(86_400)).with_layers(layers(&road));
    assert_eq!(plain.description().fingerprint, with.description().fingerprint);
}

#[test]
fn modes_enter_the_fingerprint() {
    let road = grid();
    let describe = |mode| {
        let (travellers, trips) = demand(
            vec![trip("x", 0, at(&road, 0, 0), at(&road, 4, 4), 0, mode)],
            &owning(true, true),
        );
        Run::new(road.clone(), travellers, trips, Second(86_400))
            .with_layers(layers(&road))
            .description()
            .fingerprint
    };
    let (car, bike, walk) = (describe(Mode::Car), describe(Mode::Bike), describe(Mode::Walk));
    assert!(car != bike && bike != walk && car != walk);
}

#[test]
fn a_bike_that_is_not_at_the_origin_cannot_be_ridden() {
    let road = grid();
    // Drive to work, then try to cycle home: the bike is still at home.
    let rows = vec![
        trip("p", 0, at(&road, 0, 0), at(&road, 4, 4), 0, Mode::Car),
        trip("p", 1, at(&road, 4, 4), at(&road, 0, 0), 3600, Mode::Bike),
    ];
    let result = run(&road, rows, &owning(true, true));
    assert_eq!(result.by_mode[Mode::Car.index()].completion.completed, 1);
    assert_eq!(result.by_mode[Mode::Bike.index()].completion.no_vehicle_available, 1);

    // Cycle to work and back: the bike goes with its rider.
    let rows = vec![
        trip("q", 0, at(&road, 0, 0), at(&road, 4, 4), 0, Mode::Bike),
        trip("q", 1, at(&road, 4, 4), at(&road, 0, 0), 3600, Mode::Bike),
    ];
    let result = run(&road, rows, &owning(false, true));
    assert_eq!(result.by_mode[Mode::Bike.index()].completion.completed, 2);
    // And someone without a bike cannot cycle at all.
    let rows = vec![trip("r", 0, at(&road, 0, 0), at(&road, 4, 4), 0, Mode::Bike)];
    let result = run(&road, rows, &owning(true, false));
    assert_eq!(result.by_mode[Mode::Bike.index()].completion.no_vehicle_available, 1);
}

#[test]
fn a_mode_without_its_layer_or_not_built_yet_is_reported_not_guessed() {
    let road = grid();
    let rows = vec![
        trip("t", 0, at(&road, 0, 0), at(&road, 4, 4), 0, Mode::Transit),
        trip("u", 0, at(&road, 0, 0), at(&road, 4, 4), 0, Mode::BikeTransit),
    ];
    let result = run(&road, rows, &owning(true, true));
    assert_eq!(result.completion.mode_not_available, 2);
    assert!(result.events.iter().all(|e| e.event_type == EventType::ModeNotAvailable));

    // A bike trip in a run given no bike layer.
    let (travellers, trips) = demand(
        vec![trip("b", 0, at(&road, 0, 0), at(&road, 4, 4), 0, Mode::Bike)],
        &owning(false, true),
    );
    let mut bare = Run::new(road.clone(), travellers, trips, Second(86_400));
    let result = bare.execute(&mut Diagnostics::new());
    assert_eq!(result.by_mode[Mode::Bike.index()].completion.mode_not_available, 1);
}

#[test]
fn by_mode_adds_up_to_the_run_s_totals() {
    let road = grid();
    let mut rows = Vec::new();
    for i in 0..6u32 {
        let mode = [Mode::Car, Mode::Bike, Mode::Walk][i as usize % 3];
        rows.push(trip(&format!("m{i}"), 0, at(&road, 0, i % 5), at(&road, 4, 2), 60, mode));
    }
    rows.push(trip("t", 0, at(&road, 0, 0), at(&road, 1, 1), 0, Mode::Transit));
    let result = run(&road, rows, &owning(true, true));
    let sum =
        |f: fn(&openmobisim_core_sim::ModeTotals) -> u32| result.by_mode.iter().map(f).sum::<u32>();
    assert_eq!(sum(|m| m.completion.total_trips), result.completion.total_trips);
    assert_eq!(sum(|m| m.completion.completed), result.completion.completed);
    assert_eq!(sum(|m| m.completion.mode_not_available), result.completion.mode_not_available);
    let time: f64 = result.by_mode.iter().map(|m| m.total_travel_time.get()).sum();
    assert!((time - result.total_travel_time.get()).abs() < 1e-6);
}

#[test]
fn an_iterating_run_routes_bikes_once_and_leaves_them_out_of_the_gap() {
    let road = grid();
    let mut rows = Vec::new();
    for i in 0..30 {
        let mode = if i % 2 == 0 { Mode::Car } else { Mode::Bike };
        rows.push(trip(&format!("i{i}"), 0, at(&road, 0, 0), at(&road, 4, 4), i * 10, mode));
    }
    let (travellers, trips) = demand(rows, &owning(true, true));
    let mut iterating = Run::new(road.clone(), travellers, trips, Second(86_400))
        .with_flow_motor(ltm(&road))
        .with_layers(layers(&road))
        .with_equilibration(Arc::from(
            strategy("msa", &[("iterations".to_string(), 4.0)].into_iter().collect())
                .expect("built in"),
        ));
    let result = iterating.execute(&mut Diagnostics::new());
    assert_eq!(result.iterations.len(), 4);
    let bike = result.by_mode[Mode::Bike.index()];
    assert_eq!(bike.completion.completed, 15);
    let one = run(
        &road,
        vec![trip("one", 0, at(&road, 0, 0), at(&road, 4, 4), 0, Mode::Bike)],
        &owning(false, true),
    );
    let each = one.by_mode[Mode::Bike.index()].total_travel_time.get();
    assert_eq!(bike.total_travel_time.get(), 15.0 * each, "every bike trip takes the same time");
    // The car's route choices cover only car trips.
    let choices = result.route_choices.expect("a run chooses");
    let chosen = choices.route.iter().filter(|&&r| r != openmobisim_core_sim::NO_ROUTE).count();
    assert_eq!(chosen, 15);
}
