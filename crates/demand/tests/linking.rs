//! Linking against a full scan: 20 000 random points (the two ends of 10 000 trips) over a
//! jittered 100 × 100 grid, each linked for car, bike and walk to the nearest node with a link
//! its mode may use, equal to a scan of every node (ties to the lower id), and a bike's end on
//! a cycle path by hand.
//!
//! The grid: nodes 100 m apart, each moved by a whole number of metres in −20..=20 in x and y;
//! streets between rows 40 and 49 are for cars only, between rows 60 and 69 cycle paths (bike
//! and walk), between rows 80 and 84 footways (walk only), so the three modes link to different
//! node sets. Points are drawn over the box from −300 m to 10 200 m, a little beyond the grid.
//! The tie rule is exercised: a quarter of the points lie midway between two neighbouring
//! nodes, a quarter at whole metres (ends decided by the lower id counted, at least 20).

mod common;

use common::{CARS_ONLY, CYCLE_PATH, Draws, FOOTWAY, STREET, at, grid, one_class, trip};
use openmobisim_demand::demand::Demand;
use openmobisim_graph::geometry::{Position, Xy};
use openmobisim_graph::network::{LinkRow, Network, NetworkBuilder};
use openmobisim_types::ids::{EntityId, LinkId, NodeId, TripId};
use openmobisim_types::mode::Mode;
use openmobisim_types::units::Metres;
use rayon::prelude::*;

const LINK_MODES: [Mode; 3] = [Mode::Car, Mode::Bike, Mode::Walk];

/// The rows of a street between grid nodes `a` and `z`.
fn zoned(a: (u32, u32), z: (u32, u32)) -> LinkRow {
    let both = |lo: u32, hi: u32| (lo..=hi).contains(&a.0) && (lo..=hi).contains(&z.0);
    if both(40, 49) {
        CARS_ONLY
    } else if both(60, 69) {
        CYCLE_PATH
    } else if both(80, 84) {
        FOOTWAY
    } else {
        STREET
    }
}

/// The nodes with a link `mode` may use, in id order, with their positions.
fn usable(net: &Network, mode: Mode) -> Vec<(NodeId, Xy)> {
    let uses = |links: &[LinkId]| links.iter().any(|&l| net.modes(l).contains(mode));
    NodeId::iter_space(net.node_count())
        .filter(|&n| uses(net.out_links(n)) || uses(net.in_links(n)))
        .map(|n| (n, net.node_position(n)))
        .collect()
}

/// The full scan: the node of `nodes` (in id order) nearest to `p`, the lower id at equal
/// distance, its distance, and whether two nodes were at that distance.
#[allow(clippy::float_cmp, reason = "an exact tie of the same distance computation")]
fn scan(nodes: &[(NodeId, Xy)], p: Xy) -> (NodeId, f64, bool) {
    let mut best = (NodeId::NULL, f64::INFINITY, false);
    for &(n, xy) in nodes {
        let d = p.distance_to(xy);
        if d < best.1 {
            best = (n, d, false);
        } else if d == best.1 {
            best.2 = true; // n has a higher id than best.0: ids are scanned in order
        }
    }
    best
}

