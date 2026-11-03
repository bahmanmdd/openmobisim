//! The assignment loop with the point queue (level 2), checked against values derived in each
//! test from the case's own constants and a named result of traffic-flow theory, never against
//! a run's output.
//!
//! | Test | Case | Theory | What is derived |
//! |---|---|---|---|
//! | 1 | a line of three links, the middle one a bottleneck | the deterministic queue (Newell 1982) | the total delay `½ (λ − μ) T T'`, its change when demand rises by a fifth, when the queue clears |
//! | 2 | two routes, the short one with a bottleneck | dynamic user equilibrium of the bottleneck (Vickrey 1969) | when the long route comes into use, the mean trip, the short route's share after that |
//! | 3 | the Braess network | the steady state of the point-queue equilibrium (Cominetti, Correa and Olver 2022) | route shares, queue delays and trip times in the second hour, with and without the middle link, and below capacity |
//! | 4 | a 3 × 3 grid with a signal at its centre | Webster's uniform delay; the deterministic queue at the green share of the capacity | one car's time; the total delay of 1 200 cars an hour |
//! | 5 | the line of test 1 below capacity | free flow | every trip at its free-flow time; a second iteration changes nothing |
//! | 6 | a line whose first link is the bottleneck; two routes, one behind a metered origin | the deterministic queue; the wait at an origin | a late car's measured time against its planned time; the route chosen with the wait counted |

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::float_cmp,
    clippy::too_many_lines,
    reason = "test fixtures: small counts and seconds, exact comparisons of values that must be equal"
)]

use std::time::Instant;

use openmobisim_core_assignment::{
    CarPlanner, Coefficients, CostSource, FreeFlow, HighestUtility, Inputs, IterationReport,
    LegPlanner, Loaded, Loader, Params, Place, PointQueue, StageHook, State, assign,
};
use openmobisim_core_demand::{
    ClassDefaults, Ownership, RawTrip, Travellers, Trips, build_travellers,
};
use openmobisim_core_graph::defaults::{GlobalMultipliers, RoadClass, SignalDefaults};
use openmobisim_core_graph::examples::{manhattan_grid, node_name};
use openmobisim_core_graph::geometry::LonLat;
use openmobisim_core_graph::network::{LinkSpec, RoadNetwork, RoadNetworkBuilder};
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_loading::{
    FidelityLevel, LinkTimes, Recording, Rules, Vehicle, run_ltm_chained,
};
use openmobisim_core_routes::search::{Search, SearchContext};
use openmobisim_core_types::diagnostics::Diagnostics;
use openmobisim_core_types::ids::{EntityId, LinkId, NodeId, TripId, UserClassId, VehicleId};
use openmobisim_core_types::rng::RngKey;
use openmobisim_core_types::time::Second;
use openmobisim_core_types::units::{Duration, Pcu};

// --- helpers ------------------------------------------------------------------------------

fn metres_per_second(km_h: f64) -> f64 {
    km_h / 3.6
}

/// A link of a hand-built case: its name, its ends, its length (m), its free-flow speed
/// (km/h), its capacity across its lanes (veh/h) and its lanes.
struct LinkDef {
    name: &'static str,
    from: &'static str,
    to: &'static str,
    metres: f64,
    km_h: f64,
    veh_h: f64,
    lanes: u8,
}

const fn def(
    name: &'static str,
    from: &'static str,
    to: &'static str,
    metres: f64,
    km_h: f64,
    veh_h: f64,
    lanes: u8,
) -> LinkDef {
    LinkDef { name, from, to, metres, km_h, veh_h, lanes }
}

/// A network of `nodes` (name, metres east, metres north) and `links`, every length, speed,
/// capacity and lane count stated. The test checks each link's free-flow time against its
/// length over its speed before using it ([`check_free_flow`]).
fn network(nodes: &[(&str, f64, f64)], links: &[LinkDef]) -> RoadNetwork {
    let mut b = RoadNetworkBuilder::new();
    for &(n, x, y) in nodes {
        b.add_node(n, LonLat::new(4.8 + x / 77_800.0, 45.7 + y / 110_574.0));
    }
    for l in links {
        let mut spec = LinkSpec::new(RoadClass::Secondary);
        spec.length_m = Some(l.metres);
        spec.maxspeed_km_h = Some(l.km_h);
        spec.capacity_veh_h = Some(l.veh_h);
        spec.lanes = Some(l.lanes);
        b.add_link(l.name, l.from, l.to, spec);
    }
    b.build(GlobalMultipliers::default(), SignalDefaults::SHIPPED, &mut Diagnostics::new())
        .expect("buildable")
}

/// Every link's free-flow time is its stated length over its stated speed (no signal on these
/// cases): the precondition every derivation below rests on.
fn check_free_flow(net: &RoadNetwork, links: &[LinkDef]) {
    for l in links {
        let derived = l.metres / metres_per_second(l.km_h);
        let built = net.free_flow_time(link(net, l.name)).get();
        assert!(
            (built - derived).abs() < 1e-9,
            "link {}: built {built} s, derived {derived} s",
            l.name
        );
    }
}

fn node(net: &RoadNetwork, name: &str) -> NodeId {
    net.node_external_ids().typed_id_of(name).expect("a node of the case")
}

