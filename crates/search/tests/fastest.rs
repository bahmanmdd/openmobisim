//! The fastest route against hand derivations: a free-flow car detour on a grid (with the tie
//! rule decided by hand), two routes arriving at the same instant by car, bike and on foot
//! whichever is found first, the switch between two routes as a queue grows (Vickrey 1969), the
//! contract's edge cases (origin equal to destination, a mode a link forbids, transit), and a
//! batch of queries answered each in its place, a batch of one and a batch of none.
//!
//! Every car link is one lane at 50 km/h unless said otherwise: 100 m take 100 / (50 / 3.6) =
//! 7.2 s and 1 000 m take 72 s; at 20 km/h 100 m take 18 s; a bike rides 100 m at 15 km/h in
//! 24 s and 1 000 m in 240 s.

mod common;

use common::{
    VALUES, at, car_link, free_flow, grid_link, grid_node, grid_with, loaded, row, vehicle, window,
};
use openmobisim_graph::network::{LinkRow, Network, NetworkBuilder};
use openmobisim_loading::link_times::{LinkTimes, Reader};
use openmobisim_search::search::{Query, Route, Search, fastest_many};
use openmobisim_types::ids::{EntityId, LinkId, NodeId, TripId};
use openmobisim_types::mode::{Mode, ModeSet};
use openmobisim_types::units::Duration;

/// Whether `a` and `b` agree to a relative 1e-12 (a few sums of rounded link times).
fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-12 * b.abs().max(1.0)
}

/// The fastest route by `mode` from `origin` to `destination` setting out `departure` seconds
/// after the window's start, for a traveller with no car in the loading.
fn fastest(
    net: &Network,
    times: &LinkTimes,
    ends: (NodeId, NodeId),
    mode: Mode,
    departure: u32,
) -> Option<Route> {
    fastest_for(net, times, ends, mode, (departure, None))
}

/// [`fastest`] for the traveller of trip `car` (`None`: one with no car in the loading).
fn fastest_for(
    net: &Network,
    times: &LinkTimes,
    (origin, destination): (NodeId, NodeId),
    mode: Mode,
    (departure, car): (u32, Option<u32>),
) -> Option<Route> {
    Search::new(net, times).fastest(&Query {
        origin,
        destination,
        mode,
        departure: Duration(f64::from(departure)),
        trip: car.map(TripId::new),
    })
}

/// A 5 × 5 grid (four blocks a side, so row 0 has four links each way) whose middle two links
/// of row 0, between columns 1 and 2 and between columns 2 and 3, both ways, are at 20 km/h.
fn slow_middle() -> Network {
    grid_with(5, |a, z| {
        let middle = a.0 == 0 && z.0 == 0 && a.1.min(z.1) >= 1 && a.1.max(z.1) <= 3;
        row(1, if middle { 20.0 } else { 50.0 })
    })
}

#[test]
fn a_free_flow_car_detours_one_row_down_round_two_slow_links() {
    // By hand, from (0, 0) to (0, 4) at free flow. Along row 0: 2 × 7.2 + 2 × 18 = 50.4 s.
    // Avoiding both slow links: leave row 0 at column 0 or 1, return at column 3 or 4, at least
    // 4 links east, 1 down and 1 up, 6 × 7.2 = 43.2 s. Using exactly one slow link costs at
    // least 5 × 7.2 + 18 = 54 s. So 43.2 s, by four routes of 600 m and 6 links arriving at the
    // same instant (the same six additions of 7.2 s). The tie rule, by hand: same metres, same
    // links, so the lower id of the link arriving decides. Ids follow sorted external ids
    // (`from-to`): into (0, 4), "n0_3-n0_4" < "n1_4-n0_4"; (0, 3) is reached at 36 s only from
    // (1, 3) (from (0, 2) it is 25.2 + 18 = 43.2 s), (1, 3) only from (1, 2), (1, 2) only from
    // (1, 1); into (1, 1) at 14.4 s, "n0_1-n1_1" < "n1_0-n1_1". The route is therefore
    // (0,0) (0,1) (1,1) (1,2) (1,3) (0,3) (0,4).
    let net = slow_middle();
    let times = free_flow(&net);
    let ends = (grid_node(&net, 0, 0), grid_node(&net, 0, 4));
    let car = fastest(&net, &times, ends, Mode::Car, 0).expect("the grid is connected");
    let nodes = [(0, 0), (0, 1), (1, 1), (1, 2), (1, 3), (0, 3), (0, 4)];
    let by_hand: Vec<LinkId> = nodes.windows(2).map(|w| grid_link(&net, w[0], w[1])).collect();
    assert_eq!(car.links, by_hand, "the car's route");
    assert!(
        close(car.arrival.get(), 43.2),
        "the car arrives at {} s, by hand 43.2 s",
        car.arrival.get()
    );
    // A bike rides at 15 km/h on every link, whatever the car speed: the direct 400 m, 96 s.
    let bike = fastest(&net, &times, ends, Mode::Bike, 0).expect("the grid is connected");
    let direct: Vec<LinkId> = (0..4).map(|c| grid_link(&net, (0, c), (0, c + 1))).collect();
    assert_eq!(bike.links, direct, "the bike's route");
    assert!(
        close(bike.arrival.get(), 96.0),
        "the bike arrives at {} s, by hand 96 s",
        bike.arrival.get()
    );
}

