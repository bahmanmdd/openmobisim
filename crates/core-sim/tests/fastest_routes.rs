//! Every trip's fastest route after a run (`trees::trip_fastest_routes`).
//!
//! What is defended, and how each is checked, on a hand-made network whose answers are known by
//! hand: a short one-lane road `fast` from `o` to `d` (600 m at 30 km/h: 72 s) and a longer road
//! `slow1`, `slow2` through `m` (700 m at 60 km/h each: 42 + 42 = 84 s).
//!
//! * **at free flow** every trip's fastest route is `fast`, 72 s, whatever its departure;
//! * **with a queue of known delay**: everyone who entered `fast` in the first quarter hour took
//!   100 s (a queue of 28 s), nobody was recorded after: a trip setting out in that quarter hour
//!   is faster by `slow` (84 s); one setting out an hour later, when the recorded time has been
//!   carried down to free flow, by `fast` (72 s); a wait of 30 s recorded at the origin onto `slow1`
//!   is added to the route that starts there;
//! * **the bin**: the route is the fastest at the middle of the trip's departure bin, costed at
//!   the trip's own second;
//! * **determinism**: the same answer on one thread and on seven, and when asked twice;
//! * **relabelling**: the same network with other ids, its links and nodes given in another
//!   order, gives every trip the same route (by the links' ids) and the same seconds.

use openmobisim_core_graph::defaults::{GlobalMultipliers, RoadClass, SignalDefaults};
use openmobisim_core_graph::geometry::LonLat;
use openmobisim_core_graph::network::{LinkSpec, RoadNetwork, RoadNetworkBuilder};
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_loading::{EntryTables, LinkBinRecorder};
use openmobisim_core_routes::{RouteKey, RouteSets, Shortest};
use openmobisim_core_sim::LinkTimes;
use openmobisim_core_sim::trees::{TripRoute, trip_fastest_routes};
use openmobisim_core_types::diagnostics::Diagnostics;
use openmobisim_core_types::ids::{EntityId, LinkId, NodeId};

/// The three links as `(id, from, to, metres, km/h)`, `fast` first.
const LINKS: [(&str, &str, &str, f64, f64); 3] = [
    ("fast", "o", "d", 600.0, 30.0),
    ("slow1", "o", "m", 700.0, 60.0),
    ("slow2", "m", "d", 700.0, 60.0),
];

/// The other ids of [`LINKS`]' nodes and links, chosen so that they sort in another order.
const OTHER: [(&str, &str); 6] =
    [("o", "q"), ("m", "b"), ("d", "a"), ("fast", "z9"), ("slow1", "a1"), ("slow2", "a2")];

/// `id`, or its other id if `relabel`.
fn named(id: &str, relabel: bool) -> String {
    if relabel {
        OTHER.iter().find(|(a, _)| *a == id).unwrap().1.to_string()
    } else {
        id.to_string()
    }
}

/// The network, its nodes and links given in `order` (indices into `LINKS`), every id its other
/// one if `relabel`.
fn network(order: [usize; 3], relabel: bool) -> RoadNetwork {
    let mut b = RoadNetworkBuilder::new();
    let at = |n: &str| match n {
        "o" => LonLat::new(4.800, 45.700),
        "m" => LonLat::new(4.805, 45.7004),
        _ => LonLat::new(4.810, 45.700),
    };
    let nodes: Vec<&str> = if order[0] == 0 { vec!["o", "m", "d"] } else { vec!["d", "m", "o"] };
    for n in nodes {
        b.add_node(named(n, relabel), at(n));
    }
    for i in order {
        let (id, from, to, metres, speed) = LINKS[i];
        let mut spec = LinkSpec::new(RoadClass::Primary);
        spec.lanes = Some(1);
        spec.length_m = Some(metres);
        spec.maxspeed_km_h = Some(speed);
        b.add_link(named(id, relabel), named(from, relabel), named(to, relabel), spec);
    }
    b.build(GlobalMultipliers::default(), SignalDefaults::SHIPPED, &mut Diagnostics::new())
        .expect("buildable")
}

