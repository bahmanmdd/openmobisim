//! The timetable of one service day: stops, runs and the footpaths between stops.
//!
//! # What it holds
//!
//! A **stop** is where runs call, known by its external id and its position in metres; it is
//! not a node of the network (linking it to the walk network is the caller's). A **run** is
//! one vehicle's trip through the day: its **calls** in order, each a stop, an arrival and a
//! departure second, arrival at or before departure and the times never falling along the run.
//! A **footpath** is a walk from one stop to another in seconds, given, not derived: this crate
//! never walks on a network.
//!
//! # Patterns
//!
//! A **pattern** is a set of runs that call at the same stops in the same order and never
//! overtake one another: at every call a later run arrives and departs no earlier than the run
//! before it. That is what lets the search find the earliest run a traveller can catch at a
//! stop by a binary search over one column of departures. The runs of one stop sequence are
//! taken in order of departure from its first stop (then by every later time, then by run id),
//! and each joins the first pattern of that sequence whose last run it does not overtake, or
//! else opens a new one: an express and a local on the same stops become two patterns.
//!
//! # Identity
//!
//! Stops and runs get dense ids by sorted external id ([`ExternalIdTableBuilder`]), so the same
//! input gives the same ids in any order. Patterns are numbered by their lowest run id.
//!
//! # Units
//!
//! Times are whole seconds of the clock ([`Second`]); footpaths are [`Duration`]s, put on the
//! clock rounded up to the whole second (a traveller who arrives at 100.3 s catches a run
//! leaving at 101 s and not one leaving at 100 s, so the rounding decides nothing a fraction
//! would not). Positions are metres.

use core::fmt;
use std::collections::HashMap;

use openmobisim_types::fingerprint::Fingerprint;
use openmobisim_types::ids::{
    EntityId, ExternalIdTable, ExternalIdTableBuilder, StopId, TransitRunId,
};
use openmobisim_types::time::Second;
use openmobisim_types::units::{Duration, Metres};

/// One call of a run, as given to the builder: the stop by its external id, the arrival and the
/// departure there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Call {
    /// The external id of the stop called at.
    pub stop: String,
    /// When the run arrives there.
    pub arrival: Second,
    /// When it leaves; no earlier than the arrival.
    pub departure: Second,
}

/// Why a timetable could not be built. Each names the run or the stop at fault by its external
/// id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TimetableError {
    /// Two stops share an external id.
    DuplicateStop {
        /// The external id.
        stop: String,
    },
    /// Two runs share an external id.
    DuplicateRun {
        /// The external id.
        run: String,
    },
    /// A run of fewer than two calls: nothing can be ridden on it.
    TooFewCalls {
        /// The run.
        run: String,
    },
    /// A call names a stop the timetable does not have.
    UnknownStop {
        /// The run.
        run: String,
        /// The stop named.
        stop: String,
    },
    /// A call departs before it arrives, or arrives before the call before it departs.
    CallOutOfOrder {
        /// The run.
        run: String,
        /// The call's place in the run, from 0.
        call: usize,
    },
    /// A footpath names a stop the timetable does not have.
    FootpathUnknownStop {
        /// The stop named.
        stop: String,
    },
    /// A footpath's seconds are negative or not finite.
    FootpathSeconds {
        /// From this stop.
        from: String,
        /// To this one.
        to: String,
    },
}

impl fmt::Display for TimetableError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateStop { stop } => write!(f, "stop {stop} is given twice"),
            Self::DuplicateRun { run } => write!(f, "run {run} is given twice"),
            Self::TooFewCalls { run } => write!(f, "run {run} has fewer than two calls"),
            Self::UnknownStop { run, stop } => write!(f, "run {run} calls at unknown stop {stop}"),
            Self::CallOutOfOrder { run, call } => {
                write!(
                    f,
                    "run {run}: call {call} departs before it arrives or before the call before it"
                )
            }
            Self::FootpathUnknownStop { stop } => write!(f, "a footpath names unknown stop {stop}"),
            Self::FootpathSeconds { from, to } => {
                write!(f, "the footpath from {from} to {to} has negative or infinite seconds")
            }
        }
    }
}