fn link(net: &RoadNetwork, name: &str) -> LinkId {
    net.link_external_ids().typed_id_of(name).expect("a link of the case")
}

fn links(net: &RoadNetwork, names: &[&str]) -> Vec<LinkId> {
    names.iter().map(|n| link(net, n)).collect()
}

/// The departure seconds of cars leaving at `per_hour` a hour, regularly, from `start` for
/// `seconds`: the k-th at `start + ⌊k · 3600 / per_hour⌋` (departures are whole seconds).
fn departures(per_hour: u64, start: u32, seconds: u64) -> Vec<u32> {
    let n = per_hour * seconds / 3600;
    (0..n).map(|k| start + (k * 3600 / per_hour) as u32).collect()
}

/// One trip of weight 1 per departure, from `from` to `to`.
fn trips(net: &RoadNetwork, from: &str, to: &str, departures: &[u32]) -> Vec<RawTrip> {
    let at = |name: &str| net.node_lonlat(node(net, name));
    departures
        .iter()
        .enumerate()
        .map(|(i, &d)| RawTrip {
            traveller_id: format!("t{i:05}"),
            trip_seq: 0,
            origin: at(from),
            destination: at(to),
            departure_time: Second(d),
            user_class: "everyone".to_string(),
            weight: Some(1),
            mode: None,
        })
        .collect()
}

/// A network and everything an assignment on it needs but the loader.
struct Case {
    net: RoadNetwork,
    turns: TurnTable,
    travellers: Travellers,
    trips: Trips,
    classes: Vec<Coefficients>,
    money: Vec<f64>,
}

fn case(net: RoadNetwork, raw: Vec<RawTrip>) -> Case {
    let owners = ClassDefaults::new().with_fallback(Ownership::VEHICLES);
    let (travellers, trips) =
        build_travellers(raw, Vec::new(), &owners, 1, &mut Diagnostics::new()).expect("demand");
    let turns = TurnTable::build(&net, net.signals());
    let money = vec![0.0; net.link_count() as usize];
    Case { net, turns, travellers, trips, classes: vec![Coefficients::time_based()], money }
}

/// Run exactly `iterations` iterations.
fn exactly(iterations: u32) -> Params {
    Params { min_iterations: iterations, max_iterations: iterations, ..Params::default() }
}

/// What the last iteration loaded, and (if asked) every iteration's state after its measure
/// stage.
#[derive(Default)]
struct Recorder {
    keep_states: bool,
    /// The last loading's vehicles: route and departure second.
    vehicles: Vec<(Vec<LinkId>, u32)>,
    /// The last loading's arrivals, by vehicle.
    arrivals: Vec<Option<f64>>,
    states: Vec<State>,
}

impl StageHook for Recorder {
    fn before_load(&mut self, _iteration: u32, _state: &State, vehicles: &[Vehicle]) {
        self.vehicles = vehicles.iter().map(|v| (v.route.clone(), v.departure.get())).collect();
    }

    fn after_load(&mut self, _iteration: u32, loaded: &Loaded) {
        self.arrivals.clone_from(&loaded.arrival);
    }

    fn after_measure(&mut self, _iteration: u32, state: &State) {
        if self.keep_states {
            self.states.push(state.clone());
        }
    }
}

impl Recorder {
    /// Each vehicle of the last loading: its route, its departure and its measured trip time
    /// (arrival less departure); panics if one had not arrived.
    fn trips(&self) -> Vec<(&[LinkId], f64, f64)> {
        self.vehicles
            .iter()
            .zip(&self.arrivals)
            .map(|((route, d), a)| {
                let d = f64::from(*d);
                (route.as_slice(), d, a.expect("every vehicle arrives within the window") - d)
            })
            .collect()
    }
}

/// The result of one run, with its wall-clock time.
struct Run {
    state: State,
    reports: Vec<IterationReport>,
    recorder: Recorder,
    seconds: f64,
}

fn run(c: &Case, loader: &dyn Loader, params: &Params, keep_states: bool) -> Run {
    let inputs = Inputs {
        network: &c.net,
        turns: &c.turns,
        travellers: &c.travellers,
        trips: &c.trips,
        classes: &c.classes,
        money_per_link: &c.money,
        model: &HighestUtility,
        loader,
        seed: RngKey::from_seed(2026),
        initial: None,
    };
    let mut recorder = Recorder { keep_states, ..Recorder::default() };
    let clock = Instant::now();
    let (state, reports) = assign(&inputs, params, &mut recorder).expect("the run");
    let seconds = clock.elapsed().as_secs_f64();
    assert_eq!(recorder.vehicles.len(), c.trips.len() as usize, "one vehicle per trip");
    Run { state, reports, recorder, seconds }
}

/// The run time of a run and of each stage summed over its iterations.
fn timing(name: &str, r: &Run) {
    let sum = |f: fn(&IterationReport) -> f64| r.reports.iter().map(f).sum::<f64>();
    println!(
        "{name}: {} iterations in {:.3} s (search {:.3}, choose {:.3}, load {:.3}, measure {:.3}, assess {:.3})",
        r.reports.len(),
        r.seconds,
        sum(|x| x.stage_seconds.search),
        sum(|x| x.stage_seconds.choose),
        sum(|x| x.stage_seconds.load),
        sum(|x| x.stage_seconds.measure),
        sum(|x| x.stage_seconds.assess),
    );
}

