//! A fleet day without a run, on the toy network: trips as a table, their legs planned to the
//! nearest docks, one engine day with closed-form arrivals.
//!
//! The toy (`openmobisim_core_graph::examples::toy_network`): `D2` and `R2` 300 m apart on the
//! road `e2`; `N1` about 830 m from `R2`.

use std::sync::Arc;

use openmobisim_core_graph::defaults::SignalDefaults;
use openmobisim_core_graph::geometry::LonLat;
use openmobisim_core_graph::layers::{BikeCost, StaticLayerDefaults};
use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_graph::{toy_network, toy_network_layers};
use openmobisim_core_sim::LayerSetup;
use openmobisim_core_sim::services::engine::{FleetDay, FleetEngine};
use openmobisim_core_sim::services::frozen::{TripRow, plan_legs};
use openmobisim_core_sim::services::outcome::LegStatus;
use openmobisim_core_sim::services::roads::{FrozenPaths, RoadTimes};
use openmobisim_core_sim::services::world::FrozenWorld;
use openmobisim_core_sim::services::{DockDefaults, DockRow, ServiceSetup};

fn at(road: &RoadNetwork, name: &str) -> LonLat {
    road.node_lonlat(road.node_external_ids().typed_id_of(name).expect("a toy node"))
}

fn dock(id: &str, at: LonLat, capacity: u32, bikes: u32) -> DockRow {
    DockRow {
        dock_id: id.into(),
        name: None,
        hub_id: None,
        position: at,
        capacity,
        bikes: Some(bikes),
    }
}

#[test]
fn trips_ride_between_their_nearest_docks_and_the_second_walks_on_to_the_returned_bike() {
    let (road, _) = toy_network();
    let (bike, walk) = toy_network_layers();
    let d = StaticLayerDefaults::SHIPPED;
    let bike = Arc::new(LayerSetup::new(Arc::new(bike), BikeCost::Dedicated, d));
    let walk = Arc::new(LayerSetup::new(Arc::new(walk), BikeCost::Dedicated, d));
    // One bike at R2; none at D2, which has room. (The bike layer's `e2` runs from R2 to D2.)
    let rows = [dock("d-D2", at(&road, "D2"), 4, 0), dock("d-R2", at(&road, "R2"), 4, 1)];
    let setups = vec![
        ServiceSetup::bike_docked("bike_share", &rows, &walk, &bike, None, DockDefaults::SHIPPED)
            .unwrap(),
    ];
    let paths = FrozenPaths::new(bike.network(), bike.seconds(), walk.network(), walk.seconds());
    let trip = |id: u32, t: f64, from: &str, to: &str| TripRow {
        trip: id,
        service: 0,
        weight: 1,
        departure_s: t,
        origin: at(&road, from),
        destination: at(&road, to),
    };
    let trips = [
        trip(1, 28_000.0, "R2", "D2"),
        trip(2, 28_010.0, "R2", "D2"),
        trip(3, 28_020.0, "D2", "D2"),
    ];
    let (planned, report) = plan_legs(&setups, &trips, &road, &walk, &paths);
    assert_eq!(
        (report.trips, report.legs, report.same_dock),
        (3, 2, 1),
        "D2 to D2: one dock for both"
    );
    let legs: Vec<_> = planned.iter().map(|&(_, l)| l).collect();
    assert_eq!((legs[0].pickup_dock, legs[0].return_dock), (1, 0), "docks in id order: d-D2, d-R2");
    assert!(legs[0].ride_s > 0.0 && legs[0].ride_s.is_finite());
    assert!(legs[0].egress_s.abs() < 1e-9, "the dock is at the destination's node");

    let day = Arc::new(FleetDay { window: (27_000.0, 34_200.0), legs: legs.clone(), seed: 1 });
    // The city, owned: the engine borrows nothing.
    let turns = Arc::new(TurnTable::build(&road, SignalDefaults::SHIPPED));
    let road = Arc::new(road);
    let setups = Arc::new(setups);
    let world = FrozenWorld::new(
        Arc::clone(&road),
        turns,
        Arc::clone(&walk),
        Arc::clone(&bike),
        RoadTimes::free_flow(&road),
        Arc::clone(&setups),
        &legs,
    );
    let out =
        FleetEngine::in_world(setups, day, Arc::new(world), None).run_day().expect("no policy");
    let (first, second) = (out.legs[0], out.legs[1]);
    let s = DockDefaults::SHIPPED;
    assert_eq!(first.status, LegStatus::Served);
    let expected = legs[0].request_s + s.pickup_s + legs[0].ride_s + s.return_s + legs[0].egress_s;
    assert!(
        (first.arrived_s - expected).abs() < 1e-6,
        "{} s against {expected} s",
        first.arrived_s
    );
    // The second finds R2 empty and walks on to D2, the nearest other dock, where the first
    // rider has just returned the bike: one redirection, the bike taken there.
    assert_eq!((second.picked, second.redirections), (Some(0), 1));
    let walk_r2_d2 = paths.walk_s(
        walk.network().network().node_external_ids().typed_id_of("R2").unwrap(),
        walk.network().network().node_external_ids().typed_id_of("D2").unwrap(),
    );
    assert!((second.extra_walk_s - walk_r2_d2).abs() < 1e-6, "{}", second.extra_walk_s);
    assert!(second.picked_s >= first.arrived_s - legs[0].egress_s - 1e-6, "after the return");
    assert_eq!(out.invariant_violations, 0);
    assert_eq!(
        out.docks[0].as_ref().map(|b| b.pickups.iter().sum::<f64>()),
        Some(2.0),
        "one bike, taken twice"
    );
}
