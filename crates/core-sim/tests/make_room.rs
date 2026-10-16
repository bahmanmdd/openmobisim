//! `make_room`: a full set drops only routes nobody uses — not one a route choice
//! names, and not one an itinerary trip drives as its car alternative (`kept`), which no route
//! choice names (review R-1).

use openmobisim_core_graph::defaults::{GlobalMultipliers, RoadClass, SignalDefaults};
use openmobisim_core_graph::geometry::LonLat;
use openmobisim_core_graph::network::{LinkSpec, RoadNetwork, RoadNetworkBuilder};
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_routes::{Route, RouteKey, RouteSets, Shortest};
use openmobisim_core_sim::route_update::make_room;
use openmobisim_core_types::diagnostics::Diagnostics;
use openmobisim_core_types::ids::{EntityId, LinkId, NodeId};

/// Two routes from `o` to `d`: a short residential street and a longer primary road via `m`.
fn two_routes() -> RoadNetwork {
    let mut b = RoadNetworkBuilder::new();
    b.add_node("o", LonLat::new(4.800, 45.700));
    b.add_node("m", LonLat::new(4.805, 45.7004));
    b.add_node("d", LonLat::new(4.810, 45.700));
    let mut fast = LinkSpec::new(RoadClass::Residential);
    fast.lanes = Some(1);
    fast.length_m = Some(600.0);
    b.add_link("fast", "o", "d", fast);
    let mut slow = LinkSpec::new(RoadClass::Primary);
    slow.lanes = Some(2);
    slow.length_m = Some(900.0);
    b.add_link("slow1", "o", "m", slow);
    b.add_link("slow2", "m", "d", slow);
    b.build(GlobalMultipliers::default(), SignalDefaults::SHIPPED, &mut Diagnostics::new())
        .expect("buildable")
}

fn sets() -> RouteSets {
    let network = two_routes();
    let turns = TurnTable::build(&network, network.signals());
    let node = |n: &str| network.node_external_ids().typed_id_of::<NodeId>(n).unwrap();
    let key = RouteKey::new(node("o"), node("d"));
    let base = RouteSets::generate(&network, &turns, &[key], &Shortest);
    assert_eq!(base.route_count(), 1, "the shortest route alone");
    // The other route, added as a route update would (its cost above the shortest's).
    let fast = network.link_external_ids().typed_id_of::<LinkId>("fast").unwrap();
    let (sets, _) = base.extended(
        &[(0, vec![Route { links: vec![fast], cost: 100.0, overlap: 0.0 }])],
        1,
        "test",
    );
    assert_eq!(sets.route_count(), 2, "the two routes of the toy");
    sets
}

fn addition() -> Vec<(usize, Vec<Route>)> {
    vec![(0, vec![Route { links: vec![LinkId::new(0)], cost: 1.0, overlap: 0.0 }])]
}

#[test]
fn a_route_nobody_uses_makes_room() {
    let sets = sets();
    let mut additions = addition();
    // One trip, on the shortest route (route 0 by construction: best first).
    let removed = make_room(&sets, &mut additions, &[0], &[1], &[], 2);
    assert_eq!(removed.iter().filter(|&&r| r).count(), 1, "one route removed");
    assert!(!removed[0], "never the free-flow shortest");
    assert_eq!(additions.len(), 1, "the addition fits");
}

#[test]
fn a_route_an_itinerary_trip_drives_is_kept() {
    let sets = sets();
    let mut additions = addition();
    let mut kept = vec![false; sets.route_count()];
    let other = (0..sets.route_count()).find(|&r| r != 0).expect("a second route");
    kept[other] = true;
    let removed = make_room(&sets, &mut additions, &[0], &[1], &kept, 2);
    assert!(removed.iter().all(|&r| !r), "nothing removed: every route is in use");
    assert!(additions.is_empty(), "the addition is dropped instead");
}
