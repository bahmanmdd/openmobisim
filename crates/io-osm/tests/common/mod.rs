//! Fixtures shared by the reader's tests: build values, a straight street of nodes, and the
//! reading of a network's links back by their way.

#![allow(dead_code, reason = "each test binary uses a different part of this module")]

use openmobisim_graph::network::{BuildValues, Network};
use openmobisim_io_osm::{Osm, OsmError, OsmNode, OsmWay, Report, network};
use openmobisim_types::ids::{EntityId, LinkId};
use openmobisim_types::mode::Mode;

/// Test values for the three build values (not defaults): capacity_per_lane 1 800 veh/h,
/// bike_speed 15 km/h, walk_speed 4.8 km/h.
pub const VALUES: BuildValues =
    BuildValues { capacity_per_lane: 1_800.0, bike_speed: 15.0, walk_speed: 4.8 };

/// Node `id` at `i` thousandths of a degree east of 4.36°E on the parallel 52.0°N (about 69 m
/// apart, in Delft).
#[allow(clippy::cast_precision_loss, reason = "small test counts")]
pub fn at(id: i64, i: i64) -> OsmNode {
    OsmNode::new(id, 4.36 + i as f64 * 0.001, 52.0)
}

/// An extract of nodes 1..=n in a row, way 100 through them with `tags`, and a two-way
/// residential street back from node n to node 1 through node 999 north of them (way 900),
/// so that every layer is strongly connected whatever way 100's direction.
pub fn street(n: i64, tags: &[(&str, &str)]) -> Osm {
    let nodes: Vec<i64> = (1..=n).collect();
    let mut all: Vec<OsmNode> = (1..=n).map(|i| at(i, i)).collect();
    all.push(OsmNode::new(999, 4.36, 52.001));
    let back = OsmWay::new(900, &[n, 999, 1], &[("highway", "residential")]);
    Osm { nodes: all, ways: vec![OsmWay::new(100, &nodes, tags), back] }
}

/// The network of `osm` (the whole extract) at the test values.
pub fn read(osm: &Osm) -> Result<(Network, Report), OsmError> {
    network(osm, VALUES, None)
}

/// One direction of one way on one mode's layer, as the network holds it.
#[derive(Debug, PartialEq)]
pub struct Direction {
    /// The car's lanes (`None` on a bike or walk link).
    pub lanes: Option<u8>,
    /// The car's free-flow speed in km/h (`None` on a bike or walk link).
    pub speed_km_h: Option<f64>,
    /// The car's capacity in veh/h (`None` on a bike or walk link).
    pub capacity_veh_h: Option<f64>,
    /// The sum of its links' lengths, metres.
    pub length_m: f64,
    /// Its links.
    pub links: Vec<LinkId>,
}

/// The links of `mode` whose first piece is of way `way` in direction `tag` (`f` along its node
/// order, `b` against it), or `None` if it has none. Panics if they disagree on a car field.
pub fn direction(net: &Network, mode: Mode, way: i64, tag: &str) -> Option<Direction> {
    let links: Vec<LinkId> = LinkId::iter_space(net.link_count())
        .filter(|&l| {
            let id = net.link_ids().external(l.raw());
            id.starts_with(&format!("{mode}:{way}:")) && id.ends_with(&format!(":{tag}"))
        })
        .collect();
    let first = *links.first()?;
    let car = |l: LinkId| {
        net.modes(l).contains(Mode::Car).then(|| {
            let speed = net.free_flow_speed(l, Mode::Car).0 * 3.6;
            (net.lanes(l), (speed * 1e9).round() / 1e9, net.capacity(l).0 * 3600.0)
        })
    };
    for &l in &links {
        assert_eq!(car(l), car(first), "way {way} {tag}: links differ");
    }
    let length_m = links.iter().map(|&l| net.length(l).0).sum();
    let c = car(first);
    Some(Direction {
        lanes: c.map(|c| c.0),
        speed_km_h: c.map(|c| c.1),
        capacity_veh_h: c.map(|c| c.2),
        length_m,
        links,
    })
}

/// The modes with a link of way `way` in direction `tag`, as letters `c`, `b`, `w` in that
/// order (`-` for none).
pub fn modes(net: &Network, way: i64, tag: &str) -> String {
    let letters: String = [(Mode::Car, 'c'), (Mode::Bike, 'b'), (Mode::Walk, 'w')]
        .into_iter()
        .filter(|&(m, _)| direction(net, m, way, tag).is_some())
        .map(|(_, c)| c)
        .collect();
    if letters.is_empty() { "-".to_owned() } else { letters }
}

/// `a` and `b` within `tol`.
pub fn close(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}
