//! Transit trips on the toy network's tram `T1` (S199): every number derived
//! by hand.
//!
//! The tram shuttles between stop `N1` (at the node `N1`) and stop `H` (at
//! `D2`): out from `N1` at `600·k`, 300 s to `H`; back from `H` at
//! `600·k + 300`, 300 s to `N1`; `k = 0..18`. Walking is 4.8 km/h (4/3 m/s) on
//! the walk layer, each walk floored to whole seconds once; a passenger boards
//! only if at the stop 60 s before the tram leaves (the boarding slack); walks
//! to and from stops are at most 900 s (1200 m). Walks used: `W → N1` on the
//! diagonal `w1`, 424.26 m = **318 s**; `H → N3` (e2, then round the ring, n3),
//! 460 m = **345 s**; `D1 → H` (a4, a5, r1, e2), 690 m = **517 s**. `N1` and
//! `H` are 1302 m apart on foot: out of each other's reach.
//!
//! | Case | Trip | Hand value |
//! |---|---|---|
//! | TR1 | `N1 → D2`, leaving 0 | at the stop at 0; 0 + 60 > 0 misses the 0 tram; the 600 tram, at `H` 900; **900 s** |
//! | TR2 | `W → N3`, leaving 200 | stop at 518; 578 ≤ 600: the 600 tram, `H` 900; + 345: **1245**, travel **1045 s** |
//! | TR3 | `D1 → W`, leaving 0 | stop `H` at 517; 577 > 300: the 900 tram back, `N1` 1200; + 318: **1518 s** |
//! | TR4 | `N1 → D2`, leaving 20 000 | after the last tram (10 500): **no path** |
//! | TR5 | TR2 with the window at 1000 s | arrives at 1245: **truncated** |
//! | TR6 | TR1 in a run without a timetable | **mode not available** |

#![allow(clippy::float_cmp, reason = "travel times here are whole seconds")]

use std::sync::Arc;

use openmobisim_core_demand::{ClassDefaults, Mode, Ownership, RawTrip, build_travellers};
use openmobisim_core_graph::hubs::HubKind;
use openmobisim_core_graph::layers::{BikeCost, Layer, StaticLayerDefaults};
use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_graph::{LonLat, toy_network, toy_network_layers};
use openmobisim_core_sim::{EventType, LayerSetup, Run, RunResult, StaticLayers, TransitSetup};
use openmobisim_core_transit::TransitDefaults;
use openmobisim_core_transit::examples::toy_tram;
use openmobisim_core_types::diagnostics::Diagnostics;
use openmobisim_core_types::ids::{EntityId, NodeId, TransitRunId};
use openmobisim_core_types::time::Second;

struct Toy {
    road: Arc<RoadNetwork>,
    layers: Arc<StaticLayers>,
    transit: Arc<TransitSetup>,
}

fn toy() -> Toy {
    let (road, _) = toy_network();
    let (bike, walk) = toy_network_layers();
    let d = StaticLayerDefaults::SHIPPED;
    let layers = Arc::new(StaticLayers {
        bike: Some(LayerSetup::new(Arc::new(bike), BikeCost::Dedicated, d)),
        walk: Some(LayerSetup::new(Arc::new(walk), BikeCost::Dedicated, d)),
    });
    let transit = Arc::new(TransitSetup::new(
        Arc::new(toy_tram()),
        layers.walk.as_ref().unwrap(),
        layers.bike.as_ref(),
        TransitDefaults::SHIPPED,
    ));
    Toy { road: Arc::new(road), layers, transit }
}

impl Toy {
    fn at(&self, name: &str) -> LonLat {
        let node: NodeId = self.road.node_external_ids().typed_id_of(name).expect("a toy node");
        self.road.node_lonlat(node)
    }

    fn trip(&self, who: &str, from: &str, to: &str, t: u32) -> RawTrip {
        RawTrip {
            traveller_id: who.to_string(),
            trip_seq: 0,
            origin: self.at(from),
            destination: self.at(to),
            departure_time: Second(t),
            user_class: "everyone".to_string(),
            weight: None,
            mode: Some(Mode::Transit),
        }
    }

    fn run(&self, rows: Vec<RawTrip>, window: u32, with_transit: bool) -> RunResult {
        let defaults = ClassDefaults::new()
            .with_default("everyone", Ownership { car: false, bike: false, transit_pass: false });
        let (travellers, trips) =
            build_travellers(rows, Vec::new(), &defaults, 1, &mut Diagnostics::new()).unwrap();
        let mut run =
            Run::new(self.road.clone(), Arc::new(travellers), Arc::new(trips), Second(window))
                .with_layers(self.layers.clone())
                .with_link_bins(300);
        if with_transit {
            run = run.with_transit(self.transit.clone());
        }
        run.execute(&mut Diagnostics::new())
    }
}

