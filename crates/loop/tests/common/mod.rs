//! Fixtures shared by the loop tests: networks from small tables, demands at constant rates,
//! time-only parameters, and readers of a run's tables.

#![allow(dead_code, reason = "each test binary uses a different part of this module")]
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "counts of trips and seconds of the window, far below 2^53 and above zero"
)]

use std::collections::HashMap;

use openmobisim_demand::demand::Demand;
use openmobisim_demand::input::TripRow;
use openmobisim_graph::geometry::{Position, Xy};
use openmobisim_graph::network::{BuildValues, LinkRow, Network, NetworkBuilder};
use openmobisim_loop::alternative::Leg;
use openmobisim_loop::parameters::{COLUMNS, ChoiceModel, Parameters};
use openmobisim_loop::run::Run;
use openmobisim_types::ids::{EntityId, LinkId, TripId};
use openmobisim_types::mode::{Mode, ModeSet};
use openmobisim_types::time::Second;
use openmobisim_types::units::Metres;

/// The default per-lane capacity (veh/h), bike speed and walking speed (km/h) of `PARAMETERS.md`.
pub const VALUES: BuildValues =
    BuildValues { capacity_per_lane: 1_800.0, bike_speed: 15.0, walk_speed: 4.8 };

/// A car-only link of one lane: `length_m` at `speed_km_h` with `capacity_veh_h`.
#[must_use]
pub fn car_link(length_m: f64, speed_km_h: f64, capacity_veh_h: f64) -> LinkRow {
    LinkRow {
        modes: ModeSet::of(&[Mode::Car]),
        lanes: Some(1),
        car_speed_km_h: Some(speed_km_h),
        length_m: Some(length_m),
        capacity_veh_h: Some(capacity_veh_h),
        signal_at_downstream_end: false,
    }
}

/// A point in metres.
#[must_use]
pub fn at(x: f64, y: f64) -> Position {
    Position::Metres(Xy::new(x, y))
}

/// A network from a node table (id, x, y) and a link table (id, upstream, downstream, row).
///
/// # Panics
///
/// If the build fails.
#[must_use]
pub fn network(nodes: &[(&str, f64, f64)], links: &[(&str, &str, &str, LinkRow)]) -> Network {
    let mut b = NetworkBuilder::new();
    for &(id, x, y) in nodes {
        b.add_node(id, at(x, y));
    }
    for (id, up, down, row) in links {
        b.add_link(*id, *up, *down, *row);
    }
    b.build(VALUES).expect("a network")
}

/// Car trips from `origin` to `destination` (node ids) at `rate_per_hour` for `hours`, spaced
/// regularly from `from` (whole seconds, floored), named `prefix{i}`.
#[must_use]
pub fn stream(
    network: &Network,
    prefix: &str,
    origin: &str,
    destination: &str,
    rate_per_hour: f64,
    hours: f64,
    from: Second,
) -> Vec<TripRow> {
    let (o, d) = (node_xy(network, origin), node_xy(network, destination));
    let count = (rate_per_hour * hours).round() as usize;
    (0..count)
        .map(|i| TripRow {
            id: format!("{prefix}{i:06}"),
            origin: at(o.0, o.1),
            destination: at(d.0, d.1),
            departure: from.saturating_add((i as f64 * 3_600.0 / rate_per_hour).floor() as u32),
            class: "default".to_string(),
            modes: ModeSet::of(&[Mode::Car]),
        })
        .collect()
}

/// A node's position in metres.
#[must_use]
pub fn node_xy(network: &Network, id: &str) -> (f64, f64) {
    let node = network.node_ids().typed_id_of(id).expect("a node");
    let p = network.node_position(node);
    (p.x, p.y)
}

/// A demand of one class, no timetable, from `rows`.
///
/// # Panics
///
/// If the build fails.
#[must_use]
pub fn demand(network: &Network, rows: &[TripRow]) -> Demand {
    Demand::build(rows, &["default".to_string()], network, None, Metres(1_500.0)).expect("a demand")
}

/// The coefficients of a time-only run: 1 on each of the nine time columns (minutes), 0 on the
/// transfers, money, path size and the mode constants, so a cost is the door-to-door time in
/// minutes.
pub const TIME_ONLY: [f64; 16] =
    [1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];

/// Time-only parameters with the deterministic choice: cost in minutes, door to door,
/// `link_bin` 60 s, the cap `iterations`, seed 0.
#[must_use]
pub fn time_only(iterations: u32) -> Parameters {
    assert!(COLUMNS[..9].iter().all(|c| c.ends_with("_min")), "the time columns come first");
    Parameters {
        link_bin: 60,
        iteration_cap: iterations,
        choice: ChoiceModel::Deterministic,
        coefficients: vec![TIME_ONLY],
        ..Parameters::defaults(1)
    }
}

