//! Shared by tests 5 and 6: the capacity-constrained equilibrium fixture written by
//! `equilibrium_lp.py`, the demand streams of an OD table, and the readings of a run's steady
//! hour.

#![allow(dead_code, reason = "each test binary uses a different part of this module")]
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "counts of trips and seconds of the window, far below 2^53 and above zero"
)]

use std::collections::HashMap;
use std::path::Path;

use openmobisim_demand::demand::Demand;
use openmobisim_demand::input::TripRow;
use openmobisim_graph::network::Network;
use openmobisim_loop::run::Run;
use openmobisim_types::ids::{EntityId, LinkId, TripId};
use openmobisim_types::mode::Mode;
use openmobisim_types::time::Second;

/// The folder of the fixtures.
#[must_use]
pub fn fixtures(case: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(case)
}

/// The linear program's answer, as the script wrote it.
pub struct Equilibrium {
    /// The demand and capacity scales the fixture was solved at.
    pub scale: (f64, f64),
    /// The mean trip, seconds: the range over the dual's optimal face.
    pub mean: (f64, f64),
    /// `Σ τ x`, vehicle-seconds per hour, and the demand, vehicles per hour.
    pub tau_x: (f64, f64),
    /// Per link (external id): flow in veh/h, the least and the greatest queue in seconds over
    /// the dual's optimal face, capacity in veh/h.
    pub links: HashMap<String, (f64, f64, f64, f64)>,
    /// Per OD pair: the rate in veh/h, the least and the greatest OD time in seconds over the
    /// dual's optimal face.
    pub od: Vec<((String, String), f64, f64, f64)>,
}

/// Read the fixture `file` of `case`.
///
/// # Panics
///
/// If the file is missing or malformed.
#[must_use]
pub fn equilibrium(case: &str, file: &str) -> Equilibrium {
    let text = std::fs::read_to_string(fixtures(case).join(file)).expect("the fixture");
    let mut e = Equilibrium {
        scale: (0.0, 0.0),
        mean: (0.0, 0.0),
        tau_x: (0.0, 0.0),
        links: HashMap::new(),
        od: Vec::new(),
    };
    let num = |s: &str| s.parse::<f64>().expect("a number");
    for line in text.lines() {
        let f: Vec<&str> = line.split(',').collect();
        match f[0] {
            "scale" => e.scale = (num(f[1]), num(f[2])),
            "mean" => e.mean = (num(f[1]), num(f[2])),
            "taux" => e.tau_x = (num(f[1]), num(f[2])),
            "link" => {
                e.links.insert(f[1].to_string(), (num(f[2]), num(f[3]), num(f[4]), num(f[5])));
            }
            "od" => {
                e.od.push(((f[1].to_string(), f[2].to_string()), num(f[3]), num(f[4]), num(f[5])));
            }
            other => panic!("an unknown row {other}"),
        }
    }
    e
}

/// Car trips for every OD pair at its rate for `hours` from second 0, spaced regularly, the
/// ends at the zones' nodes.
#[must_use]
pub fn streams(network: &Network, e: &Equilibrium, hours: f64) -> Vec<TripRow> {
    let mut rows = Vec::new();
    for ((o, d), rate, _, _) in &e.od {
        rows.extend(crate::common::stream(
            network,
            &format!("{o}-{d}-"),
            o,
            d,
            *rate,
            hours,
            Second(0),
        ));
    }
    rows
}

/// What a run says about the steady hour: the trips departing in `[from, to)` seconds.
pub struct Steady {
    /// Their mean door-to-door time, seconds.
    pub mean: f64,
    /// Their count.
    pub count: usize,
    /// The sum over them of their route's free-flow time, vehicle-seconds.
    pub tau_x: f64,
    /// Per OD pair (origin, destination external ids), their mean time, seconds.
    pub od_mean: HashMap<(String, String), f64>,
}

