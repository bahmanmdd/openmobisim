//! A morning's trips drawn from the streets of a network the reader read (experimental): a
//! starting point for trying ideas, not a model of the city, and nothing in it is calibrated.
//!
//! - **Where people live and go.** The plane is cut in square cells of [`CELL_M`]. A cell's homes
//!   are in proportion to the length of its residential and living streets, its activities to
//!   the length of its other streets ([`ACTIVITY`]); where a network has none of a kind, the
//!   other kind serves for both. Streets are the walk links, each counted in the cell of the
//!   middle of its two ends. A trip starts at a point of a home street of its home cell and ends
//!   at a point of an activity street of its activity cell (a point of the straight line between
//!   the link's ends). Given the home cell, the activity cell is drawn in proportion to its
//!   activities × exp(−distance / [`DECAY_M`]), the distance between the cells' centres, at
//!   least a third of a cell.
//! - **Who travels.** The four classes of [`CLASSES`] by their shares, each with the modes open
//!   to it (transit only when the case has a timetable).
//! - **Through traffic.** [`THROUGH_SHARE`] of the trips are cars of the class [`THROUGH`]
//!   entering at one cut end of a motorway or trunk road (the ends the reader closes with a
//!   turnaround) and leaving at another at least [`GATES_APART_M`] away, each drawn in
//!   proportion to the capacity of the car links out of the one and into the other. A network
//!   without two such ends has none.
//! - **When.** Each trip sets out at a second drawn evenly from [`DEPARTURES_S`] after the start.
//!
//! Bikes: half the population by default has a bike (the classes `car_and_bike` and `bike_only`)
//! and rides every street the bike layer holds. The draws are [`Stream::Population`] at (trip,
//! 0), words 0 to 8 in the order of [`draw`]'s body: the same network, count and seed give the
//! same trips.

use std::collections::BTreeMap;

use openmobisim_demand::input::TripRow;
use openmobisim_graph::geometry::{Position, Xy};
use openmobisim_graph::network::Network;
use openmobisim_types::ids::{EntityId, LinkId, NodeId};
use openmobisim_types::mode::{Mode, ModeSet};
use openmobisim_types::rng::{DrawAddress, RngKey, Stream, StreamRng};
use openmobisim_types::time::Second;

use crate::region::TURNAROUND_FIRST_ID;

/// The side of a cell, metres.
pub const CELL_M: f64 = 500.0;
/// The distance over which the pull of an activity falls by e, metres.
pub const DECAY_M: f64 = 4000.0;
/// The share of the trips that drive through.
pub const THROUGH_SHARE: f64 = 0.05;
/// A through trip leaves at a cut end at least this far from where it entered, metres.
pub const GATES_APART_M: f64 = 1000.0;
/// Departures are spread evenly over this long after the start, seconds.
pub const DEPARTURES_S: u32 = 3600;
/// The street classes homes are counted on.
pub const HOME: [&str; 2] = ["residential", "living_street"];
/// The street classes activities are counted on.
pub const ACTIVITY: [&str; 9] = [
    "primary",
    "primary_link",
    "secondary",
    "secondary_link",
    "tertiary",
    "tertiary_link",
    "unclassified",
    "service",
    "pedestrian",
];
/// The classes: the id, the share of the travellers, the modes open to it.
pub const CLASSES: [(&str, f64, &[Mode]); 4] = [
    ("car_and_bike", 0.4, &[Mode::Car, Mode::Bike, Mode::Walk, Mode::Transit]),
    ("car_only", 0.2, &[Mode::Car, Mode::Walk, Mode::Transit]),
    ("bike_only", 0.2, &[Mode::Bike, Mode::Walk, Mode::Transit]),
    ("no_vehicle", 0.2, &[Mode::Walk, Mode::Transit]),
];
/// The class of the cars driving through.
pub const THROUGH: &str = "through";

/// The index of the first of `sums` (running totals) above `u` of the whole.
fn pick(sums: &[f64], u: f64) -> usize {
    let target = u * sums.last().copied().unwrap_or(0.0);
    sums.partition_point(|&s| s <= target).min(sums.len().saturating_sub(1))
}

