//! The search's arrival against every route enumerated: on five nodes with a link each way
//! between every pair, loaded with queues (300 cars in the first 600 s of a one-hour window,
//! 200 queries in its first 1 200 s, where the queues are), the earliest arrival over every
//! simple path, each walked on the reading from the departure by the query's traveller
//! ([`LinkTimes::route_time`]: link by link, carrying the car whose place the traveller took),
//! is the search's arrival. Simple paths suffice: on first-in-first-out times a route that
//! revisits a node never arrives earlier than the same route with the loop cut out (Kaufman
//! and Smith 1993). Run for one loading, and again over 40 loadings with half the queries
//! asked by a traveller of the loading, whose own car is then not in its way.
//!
//! The tree form (`reach`) against the same enumeration: over 40 loadings, 2 000 trees from a
//! random origin at a random second of the first 1 200, by no traveller of the loading, with a
//! bound of 120.02 s and again of 300.02 s. The first lies between the free-flow times of the
//! shortest and the longest link (955 m, 68.76 s; 1 990 m, 143.28 s), so that some nodes are
//! cut off at free flow and more by the queues; but below the free-flow time of every path of
//! two links (at least the two shortest links, 955 + 960 m, 137.88 s; a queue only adds), so
//! every node within it is reached over its direct link and the first bound does not test the
//! tree beyond the origin's links. The second lies above twice the longest link's free-flow
//! time (286.56 s), so every path of two links is within it at free flow, and the nodes whose
//! best path under the queues takes two links or more are reached through the tree beyond the
//! origin's links; it cuts few nodes off, which the first does. A node is reached exactly when
//! the earliest arrival over every simple path from the origin is at most the bound after the
//! departure, and its arrival is that earliest arrival; no enumerated arrival lies within
//! 1e-6 s of a bound, so the sets do not depend on rounding. Why not 120 s: every instant here
//! is on a lattice of 0.04 s. The departures are whole seconds, the headways whole seconds
//! (30, 20, 15 and 10 s), and the free-flow times multiples of 0.36 s (lengths in multiples of
//! 5 m at 50 km/h, 0.072 s a metre); an exit is an entry plus a free-flow time or the exit
//! ahead plus a headway, so every arrival less its departure is a multiple of 0.04 s, and a
//! bound of whole seconds can be met exactly (it is, twice, at 120 s). 120.02 s and 300.02 s
//! lie 0.02 s from every point of the lattice.
//!
//! For bikes: a bike reads no queue (it crosses each link in its length over its speed,
//! whatever the cars do), so its tree is its tree at free flow, judged against networkx's
//! bounded Dijkstra in `reach.rs`; the trees here are by car.

mod common;

use common::{Draws, VALUES, at, car_link, loaded, vehicle, window};
use openmobisim_graph::network::{Network, NetworkBuilder};
use openmobisim_loading::input::Vehicle;
use openmobisim_loading::link_times::{LinkTimes, Reader};
use openmobisim_search::search::{Query, Search};
use openmobisim_types::ids::{EntityId, LinkId, NodeId, TripId};
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

/// The arrival of `reader`'s car on `path` departing at `departure` (seconds since the window's
/// start): the reading's own walk of the path.
fn walk(times: &LinkTimes, path: &[LinkId], departure: f64, reader: Reader) -> f64 {
    let start = Duration(departure);
    (start + times.route_time(path, Mode::Car, start, reader)).get()
}