/// The total delay of the deterministic queue: arrivals at `lambda` vehicles a second for
/// `t` seconds at a server of `mu` vehicles a second (`lambda > mu`), the queue growing at
/// `λ − μ` and gone at `T' = λT/μ`: `½ (λ − μ) T T'`, in vehicle-seconds.
fn queue_delay(lambda: f64, mu: f64, t: f64) -> f64 {
    let cleared = lambda * t / mu;
    0.5 * (lambda - mu) * t * cleared
}

/// The same queue served vehicle by vehicle (Lindley's recursion): each vehicle reaches the
/// server at its own second `a_k` and leaves at `s_k = max(a_k, s_{k−1} + 1/μ)`; the summed
/// wait `Σ (s_k − a_k)` and the last departure from the server.
fn queue_delay_by_vehicle(arrivals: &[f64], mu: f64) -> (f64, f64) {
    let headway = 1.0 / mu;
    let (mut total, mut last) = (0.0, f64::NEG_INFINITY);
    for &a in arrivals {
        let s = a.max(last + headway);
        total += s - a;
        last = s;
    }
    (total, last)
}

// --- 1. the deterministic queue -----------------------------------------------------------

/// The line `o → m → b → d`, 1 000 m links at 50 km/h (72 s each); `m → b` one lane of
/// 1 400 veh/h, the others three lanes of 6 000 veh/h together.
const LINE_KM_H: f64 = 50.0;
const LINE_M: f64 = 1000.0;
const BOTTLENECK_VEH_H: f64 = 1400.0;
const AMPLE_VEH_H: f64 = 6000.0;

fn line_links() -> [LinkDef; 3] {
    [
        def("om", "o", "m", LINE_M, LINE_KM_H, AMPLE_VEH_H, 3),
        def("mb", "m", "b", LINE_M, LINE_KM_H, BOTTLENECK_VEH_H, 1),
        def("bd", "b", "d", LINE_M, LINE_KM_H, AMPLE_VEH_H, 3),
    ]
}

fn line() -> RoadNetwork {
    network(
        &[("o", 0.0, 0.0), ("m", 1000.0, 0.0), ("b", 2000.0, 0.0), ("d", 3000.0, 0.0)],
        &line_links(),
    )
}

/// The total delay `Σ (trip − free-flow trip)` of cars leaving `o` at `per_hour` for 30 min,
/// with the last car's arrival and the run.
fn line_delay(per_hour: u64) -> (f64, f64, Vec<u32>, Run) {
    let net = line();
    check_free_flow(&net, &line_links());
    let deps = departures(per_hour, 0, 1800);
    let c = case(line(), trips(&net, "o", "d", &deps));
    let loader = PointQueue::after_last_departure(&c.turns, &c.trips, 3600.0);
    let r = run(&c, &loader, &exactly(1), false);
    let free = 3.0 * LINE_M / metres_per_second(LINE_KM_H);
    let trips = r.recorder.trips();
    let delay: f64 = trips.iter().map(|&(_, _, t)| t - free).sum();
    let last = trips.iter().map(|&(_, d, t)| d + t).fold(f64::NEG_INFINITY, f64::max);
    (delay, last, deps, r)
}

#[test]
fn the_bottleneck_delay_is_the_deterministic_queue() {
    let t = 1800.0;
    let mu = BOTTLENECK_VEH_H / 3600.0;
    let link_s = LINE_M / metres_per_second(LINE_KM_H);
    let mut results = Vec::new();
    for per_hour in [2000_u64, 2400] {
        let lambda = per_hour as f64 / 3600.0;
        let (simulated, last, deps, r) = line_delay(per_hour);
        timing(&format!("line at {per_hour} veh/h"), &r);
        let formula = queue_delay(lambda, mu, t);
        // Vehicle by vehicle: each car reaches the bottleneck's downstream end at its
        // departure plus two links' free-flow time.
        let at_server: Vec<f64> = deps.iter().map(|&d| f64::from(d) + 2.0 * link_s).collect();
        let (by_vehicle, served_last) = queue_delay_by_vehicle(&at_server, mu);
        let n = deps.len() as f64;
        println!(
            "λ = {per_hour} veh/h, n = {n}: total delay simulated {simulated:.1} veh·s, \
             fluid formula {formula:.1} ({:+.3} %), vehicle by vehicle {by_vehicle:.1}; \
             last arrival simulated {last:.1} s, derived {:.1} s",
            100.0 * (simulated / formula - 1.0),
            served_last + link_s
        );
        assert!((simulated / formula - 1.0).abs() <= 0.03, "within 3 % of ½ (λ − μ) T T'");
        // Arrivals are floored to the whole second: each car's time is within 1 s below its
        // exact value.
        assert!(simulated <= by_vehicle + 1e-6 && simulated >= by_vehicle - n);
        assert!((last - (served_last + link_s)).abs() <= 1.0, "the queue clears when derived");
        assert_eq!(r.reports[0].unfinished, 0.0);
        results.push((simulated, formula));
    }
    let (moved, derived) = (results[1].0 - results[0].0, results[1].1 - results[0].1);
    println!("demand + 20 %: delay moved by {moved:.1} veh·s, formula {derived:.1}");
    assert!((moved / derived - 1.0).abs() <= 0.03);
}