/// Two routes of two links from `o` to `d`, by `x` and by `y`, every link 150 m: the links from
/// `o` to `x`, `o` to `y`, `x` to `d` and `y` to `d` under the names given.
fn diamond(names: [&str; 4]) -> Network {
    let mut b = NetworkBuilder::new();
    for (node, x, y) in
        [("o", 0.0, 0.0), ("x", 100.0, 100.0), ("y", 100.0, -100.0), ("d", 200.0, 0.0)]
    {
        b.add_node(node, at(x, y));
    }
    for (name, (from, to)) in
        names.into_iter().zip([("o", "x"), ("o", "y"), ("x", "d"), ("y", "d")])
    {
        b.add_link(name, from, to, car_link(150.0, 1_800.0));
    }
    b.build(VALUES).expect("the diamond builds")
}

#[test]
fn at_an_equal_instant_the_last_link_of_lower_id_wins_whichever_route_is_found_first() {
    // By hand, at free flow from second 0, every link 150 m: a car at 50 km/h takes 10.8 s a
    // link and arrives at 21.6 s, a bike at 15 km/h 36 s and 72 s, a walker at 4.8 km/h 112.5 s
    // and 225 s. For each mode both routes arrive after the same two additions, so at the same
    // instant to the bit, over 300 m and two links. The tie rule's last step decides: the route
    // whose last link has the lower id, and ids follow the sorted names. With the lower name on
    // the link from `x`, the route is by `x`; with it on the link from `y`, by `y`. `x` and `y`
    // are reached at the same instant, and whichever of them the search goes on from first, in
    // one of the two networks the winning link is the second it comes to, when `d` already holds
    // the same instant by the other.
    for (names, by_hand) in [
        (["l1_ox", "l2_oy", "l3_xd", "l4_yd"], ["l1_ox", "l3_xd"]),
        (["l1_ox", "l2_oy", "l4_xd", "l3_yd"], ["l2_oy", "l3_yd"]),
    ] {
        let net = diamond(names);
        let times = free_flow(&net);
        let node = |id| net.node_ids().typed_id_of::<NodeId>(id).expect("a node");
        for (mode, arrival) in [(Mode::Car, 21.6), (Mode::Bike, 72.0), (Mode::Walk, 225.0)] {
            let route = fastest(&net, &times, (node("o"), node("d")), mode, 0).expect("connected");
            let taken: Vec<&str> =
                route.links.iter().map(|l| net.link_ids().external(l.raw())).collect();
            assert_eq!(taken, by_hand, "{mode}, the links named {names:?}");
            let got = route.arrival.get();
            assert!(close(got, arrival), "{mode}: arrival {got} s, by hand {arrival} s");
        }
    }
}

/// Nodes `a` (0, 0), `b` (1 000, 0) and `c` (500, 500); the short route `s` from `a` to `b`,
/// 1 000 m at 1 200 veh/h; the long route `l1` (`a` to `c`) then `l2` (`c` to `b`), each 1 000 m
/// with unlimited capacity. The network and the ids of `s`, `l1`, `l2`, `a`, `b`.
fn two_routes() -> (Network, [LinkId; 3], [NodeId; 2]) {
    let mut b = NetworkBuilder::new();
    for (node, x, y) in [("a", 0.0, 0.0), ("b", 1_000.0, 0.0), ("c", 500.0, 500.0)] {
        b.add_node(node, at(x, y));
    }
    b.add_link("s", "a", "b", car_link(1_000.0, 1_200.0));
    b.add_link("l1", "a", "c", car_link(1_000.0, f64::INFINITY));
    b.add_link("l2", "c", "b", car_link(1_000.0, f64::INFINITY));
    let net = b.build(VALUES).expect("two routes build");
    let link = |id| net.link_ids().typed_id_of(id).expect("a link");
    let node = |id| net.node_ids().typed_id_of(id).expect("a node");
    let ids = [link("s"), link("l1"), link("l2")];
    let ends = [node("a"), node("b")];
    (net, ids, ends)
}

