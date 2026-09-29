//! The toy bus `B1` on the roads (S199): every number derived by hand.
//!
//! `B1` calls at `W`, `M` and `D1` on the one-way arterial, scheduled at `+0`,
//! `+120`, `+240` s, every 600 s from 0. A bus is 2 PCU and dwells 20 s at a
//! stop; a leg leaves its stop no earlier than its scheduled departure. Roads:
//! residential 30 km/h, capacity 1400/h; `a1` approaches the signal at `S`
//! (discharge `q · g/C` = 0.18667/s: 5.357 s per PCU; Webster's delay
//! 20.554 s). Free flow `W → M` (a1, the signal, a2): 36 + 20.554 + 24 =
//! 80.554 s; `M → D1` (a3, three 4-m pieces, a4): 48 + 1.44 + 36 = 85.44 s.
//!
//! | Case | Hand value |
//! |---|---|
//! | B1: alone on the roads | `M` at 80.554 → **80**; held to **120**; `D1` at 205.44 → **205** (35 s early). A passenger at `M` from 0 (boards with the 60 s slack) is at `D1` at **205** |
//! | B2: behind ten cars `W → M`, all leaving at 0 | the cars and the bus enter `a1` together (an origin takes no inflow capacity), reach the stop line at 36; the bus passes it after car 9, at 36 + 9 · 5.357 + 2 · 5.357 = 94.93, waits the signal's 20.554, enters `a2` at 115.48, `M` at 139.48 → **139**; leaves at 139.48 + 20 = **159.48**; `D1` at 159.48 + 85.44 = 244.92 → **244** (4 s late). The passenger at `M` reaches `D1` at **244**, on the realised times, not the scheduled 240 |
//! | B3: B1 at free flow (level 0) | the same as B1: **80, 120, 205** |
//! | B4: the plausibility ratio at 0.5 | 80.554 + 20 + 85.44 = 185.99 s > 0.5 · 240: run by the schedule; the passenger arrives at **240** |
//! | B5: a timetable not put on the roads | by the schedule: **240** |

#![allow(clippy::float_cmp, reason = "counts and whole seconds")]

use std::sync::Arc;

use openmobisim_core_demand::{ClassDefaults, Mode, Ownership, RawTrip, build_travellers};
use openmobisim_core_graph::defaults::SignalDefaults;
use openmobisim_core_graph::layers::{BikeCost, StaticLayerDefaults};
use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_graph::{LonLat, toy_network, toy_network_layers};
use openmobisim_core_loading::FidelityLevel;
use openmobisim_core_sim::{
    EventType, FlowMotor, LayerSetup, Run, RunResult, StaticLayers, TransitSetup,
};
use openmobisim_core_transit::TransitDefaults;
use openmobisim_core_transit::examples::toy_bus;
use openmobisim_core_types::diagnostics::Diagnostics;
use openmobisim_core_types::ids::{EntityId, NodeId, TransitRunId};
use openmobisim_core_types::time::Second;
use openmobisim_core_types::units::Duration;

struct Toy {
    road: Arc<RoadNetwork>,
    layers: Arc<StaticLayers>,
}

fn toy() -> Toy {
    let (road, _) = toy_network();
    let (bike, walk) = toy_network_layers();
    let d = StaticLayerDefaults::SHIPPED;
    let layers = Arc::new(StaticLayers {
        bike: Some(LayerSetup::new(Arc::new(bike), BikeCost::Dedicated, d)),
        walk: Some(LayerSetup::new(Arc::new(walk), BikeCost::Dedicated, d)),
    });
    Toy { road: Arc::new(road), layers }
}

impl Toy {
    fn transit(&self, defaults: TransitDefaults, on_roads: bool) -> Arc<TransitSetup> {
        let setup = TransitSetup::new(
            Arc::new(toy_bus()),
            self.layers.walk.as_ref().unwrap(),
            self.layers.bike.as_ref(),
            defaults,
        );
        Arc::new(if on_roads { setup.with_roads(self.road.clone()) } else { setup })
    }

    fn at(&self, name: &str) -> LonLat {
        let node: NodeId = self.road.node_external_ids().typed_id_of(name).expect("a toy node");
        self.road.node_lonlat(node)
    }

    fn trip(&self, who: &str, from: &str, to: &str, mode: Mode) -> RawTrip {
        RawTrip {
            traveller_id: who.to_string(),
            trip_seq: 0,
            origin: self.at(from),
            destination: self.at(to),
            departure_time: Second(0),
            user_class: "everyone".to_string(),
            weight: None,
            mode: Some(mode),
        }
    }

    fn run(&self, rows: Vec<RawTrip>, transit: Arc<TransitSetup>, ltm: bool) -> RunResult {
        let defaults = ClassDefaults::new()
            .with_default("everyone", Ownership { car: true, bike: false, transit_pass: false });
        let (travellers, trips) =
            build_travellers(rows, Vec::new(), &defaults, 1, &mut Diagnostics::new()).unwrap();
        let mut run =
            Run::new(self.road.clone(), Arc::new(travellers), Arc::new(trips), Second(86_400))
                .with_layers(self.layers.clone())
                .with_transit(transit)
                .with_link_bins(300);
        if ltm {
            run = run.with_flow_motor(FlowMotor::Ltm {
                turns: Arc::new(TurnTable::build(&self.road, SignalDefaults::SHIPPED)),
                step: Duration(300.0),
                level: FidelityLevel::Full,
            });
        }
        run.execute(&mut Diagnostics::new())
    }
}