// --- 5. level 2 under free-flow demand ----------------------------------------------------

/// 600 cars an hour on the line of test 1 stay below every capacity: no queue forms, so every
/// trip takes the free-flow time (3 × 72 s), and a second iteration loads the same vehicles
/// and measures the same link times.
#[test]
fn below_capacity_every_trip_is_at_free_flow_and_nothing_moves() {
    let net = line();
    let c = case(line(), trips(&net, "o", "d", &departures(600, 0, 1800)));
    let loader = PointQueue::after_last_departure(&c.turns, &c.trips, 3600.0);
    let r = run(&c, &loader, &exactly(2), true);
    timing("line at 600 veh/h", &r);
    let free = 3.0 * LINE_M / metres_per_second(LINE_KM_H);
    for (_, _, trip) in r.recorder.trips() {
        assert!((trip - free).abs() <= 1.0, "trip {trip} s, free flow {free} s");
    }
    let (first, second) = (&r.reports[0], &r.reports[1]);
    println!("iteration 0: {:?}\niteration 1: {:?}", first.without_times(), second.without_times());
    assert!((first.mean_trip_seconds - free).abs() <= 1.0);
    assert_eq!(second.found, 0);
    assert_eq!(second.switch_share, 0.0);
    assert_eq!(second.duality_gap, 0.0);
    assert_eq!(
        (second.departed, second.arrived, second.mean_trip_seconds),
        (first.departed, first.arrived, first.mean_trip_seconds)
    );
    assert_eq!(
        r.recorder.states[1], r.recorder.states[0],
        "iteration 1 leaves the state as it was"
    );
}

// --- 2. two routes in dynamic equilibrium -------------------------------------------------

/// `o → a → d`, 1 440 m + 1 440 m at 36 km/h: t1 = 288 s, `o → a` one lane of c1 = 1 200
/// veh/h; `o → b → d`, 2 305 m + 2 305 m at 36 km/h: t2 = 461 s, c2 = 1 800 veh/h.
const TWO_KM_H: f64 = 36.0;
const C1_VEH_H: f64 = 1200.0;
const C2_VEH_H: f64 = 1800.0;

fn two_route_links() -> [LinkDef; 4] {
    [
        def("oa", "o", "a", 1440.0, TWO_KM_H, C1_VEH_H, 1),
        def("ad", "a", "d", 1440.0, TWO_KM_H, AMPLE_VEH_H, 3),
        def("ob", "o", "b", 2305.0, TWO_KM_H, C2_VEH_H, 1),
        def("bd", "b", "d", 2305.0, TWO_KM_H, C2_VEH_H, 1),
    ]
}

fn two_routes() -> RoadNetwork {
    network(
        &[("o", 0.0, 0.0), ("a", 1400.0, 300.0), ("b", 1400.0, -1700.0), ("d", 2800.0, 0.0)],
        &two_route_links(),
    )
}

#[test]
fn two_routes_reach_the_bottleneck_equilibrium() {
    let net = two_routes();
    check_free_flow(&net, &two_route_links());
    let v = metres_per_second(TWO_KM_H);
    let (t1, t2) = (2.0 * 1440.0 / v, 2.0 * 2305.0 / v);
    assert!((t1 - 288.0).abs() < 1e-9 && (t2 - 461.0).abs() < 1e-9, "the case is as stated");
    let big_t = 3600.0;
    let c1 = C1_VEH_H / 3600.0;
    for q_h in [1800_u64, 2400] {
        let q = q_h as f64 / 3600.0;
        let c = case(two_routes(), trips(&net, "o", "d", &departures(q_h, 0, 3600)));
        let loader = PointQueue::after_last_departure(&c.turns, &c.trips, 3600.0);
        let r = run(&c, &loader, &exactly(100), false);
        timing(&format!("two routes at {q_h} veh/h"), &r);
        // Theory (Vickrey 1969): the short route alone until its queue costs t2 − t1.
        let t_star = (t2 - t1) * c1 / (q - c1);
        let mean =
            (t1 * t_star + (q - c1) * t_star * t_star / (2.0 * c1) + (big_t - t_star) * t2) / big_t;
        let share_after = c1 / q;
        let oa = link(&c.net, "oa");
        let short = |t: u32| {
            let chain = r.state.trip(TripId::new(t)).current_chain().expect("a chain");
            chain.legs()[0].links[0] == oa
        };
        let n = c.trips.len();
        let n_long = (0..n).filter(|&t| !short(t)).count() as f64;
        let t_star_simulated = big_t - n_long / (q - c1);
        let after: Vec<u32> = (0..n)
            .filter(|&t| f64::from(c.trips.departure(TripId::new(t)).get()) >= t_star)
            .collect();
        let share_simulated =
            after.iter().filter(|&&t| short(t)).count() as f64 / after.len() as f64;
        let last = r.reports.last().expect("reports");
        println!(
            "q = {q_h} veh/h: t* derived {t_star:.1} s, simulated {t_star_simulated:.1} s; \
             mean trip derived {mean:.2} s, simulated {:.2} s ({:+.2} %); short route's share \
             after t* derived {share_after:.4}, simulated {share_simulated:.4}; long-route trips \
             {n_long}; last iteration: switch share {:.4}, duality gap {:.5}",
            last.mean_trip_seconds,
            100.0 * (last.mean_trip_seconds / mean - 1.0),
            last.switch_share,
            last.duality_gap
        );
        assert!((last.mean_trip_seconds / mean - 1.0).abs() <= 0.03, "the mean trip within 3 %");
        assert!((t_star_simulated - t_star).abs() <= 300.0, "t* within a cost bin");
        assert!((share_simulated - share_after).abs() <= 0.03, "the share after t* within 0.03");
    }
}

