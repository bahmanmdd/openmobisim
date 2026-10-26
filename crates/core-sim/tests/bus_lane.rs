//! Bus lanes (S299): a bus on a link with a lane of its own rides at free flow, outside the
//! cars' queue.
//!
//! `A → M` is 1 000 m and `M → B` 400 m of residential street (30 km/h: **120 s** and
//! **48 s**; one lane, 1 400 cars an hour, a car every 2.571 s at a link's downstream end).
//! Fifty cars leave `A` at 0 for `B`; at the point queue they are all at `M` at 120 and leave
//! it one every 2.571 s, the last at about 246. A bus scheduled from stop `A` at 10 to stop
//! `B` at 400:
//!
//! - **no bus lane**: it reaches `M` at 130, behind the fifty cars, leaves it after them
//!   (about 249), and is at `B` 48 s later: **after 290**;
//! - **a bus lane on both links**: free flow, **10 + 168 = 178**;
//! - **a bus lane on `A → M` alone**: at `M` at 130, where it joins the queue of `M → B`, which
//!   carries the cars at the rate they arrive: at `B` at 178 or within a car's headway after;
//! - the cars' times are the same with and without the lanes: the lane is not theirs (one lane
//!   each way stays with the cars, the class's default less the bus lane, never below one).

#![allow(clippy::float_cmp, reason = "whole seconds")]

use std::sync::Arc;

use openmobisim_core_demand::{ClassDefaults, Mode, Ownership, RawTrip, build_travellers};
use openmobisim_core_graph::LonLat;
use openmobisim_core_graph::defaults::{GlobalMultipliers, RoadClass, SignalDefaults};
use openmobisim_core_graph::layers::{BikeCost, StaticLayer, StaticLayerDefaults, StaticNetwork};
use openmobisim_core_graph::network::{LinkSpec, RoadNetwork, RoadNetworkBuilder};
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_loading::FidelityLevel;
use openmobisim_core_sim::{
    FlowMotor, LayerSetup, ModeDefaults, Run, RunResult, StaticLayers, TransitSetup,
};
use openmobisim_core_transit::{
    ALIGHT, BOARD, CallSpec, RouteSpec, ServiceDate, StopSpec, TimetableBuilder, TransitDefaults,
};
use openmobisim_core_types::diagnostics::Diagnostics;
use openmobisim_core_types::time::Second;
use openmobisim_core_types::units::Duration;

const CARS: usize = 50;

fn at(name: &str) -> LonLat {
    match name {
        "A" => LonLat::new(4.900, 52.370),
        "M" => LonLat::new(4.914_7, 52.370),
        _ => LonLat::new(4.920_6, 52.370),
    }
}

/// The street, with a bus lane on `A → M` and on `M → B` as asked.
fn network(lane_am: bool, lane_mb: bool) -> Arc<RoadNetwork> {
    let mut b = RoadNetworkBuilder::new();
    for n in ["A", "M", "B"] {
        b.add_node(n, at(n));
    }
    let mut add = |id: &str, from: &str, to: &str, m: f64, lane: bool| {
        let mut spec = LinkSpec::new(RoadClass::Residential);
        spec.length_m = Some(m);
        spec.bus_lanes = u8::from(lane);
        b.add_link(id, from, to, spec);
    };
    add("AM", "A", "M", 1000.0, lane_am);
    add("MB", "M", "B", 400.0, lane_mb);
    add("MA", "M", "A", 1000.0, false);
    add("BM", "B", "M", 400.0, false);
    Arc::new(
        b.build(GlobalMultipliers::default(), SignalDefaults::SHIPPED, &mut Diagnostics::new())
            .unwrap(),
    )
}

