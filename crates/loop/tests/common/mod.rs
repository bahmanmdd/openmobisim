//! Fixtures shared by the loop tests: networks from small tables, demands at constant rates,
//! time-only parameters, and readers of a run's tables.

#![allow(dead_code, reason = "each test binary uses a different part of this module")]
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "counts of trips and seconds of the window, far below 2^53 and above zero"
)]

use openmobisim_demand::demand::Demand;
use openmobisim_demand::input::TripRow;
use openmobisim_graph::geometry::{Position, Xy};
use openmobisim_graph::network::{BuildValues, LinkRow, Network, NetworkBuilder};
use openmobisim_loop::parameters::{ChoiceModel, Parameters};
use openmobisim_loop::run::Run;
use openmobisim_types::ids::{EntityId, LinkId, TripId};
use openmobisim_types::mode::{Mode, ModeSet};
use openmobisim_types::time::Second;
use openmobisim_types::units::Metres;

/// The build values of the rulings.
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

/// Time-only parameters with the deterministic choice: cost in seconds, `link_bin` 60 s, the
/// cap `iterations`, seed 0.
#[must_use]
pub fn time_only(iterations: u32) -> Parameters {
    Parameters {
        link_bin: 60,
        iteration_cap: iterations,
        choice: ChoiceModel::Deterministic,
        coefficients: vec![[1.0, 0.0, 0.0]],
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

/// Print the iterations table (run with `--nocapture` to see it).
pub fn print_iterations(run: &Run) {
    println!(
        "{:>3} {:>10} {:>7} {:>5} {:>7} {:>7} {:>8} {:>7}",
        "it", "mean_s", "fin", "unf", "switch", "found", "gap", "sec"
    );
    for it in &run.iterations {
        println!(
            "{:>3} {:>10.2} {:>7} {:>5} {:>7.4} {:>7.4} {:>8.5} {:>7.3}",
            it.iteration,
            it.mean_trip_time,
            it.finished,
            it.unfinished,
            it.switch_share,
            it.found_share,
            it.gap,
            it.stage_seconds.iter().sum::<f64>()
        );
    }
    println!("stopped by the rule: {}", run.stopped_by_rule);
}
