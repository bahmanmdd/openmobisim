//! The trips between two places and the transit they ride. A place is a door position (exactly
//! equal coordinates); a pair is the trips with the same origin and destination doors, numbered
//! busiest first (most travellers; at equal travellers, the pair of the lowest trip id first); a
//! journey within a pair is a mode with its exact links, or the rides of a journey on the
//! timetable, numbered the same way. The transit layer counts every journey taken, the pairs the
//! page holds or not: per hop between two consecutive stops of a pattern, the travellers who rode
//! it; per stop, the travellers who boarded there.

use std::collections::{BTreeMap, BTreeSet};

use openmobisim_graph::geometry::Xy;
use openmobisim_io_gtfs::{Kind, Lines};
use openmobisim_transit::timetable::Timetable;
use openmobisim_types::ids::{EntityId, StopId, TransitRunId};
use openmobisim_types::mode::Mode;

use crate::MapError;
use crate::blob::Blob;
use crate::kept::Kept;
use crate::network::{decimetres, f32_of};
use crate::rules::MAX_PAIRS;
use crate::way::{self, Geometry, Way};

/// No index: a leg not drawn.
const NONE: u32 = u32::MAX;

/// The stops the page holds (those of a hop ridden, by stop id) and each one's place among them.
pub(crate) struct Stops {
    pub kept: Vec<StopId>,
    place: BTreeMap<StopId, u32>,
}

impl Stops {
    fn of(&self, stop: StopId) -> u32 {
        self.place[&stop]
    }
}

/// A count as the page's 32 bits.
fn count(n: usize) -> u32 {
    u32::try_from(n).expect("fewer than 2^32")
}

/// Add the transit layer: the stops (`tsx`, `tsy`, decimetres from `origin`), the boardings per
/// stop (`tsb`), and the hops ridden by kind (`ta`, `tb`: their stops; `tk`: the kind; `tl`: the
/// travellers on board), the least busy first so the busiest is drawn last.
pub(crate) fn add_transit(
    blob: &mut Blob,
    ways: &[Option<Way<'_>>],
    timetable: Option<&Timetable>,
    lining: &Lining<'_>,
    origin: (f64, f64),
) -> Result<Stops, MapError> {
    let (hops, boarded) = ridden(ways, lining);
    let set: BTreeSet<StopId> = hops.keys().flat_map(|&(_, a, b)| [a, b]).collect();
    let kept: Vec<StopId> = set.into_iter().collect();
    let place = kept.iter().enumerate().map(|(i, &s)| (s, count(i))).collect();
    let stops = Stops { kept, place };
    let (mut sx, mut sy) = (Vec::new(), Vec::new());
    if let Some(tt) = timetable {
        for &s in &stops.kept {
            let (x, y) = tt.stop_position(s);
            sx.push(decimetres(x.get(), origin.0)?);
            sy.push(decimetres(y.get(), origin.1)?);
        }
    }
    let mut order: Vec<((u8, StopId, StopId), u32)> = hops.into_iter().collect();
    order.sort_by_key(|&((k, a, b), load)| (load, a, b, k));
    blob.i32("tsx", &sx);
    blob.i32("tsy", &sy);
    let on = |s: &StopId| boarded.get(s).copied().unwrap_or(0);
    blob.u32("tsb", &stops.kept.iter().map(on).collect::<Vec<_>>());
    blob.u32("ta", &order.iter().map(|((_, a, _), _)| stops.of(*a)).collect::<Vec<_>>());
    blob.u32("tb", &order.iter().map(|((_, _, b), _)| stops.of(*b)).collect::<Vec<_>>());
    blob.u8("tk", &order.iter().map(|((k, _, _), _)| *k).collect::<Vec<_>>());
    blob.u32("tl", &order.iter().map(|(_, load)| *load).collect::<Vec<_>>());
    Ok(stops)
}

/// Every journey taken, counted on the timetable: per hop (two consecutive stops of a pattern,
/// merged across patterns of the same kind of service: rail, metro, tram, bus, ferry, other) the
/// travellers who rode it, and per stop those who boarded there.
pub(crate) type Ridden = (BTreeMap<(u8, StopId, StopId), u32>, BTreeMap<StopId, u32>);

/// The hops ridden and the boardings of `ways` (see [`Ridden`]), each ride's kind its line's.
pub(crate) fn ridden(ways: &[Option<Way<'_>>], lining: &Lining<'_>) -> Ridden {
    let mut hops: BTreeMap<(u8, StopId, StopId), u32> = BTreeMap::new();
    let mut boarded: BTreeMap<StopId, u32> = BTreeMap::new();
    for leg in ways.iter().flatten().flat_map(|w| &w.legs) {
        if let (Geometry::Stops(stops), Some(run)) = (leg.geometry, leg.run) {
            let kind = lining.of(run).1;
            *boarded.entry(stops[0]).or_default() += 1;
            for hop in stops.windows(2) {
                *hops.entry((kind, hop[0], hop[1])).or_default() += 1;
            }
        }
    }
    (hops, boarded)
}

/// The runs' lines and kinds (from the GTFS side table, when there is one) and the lines the
/// page holds: a ride's line is its run's line, by name and kind, each line once.
pub(crate) struct Lining<'a> {
    timetable: Option<&'a Timetable>,
    lines: Option<&'a Lines>,
    held: BTreeMap<(String, u8), u32>,
}

impl<'a> Lining<'a> {
    pub fn new(timetable: Option<&'a Timetable>, lines: Option<&'a Lines>) -> Self {
        Self { timetable, lines, held: BTreeMap::new() }
    }

    /// A run's line name (empty when unknown) and kind (5, other, when unknown).
    pub fn of(&self, run: TransitRunId) -> (String, u8) {
        let line = self
            .timetable
            .zip(self.lines)
            .and_then(|(tt, l)| l.line_of_run(tt.run_ids().external(run.raw())));
        line.map_or((String::new(), Kind::Other as u8), |l| (l.name.clone(), l.kind as u8))
    }

    /// The page's index of a run's line, the line held if new.
    fn index(&mut self, run: TransitRunId) -> u32 {
        let key = self.of(run);
        let next = count(self.held.len());
        *self.held.entry(key).or_insert(next)
    }

    /// The lines held, by index: names and kinds.
    pub fn held(&self) -> (Vec<String>, Vec<u8>) {
        let mut by: Vec<(&(String, u8), &u32)> = self.held.iter().collect();
        by.sort_by_key(|e| *e.1);
        (by.iter().map(|e| e.0.0.clone()).collect(), by.iter().map(|e| e.0.1).collect())
    }
}

/// A journey of a pair, summed over its travellers.
struct Journey<'a> {
    /// Its first traveller's way: the legs drawn.
    way: &'a Way<'a>,
    first: usize,
    travellers: u32,
    finished: u32,
    /// Over the finished travellers: door-to-door seconds, and each leg's seconds.
    time: f64,
    legs: Vec<f64>,
}

