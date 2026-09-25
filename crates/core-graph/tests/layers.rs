//! Static layers made from a road network alone (S195): the grid's and the
//! toy network's bike and walk layers, and what their costs are.

use openmobisim_core_graph::defaults::RoadClass;
use openmobisim_core_graph::layers::{
    BikeCost, BikeInfrastructure, StaticLayer, StaticLayerDefaults, StaticLink,
    StaticNetworkBuilder,
};
use openmobisim_core_graph::network::{LinkSpec, RoadNetwork, RoadNetworkBuilder};
use openmobisim_core_graph::{GlobalMultipliers, LonLat, SignalDefaults, StaticNetwork};
use openmobisim_core_graph::{manhattan_grid, toy_network};
use openmobisim_core_types::diagnostics::Diagnostics;
use openmobisim_core_types::ids::{EntityId, LinkId};

fn derive(road: &RoadNetwork, layer: StaticLayer) -> StaticNetwork {
    StaticNetwork::derive(road, layer, StaticLayerDefaults::SHIPPED, &mut Diagnostics::new())
        .expect("a built road network projects")
}

fn pairs(network: &RoadNetwork) -> Vec<(String, String)> {
    let nodes = network.node_external_ids();
    let mut v: Vec<(String, String)> = LinkId::iter_space(network.link_count())
        .map(|l| {
            (
                nodes.external(network.link_from(l).raw()).to_owned(),
                nodes.external(network.link_to(l).raw()).to_owned(),
            )
        })
        .collect();
    v.sort();
    v
}

#[test]
fn a_two_way_grid_is_the_same_graph_for_bikes_and_walkers() {
    let (road, _) = manhattan_grid(4, 100.0, false);
    for layer in [StaticLayer::Bike, StaticLayer::Walk] {
        let g = derive(&road, layer);
        assert_eq!(pairs(g.network()), pairs(&road), "{}", layer.as_str());
        assert_eq!(g.network().projection(), road.projection());
    }
}

#[test]
fn the_one_way_toy_network_is_ridden_one_way_and_walked_both_ways() {
    let (road, _) = toy_network();
    let bike = derive(&road, StaticLayer::Bike);
    assert_eq!(pairs(bike.network()), pairs(&road), "bikes follow the one-way streets");

    let walk = derive(&road, StaticLayer::Walk);
    assert_eq!(walk.network().link_count(), 2 * road.link_count(), "every street both ways");
    let walked = pairs(walk.network());
    for (from, to) in pairs(&road) {
        assert!(walked.contains(&(to.clone(), from.clone())), "{to} → {from}");
    }
    // Node positions are the road network's own.
    for i in 0..walk.network().node_count() {
        let name = walk.network().node_external_ids().external(i);
        let r = road.node_external_ids().id_of(name).expect("same nodes");
        assert_eq!(
            walk.network().node_position(EntityId::new(i)),
            road.node_position(EntityId::new(r))
        );
    }
}

#[test]
fn derived_bike_speeds_are_the_mixed_speed_capped_by_the_road() {
    let (road, _) = toy_network();
    let bike = derive(&road, StaticLayer::Bike);
    let d = StaticLayerDefaults::SHIPPED;
    for link in LinkId::iter_space(bike.network().link_count()) {
        assert_eq!(bike.infrastructure(link), BikeInfrastructure::Mixed);
        assert!((bike.speed(link) * 3.6 - d.bike_mixed_km_h).abs() < 1e-9);
    }
}

/// Two nodes a kilometre apart, joined by a mixed street, and by a cycle track
/// 1.15 km long through a third node.
fn street_and_track() -> StaticNetwork {
    let mut b = StaticNetworkBuilder::new(StaticLayer::Bike);
    b.add_node("A", LonLat::new(4.80, 52.0));
    b.add_node("B", LonLat::new(4.8146, 52.0));
    b.add_node("T", LonLat::new(4.8073, 52.003));
    let link = |class, infrastructure, km_h, length| StaticLink {
        class,
        speed_km_h: km_h,
        infrastructure,
        length_m: Some(length),
    };
    let d = StaticLayerDefaults::SHIPPED;
    b.add_link(
        "street",
        "A",
        "B",
        link(RoadClass::Residential, BikeInfrastructure::Mixed, d.bike_mixed_km_h, 1000.0),
    );
    b.add_link(
        "t1",
        "A",
        "T",
        link(RoadClass::Cycleway, BikeInfrastructure::Separated, d.bike_dedicated_km_h, 575.0),
    );
    b.add_link(
        "t2",
        "T",
        "B",
        link(RoadClass::Cycleway, BikeInfrastructure::Separated, d.bike_dedicated_km_h, 575.0),
    );
    let projection = {
        let mut r = RoadNetworkBuilder::new();
        r.add_node("A", LonLat::new(4.80, 52.0));
        r.add_node("B", LonLat::new(4.8146, 52.0));
        r.add_link("x", "A", "B", LinkSpec::new(RoadClass::Residential));
        r.build(GlobalMultipliers::default(), SignalDefaults::SHIPPED, &mut Diagnostics::new())
            .unwrap()
            .projection()
    };
    b.build(projection, &mut Diagnostics::new()).unwrap()
}

#[test]
fn the_two_bike_costs_are_time_and_time_with_a_premium_on_mixed_traffic() {
    let g = street_and_track();
    let id = |name: &str| g.network().link_external_ids().id_of(name).unwrap() as usize;
    let seconds = g.link_seconds();
    // 1000 m at 15 km/h = 240 s; 575 m at 18 km/h = 115 s.
    assert!((seconds[id("street")] - 240.0).abs() < 1e-9);
    assert!((seconds[id("t1")] - 115.0).abs() < 1e-9);

    let time = g.link_costs(BikeCost::Time, 1.2);
    assert_eq!(time, seconds);
    let dedicated = g.link_costs(BikeCost::Dedicated, 1.2);
    assert!((dedicated[id("street")] - 288.0).abs() < 1e-9, "mixed: 240 × 1.2");
    assert!((dedicated[id("t1")] - 115.0).abs() < 1e-9, "dedicated: unchanged");
    // The track takes 230 s against the street's 240: it is the faster route
    // under both costs here, and by a wider margin under `dedicated`.
}

#[test]
fn the_walk_layer_s_cost_is_its_time_whatever_the_bike_cost() {
    let (road, _) = toy_network();
    let walk = derive(&road, StaticLayer::Walk);
    assert_eq!(walk.link_costs(BikeCost::Dedicated, 1.2), walk.link_seconds());
}