/// A street: its length (in [`Cells`], the running total of the lengths of its cell) and its ends.
type Street = (f64, Xy, Xy);
/// A cut end to enter by: where, the running total of the capacity out of the entries so far,
/// and the exits far enough, each with the running total of the capacity into them.
type Gate = (Xy, f64, Vec<(Xy, f64)>);

/// Cells with their streets: the centres, the running total of the cells' street lengths, and
/// each cell's streets.
#[derive(Default)]
struct Cells {
    centre: Vec<Xy>,
    sums: Vec<f64>,
    streets: Vec<Vec<Street>>,
}

impl Cells {
    fn new(by_cell: BTreeMap<(i64, i64), Vec<Street>>) -> Self {
        let mut cells = Self::default();
        let mut total = 0.0;
        for ((i, j), mut streets) in by_cell {
            let mut length = 0.0;
            for s in &mut streets {
                length += s.0;
                s.0 = length;
            }
            total += length;
            #[allow(clippy::cast_precision_loss, reason = "a cell index is far below 2^53")]
            cells.centre.push(Xy::new((i as f64 + 0.5) * CELL_M, (j as f64 + 0.5) * CELL_M));
            cells.sums.push(total);
            cells.streets.push(streets);
        }
        cells
    }

    /// A point of a street of `cell`: the street by length (`u`), the point along it (`along`).
    fn point(&self, cell: usize, u: f64, along: f64) -> Xy {
        let streets = &self.streets[cell];
        let sums: Vec<f64> = streets.iter().map(|s| s.0).collect();
        let (_, a, b) = streets[pick(&sums, u)];
        Xy::new(a.x + along * (b.x - a.x), a.y + along * (b.y - a.y))
    }
}

/// The cut ends to drive through, each with the exits at least [`GATES_APART_M`] away.
fn gates(network: &Network) -> Vec<Gate> {
    let nodes = network.node_count() as usize;
    let (mut out, mut into) = (vec![0.0; nodes], vec![0.0; nodes]);
    let (mut entries, mut exits) = (Vec::new(), Vec::new());
    for l in LinkId::iter_space(network.link_count()) {
        if !network.modes(l).contains(Mode::Car) {
            continue;
        }
        let (up, down) = (network.upstream(l), network.downstream(l));
        out[up.index()] += network.capacity(l).0;
        into[down.index()] += network.capacity(l).0;
        // A link's id is `mode:way:piece:direction`; the turnarounds' ways count up from
        // `TURNAROUND_FIRST_ID`, far below any way of an extract.
        let way = network.link_ids().external(l.raw()).split(':').nth(1);
        if way.and_then(|w| w.parse::<i64>().ok()).is_some_and(|w| w < TURNAROUND_FIRST_ID / 2) {
            exits.push(up);
            entries.push(down);
        }
    }
    entries.sort_unstable();
    entries.dedup();
    exits.sort_unstable();
    exits.dedup();
    let at = |n: NodeId| network.node_position(n);
    let mut total = 0.0;
    let mut made = Vec::new();
    for e in entries {
        let mut reach = 0.0;
        let far: Vec<(Xy, f64)> = exits
            .iter()
            .filter(|&&x| libm::hypot(at(x).x - at(e).x, at(x).y - at(e).y) >= GATES_APART_M)
            .map(|&x| {
                reach += into[x.index()];
                (at(x), reach)
            })
            .collect();
        if !far.is_empty() {
            total += out[e.index()];
            made.push((at(e), total, far));
        }
    }
    made
}