// --- 3. the Braess paradox at steady state ------------------------------------------------

/// `s → a` and `b → t`: 1 km at 1 000 veh/h; `a → t` and `s → b`: 10 km; `a → b`: 1 km; all at
/// 60 km/h (1 min a kilometre), the last three at 6 000 veh/h.
const BRAESS_KM_H: f64 = 60.0;

fn braess_links(middle: bool) -> Vec<LinkDef> {
    let mut l = vec![
        def("sa", "s", "a", 1000.0, BRAESS_KM_H, 1000.0, 1),
        def("bt", "b", "t", 1000.0, BRAESS_KM_H, 1000.0, 1),
        def("at", "a", "t", 10_000.0, BRAESS_KM_H, 6000.0, 3),
        def("sb", "s", "b", 10_000.0, BRAESS_KM_H, 6000.0, 3),
    ];
    if middle {
        l.push(def("ab", "a", "b", 1000.0, BRAESS_KM_H, 6000.0, 3));
    }
    l
}

fn braess(middle: bool) -> RoadNetwork {
    network(
        &[("s", 0.0, 0.0), ("a", 500.0, 400.0), ("b", 500.0, -400.0), ("t", 1000.0, 0.0)],
        &braess_links(middle),
    )
}

/// What the trips leaving in the second hour did: per route (by its links' names), the share
/// of trips and their mean measured time, and the mean queue delay at `s → a` and `b → t`
/// (the link's measured time less its free-flow time, by bin of entry, averaged over the
/// second hour's bins).
struct SteadyState {
    routes: Vec<(String, f64, f64)>,
    mean_trip: f64,
    queue_sa: f64,
    queue_bt: f64,
}

fn steady_state(c: &Case, r: &Run) -> SteadyState {
    let names = |route: &[LinkId]| {
        route
            .iter()
            .map(|&l| c.net.link_external_ids().external(l.raw()))
            .collect::<Vec<_>>()
            .join("-")
    };
    let hour: Vec<(String, f64)> = r
        .recorder
        .trips()
        .into_iter()
        .filter(|&(_, d, _)| (3600.0..7200.0).contains(&d))
        .map(|(route, _, t)| (names(route), t))
        .collect();
    let n = hour.len() as f64;
    let mut kinds: Vec<String> = hour.iter().map(|(k, _)| k.clone()).collect();
    kinds.sort();
    kinds.dedup();
    let routes = kinds
        .into_iter()
        .map(|k| {
            let times: Vec<f64> = hour.iter().filter(|(x, _)| *x == k).map(|&(_, t)| t).collect();
            let share = times.len() as f64 / n;
            let mean = times.iter().sum::<f64>() / times.len() as f64;
            (k, share, mean)
        })
        .collect();
    let mean_trip = hour.iter().map(|&(_, t)| t).sum::<f64>() / n;
    let times = r.state.link_times();
    let queue = |name: &str| {
        let l = link(&c.net, name);
        let free = c.net.free_flow_time(l).get();
        let bins: Vec<f64> = (12..24)
            .map(|k| times.link_seconds(l.raw(), (f64::from(k) + 0.5) * 300.0) - free)
            .collect();
        bins.iter().sum::<f64>() / bins.len() as f64
    };
    SteadyState { routes, mean_trip, queue_sa: queue("sa"), queue_bt: queue("bt") }
}

fn braess_run(middle: bool, per_hour: u64) -> (SteadyState, Run) {
    let net = braess(middle);
    check_free_flow(&net, &braess_links(middle));
    let c = case(braess(middle), trips(&net, "s", "t", &departures(per_hour, 0, 3 * 3600)));
    let loader = PointQueue::after_last_departure(&c.turns, &c.trips, 3600.0);
    let r = run(&c, &loader, &exactly(100), false);
    let s = steady_state(&c, &r);
    timing(&format!("Braess, middle link {middle}, {per_hour} veh/h"), &r);
    for x in r.reports.iter().filter(|x| [0, 9, 24, 49, 99].contains(&x.iteration)) {
        println!(
            "  iteration {}: switch share {:.4}, duality gap {:.5}, mean trip {:.1} s",
            x.iteration, x.switch_share, x.duality_gap, x.mean_trip_seconds
        );
    }
    println!(
        "  second hour: mean trip {:.1} s; queue delay s→a {:.1} s, b→t {:.1} s; routes (share, mean trip s): {:?}",
        s.mean_trip, s.queue_sa, s.queue_bt, s.routes
    );
    (s, r)
}

fn share_of(s: &SteadyState, route: &str) -> (f64, f64) {
    s.routes.iter().find(|(k, _, _)| k == route).map_or((0.0, f64::NAN), |&(_, sh, m)| (sh, m))
}