#[test]
fn a_car_takes_the_short_link_until_its_queue_costs_more_than_the_long_way() {
    // Vickrey (1969), as in the loading's tests: 300 cars enter `s` every 2 s from second 0;
    // car k enters at t = 2k and leaves at 72 + 3k (a 3 s headway), a time of 72 + t/2. Each
    // search below is by the car that entered `s` at that instant, which reads its own time
    // there. The long route takes 72 + 72 = 144 s at any instant. The switch: 72 + t/2 = 144
    // at t = 144 s. By hand:
    //   car   0, departure   0: `s` (its own 72 s < 144 s), arrival 72 s;
    //   car  50, departure 100: `s` (its own 122 s < 144 s), arrival 222 s;
    //   car 150, departure 300: `l1`, `l2` (its own time on `s` is 222 s > 144 s), arrival 444 s.
    let (net, [s, l1, l2], [a, b]) = two_routes();
    let cars: Vec<_> = (0..300).map(|k| vehicle(k, Mode::Car, &[s], 2 * k)).collect();
    let times = loaded(&net, &cars, window(300, 3_600));
    for (departure, k, route, arrival) in
        [(0, 0, vec![s], 72.0), (100, 50, vec![s], 222.0), (300, 150, vec![l1, l2], 444.0)]
    {
        let car =
            fastest_for(&net, &times, (a, b), Mode::Car, (departure, Some(k))).expect("connected");
        assert_eq!(car.links, route, "departing at {departure} s");
        let got = car.arrival.get();
        assert!(
            close(got, arrival),
            "departing at {departure} s: arrival {got} s, by hand {arrival} s"
        );
        // The arrival is the reading's own walk of the route, to the bit.
        let start = Duration(f64::from(departure));
        let reader = Reader::of(TripId::new(k));
        let walked = (start + times.route_time(&car.links, Mode::Car, start, reader)).get();
        assert_eq!(
            got.to_bits(),
            walked.to_bits(),
            "departing at {departure} s: {got} s, walked {walked} s"
        );
    }
    // The queue holds no bike: at 300 s it rides `s`, 1 000 m at 15 km/h, 240 s.
    let bike = fastest(&net, &times, (a, b), Mode::Bike, 300).expect("connected");
    assert_eq!(bike.links, vec![s]);
    assert!(
        close(bike.arrival.get(), 540.0),
        "the bike arrives at {} s, by hand 540 s",
        bike.arrival.get()
    );
}

#[test]
fn from_a_node_to_itself_the_route_is_empty_and_arrives_at_the_departure() {
    let (net, _, [a, _]) = two_routes();
    let times = free_flow(&net);
    for mode in [Mode::Car, Mode::Bike, Mode::Walk] {
        let route = fastest(&net, &times, (a, a), mode, 1_234).expect("a node reaches itself");
        assert!(route.links.is_empty(), "{mode}: {:?}", route.links);
        assert_eq!(route.arrival.get().to_bits(), 1_234.0_f64.to_bits(), "{mode}");
    }
}

#[test]
fn a_link_that_forbids_the_mode_is_never_taken() {
    // `a` to `b` by a cycle path (bike and walk, 1 000 m) or by a street through `c` (two
    // 1 000 m car links). By hand: a car must go round, 144 s; a bike rides the path, 240 s.
    // From `b` back to `a` there is only the cycle path's reverse: no car route at all.
    let path = LinkRow {
        modes: ModeSet::of(&[Mode::Bike, Mode::Walk]),
        lanes: None,
        car_speed_km_h: None,
        length_m: Some(1_000.0),
        capacity_veh_h: None,
        signal_at_downstream_end: false,
    };
    let mut builder = NetworkBuilder::new();
    for (node, x, y) in [("a", 0.0, 0.0), ("b", 1_000.0, 0.0), ("c", 500.0, 500.0)] {
        builder.add_node(node, at(x, y));
    }
    builder.add_link("p", "a", "b", path);
    builder.add_link("q", "b", "a", path);
    builder.add_link("l1", "a", "c", car_link(1_000.0, 1_800.0));
    builder.add_link("l2", "c", "b", car_link(1_000.0, 1_800.0));
    let net = builder.build(VALUES).expect("builds");
    let times = free_flow(&net);
    let node = |id| net.node_ids().typed_id_of::<NodeId>(id).expect("a node");
    let (a, b) = (node("a"), node("b"));
    let car = fastest(&net, &times, (a, b), Mode::Car, 0).expect("round by c");
    assert_eq!(car.links.len(), 2, "the car goes round: {:?}", car.links);
    assert!(
        close(car.arrival.get(), 144.0),
        "the car arrives at {} s, by hand 144 s",
        car.arrival.get()
    );
    let bike = fastest(&net, &times, (a, b), Mode::Bike, 0).expect("the path");
    assert!(
        close(bike.arrival.get(), 240.0),
        "the bike arrives at {} s, by hand 240 s",
        bike.arrival.get()
    );
    assert_eq!(fastest(&net, &times, (b, a), Mode::Car, 0), None, "no car link leads back");
    assert!(fastest(&net, &times, (b, a), Mode::Walk, 0).is_some(), "a walker takes the path");
}

