//! Scenario edits of a road network (S238): closures, lanes, capacity factors.

use openmobisim_core_graph::network::LinkChanges;
use openmobisim_core_graph::toy_network;
use openmobisim_core_types::ids::{EntityId, LinkId};

#[test]
fn a_closed_link_keeps_its_index_and_data_but_no_node_reaches_it() {
    let (road, _) = toy_network();
    let a2 = LinkId::from_index(road.link_external_ids().id_of("a2").unwrap() as usize);
    let edited =
        road.with_changes(&LinkChanges { closed: vec![a2], ..LinkChanges::default() }).unwrap();
    assert_eq!(edited.link_count(), road.link_count());
    assert!(edited.is_closed(a2) && !edited.is_drivable(a2) && road.is_drivable(a2));
    assert_eq!(edited.link_from(a2), road.link_from(a2), "its data kept");
    assert!(!edited.out_links(road.link_from(a2)).contains(&a2));
    assert!(!edited.in_links(road.link_to(a2)).contains(&a2));
    assert!(road.out_links(road.link_from(a2)).contains(&a2), "the original untouched");
    let others = (0..road.link_count()).map(LinkId::new).filter(|&l| l != a2);
    for l in others {
        assert!(!edited.is_closed(l));
        assert!(edited.out_links(edited.link_from(l)).contains(&l));
    }
}

#[test]
fn lanes_scale_the_diagram_and_a_capacity_factor_keeps_it_a_triangle() {
    let (road, _) = toy_network();
    let id =
        |name: &str| LinkId::from_index(road.link_external_ids().id_of(name).unwrap() as usize);
    let (m1, a1) = (id("m1"), id("a1"));
    let changes = LinkChanges {
        lanes: vec![(m1, 1)],
        capacity_factor: vec![(a1, 0.5)],
        ..LinkChanges::default()
    };
    let edited = road.with_changes(&changes).unwrap();
    let (p, q) = (road.link_parameters(m1), edited.link_parameters(m1));
    assert!((q.capacity.get() - p.capacity.get() / 2.0).abs() < 1e-9);
    assert!((q.jam_density.get() - p.jam_density.get() / 2.0).abs() < 1e-12);
    assert_eq!(q.wave_speed, p.wave_speed, "both halved: the same wave speed");
    assert!((edited.storage(m1).get() - road.storage(m1).get() / 2.0).abs() < 1e-9);
    let (p, q) = (road.link_parameters(a1), edited.link_parameters(a1));
    assert!((q.capacity.get() - p.capacity.get() / 2.0).abs() < 1e-9);
    assert!(q.consistency_error() < 1e-9, "still a triangle");
    assert_eq!(edited.storage(a1), road.storage(a1), "a bottleneck keeps its storage");
    assert_eq!(edited.free_flow_time(a1), road.free_flow_time(a1));
}

#[test]
fn wrong_changes_are_refused() {
    let (road, _) = toy_network();
    let far = LinkId::new(road.link_count());
    let refused = |c: LinkChanges| road.with_changes(&c).unwrap_err();
    assert!(
        refused(LinkChanges { closed: vec![far], ..LinkChanges::default() }).contains("no link")
    );
    assert!(
        refused(LinkChanges { lanes: vec![(LinkId::new(0), 0)], ..LinkChanges::default() })
            .contains("0 lanes")
    );
    assert!(
        refused(LinkChanges {
            capacity_factor: vec![(LinkId::new(0), 0.0)],
            ..LinkChanges::default()
        })
        .contains("above 0")
    );
}