fn run(lane_am: bool, lane_mb: bool, level: FidelityLevel) -> RunResult {
    let road = network(lane_am, lane_mb);
    let d = StaticLayerDefaults::SHIPPED;
    let walk = StaticNetwork::derive(&road, StaticLayer::Walk, d, &mut Diagnostics::new()).unwrap();
    let layers = Arc::new(StaticLayers {
        bike: None,
        walk: Some(LayerSetup::new(Arc::new(walk), BikeCost::Dedicated, d)),
    });
    let mut tt = TimetableBuilder::new(ServiceDate::parse("20261009").unwrap());
    let stop = |name: &str| StopSpec {
        external_id: name.into(),
        name: name.into(),
        position: at(name),
        parent: None,
    };
    let (a, bstop) = (tt.add_stop(stop("A")), tt.add_stop(stop("B")));
    let line =
        tt.add_route(RouteSpec { external_id: "X".into(), short_name: "X".into(), route_type: 3 });
    let call =
        |s: u32, t: u32| CallSpec { stop: s, arrival: t, departure: t, flags: BOARD | ALIGHT };
    tt.add_run("X:0", line, &[call(a, 10), call(bstop, 400)]);
    let transit = Arc::new(
        TransitSetup::new(
            Arc::new(tt.build().0),
            layers.walk.as_ref().unwrap(),
            None,
            TransitDefaults::SHIPPED,
        )
        .with_roads(road.clone()),
    );
    assert_eq!(transit.bus_report().unwrap().groups_on_roads, 1);
    let cars = (0..CARS)
        .map(|i| RawTrip {
            traveller_id: format!("car{i}"),
            trip_seq: 0,
            origin: at("A"),
            destination: at("B"),
            departure_time: Second(0),
            user_class: "everyone".into(),
            weight: None,
            mode: Some(Mode::Car),
        })
        .collect();
    let defaults = ClassDefaults::new()
        .with_default("everyone", Ownership { car: true, bike: false, transit_pass: false });
    let (travellers, trips) =
        build_travellers(cars, Vec::new(), &defaults, 1, &mut Diagnostics::new()).unwrap();
    Run::new(road.clone(), Arc::new(travellers), Arc::new(trips), Second(86_400))
        .with_flow_motor(FlowMotor::Ltm {
            turns: Arc::new(TurnTable::build(&road, SignalDefaults::SHIPPED)),
            step: Duration(300.0),
            level,
        })
        .with_layers(layers)
        .with_transit(transit)
        .with_mode_defaults(ModeDefaults {
            car_access_s: 0.0,
            car_egress_s: 0.0,
            bike_access_s: 0.0,
            bike_egress_s: 0.0,
            ..ModeDefaults::SHIPPED
        })
        .execute(&mut Diagnostics::new())
}

fn bus_times(result: &RunResult) -> (u32, u32) {
    let times = &result.transit.as_ref().unwrap().times;
    (times.departure[0], times.arrival[1])
}

fn car_time(result: &RunResult) -> f64 {
    result.by_mode[Mode::Car.index()].total_travel_time.get()
}

#[test]
fn a_bus_without_a_bus_lane_queues_behind_the_cars() {
    let result = run(false, false, FidelityLevel::PointQueue);
    let (departure, arrival) = bus_times(&result);
    assert_eq!(departure, 10);
    assert!(arrival > 290, "behind fifty cars: {arrival}");
}

#[test]
fn a_bus_on_bus_lanes_rides_at_free_flow_and_the_cars_keep_their_times() {
    let without = run(false, false, FidelityLevel::PointQueue);
    for level in [FidelityLevel::PointQueue, FidelityLevel::SpatialQueue, FidelityLevel::Full] {
        let result = run(true, true, level);
        assert_eq!(bus_times(&result), (10, 178), "{level:?}: 10 + 120 + 48");
    }
    let with = run(true, true, FidelityLevel::PointQueue);
    assert_eq!(car_time(&with), car_time(&without), "the bus lane takes nothing from the cars");
}

#[test]
fn a_bus_lane_on_part_of_a_hop_skips_that_part_s_queue() {
    let result = run(true, false, FidelityLevel::PointQueue);
    let (departure, arrival) = bus_times(&result);
    assert_eq!(departure, 10, "the hop leaves its stop, then rides the lane");
    assert!((178..=181).contains(&arrival), "free flow on A → M, then M → B's flow: {arrival}");
    // A bus lane on the second link alone: the bus waits behind the cars on the first.
    let result = run(false, true, FidelityLevel::PointQueue);
    let (departure, arrival) = bus_times(&result);
    assert_eq!(departure, 10);
    assert!(arrival > 290, "the queue is on A → M: {arrival}");
}
