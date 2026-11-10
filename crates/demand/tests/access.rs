//! Test 2, the reach and the lists against networkx (Hagberg, Schult and Swart 2008, networkx
//! 3.7): every trip's access and egress stops and their walking seconds equal networkx's
//! bounded Dijkstra by length plus the two gaps, over walking speed.
//!
//! The fixture `fixtures/networkx_access.txt` is written by `networkx_access.py` (beside this
//! file, not run in CI): a 20 × 20 grid of nodes 100 m apart, each moved by up to 20 m, a link
//! each way between neighbours whose length is the straight line rounded to 0.01 m, walking
//! allowed one way only on 10 % of the streets; 6 stops and 25 trips (50 ends) at random points
//! off the nodes, each linked to its nearest walk node by a full scan; per trip the stops whose
//! whole walk (gap, network, gap) is at most 600 m, each way, with the walk's metres. No walk
//! is within 0.01 m of 600 m (checked by the script). A walker covers a metre in 1 / (4.8 / 3.6)
//! seconds.

mod common;

use std::collections::BTreeMap;

use common::{VALUES, WALK_M_S, at, one_class, trip};
use openmobisim_demand::demand::Demand;
use openmobisim_demand::input::TripRow;
use openmobisim_graph::network::{LinkRow, Network, NetworkBuilder};
use openmobisim_transit::timetable::{Timetable, TimetableBuilder};
use openmobisim_types::ids::{EntityId, NodeId, StopId, TripId};
use openmobisim_types::mode::{Mode, ModeSet};
use openmobisim_types::units::{Duration, Metres};

const FIXTURE: &str = include_str!("fixtures/networkx_access.txt");

/// What the fixture expects of one trip: its nodes and gaps, and its two lists by stop name.
#[derive(Default)]
struct Expected {
    ends: [(String, f64); 2],
    lists: [Vec<(String, f64)>; 2],
}

/// The fixture: the network, the stops, the trip rows, per stop its node and gap, and per trip
/// what it expects.
struct Fixture {
    net: Network,
    stops: Timetable,
    rows: Vec<TripRow>,
    stop_ends: BTreeMap<String, (String, f64)>,
    expected: BTreeMap<String, Expected>,
}

fn read() -> Fixture {
    let (mut net, mut tt) = (NetworkBuilder::new(), TimetableBuilder::new());
    let (mut rows, mut stops, mut expected) = (Vec::new(), BTreeMap::new(), BTreeMap::new());
    for line in FIXTURE.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        let num = |i: usize| f[i].parse::<f64>().expect("a number");
        match f[0] {
            "node" => net.add_node(f[1], at(num(2), num(3))),
            "link" => {
                let mut modes = ModeSet::of(&[Mode::Car, Mode::Bike]);
                if f[5] == "cbw" {
                    modes = modes.with(Mode::Walk);
                }
                let row = LinkRow { modes, length_m: Some(num(4)), ..common::STREET };
                net.add_link(format!("l{}", f[1]), f[2], f[3], row);
            }
            "stop" => {
                tt.add_stop(f[1], (Metres(num(2)), Metres(num(3))));
                stops.insert(f[1].to_owned(), (f[4].to_owned(), num(5)));
            }
            "trip" => {
                let modes = [Mode::Transit, Mode::Walk];
                rows.push(trip(f[1], at(num(2), num(3)), at(num(4), num(5)), &modes));
                let e: &mut Expected = expected.entry(f[1].to_owned()).or_default();
                e.ends = [(f[6].to_owned(), num(7)), (f[8].to_owned(), num(9))];
            }
            "access" | "egress" => {
                let e = expected.get_mut(f[1]).expect("the trip comes first");
                e.lists[usize::from(f[0] == "egress")] = f[2..]
                    .iter()
                    .map(|s| {
                        let (stop, metres) = s.split_once(':').expect("stop:metres");
                        (stop.to_owned(), metres.parse().expect("metres"))
                    })
                    .collect();
            }
            other => panic!("unknown record {other}"),
        }
    }
    Fixture {
        net: net.build(VALUES).expect("the fixture builds"),
        stops: tt.build().expect("the stops build"),
        rows,
        stop_ends: stops,
        expected,
    }
}

/// Whether `seconds` is `metres` at walking speed, to a relative 1e-9 (a few sums of rounded
/// link times and gaps).
fn walks(seconds: Duration, metres: f64) -> bool {
    let by_hand = metres / WALK_M_S;
    (seconds.get() - by_hand).abs() <= 1e-9 * by_hand.max(1.0)
}

#[test]
fn access_and_egress_lists_are_networkx_bounded_dijkstra_plus_the_gaps() {
    let f = read();
    assert_eq!((f.net.node_count(), f.stops.stop_count(), f.rows.len()), (400, 6, 25));
    let demand = Demand::build(&f.rows, &one_class(), &f.net, Some(&f.stops), Metres(600.0))
        .expect("builds");
    let node = |n: NodeId| f.net.node_ids().external(n.raw()).to_owned();
    for s in StopId::iter_space(demand.stop_count()) {
        let (name, end) = (f.stops.stop_ids().external(s.raw()), demand.stop_end(s));
        let (at_node, gap) = &f.stop_ends[name];
        assert_eq!(&node(end.node), at_node, "stop {name}'s node");
        assert!((end.gap.get() - gap).abs() <= 1e-9 * gap, "stop {name}'s gap");
    }
    let (mut empty, mut entries) = ([0, 0], 0);
    for t in TripId::iter_space(demand.trip_count()) {
        let name = demand.trip_ids().external(t.raw());
        let e = &f.expected[name];
        let ends = [demand.origin_end(t, Mode::Transit), demand.destination_end(t, Mode::Transit)];
        let lists = [demand.access(t), demand.egress(t)];
        for side in 0..2 {
            let what = format!("trip {name} {}", ["access", "egress"][side]);
            let end = ends[side].expect("transit is in the set");
            assert_eq!(node(end.node), e.ends[side].0, "{what}: the walk node");
            assert!((end.gap.get() - e.ends[side].1).abs() <= 1e-9 * e.ends[side].1, "{what}: gap");
            assert_eq!(
                Some(end),
                [demand.origin_end(t, Mode::Walk), demand.destination_end(t, Mode::Walk)][side],
                "{what}: transit links by walking"
            );
            let got: Vec<&str> =
                lists[side].iter().map(|(s, _)| f.stops.stop_ids().external(s.raw())).collect();
            let want: Vec<&str> = e.lists[side].iter().map(|(s, _)| s.as_str()).collect();
            assert_eq!(got, want, "{what}: the stops (by id, and the names sort as the ids)");
            for ((_, seconds), (stop, metres)) in lists[side].iter().zip(&e.lists[side]) {
                assert!(walks(*seconds, *metres), "{what} {stop}: {seconds} s for {metres} m");
            }
            empty[side] += usize::from(lists[side].is_empty());
            entries += lists[side].len();
        }
    }
    assert_eq!(
        (demand.origins_without_stop() as usize, demand.destinations_without_stop() as usize),
        (empty[0], empty[1]),
        "the ends without a stop in reach, counted"
    );
    eprintln!(
        "{entries} list entries equal; ends without a stop: {} origins, {} destinations",
        empty[0], empty[1]
    );
}