/// Each trip's outcome and time, by traveller id (one trip each).
fn outcomes(result: &RunResult) -> Vec<(EventType, u32)> {
    let mut e: Vec<_> =
        result.events.iter().map(|e| (e.entity_id, e.event_type, e.second.get())).collect();
    e.sort_by_key(|x| x.0);
    e.into_iter().map(|(_, t, s)| (t, s)).collect()
}

#[test]
fn stops_are_hubs_with_walk_bike_and_transit_access_points() {
    let t = toy();
    let hubs = t.transit.hubs();
    assert_eq!(hubs.len(), 2);
    for (stop, node) in [("H", "D2"), ("N1", "N1")] {
        let hub = hubs.id_of(stop).expect("a stop hub");
        assert_eq!(hubs.kind(hub), HubKind::Stop);
        let layers: Vec<Layer> =
            hubs.access_points(hub).map(|p| hubs.access_point(p).layer).collect();
        assert_eq!(layers, [Layer::Bike, Layer::Walk, Layer::Transit], "{stop}");
        let walk = hubs.access_point(hubs.access_point_on(hub, Layer::Walk).unwrap()).node;
        let walk_graph = t.layers.walk.as_ref().unwrap().network().network();
        assert_eq!(walk_graph.node_external_ids().external(walk.raw()), node);
    }
    // The two stops are out of each other's walking reach: no footpaths.
    assert!(t.transit.footpaths().is_empty());
}

#[test]
fn tr1_to_tr3_ride_the_tram() {
    let t = toy();
    let result = t.run(
        vec![t.trip("a", "N1", "D2", 0), t.trip("b", "W", "N3", 200), t.trip("c", "D1", "W", 0)],
        86_400,
        true,
    );
    assert_eq!(
        outcomes(&result),
        [
            (EventType::TripCompleted, 900),
            (EventType::TripCompleted, 1245),
            (EventType::TripCompleted, 1518),
        ]
    );
    let transit = result.by_mode[Mode::Transit.index()];
    assert_eq!(transit.completion.completed, 3);
    assert_eq!(transit.total_travel_time.get(), 900.0 + 1045.0 + 1518.0);
    assert_eq!(result.total_travel_time.get(), transit.total_travel_time.get());

    // Who boarded where: two on the 600 tram out, one on the 900 tram back.
    let tt = t.transit.timetable();
    let loads = result.transit.as_ref().expect("the run has a timetable");
    let run = |id: &str| TransitRunId::new(tt.run_ids().id_of(id).unwrap());
    let out = tt.run_calls(run("T1:out:01"));
    assert_eq!((loads.boardings[out.start], loads.alightings[out.start + 1]), (2.0, 2.0));
    let back = tt.run_calls(run("T1:back:01"));
    assert_eq!((loads.boardings[back.start], loads.alightings[back.start + 1]), (1.0, 1.0));
    assert_eq!(loads.boardings.iter().sum::<f64>(), 3.0);
    assert_eq!(loads.on_board(tt)[out.start], 2.0);
    assert_eq!(loads.times, *tt.scheduled(), "a tram runs by the schedule");

    // The walks on the walk layer: w1 and the four links to N3 (b); four links
    // from D1 and w1 back (c); a walks nowhere.
    let walk = result.walk_link_bins.as_ref().expect("walks were recorded");
    assert_eq!(walk.crossings().iter().sum::<u32>(), 5 + 5);
}

#[test]
fn tr4_after_the_last_tram_there_is_no_path() {
    let t = toy();
    let result = t.run(vec![t.trip("a", "N1", "D2", 20_000)], 86_400, true);
    assert_eq!(outcomes(&result), [(EventType::NoFeasiblePath, 20_000)]);
    assert_eq!(result.by_mode[Mode::Transit.index()].completion.no_feasible_path, 1);
}

#[test]
fn tr5_a_trip_still_on_its_way_at_the_end_is_truncated() {
    let t = toy();
    let result = t.run(vec![t.trip("b", "W", "N3", 200)], 1000, true);
    assert_eq!(outcomes(&result), [(EventType::TripTruncated, 1000)]);
    assert_eq!(result.completion.truncated, 1);
}

#[test]
fn tr6_without_a_timetable_transit_is_not_available() {
    let t = toy();
    let result = t.run(vec![t.trip("a", "N1", "D2", 0)], 86_400, false);
    assert_eq!(outcomes(&result), [(EventType::ModeNotAvailable, 0)]);
    assert!(result.transit.is_none());
}