/// A link's id by its external id.
#[must_use]
pub fn link(network: &Network, id: &str) -> LinkId {
    network.link_ids().typed_id_of(id).expect("a link")
}

/// The trips (by index) that departed in `[from, to)` seconds of the clock and finished.
#[must_use]
pub fn departed_in(run: &Run, from: u32, to: u32) -> Vec<usize> {
    (0..run.trips.departure.len())
        .filter(|&i| {
            let d = run.trips.departure[i].get();
            d >= from && d < to && run.trips.finished[i]
        })
        .collect()
}

/// The mean realised travel time, seconds, over `trips` (by index).
#[must_use]
pub fn mean_time(run: &Run, trips: &[usize]) -> f64 {
    trips.iter().map(|&i| run.trips.travel_time[i]).sum::<f64>() / trips.len() as f64
}

/// Whether trip `i` took a route through `link`.
#[must_use]
pub fn took(run: &Run, i: usize, link: LinkId) -> bool {
    run.taken(TripId::from_index(i)).and_then(|a| a.route()).is_some_and(|r| r.contains(&link))
}

/// The share of `trips` that took a route through `link`.
#[must_use]
pub fn share_via(run: &Run, trips: &[usize], link: LinkId) -> f64 {
    trips.iter().filter(|&&i| took(run, i, link)).count() as f64 / trips.len() as f64
}

/// The cars that entered `link` in the bins starting in `[from, to)` seconds of the clock, and
/// their mean crossing time in seconds.
#[must_use]
pub fn link_cars(run: &Run, link: LinkId, from: u32, to: u32) -> (u32, f64) {
    let w = run.links.window();
    let (mut cars, mut time) = (0_u32, 0.0);
    for k in 0..run.links.len() {
        let start = w.step_start(run.links.bins()[k]).get();
        if run.links.links()[k] == link && start >= from && start < to {
            cars += run.links.cars()[k];
            time += run.links.car_time()[k].get();
        }
    }
    (cars, time / f64::from(cars))
}

/// Part 1's line, seconds, on both of its differences: the replay and the record are exact
/// (instants are never rounded), so anything above a rounding error is a fault.
pub const PART_1_S: f64 = 1e-6;

/// The larger of `a` and `b`, `NaN` when either is: unlike `f64::max`, a `NaN` is never
/// dropped, so a fold of part 1's differences fails on it.
#[must_use]
pub fn larger(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() { f64::NAN } else { a.max(b) }
}

/// What part 1 found: the queue on its links, and the record along every car's route.
#[derive(Clone, Copy, Debug)]
pub struct QueueReplay {
    /// The largest difference between a recorded exit and the replayed one, seconds; `NaN`
    /// when an entry or an exit is `NaN`.
    pub largest_s: f64,
    /// The link of that largest difference; none without cars.
    pub on: Option<LinkId>,
    /// The links replayed.
    pub links: usize,
    /// The cars replayed.
    pub cars: usize,
    /// Of them, the cars the replay holds behind the car ahead: those that met a queue.
    pub queued: usize,
    /// The largest gap, seconds, between where a car's record says it entered a link and where
    /// its route says it did: its first link at its departure plus the walk before its route,
    /// each next link at its exit from the one before; `NaN` when one of them is `NaN`.
    pub gap_s: f64,
    /// The cars of the last loading followed along their routes.
    pub routed: usize,
    /// The records missing (a link of a route entered before the window's end without its
    /// record), given twice, or of no car's route.
    pub misrecorded: usize,
}

impl QueueReplay {
    /// One line for a message: the two differences, where, over how many cars.
    #[must_use]
    pub fn describe(&self, network: &Network) -> String {
        let on =
            self.on.map_or("none".to_string(), |l| network.link_ids().external(l.raw()).into());
        format!(
            "largest difference from the replayed point queue {:.2e} s (on link {on}), over {} cars on {} links, {} of them queued; record along the routes of {} cars: largest gap {:.2e} s, {} records missing, twice or off every route",
            self.largest_s,
            self.cars,
            self.links,
            self.queued,
            self.routed,
            self.gap_s,
            self.misrecorded
        )
    }

