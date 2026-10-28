//! A fleet day's roads without a run, on the toy network: drives on stored road times, the
//! bounded reaches, a shared bike's rides and walks.
//!
//! The toy (`openmobisim_core_graph::examples::toy_network`): `R2` to `D2` is the 300 m road
//! `e2`; the ring `R1 → R2 → R3 → R4 → R1` runs in 30 m links.

use openmobisim_core_graph::defaults::SignalDefaults;
use openmobisim_core_graph::layers::{BikeCost, StaticLayerDefaults};
use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_graph::{toy_network, toy_network_layers};
use openmobisim_core_routes::SearchContext;
use openmobisim_core_sim::LayerSetup;
use openmobisim_core_sim::services::roads::{FrozenPaths, FrozenRoads, RoadTimes};
use openmobisim_core_types::ids::{EntityId, NodeId};
use std::sync::Arc;

fn node(road: &RoadNetwork, name: &str) -> NodeId {
    road.node_external_ids().typed_id_of(name).expect("a toy node")
}

fn link(road: &RoadNetwork, name: &str) -> u32 {
    road.link_external_ids().id_of(name).expect("a toy link")
}

#[test]
fn a_drive_takes_the_road_times_of_its_bins_and_free_flow_elsewhere() {
    let (road, _) = toy_network();
    let turns = TurnTable::build(&road, SignalDefaults::SHIPPED);
    let ctx = SearchContext::new(&road, &turns);
    let (r2, d2) = (node(&road, "R2"), node(&road, "D2"));
    let e2 = link(&road, "e2");
    let ff = road.free_flow_time(openmobisim_core_types::ids::LinkId::new(e2)).get();

    let free = FrozenRoads::new(&road, &ctx, RoadTimes::free_flow(&road));
    let (s, m) = free.drive(r2, d2, 8.0 * 3600.0);
    assert!((m - 300.0).abs() < 1e-6, "the one road e2: {m} m");
    assert!(s >= ff - 1e-9, "at least the link's free flow: {s} s against {ff} s");

    // e2 three times slower in the 15 minutes from 08:00 (bin 32), and only then.
    let slow = RoadTimes::from_rows(&road, 900.0, &[(e2, 32, ff * 3.0)]).unwrap();
    let roads = FrozenRoads::new(&road, &ctx, slow);
    let (in_bin, _) = roads.drive(r2, d2, 8.0 * 3600.0);
    let (after, _) = roads.drive(r2, d2, 8.25 * 3600.0);
    assert!((in_bin - s - 2.0 * ff).abs() < 1e-6, "{in_bin} s: two free-flow times more");
    assert!((after - s).abs() < 1e-9, "the next bin is free flow again");
    assert_eq!(roads.drive(d2, d2, 0.0), (0.0, 0.0));

    // The reaches: D2 within 2 minutes of R2 at free flow, nearest first.
    let to = roads.reach_to(d2, 120.0);
    assert_eq!(to.first().map(|r| r.0), Some(d2), "the node itself, at 0 s");
    assert!(to.windows(2).all(|w| w[0].1 <= w[1].1), "nearest first");
    assert!(to.iter().any(|r| r.0 == r2));
    assert!(roads.reach_from(r2, 120.0).iter().any(|r| r.0 == d2));
}

#[test]
fn a_ride_and_a_walk_take_the_layer_s_shortest_time() {
    let (bike, walk) = toy_network_layers();
    let d = StaticLayerDefaults::SHIPPED;
    let bike = LayerSetup::new(Arc::new(bike), BikeCost::Dedicated, d);
    let walk = LayerSetup::new(Arc::new(walk), BikeCost::Dedicated, d);
    let paths = FrozenPaths::new(bike.network(), bike.seconds(), walk.network(), walk.seconds());
    let (bg, wg) = (bike.network().network(), walk.network().network());
    let (b_r2, b_d2) = (node(bg, "R2"), node(bg, "D2"));
    let ride = paths.ride_s(b_r2, b_d2);
    assert!(ride.is_finite() && ride > 0.0, "{ride}");
    assert!((paths.ride_s(b_r2, b_d2) - ride).abs() < 1e-12, "kept per pair");
    assert!(paths.ride_s(b_d2, b_d2).abs() < 1e-12);
    let walked = paths.walk_s(node(wg, "R2"), node(wg, "D2"));
    assert!(walked > ride, "walking 300 m takes longer than riding it: {walked} s, {ride} s");
    assert!(walked.is_finite());
}