/// A pair: its trips, in trip id order.
struct Pair {
    trips: Vec<usize>,
}

/// The pairs, busiest first: the trips grouped by their exact doors, those left out aside.
fn pairs(kept: &Kept) -> Vec<Pair> {
    let key = |t: usize| {
        let (o, d) = (kept.trips[t].origin, kept.trips[t].destination);
        // + 0.0 makes −0.0 into 0.0: the same place.
        [o.x, o.y, d.x, d.y].map(|v| (v + 0.0).to_bits())
    };
    let mut by_doors: BTreeMap<[u64; 4], Vec<usize>> = BTreeMap::new();
    // A trip left out of the journeys (its routes do not agree with the trips table) is in no
    // pair: the footer counts it.
    for t in (0..kept.trips.len()).filter(|&t| kept.trips[t].legs.is_some()) {
        by_doors.entry(key(t)).or_default().push(t);
    }
    let mut out: Vec<Pair> = by_doors.into_values().map(|trips| Pair { trips }).collect();
    out.sort_by_key(|p| (std::cmp::Reverse(p.trips.len()), p.trips[0]));
    out
}

/// A pair's journeys, busiest first, and its travellers without an alternative.
fn journeys<'a>(kept: &Kept, ways: &'a [Option<Way<'a>>], pair: &Pair) -> (Vec<Journey<'a>>, u32) {
    let mut by_key: BTreeMap<(Mode, &[u32]), Journey<'a>> = BTreeMap::new();
    let mut without = 0;
    for &t in &pair.trips {
        let Some(way) = &ways[t] else {
            without += 1;
            continue;
        };
        let j = by_key.entry((way.mode, way.key.as_slice())).or_insert_with(|| Journey {
            way,
            first: t,
            travellers: 0,
            finished: 0,
            time: 0.0,
            legs: vec![0.0; way.legs.len()],
        });
        j.travellers += 1;
        let trip = &kept.trips[t];
        if trip.finished {
            j.finished += 1;
            j.time += trip.travel_time;
            for (sum, leg) in j.legs.iter_mut().zip(&way.legs) {
                *sum += leg.seconds;
            }
        }
    }
    let mut out: Vec<Journey<'a>> = by_key.into_values().collect();
    out.sort_by_key(|j| (std::cmp::Reverse(j.travellers), j.first));
    (out, without)
}

/// The arrays of the pairs, their journeys and their legs, filled in order.
#[derive(Default)]
struct Columns {
    door: [Vec<i32>; 4],
    travellers: Vec<u32>,
    without: Vec<u32>,
    journeys_start: Vec<u32>,
    mode: Vec<u8>,
    on: Vec<u32>,
    finished: Vec<u32>,
    time: Vec<f32>,
    legs_start: Vec<u32>,
    kind: Vec<u8>,
    seconds: Vec<f32>,
    line: Vec<u32>,
    from: Vec<u32>,
    to: Vec<u32>,
    links: Vec<u32>,
    stops: Vec<u32>,
}