/// The home cells and the activity cells of `network`'s walk links; `None` without a street of
/// [`HOME`] or [`ACTIVITY`] to walk on.
fn cells(network: &Network) -> Option<(Cells, Cells)> {
    let (mut homes, mut activities) = (BTreeMap::new(), BTreeMap::new());
    for l in LinkId::iter_space(network.link_count()) {
        let Some(class) = network.road_class(l).filter(|_| network.modes(l).contains(Mode::Walk))
        else {
            continue;
        };
        let by_cell = match class {
            c if HOME.contains(&c) => &mut homes,
            c if ACTIVITY.contains(&c) => &mut activities,
            _ => continue,
        };
        let a = network.node_position(network.upstream(l));
        let b = network.node_position(network.downstream(l));
        #[allow(clippy::cast_possible_truncation, reason = "a plane's metres over a cell fit i64")]
        let cell = (
            libm::floor((a.x + b.x) / 2.0 / CELL_M) as i64,
            libm::floor((a.y + b.y) / 2.0 / CELL_M) as i64,
        );
        by_cell.entry(cell).or_insert_with(Vec::new).push((network.length(l).0, a, b));
    }
    let (homes, activities) = match (homes.is_empty(), activities.is_empty()) {
        (true, true) => return None,
        (true, false) => (activities.clone(), activities),
        (false, true) => (homes.clone(), homes),
        (false, false) => (homes, activities),
    };
    Some((Cells::new(homes), Cells::new(activities)))
}

/// `trips` trips on `network`, setting out from `start`, transit among their modes only
/// `with_transit`, drawn with `seed`; none on a network without a street of [`HOME`] or
/// [`ACTIVITY`] to walk on. Ids `t0000000` upwards; positions of the network's kind.
#[must_use]
pub fn draw(
    network: &Network,
    trips: u32,
    start: Second,
    with_transit: bool,
    seed: u64,
) -> Vec<TripRow> {
    let Some((homes, activities)) = cells(network) else { return Vec::new() };
    let gates = gates(network);
    let gate_sums: Vec<f64> = gates.iter().map(|g| g.1).collect();
    // The pull of every activity cell from a home cell, as running totals, made when first asked.
    let mut pulls: Vec<Option<Vec<f64>>> = vec![None; homes.centre.len()];
    let pull = |h: usize| -> Vec<f64> {
        let (from, mut total, mut before) = (homes.centre[h], 0.0, 0.0);
        let cells = activities.centre.iter().zip(&activities.sums);
        cells
            .map(|(to, &sum)| {
                let m = libm::hypot(to.x - from.x, to.y - from.y).max(CELL_M / 3.0);
                total += (sum - before) * libm::exp(-m / DECAY_M);
                before = sum;
                total
            })
            .collect()
    };
    let place = |xy: Xy| match network.projection() {
        Some(p) => Position::LonLat(p.unproject(xy)),
        None => Position::Metres(xy),
    };
    let class_sums: Vec<f64> = CLASSES
        .iter()
        .scan(0.0, |total, c| {
            Some({
                *total += c.1;
                *total
            })
        })
        .collect();
    let rng = StreamRng::new(RngKey::new(seed), Stream::Population);
    (0..trips)
        .map(|k| {
            let b = rng.block(DrawAddress::from_pair(k, 0));
            let (origin, destination, class, modes) =
                if !gates.is_empty() && b.unit(0) < THROUGH_SHARE {
                    let (entry, _, exits) = &gates[pick(&gate_sums, b.unit(1))];
                    let sums: Vec<f64> = exits.iter().map(|x| x.1).collect();
                    (*entry, exits[pick(&sums, b.unit(2))].0, THROUGH, ModeSet::of(&[Mode::Car]))
                } else {
                    let (id, _, open) = CLASSES[pick(&class_sums, b.unit(1))];
                    let h = pick(&homes.sums, b.unit(2));
                    let a = pick(pulls[h].get_or_insert_with(|| pull(h)), b.unit(5));
                    let modes = open
                        .iter()
                        .filter(|&&m| with_transit || m != Mode::Transit)
                        .fold(ModeSet::EMPTY, |set, &m| set.with(m));
                    (
                        homes.point(h, b.unit(3), b.unit(4)),
                        activities.point(a, b.unit(6), b.unit(7)),
                        id,
                        modes,
                    )
                };
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "a unit draw times an hour"
            )]
            let after = (b.unit(8) * f64::from(DEPARTURES_S)) as u32;
            TripRow {
                id: format!("t{k:07}"),
                origin: place(origin),
                destination: place(destination),
                departure: Second(start.get() + after),
                class: class.to_owned(),
                modes,
            }
        })
        .collect()
}