impl core::error::Error for TimetableError {}

/// Collects stops, runs and footpaths; [`Self::build`] checks them and assigns ids.
#[derive(Clone, Debug, Default)]
pub struct TimetableBuilder {
    stops: Vec<(String, (Metres, Metres))>,
    runs: Vec<(String, Vec<Call>)>,
    footpaths: Vec<(String, String, Duration)>,
}

impl TimetableBuilder {
    /// An empty builder.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a stop: its external id and its position, x and y in metres.
    pub fn add_stop(&mut self, external: impl Into<String>, position: (Metres, Metres)) {
        self.stops.push((external.into(), position));
    }

    /// Add a run: its external id and its calls in order.
    pub fn add_run(&mut self, external: impl Into<String>, calls: Vec<Call>) {
        self.runs.push((external.into(), calls));
    }

    /// Add a footpath from stop `from` to stop `to` (external ids), walked in `seconds`.
    pub fn add_footpath(
        &mut self,
        from: impl Into<String>,
        to: impl Into<String>,
        seconds: Duration,
    ) {
        self.footpaths.push((from.into(), to.into(), seconds));
    }

    /// Check everything and freeze it into a [`Timetable`].
    ///
    /// # Errors
    ///
    /// [`TimetableError`], naming the run or the stop: a repeated external id, a run of fewer
    /// than two calls, a call at an unknown stop, a call out of order, a footpath to or from an
    /// unknown stop or with negative or infinite seconds.
    /// Runs are checked in the order of their external ids, so the error is the same whatever
    /// order they were added in.
    pub fn build(self) -> Result<Timetable, TimetableError> {
        let stop_ids = table(self.stops.iter().map(|s| s.0.as_str()))
            .map_err(|stop| TimetableError::DuplicateStop { stop })?;
        let run_ids = table(self.runs.iter().map(|r| r.0.as_str()))
            .map_err(|run| TimetableError::DuplicateRun { run })?;
        let mut positions = vec![(Metres::ZERO, Metres::ZERO); stop_ids.len()];
        for (external, position) in &self.stops {
            positions[stop_index(&stop_ids, external)] = *position;
        }
        // The runs in id order, each as its stops and its (arrival, departure) per call.
        let mut by_id: Vec<&(String, Vec<Call>)> = self.runs.iter().collect();
        by_id.sort_by(|a, b| a.0.cmp(&b.0));
        let mut runs = Vec::with_capacity(by_id.len());
        for (external, calls) in by_id {
            runs.push(resolve(&stop_ids, external, calls)?);
        }
        let mut tt = Timetable {
            stop_ids,
            positions,
            run_ids,
            run_pattern: vec![0; runs.len()],
            patterns: Vec::new(),
            pattern_stops: Vec::new(),
            pattern_runs: Vec::new(),
            arrival: Vec::new(),
            departure: Vec::new(),
            stop_patterns_start: Vec::new(),
            stop_patterns: Vec::new(),
            footpath_start: Vec::new(),
            footpaths: Vec::new(),
        };
        tt.group(&runs);
        tt.index_stops();
        tt.add_footpaths(&self.footpaths)?;
        Ok(tt)
    }
}

/// The table of `externals`, or the first external id given twice.
fn table<'a>(externals: impl Iterator<Item = &'a str> + Clone) -> Result<ExternalIdTable, String> {
    let mut b = ExternalIdTableBuilder::new();
    b.extend(externals.clone());
    let t = b.build();
    if t.len() == externals.clone().count() {
        return Ok(t);
    }
    let mut all: Vec<&str> = externals.collect();
    all.sort_unstable();
    let twice = all.windows(2).find(|w| w[0] == w[1]).map(|w| w[0].to_owned());
    Err(twice.expect("a shorter table has a repeated id"))
}

