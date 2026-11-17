//! A link's drawing (its shape and road class) against hand values: kept as given, projected as
//! the nodes are, and seen by nothing a run reads (the length and the fingerprint unchanged).
//! For bikes: a cycle path keeps its shape and class as a street does.

mod common;

use common::{VALUES, at, cycle_path, row, two_nodes};
use openmobisim_graph::geometry::{LonLat, Position, Projection, Xy};
use openmobisim_graph::network::{LinkDrawing, NetworkBuilder};
use openmobisim_types::ids::{EntityId, LinkId};

fn drawing(shape: Vec<Position>, class: Option<&str>) -> LinkDrawing {
    LinkDrawing { shape, road_class: class.map(str::to_owned) }
}

/// A bend 300 m north of the middle of `ab` (a 1 000 m link east).
fn bent() -> LinkDrawing {
    drawing(vec![at(500.0, 300.0)], Some("residential"))
}

#[test]
fn a_shape_and_a_class_are_kept_as_given() {
    let mut b = two_nodes(row(1, 50.0));
    b.add_drawing("ab", bent());
    let net = b.build(VALUES).unwrap();
    let ab = LinkId::new(0);
    assert_eq!(net.shape(ab), &[Xy::new(500.0, 300.0)]);
    assert_eq!(net.road_class(ab), Some("residential"));
}

#[test]
fn a_link_without_a_drawing_is_straight_and_unclassed() {
    let net = two_nodes(row(1, 50.0)).build(VALUES).unwrap();
    assert!(net.shape(LinkId::new(0)).is_empty());
    assert_eq!(net.road_class(LinkId::new(0)), None);
}

#[test]
fn the_shape_changes_neither_the_length_nor_the_fingerprint() {
    // The length absent is the straight line, 1 000 m, not the 1 166.2 m along the bend
    // (2 × √(500² + 300²)); a class changes nothing a run reads either.
    let plain = two_nodes(row(1, 50.0)).build(VALUES).unwrap();
    let mut b = two_nodes(row(1, 50.0));
    b.add_drawing("ab", bent());
    let drawn = b.build(VALUES).unwrap();
    assert_eq!(drawn.length(LinkId::new(0)).get(), 1_000.0);
    assert_eq!(drawn.fingerprint(), plain.fingerprint());
    let mut b = two_nodes(cycle_path());
    b.add_drawing("ab", drawing(vec![at(500.0, -40.0), at(800.0, -40.0)], Some("cycleway")));
    let path = b.build(VALUES).unwrap();
    assert_eq!(path.fingerprint(), two_nodes(cycle_path()).build(VALUES).unwrap().fingerprint());
    assert_eq!(path.shape(LinkId::new(0)), &[Xy::new(500.0, -40.0), Xy::new(800.0, -40.0)]);
    assert_eq!(path.road_class(LinkId::new(0)), Some("cycleway"));
}

#[test]
fn a_longitude_latitude_shape_is_projected_as_the_nodes_are() {
    let (a, m, c) = (LonLat::new(4.36, 52.0), LonLat::new(4.365, 52.003), LonLat::new(4.37, 52.0));
    let mut b = NetworkBuilder::new();
    b.add_node("a", Position::LonLat(a));
    b.add_node("c", Position::LonLat(c));
    b.add_link("ac", "a", "c", row(1, 50.0));
    b.add_drawing("ac", drawing(vec![Position::LonLat(m)], None));
    let net = b.build(VALUES).unwrap();
    let projection = Projection::for_extent([a, c]).unwrap();
    assert_eq!(net.shape(LinkId::new(0)), &[projection.project(m)]);
}

