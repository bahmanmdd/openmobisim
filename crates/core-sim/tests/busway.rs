//! Busways (S199): a bus takes the busway, a car cannot.
//!
//! `A → B` directly is a 300 m busway (50 km/h: **21.6 s**); round by `C` it is two
//! residential streets of 400 m (30 km/h: **96 s**; capacity 1400/h, so a 2-PCU bus
//! passes a link end 5.143 s after the car before it). A bus scheduled from stop `A`
//! at 0 to stop `B` at 120 arrives at **21**. A car leaving `A` at 0 goes round: **96 s**
//! either way. Without the busway the bus goes round too, behind the car: out of `AC`
//! at 48 + 5.143, at `B` at 101.14 → **101**.

#![allow(clippy::float_cmp, reason = "whole seconds")]

use std::sync::Arc;

use openmobisim_core_demand::{ClassDefaults, Mode, Ownership, RawTrip, build_travellers};
use openmobisim_core_graph::LonLat;
use openmobisim_core_graph::defaults::{GlobalMultipliers, RoadClass, SignalDefaults};
use openmobisim_core_graph::layers::{BikeCost, StaticLayer, StaticLayerDefaults, StaticNetwork};
use openmobisim_core_graph::network::{LinkSpec, RoadNetwork, RoadNetworkBuilder};
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_loading::FidelityLevel;
use openmobisim_core_sim::{FlowMotor, LayerSetup, Run, RunResult, StaticLayers, TransitSetup};
use openmobisim_core_transit::{
    ALIGHT, BOARD, CallSpec, RouteSpec, ServiceDate, StopSpec, TimetableBuilder, TransitDefaults,
};
use openmobisim_core_types::diagnostics::Diagnostics;
use openmobisim_core_types::time::Second;
use openmobisim_core_types::units::Duration;

fn at(name: &str) -> LonLat {
    match name {
        "A" => LonLat::new(4.900, 52.370),
        "B" => LonLat::new(4.904, 52.370),
        _ => LonLat::new(4.902, 52.373),
    }
}

fn network(busway: bool) -> Arc<RoadNetwork> {
    let mut b = RoadNetworkBuilder::new();
    for n in ["A", "B", "C"] {
        b.add_node(n, at(n));
    }
    let mut add = |id: &str, from: &str, to: &str, class: RoadClass, m: f64| {
        let mut spec = LinkSpec::new(class);
        spec.length_m = Some(m);
        b.add_link(id, from, to, spec);
    };
    add("AC", "A", "C", RoadClass::Residential, 400.0);
    add("CB", "C", "B", RoadClass::Residential, 400.0);
    add("CA", "C", "A", RoadClass::Residential, 400.0);
    add("BC", "B", "C", RoadClass::Residential, 400.0);
    if busway {
        add("AB", "A", "B", RoadClass::Busway, 300.0);
    }
    Arc::new(
        b.build(GlobalMultipliers::default(), SignalDefaults::SHIPPED, &mut Diagnostics::new())
            .unwrap(),
    )
}

fn run(busway: bool) -> (RunResult, Arc<TransitSetup>) {
    let road = network(busway);
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
    tt.add_run("X:0", line, &[call(a, 0), call(bstop, 120)]);
    let transit = Arc::new(
        TransitSetup::new(
            Arc::new(tt.build().0),
            layers.walk.as_ref().unwrap(),
            None,
            TransitDefaults::SHIPPED,
        )
        .with_roads(road.clone()),
    );
    let car = RawTrip {
        traveller_id: "car".into(),
        trip_seq: 0,
        origin: at("A"),
        destination: at("B"),
        departure_time: Second(0),
        user_class: "everyone".into(),
        weight: None,
        mode: Some(Mode::Car),
    };
    let defaults = ClassDefaults::new()
        .with_default("everyone", Ownership { car: true, bike: false, transit_pass: false });
    let (travellers, trips) =
        build_travellers(vec![car], Vec::new(), &defaults, 1, &mut Diagnostics::new()).unwrap();
    let result = Run::new(road.clone(), Arc::new(travellers), Arc::new(trips), Second(86_400))
        .with_flow_motor(FlowMotor::Ltm {
            turns: Arc::new(TurnTable::build(&road, SignalDefaults::SHIPPED)),
            step: Duration(300.0),
            level: FidelityLevel::Full,
        })
        .with_layers(layers)
        .with_transit(transit.clone())
        .execute(&mut Diagnostics::new());
    (result, transit)
}

fn bus_arrival(result: &RunResult) -> u32 {
    result.transit.as_ref().unwrap().times.arrival[1]
}

#[test]
fn a_bus_takes_the_busway_and_a_car_goes_round() {
    let (result, transit) = run(true);
    assert_eq!(transit.bus_report().unwrap().groups_on_roads, 1);
    assert_eq!(bus_arrival(&result), 21, "300 m at 50 km/h: 21.6 s");
    assert_eq!(result.by_mode[Mode::Car.index()].total_travel_time.get(), 96.0);
}

#[test]
fn without_the_busway_the_bus_goes_round_too_behind_the_car() {
    let (result, _) = run(false);
    assert_eq!(bus_arrival(&result), 101);
    assert_eq!(result.by_mode[Mode::Car.index()].total_travel_time.get(), 96.0);
}
