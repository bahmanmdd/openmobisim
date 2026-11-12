//! A trip end joins the network where its trip can use it: an origin at the nearest node with
//! an out-link its mode may use, a destination at the nearest with an in-link; two ends nearest
//! the same node both join it. By hand on one-way footways along the x axis.
//!
//! For bikes: the same rule on the links a bike may use (the cycle-path case below).

mod common;

use common::{CYCLE_PATH, FOOTWAY, STREET, at, one_class, trip};
use openmobisim_demand::demand::Demand;
use openmobisim_graph::network::{LinkRow, Network, NetworkBuilder};
use openmobisim_types::ids::{EntityId, TripId};
use openmobisim_types::mode::Mode;
use openmobisim_types::units::Metres;

/// Nodes on the x axis at `nodes` (id, x in metres) and one-way links `(from, to, row)`.
fn axis(nodes: &[(&str, f64)], links: &[(&str, &str, LinkRow)]) -> Network {
    let mut b = NetworkBuilder::new();
    for &(id, x) in nodes {
        b.add_node(id, at(x, 0.0));
    }
    for &(from, to, row) in links {
        b.add_link(format!("{from}-{to}"), from, to, row);
    }
    b.build(common::VALUES).expect("a network")
}

/// The node and the gap of trip 0's two ends for `mode`, as (origin id, gap, destination id, gap).
fn ends(net: &Network, dem: &Demand, mode: Mode) -> (String, f64, String, f64) {
    let name = |n| net.node_ids().external(EntityId::raw(n)).to_owned();
    let o = dem.origin_end(TripId::new(0), mode).expect("an origin end");
    let d = dem.destination_end(TripId::new(0), mode).expect("a destination end");
    (name(o.node), o.gap.get(), name(d.node), d.gap.get())
}

#[test]
fn an_origin_beside_a_dead_end_joins_the_nearest_node_it_can_leave() {
    // S at 0 m, A at 1 m, D at 100 m; footways A→S and A→D. S has no way out. A walker from
    // 0 m to 100 m joins at A, 1 m away, and ends at D.
    let net =
        axis(&[("S", 0.0), ("A", 1.0), ("D", 100.0)], &[("A", "S", FOOTWAY), ("A", "D", FOOTWAY)]);
    let rows = [trip("t", at(0.0, 0.0), at(100.0, 0.0), &[Mode::Walk])];
    let dem = Demand::build(&rows, &one_class(), &net, None, Metres(1_500.0)).expect("a demand");
    assert_eq!(ends(&net, &dem, Mode::Walk), ("A".to_owned(), 1.0, "D".to_owned(), 0.0));
    // A destination at 0 m ends at S, which a walker can arrive at.
    let rows = [trip("t", at(1.0, 0.0), at(0.0, 0.0), &[Mode::Walk])];
    let dem = Demand::build(&rows, &one_class(), &net, None, Metres(1_500.0)).expect("a demand");
    assert_eq!(ends(&net, &dem, Mode::Walk), ("A".to_owned(), 0.0, "S".to_owned(), 0.0));
}

#[test]
fn a_destination_beside_a_node_nothing_enters_joins_the_nearest_node_it_can_arrive_at() {
    // O at 0 m, A at 99 m, S at 100 m; footways O→A and S→A. Nothing enters S. A walker from
    // 0 m to 100 m ends at A, 1 m short.
    let net =
        axis(&[("O", 0.0), ("A", 99.0), ("S", 100.0)], &[("O", "A", FOOTWAY), ("S", "A", FOOTWAY)]);
    let rows = [trip("t", at(0.0, 0.0), at(100.0, 0.0), &[Mode::Walk])];
    let dem = Demand::build(&rows, &one_class(), &net, None, Metres(1_500.0)).expect("a demand");
    assert_eq!(ends(&net, &dem, Mode::Walk), ("O".to_owned(), 0.0, "A".to_owned(), 1.0));
}

#[test]
fn each_mode_joins_by_its_own_links_and_ends_nearest_one_node_both_join_it() {
    // A street both ways between M (0 m) and N (50 m); a one-way cycle path N→P (P at 60 m).
    // A trip from 53 m to 58 m: by car both ends are nearest N (3 and 8 m). By bike the origin
    // is nearest N and the destination nearest P, two nodes: the origin joins N, which a bike
    // can leave, and the destination P, which a bike can arrive at (2 m).
    let net = axis(
        &[("M", 0.0), ("N", 50.0), ("P", 60.0)],
        &[("M", "N", STREET), ("N", "M", STREET), ("N", "P", CYCLE_PATH)],
    );
    let rows = [trip("t", at(53.0, 0.0), at(58.0, 0.0), &[Mode::Car, Mode::Bike])];
    let dem = Demand::build(&rows, &one_class(), &net, None, Metres(1_500.0)).expect("a demand");
    assert_eq!(ends(&net, &dem, Mode::Car), ("N".to_owned(), 3.0, "N".to_owned(), 8.0));
    assert_eq!(ends(&net, &dem, Mode::Bike), ("N".to_owned(), 3.0, "P".to_owned(), 2.0));
    // Both ends nearest P, where no bike can set out: both join P and the trip rides no link.
    let rows = [trip("t", at(58.0, 0.0), at(59.0, 0.0), &[Mode::Bike])];
    let dem = Demand::build(&rows, &one_class(), &net, None, Metres(1_500.0)).expect("a demand");
    assert_eq!(ends(&net, &dem, Mode::Bike), ("P".to_owned(), 2.0, "P".to_owned(), 1.0));
}
