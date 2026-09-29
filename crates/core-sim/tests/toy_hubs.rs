//! Itineraries through the toy network's hub `H` at `D2` (S198): every number
//! derived by hand.
//!
//! The toy network's constants: residential 30 km/h
//! (8.333 m/s), service 20 km/h (5.556 m/s); the signal at `S` gives its
//! approach `a1` Webster's uniform delay, 0.5 · 90 · 0.52² / (1 − 0.85 · 0.48)
//! = 20.554 s. Bikes 15 km/h in mixed traffic, 18 km/h on the track; walking
//! 4.8 km/h. The transfer at `H` is 60 s.
//!
//! | Case | Itinerary | Hand value |
//! |---|---|---|
//! | H1 | bike `S → H` (a2, the track, c1–c3, a5, r1, e2), transfer, walk `H → N3` | 48 + 104 + 2.88 + 14.4 + 7.2 + 72 = **248.48 s**; + 60; + 460 m / 1.333 = **345 s**; **653.48 s** |
//! | H2 | car `W → H` at free flow (a1 with the signal, a2, a3, c1–c3, a5, r1, e2), transfer, walk `H → N3` | 56.554 + 24 + 48 + 1.44 + 10.8 + 3.6 + 36 = **180.394 s**; + 60 + 345 = **585.394 s** |
//! | H3 | anything leaving `H` by bike | `D2` is a dead end for bikes (`e2` runs one way into it): **no itinerary** |
//! | H4 | to transit at `H` | no transit access point yet: **no itinerary** |

use openmobisim_core_graph::defaults::SignalDefaults;
use openmobisim_core_graph::layers::{BikeCost, Layer, StaticLayerDefaults};
use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_graph::{toy_network, toy_network_hubs, toy_network_layers};
use openmobisim_core_sim::{LayerSetup, Networks, StaticLayers};
use openmobisim_core_types::ids::{EntityId, NodeId};

struct Toy {
    road: RoadNetwork,
    turns: TurnTable,
    layers: StaticLayers,
}

fn toy() -> Toy {
    let (road, _) = toy_network();
    let turns = TurnTable::build(&road, SignalDefaults::SHIPPED);
    let (bike, walk) = toy_network_layers();
    let d = StaticLayerDefaults::SHIPPED;
    let layers = StaticLayers {
        bike: Some(LayerSetup::new(std::sync::Arc::new(bike), BikeCost::Dedicated, d)),
        walk: Some(LayerSetup::new(std::sync::Arc::new(walk), BikeCost::Dedicated, d)),
    };
    Toy { road, turns, layers }
}

impl Toy {
    fn networks(&self) -> Networks<'_> {
        Networks { road: &self.road, road_turns: &self.turns, layers: &self.layers }
    }

    fn node(&self, layer: Layer, name: &str) -> NodeId {
        let graph = match layer {
            Layer::Car => &self.road,
            _ => self.layers.get(layer.as_static().unwrap()).unwrap().network().network(),
        };
        graph.node_external_ids().typed_id_of(name).expect("a toy node")
    }

    fn link_names(
        &self,
        layer: Layer,
        links: &[openmobisim_core_types::ids::LinkId],
    ) -> Vec<String> {
        let graph = match layer {
            Layer::Car => &self.road,
            _ => self.layers.get(layer.as_static().unwrap()).unwrap().network().network(),
        };
        links.iter().map(|l| graph.link_external_ids().external(l.raw()).to_string()).collect()
    }
}

const WEBSTER_A1: f64 = 0.5 * 90.0 * 0.52 * 0.52 / (1.0 - 0.85 * 0.48);

#[test]
fn h1_cycle_to_the_hub_change_and_walk_on() {
    let t = toy();
    let hubs = toy_network_hubs();
    let h = hubs.id_of("H").unwrap();
    let it = t
        .networks()
        .via_hub(
            &hubs,
            h,
            Layer::Bike,
            t.node(Layer::Bike, "S"),
            Layer::Walk,
            t.node(Layer::Walk, "N3"),
        )
        .expect("an itinerary");
    assert_eq!(
        t.link_names(Layer::Bike, &it.legs[0].links),
        ["a2", "t1", "t2", "c1", "c2", "c3", "a5", "r1", "e2"],
        "the track, not a3, under the dedicated cost"
    );
    assert!((it.legs[0].seconds - 248.48).abs() < 1e-9, "{}", it.legs[0].seconds);
    assert!((it.transfers[0].1 - 60.0).abs() < 1e-12);
    assert!((it.legs[1].seconds - 345.0).abs() < 1e-9, "{}", it.legs[1].seconds);
    assert!((it.seconds() - 653.48).abs() < 1e-9);
}

#[test]
fn h2_drive_to_the_hub_change_and_walk_on() {
    let t = toy();
    let hubs = toy_network_hubs();
    let h = hubs.id_of("H").unwrap();
    let it = t
        .networks()
        .via_hub(
            &hubs,
            h,
            Layer::Car,
            t.node(Layer::Car, "W"),
            Layer::Walk,
            t.node(Layer::Walk, "N3"),
        )
        .expect("an itinerary");
    assert_eq!(
        t.link_names(Layer::Car, &it.legs[0].links),
        ["a1", "a2", "a3", "c1", "c2", "c3", "a5", "r1", "e2"]
    );
    let car = (36.0 + WEBSTER_A1) + 24.0 + 48.0 + 1.44 + 10.8 + 3.6 + 36.0;
    assert!((car - 180.394).abs() < 1e-3, "the hand value itself: {car}");
    assert!((it.legs[0].seconds - car).abs() < 1e-6, "{} vs {car}", it.legs[0].seconds);
    assert!((it.seconds() - (car + 60.0 + 345.0)).abs() < 1e-6);
}

#[test]
fn h3_no_bike_leaves_the_hub_because_d2_is_a_dead_end_for_bikes() {
    let t = toy();
    let hubs = toy_network_hubs();
    let h = hubs.id_of("H").unwrap();
    let from = t.node(Layer::Walk, "N3");
    let to = t.node(Layer::Bike, "W");
    assert!(t.networks().via_hub(&hubs, h, Layer::Walk, from, Layer::Bike, to).is_none());
    // Walking back from H to N3 and on by bike is another matter: N3 → R4 → R1 … is ridden.
    assert!(
        t.networks()
            .leg(Layer::Bike, t.node(Layer::Bike, "N3"), t.node(Layer::Bike, "D2"))
            .is_some()
    );
}

#[test]
fn h4_no_transit_access_point_yet_means_no_itinerary() {
    let t = toy();
    let hubs = toy_network_hubs();
    let h = hubs.id_of("H").unwrap();
    assert_eq!(hubs.access_points(h).count(), 3);
    assert!(hubs.access_point_on(h, Layer::Transit).is_none());
    let from = t.node(Layer::Walk, "N3");
    assert!(
        t.networks().via_hub(&hubs, h, Layer::Walk, from, Layer::Transit, NodeId::new(0)).is_none()
    );
}