/// The theorem of Cominetti, Correa and Olver (2022, *Operations Research* 70(1)): with one
/// origin, one destination and a constant inflow below the minimum cut, the point-queue
/// equilibrium reaches a steady state whose route flows solve the linear program
/// `min Σ τ_e y_e` under the capacities, the queue delays being the capacities' dual prices.
/// For these constants (free-flow times in minutes: s-a-t 11, s-b-t 11, s-a-b-t 3; capacities
/// 1 000 veh/h on `s → a` and `b → t`), the program's solution, worked by hand:
///
/// - **without `a → b`**: any split with at most 1 000 veh/h a route carries 1 500 veh/h with
///   no queue; everyone takes 11 min;
/// - **with `a → b`, 1 500 veh/h**: maximising the flow on s-a-b-t (3 min against 11) under
///   `y_sat + y_sabt ≤ 1 000`, `y_sbt + y_sabt ≤ 1 000`, `Σ y = 1 500` gives 500 veh/h on each
///   route; equal costs `11 + q_sa = 11 + q_bt = 3 + q_sa + q_bt` give queues of 8 min at
///   `s → a` and at `b → t`, and 19 min for everyone;
/// - **with `a → b`, 900 veh/h**: below both capacities, everyone on s-a-b-t, 3 min.
#[test]
fn the_braess_network_reaches_the_linear_programs_steady_state() {
    let minute = 60.0;
    // Without the middle link.
    let (s, _) = braess_run(false, 1500);
    let derived = 11.0 * minute;
    assert!((s.mean_trip / derived - 1.0).abs() <= 0.02, "11 min within 2 %");
    for route in ["sa-at", "sb-bt"] {
        let (share, mean) = share_of(&s, route);
        assert!(
            (1.0 / 3.0 - 0.03..=2.0 / 3.0 + 0.03).contains(&share),
            "{route}: share {share} outside the equilibria's [1/3, 2/3]"
        );
        assert!((mean / derived - 1.0).abs() <= 0.02, "{route}: {mean} s");
    }
    // With it.
    let (s, _) = braess_run(true, 1500);
    let derived = 19.0 * minute;
    let queue = 8.0 * minute;
    assert!((s.mean_trip / derived - 1.0).abs() <= 0.02, "19 min within 2 %");
    for route in ["sa-at", "sb-bt", "sa-ab-bt"] {
        let (share, mean) = share_of(&s, route);
        assert!((share - 1.0 / 3.0).abs() <= 0.03, "{route}: share {share}");
        assert!((mean / derived - 1.0).abs() <= 0.02, "{route}: {mean} s");
    }
    assert!((s.queue_sa / queue - 1.0).abs() <= 0.05, "the queue at s → a: {} s", s.queue_sa);
    assert!((s.queue_bt / queue - 1.0).abs() <= 0.05, "the queue at b → t: {} s", s.queue_bt);
    // Below capacity.
    let (s, _) = braess_run(true, 900);
    let derived = 3.0 * minute;
    assert!((s.mean_trip / derived - 1.0).abs() <= 0.02, "3 min within 2 %");
    let (share, _) = share_of(&s, "sa-ab-bt");
    assert!((share - 1.0).abs() <= 0.03, "everyone on s-a-b-t: {share}");
}

// --- 4. a signalised approach -------------------------------------------------------------

/// `manhattan_grid`'s constants: blocks of 200 m (asked for), secondary roads at 50 km/h, a
/// capacity of 1 800 veh/h stated per link (one lane), a signal at every interior junction
/// (here the one at row 1, column 1). The shipped signal settings: a cycle of C = 90 s, a green
/// share of g = 0.48, the control delay evaluated at a degree of saturation of x = 0.85.
const BLOCK_M: f64 = 200.0;
const GRID_KM_H: f64 = 50.0;
const GRID_VEH_H: f64 = 1800.0;
const CYCLE_S: f64 = 90.0;
const GREEN: f64 = 0.48;
const SATURATION: f64 = 0.85;

/// Webster's uniform delay (1958): `d = ½ C (1 − g)² / (1 − x g)`.
fn webster() -> f64 {
    0.5 * CYCLE_S * (1.0 - GREEN) * (1.0 - GREEN) / (1.0 - SATURATION * GREEN)
}

fn grid_case(signals: bool, deps: &[u32]) -> Case {
    let (net, _) = manhattan_grid(3, BLOCK_M, signals);
    let raw = trips(&net, &node_name(1, 0), &node_name(1, 2), deps);
    case(manhattan_grid(3, BLOCK_M, signals).0, raw)
}