/// The index of a stop known to be in `stops`.
fn stop_index(stops: &ExternalIdTable, external: &str) -> usize {
    stops.id_of(external).expect("the stop was offered") as usize
}

/// A run's calls checked and resolved: its stops, and its (arrival, departure) per call.
type Resolved = (Vec<StopId>, Vec<(u32, u32)>);

/// Resolve and check the calls of run `run`.
fn resolve(stops: &ExternalIdTable, run: &str, calls: &[Call]) -> Result<Resolved, TimetableError> {
    if calls.len() < 2 {
        return Err(TimetableError::TooFewCalls { run: run.to_owned() });
    }
    let mut ids = Vec::with_capacity(calls.len());
    let mut times = Vec::with_capacity(calls.len());
    let mut clock = 0;
    for (i, c) in calls.iter().enumerate() {
        let Some(stop) = stops.typed_id_of::<StopId>(&c.stop) else {
            return Err(TimetableError::UnknownStop { run: run.to_owned(), stop: c.stop.clone() });
        };
        let (a, d) = (c.arrival.get(), c.departure.get());
        if a > d || a < clock {
            return Err(TimetableError::CallOutOfOrder { run: run.to_owned(), call: i });
        }
        clock = d;
        ids.push(stop);
        times.push((a, d));
    }
    Ok((ids, times))
}

/// Where a pattern's stops, runs and times start in the timetable's arrays, and its sizes.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Pattern {
    /// Its first stop in `pattern_stops`.
    pub stops_start: u32,
    /// How many stops it calls at.
    pub len: u32,
    /// Its first run in `pattern_runs`.
    pub runs_start: u32,
    /// How many runs it has.
    pub runs: u32,
    /// Its first time in `arrival` and `departure`, which hold a pattern's times stop by stop,
    /// its runs side by side at each stop.
    pub times_start: u32,
}

/// A footpath as stored: to which stop, its seconds as given, and on the clock (rounded up).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Walk {
    /// The stop walked to.
    pub to: StopId,
    /// The seconds as given.
    pub seconds: Duration,
    /// The seconds rounded up to a whole second.
    pub clock: u32,
}

/// The timetable of one service day (see the [module documentation](self)). Read-only.
#[derive(Clone, Debug)]
pub struct Timetable {
    stop_ids: ExternalIdTable,
    positions: Vec<(Metres, Metres)>,
    run_ids: ExternalIdTable,
    /// Per run, its pattern.
    run_pattern: Vec<u32>,
    patterns: Vec<Pattern>,
    pattern_stops: Vec<StopId>,
    pattern_runs: Vec<TransitRunId>,
    arrival: Vec<u32>,
    departure: Vec<u32>,
    /// Per stop (`stops + 1` offsets), the patterns through it and the position there, sorted.
    stop_patterns_start: Vec<u32>,
    stop_patterns: Vec<(u32, u32)>,
    /// Per stop (`stops + 1` offsets), the footpaths from it, by stop walked to then seconds.
    footpath_start: Vec<u32>,
    footpaths: Vec<Walk>,
}

/// A count or an offset as `u32`.
fn to_u32(n: usize) -> u32 {
    u32::try_from(n).expect("timetable arrays fit u32")
}