/// Ask 200 queries of `search` in the first 1 200 s and hold each against every simple path;
/// with `recorded`, every other query is by a traveller of the loading (a trip below 300).
/// How many arrived over 1 s later than at free flow, and how many took two links or more.
fn against_every_path(
    net: &Network,
    times: &LinkTimes,
    draws: &mut Draws,
    recorded: bool,
) -> (usize, usize) {
    let mut search = Search::new(net, times);
    let (mut delayed, mut detours) = (0, 0);
    for k in 0..200 {
        let (origin, destination) = (NodeId::new(draws.below(5)), NodeId::new(draws.below(5)));
        let departure = draws.below(1_200);
        let trip = (recorded && k % 2 == 0).then(|| TripId::new(draws.below(300)));
        let reader = trip.map_or_else(Reader::stranger, Reader::of);
        let t = f64::from(departure);
        let query = Query { origin, destination, mode: Mode::Car, departure: Duration(t), trip };
        let route = search.fastest(&query).expect("every pair is linked");
        let paths = simple_paths(net, origin, destination);
        let best = paths.iter().map(|p| walk(times, p, t, reader)).fold(f64::INFINITY, f64::min);
        let got = route.arrival.get();
        let q = format!("{origin} to {destination} at {departure} s for {trip:?}");
        assert!(
            (got - best).abs() <= 1e-9 * best,
            "{q}: the search {got} s, the best path {best} s"
        );
        let walked = walk(times, &route.links, t, reader);
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
    (delayed, detours)
}

#[test]
fn the_search_arrives_as_early_as_the_best_of_every_simple_path() {
    let net = five_nodes();
    let mut draws = Draws::new(4);
    let times = loaded(&net, &cars(&net, &mut draws), window(300, 3_600));
    let (delayed, detours) = against_every_path(&net, &times, &mut draws, false);
    // Without queues the test would not test the time dependence.
    assert!(delayed >= 20, "only {delayed} of 200 queries met a queue");
    assert!(detours >= 10, "only {detours} of 200 queries took two links or more");
    eprintln!(
        "{delayed} of 200 queries arrive over 1 s later than at free flow; {detours} take two links or more"
    );
}

#[test]
fn over_40_loadings_and_for_travellers_of_the_loading_too() {
    let net = five_nodes();
    let (mut delayed, mut detours) = (0, 0);
    for seed in 100..140 {
        let mut draws = Draws::new(seed);
        let times = loaded(&net, &cars(&net, &mut draws), window(300, 3_600));
        let (d, r) = against_every_path(&net, &times, &mut draws, true);
        delayed += d;
        detours += r;
    }
    assert!(delayed >= 800, "only {delayed} of 8 000 queries met a queue");
    assert!(detours >= 400, "only {detours} of 8 000 queries took two links or more");
    eprintln!(
        "{delayed} of 8 000 queries arrive over 1 s later than at free flow; {detours} take two links or more"
    );
}

/// The two bounds of the trees, seconds after the departure: the first cuts nodes off, the
/// second reaches over paths of two links or more (the file's header). Both half a step of the
/// 0.04 s lattice of the instants off a whole second.
const BOUNDS_S: [f64; 2] = [120.02, 300.02];

/// What the trees within one bound met: nodes reached (the origin left out), of them those
/// arriving over 1 s later than at free flow and those whose best path is not the direct link
/// (two links or more); nodes cut off, of them those within the bound at free flow; and trees
/// with a node cut off.
#[derive(Default)]
struct Trees {
    reached: usize,
    delayed: usize,
    deep: usize,
    cut: usize,
    cut_by_queue: usize,
    trees_cut: usize,
}

/// Per node, by id, from the enumeration of every simple path from the origin at the
/// departure: the earliest arrival, the earliest at free flow, and whether the direct link
/// arrives later than the earliest.
type Enumerated = Vec<(f64, f64, bool)>;

fn enumerate(net: &Network, times: &LinkTimes, origin: NodeId, t: f64) -> Enumerated {
    (0..5)
        .map(|d| {
            let paths = simple_paths(net, origin, NodeId::new(d));
            let arrive = |p: &Vec<LinkId>| walk(times, p, t, Reader::stranger());
            let best = paths.iter().map(arrive).fold(f64::INFINITY, f64::min);
            let free = paths
                .iter()
                .map(|p| t + p.iter().map(|&l| net.free_flow_time(l, Mode::Car).get()).sum::<f64>())
                .fold(f64::INFINITY, f64::min);
            let direct = paths.iter().filter(|p| p.len() == 1).map(arrive).next();
            (best, free, direct.is_some_and(|a| a > best))
        })
        .collect()
}

/// Hold one tree, `got`, from `origin` at second `t` within `bound`, against `enumerated`.
fn check_tree(
    got: &[(NodeId, Duration)],
    (origin, t, bound): (NodeId, f64, f64),
    enumerated: &Enumerated,
    out: &mut Trees,
) {
    assert!(got.windows(2).all(|w| w[0].0 < w[1].0), "sorted by node id");
    let (mut within, mut cut_here) = (0, false);
    for (d, &(best, free, deep)) in (0_u32..).zip(enumerated) {
        let node = NodeId::new(d);
        let q = format!("from {origin} at {t} s to {node} within {bound} s");
        assert!(
            (best - t - bound).abs() > 1e-6,
            "{q}: the best path arrives {} s after the departure, within 1e-6 s of the bound",
            best - t
        );
        let found = got.iter().find(|x| x.0 == node).map(|x| x.1.get());
        if best - t <= bound {
            within += 1;
            let at = found.unwrap_or_else(|| panic!("{q}: the best path at {best} s, not reached"));
            assert!((at - best).abs() <= 1e-9 * best, "{q}: reach {at} s, the best path {best} s");
            if node != origin {
                out.reached += 1;
                out.delayed += usize::from(best > free + 1.0);
                out.deep += usize::from(deep);
            }
        } else {
            assert!(found.is_none(), "{q}: reached at {found:?}, the best path at {best} s");
            out.cut += 1;
            out.cut_by_queue += usize::from(free - t <= bound);
            cut_here = true;
        }
    }
    assert_eq!(got.len(), within, "from {origin} at {t} s within {bound} s: no other node reached");
    out.trees_cut += usize::from(cut_here);
}

/// Ask 50 pairs of an origin and a second of the first 1 200, drawn at random, by no traveller
/// of the loading, and hold the tree within each bound against every simple path.
fn trees_against_every_path(
    net: &Network,
    times: &LinkTimes,
    draws: &mut Draws,
    out: &mut [Trees; 2],
) {
    let mut search = Search::new(net, times);
    for _ in 0..50 {
        let origin = NodeId::new(draws.below(5));
        let departure = draws.below(1_200);
        let t = f64::from(departure);
        let enumerated = enumerate(net, times, origin, t);
        for (bound, trees) in BOUNDS_S.iter().zip(out.iter_mut()) {
            let got = search.reach(origin, Mode::Car, Second(departure), Duration(*bound));
            check_tree(&got, (origin, t, *bound), &enumerated, trees);
        }
    }
}

#[test]
fn a_tree_reaches_every_node_whose_best_path_is_within_the_bound_at_that_arrival() {
    let net = five_nodes();
    let mut trees = [Trees::default(), Trees::default()];
    for seed in 200..240 {
        let mut draws = Draws::new(seed);
        let times = loaded(&net, &cars(&net, &mut draws), window(300, 3_600));
        trees_against_every_path(&net, &times, &mut draws, &mut trees);
    }
    for (bound, t) in BOUNDS_S.iter().zip(&trees) {
        eprintln!(
            "2 000 trees within {bound} s: {} nodes reached besides the origin, {} of them over 1 s later than at free flow, {} by a best path of two links or more; {} nodes cut off, {} of them within the bound at free flow; {} trees with a node cut off",
            t.reached, t.delayed, t.deep, t.cut, t.cut_by_queue, t.trees_cut
        );
    }
    // Guards, not expected values: without nodes on both sides of the first bound, without
    // queues, and without best paths of two links or more within the second, the test would
    // not test. At least a quarter of the 8 000 nodes besides the origins on each side.
    let [first, second] = &trees;
    assert!(first.reached >= 2_000, "only {} nodes reached besides the origin", first.reached);
    assert!(first.cut >= 2_000, "only {} nodes cut off", first.cut);
    assert!(first.trees_cut >= 1_000, "only {} of 2 000 trees cut a node off", first.trees_cut);
    assert!(first.delayed >= 200, "only {} reached nodes met a queue", first.delayed);
    assert!(first.cut_by_queue >= 50, "only {} nodes cut off by a queue", first.cut_by_queue);
    assert!(second.deep >= 400, "only {} nodes reached by two links or more", second.deep);
}
