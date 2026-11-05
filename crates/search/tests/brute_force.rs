//! The search's arrival against every route enumerated: on five nodes with a link each way
//! between every pair, loaded with queues (300 cars in the first 600 s of a one-hour window,
//! 200 queries in its first 1 200 s, where the queues are), the earliest arrival over every simple path, each
//! walked link by link on the reading from the departure, is the search's arrival. Simple paths
//! suffice: on first-in-first-out times a route that revisits a node never arrives earlier than
//! the same route with the loop cut out (Kaufman and Smith 1993).

mod common;

use common::{Draws, VALUES, at, car_link, loaded, vehicle, window};
use openmobisim_graph::network::{Network, NetworkBuilder};
use openmobisim_loading::input::Vehicle;
use openmobisim_loading::link_times::LinkTimes;
use openmobisim_search::search::{Query, Search};
use openmobisim_types::ids::{EntityId, LinkId, NodeId};
use openmobisim_types::mode::Mode;
use openmobisim_types::time::Second;
use openmobisim_types::units::Duration;

/// The five nodes, in metres.
const NODES: [(f64, f64); 5] =
    [(0.0, 0.0), (1_000.0, 0.0), (1_300.0, 900.0), (500.0, 1_500.0), (-300.0, 900.0)];

/// Per pair of nodes `(i, j)`, `i < j`: the length from `i` to `j` and from `j` to `i`, in
/// metres, all twenty distinct, chosen by hand between 0.95 and 1.29 times the straight line
/// (the link from 3 to 1 below it, which lowers the search's bound and must not mislead it).
const LENGTHS: [(usize, usize, f64, f64); 10] = [
    (0, 1, 1_010.0, 1_130.0),
    (0, 2, 1_620.0, 1_745.0),
    (0, 3, 1_590.0, 1_880.0),
    (0, 4, 960.0, 1_075.0),
    (1, 2, 955.0, 1_210.0),
    (1, 3, 1_650.0, 1_505.0),
    (1, 4, 1_700.0, 1_820.0),
    (2, 3, 1_040.0, 1_165.0),
    (2, 4, 1_615.0, 1_990.0),
    (3, 4, 1_025.0, 1_290.0),
];

/// The capacities, in veh/h, taken in turn by the links in the order of [`LENGTHS`].
const CAPACITIES: [f64; 4] = [120.0, 180.0, 240.0, 360.0];

fn five_nodes() -> Network {
    let mut b = NetworkBuilder::new();
    for (i, &(x, y)) in NODES.iter().enumerate() {
        b.add_node(format!("v{i}"), at(x, y));
    }
    for (k, &(i, j, there, back)) in LENGTHS.iter().enumerate() {
        b.add_link(
            format!("e{i}{j}"),
            format!("v{i}"),
            format!("v{j}"),
            car_link(there, CAPACITIES[(2 * k) % 4]),
        );
        b.add_link(
            format!("e{j}{i}"),
            format!("v{j}"),
            format!("v{i}"),
            car_link(back, CAPACITIES[(2 * k + 1) % 4]),
        );
    }
    b.build(VALUES).expect("five nodes build")
}

/// 300 cars, each on a random simple path of 1 to 4 links from a random node, departing at a
/// random second of the first 600.
fn cars(net: &Network, draws: &mut Draws) -> Vec<Vehicle> {
    (0..300)
        .map(|trip| {
            let mut node = NodeId::new(draws.below(5));
            let mut seen = vec![node];
            let mut route = Vec::new();
            for _ in 0..=draws.below(4) {
                let next: Vec<LinkId> = net
                    .out_links(node)
                    .iter()
                    .copied()
                    .filter(|&l| !seen.contains(&net.downstream(l)))
                    .collect();
                let l = next[draws.below(u32::try_from(next.len()).unwrap()) as usize];
                route.push(l);
                node = net.downstream(l);
                seen.push(node);
            }
            vehicle(trip, Mode::Car, &route, draws.below(600))
        })
        .collect()
}

/// Every simple path from `from` to `to`, as links (one empty path when they are equal).
fn simple_paths(net: &Network, from: NodeId, to: NodeId) -> Vec<Vec<LinkId>> {
    fn grow(
        net: &Network,
        to: NodeId,
        path: &mut Vec<LinkId>,
        seen: &mut Vec<NodeId>,
        out: &mut Vec<Vec<LinkId>>,
    ) {
        let here = *seen.last().expect("the origin");
        if here == to {
            out.push(path.clone());
            return;
        }
        for &l in net.out_links(here) {
            if !seen.contains(&net.downstream(l)) {
                path.push(l);
                seen.push(net.downstream(l));
                grow(net, to, path, seen, out);
                seen.pop();
                path.pop();
            }
        }
    }
    let mut out = Vec::new();
    grow(net, to, &mut Vec::new(), &mut vec![from], &mut out);
    out
}

/// The arrival of a car on `path` departing at `departure` (seconds since the window's start),
/// each link entered when the one before it ends.
fn walk(times: &LinkTimes, path: &[LinkId], departure: f64) -> f64 {
    path.iter().fold(departure, |t, &l| t + times.time(l, Mode::Car, Duration(t)).get())
}

#[test]
fn the_search_arrives_as_early_as_the_best_of_every_simple_path() {
    let net = five_nodes();
    let mut draws = Draws::new(4);
    let times = loaded(&net, &cars(&net, &mut draws), window(300, 3_600));
    let mut search = Search::new(&net, &times);
    let (mut delayed, mut detours) = (0, 0);
    for _ in 0..200 {
        let (origin, destination) = (NodeId::new(draws.below(5)), NodeId::new(draws.below(5)));
        let departure = draws.below(1_200);
        let query = Query { origin, destination, mode: Mode::Car, departure: Second(departure) };
        let route = search.fastest(&query).expect("every pair is linked");
        let t = f64::from(departure);
        let paths = simple_paths(&net, origin, destination);
        let best = paths.iter().map(|p| walk(&times, p, t)).fold(f64::INFINITY, f64::min);
        let got = route.arrival.get();
        let q = format!("{origin} to {destination} at {departure} s");
        assert!(
            (got - best).abs() <= 1e-9 * best,
            "{q}: the search {got} s, the best path {best} s"
        );
        let walked = walk(&times, &route.links, t);
        assert_eq!(got.to_bits(), walked.to_bits(), "{q}: reported {got} s, walked {walked} s");
        // How often the queues mattered: the arrival over 1 s later than at free flow, and the
        // routes of two links or more (on these lengths the direct link is always the fastest
        // at free flow: only a queue makes a longer route the faster).
        let free = paths
            .iter()
            .map(|p| t + p.iter().map(|&l| net.free_flow_time(l, Mode::Car).get()).sum::<f64>())
            .fold(f64::INFINITY, f64::min);
        delayed += usize::from(got > free + 1.0);
        detours += usize::from(route.links.len() > 1);
    }
    // Without queues the test would not test the time dependence.
    assert!(delayed >= 20, "only {delayed} of 200 queries met a queue");
    assert!(detours >= 10, "only {detours} of 200 queries took two links or more");
    eprintln!(
        "{delayed} of 200 queries arrive over 1 s later than at free flow; {detours} take two links or more"
    );
}