impl Timetable {
    /// Group the runs (in id order, each resolved) into patterns and lay out their times: the
    /// runs of one stop sequence by their times, each joining the first pattern of that sequence
    /// whose last run it does not overtake, else opening a new one; the patterns numbered by their
    /// lowest run id.
    fn group(&mut self, runs: &[Resolved]) {
        let mut sequence_of: HashMap<&[StopId], usize> = HashMap::new();
        let mut sequences: Vec<Vec<usize>> = Vec::new();
        for (r, (stops, _)) in runs.iter().enumerate() {
            let next = sequences.len();
            let q = *sequence_of.entry(stops.as_slice()).or_insert(next);
            if q == next {
                sequences.push(Vec::new());
            }
            sequences[q].push(r);
        }
        let mut patterns: Vec<Vec<usize>> = Vec::new();
        for mut list in sequences {
            list.sort_by(|&a, &b| runs[a].1.cmp(&runs[b].1).then(a.cmp(&b)));
            let mut split: Vec<Vec<usize>> = Vec::new();
            for r in list {
                let behind = |last: usize| {
                    runs[last].1.iter().zip(&runs[r].1).all(|(x, y)| y.0 >= x.0 && y.1 >= x.1)
                };
                match split.iter_mut().find(|p| behind(*p.last().expect("never empty"))) {
                    Some(p) => p.push(r),
                    None => split.push(vec![r]),
                }
            }
            patterns.extend(split);
        }
        patterns.sort_by_key(|p| p.iter().min().copied());
        for (p, list) in patterns.iter().enumerate() {
            for &r in list {
                self.run_pattern[r] = to_u32(p);
            }
            self.push_pattern(runs, list);
        }
    }

