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
/// so that the car network is strongly connected whatever way 100's direction.
pub fn street(n: i64, tags: &[(&str, &str)]) -> Osm {
    let nodes: Vec<i64> = (1..=n).collect();
    let mut all: Vec<OsmNode> = (1..=n).map(|i| at(i, i)).collect();
    all.push(OsmNode::new(999, 4.36, 52.001));
    let back = OsmWay::new(900, &[n, 999, 1], &[("highway", "residential")]);
    Osm { nodes: all, ways: vec![OsmWay::new(100, &nodes, tags), back] }
}

/// The network of `osm` at the test values.
pub fn read(osm: &Osm) -> Result<(Network, Report), OsmError> {
    network(osm, VALUES)
}

/// One direction of one way, as the network holds it.
#[derive(Debug, PartialEq)]
pub struct Direction {
    /// The modes, as letters: `c`, `b`, `w`, in that order.
    pub modes: String,
    /// The car's lanes (`None` without cars).
    pub lanes: Option<u8>,
    /// The car's free-flow speed in km/h (`None` without cars).
    pub speed_km_h: Option<f64>,
    /// The sum of its links' lengths, metres.
    pub length_m: f64,
    /// Its links.
    pub links: Vec<LinkId>,
}

/// The links of way `way` in direction `tag` (`f` along its node order, `b` against it), or
/// `None` if it has none. Panics if its links disagree on modes, lanes or speed.
pub fn direction(net: &Network, way: i64, tag: &str) -> Option<Direction> {
    let links: Vec<LinkId> = LinkId::iter_space(net.link_count())
        .filter(|&l| {
            let id = net.link_ids().external(l.raw());
            id.starts_with(&format!("{way}:")) && id.ends_with(&format!(":{tag}"))
        })
        .collect();
    let first = *links.first()?;
    let read = |l: LinkId| {
        let modes: String = [(Mode::Car, 'c'), (Mode::Bike, 'b'), (Mode::Walk, 'w')]
            .iter()
            .filter(|(m, _)| net.modes(l).contains(*m))
            .map(|&(_, c)| c)
            .collect();
        let car = net.modes(l).contains(Mode::Car);
        let speed = car.then(|| net.free_flow_speed(l, Mode::Car).0 * 3.6);
        (modes, car.then(|| net.lanes(l)), speed)
    };
    let (modes, lanes, speed_km_h) = read(first);
    for &l in &links {
        assert_eq!(read(l), (modes.clone(), lanes, speed_km_h), "way {way} {tag}: links differ");
    }
    let length_m = links.iter().map(|&l| net.length(l).0).sum();
    Some(Direction { modes, lanes, speed_km_h, length_m, links })
}

/// `a` and `b` within `tol`.
pub fn close(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}
