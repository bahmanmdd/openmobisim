//! The assignment loop with the point queue (level 2), checked against values derived in each
//! test from the case's own constants and a named result of traffic-flow theory, never against
//! a run's output.
//!
//! | Test | Case | Theory | What is derived |
//! |---|---|---|---|
//! | 1 | a line of three links, the middle one a bottleneck | the deterministic queue (Newell 1982) | the total delay `½ (λ − μ) T T'`, its change when demand rises by a fifth, when the queue clears |
//! | 2 | two routes, the short one with a bottleneck | dynamic user equilibrium of the bottleneck (Vickrey 1969) | when the long route comes into use, the mean trip, the short route's share after that |
//! | 3 | the Braess network | the steady state of the point-queue equilibrium (Cominetti, Correa and Olver 2022) | route shares, queue delays and trip times in the second hour, with and without the middle link (with it, the mean over master seeds 1–20), and below capacity |
//! | 4 | a 3 × 3 grid with a signal at its centre | Webster's uniform delay; the deterministic queue at the green share of the capacity | one car's time; the total delay of 1 200 cars an hour |
//! | 5 | the line of test 1 below capacity | free flow | every trip at its free-flow time; a second iteration changes nothing |
//! | 6 | a line whose first link is the bottleneck; two routes, one behind a metered origin | the deterministic queue; the wait at an origin | a late car's measured time against its planned time; the route chosen with the wait counted |
//! | 7 | the two routes of test 2, logit and deterministic, run to the rule's stop | the fixed point of the loop | a run continued from its final state finds nothing beyond the band and keeps the expected switch excess within the tolerance |
//! | 8 | the two routes; the Braess network | the stopping rule, at most 200 iterations | where the loop stops and the mean trip's error there (printed only) |
//! | 9 | one car on two empty links; the two routes; the line of test 1 | free flow; the route of least cost under a constant added to every route; the deterministic queue's invariance to a shift in time | the car's fixed times of 120 s and 300 s: the door-to-door time, when the car sets off, the attributes and their cost; a trip ending after the window; no route changed; the bottleneck's delay unchanged, its pattern shifted by 120 s |
//! | 10 | the two routes, logit and deterministic, with and without the fixed times | the loading is deterministic | a state rebuilt from what a run wrote of its final state is that state, to the bit, and a run continued from it goes on alike; a saved state that does not fit is refused |
//! | 11 | a 5 × 5 grid, four corner-to-corner flows above capacity, the logit, the FIFO reading; the two routes | the model has no hour: a shift of every departure and the window by 3 600 s; the FIFO reading reads entrants | every trip's route and time the same; a loading that records none refused under the FIFO reading |

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
    AssignError, CarPlanner, ChainSearch, Coefficients, CostSource, FixedTimes, FreeFlow,
    HighestUtility, Inputs, IterationReport, LegPlanner, Loaded, Loader, Params, Place, PointQueue,
    SavedTrip, StageHook, State, assign, logit, restore,
};
use openmobisim_core_choice::ChoiceModel;
use openmobisim_core_demand::{
    ClassDefaults, Ownership, RawTrip, Travellers, Trips, build_travellers,
};
use openmobisim_core_graph::defaults::{GlobalMultipliers, RoadClass, SignalDefaults};
use openmobisim_core_graph::examples::{manhattan_grid, node_name};
use openmobisim_core_graph::geometry::LonLat;
use openmobisim_core_graph::network::{LinkSpec, RoadNetwork, RoadNetworkBuilder};
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_loading::{
    FidelityLevel, LinkTimes, LinkTimesReading, Recording, Rules, Vehicle, run_ltm_chained,
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
    run_with(c, loader, params, keep_states, &HighestUtility, 2026)
}

