//! A bus at its stops (S299): the feed's own dwell, and a stop snapped to the bus's side of a
//! dual carriageway. Every number by hand; residential streets, 30 km/h; the bus alone.
//!
//! Times are kept in whole seconds, rounded down; the lengths keep them off the boundaries.
//!
//! **Dwell.** A straight street `A → M → B`, 1 004 m (**120.48 s**) and 404 m (**48.48 s**).
//! The bus is scheduled at `A` 0, at `M` arriving 100 and leaving 130 (the feed's dwell: 30 s),
//! at `B` 300. It reaches `M` at **120**, stays the feed's 30 s and leaves at **150**, reaches
//! `B` at 198.96 → **198**. With `bus_dwell_s` 20 on top: leaves `M` at 170, `B` at 218. A
//! feed giving `M` one time (arrival = departure = 100): no dwell, `M` at 120, `B` at 168.
//!
//! **Snapping.** Two one-way carriageways 30 m apart, each two links of 504 m (**60.48 s**): the
//! south one eastbound `WS → MS → ES`, the north one westbound `EN → MN → WN`, joined at both
//! ends by two-way links of 30 m (3.6 s). An eastbound bus calls at `A` (at `WS`), `M` (20 m
//! north of `MS`, so 10 m from `MN`, the westbound carriageway's node) and `B` (at `ES`),
//! scheduled at 0, 70 and 140. Snapped to the nearest node, `M` would be `MN`: 185.0 s to it
//! and as long on, against 140 scheduled, so the pattern would be implausible (over 1.6 times
//! its schedule) and run by the schedule. Snapped to the node the bus can leave eastbound, `M`
//! is `MS`: at `M` at **60**, held to **70**, at `B` at 130.48 → **130**.

#![allow(clippy::float_cmp, reason = "whole seconds")]

use std::sync::Arc;

use openmobisim_core_demand::{ClassDefaults, Mode, Ownership, RawTrip, build_travellers};
use openmobisim_core_graph::LonLat;
use openmobisim_core_graph::defaults::{GlobalMultipliers, RoadClass, SignalDefaults};
use openmobisim_core_graph::layers::{BikeCost, StaticLayer, StaticLayerDefaults, StaticNetwork};
use openmobisim_core_graph::network::{LinkSpec, RoadNetwork, RoadNetworkBuilder};
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_loading::FidelityLevel;
use openmobisim_core_sim::{FlowMotor, LayerSetup, Run, StaticLayers, TransitSetup};
use openmobisim_core_transit::{
    ALIGHT, BOARD, CallSpec, RouteSpec, ServiceDate, StopSpec, TimetableBuilder, TransitDefaults,
};
use openmobisim_core_types::diagnostics::Diagnostics;
use openmobisim_core_types::time::Second;
use openmobisim_core_types::units::Duration;

/// Metres east and north of a point at 52.37° N, as a coordinate.
fn at(east_m: f64, north_m: f64) -> LonLat {
    let lat = 52.37_f64;
    LonLat::new(4.9 + east_m / (111_320.0 * lat.to_radians().cos()), lat + north_m / 110_574.0)
}

fn network(nodes: &[(&str, f64, f64)], links: &[(&str, &str, f64)]) -> Arc<RoadNetwork> {
    let mut b = RoadNetworkBuilder::new();
    for &(n, x, y) in nodes {
        b.add_node(n, at(x, y));
    }
    for &(from, to, m) in links {
        let mut spec = LinkSpec::new(RoadClass::Residential);
        spec.length_m = Some(m);
        b.add_link(format!("{from}{to}"), from, to, spec);
    }
    Arc::new(
        b.build(GlobalMultipliers::default(), SignalDefaults::SHIPPED, &mut Diagnostics::new())
            .unwrap(),
    )
}