#[test]
fn bad_drawings_are_set_aside_and_noted() {
    let built = |link: &str, d: LinkDrawing| {
        let mut b = two_nodes(row(1, 50.0));
        b.add_drawing(link, d);
        b.build(VALUES).unwrap()
    };
    let noted = |net: &openmobisim_graph::network::Network| -> Vec<(&str, u64, Vec<String>)> {
        net.repairs().kinds().iter().map(|k| (k.key, k.count, k.first.clone())).collect()
    };
    let net = built("zz", bent());
    assert!(net.shape(LinkId::new(0)).is_empty());
    assert_eq!(noted(&net), [("drawings_unplaced", 1, vec!["zz".to_string()])]);
    // A point of the other kind, or not finite: the link drawn straight, its class kept.
    let lon_lat = Position::LonLat(LonLat::new(4.0, 52.0));
    for shape in [vec![at(1.0, 1.0), lon_lat], vec![at(f64::NAN, 0.0)]] {
        let net = built("ab", drawing(shape, Some("primary")));
        assert!(net.shape(LinkId::new(0)).is_empty());
        assert_eq!(net.road_class(LinkId::new(0)), Some("primary"));
        assert_eq!(noted(&net), [("shapes_unread", 1, vec!["ab".to_string()])]);
    }
    // Two drawings of one link: the first given kept.
    let mut b = two_nodes(row(1, 50.0));
    b.add_drawing("ab", bent());
    b.add_drawing("ab", drawing(vec![], Some("cycleway")));
    let net = b.build(VALUES).unwrap();
    assert_eq!(net.road_class(LinkId::new(0)), Some("residential"));
    assert_eq!(noted(&net), [("drawings_repeated", 1, vec!["ab".to_string()])]);
}

/// A link id given twice, each row with its drawing, the first row naming a node not held: the
/// second row is kept with its own drawing (the cycleway), and nothing else is noted.
#[test]
fn a_repeated_link_keeps_the_drawing_of_its_row_kept() {
    let mut b = NetworkBuilder::new();
    b.add_node("a", at(0.0, 0.0));
    b.add_node("b", at(1_000.0, 0.0));
    b.add_link("ab", "a", "nowhere", row(1, 50.0));
    b.add_drawing("ab", bent());
    b.add_link("ab", "a", "b", row(1, 50.0));
    b.add_drawing("ab", drawing(vec![], Some("cycleway")));
    let net = b.build(VALUES).unwrap();
    assert_eq!(net.road_class(LinkId::new(0)), Some("cycleway"));
    assert!(net.shape(LinkId::new(0)).is_empty());
    let keys: Vec<_> = net.repairs().kinds().iter().map(|k| (k.key, k.count)).collect();
    assert_eq!(keys, [("links_repeated", 1)]);
}

/// Three links a→b, b→c, c→a with a shape and a class each, given with their drawings in one
/// order and in the reverse: the same shape and class on every link.
#[test]
fn drawings_given_in_any_order_build_the_same_network() {
    let build = |reversed: bool| {
        let mut b = NetworkBuilder::new();
        for (id, x, y) in [("a", 0.0, 0.0), ("b", 1_000.0, 0.0), ("c", 500.0, 800.0)] {
            b.add_node(id, at(x, y));
        }
        let links = [("ab", "a", "b"), ("bc", "b", "c"), ("ca", "c", "a")];
        let mut drawings: Vec<(&str, LinkDrawing)> = vec![
            ("ab", drawing(vec![at(500.0, -50.0)], Some("primary"))),
            ("bc", drawing(vec![at(800.0, 400.0), at(700.0, 600.0)], Some("residential"))),
            ("ca", drawing(vec![], Some("cycleway"))),
        ];
        if reversed {
            drawings.reverse();
        }
        for (id, up, down) in links {
            b.add_link(id, up, down, row(1, 50.0));
        }
        for (id, d) in drawings {
            b.add_drawing(id, d);
        }
        b.build(VALUES).unwrap()
    };
    let (one, other) = (build(false), build(true));
    for l in (0..3).map(LinkId::new) {
        assert_eq!(one.shape(l), other.shape(l));
        assert_eq!(one.road_class(l), other.road_class(l));
    }
    assert_eq!(one.shape(LinkId::new(1)), &[Xy::new(800.0, 400.0), Xy::new(700.0, 600.0)]);
    assert_eq!(one.road_class(LinkId::new(2)), Some("cycleway"));
}