    /// Judge part 1: some car replayed, both differences within [`PART_1_S`] (a `NaN` fails),
    /// no record missing, twice or off every route; and, when the test's theory has a queue,
    /// some car held behind another.
    ///
    /// # Panics
    ///
    /// When one of them fails.
    #[track_caller]
    pub fn judge(&self, network: &Network, theory_has_a_queue: bool) {
        let text = self.describe(network);
        assert!(self.cars > 0, "part 1, queue: no car in the record: {text}");
        assert!(
            !theory_has_a_queue || self.queued > 0,
            "part 1, queue: the theory has a queue and no car met one: {text}"
        );
        assert!(self.largest_s <= PART_1_S, "part 1, queue: {text}");
        assert!(self.gap_s <= PART_1_S, "part 1, record: {text}");
        assert!(self.misrecorded == 0, "part 1, record: {text}");
    }
}

/// Part 1 of an equilibrium test, the queue and the record, from the last loading's record of
/// every car crossing (`run.links.crossings()`): entry, exit and trip per link.
///
/// The queue: given the cars that entered each of `links` and when, is each car's exit right?
/// The point queue's recursion (Vickrey 1969) is replayed here from the loading's contract, not
/// by calling the loading: a link's cars in the order of their entry, of two entering at one
/// instant the lower trip first; each leaves at the later of its entry plus the link's car
/// free-flow time and one headway (1 / capacity, 0 at an infinite capacity) after the exit of
/// the car ahead. `largest_s` is the largest difference from the recorded exits.
///
/// The record: is every car of the last loading in it where its route puts it? By the loop's
/// contract a car's route is entered at its departure plus the walks before the route, and by
/// the loading's it enters each next link at its exit from the one before; a crossing is
/// recorded when it starts before the window's end. `gap_s` is the largest gap between those
/// instants and the recorded entries; `misrecorded` counts the links of a route entered before
/// the end without their record, the records given twice and the records of no car's route.
///
/// A pass means each link's queue and the hand-over between links are right for the cars
/// recorded; a difference is a fault of the loading or of its record, never of the re-choice.
/// Bikes and walkers ride at free flow and are not in the record; a car on a link without a
/// queue leaves at free flow, so `queued` counts the cars that matter, those the replay holds
/// behind another.
#[must_use]
pub fn replay_queues(network: &Network, run: &Run, links: &[LinkId]) -> QueueReplay {
    let (start, entry, exit, trip) = run.links.crossings();
    let mut out = QueueReplay {
        largest_s: 0.0,
        on: None,
        links: links.len(),
        cars: 0,
        queued: 0,
        gap_s: 0.0,
        routed: 0,
        misrecorded: 0,
    };
    for &l in links {
        // A table no loading filled has no offsets; a link no car entered has no crossings.
        if start.len() < l.index() + 2 || start[l.index()] == start[l.index() + 1] {
            continue;
        }
        let (from, to) = (start[l.index()] as usize, start[l.index() + 1] as usize);
        let mut cars: Vec<(f64, TripId, f64)> =
            (from..to).map(|k| (entry[k], trip[k], exit[k])).collect();
        cars.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        let free = network.free_flow_time(l, Mode::Car).get();
        let headway = 1.0 / network.capacity(l).get();
        let mut ahead = f64::NEG_INFINITY;
        for (entered, _, recorded) in cars {
            let replayed = (entered + free).max(ahead + headway);
            out.queued += usize::from(replayed > entered + free);
            out.cars += 1;
            // `f64::max` drops a NaN entry: a NaN entry or exit is a NaN difference.
            let difference = if entered.is_nan() { f64::NAN } else { (recorded - replayed).abs() };
            if out.on.is_none() || difference.is_nan() || difference > out.largest_s {
                out.on = Some(l);
            }
            out.largest_s = larger(out.largest_s, difference);
            ahead = replayed;
        }
    }
    record_along_routes(run, &mut out);
    out
}