#[test]
fn a_signalised_approach_adds_websters_delay_and_discharges_at_the_green_share() {
    let s = SignalDefaults::SHIPPED;
    assert_eq!(
        (s.cycle_seconds, s.green_fraction, s.nominal_degree_of_saturation),
        (CYCLE_S, GREEN, SATURATION),
        "the constants stated are the shipped ones"
    );
    let t = BLOCK_M / metres_per_second(GRID_KM_H);
    let d = webster();
    println!("one block t = {t:.3} s, Webster's uniform delay d = {d:.3} s");
    // (a) One car, at free flow and at the point queue.
    for signals in [true, false] {
        let derived = if signals { 2.0 * t + d } else { 2.0 * t };
        let c = grid_case(signals, &[0]);
        let pq = PointQueue::after_last_departure(&c.turns, &c.trips, 600.0);
        let free = FreeFlow::default();
        for (name, loader) in [("free flow", &free as &dyn Loader), ("point queue", &pq)] {
            let r = run(&c, loader, &exactly(1), false);
            let (route, _, trip) = r.recorder.trips()[0];
            println!(
                "one car, signals {signals}, {name}: {} links, trip {trip:.3} s, derived {derived:.3} s",
                route.len()
            );
            assert_eq!(route.len(), 2, "straight through the centre");
            assert!((trip - derived).abs() <= 1.0);
        }
    }
    // (b) 1 200 cars an hour for 30 min at the point queue.
    let lambda = 1200.0 / 3600.0;
    let big_t = 1800.0;
    for signals in [true, false] {
        let c = grid_case(signals, &departures(1200, 0, 1800));
        let loader = PointQueue::after_last_departure(&c.turns, &c.trips, 3600.0);
        let r = run(&c, &loader, &exactly(1), false);
        timing(&format!("grid, signals {signals}, 1 200 veh/h"), &r);
        let free = if signals { 2.0 * t + d } else { 2.0 * t };
        let trips = r.recorder.trips();
        let delay: f64 = trips.iter().map(|&(_, _, x)| x - free).sum();
        if signals {
            let mu = GREEN * GRID_VEH_H / 3600.0;
            let derived = queue_delay(lambda, mu, big_t);
            println!(
                "signalised approach, μ = {:.0} veh/h: total delay simulated {delay:.1} veh·s, \
                 derived {derived:.1} ({:+.3} %)",
                mu * 3600.0,
                100.0 * (delay / derived - 1.0)
            );
            assert!((delay / derived - 1.0).abs() <= 0.05);
        } else {
            let worst = trips.iter().map(|&(_, _, x)| (x - free).abs()).fold(0.0, f64::max);
            println!("no signal, μ = 1 800 veh/h > λ: largest delay of a car {worst:.3} s");
            assert!(worst <= 1.0, "no queue: every car within 1 s of free flow");
        }
    }
}

// --- 6. the wait at the origin: planned and measured times agree --------------------------

/// The line `o → m → d`, 1 000 m links at 50 km/h, `o → m` the bottleneck (one lane, 1 400
/// veh/h); 2 000 cars an hour for 30 min, then one more car at 1 950 s, while the queue still
/// stands. The bottleneck lets a car out every `h = 3600 / 1400` s from the first car's arrival
/// at its downstream end (72 s), and arrivals come faster than that throughout, so the late car,
/// the 1 001st, leaves it at `72 + 1000 h` and arrives 72 s later. It enters `o → m` alone in
/// its 300-s bin of entry time, so the time the measure stage prices its chain at is its own.
#[test]
fn a_late_car_through_a_first_link_bottleneck_takes_its_planned_time() {
    let ld = [
        def("om", "o", "m", LINE_M, LINE_KM_H, BOTTLENECK_VEH_H, 1),
        def("md", "m", "d", LINE_M, LINE_KM_H, AMPLE_VEH_H, 3),
    ];
    let build = || network(&[("o", 0.0, 0.0), ("m", 1000.0, 0.0), ("d", 2000.0, 0.0)], &ld);
    let net = build();
    check_free_flow(&net, &ld);
    let late = 1950_u32;
    let mut deps = departures(2000, 0, 1800);
    let n = deps.len() as f64;
    deps.push(late);
    let c = case(build(), trips(&net, "o", "d", &deps));
    let loader = PointQueue::after_last_departure(&c.turns, &c.trips, 3600.0);
    let r = run(&c, &loader, &exactly(1), false);
    timing("first-link bottleneck", &r);
    let link_s = LINE_M / metres_per_second(LINE_KM_H);
    let h = 3600.0 / BOTTLENECK_VEH_H;
    let derived = link_s + n * h - f64::from(late) + link_s;
    let probe = TripId::new(c.trips.len() - 1);
    let measured = r.recorder.trips().last().expect("the late car").2;
    let chain = r.state.trip(probe).current_chain().expect("a chain");
    let planned = chain.total().in_vehicle_seconds;
    // The planner's own search at the loaded times, from the late car's departure.
    let planner = CarPlanner::new(&c.net, &c.turns, &c.classes, &c.money).expect("planner");
    let (from, to) = (Place::Road(node(&c.net, "o")), Place::Road(node(&c.net, "d")));
    let searched = planner
        .plan(from, to, f64::from(late), UserClassId::new(0), &r.state, CostSource::Exact)
        .expect("a route");
    println!(
        "late car: measured {measured:.3} s, planned at the loaded times {planned:.3} s, \
         searched {:.3} s, derived {derived:.3} s",
        searched.attributes.in_vehicle_seconds
    );
    assert!((measured - derived).abs() <= 1.0, "the late car's time is the queue's");
    assert!((measured - planned).abs() <= 1.0, "measured and planned agree within 1 s");
    assert_eq!(searched.links, chain.legs()[0].links);
    assert!((searched.attributes.in_vehicle_seconds - planned).abs() < 1e-9);
    // Cars sharing a bin of entry time are priced at the bin's mean: within a bin the queue
    // delay grows at (λ − μ)/μ per second of departure, so no car is further from its planned
    // time than that over one bin, and a second of rounding.
    let lambda = 2000.0 / 3600.0;
    let mu = BOTTLENECK_VEH_H / 3600.0;
    let bound = (lambda - mu) / mu * 300.0 + 1.0;
    let worst = r
        .recorder
        .trips()
        .iter()
        .enumerate()
        .map(|(t, &(_, _, m))| {
            let p = r.state.trip(TripId::from_index(t)).current_chain().expect("a chain");
            (m - p.total().in_vehicle_seconds).abs()
        })
        .fold(0.0, f64::max);
    println!("largest |measured − planned| over all cars: {worst:.1} s (bound {bound:.1} s)");
    assert!(worst <= bound);
}