    /// Lay out one pattern: its stops, its runs in order, its times stop by stop.
    fn push_pattern(&mut self, runs: &[Resolved], list: &[usize]) {
        let stops = &runs[list[0]].0;
        self.patterns.push(Pattern {
            stops_start: to_u32(self.pattern_stops.len()),
            len: to_u32(stops.len()),
            runs_start: to_u32(self.pattern_runs.len()),
            runs: to_u32(list.len()),
            times_start: to_u32(self.arrival.len()),
        });
        self.pattern_stops.extend_from_slice(stops);
        self.pattern_runs.extend(list.iter().map(|&r| TransitRunId::from_index(r)));
        #[allow(
            clippy::needless_range_loop,
            reason = "the position indexes every run of the list in turn, not one slice"
        )]
        for pos in 0..stops.len() {
            for &r in list {
                let (a, d) = runs[r].1[pos];
                self.arrival.push(a);
                self.departure.push(d);
            }
        }
    }

    /// Index the patterns by the stops they call at.
    fn index_stops(&mut self) {
        let mut pairs: Vec<(StopId, u32, u32)> = Vec::new();
        for (p, pattern) in self.patterns.iter().enumerate() {
            for pos in 0..pattern.len {
                let stop = self.pattern_stops[(pattern.stops_start + pos) as usize];
                pairs.push((stop, to_u32(p), pos));
            }
        }
        pairs.sort_unstable();
        self.stop_patterns_start = offsets(self.stop_ids.len(), pairs.iter().map(|x| x.0));
        self.stop_patterns = pairs.into_iter().map(|(_, p, pos)| (p, pos)).collect();
    }

    /// Check and store the footpaths.
    fn add_footpaths(
        &mut self,
        given: &[(String, String, Duration)],
    ) -> Result<(), TimetableError> {
        let mut all = Vec::with_capacity(given.len());
        for (from, to, seconds) in given {
            let id = |s: &String| {
                self.stop_ids
                    .typed_id_of::<StopId>(s)
                    .ok_or_else(|| TimetableError::FootpathUnknownStop { stop: s.clone() })
            };
            let (f, t) = (id(from)?, id(to)?);
            if !seconds.is_finite() || seconds.get() < 0.0 {
                return Err(TimetableError::FootpathSeconds { from: from.clone(), to: to.clone() });
            }
            all.push((f, Walk { to: t, seconds: *seconds, clock: on_clock(*seconds) }));
        }
        all.sort_by(|a, b| {
            (a.0, a.1.to).cmp(&(b.0, b.1.to)).then(a.1.seconds.get().total_cmp(&b.1.seconds.get()))
        });
        self.footpath_start = offsets(self.stop_ids.len(), all.iter().map(|x| x.0));
        self.footpaths = all.into_iter().map(|x| x.1).collect();
        Ok(())
    }

    /// How many stops.
    #[must_use]
    pub fn stop_count(&self) -> u32 {
        self.stop_ids.count()
    }

    /// How many runs.
    #[must_use]
    pub fn run_count(&self) -> u32 {
        self.run_ids.count()
    }

    /// How many patterns.
    #[must_use]
    pub fn pattern_count(&self) -> u32 {
        to_u32(self.patterns.len())
    }

    /// The stops' external ids, by stop id.
    #[must_use]
    pub fn stop_ids(&self) -> &ExternalIdTable {
        &self.stop_ids
    }

    /// The runs' external ids, by run id.
    #[must_use]
    pub fn run_ids(&self) -> &ExternalIdTable {
        &self.run_ids
    }

    /// Where a stop is: x and y, metres.
    #[must_use]
    pub fn stop_position(&self, stop: StopId) -> (Metres, Metres) {
        self.positions[stop.index()]
    }

    /// The pattern a run belongs to.
    #[must_use]
    pub fn run_pattern(&self, run: TransitRunId) -> u32 {
        self.run_pattern[run.index()]
    }

    /// A pattern's stops, in the order its runs call at them.
    #[must_use]
    pub fn pattern_stops(&self, pattern: u32) -> &[StopId] {
        let p = self.patterns[pattern as usize];
        &self.pattern_stops[p.stops_start as usize..(p.stops_start + p.len) as usize]
    }

    /// A pattern's runs, in order: none overtakes the one before it.
    #[must_use]
    pub fn pattern_runs(&self, pattern: u32) -> &[TransitRunId] {
        let p = self.patterns[pattern as usize];
        &self.pattern_runs[p.runs_start as usize..(p.runs_start + p.runs) as usize]
    }

    /// A run's calls in order: stop, arrival, departure.
    ///
    /// # Panics
    ///
    /// For a run the timetable does not have.
    pub fn calls(&self, run: TransitRunId) -> impl Iterator<Item = (StopId, Second, Second)> + '_ {
        let p = self.run_pattern[run.index()];
        let rank =
            to_u32(self.pattern_runs(p).iter().position(|&r| r == run).expect("its pattern"));
        let stops = self.pattern_stops(p);
        stops.iter().enumerate().map(move |(pos, &s)| {
            let i = self.time_index(p, rank, to_u32(pos));
            (s, Second(self.arrival[i]), Second(self.departure[i]))
        })
    }

    /// The footpaths from `stop`: the stop walked to and the seconds as given, by stop then
    /// seconds.
    pub fn footpaths(&self, stop: StopId) -> impl Iterator<Item = (StopId, Duration)> + '_ {
        self.walks(stop).iter().map(|w| (w.to, w.seconds))
    }

    /// The fingerprint of what the timetable was built from (`openmobisim_types::fingerprint`),
    /// fed in this order: the stop count, then per stop in id order its external id, x and y
    /// (metres); the run count, then per run in id order its external id, its call count and
    /// per call the stop id, the arrival and the departure (seconds of the clock); the footpath
    /// count, then per footpath, by stop walked from, stop walked to, then seconds, the two stop
    /// ids and the seconds as given. Two builds of the same input, in any order, have the same
    /// fingerprint. The patterns are not fed: they follow from the runs.
    #[must_use]
    pub fn fingerprint(&self) -> u64 {
        let mut h = Fingerprint::new();
        h.u32(self.stop_count());
        for (i, id) in self.stop_ids.iter().enumerate() {
            h.str(id);
            h.f64(self.positions[i].0.get());
            h.f64(self.positions[i].1.get());
        }
        h.u32(self.run_count());
        for (i, id) in self.run_ids.iter().enumerate() {
            h.str(id);
            let run = TransitRunId::from_index(i);
            h.u32(to_u32(self.pattern_stops(self.run_pattern(run)).len()));
            for (stop, arrival, departure) in self.calls(run) {
                h.u32(stop.raw());
                h.u32(arrival.get());
                h.u32(departure.get());
            }
        }
        h.u64(self.footpaths.len() as u64);
        for from in 0..self.stop_count() {
            let from = StopId::new(from);
            for (to, seconds) in self.footpaths(from) {
                h.u32(from.raw());
                h.u32(to.raw());
                h.f64(seconds.get());
            }
        }
        h.finish()
    }

    // What the search reads.

    /// Pattern `p`'s layout.
    pub(crate) fn pattern(&self, p: u32) -> Pattern {
        self.patterns[p as usize]
    }

    /// The stop at position `pos` of pattern `p`.
    pub(crate) fn stop_at(&self, p: u32, pos: u32) -> StopId {
        self.pattern_stops[(self.patterns[p as usize].stops_start + pos) as usize]
    }

    /// The run of rank `rank` in pattern `p`.
    pub(crate) fn run_at(&self, p: u32, rank: u32) -> TransitRunId {
        self.pattern_runs[(self.patterns[p as usize].runs_start + rank) as usize]
    }

    fn time_index(&self, p: u32, rank: u32, pos: u32) -> usize {
        let pattern = self.patterns[p as usize];
        (pattern.times_start + pos * pattern.runs + rank) as usize
    }

    /// The arrival of run `rank` of pattern `p` at position `pos`, seconds.
    pub(crate) fn arrival_at(&self, p: u32, rank: u32, pos: u32) -> u32 {
        self.arrival[self.time_index(p, rank, pos)]
    }

    /// The departure of run `rank` of pattern `p` at position `pos`, seconds.
    pub(crate) fn departure_at(&self, p: u32, rank: u32, pos: u32) -> u32 {
        self.departure[self.time_index(p, rank, pos)]
    }

    /// The arrivals at position `pos` of pattern `p`, run by run (never falling).
    pub(crate) fn arrivals_at(&self, p: u32, pos: u32) -> &[u32] {
        let i = self.time_index(p, 0, pos);
        &self.arrival[i..i + self.patterns[p as usize].runs as usize]
    }

    /// The departures at position `pos` of pattern `p`, run by run (never falling).
    pub(crate) fn departures_at(&self, p: u32, pos: u32) -> &[u32] {
        let i = self.time_index(p, 0, pos);
        &self.departure[i..i + self.patterns[p as usize].runs as usize]
    }

    /// The patterns through `stop`, each with the stop's position in it.
    pub(crate) fn patterns_through(&self, stop: StopId) -> &[(u32, u32)] {
        let s = stop.index();
        &self.stop_patterns
            [self.stop_patterns_start[s] as usize..self.stop_patterns_start[s + 1] as usize]
    }

    /// The footpaths from `stop`, as stored.
    pub(crate) fn walks(&self, stop: StopId) -> &[Walk] {
        let s = stop.index();
        &self.footpaths[self.footpath_start[s] as usize..self.footpath_start[s + 1] as usize]
    }
}

/// `n + 1` offsets of a list sorted by stop, from the stops of its entries.
fn offsets(n: usize, stops: impl Iterator<Item = StopId>) -> Vec<u32> {
    let mut start = vec![0_u32; n + 1];
    for s in stops {
        start[s.index() + 1] += 1;
    }
    for i in 0..n {
        start[i + 1] += start[i];
    }
    start
}

/// A finite, non-negative duration on the clock: rounded up to the whole second, at most the
/// clock's last second.
pub(crate) fn on_clock(d: Duration) -> u32 {
    debug_assert!(d.is_finite() && d.get() >= 0.0, "a duration on the clock is finite and >= 0");
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "finite, non-negative and capped at u32::MAX"
    )]
    let s = d.get().ceil().min(f64::from(u32::MAX)) as u32;
    s
}