/// The record part of [`replay_queues`]: every car of the last loading followed along its
/// route, its records taken from those of `(link, trip)` in the order of entry.
fn record_along_routes(run: &Run, out: &mut QueueReplay) {
    let (start, entry, exit, trip) = run.links.crossings();
    let mut records: HashMap<(LinkId, TripId), Vec<(f64, f64)>> = HashMap::new();
    for l in 0..start.len().saturating_sub(1) {
        for k in start[l] as usize..start[l + 1] as usize {
            records.entry((LinkId::from_index(l), trip[k])).or_default().push((entry[k], exit[k]));
        }
    }
    for v in records.values_mut() {
        v.sort_by(|a, b| a.0.total_cmp(&b.0));
        v.reverse();
    }
    let window = run.links.window();
    let (origin, end) =
        (window.origin().get(), f64::from(window.end().get() - window.origin().get()));
    for i in 0..run.trips.departure.len() {
        let t = TripId::from_index(i);
        let Some(taken) = run.taken(t) else { continue };
        let mut route = None;
        let mut at = f64::from(run.trips.departure[i].get() - origin);
        for leg in &taken.legs {
            match leg {
                Leg::Walk(d) => at += d.get(),
                Leg::Route { mode: Mode::Car, links } if !links.is_empty() => {
                    route = Some(links);
                    break;
                }
                _ => break,
            }
        }
        let Some(route) = route else { continue };
        out.routed += 1;
        for &l in route {
            // No crossing starts at or after the window's end (a NaN instant is not skipped).
            if at >= end {
                break;
            }
            let Some((entered, left)) = records.get_mut(&(l, t)).and_then(Vec::pop) else {
                out.misrecorded += 1;
                break;
            };
            out.gap_s = larger(out.gap_s, (entered - at).abs());
            at = left;
        }
    }
    out.misrecorded += records.values().map(Vec::len).sum::<usize>();
}

/// Every link of `network`, for part 1 on a whole network.
#[must_use]
pub fn all_links(network: &Network) -> Vec<LinkId> {
    LinkId::iter_space(network.link_count()).collect()
}

/// Part 3 under the deterministic choice, printed, not judged: per trip the time it took less
/// the least time of its own set, in minutes, as the last measure reads them (`NaN` for an
/// unfinished trip); printed, how many are over 0.5 min and over 5 min, the largest and the
/// mean. A traveller far from its best holds a stale choice: the re-choice rule or the
/// iteration count, not the queue.
#[must_use]
pub fn far_from_best(run: &Run) -> Vec<f64> {
    let excess: Vec<f64> = (0..run.trips.finished.len())
        .map(|i| {
            if !run.trips.finished[i] {
                return f64::NAN;
            }
            let trip = TripId::from_index(i);
            let best = run.set(trip).iter().map(|a| a.time).fold(f64::INFINITY, f64::min);
            (run.taken(trip).map_or(best, |a| a.time) - best) / 60.0
        })
        .collect();
    let finished: Vec<f64> = excess.iter().copied().filter(|x| !x.is_nan()).collect();
    println!(
        "part 3, choices (printed, not judged): travellers over 0.5 min from the best of their set {}; over 5 min {}; the largest {:.1} min; the mean {:.3} min",
        finished.iter().filter(|&&x| x > 0.5).count(),
        finished.iter().filter(|&&x| x > 5.0).count(),
        finished.iter().copied().fold(0.0, f64::max),
        finished.iter().sum::<f64>() / finished.len() as f64
    );
    excess
}

/// The seeds a fast equilibrium test is judged over: its quantities against theory on their
/// mean over seeds 0 to 4, since one seed scatters about as much as the tolerances.
pub const SEEDS: u64 = 5;

/// The mean and its standard error (the standard deviation over `√n`) of `v`.
#[must_use]
pub fn mean_se(v: &[f64]) -> (f64, f64) {
    let n = v.len() as f64;
    let mean = v.iter().sum::<f64>() / n;
    let var = v.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1.0);
    (mean, (var / n).sqrt())
}

/// The mean of `v`, one value per seed from seed 0, printed with each seed's value and the
/// standard error of the mean under `label`.
#[must_use]
pub fn seed_mean(label: &str, v: &[f64]) -> f64 {
    let (mean, se) = mean_se(v);
    let each: Vec<String> = v.iter().map(|x| format!("{x:.3}")).collect();
    println!("{label}: seeds 0 to {} {}; mean {mean:.3} ± {se:.3}", v.len() - 1, each.join(", "));
    mean
}

/// Print the iterations table (run with `--nocapture` to see it).
pub fn print_iterations(run: &Run) {
    println!(
        "{:>3} {:>10} {:>7} {:>5} {:>7} {:>7} {:>7} {:>8} {:>7}",
        "it", "mean_s", "fin", "unf", "offer", "switch", "found", "gap", "sec"
    );
    for it in &run.iterations {
        println!(
            "{:>3} {:>10.2} {:>7} {:>5} {:>7.4} {:>7.4} {:>7.4} {:>8.5} {:>7.3}",
            it.iteration,
            it.mean_trip_time,
            it.finished,
            it.unfinished,
            it.offered_share,
            it.switch_share,
            it.found_share,
            it.gap,
            it.stage_seconds.iter().sum::<f64>()
        );
    }
    println!("stopped by the rule: {}", run.stopped_by_rule);
}