impl Columns {
    /// Add a journey and its legs: each leg's seconds the mean over the finished travellers (0
    /// when none finished), and where it is drawn: a range of `links` or of `stops`, two stops,
    /// or none.
    fn journey(&mut self, j: &Journey<'_>, stops: &Stops, lining: &mut Lining<'_>) {
        let done = f64::from(j.finished);
        self.mode.push(j.way.mode.as_u8());
        self.on.push(j.travellers);
        self.finished.push(j.finished);
        self.time.push(if j.finished > 0 { f32_of(j.time / done) } else { f32::NAN });
        for (leg, &sum) in j.way.legs.iter().zip(&j.legs) {
            self.kind.push(leg.kind);
            self.line.push(leg.run.map_or(NONE, |r| lining.index(r)));
            self.seconds.push(if j.finished > 0 { f32_of(sum / done) } else { 0.0 });
            let (from, to) = match leg.geometry {
                Geometry::None => (NONE, NONE),
                Geometry::Links(links) => {
                    let from = count(self.links.len());
                    self.links.extend(links.iter().map(|l| l.raw()));
                    (from, count(self.links.len()))
                }
                Geometry::Stops(s) => {
                    let from = count(self.stops.len());
                    self.stops.extend(s.iter().map(|&s| stops.of(s)));
                    (from, count(self.stops.len()))
                }
                Geometry::Between(a, b) => (stops.of(a), stops.of(b)),
            };
            self.from.push(from);
            self.to.push(to);
        }
        self.legs_start.push(count(self.kind.len()));
    }
}

/// Add the busiest [`MAX_PAIRS`] pairs: their doors (`qox`, `qoy`, `qdx`, `qdy`, decimetres from
/// `origin`), travellers (`qn`), travellers without an alternative (`qr`) and journeys (`qs`:
/// the first of each pair's and one past the last); per journey its mode (`om`), travellers
/// (`on`), finished travellers (`of`), mean door-to-door seconds of the finished (`ot`; NaN when
/// none finished) and legs (`ol`); per leg its kind (`lk`), mean seconds (`lsec`) and where it is
/// drawn (`la`, `lb`: a range of `pl`, the links, or of `pst`, the stops; or two stops; or none,
/// `u32::MAX`), and each ride's line (`lline`: its place in `lining`'s lines, else `u32::MAX`).
/// Returns the held pairs' doors and how many pairs there are.
pub(crate) fn add_pairs(
    blob: &mut Blob,
    kept: &Kept,
    ways: &[Option<Way<'_>>],
    stops: &Stops,
    lining: &mut Lining<'_>,
    origin: (f64, f64),
) -> Result<(Vec<(Xy, Xy)>, usize), MapError> {
    let all = pairs(kept);
    let mut c = Columns { journeys_start: vec![0], legs_start: vec![0], ..Columns::default() };
    let mut doors = Vec::new();
    for pair in all.iter().take(MAX_PAIRS) {
        let t = &kept.trips[pair.trips[0]];
        doors.push((t.origin, t.destination));
        for (column, (v, from)) in c.door.iter_mut().zip([
            (t.origin.x, origin.0),
            (t.origin.y, origin.1),
            (t.destination.x, origin.0),
            (t.destination.y, origin.1),
        ]) {
            column.push(decimetres(v, from)?);
        }
        let (journeys, without) = journeys(kept, ways, pair);
        c.travellers.push(count(pair.trips.len()));
        c.without.push(without);
        for j in &journeys {
            c.journey(j, stops, lining);
        }
        c.journeys_start.push(count(c.mode.len()));
    }
    for (name, column) in ["qox", "qoy", "qdx", "qdy"].into_iter().zip(&c.door) {
        blob.i32(name, column);
    }
    blob.u32("qn", &c.travellers);
    blob.u32("qr", &c.without);
    blob.u32("qs", &c.journeys_start);
    blob.u8("om", &c.mode);
    blob.u32("on", &c.on);
    blob.u32("of", &c.finished);
    blob.f32("ot", &c.time);
    blob.u32("ol", &c.legs_start);
    blob.u8("lk", &c.kind);
    blob.f32("lsec", &c.seconds);
    blob.u32("lline", &c.line);
    blob.u32("la", &c.from);
    blob.u32("lb", &c.to);
    blob.u32("pl", &c.links);
    blob.u32("pst", &c.stops);
    Ok((doors, all.len()))
}

/// Every trip's way, in trip order.
///
/// # Errors
///
/// As [`way::of`].
pub(crate) fn ways<'a>(
    kept: &'a Kept,
    timetable: Option<&'a Timetable>,
) -> Result<Vec<Option<Way<'a>>>, MapError> {
    kept.trips.iter().map(|t| way::of(t, timetable)).collect()
}