struct Toy {
    network: RoadNetwork,
    turns: TurnTable,
    sets: RouteSets,
    key: RouteKey,
    relabel: bool,
}

impl Toy {
    fn new(order: [usize; 3], relabel: bool) -> Self {
        let network = network(order, relabel);
        let turns = TurnTable::build(&network, SignalDefaults::SHIPPED);
        let node = |n: &str| {
            network.node_external_ids().typed_id_of::<NodeId>(&named(n, relabel)).unwrap()
        };
        let key = RouteKey::new(node("o"), node("d"));
        let sets = RouteSets::generate(&network, &turns, &[key], &Shortest::ONE);
        Self { network, turns, sets, key, relabel }
    }

    fn link(&self, id: &str) -> LinkId {
        self.network.link_external_ids().typed_id_of::<LinkId>(&named(id, self.relabel)).unwrap()
    }

    /// Times where everyone entering `fast` in `[0, 900)` took `took` seconds, and everyone who
    /// set out onto `slow1` in that quarter hour waited `wait` seconds to get onto it; nothing
    /// recorded after.
    fn queued(&self, took: f64, wait: f64) -> LinkTimes {
        let n = self.network.link_count() as usize;
        let mut recorder = LinkBinRecorder::new(n, 900, 7_200.0).with_entry_bins();
        recorder.record(self.link("fast"), 100.0, 100.0 + took, 1.0);
        if wait > 0.0 {
            recorder.record_origin_wait(self.link("slow1"), 100.0, 100.0 + wait, 1.0);
        }
        let tables: EntryTables = recorder.finish_with_entry().1.expect("entry tables asked for");
        LinkTimes::from_tables(&self.network, &tables)
    }

    fn routes(&self, times: &LinkTimes, departures: &[f64], step_s: f64) -> Vec<Option<TripRoute>> {
        let pairs: Vec<(RouteKey, f64)> = departures.iter().map(|&d| (self.key, d)).collect();
        trip_fastest_routes((&self.network, &self.turns, times), &pairs, &self.sets, step_s)
    }

    /// A route's links by their ids in [`LINKS`].
    fn ids(&self, route: &TripRoute) -> Vec<String> {
        let ids = self.network.link_external_ids();
        route
            .links
            .iter()
            .map(|&l| {
                let id = ids.external(l);
                if self.relabel {
                    OTHER.iter().find(|(_, b)| *b == id).unwrap().0.to_string()
                } else {
                    id.to_string()
                }
            })
            .collect()
    }
}

const PLAIN: [usize; 3] = [0, 1, 2];

#[test]
fn the_network_is_the_one_derived_by_hand() {
    let toy = Toy::new(PLAIN, false);
    for (id, _, _, metres, speed) in LINKS {
        let ff = toy.network.free_flow_time(toy.link(id)).get();
        assert!((ff - metres / (speed / 3.6)).abs() < 1e-9, "{id}: {ff} s");
    }
}

#[test]
fn at_free_flow_every_trip_takes_the_short_road_in_72_s() {
    let toy = Toy::new(PLAIN, false);
    let free = LinkTimes::free_flow(&toy.network);
    for route in toy.routes(&free, &[0.0, 450.0, 3_000.0], 300.0) {
        let route = route.expect("found");
        assert_eq!(toy.ids(&route), ["fast"]);
        assert!((route.seconds - 72.0).abs() < 1e-9, "{}", route.seconds);
    }
}