#[test]
fn twenty_thousand_points_link_as_a_full_scan_finds() {
    let mut draws = Draws::new(1);
    let jitter: Vec<(f64, f64)> = (0..10_000)
        .map(|_| (f64::from(draws.below(41)) - 20.0, f64::from(draws.below(41)) - 20.0))
        .collect();
    let net = grid((100, 100), |r, c| jitter[(r * 100 + c) as usize], zoned);
    // Every fourth point midway between two neighbours (a tie unless a third node is nearer),
    // every fourth at whole metres, half anywhere, some outside the grid's box.
    let mut point = |i: u32| {
        let (x, y) = (draws.between(-300.0, 10_200.0), draws.between(-300.0, 10_200.0));
        match i % 4 {
            0 => {
                let (r, c) = (draws.below(100), draws.below(99));
                let (a, z) = (jitter[(r * 100 + c) as usize], jitter[(r * 100 + c + 1) as usize]);
                let y0 = f64::from(r) * 100.0;
                at(f64::from(c) * 100.0 + 50.0 + (a.0 + z.0) / 2.0, y0 + (a.1 + z.1) / 2.0)
            }
            1 => at(x.floor(), y.floor()),
            _ => at(x, y),
        }
    };
    let rows: Vec<_> = (0..10_000)
        .map(|i| trip(&format!("t{i:05}"), point(2 * i), point(2 * i + 1), &LINK_MODES))
        .collect();
    let demand = Demand::build(&rows, &one_class(), &net, None, Metres(1_500.0)).expect("builds");
    assert_eq!(demand.trip_count(), 10_000);
    let nodes = LINK_MODES.map(|m| usable(&net, m));
    // The scans in parallel: a debug build scans 600 million distances.
    let ties: usize = rows
        .par_iter()
        .enumerate()
        .map(|(i, row)| {
            let t = TripId::from_index(i); // ids "t00000".. sort as the rows
            let mut ties = 0;
            for (position, origin) in [(row.origin, true), (row.destination, false)] {
                let Position::Metres(p) = position else { unreachable!() };
                for (&mode, nodes) in LINK_MODES.iter().zip(&nodes) {
                    let end = if origin {
                        demand.origin_end(t, mode)
                    } else {
                        demand.destination_end(t, mode)
                    };
                    let end = end.expect("the mode is in the set");
                    let (node, gap, tie) = scan(nodes, p);
                    assert_eq!(
                        (end.node, end.gap.get()),
                        (node, gap),
                        "trip {} {} by {mode}: the index against the scan",
                        row.id,
                        if origin { "origin" } else { "destination" }
                    );
                    ties += usize::from(tie);
                }
            }
            ties
        })
        .sum();
    assert!(ties >= 20, "only {ties} ends had two nodes at the least distance");
    eprintln!("60 000 links compared, 0 mismatches; {ties} decided by the lower id");
}

#[test]
fn a_bike_end_is_its_own_nearest_node_by_the_bike_mode_set() {
    // By hand: a street a–b along y = 0 and a cycle path c–d along y = 100, each 100 m long.
    // A point at (50, 60) is 78.1 m from a and b and 64.0 m from c and d: the bike and the
    // walker take c (the lower id of the two at 64.0 m), the car a (cars may not use c–d).
    let mut b = NetworkBuilder::new();
    for (id, x, y) in [("a", 0.0, 0.0), ("b", 100.0, 0.0), ("c", 0.0, 100.0), ("d", 100.0, 100.0)] {
        b.add_node(id, at(x, y));
    }
    b.add_link("a-b", "a", "b", STREET);
    b.add_link("c-d", "c", "d", CYCLE_PATH);
    let net = b.build(common::VALUES).expect("builds");
    let modes = [Mode::Car, Mode::Bike, Mode::Walk];
    let rows = [trip("t", at(50.0, 60.0), at(50.0, 60.0), &modes)];
    let demand = Demand::build(&rows, &one_class(), &net, None, Metres(1_500.0)).expect("builds");
    let t = TripId::new(0);
    let node = |id: &str| net.node_ids().typed_id_of::<NodeId>(id).expect("a node");
    let end = |mode| demand.origin_end(t, mode).expect("in the set");
    assert_eq!(end(Mode::Car).node, node("a"));
    assert!((end(Mode::Car).gap.get() - 50.0_f64.hypot(60.0)).abs() < 1e-12);
    assert_eq!(end(Mode::Bike).node, node("c"));
    assert!((end(Mode::Bike).gap.get() - 50.0_f64.hypot(40.0)).abs() < 1e-12);
    assert_eq!(end(Mode::Walk), end(Mode::Bike));
    assert_eq!(demand.origin_end(t, Mode::Transit), None, "transit is not in the set");
}