/// Two routes from `o` to `d` at 36 km/h: `o → x → d`, 1 000 m + 1 000 m (200 s), `o → x` one
/// lane of 600 veh/h; `o → y → d`, 3 000 m + 3 000 m (600 s), ample. A loading that meters
/// entries from origins at the first link's capacity (`core-loading`'s origin inflow rule) takes
/// 200 cars all leaving at second 0 onto `o → x → d`: they enter one every `h = 6` s, so the
/// mean wait at the origin of the bin of departure is `h (200 − 1) / 2 = 597` s, and they cross
/// both links at free flow. A car leaving at second 0 on those link times then takes
/// 597 + 200 = 797 s by `x` and 600 s by `y`: the planner, counting the wait, routes it by `y`;
/// a search blind to the wait would route it by `x`.
#[test]
fn the_search_counts_the_wait_at_the_origin() {
    let km_h = 36.0;
    let ld = [
        def("ox", "o", "x", 1000.0, km_h, 600.0, 1),
        def("xd", "x", "d", 1000.0, km_h, AMPLE_VEH_H, 3),
        def("oy", "o", "y", 3000.0, km_h, AMPLE_VEH_H, 3),
        def("yd", "y", "d", 3000.0, km_h, AMPLE_VEH_H, 3),
    ];
    let net = network(
        &[("o", 0.0, 0.0), ("x", 900.0, 300.0), ("y", 900.0, -2000.0), ("d", 1800.0, 0.0)],
        &ld,
    );
    check_free_flow(&net, &ld);
    let turns = TurnTable::build(&net, net.signals());
    let cars = 200_u32;
    let by_x = links(&net, &["ox", "xd"]);
    let vehicles: Vec<Vehicle> = (0..cars)
        .map(|i| Vehicle::new(VehicleId::new(i), by_x.clone(), Pcu(1.0), Second(0)))
        .collect();
    let out = run_ltm_chained(
        &net,
        &turns,
        &vehicles,
        &[],
        Duration(7200.0),
        Duration(300.0),
        FidelityLevel::PointQueue,
        Recording::BinsAndEntry(300, 300),
        Rules { origin_inflow: true, ..Rules::default() },
        None,
    );
    let times = LinkTimes::from_tables(&net, &out.entry.expect("entry tables"));
    let h = 3600.0 / 600.0;
    let wait = h * f64::from(cars - 1) / 2.0;
    let v = metres_per_second(km_h);
    let (via_x, via_y) = (wait + 2000.0 / v, 6000.0 / v);
    let recorded = times.origin_wait_seconds(link(&net, "ox").raw(), 0.0);
    println!(
        "mean wait at the origin: recorded {recorded:.6} s, derived {wait} s; by x {via_x} s, by y {via_y} s"
    );
    assert!((recorded - wait).abs() < 1e-6);

    let classes = [Coefficients::time_based()];
    let money = vec![0.0; net.link_count() as usize];
    let planner = CarPlanner::new(&net, &turns, &classes, &money).expect("planner");
    let owners = ClassDefaults::new().with_fallback(Ownership::VEHICLES);
    let (travellers, trip_table) = build_travellers(
        trips(&net, "o", "d", &[0]),
        Vec::new(),
        &owners,
        1,
        &mut Diagnostics::new(),
    )
    .expect("demand");
    let mut state = State::initial(&net, &travellers, &trip_table);
    state.set_link_times(times.clone());
    let (o, d) = (Place::Road(node(&net, "o")), Place::Road(node(&net, "d")));
    let leg =
        planner.plan(o, d, 0.0, UserClassId::new(0), &state, CostSource::Exact).expect("a route");
    let priced_x = planner.leg_along(o, d, 0.0, by_x.clone(), &state).attributes.in_vehicle_seconds;
    println!(
        "planned: {:?}, {:.6} s; the route by x priced at {priced_x:.6} s",
        leg.links.iter().map(|&l| net.link_external_ids().external(l.raw())).collect::<Vec<_>>(),
        leg.attributes.in_vehicle_seconds
    );
    assert_eq!(leg.links, links(&net, &["oy", "yd"]), "the wait counted, the route by y");
    assert!((leg.attributes.in_vehicle_seconds - via_y).abs() < 1e-6);
    assert!((priced_x - via_x).abs() < 1e-6, "the route by x is priced with the wait");
    // A search that does not see the wait would have taken the route by x.
    let context = SearchContext::new(&net, &turns);
    let mut blind = Search::new(&context);
    let seconds = |l: u32, t: f64| times.link_seconds(l, t);
    let (_, _, route) = blind
        .cheapest_route_from(node(&net, "o"), node(&net, "d"), 0.0, f64::INFINITY, &seconds)
        .expect("a route");
    assert_eq!(route, by_x, "without the wait the queue is invisible");
}