/// Read the trips departing in `[from, to)`; every one must have finished. A trip's OD pair
/// is read from its external id, `o-d-n`, through the demand's id table (trip ids follow the
/// sorted external id, not the order of the rows).
///
/// # Panics
///
/// If a trip of the hour is unfinished.
#[must_use]
pub fn steady(network: &Network, demand: &Demand, run: &Run, from: u32, to: u32) -> Steady {
    let mut sums: HashMap<(String, String), (f64, usize)> = HashMap::new();
    let (mut total, mut count, mut tau_x) = (0.0, 0, 0.0);
    for i in 0..run.trips.departure.len() {
        let id = demand.trip_ids().external(TripId::from_index(i).raw());
        let d = run.trips.departure[i].get();
        if d < from || d >= to {
            continue;
        }
        assert!(run.trips.finished[i], "trip {i} did not finish");
        let route = run.taken(TripId::from_index(i)).and_then(|a| a.route()).expect("a route");
        tau_x += route.iter().map(|&l| network.free_flow_time(l, Mode::Car).get()).sum::<f64>();
        total += run.trips.travel_time[i];
        count += 1;
        let mut parts = id.split('-');
        let key = (parts.next().expect("o").to_string(), parts.next().expect("d").to_string());
        let s = sums.entry(key).or_insert((0.0, 0));
        s.0 += run.trips.travel_time[i];
        s.1 += 1;
    }
    let od_mean = sums.into_iter().map(|(k, (t, n))| (k, t / n as f64)).collect();
    Steady { mean: total / count as f64, count, tau_x, od_mean }
}

/// What a link did over the bins starting in `[from, to)`.
#[derive(Clone, Copy, Debug)]
pub struct LinkHour {
    /// The cars that entered, in veh/h.
    pub entries: f64,
    /// The discharge: the cars that entered less one, over the span from the first of them to
    /// leave to the last, in veh/h; `NaN` below two cars.
    pub discharge: f64,
    /// The mean delay over free flow of the cars that entered, seconds.
    pub delay: f64,
    /// Whether a queue stands in every bin: every bin's mean crossing time more than 60 s over
    /// free flow.
    pub standing: bool,
}

/// Per link, what it did over the bins starting in `[from, to)`.
#[must_use]
pub fn link_hours(network: &Network, run: &Run, from: u32, to: u32) -> HashMap<LinkId, LinkHour> {
    let w = run.links.window();
    let hours = f64::from(to - from) / 3_600.0;
    let bins_expected = (to - from) / w.step_seconds();
    let mut out: HashMap<LinkId, (u32, u32, f64, f64, f64)> = HashMap::new();
    for k in 0..run.links.len() {
        let start = w.step_start(run.links.bins()[k]).get();
        if start < from || start >= to {
            continue;
        }
        let link = run.links.links()[k];
        let cars = run.links.cars()[k];
        let mean = run.links.car_time()[k].get() / f64::from(cars);
        let standing = cars > 0 && mean > network.free_flow_time(link, Mode::Car).get() + 60.0;
        let e = out.entry(link).or_insert((0, 0, f64::NAN, f64::NAN, 0.0));
        e.0 += cars;
        e.1 += u32::from(standing);
        if cars > 0 {
            let free = network.free_flow_time(link, Mode::Car).get();
            e.4 += run.links.car_time()[k].get() - f64::from(cars) * free;
        }
        if cars > 0 {
            if e.2.is_nan() {
                e.2 = run.links.first_exit()[k].get();
            }
            e.3 = run.links.last_exit()[k].get();
        }
    }
    out.into_iter()
        .map(|(l, (cars, standing, first, last, delay))| {
            let discharge =
                if cars >= 2 { f64::from(cars - 1) / (last - first) * 3_600.0 } else { f64::NAN };
            let hour = LinkHour {
                entries: f64::from(cars) / hours,
                discharge,
                delay: delay / f64::from(cars),
                standing: standing == bins_expected,
            };
            (l, hour)
        })
        .collect()
}