#[test]
#[should_panic(expected = "transit is not on links")]
fn transit_is_refused_as_the_reading_refuses_it() {
    let (net, _, [a, b]) = two_routes();
    let times = free_flow(&net);
    let _ = fastest(&net, &times, (a, b), Mode::Transit, 0);
}

/// An answer by hand: the route's links and its arrival in seconds, or no route.
type ByHand = Option<(Vec<LinkId>, f64)>;

/// A loading, six queries on it and their answers by hand, as the tests above derive them: 300
/// cars enter `s` every 2 s, so car 0 leaving at second 0 takes `s` and arrives at 72 s, car 50
/// at second 100 takes `s` and arrives at 222 s, car 150 at second 300 goes round by `l1` and
/// `l2` and arrives at 444 s; a bike at second 300 rides `s` and arrives at 540 s; no car link
/// leads from `b` back to `a`; from `a` to `a` the route is empty and arrives at the departure.
/// The queries are in no order of mode or destination.
fn six_queries() -> (Network, LinkTimes, [Query; 6], [ByHand; 6]) {
    let (net, [s, l1, l2], [a, b]) = two_routes();
    let cars: Vec<_> = (0..300).map(|k| vehicle(k, Mode::Car, &[s], 2 * k)).collect();
    let times = loaded(&net, &cars, window(300, 3_600));
    let query = |(origin, destination), mode, departure: u32, car: Option<u32>| Query {
        origin,
        destination,
        mode,
        departure: Duration(f64::from(departure)),
        trip: car.map(TripId::new),
    };
    let queries = [
        query((a, b), Mode::Car, 300, Some(150)),
        query((a, b), Mode::Bike, 300, None),
        query((b, a), Mode::Car, 0, None),
        query((a, b), Mode::Car, 0, Some(0)),
        query((a, a), Mode::Walk, 1_234, None),
        query((a, b), Mode::Car, 100, Some(50)),
    ];
    let by_hand = [
        Some((vec![l1, l2], 444.0)),
        Some((vec![s], 540.0)),
        None,
        Some((vec![s], 72.0)),
        Some((vec![], 1_234.0)),
        Some((vec![s], 222.0)),
    ];
    (net, times, queries, by_hand)
}

/// Whether `answer` is the route and the arrival derived by hand, or none where none is.
fn same(answer: Option<&Route>, by_hand: &ByHand) -> bool {
    match (answer, by_hand) {
        (Some(route), Some((links, arrival))) => {
            route.links == *links && close(route.arrival.get(), *arrival)
        }
        (None, None) => true,
        _ => false,
    }
}

#[test]
fn a_batch_answers_each_query_in_its_place() {
    let (net, times, queries, by_hand) = six_queries();
    let answers = fastest_many(&net, &times, &queries);
    assert_eq!(answers.len(), queries.len());
    for (i, (answer, by_hand)) in answers.iter().zip(&by_hand).enumerate() {
        assert!(same(answer.as_ref(), by_hand), "query {i}: {answer:?}, by hand {by_hand:?}");
    }
}

#[test]
fn a_batch_of_one_query_is_answered_and_a_batch_of_none_is_empty() {
    // A lone query is answered on the calling thread, which here is not one of the pool's.
    let (net, times, queries, by_hand) = six_queries();
    for (i, (query, by_hand)) in queries.iter().zip(&by_hand).enumerate() {
        let alone = fastest_many(&net, &times, core::slice::from_ref(query));
        assert_eq!(alone.len(), 1);
        assert!(same(alone[0].as_ref(), by_hand), "query {i}: {:?}, by hand {by_hand:?}", alone[0]);
    }
    assert!(fastest_many(&net, &times, &[]).is_empty(), "no query, no answer");
}