/// The realised arrival and departure at `W`, `M`, `D1` of run `B1:{k}`.
fn bus_times(result: &RunResult, transit: &TransitSetup, k: u32) -> [(u32, u32); 3] {
    let t = transit.timetable();
    let run = TransitRunId::new(t.run_ids().id_of(&format!("B1:{k:02}")).unwrap());
    let times = &result.transit.as_ref().unwrap().times;
    let calls: Vec<usize> = t.run_calls(run).collect();
    [0, 1, 2].map(|i| (times.arrival[calls[i]], times.departure[calls[i]]))
}

/// The passenger's arrival (the last trip, `p`).
fn passenger(result: &RunResult) -> (EventType, u32) {
    let e = result.events.iter().max_by_key(|e| e.entity_id).unwrap();
    (e.event_type, e.second.get())
}

#[test]
fn b1_alone_on_the_roads_the_bus_holds_for_its_schedule() {
    let t = toy();
    let transit = t.transit(TransitDefaults::SHIPPED, true);
    let report = transit.bus_report().unwrap();
    assert_eq!((report.groups_on_roads, report.runs_on_roads), (1, 18));
    let result = t.run(vec![t.trip("p", "M", "D1", Mode::Transit)], transit.clone(), true);
    assert_eq!(bus_times(&result, &transit, 0), [(0, 0), (80, 120), (205, 205)]);
    assert_eq!(bus_times(&result, &transit, 1), [(600, 600), (680, 720), (805, 805)]);
    assert_eq!(passenger(&result), (EventType::TripCompleted, 205));
    // The bus is on the road results: one crossing of each of its seven links per run.
    let bins = result.link_bins.as_ref().unwrap();
    assert_eq!(bins.crossings().iter().sum::<u32>(), 7 * 18);
}

#[test]
fn b2_behind_ten_cars_the_bus_is_late_and_so_is_its_passenger() {
    let t = toy();
    let transit = t.transit(TransitDefaults::SHIPPED, true);
    let mut rows: Vec<RawTrip> =
        (0..10).map(|i| t.trip(&format!("c{i}"), "W", "M", Mode::Car)).collect();
    rows.push(t.trip("p", "M", "D1", Mode::Transit));
    let result = t.run(rows, transit.clone(), true);
    assert_eq!(bus_times(&result, &transit, 0), [(0, 0), (139, 159), (244, 244)]);
    assert_eq!(passenger(&result), (EventType::TripCompleted, 244));
    assert_eq!(result.by_mode[Mode::Car.index()].completion.completed, 10);
    // Run 0 is 4 s late at D1, the other seventeen 35 s early (as B1).
    let buses = result.transit.as_ref().unwrap().buses.unwrap();
    assert_eq!((buses.runs_on_roads, buses.runs_arrived), (18, 18));
    assert!((buses.delay_mean_s - (4.0 - 17.0 * 35.0) / 18.0).abs() < 1e-9, "{buses:?}");
}

#[test]
fn b3_at_free_flow_the_same_as_alone_on_the_roads() {
    let t = toy();
    let transit = t.transit(TransitDefaults::SHIPPED, true);
    let result = t.run(vec![t.trip("p", "M", "D1", Mode::Transit)], transit.clone(), false);
    assert_eq!(bus_times(&result, &transit, 0), [(0, 0), (80, 120), (205, 205)]);
    assert_eq!(passenger(&result), (EventType::TripCompleted, 205));
}

#[test]
fn b4_an_implausible_road_route_runs_by_the_schedule() {
    let t = toy();
    let defaults = TransitDefaults { bus_plausibility_ratio: 0.5, ..TransitDefaults::SHIPPED };
    let transit = t.transit(defaults, true);
    let report = transit.bus_report().unwrap();
    assert_eq!((report.by_schedule_implausible, report.runs_on_roads), (1, 0));
    let result = t.run(vec![t.trip("p", "M", "D1", Mode::Transit)], transit.clone(), true);
    assert_eq!(bus_times(&result, &transit, 0), [(0, 0), (120, 120), (240, 240)]);
    assert_eq!(passenger(&result), (EventType::TripCompleted, 240));
    assert!(result.link_bins.as_ref().unwrap().is_empty(), "no bus on the roads");
}

#[test]
fn b5_a_timetable_not_put_on_the_roads_runs_by_the_schedule() {
    let t = toy();
    let transit = t.transit(TransitDefaults::SHIPPED, false);
    assert!(transit.bus_report().is_none());
    let result = t.run(vec![t.trip("p", "M", "D1", Mode::Transit)], transit.clone(), true);
    assert_eq!(passenger(&result), (EventType::TripCompleted, 240));
}