#[test]
fn a_queue_of_known_delay_sends_the_trips_of_its_quarter_hour_round_it() {
    let toy = Toy::new(PLAIN, false);
    let times = toy.queued(100.0, 0.0);
    // In the queued quarter hour `fast` takes 100 s, `slow` 84; an hour later the recorded 100 s
    // has been carried down to free flow (100 − 900 < 72) and `fast` takes 72 again.
    let routes = toy.routes(&times, &[200.0, 4_000.0], 900.0);
    let (early, late) = (routes[0].as_ref().unwrap(), routes[1].as_ref().unwrap());
    assert_eq!(toy.ids(early), ["slow1", "slow2"]);
    assert!((early.seconds - 84.0).abs() < 1e-9, "{}", early.seconds);
    assert_eq!(toy.ids(late), ["fast"]);
    assert!((late.seconds - 72.0).abs() < 1e-9, "{}", late.seconds);
    // A wait of 30 s to get onto `slow1` in that quarter hour: `slow` then takes 114 s, `fast`
    // 100, and `fast` is the faster.
    let waited = toy.routes(&toy.queued(100.0, 30.0), &[200.0], 900.0);
    let route = waited[0].as_ref().unwrap();
    assert_eq!(toy.ids(route), ["fast"]);
    assert!((route.seconds - 100.0).abs() < 1e-9, "{}", route.seconds);
}

#[test]
fn the_route_is_the_fastest_at_the_bins_middle_costed_at_the_trips_own_second() {
    let toy = Toy::new(PLAIN, false);
    let times = toy.queued(100.0, 0.0);
    // Bins of an hour: a trip at 800 s is searched at 1 800 s, where `fast` has been carried
    // down to free flow (in the bin of 1 800 s: 100 − 900 < 72), so `fast` is found; costed at
    // 800 s, its own second, `fast` takes 100 s.
    let routes = toy.routes(&times, &[800.0], 3_600.0);
    let route = routes[0].as_ref().unwrap();
    assert_eq!(toy.ids(route), ["fast"]);
    assert!((route.seconds - 100.0).abs() < 1e-9, "{}", route.seconds);
    // Bins of a quarter hour: searched at 450 s, `slow` is found, 84 s.
    let routes = toy.routes(&times, &[800.0], 900.0);
    assert_eq!(toy.ids(routes[0].as_ref().unwrap()), ["slow1", "slow2"]);
}

#[test]
fn a_pair_without_routes_has_none_and_a_pair_at_one_node_takes_nothing() {
    let toy = Toy::new(PLAIN, false);
    let node = |n: &str| toy.network.node_external_ids().typed_id_of::<NodeId>(n).unwrap();
    let free = LinkTimes::free_flow(&toy.network);
    let pairs =
        [(RouteKey::new(node("d"), node("o")), 0.0), (RouteKey::new(node("o"), node("o")), 0.0)];
    let found = trip_fastest_routes((&toy.network, &toy.turns, &free), &pairs, &toy.sets, 300.0);
    assert_eq!(found[0], None, "d to o has no set (and no road)");
    assert_eq!(found[1], Some(TripRoute { seconds: 0.0, links: Vec::new() }));
}

#[test]
fn the_same_on_one_thread_as_on_seven_and_when_asked_twice() {
    let toy = Toy::new(PLAIN, false);
    let times = toy.queued(100.0, 0.0);
    let departures: Vec<f64> = (0..400).map(|i| f64::from(i) * 13.0).collect();
    let on = |threads: usize| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap()
            .install(|| toy.routes(&times, &departures, 300.0))
    };
    let one = on(1);
    assert_eq!(one, on(7));
    assert_eq!(one, on(1));
    assert!(one.iter().all(Option::is_some));
}

#[test]
fn other_ids_and_another_order_give_every_trip_the_same_route_and_seconds() {
    let departures = [0.0, 200.0, 800.0, 4_000.0];
    let plain = Toy::new(PLAIN, false);
    let other = Toy::new([2, 0, 1], true);
    assert_ne!(plain.link("fast").raw(), other.link("fast").raw(), "the order differs");
    for (took, wait) in [(100.0, 0.0), (100.0, 30.0), (72.0, 0.0)] {
        let a = plain.routes(&plain.queued(took, wait), &departures, 900.0);
        let b = other.routes(&other.queued(took, wait), &departures, 900.0);
        for (a, b) in a.iter().zip(&b) {
            let (a, b) = (a.as_ref().unwrap(), b.as_ref().unwrap());
            assert_eq!(plain.ids(a), other.ids(b));
            assert_eq!(a.seconds.to_bits(), b.seconds.to_bits());
        }
    }
}
