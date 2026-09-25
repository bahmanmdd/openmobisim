//! The toy network's bike and walk layers (S195): every number derived by hand.
//!
//! Speeds are the shipped defaults: 15 km/h in mixed traffic, 18 km/h on the
//! cycle track, 4.8 km/h on foot; a trip leaves at 0 s and its time is its
//! exact time floored once (S88).
//!
//! | Case | Route | Hand value |
//! |---|---|---|
//! | L1 bike `M → X0`, cost `time` | `a3`, 400 m mixed | 400 / (15/3.6) = **96 s** (the track: 520 / 5 = 104 s) |
//! | L2 bike `M → X0`, cost `dedicated` | the track `t1`,`t2`, 520 m | **104 s** (`a3` costs 96 × 1.2 = 115.2 s) |
//! | L3 bike `S → N1` | contraflow `s1:c`, 300 m | 300 / (15/3.6) = **72 s**; a car has no route |
//! | L4 walk `N1 → N2` | `w2`, 200 m | 200 / (4.8/3.6) = **150 s** (by the streets 800 m) |
//! | L5 walk `W → N1` | `w1`, 300√2 m | 424.26 / 1.3333 = 318.2 → **318 s** |
//! | L6 walk `M → S` | `a2` walked against the traffic, 200 m | **150 s** |

#![allow(clippy::float_cmp, reason = "hand-derived values, floored to whole seconds")]

use std::sync::Arc;

use openmobisim_core_demand::{ClassDefaults, Mode, Ownership, RawTrip, build_travellers};
use openmobisim_core_graph::layers::{BikeCost, StaticLayerDefaults};
use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_graph::{toy_network, toy_network_layers};
use openmobisim_core_sim::{LayerSetup, Run, RunResult, StaticLayers};
use openmobisim_core_types::diagnostics::Diagnostics;
use openmobisim_core_types::time::Second;

fn place(road: &RoadNetwork, name: &str) -> openmobisim_core_graph::LonLat {
    let id = road.node_external_ids().typed_id_of(name).expect("a toy node");
    road.node_lonlat(id)
}

fn one_trip(from: &str, to: &str, mode: Mode, cost: BikeCost) -> RunResult {
    let (road, _) = toy_network();
    let road = Arc::new(road);
    let (bike, walk) = toy_network_layers();
    let d = StaticLayerDefaults::SHIPPED;
    let layers = StaticLayers {
        bike: Some(LayerSetup::new(Arc::new(bike), cost, d)),
        walk: Some(LayerSetup::new(Arc::new(walk), cost, d)),
    };
    let raw = vec![RawTrip {
        traveller_id: "t".to_string(),
        trip_seq: 0,
        origin: place(&road, from),
        destination: place(&road, to),
        departure_time: Second(0),
        user_class: "all".to_string(),
        weight: None,
        mode: Some(mode),
    }];
    let defaults = ClassDefaults::new()
        .with_default("all", Ownership { car: true, bike: true, transit_pass: false });
    let (travellers, trips) =
        build_travellers(raw, Vec::new(), &defaults, 1, &mut Diagnostics::new()).expect("builds");
    let mut run = Run::new(road, Arc::new(travellers), Arc::new(trips), Second(86_400))
        .with_layers(Arc::new(layers));
    run.execute(&mut Diagnostics::new())
}

fn seconds(result: &RunResult, mode: Mode) -> f64 {
    let m = result.by_mode[mode.index()];
    assert_eq!(m.completion.completed, 1, "{m:?}");
    m.total_travel_time.get()
}

#[test]
fn l1_by_time_the_street_is_faster_than_the_track() {
    let r = one_trip("M", "X0", Mode::Bike, BikeCost::Time);
    assert_eq!(seconds(&r, Mode::Bike), 96.0);
}

#[test]
fn l2_under_the_dedicated_cost_the_track_wins() {
    let r = one_trip("M", "X0", Mode::Bike, BikeCost::Dedicated);
    assert_eq!(seconds(&r, Mode::Bike), 104.0);
}

#[test]
fn l3_a_bike_rides_against_the_one_way_where_no_car_can() {
    let bike = one_trip("S", "N1", Mode::Bike, BikeCost::Dedicated);
    assert_eq!(seconds(&bike, Mode::Bike), 72.0);
    let car = one_trip("S", "N1", Mode::Car, BikeCost::Dedicated);
    assert_eq!(car.completion.no_feasible_path, 1);
}

#[test]
fn l4_l5_walkers_take_the_walk_only_paths() {
    assert_eq!(seconds(&one_trip("N1", "N2", Mode::Walk, BikeCost::Dedicated), Mode::Walk), 150.0);
    assert_eq!(seconds(&one_trip("W", "N1", Mode::Walk, BikeCost::Dedicated), Mode::Walk), 318.0);
}

#[test]
fn l6_walkers_go_against_one_way_streets() {
    assert_eq!(seconds(&one_trip("M", "S", Mode::Walk, BikeCost::Dedicated), Mode::Walk), 150.0);
}

#[test]
fn the_walk_cost_ignores_the_bike_cost() {
    for cost in [BikeCost::Time, BikeCost::Dedicated] {
        assert_eq!(seconds(&one_trip("N1", "N2", Mode::Walk, cost), Mode::Walk), 150.0);
    }
}