/// [`run`] with another choice model and master seed.
fn run_with(
    c: &Case,
    loader: &dyn Loader,
    params: &Params,
    keep_states: bool,
    model: &dyn ChoiceModel,
    seed: u64,
) -> Run {
    let inputs = Inputs {
        network: &c.net,
        turns: &c.turns,
        layers: openmobisim_core_assignment::Layers::NONE,
        transit: None,
        travellers: &c.travellers,
        trips: &c.trips,
        classes: &c.classes,
        mode_choice: &[],
        money_per_link: &c.money,
        model,
        loader,
        seed: RngKey::from_seed(seed),
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

/// The closed-form mean trip of the two-route case (Vickrey 1969) at `q_h` veh/h for an hour:
/// the short route alone until `t* = (t2 − t1) c1 / (q − c1)`, its queue growing at `q − c1`,
/// then both routes at the cost `t2`.
fn two_route_mean(q_h: u64) -> f64 {
    let v = metres_per_second(TWO_KM_H);
    let (t1, t2) = (2.0 * 1440.0 / v, 2.0 * 2305.0 / v);
    let (q, big_t, c1) = (q_h as f64 / 3600.0, 3600.0, C1_VEH_H / 3600.0);
    let t_star = (t2 - t1) * c1 / (q - c1);
    (t1 * t_star + (q - c1) * t_star * t_star / (2.0 * c1) + (big_t - t_star) * t2) / big_t
}

/// The mean of the last three reports' values of `f`.
fn tail_mean(reports: &[IterationReport], f: fn(&IterationReport) -> f64) -> f64 {
    let tail = &reports[reports.len().saturating_sub(3)..];
    tail.iter().map(f).sum::<f64>() / tail.len() as f64
}

/// Where the switch share above its floor over all trips would have stopped the loop (its mean
/// over three iterations at most 0.01, after the minimum iterations and with nothing found),
/// read from the reports of a run: a measurement for comparison, not the rule.
fn stop_by_above_floor(reports: &[IterationReport], params: &Params) -> Option<usize> {
    (params.min_iterations.max(1) as usize..reports.len()).find(|&k| {
        tail_mean(&reports[..k], |r| r.switch_above_floor) <= 0.01 && reports[k].found_share == 0.0
    })
}

/// The stopping rule at its defaults but with at most 200 iterations: where the loop stops on
/// the two-route case, and the mean trip's error there against the closed form, printed (with,
/// for comparison, where the switch share above its floor over all trips would have stopped it).
#[test]
fn where_the_loop_stops_on_the_two_routes() {
    let net = two_routes();
    let params = Params { max_iterations: 200, ..Params::default() };
    for q_h in [1800_u64, 2400] {
        let c = case(two_routes(), trips(&net, "o", "d", &departures(q_h, 0, 3600)));
        let loader = PointQueue::after_last_departure(&c.turns, &c.trips, 3600.0);
        let r = run(&c, &loader, &params, false);
        timing(&format!("two routes at {q_h} veh/h, at most 200 iterations"), &r);
        let last = r.reports.last().expect("reports");
        let mean = two_route_mean(q_h);
        let other = stop_by_above_floor(&r.reports, &params);
        println!(
            "q = {q_h} veh/h: stopped by the rule: {} ({} iterations, most {}); expected switch \
             excess, last three {:.4}; realised excess, last {:.4}; switch above floor, last three {:.5}; found share {:.4}; mean \
             trip {:.2} s against {mean:.2} s ({:+.2} %); duality gap {:.5}; the above-floor \
             measure would have stopped at {other:?} (mean trip then {:?} s)",
            r.reports.len() < params.max_iterations as usize,
            r.reports.len(),
            params.max_iterations,
            tail_mean(&r.reports, |x| x.switch_excess_expected),
            last.switch_excess,
            tail_mean(&r.reports, |x| x.switch_above_floor),
            last.found_share,
            last.mean_trip_seconds,
            100.0 * (last.mean_trip_seconds / mean - 1.0),
            last.duality_gap,
            other.map(|k| r.reports[k - 1].mean_trip_seconds)
        );
    }
}

/// As above, on the Braess network with its middle link at 1 500 veh/h: the second hour's mean
/// trip against 19 min and its queues against 8 min at the stop, printed.
#[test]
fn where_the_loop_stops_on_the_braess_network() {
    let net = braess(true);
    let c = case(braess(true), trips(&net, "s", "t", &departures(1500, 0, 3 * 3600)));
    let loader = PointQueue::after_last_departure(&c.turns, &c.trips, 3600.0);
    let params = Params { max_iterations: 200, ..Params::default() };
    let r = run(&c, &loader, &params, false);
    timing("Braess, middle link, 1 500 veh/h, at most 200 iterations", &r);
    let s = steady_state(&c, &r);
    let last = r.reports.last().expect("reports");
    println!(
        "Braess: stopped by the rule: {} ({} iterations, most {}); expected switch excess, \
         last three {:.4}; realised excess, last {:.4}; switch above floor, last three {:.5}; found share {:.4}; mean trip, second hour \
         {:.1} s against 1 140 s ({:+.2} %); queues s→a {:.1} s, b→t {:.1} s against 480 s; \
         routes {:?}; the above-floor measure would have stopped at {:?}",
        r.reports.len() < params.max_iterations as usize,
        r.reports.len(),
        params.max_iterations,
        tail_mean(&r.reports, |x| x.switch_excess_expected),
        last.switch_excess,
        tail_mean(&r.reports, |x| x.switch_above_floor),
        last.found_share,
        s.mean_trip,
        100.0 * (s.mean_trip / 1140.0 - 1.0),
        s.queue_sa,
        s.queue_bt,
        s.routes,
        stop_by_above_floor(&r.reports, &params)
    );
}

/// The stopping rule is not blind: with a deterministic choice the expected switch excess is the
/// share of trips on a chain beyond the indifference band of their best, so the two-route case at 1 800 veh/h, which is far from its
/// equilibrium after the minimum iterations (both routes are in every set by iteration 1, so
/// nothing new is found from then on), runs past the minimum; and when it does stop, the mean
/// trip is within 3 % of the closed form, as after 100 iterations.
#[test]
fn the_loop_does_not_stop_while_the_deterministic_choice_still_moves_travellers() {
    let mean = two_route_mean(1800);
    let net = two_routes();
    let c = case(two_routes(), trips(&net, "o", "d", &departures(1800, 0, 3600)));
    let loader = PointQueue::after_last_departure(&c.turns, &c.trips, 3600.0);
    let params = Params { max_iterations: 100, ..Params::default() };
    let r = run(&c, &loader, &params, false);
    let last = r.reports.last().expect("reports");
    println!(
        "stopped after {} iterations; switch excess {:.4}, mean trip {:.2} s against {mean:.2} s",
        r.reports.len(),
        last.switch_excess,
        last.mean_trip_seconds
    );
    assert!(
        r.reports.len() > params.min_iterations as usize,
        "the rule did not stop at the minimum"
    );
    assert!(
        tail_mean(&r.reports, |x| x.switch_excess_expected).abs() <= params.switch_tolerance
            || r.reports.len() == 100
    );
    assert!((last.mean_trip_seconds / mean - 1.0).abs() <= 0.03, "the mean trip within 3 %");
}

/// Idempotence: a run that stopped by its own rule, continued from its final state, goes on
/// where it stopped (the state carries its iteration count, so the averaging step goes on).
/// The rule stopped because the search found nothing beyond the band and the expected switch
/// excess (the share of trips that would change chain beyond the stationary chance if everyone
/// reconsidered, a quantity with no sampling noise) averaged at most the tolerance; at a fixed
/// point of the loop each continued iteration again finds nothing (`found` 0) and keeps the
/// expected excess within the tolerance in absolute value, an exact check. The two routes at
/// 1 800 veh/h, given at most 200 iterations; the logit here, the deterministic choice below.
#[test]
fn a_run_continued_from_its_final_state_finds_nothing_and_moves_nobody_beyond_chance() {
    let the_logit = logit();
    continued_from_the_stop("logit", &the_logit, true);
}

/// [`a_run_continued_from_its_final_state_finds_nothing_and_moves_nobody_beyond_chance`] with
/// the deterministic choice. The row is a property of the rule's stop: it is asserted when the
/// rule stops within 200 iterations; otherwise where the run stood is printed and nothing is
/// asserted (on this case a quarter of the trips are still beyond the band at 200 iterations).
#[test]
fn a_deterministic_run_continued_from_its_final_state_moves_nobody_beyond_chance() {
    continued_from_the_stop("deterministic", &HighestUtility, false);
}

/// The two routes at 1 800 veh/h run with `model` until the rule stops them (at most 200
/// iterations), continued 3 iterations, checked as above; a run that does not stop fails the
/// row when `must_stop`, and is only printed otherwise.
fn continued_from_the_stop(name: &str, model: &dyn ChoiceModel, must_stop: bool) {
    let net = two_routes();
    let c = case(two_routes(), trips(&net, "o", "d", &departures(1800, 0, 3600)));
    let loader = PointQueue::after_last_departure(&c.turns, &c.trips, 3600.0);
    let params = Params { max_iterations: 200, ..Params::default() };
    let first = run_with(&c, &loader, &params, false, model, 2026);
    let stopped = first.reports.len() < params.max_iterations as usize;
    let inputs = Inputs {
        network: &c.net,
        turns: &c.turns,
        layers: openmobisim_core_assignment::Layers::NONE,
        transit: None,
        travellers: &c.travellers,
        trips: &c.trips,
        classes: &c.classes,
        mode_choice: &[],
        money_per_link: &c.money,
        model,
        loader: &loader,
        seed: RngKey::from_seed(2026),
        initial: Some(first.state.clone()),
    };
    let start = first.state.iterations();
    let clock = Instant::now();
    let (_, again) = assign(&inputs, &exactly(3), &mut Recorder::default()).expect("the run");
    let seconds = clock.elapsed().as_secs_f64();
    let last = first.reports.last().expect("reports");
    println!(
        "{name}: first run {} iterations ({:.3} s), stopped by the rule: {stopped}; expected \
         excess, last three {:.4}; mean trip {:.2} s against {:.2} s; continued 3 iterations \
         ({seconds:.3} s)",
        first.reports.len(),
        first.seconds,
        tail_mean(&first.reports, |x| x.switch_excess_expected),
        last.mean_trip_seconds,
        two_route_mean(1800)
    );
    for x in &again {
        println!(
            "  iteration {}: found {}, added {}, expected excess {:.5}, realised excess {:.4}, \
             above floor {:.5}, gap {:.5}",
            x.iteration,
            x.found,
            x.added,
            x.switch_excess_expected,
            x.switch_excess,
            x.switch_above_floor,
            x.duality_gap
        );
    }
    if !stopped && !must_stop {
        println!("{name}: no stop within 200 iterations; the row is not asserted");
        return;
    }
    assert!(stopped, "{name}: the first run stops by its own rule within 200 iterations");
    for (k, x) in again.iter().enumerate() {
        assert_eq!(x.iteration, start + k as u32, "{name}: the numbering goes on");
        assert_eq!(x.found, 0, "{name}, iteration {}: nothing beyond the band", x.iteration);
        assert!(
            x.switch_excess_expected.abs() <= params.switch_tolerance,
            "{name}, iteration {}: expected excess {} beyond {}",
            x.iteration,
            x.switch_excess_expected,
            params.switch_tolerance
        );
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
    braess_run_seeded(middle, per_hour, 2026)
}

fn braess_run_seeded(middle: bool, per_hour: u64, seed: u64) -> (SteadyState, Run) {
    let net = braess(middle);
    check_free_flow(&net, &braess_links(middle));
    let c = case(braess(middle), trips(&net, "s", "t", &departures(per_hour, 0, 3 * 3600)));
    let loader = PointQueue::after_last_departure(&c.turns, &c.trips, 3600.0);
    let r = run_with(&c, &loader, &exactly(100), false, &HighestUtility, seed);
    let s = steady_state(&c, &r);
    timing(&format!("Braess, middle link {middle}, {per_hour} veh/h, seed {seed}"), &r);
    for x in r.reports.iter().filter(|x| [0, 9, 24, 49, 99].contains(&x.iteration)) {
        println!(
            "  iteration {}: switch share {:.4}, excess {:.4}, found share {:.4}, duality gap \
             {:.5}, mean trip {:.1} s",
            x.iteration,
            x.switch_share,
            x.switch_excess,
            x.found_share,
            x.duality_gap,
            x.mean_trip_seconds
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
///
/// With the middle link, the queues, the shares and the trip times are judged on their mean
/// over master seeds 1 to 20. The discretised point-queue process is stochastic in who
/// reconsiders (the averaging rule's draws), so the theorem's steady state is its expectation:
/// one seed can sit 7 % off while the mean sits within 1 %. Twenty seeds, not five, so that the 2 % tolerance sits about three
/// standard errors from the mean rather than two (S312). The twenty values and their standard
/// deviation are printed.
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
    // With it, over master seeds 1 to 20.
    let runs: Vec<SteadyState> =
        (1..=20).map(|seed| braess_run_seeded(true, 1500, seed).0).collect();
    let derived = 19.0 * minute;
    let queue = 8.0 * minute;
    // The mean and the standard deviation (n − 1) of a value over the seeds, printed.
    let over_seeds = |name: &str, f: &dyn Fn(&SteadyState) -> f64| {
        let v: Vec<f64> = runs.iter().map(f).collect();
        let m = v.iter().sum::<f64>() / v.len() as f64;
        let sd = (v.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (v.len() - 1) as f64).sqrt();
        println!("  seeds 1–20, {name}: {v:.4?}; mean {m:.4}, standard deviation {sd:.4}");
        m
    };
    let mean_trip = over_seeds("mean trip (s)", &|s| s.mean_trip);
    assert!((mean_trip / derived - 1.0).abs() <= 0.02, "19 min within 2 %: {mean_trip} s");
    for route in ["sa-at", "sb-bt", "sa-ab-bt"] {
        let share = over_seeds(&format!("{route} share"), &|s| share_of(s, route).0);
        let mean = over_seeds(&format!("{route} mean trip (s)"), &|s| share_of(s, route).1);
        assert!((share - 1.0 / 3.0).abs() <= 0.03, "{route}: share {share}");
        assert!((mean / derived - 1.0).abs() <= 0.02, "{route}: {mean} s");
    }
    let sa = over_seeds("queue at s → a (s)", &|s| s.queue_sa);
    let bt = over_seeds("queue at b → t (s)", &|s| s.queue_bt);
    assert!((sa / queue - 1.0).abs() <= 0.05, "the queue at s → a: {sa} s");
    assert!((bt / queue - 1.0).abs() <= 0.05, "the queue at b → t: {bt} s");
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

// --- 9. the car's fixed times ---------------------------------------------------------------

/// The car's fixed times of these tests: 120 s to reach the car, 300 s to park and leave it.
const ACCESS_S: f64 = 120.0;
const EGRESS_S: f64 = 300.0;

fn with_fixed(params: Params) -> Params {
    Params { car_fixed_times: FixedTimes::new(ACCESS_S, EGRESS_S), ..params }
}

/// `o → m → d`, two 1 000-m links at 50 km/h (72 s each), three lanes of 6 000 veh/h.
fn two_link_links() -> [LinkDef; 2] {
    [
        def("om", "o", "m", LINE_M, LINE_KM_H, AMPLE_VEH_H, 3),
        def("md", "m", "d", LINE_M, LINE_KM_H, AMPLE_VEH_H, 3),
    ]
}

fn two_links() -> RoadNetwork {
    network(&[("o", 0.0, 0.0), ("m", 1000.0, 0.0), ("d", 2000.0, 0.0)], &two_link_links())
}

/// One car leaving `o` for `d` at 1 000 s on the empty two-link network, at free flow (level
/// 0) and by the point queue (level 2), with 120 s to reach the car and 300 s to leave it.
/// Derived: the drive is 2 × 1 000 m at 50 km/h = 144 s; the car sets off at 1 000 + 120 =
/// 1 120 s and arrives at 1 264 s; the trip ends at 1 564 s, door to door 120 + 144 + 300 =
/// 564 s. The chain's attributes: 144 s in the vehicle, 120 s walking, 300 s parking; its cost
/// for coefficients 1, 2 and 3 a minute in the vehicle, walking and parking: 144/60 + 2 ×
/// 120/60 + 3 × 300/60 = 21.4.
#[test]
fn one_car_pays_its_fixed_times_door_to_door() {
    let net = two_links();
    check_free_flow(&net, &two_link_links());
    let drive = 2.0 * LINE_M / metres_per_second(LINE_KM_H);
    assert!((drive - 144.0).abs() < 1e-9, "the case is as stated");
    let mut c = case(two_links(), trips(&net, "o", "d", &[1000]));
    let k = Coefficients {
        in_vehicle_per_minute: 1.0,
        walk_per_minute: 2.0,
        parking_per_minute: 3.0,
        ..Coefficients::time_based()
    };
    c.classes = vec![k];
    let point_queue = PointQueue::new(&c.turns, 10_000.0);
    let free_flow = FreeFlow::default();
    for (level, loader) in [(0, &free_flow as &dyn Loader), (2, &point_queue)] {
        let r = run(&c, loader, &with_fixed(exactly(1)), false);
        let (_, departure) = r.recorder.vehicles[0];
        let arrival = r.recorder.arrivals[0].expect("arrived");
        let report = &r.reports[0];
        let chain = r.state.trip(TripId::new(0)).current_chain().expect("a chain");
        let a = chain.total();
        println!(
            "level {level}: car sets off {departure} s, arrives {arrival} s; door to door {} s; \
             attributes {a:?}; cost {}",
            report.mean_trip_seconds,
            k.cost(a)
        );
        assert_eq!(departure, 1120, "level {level}: the car sets off once reached");
        assert_eq!(arrival, 1120.0 + drive, "level {level}: the drive at free flow");
        assert_eq!(report.mean_trip_seconds, ACCESS_S + drive + EGRESS_S, "level {level}");
        assert_eq!(report.arrived, 1.0);
        assert_eq!(a.in_vehicle_seconds, drive);
        assert_eq!(a.walk_seconds, ACCESS_S);
        assert_eq!(a.parking_seconds, EGRESS_S);
        assert_eq!(a.total_seconds(), 564.0);
        assert!((k.cost(a) - 21.4).abs() < 1e-12);
        assert_eq!(chain.legs()[0].vehicle_departure(), 1120.0);
    }
}

/// A trip ends when its car is parked and left: a car arriving before the loading's end whose
/// trip would end after it is unfinished. Derived: setting off at 1 120 s, the car arrives at
/// 1 264 s, before the end at 1 400 s; the trip would end at 1 564 s, after it.
#[test]
fn a_trip_whose_egress_ends_after_the_window_is_unfinished() {
    let net = two_links();
    let c = case(two_links(), trips(&net, "o", "d", &[1000]));
    let point_queue = PointQueue::new(&c.turns, 1400.0);
    let free_flow = FreeFlow { end_seconds: 1400.0 };
    for (level, loader) in [(0, &free_flow as &dyn Loader), (2, &point_queue)] {
        let r = run(&c, loader, &with_fixed(exactly(1)), false);
        assert_eq!(r.recorder.arrivals[0], Some(1264.0), "level {level}: the car arrived");
        let report = &r.reports[0];
        assert_eq!((report.departed, report.arrived, report.unfinished), (1.0, 0.0, 1.0));
    }
}

/// The fixed times change no route: every route of a leg costs the same fixed amount more, so
/// the route of least cost from the car's departure `t + 120` is the leg's. Checked on the
/// states of a congested run of the two routes (1 800 veh/h for an hour, five iterations), where
/// the link times change from bin to bin: for every trip and state, the planner with the fixed
/// times plans at `t` the route the planner without them plans at `t + 120`; with the egress
/// time alone, the route it plans at `t`; and every trip's search over the shared searches
/// agrees, link for link.
#[test]
fn the_fixed_times_change_no_route() {
    let net = two_routes();
    let c = case(two_routes(), trips(&net, "o", "d", &departures(1800, 0, 3600)));
    let loader = PointQueue::after_last_departure(&c.turns, &c.trips, 3600.0);
    let r = run(&c, &loader, &exactly(5), true);
    let plain = CarPlanner::new(&c.net, &c.turns, &c.classes, &c.money).expect("a planner");
    let both = CarPlanner::new(&c.net, &c.turns, &c.classes, &c.money)
        .and_then(|p| p.with_fixed_times(FixedTimes::new(ACCESS_S, EGRESS_S)))
        .expect("a planner");
    let egress = CarPlanner::new(&c.net, &c.turns, &c.classes, &c.money)
        .and_then(|p| p.with_fixed_times(FixedTimes::new(0.0, EGRESS_S)))
        .expect("a planner");
    let (o, d) = (Place::Road(node(&net, "o")), Place::Road(node(&net, "d")));
    let class = UserClassId::new(0);
    let (mut compared, mut long) = (0, 0);
    for state in &r.recorder.states {
        for t in 0..c.trips.len() {
            let at = f64::from(c.trips.departure(TripId::new(t)).get());
            let plan = |p: &CarPlanner<'_>, at: f64| {
                p.plan(o, d, at, class, state, CostSource::Exact).expect("a route")
            };
            let with = plan(&both, at);
            assert_eq!(with.links, plan(&plain, at + ACCESS_S).links, "trip {t}: access");
            assert_eq!(
                with.attributes.in_vehicle_seconds,
                plan(&plain, at + ACCESS_S).attributes.in_vehicle_seconds
            );
            assert_eq!(plan(&egress, at).links, plan(&plain, at).links, "trip {t}: egress");
            long += usize::from(with.links[0] == link(&c.net, "ob"));
            compared += 1;
        }
    }
    println!(
        "{compared} plans compared over {} states, {long} by the long route",
        r.recorder.states.len()
    );
    assert!(long > 0 && long < compared, "both routes are cheapest somewhere: the test bites");
    // The search stage (shared searches) gives each trip the leg its own plan gives.
    let fixed = with_fixed(exactly(5));
    let search = ChainSearch::new(&c.net, &c.travellers, &c.trips, both, &fixed);
    for state in &r.recorder.states {
        let all = search.search_all(state, &fixed);
        for (t, chains) in all.iter().enumerate() {
            let trip = TripId::new(t as u32);
            let own = search.search(c.trips.traveller(trip), trip, state, &fixed);
            assert_eq!(chains, &own, "trip {t}: the shared search differs");
        }
    }
}

/// The bottleneck of test 1 at 2 000 veh/h for 30 min, every car reaching its car for 120 s
/// first and leaving it for 300 s after: the deterministic queue is invariant to a shift of
/// every arrival by the same time, so every car meets the queue of test 1 120 s later. Derived:
/// each car reaches the bottleneck's downstream end at `d + 120 + 2 × 72 s`; its queue wait is
/// Lindley's recursion over those times, whose total is the same as without the access time
/// (every term shifts by 120 s) and within 3 % of `½ (λ − μ) T T'`; each trip's time is its
/// drive plus 420 s; and each car's drive is, to the second, its drive without the fixed times
/// (the time-shift invariance of the loading, a property).
#[test]
fn the_access_time_shifts_the_bottleneck_queue_and_leaves_its_delay() {
    let (t, per_hour) = (1800.0, 2000_u64);
    let (lambda, mu) = (per_hour as f64 / 3600.0, BOTTLENECK_VEH_H / 3600.0);
    let link_s = LINE_M / metres_per_second(LINE_KM_H);
    let net = line();
    let deps = departures(per_hour, 0, 1800);
    let c = case(line(), trips(&net, "o", "d", &deps));
    let loader = PointQueue::after_last_departure(&c.turns, &c.trips, 3600.0 + ACCESS_S + EGRESS_S);
    let shifted = run(&c, &loader, &with_fixed(exactly(1)), false);
    let plain = run(&c, &loader, &exactly(1), false);
    let free = 3.0 * link_s;
    let drives: Vec<f64> = shifted.recorder.trips().iter().map(|&(_, _, d)| d).collect();
    let delay: f64 = drives.iter().map(|d| d - free).sum();
    let at_server: Vec<f64> =
        deps.iter().map(|&d| f64::from(d) + ACCESS_S + 2.0 * link_s).collect();
    let (by_vehicle, _) = queue_delay_by_vehicle(&at_server, mu);
    let at_server_plain: Vec<f64> = deps.iter().map(|&d| f64::from(d) + 2.0 * link_s).collect();
    let (by_vehicle_plain, _) = queue_delay_by_vehicle(&at_server_plain, mu);
    let formula = queue_delay(lambda, mu, t);
    let n = deps.len() as f64;
    println!(
        "access 120 s: total delay {delay:.1} veh·s; Lindley {by_vehicle:.1} (without the access \
         time {by_vehicle_plain:.1}); fluid formula {formula:.1}; mean trip {:.2} s",
        shifted.reports[0].mean_trip_seconds
    );
    timing("bottleneck with fixed times", &shifted);
    assert!((by_vehicle - by_vehicle_plain).abs() <= 1e-6 * by_vehicle, "Lindley shifts");
    assert!((delay / formula - 1.0).abs() <= 0.03, "within 3 % of ½ (λ − μ) T T'");
    assert!(delay <= by_vehicle + 1e-6 && delay >= by_vehicle - n, "floored to the second");
    for (k, &(_, d, _)) in shifted.recorder.trips().iter().enumerate() {
        assert_eq!(d, f64::from(deps[k]) + ACCESS_S, "car {k} sets off once reached");
    }
    let plain_drives: Vec<f64> = plain.recorder.trips().iter().map(|&(_, _, d)| d).collect();
    assert_eq!(drives, plain_drives, "every drive the same, shifted by 120 s");
    let mean_drive = drives.iter().sum::<f64>() / n;
    assert!(
        (shifted.reports[0].mean_trip_seconds - (mean_drive + ACCESS_S + EGRESS_S)).abs() < 1e-6
    );
}

// --- 10. a state rebuilt from what a run wrote -------------------------------------------

/// A state rebuilt from what a run wrote of its final state (every trip's chains as links, its
/// current chain, its probabilities, and the count of iterations) equals that state to the bit,
/// link travel times included, because the loading of the current chains is deterministic; and
/// a run continued from it gives the same reports as one continued from the state itself. The
/// two routes at 1 800 veh/h, the logit and the deterministic choice, without and with the
/// fixed times.
#[test]
fn a_state_rebuilt_from_what_a_run_wrote_is_the_state() {
    let net = two_routes();
    let c = case(two_routes(), trips(&net, "o", "d", &departures(1800, 0, 3600)));
    let loader = PointQueue::after_last_departure(&c.turns, &c.trips, 3600.0);
    let the_logit = logit();
    for (name, model) in
        [("logit", &the_logit as &dyn ChoiceModel), ("deterministic", &HighestUtility)]
    {
        for params in [exactly(8), with_fixed(exactly(8))] {
            let first = run_with(&c, &loader, &params, false, model, 2026);
            let saved: Vec<SavedTrip> = first.state.trips().iter().map(SavedTrip::of).collect();
            let mut inputs = Inputs {
                network: &c.net,
                turns: &c.turns,
                layers: openmobisim_core_assignment::Layers::NONE,
                transit: None,
                travellers: &c.travellers,
                trips: &c.trips,
                classes: &c.classes,
                mode_choice: &[],
                money_per_link: &c.money,
                model,
                loader: &loader,
                seed: RngKey::from_seed(2026),
                initial: None,
            };
            let rebuilt =
                restore(&inputs, &params, &saved, first.state.iterations()).expect("rebuilt");
            assert!(rebuilt == first.state, "{name}: the rebuilt state is the state");
            assert_eq!(rebuilt.iterations(), 8);
            let mut go_on = |from: State| {
                inputs.initial = Some(from);
                let (s, reports) =
                    assign(&inputs, &exactly(2), &mut Recorder::default()).expect("run");
                (s, reports.iter().map(IterationReport::without_times).collect::<Vec<_>>())
            };
            let (a, ra) = go_on(first.state.clone());
            let (b, rb) = go_on(rebuilt);
            assert!(a == b && ra == rb, "{name}: continued alike");
            assert_eq!(ra[0].iteration, 8, "{name}: the numbering goes on");
            println!(
                "{name}, fixed times {:?}: rebuilt equal; continued iterations 8–9 alike (found {}, {})",
                params.car_fixed_times, ra[0].found, ra[1].found
            );
        }
    }
}

/// What a rebuilt state refuses: chains whose links do not join the trip's ends, a chain twice,
/// a current chain outside the set, a state for other demand.
#[test]
fn a_saved_state_that_does_not_fit_is_refused() {
    let net = two_routes();
    let c = case(two_routes(), trips(&net, "o", "d", &[0, 10]));
    let loader = FreeFlow::default();
    let inputs = Inputs {
        network: &c.net,
        turns: &c.turns,
        layers: openmobisim_core_assignment::Layers::NONE,
        transit: None,
        travellers: &c.travellers,
        trips: &c.trips,
        classes: &c.classes,
        mode_choice: &[],
        money_per_link: &c.money,
        model: &HighestUtility,
        loader: &loader,
        seed: RngKey::from_seed(1),
        initial: None,
    };
    let short = links(&c.net, &["oa", "ad"]);
    let good = SavedTrip {
        chains: vec![short.clone()],
        modes: vec![],
        current: Some(0),
        probabilities: vec![1.0],
    };
    let params = Params::default();
    assert!(restore(&inputs, &params, &[good.clone(), good.clone()], 3).is_ok());
    let cases = [
        ("broken", SavedTrip { chains: vec![links(&c.net, &["oa", "bd"])], ..good.clone() }),
        ("half", SavedTrip { chains: vec![links(&c.net, &["oa"])], ..good.clone() }),
        ("twice", SavedTrip { chains: vec![short.clone(), short.clone()], ..good.clone() }),
        ("outside", SavedTrip { current: Some(1), ..good.clone() }),
        ("probabilities", SavedTrip { probabilities: vec![0.5, 0.5], ..good.clone() }),
    ];
    for (name, bad) in cases {
        let e = restore(&inputs, &params, &[good.clone(), bad], 3);
        println!("{name}: {e:?}");
        assert!(matches!(e, Err(AssignError::Input(_))), "{name}");
    }
    assert!(restore(&inputs, &params, &[good], 3).is_err(), "one trip for two");
}

// --- 11. a shift in time under the FIFO reading -------------------------------------------

/// **Property, from the model's definition:** nothing in the loop depends on the hour, so a run
/// whose departures and window are shifted by 3 600 s gives every trip the same route and the
/// same trip time, under the FIFO reading of the link times as under the bins. A 5 × 5 grid of
/// 200-m blocks (1 800 veh/h a link), 3 000 cars an hour from each corner to the opposite one
/// for an hour, the logit, 12 iterations, from 8 h and from 9 h (the clock passes 2¹⁵ s =
/// 9.1 h in the later run only, so its seconds round at another magnitude). Before S312 the
/// reading formed the exit second at the clock's magnitude, its last bits depended on the
/// hour, and routes of equal cost were told apart differently: trips changed by minutes.
#[test]
fn a_run_shifted_by_an_hour_gives_the_same_trips_under_the_fifo_reading() {
    const RATE: u64 = 3000;
    let corners = [((0, 0), (4, 4)), ((4, 4), (0, 0)), ((0, 4), (4, 0)), ((4, 0), (0, 4))];
    let the_logit = logit();
    let run_from = |start: u32| {
        let (net, _) = manhattan_grid(5, BLOCK_M, false);
        let mut raw = Vec::new();
        for ((r0, c0), (r1, c1)) in corners {
            let from = node_name(r0, c0);
            let to = node_name(r1, c1);
            raw.extend(trips(&net, &from, &to, &departures(RATE, start, 3600)));
        }
        for (i, t) in raw.iter_mut().enumerate() {
            t.traveller_id = format!("t{i:05}");
        }
        let c = case(net, raw);
        // No flag: under the FIFO reading the assignment asks the loading for its entrants.
        let loader = PointQueue::new(&c.turns, f64::from(start) + 4.0 * 3600.0);
        let params = Params { link_times: LinkTimesReading::Fifo, ..exactly(12) };
        let r = run_with(&c, &loader, &params, false, &the_logit, 2026);
        timing(&format!("grid from {start} s"), &r);
        r.recorder
            .trips()
            .into_iter()
            .map(|(route, d, t)| (route.to_vec(), d - f64::from(start), t))
            .collect::<Vec<_>>()
    };
    let early = run_from(8 * 3600);
    let late = run_from(9 * 3600);
    let mean = early.iter().map(|&(_, _, t)| t).sum::<f64>() / early.len() as f64;
    let free = 8.0 * BLOCK_M / metres_per_second(GRID_KM_H);
    println!("mean trip {mean:.1} s against {free:.1} s at free flow");
    assert!(mean > 1.5 * free, "the grid is congested: {mean} s");
    let changed = early.iter().zip(&late).filter(|(a, b)| a != b).count();
    let worst = early.iter().zip(&late).map(|(a, b)| (a.2 - b.2).abs()).fold(0.0, f64::max);
    assert_eq!(changed, 0, "{changed} of {} trips changed, by up to {worst} s", early.len());
}

/// A loader that records its table without the entrants whatever it is asked: refused under the
/// FIFO reading (it would be read by the floor unnoticed), run under the bins reading.
#[test]
fn the_fifo_reading_refuses_a_loading_without_its_entrants() {
    struct Plain<'a>(PointQueue<'a>);
    impl Loader for Plain<'_> {
        fn load(&self, vehicles: &[Vehicle], network: &RoadNetwork) -> Loaded {
            self.0.load(vehicles, network)
        }
    }
    let net = two_routes();
    let c = case(two_routes(), trips(&net, "o", "d", &departures(1800, 0, 600)));
    let loader = Plain(PointQueue::after_last_departure(&c.turns, &c.trips, 3600.0));
    let inputs = Inputs {
        network: &c.net,
        turns: &c.turns,
        layers: openmobisim_core_assignment::Layers::NONE,
        transit: None,
        travellers: &c.travellers,
        trips: &c.trips,
        classes: &c.classes,
        mode_choice: &[],
        money_per_link: &c.money,
        model: &HighestUtility,
        loader: &loader,
        seed: RngKey::from_seed(1),
        initial: None,
    };
    let fifo = Params { link_times: LinkTimesReading::Fifo, ..exactly(2) };
    let e = assign(&inputs, &fifo, &mut Recorder::default()).map(|_| ());
    assert!(matches!(&e, Err(AssignError::Input(m)) if m.contains("entrant")), "{e:?}");
    let bins = Params { link_times: LinkTimesReading::Bins, ..exactly(2) };
    assert!(assign(&inputs, &bins, &mut Recorder::default()).is_ok());
}