/// One bus run calling at `stops` (position, arrival, departure), on `road`; the call times
/// realised (arrival, departure) and how many patterns rode the roads.
fn run_bus(
    road: &Arc<RoadNetwork>,
    stops: &[(LonLat, u32, u32)],
    defaults: TransitDefaults,
) -> (Vec<(u32, u32)>, u32) {
    let d = StaticLayerDefaults::SHIPPED;
    let walk = StaticNetwork::derive(road, StaticLayer::Walk, d, &mut Diagnostics::new()).unwrap();
    let layers = Arc::new(StaticLayers {
        bike: None,
        walk: Some(LayerSetup::new(Arc::new(walk), BikeCost::Dedicated, d)),
    });
    let mut tt = TimetableBuilder::new(ServiceDate::parse("20261009").unwrap());
    let ids: Vec<u32> = stops
        .iter()
        .enumerate()
        .map(|(i, &(position, _, _))| {
            tt.add_stop(StopSpec {
                external_id: format!("s{i}"),
                name: format!("s{i}"),
                position,
                parent: None,
            })
        })
        .collect();
    let line =
        tt.add_route(RouteSpec { external_id: "X".into(), short_name: "X".into(), route_type: 3 });
    let calls: Vec<CallSpec> = stops
        .iter()
        .zip(&ids)
        .map(|(&(_, arrival, departure), &stop)| CallSpec {
            stop,
            arrival,
            departure,
            flags: BOARD | ALIGHT,
        })
        .collect();
    tt.add_run("X:0", line, &calls);
    let transit = Arc::new(
        TransitSetup::new(Arc::new(tt.build().0), layers.walk.as_ref().unwrap(), None, defaults)
            .with_roads(road.clone()),
    );
    let on_roads = transit.bus_report().unwrap().groups_on_roads;
    let defaults = ClassDefaults::new()
        .with_default("everyone", Ownership { car: true, bike: false, transit_pass: false });
    // One car, long after the bus: a run has a trip.
    let car = RawTrip {
        traveller_id: "car".into(),
        trip_seq: 0,
        origin: stops[0].0,
        destination: stops[stops.len() - 1].0,
        departure_time: Second(3600),
        user_class: "everyone".into(),
        weight: None,
        mode: Some(Mode::Car),
    };
    let (travellers, trips) =
        build_travellers(vec![car], Vec::new(), &defaults, 1, &mut Diagnostics::new()).unwrap();
    let result = Run::new(road.clone(), Arc::new(travellers), Arc::new(trips), Second(86_400))
        .with_flow_motor(FlowMotor::Ltm {
            turns: Arc::new(TurnTable::build(road, SignalDefaults::SHIPPED)),
            step: Duration(300.0),
            level: FidelityLevel::PointQueue,
        })
        .with_layers(layers)
        .with_transit(transit)
        .execute(&mut Diagnostics::new());
    let times = &result.transit.as_ref().unwrap().times;
    ((0..stops.len()).map(|c| (times.arrival[c], times.departure[c])).collect(), on_roads)
}

fn street() -> Arc<RoadNetwork> {
    network(
        &[("A", 0.0, 0.0), ("M", 1000.0, 0.0), ("B", 1400.0, 0.0)],
        &[("A", "M", 1004.0), ("M", "B", 404.0), ("M", "A", 1004.0), ("B", "M", 404.0)],
    )
}

#[test]
fn a_bus_stays_the_feed_s_dwell_and_bus_dwell_s_on_top() {
    let road = street();
    let stops = |m_arrival, m_departure| {
        [
            (at(0.0, 0.0), 0, 0),
            (at(1000.0, 0.0), m_arrival, m_departure),
            (at(1400.0, 0.0), 300, 300),
        ]
    };
    let (times, on_roads) = run_bus(&road, &stops(100, 130), TransitDefaults::SHIPPED);
    assert_eq!(on_roads, 1);
    assert_eq!(times, vec![(0, 0), (120, 150), (198, 198)], "the feed's 30 s at M");
    let extra = TransitDefaults { bus_dwell_s: 20.0, ..TransitDefaults::SHIPPED };
    let (times, _) = run_bus(&road, &stops(100, 130), extra);
    assert_eq!(times, vec![(0, 0), (120, 170), (218, 218)], "and 20 s more");
    let (times, _) = run_bus(&road, &stops(100, 100), TransitDefaults::SHIPPED);
    assert_eq!(times, vec![(0, 0), (120, 120), (168, 168)], "one time at M: no dwell");
}

#[test]
fn a_stop_snaps_to_the_carriageway_its_bus_travels() {
    let road = network(
        &[
            ("WS", 0.0, 0.0),
            ("MS", 500.0, 0.0),
            ("ES", 1000.0, 0.0),
            ("WN", 0.0, 30.0),
            ("MN", 500.0, 30.0),
            ("EN", 1000.0, 30.0),
        ],
        &[
            ("WS", "MS", 504.0),
            ("MS", "ES", 504.0),
            ("EN", "MN", 504.0),
            ("MN", "WN", 504.0),
            ("ES", "EN", 30.0),
            ("EN", "ES", 30.0),
            ("WN", "WS", 30.0),
            ("WS", "WN", 30.0),
        ],
    );
    let stops = [(at(0.0, 0.0), 0, 0), (at(500.0, 20.0), 70, 70), (at(1000.0, 0.0), 140, 140)];
    let (times, on_roads) = run_bus(&road, &stops, TransitDefaults::SHIPPED);
    assert_eq!(on_roads, 1, "the pattern rides the roads: its middle stop is on its side");
    assert_eq!(times, vec![(0, 0), (60, 70), (130, 130)]);
}
