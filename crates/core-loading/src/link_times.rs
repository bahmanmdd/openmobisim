//! What a link costs at a given moment, from the last loading.
//!
//! After a loading, the per-link, per-time-bin table ([`LinkBins`]) says how
//! long traffic took to cross each link in each bin. [`LinkTimes`] reads it back
//! as a **time-dependent cost**: `link_seconds(link, t)` is the mean traversal
//! time in the bin containing `t`; where nobody entered the link in that bin it is
//! the last recorded bin's time **carried forward**, less the time elapsed since
//! that bin, down to the link's free-flow time (and free flow where no earlier
//! entrant can still be on the link). A route's expected time for a traveller
//! leaving at `t` is found by walking it link by link, arriving at each link at
//! the time the last one ended (time-dependent cost is fine under FIFO).
//!
//! **Why the carry (X-63, S286).** The table records crossings, not states: a link
//! whose queue stood still through a whole bin, with nothing able to enter, has no
//! row for that bin, and reading it as free flow would price a standing jam as an
//! empty link. Under first-in-first-out a vehicle entering now cannot leave before
//! those that entered a bin earlier and took `m`, so `m − elapsed` is a lower bound
//! on its time: that is what a bin nobody entered reads.
//!
//! **The table is by entry time.** A traveller about to enter a link needs the
//! time taken by those who *entered* when they would, not by those who happened to
//! *leave* then: in a growing queue the two differ by the queue's length, and
//! reading the exit-time table would show the queue late (found on a
//! hand-made bottleneck, where it hid a third of the disequilibrium). The loading
//! records both ([`crate::LinkBinRecorder::with_entry_bins`]);
//! only vehicles that finished the link inside the window count, so in a jam that
//! never clears the last stretch is optimistic.
//!
//! **Two readings** ([`LinkTimesReading`]). The **bins** reading (the default) is the step
//! function above: `τ(t)`, the time of the bin containing `t`. Within a bin the exit second
//! `t + τ(t)` rises with slope 1; at a boundary between bins `k` and `k + 1` it jumps by
//! `τ_{k+1} − τ_k`, so where the time falls (a queue clearing) a vehicle entering in the
//! last `τ_k − τ_{k+1}` seconds of bin `k` leaves later than one entering at the boundary:
//! the reading is not first-in-first-out (FIFO), and a label-setting time-dependent search,
//! exact only under FIFO, can miss a faster route.
//!
//! The **FIFO** reading ([`LinkTimesReading::Fifo`]) reads the **exit second** instead, from
//! the first and the last vehicle to enter each link in each bin, their entry and exit seconds
//! being recorded by the loading ([`crate::LinkBins::entrants`]). Each exit is raised to the
//! latest exit before it on its link (a no-op on a FIFO loading such as the point queue), so the
//! exits never fall, and the exit second `e(t)` of an entry at `t` is read:
//!
//! - between a bin's first and last entrant, on the straight line through them (a bin with one
//!   entrant has one point);
//! - between the last entrant of bin `k` and the first of bin `k + 1`, across the boundary, on
//!   the straight line through them;
//! - after a bin's last entrant when the next bin had none (an empty bin, or past the last bin
//!   that had any): `max(x_last, t + τ_free)`, behind the last exit at free flow at least, the
//!   carry of the bins reading made exact;
//! - before a bin's first entrant when the bin before had none: `max(x_before, t + τ_first)`,
//!   behind the last exit before (if any) at the first entrant's time;
//! - before the link's first recorded bin, and on a link nobody entered: free flow.
//!
//! The time is `τ(t) = max(e(t) − t, τ_free)`. Every piece is nondecreasing in `t` and they
//! meet without a fall, so the reading is FIFO by construction, and at a recorded entrant it is
//! that entrant's own time. [`LinkTimes::link_seconds_held`] holds the last entrant's time after
//! it. A read is one lookup in the link's bins and one interpolation. A table recorded without
//! its entrants (any but the one-loop engine's under this reading) is read FIFO by a **floor**
//! instead: `τ_FIFO(t) = max(τ(t), E(t) − t)`, `E(t)` the latest exit second, by the bins
//! reading, of anyone entering before `t`; within bin `k`, starting at `s_k`,
//! `E(t) = max(X_k, t + τ_k)`, `X_{k+1} = max(X_k, s_{k+1} + τ_k)` carried once at construction
//! (`X` starting at `s + τ_free` at the first recorded bin `s`; past the last bin the free-flow
//! time reads `max(τ_free, X − t)`). The floor overstates a falling bin's last exit (a bin whose
//! queue clears midway has a high mean though its late entrants cross fast), which the exit
//! seconds do not. A closure the travellers know of keeps FIFO either way: an entry while closed
//! is moved to the reopening, which never moves a later entry before an earlier one. The origin
//! waits are read by their bins under every reading.
//!
//! **Cost:** 12 bytes per recorded (link, bin) row plus 12 bytes per link; a lookup
//! is one binary search over that link's few bins, about 30 ns; a route of 150
//! links costs about 5 µs. The FIFO reading adds 40 bytes per recorded (link, bin) row and 4 per
//! cell of the times laid out (the floor: 8 per cell and 8 per link), built in one pass.

use crate::{Entrants, EntryTables, LinkBins};
use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_types::ids::{EntityId, LinkId};

/// How [`LinkTimes`] reads a link's time at an entry second (see the module docs).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LinkTimesReading {
    /// The time of the bin of entry: a step function of the entry second, not
    /// first-in-first-out where the time falls from one bin to the next.
    #[default]
    Bins,
    /// First-in-first-out: the exit second read on straight lines through the recorded first
    /// and last entrants of each bin ([`crate::LinkBins::entrants`]), FIFO because the loading
    /// is; on a table recorded without them, the least FIFO time at or above the bins (the
    /// floor). See the module docs.
    Fifo,
}

impl LinkTimesReading {
    /// Every reading, the default first.
    pub const ALL: [Self; 2] = [Self::Bins, Self::Fifo];

    /// The reading's name: `"bins"` or `"fifo"`.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Bins => "bins",
            Self::Fifo => "fifo",
        }
    }

    /// The reading named `name` ([`Self::name`]), if there is one.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|r| r.name() == name)
    }
}

/// Link travel times by time of entry, from a loading's entry-time table.
#[derive(Clone, Debug)]
pub struct LinkTimes {
    bin_seconds: f64,
    free_flow: Vec<f64>,
    /// The wait to get onto each link from an origin, by the bin of departure, laid
    /// out like the times.
    wait_start: Vec<u32>,
    wait_bin: Vec<u32>,
    wait_mean: Vec<f64>,
    /// Per link, its first recorded bin; its times from that bin to the last bin a recorded
    /// time reaches are `span[span_start[l]..span_start[l + 1]]`, a bin with no row holding
    /// the last recorded time carried forward (see the module docs): a lookup is one read
    /// instead of a binary search.
    span_first: Vec<u32>,
    span_start: Vec<u32>,
    span: Vec<f64>,
    /// Per link, where its last recorded bin is in its span, counted from its first: what
    /// [`LinkTimes::link_seconds_held`] holds after it.
    span_last: Vec<u32>,
    /// Closures the travellers know of ([`Self::with_closures`]), `(link, from, to)`, and per link
    /// whether it has one; both empty without.
    closures: Vec<(u32, f64, f64)>,
    has_closure: Vec<bool>,
    /// The FIFO reading's floors ([`Self::with_reading`]): per cell of the span, `X_k`, the
    /// latest exit second of the entries before the cell's bin, and per link the same after its
    /// span; both empty under the bins reading.
    exit_floor: Vec<f64>,
    exit_floor_after: Vec<f64>,
    /// Per link, its recorded bins' first and last entrants,
    /// `knots[knot_start[l]..knot_start[l + 1]]`, their bins in `knot_bin`, each exit raised to the latest exit before it (so the
    /// exits never fall); per cell of the span, its latest recorded bin's place in `knots`. All
    /// empty unless the table was recorded with its entrants.
    knot_start: Vec<u32>,
    knot_bin: Vec<u32>,
    knots: Vec<Entrants>,
    knot_of_cell: Vec<u32>,
    /// Whether the FIFO reading reads the exit seconds (the table has its entrants).
    interpolated: bool,
}

/// The first and last entrants of `bins` by link then bin, each exit raised to the latest exit
/// before it on its link: `(starts, bins, knots)`; all empty if the table has none.
fn knots_by_link(links: usize, bins: &LinkBins) -> (Vec<u32>, Vec<u32>, Vec<Entrants>) {
    if bins.entrants().is_empty() {
        return (Vec::new(), Vec::new(), Vec::new());
    }
    let mut start = vec![0_u32; links + 1];
    for &l in bins.links() {
        start[l as usize + 1] += 1;
    }
    for l in 0..links {
        start[l + 1] += start[l];
    }
    let mut fill = start.clone();
    let mut bin = vec![0_u32; bins.len()];
    let mut knots = vec![Entrants::default(); bins.len()];
    for r in 0..bins.len() {
        let l = bins.links()[r] as usize;
        let slot = fill[l] as usize;
        fill[l] += 1;
        bin[slot] = bins.bins()[r];
        knots[slot] = bins.entrants()[r];
    }
    for l in 0..links {
        let mut latest = f64::NEG_INFINITY;
        for k in &mut knots[start[l] as usize..start[l + 1] as usize] {
            k.first_exit = k.first_exit.max(latest);
            k.last_exit = k.last_exit.max(k.first_exit);
            latest = k.last_exit;
        }
    }
    (start, bin, knots)
}

/// The exit second at `t` on the line through `(a_enter, a_exit)` and `(b_enter, b_exit)`; the
/// later exit where the two entered at the same second.
fn between(a_enter: f64, a_exit: f64, b_enter: f64, b_exit: f64, t: f64) -> f64 {
    if b_enter > a_enter {
        a_exit + (b_exit - a_exit) * ((t - a_enter) / (b_enter - a_enter))
    } else {
        a_exit.max(b_exit)
    }
}

/// One table's rows, by link then bin.
fn by_link(links: usize, bins: &LinkBins) -> (Vec<u32>, Vec<u32>, Vec<f64>) {
    let mut start = vec![0_u32; links + 1];
    for &l in bins.links() {
        start[l as usize + 1] += 1;
    }
    for l in 0..links {
        start[l + 1] += start[l];
    }
    let mut fill = start.clone();
    let mut bin = vec![0_u32; bins.len()];
    let mut mean = vec![0.0; bins.len()];
    for r in 0..bins.len() {
        let l = bins.links()[r] as usize;
        let slot = fill[l] as usize;
        fill[l] += 1;
        bin[slot] = bins.bins()[r];
        mean[slot] = bins.mean_seconds(r);
    }
    (start, bin, mean)
}

/// How many bins after one in which traffic took `mean` seconds that time, less the bins
/// elapsed, still exceeds `free_flow`: the bins a recorded time is carried into past the last
/// recorded bin.
fn carried_bins(mean: f64, free_flow: f64, bin_seconds: f64) -> usize {
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a count of bins, non-negative and bounded by the window"
    )]
    let n = (((mean - free_flow) / bin_seconds).ceil() - 1.0).max(0.0) as usize;
    n
}

/// Each link's times laid out from its first recorded bin to the last bin a recorded time
/// reaches; a bin with no row holds the last recorded time carried forward, less the bins
/// elapsed, floored at the free-flow time: `(first bin, offsets, times, last recorded bin)`, the
/// last counted from the first (see [`LinkTimes`]).
fn spans(
    links: usize,
    start: &[u32],
    bin: &[u32],
    mean: &[f64],
    free_flow: &[f64],
    bin_seconds: f64,
) -> (Vec<u32>, Vec<u32>, Vec<f64>, Vec<u32>) {
    let mut first = vec![0_u32; links];
    let mut offsets = vec![0_u32; links + 1];
    let mut span = Vec::new();
    let mut last_recorded = vec![0_u32; links];
    for l in 0..links {
        let rows = start[l] as usize..start[l + 1] as usize;
        if let (Some(&lo), Some(&hi)) = (bin[rows.clone()].first(), bin[rows.clone()].last()) {
            first[l] = lo;
            last_recorded[l] = hi - lo;
            let base = span.len();
            let tail = carried_bins(mean[rows.end - 1], free_flow[l], bin_seconds);
            span.resize(base + (hi - lo) as usize + 1 + tail, f64::NAN);
            for r in rows {
                span[base + (bin[r] - lo) as usize] = mean[r];
            }
            // The carry: a bin with no row reads the last recorded time less the bins elapsed.
            let mut last: Option<(usize, f64)> = None;
            for (k, cell) in span.iter_mut().enumerate().skip(base) {
                if cell.is_nan() {
                    *cell = match last {
                        #[allow(clippy::cast_precision_loss, reason = "a bin count far below 2^52")]
                        Some((k0, m0)) => (m0 - (k - k0) as f64 * bin_seconds).max(free_flow[l]),
                        None => free_flow[l],
                    };
                } else {
                    last = Some((k, *cell));
                }
            }
        }
        offsets[l + 1] = u32::try_from(span.len()).expect("fewer than 2^32 link-bin cells");
    }
    (first, offsets, span, last_recorded)
}

/// The mean in `bin` of a link's rows `rows` of `(bins, means)`, if it has one.
fn lookup(bins: &[u32], means: &[f64], rows: core::ops::Range<usize>, wanted: u32) -> Option<f64> {
    bins[rows.clone()].binary_search(&wanted).ok().map(|i| means[rows.start + i])
}

impl LinkTimes {
    /// Times from a loading's [`EntryTables`]: link times by the bin of entry (a bin
    /// nobody entered reading the last recorded time carried forward, down to free
    /// flow; see the module docs) and the wait at an origin by the bin of departure,
    /// zero where nobody was recorded. A table recorded with its entrants
    /// ([`crate::LinkBins::entrants`]) also keeps them, for the FIFO reading.
    ///
    /// # Panics
    ///
    /// Never in practice: a day's bins and a table's rows fit `u32`.
    #[must_use]
    pub fn from_tables(network: &RoadNetwork, tables: &EntryTables) -> Self {
        let links = network.link_count() as usize;
        let (start, bin, mean) = by_link(links, &tables.entry);
        let (wait_start, wait_bin, wait_mean) = by_link(links, &tables.origin_wait);
        let free_flow: Vec<f64> = (0..network.link_count())
            .map(|i| network.free_flow_time(LinkId::new(i)).get())
            .collect();
        let bin_seconds = f64::from(tables.entry.bin_seconds());
        let (span_first, span_start, span, span_last) =
            spans(links, &start, &bin, &mean, &free_flow, bin_seconds);
        let (knot_start, knot_bin, knots) = knots_by_link(links, &tables.entry);
        // Each cell's latest recorded bin, in `knots`.
        let mut knot_of_cell = Vec::new();
        if !knots.is_empty() {
            knot_of_cell.reserve(span.len());
            for l in 0..links {
                let (a, z) = (span_start[l] as usize, span_start[l + 1] as usize);
                let mut j = knot_start[l] as usize;
                for k in 0..z - a {
                    let cell_bin = span_first[l] + u32::try_from(k).expect("bins fit u32");
                    while j + 1 < knot_start[l + 1] as usize && knot_bin[j + 1] <= cell_bin {
                        j += 1;
                    }
                    knot_of_cell.push(u32::try_from(j).expect("fewer than 2^32 rows"));
                }
            }
        }
        Self {
            bin_seconds,
            free_flow,
            wait_start,
            wait_bin,
            wait_mean,
            span_first,
            span_start,
            span,
            span_last,
            closures: Vec::new(),
            has_closure: Vec::new(),
            exit_floor: Vec::new(),
            exit_floor_after: Vec::new(),
            knot_start,
            knot_bin,
            knots,
            knot_of_cell,
            interpolated: false,
        }
    }

    /// These times with the closures the travellers know of (S300), `(link, from, to)` in
    /// seconds: entering a closed link at `at` in `[from, to)` costs the wait until it reopens
    /// plus its time when entered then, the loading's own rule (a closed link takes no vehicle
    /// in). Read from the closure itself, not from who crossed the link: nobody enters a closed
    /// link, so its recorded bins say nothing of it. A disruption the travellers know of is the
    /// network as it will be, for every route cost read from these times.
    #[must_use]
    pub fn with_closures(mut self, closures: &[(u32, f64, f64)]) -> Self {
        if closures.is_empty() {
            return self;
        }
        self.has_closure = vec![false; self.free_flow.len()];
        for &(l, _, _) in closures {
            self.has_closure[l as usize] = true;
        }
        self.closures = closures.to_vec();
        self
    }

    /// These times read by `reading` (see the module docs): [`LinkTimesReading::Bins`], the
    /// default, as they were built; [`LinkTimesReading::Fifo`] with each link's floors of exit
    /// seconds carried forward once, here, so that a read stays one lookup.
    #[must_use]
    pub fn with_reading(mut self, reading: LinkTimesReading) -> Self {
        self.exit_floor = Vec::new();
        self.exit_floor_after = Vec::new();
        self.interpolated = reading == LinkTimesReading::Fifo && !self.knots.is_empty();
        if reading == LinkTimesReading::Bins || self.interpolated {
            return self;
        }
        self.exit_floor = vec![f64::NEG_INFINITY; self.span.len()];
        self.exit_floor_after = vec![f64::NEG_INFINITY; self.free_flow.len()];
        for (l, &first) in self.span_first.iter().enumerate() {
            let (a, z) = (self.span_start[l] as usize, self.span_start[l + 1] as usize);
            if a == z {
                continue;
            }
            // Entries before the first recorded bin read free flow; there are none before 0.
            let start = f64::from(first) * self.bin_seconds;
            let mut floor = if first > 0 { start + self.free_flow[l] } else { f64::NEG_INFINITY };
            for (k, &seconds) in self.span[a..z].iter().enumerate() {
                self.exit_floor[a + k] = floor;
                #[allow(clippy::cast_precision_loss, reason = "a bin count far below 2^52")]
                let next = start + (k + 1) as f64 * self.bin_seconds;
                floor = floor.max(next + seconds);
            }
            self.exit_floor_after[l] = floor;
        }
        self
    }

    /// The reading these times are read by ([`Self::with_reading`]).
    #[must_use]
    pub fn reading(&self) -> LinkTimesReading {
        if self.interpolated || !self.exit_floor_after.is_empty() {
            LinkTimesReading::Fifo
        } else {
            LinkTimesReading::Bins
        }
    }

    /// `seconds`, raised to the FIFO reading's floor `floor` for an entry at `at`, if there is one.
    fn floored(seconds: f64, floor: Option<&f64>, at: f64) -> f64 {
        match floor {
            Some(&exit) => seconds.max(exit - at),
            None => seconds,
        }
    }

    /// When `link`, closed at `at` by a closure the travellers know of, reopens.
    fn reopens(&self, link: u32, at: f64) -> Option<f64> {
        if !self.has_closure.get(link as usize).copied().unwrap_or(false) {
            return None;
        }
        self.closures
            .iter()
            .find(|&&(l, from, to)| l == link && from <= at && at < to)
            .map(|&(_, _, to)| to)
    }

    /// Free-flow times everywhere: what the first assignment sees.
    #[must_use]
    pub fn free_flow(network: &RoadNetwork) -> Self {
        let links = network.link_count() as usize;
        Self {
            bin_seconds: 1.0,
            free_flow: (0..network.link_count())
                .map(|i| network.free_flow_time(LinkId::new(i)).get())
                .collect(),
            wait_start: vec![0; links + 1],
            wait_bin: Vec::new(),
            wait_mean: Vec::new(),
            span_first: vec![0; links],
            span_start: vec![0; links + 1],
            span: Vec::new(),
            span_last: vec![0; links],
            closures: Vec::new(),
            has_closure: Vec::new(),
            exit_floor: Vec::new(),
            exit_floor_after: Vec::new(),
            knot_start: Vec::new(),
            knot_bin: Vec::new(),
            knots: Vec::new(),
            knot_of_cell: Vec::new(),
            interpolated: false,
        }
    }

    /// The same time at every moment: `seconds` per link (level 1's equilibrium times, S294,
    /// none below the link's free-flow time), no wait at any origin.
    ///
    /// # Panics
    ///
    /// Panics if `seconds` is not one time per link of `network`.
    #[must_use]
    pub fn constant(network: &RoadNetwork, seconds: Vec<f64>) -> Self {
        let links = network.link_count() as usize;
        assert_eq!(seconds.len(), links, "one time per link");
        Self { free_flow: seconds, ..Self::free_flow(network) }
    }

    /// The bin length the table was recorded with, in seconds.
    #[must_use]
    pub fn bin_seconds(&self) -> f64 {
        self.bin_seconds
    }

    /// The recorded and carried times as rows `(link, bin, seconds)`, bins of
    /// [`Self::bin_seconds`] counted from midnight, by link then bin: what a link takes when
    /// entered in a bin that has a row, free flow in one that has none (S303: the road times of a
    /// run's service day), under the bins reading. Closures, origin waits and the FIFO reading's
    /// floors are not among them.
    ///
    /// # Panics
    ///
    /// Never in practice: a network's links and a day's bins fit `u32`.
    #[must_use]
    pub fn rows(&self) -> Vec<(u32, u32, f64)> {
        let mut out = Vec::with_capacity(self.span.len());
        for (l, &first) in self.span_first.iter().enumerate() {
            let (a, z) = (self.span_start[l] as usize, self.span_start[l + 1] as usize);
            let link = u32::try_from(l).expect("links fit u32");
            for (k, &s) in self.span[a..z].iter().enumerate() {
                out.push((link, first + u32::try_from(k).expect("bins fit u32"), s));
            }
        }
        out
    }

    fn bin_of(&self, at: f64) -> u32 {
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a non-negative time in seconds over a bin length, within the u32 clock"
        )]
        let bin = (at.max(0.0) / self.bin_seconds) as u32;
        bin
    }

    /// The time to cross `link` when entering it at second `at`, by the times' reading
    /// ([`Self::with_reading`]): under a known closure, the wait until it reopens plus its time
    /// then ([`Self::with_closures`]).
    #[must_use]
    pub fn link_seconds(&self, link: u32, at: f64) -> f64 {
        if let Some(to) = self.reopens(link, at) {
            return (to - at) + self.recorded_seconds(link, to);
        }
        self.recorded_seconds(link, at)
    }

    /// [`Self::link_seconds`] from the recorded table alone.
    fn recorded_seconds(&self, link: u32, at: f64) -> f64 {
        if self.interpolated {
            return self.interpolated_seconds(link, at, false);
        }
        let l = link as usize;
        let (a, z) = (self.span_start[l] as usize, self.span_start[l + 1] as usize);
        let bin = self.bin_of(at);
        let k = bin.wrapping_sub(self.span_first[l]) as usize;
        if k < z - a {
            return Self::floored(self.span[a + k], self.exit_floor.get(a + k), at);
        }
        if self.exit_floor_after.is_empty() || bin < self.span_first[l] {
            return self.free_flow[l];
        }
        Self::floored(self.free_flow[l], self.exit_floor_after.get(l), at)
    }

    /// The time to cross `link` when entering it at second `at`, as [`Self::link_seconds`]
    /// reads it up to the link's **last recorded bin**, and that bin's time **held** for any
    /// `at` after it (under the FIFO reading, with that bin's floor), where
    /// [`Self::link_seconds`] carries it down to free flow. What costs the
    /// rest of a trip still under way when the window ended (the gap measures of the assignment): not the free flow
    /// the carry decays to, though in a queue still growing at the end the last recorded time
    /// is below what those behind it will take. Free flow on a link with no recorded time, and
    /// before its first recorded bin.
    #[must_use]
    pub fn link_seconds_held(&self, link: u32, at: f64) -> f64 {
        if let Some(to) = self.reopens(link, at) {
            return (to - at) + self.link_seconds_held(link, to);
        }
        if self.interpolated {
            return self.interpolated_seconds(link, at, true);
        }
        let l = link as usize;
        let (a, z) = (self.span_start[l] as usize, self.span_start[l + 1] as usize);
        let bin = self.bin_of(at);
        if a == z || bin < self.span_first[l] {
            return self.free_flow[l];
        }
        let k = ((bin - self.span_first[l]) as usize).min(self.span_last[l] as usize);
        // Past the last recorded bin, its floor still holds: the held time does not fall.
        Self::floored(self.span[a + k], self.exit_floor.get(a + k), at)
    }

    /// The FIFO reading by the exit seconds (see the module docs) of `link` entered at `at`; `held`: past
    /// the last recorded entrant, the last entrant's time held instead of carried.
    fn interpolated_seconds(&self, link: u32, at: f64, held: bool) -> f64 {
        let l = link as usize;
        let free = self.free_flow[l];
        let (ka, kz) = (self.knot_start[l] as usize, self.knot_start[l + 1] as usize);
        let bin = self.bin_of(at);
        if ka == kz || bin < self.knot_bin[ka] {
            return free;
        }
        let (a, z) = (self.span_start[l] as usize, self.span_start[l + 1] as usize);
        let k = (bin - self.span_first[l]) as usize;
        // The latest recorded bin at or before `bin`.
        let j = if k < z - a { self.knot_of_cell[a + k] as usize } else { kz - 1 };
        let here = self.knots[j];
        let after = |t: f64| -> f64 {
            // After the bin's last entrant: on to the next bin's first if it is the next bin,
            // else behind the last exit, at free flow at least (the carry); held, its time.
            if held && j + 1 == kz {
                return t + (here.last_exit - here.last_enter);
            }
            match self.knots.get(j + 1).filter(|_| j + 1 < kz) {
                Some(next)
                    if self.knot_bin[j + 1] == self.knot_bin[j] + 1 && bin == self.knot_bin[j] =>
                {
                    between(here.last_enter, here.last_exit, next.first_enter, next.first_exit, t)
                }
                _ => here.last_exit.max(t + free),
            }
        };
        let exit = if self.knot_bin[j] == bin {
            if at < here.first_enter {
                // Before the bin's first entrant: from the last entrant of the bin before if it
                // was recorded, else behind the last exit before, at the first entrant's time.
                if j > ka && self.knot_bin[j - 1] + 1 == bin {
                    let before = self.knots[j - 1];
                    between(
                        before.last_enter,
                        before.last_exit,
                        here.first_enter,
                        here.first_exit,
                        at,
                    )
                } else {
                    let floor =
                        if j > ka { self.knots[j - 1].last_exit } else { f64::NEG_INFINITY };
                    floor.max(at + (here.first_exit - here.first_enter))
                }
            } else if at <= here.last_enter {
                between(here.first_enter, here.first_exit, here.last_enter, here.last_exit, at)
            } else {
                after(at)
            }
        } else {
            after(at)
        };
        (exit - at).max(free)
    }

    /// The bytes these times hold (closures aside).
    #[must_use]
    pub fn bytes(&self) -> usize {
        let f = size_of::<f64>();
        let u = size_of::<u32>();
        self.free_flow.len() * f
            + (self.wait_start.len() + self.wait_bin.len()) * u
            + self.wait_mean.len() * f
            + (self.span_first.len() + self.span_start.len() + self.span_last.len()) * u
            + self.span.len() * f
            + (self.exit_floor.len() + self.exit_floor_after.len()) * f
            + (self.knot_start.len() + self.knot_bin.len() + self.knot_of_cell.len()) * u
            + self.knots.len() * size_of::<Entrants>()
    }

    /// The wait outside the network of someone who sets out at second `departure`
    /// onto `link`: zero where nobody who did so was recorded.
    #[must_use]
    pub fn origin_wait_seconds(&self, link: u32, departure: f64) -> f64 {
        let l = link as usize;
        let rows = self.wait_start[l] as usize..self.wait_start[l + 1] as usize;
        lookup(&self.wait_bin, &self.wait_mean, rows, self.bin_of(departure)).unwrap_or(0.0)
    }

    /// The time to drive `links` when setting out at second `departure`: the wait to
    /// get onto the first link, then the route walked link by link.
    #[must_use]
    pub fn route_seconds(&self, links: &[u32], departure: f64) -> f64 {
        let Some(&first) = links.first() else { return 0.0 };
        let mut t = departure + self.origin_wait_seconds(first, departure);
        for &l in links {
            t += self.link_seconds(l, t);
        }
        t - departure
    }
}

/// How much the link times changed between two loadings, as a share of the
/// earlier ones, weighted by traffic: `Σ w |m_now − m_before| / Σ w m_before`
/// over every (link, bin) either loading has a row for, in both tables of
/// [`EntryTables`] (link times, where a missing row is the link's free-flow time,
/// and origin waits, where it is zero), `w` being the PCU of the later row (the
/// earlier if there is none). Zero when nothing moved.
///
/// Both must have the same bin length.
///
/// # Panics
///
/// Panics if the bin lengths differ.
#[must_use]
pub fn relative_time_change(network: &RoadNetwork, before: &EntryTables, now: &EntryTables) -> f64 {
    let free = |l: u32| network.free_flow_time(LinkId::new(l)).get();
    let (m1, b1) = table_change(&before.entry, &now.entry, free);
    let (m2, b2) = table_change(&before.origin_wait, &now.origin_wait, |_| 0.0);
    let base = b1 + b2;
    if base > 0.0 { (m1 + m2) / base } else { 0.0 }
}

/// `(Σ w |Δ|, Σ w m_before)` over the union of two tables' rows.
fn table_change(before: &LinkBins, now: &LinkBins, missing: impl Fn(u32) -> f64) -> (f64, f64) {
    assert_eq!(before.bin_seconds(), now.bin_seconds(), "compare tables of one bin length");
    let (mut i, mut j) = (0, 0);
    let (mut moved, mut base) = (0.0, 0.0);
    let key = |t: &LinkBins, r: usize| (t.bins()[r], t.links()[r]);
    while i < before.len() || j < now.len() {
        let take_before = j >= now.len() || (i < before.len() && key(before, i) <= key(now, j));
        let take_now = i >= before.len() || (j < now.len() && key(now, j) <= key(before, i));
        let (m_before, m_now, w) = match (take_before, take_now) {
            (true, true) => (before.mean_seconds(i), now.mean_seconds(j), now.pcu()[j]),
            (true, false) => (before.mean_seconds(i), missing(before.links()[i]), before.pcu()[i]),
            _ => (missing(now.links()[j]), now.mean_seconds(j), now.pcu()[j]),
        };
        moved += w * (m_now - m_before).abs();
        base += w * m_before;
        if take_before {
            i += 1;
        }
        if take_now {
            j += 1;
        }
    }
    (moved, base)
}

#[cfg(test)]
mod tests {
    //! The two readings of the link times, against values derived by hand and against a
    //! brute-force running maximum of the exit seconds; FIFO on a point queue whose queue
    //! clears.

    use super::*;
    use crate::{
        FidelityLevel, LinkBinRecorder, Vehicle, run_ltm_recorded, run_ltm_recorded_with_entrants,
    };
    use openmobisim_core_graph::defaults::{GlobalMultipliers, RoadClass, SignalDefaults};
    use openmobisim_core_graph::geometry::LonLat;
    use openmobisim_core_graph::network::{LinkSpec, RoadNetworkBuilder};
    use openmobisim_core_graph::turns::TurnTable;
    use openmobisim_core_types::diagnostics::Diagnostics;
    use openmobisim_core_types::ids::VehicleId;
    use openmobisim_core_types::time::Second;
    use openmobisim_core_types::units::{Duration, Pcu};
    use proptest::prelude::*;

    /// A line `a → b → c` of two 100-m links, `ab` of `capacity` vehicles an hour.
    fn line(capacity: f64) -> RoadNetwork {
        let mut b = RoadNetworkBuilder::new();
        for (n, x) in [("a", 4.800), ("b", 4.801), ("c", 4.802)] {
            b.add_node(n, LonLat::new(x, 45.7));
        }
        for (name, from, to, cap) in [("ab", "a", "b", capacity), ("bc", "b", "c", 7200.0)] {
            let mut spec = LinkSpec::new(RoadClass::Primary);
            spec.length_m = Some(100.0);
            spec.capacity_veh_h = Some(cap);
            b.add_link(name, from, to, spec);
        }
        b.build(GlobalMultipliers::default(), SignalDefaults::SHIPPED, &mut Diagnostics::new())
            .expect("buildable")
    }

    /// Times on `net` from rows `(link, bin, seconds)` of `bin_seconds`, no origin wait.
    fn times(net: &RoadNetwork, bin_seconds: u32, rows: &[(u32, u32, f64)]) -> LinkTimes {
        let entry = LinkBins::from_rows(
            bin_seconds,
            rows.iter().map(|&(l, b, s)| (b, l, 1, 1.0, s)).collect(),
        );
        let origin_wait = LinkBins::from_rows(bin_seconds, Vec::new());
        LinkTimes::from_tables(net, &EntryTables { entry, origin_wait })
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    /// **Derived by hand:** one link, 300-s bins, 300 s in bin 0 and 100 s in bin 1. The bins
    /// reading: entering at 299 s leaves at 599 s, at 300 s at 400 s (a drop of 199 s). FIFO:
    /// the latest exit before 300 s is 600 s, so bin 1 reads `max(100, 600 − t)`: 300 s at
    /// 300 s, 150 s at 450 s, 100 s from 500 s; past the span (600 s) the floor is
    /// `max(600, 600 + 100) = 700`, so `max(free, 700 − t)`.
    #[test]
    fn a_falling_time_reads_the_wait_behind_the_last_exit_under_fifo() {
        let net = line(1800.0);
        let free = net.free_flow_time(LinkId::new(0)).get();
        assert!(free < 50.0, "a 100-m link: {free}");
        let bins = times(&net, 300, &[(0, 0, 300.0), (0, 1, 100.0)]);
        let fifo = bins.clone().with_reading(LinkTimesReading::Fifo);
        assert_eq!(
            (bins.reading(), fifo.reading()),
            (LinkTimesReading::Bins, LinkTimesReading::Fifo)
        );
        // The bins reading, unchanged.
        for (at, want) in [(299.0, 300.0), (300.0, 100.0), (599.0, 100.0), (600.0, free)] {
            assert!(close(bins.link_seconds(0, at), want), "bins at {at}");
        }
        // The FIFO reading.
        for (at, want) in [
            (0.0, 300.0),
            (299.0, 300.0),
            (299.5, 300.0),
            (300.0, 300.0),
            (301.0, 299.0),
            (450.0, 150.0),
            (500.0, 100.0),
            (550.0, 100.0),
            (600.0, 100.0),
            (650.0, 50.0),
            (700.0 - free, free),
            (900.0, free),
        ] {
            assert!(
                close(fifo.link_seconds(0, at), want),
                "fifo at {at}: {}",
                fifo.link_seconds(0, at)
            );
        }
        // Held past the last recorded bin (1): 100 s, and its floor (600 s) inside bin 1.
        assert!(close(fifo.link_seconds_held(0, 320.0), 280.0));
        assert!(close(fifo.link_seconds_held(0, 650.0), 100.0));
        assert!(close(bins.link_seconds_held(0, 320.0), 100.0));
        // A route walks the same reading: entering at 299.5 s the first link takes 300 s.
        assert!(close(fifo.route_seconds(&[0, 1], 299.5), 300.0 + free_of(&net, 1)));
        // A link with no row, and the free-flow times, are the same under both.
        assert!(close(fifo.link_seconds(1, 450.0), free_of(&net, 1)));
        let ff = LinkTimes::free_flow(&net).with_reading(LinkTimesReading::Fifo);
        assert!(close(ff.link_seconds(0, 450.0), free));
    }

    fn free_of(net: &RoadNetwork, l: u32) -> f64 {
        net.free_flow_time(LinkId::new(l)).get()
    }

    /// **Derived by hand:** a carried time drops by exactly one bin per bin (700 s in bin 0 of
    /// 300 s reads 400 s in bin 1, 100 s in bin 2): under the bins reading the exit second falls
    /// by 300 s at each boundary; under FIFO everyone entering from 300 s until `1000 − free`
    /// leaves at 1 000 s, behind the last entrant of bin 0, as in a queue discharging.
    #[test]
    fn a_carried_time_holds_the_exit_of_the_last_entrant_under_fifo() {
        let net = line(1800.0);
        let free = free_of(&net, 0);
        let bins = times(&net, 300, &[(0, 0, 700.0)]);
        let fifo = bins.clone().with_reading(LinkTimesReading::Fifo);
        assert!(
            close(bins.link_seconds(0, 300.0), 400.0) && close(bins.link_seconds(0, 600.0), 100.0)
        );
        for at in [300.0, 400.0, 599.0, 600.0, 899.0, 900.0, 950.0, 1000.0 - free] {
            assert!(close(at + fifo.link_seconds(0, at), 1000.0), "exit of an entry at {at}");
        }
        assert!(close(fifo.link_seconds(0, 1000.0), free));
    }

    /// **Property:** a closure the travellers know of keeps the FIFO reading FIFO: on the link
    /// above, closed from 250 s to 320 s, an entry at 249 s leaves at 549 s, one at 260 s waits
    /// until 320 s and leaves at 600 s (the floor), one at 330 s at 600 s too.
    #[test]
    fn a_closure_keeps_the_fifo_reading_fifo() {
        let net = line(1800.0);
        let fifo = times(&net, 300, &[(0, 0, 300.0), (0, 1, 100.0)])
            .with_reading(LinkTimesReading::Fifo)
            .with_closures(&[(0, 250.0, 320.0)]);
        for (at, exit) in [(249.0, 549.0), (260.0, 600.0), (319.0, 600.0), (330.0, 600.0)] {
            assert!(close(at + fifo.link_seconds(0, at), exit), "entry at {at}");
        }
    }

    /// The bins of `rows` on one link as `(bin, seconds)` cells from its first recorded bin to
    /// the end of its span, as `times` laid them out.
    fn cells(t: &LinkTimes) -> Vec<(u32, f64)> {
        t.rows().into_iter().filter(|r| r.0 == 0).map(|r| (r.1, r.2)).collect()
    }

    proptest! {
        /// **Property:** the FIFO reading is the running maximum of the bins reading's exit
        /// seconds: at every entry second `t`, `t + τ_FIFO(t)` is the larger of `t + τ(t)` and
        /// the exit of an entry just before each earlier boundary (`s_k + τ_{k−1}`), computed
        /// here from every boundary at once rather than carried; so it never falls, is never
        /// below the bins reading, and equals it where the bins reading never falls.
        #[test]
        fn the_fifo_reading_is_the_running_maximum_of_the_exits(
            raw in prop::collection::vec((0_u32..3, 0.0_f64..900.0), 1..12),
            first in 0_u32..4,
        ) {
            let net = line(1800.0);
            let free = free_of(&net, 0);
            let mut rows = Vec::new();
            let mut bin = first;
            for (gap, s) in raw {
                bin += gap;
                if rows.iter().all(|r: &(u32, u32, f64)| r.1 != bin) {
                    rows.push((0, bin, free + s));
                }
                bin += 1;
            }
            let bins = times(&net, 60, &rows);
            let fifo = bins.clone().with_reading(LinkTimesReading::Fifo);
            let cells = cells(&bins);
            let lo = cells[0].0;
            let hi = cells[cells.len() - 1].0 + 1;
            // The exit of an entry just before each boundary: before the span, free flow.
            let mut before: Vec<(f64, f64)> = Vec::new();
            if lo > 0 {
                before.push((f64::from(lo) * 60.0, f64::from(lo) * 60.0 + free));
            }
            for &(b, s) in &cells {
                before.push((f64::from(b + 1) * 60.0, f64::from(b + 1) * 60.0 + s));
            }
            let mut last_exit = f64::NEG_INFINITY;
            let mut rose = true;
            let mut t = 0.0;
            while t < f64::from(hi) * 60.0 + 1200.0 {
                let step = bins.link_seconds(0, t);
                let read = fifo.link_seconds(0, t);
                let floor = before.iter().filter(|b| b.0 <= t).map(|b| b.1).fold(f64::NEG_INFINITY, f64::max);
                prop_assert!((t + read - (t + step).max(floor)).abs() < 1e-9, "at {t}");
                prop_assert!(read >= step);
                prop_assert!(t + read >= last_exit - 1e-9, "FIFO at {t}");
                rose &= t + step >= last_exit - 1e-9 || last_exit == f64::NEG_INFINITY;
                last_exit = last_exit.max(t + read);
                t += 0.5;
            }
            if rose {
                let mut t = 0.0;
                while t < f64::from(hi) * 60.0 + 1200.0 {
                    prop_assert_eq!(fifo.link_seconds(0, t).to_bits(), bins.link_seconds(0, t).to_bits());
                    t += 0.5;
                }
            }
        }
    }

    /// **Property, on a loading:** a point queue that builds for 600 s (a car a second onto a
    /// link that lets one out every 2 s) and then clears (a car every 4 s, for 1 200 s) records
    /// times that fall by `(1/2 − 1/4) / (1/2) · 60 = 30 s` per 60-s bin while it clears:
    /// the bins reading's exit second drops at those boundaries; the FIFO reading's never does,
    /// at any boundary of any link (checked just before and at each, and across each bin).
    #[test]
    fn a_queue_clearing_breaks_fifo_under_the_bins_and_not_under_the_fifo_reading() {
        let net = line(1800.0);
        let turns = TurnTable::build(&net, SignalDefaults::SHIPPED);
        let mut cars = Vec::new();
        let mut at = 0;
        while at < 1800 {
            let id = u32::try_from(cars.len()).expect("few cars");
            cars.push(Vehicle::new(
                VehicleId::new(id),
                vec![LinkId::new(0), LinkId::new(1)],
                Pcu(1.0),
                Second(at),
            ));
            at += if at < 600 { 1 } else { 4 };
        }
        let (_, _, tables) = run_ltm_recorded(
            &net,
            &turns,
            &cars,
            Duration(7200.0),
            Duration(60.0),
            FidelityLevel::PointQueue,
            60,
        );
        let bins = LinkTimes::from_tables(&net, &tables);
        let fifo = bins.clone().with_reading(LinkTimesReading::Fifo);
        let drops = |t: &LinkTimes| -> Vec<f64> {
            let mut out = Vec::new();
            for l in 0..net.link_count() {
                for b in 1..150_u32 {
                    let s = f64::from(b) * 60.0;
                    let drop =
                        (s - 1e-3 + t.link_seconds(l, s - 1e-3)) - (s + t.link_seconds(l, s));
                    if drop > 1e-6 {
                        out.push(drop);
                    }
                }
            }
            out
        };
        let (d_bins, d_fifo) = (drops(&bins), drops(&fifo));
        // The queue (300 cars at 600 s) clears at 1 800 s: 19 boundaries from 660 s to 1 740 s.
        let near_30 = d_bins.iter().filter(|&&d| (29.9..30.1).contains(&d)).count();
        assert_eq!(near_30, 19, "the clearing queue's drops of 30 s: {d_bins:?}");
        assert!(d_fifo.is_empty(), "no drop under FIFO: {d_fifo:?}");
        // And within each bin, every second: the exit never falls.
        for l in 0..net.link_count() {
            let mut last = f64::NEG_INFINITY;
            for s in 0..9000 {
                let t = f64::from(s);
                let exit = t + fifo.link_seconds(l, t);
                assert!(exit >= last - 1e-9, "link {l} at {t}");
                last = exit;
            }
        }
    }

    /// **Property:** relabelling the links relabels the times: the same table filed under other
    /// link numbers reads the same times, link for link, under the FIFO reading.
    #[test]
    fn relabelled_links_read_the_same_fifo_times() {
        let net = line(1800.0);
        let rows = [(0, 0, 300.0), (0, 1, 100.0), (1, 2, 200.0), (1, 4, 20.0)];
        let swapped: Vec<(u32, u32, f64)> = rows.iter().map(|&(l, b, s)| (1 - l, b, s)).collect();
        let (a, b) = (
            times(&net, 300, &rows).with_reading(LinkTimesReading::Fifo),
            times(&net, 300, &swapped).with_reading(LinkTimesReading::Fifo),
        );
        // The two links have the same length and class, so the same free-flow time: the swapped
        // table is the same times on the other link.
        assert!(close(free_of(&net, 0), free_of(&net, 1)));
        for s in 0..3000 {
            let t = f64::from(s);
            for l in 0..2 {
                assert_eq!(a.link_seconds(l, t).to_bits(), b.link_seconds(1 - l, t).to_bits());
            }
        }
    }

    /// Times on `net` recorded by hand with their entrants: `(link, enter, exit)` in exit order,
    /// 60-s bins.
    fn recorded(net: &RoadNetwork, traversals: &[(u32, f64, f64)]) -> LinkTimes {
        let links = net.link_count() as usize;
        let mut r = LinkBinRecorder::new(links, 60, 7200.0).with_entry_bins().with_entrants();
        for &(l, enter, exit) in traversals {
            r.record(LinkId::new(l), enter, exit, 1.0);
        }
        let (_, tables) = r.finish_with_entry();
        LinkTimes::from_tables(net, &tables.expect("entry tables"))
    }

    /// **Derived by hand:** one entrant in bin 0 (10 s → 110 s), none in bin 1, two in bin 2
    /// (130 → 200 s, 170 → 260 s). The FIFO reading by the exit seconds: 5 s, before the first entrant, takes
    /// its 100 s; 30 s, after bin 0's only entrant with the next recorded bin not adjacent,
    /// leaves behind it at 110 s (80 s); 90 s, in the empty bin, likewise (20 s); 125 s, before
    /// bin 2's first entrant, `max(110, 125 + 70)` = 195 s (70 s); 150 s, between bin 2's
    /// two, 230 s (80 s); 200 s, after the last, behind it at 260 s (60 s), then free flow;
    /// held, the last entrant's 90 s.
    #[test]
    fn the_interpolated_exit_is_read_between_the_recorded_entrants() {
        let net = line(1800.0);
        let free = free_of(&net, 0);
        assert!(free < 20.0, "{free}");
        let t = recorded(&net, &[(0, 10.0, 110.0), (0, 130.0, 200.0), (0, 170.0, 260.0)])
            .with_reading(LinkTimesReading::Fifo);
        assert_eq!(t.reading(), LinkTimesReading::Fifo);
        for (at, want) in [
            (5.0, 100.0),
            (10.0, 100.0),
            (30.0, 80.0),
            (90.0, 20.0),
            (125.0, 70.0),
            (130.0, 70.0),
            (150.0, 80.0),
            (170.0, 90.0),
            (200.0, 60.0),
            (300.0, free),
        ] {
            assert!(close(t.link_seconds(0, at), want), "at {at}: {}", t.link_seconds(0, at));
        }
        assert!(close(t.link_seconds_held(0, 200.0), 90.0));
        assert!(close(t.link_seconds_held(0, 150.0), 80.0));
        // Before the first recorded bin and on a link nobody entered: free flow.
        assert!(close(t.link_seconds(1, 150.0), free_of(&net, 1)));
        // The bins reading of the same table is unchanged: bin 0 100 s, bin 2 (70 + 90)/2 = 80 s.
        let bins = recorded(&net, &[(0, 10.0, 110.0), (0, 130.0, 200.0), (0, 170.0, 260.0)]);
        assert!(
            close(bins.link_seconds(0, 150.0), 80.0) && close(bins.link_seconds(0, 30.0), 100.0)
        );
        // A closure moves an entry to the reopening: entering at 140 s, closed until 170 s.
        let closed = t.clone().with_closures(&[(0, 135.0, 170.0)]);
        assert!(close(140.0 + closed.link_seconds(0, 140.0), 260.0));
        // FIFO at every half second.
        let mut last = f64::NEG_INFINITY;
        for h in 0..1000 {
            let at = f64::from(h) * 0.5;
            let exit = at + closed.link_seconds(0, at);
            assert!(exit >= last - 1e-9, "at {at}");
            last = exit;
        }
    }

    /// **Derived by hand, on a loading:** a car a second for 600 s onto a link that lets one out
    /// every 2 s, then one every 4 s until 1 800 s. The car entering at `t` leaves at
    /// `free + 2t` while the queue grows (`t < 600`), and at `free + 1 200 + (t − 600)/2` while it
    /// clears (it is gone at 1 800 s): one straight line per phase through the recorded first and
    /// last entrants of every 60-s bin, so the FIFO reading by the exit seconds is exact at every second, the
    /// cars' and those between, where the bins reading is off by up to 30 s; FIFO everywhere.
    #[test]
    fn a_clearing_queue_is_read_exactly_by_the_interpolated_exit() {
        let net = line(1800.0);
        let free = free_of(&net, 0);
        let turns = TurnTable::build(&net, SignalDefaults::SHIPPED);
        let mut cars = Vec::new();
        let mut at = 0;
        while at < 1800 {
            let id = u32::try_from(cars.len()).expect("few cars");
            cars.push(Vehicle::new(
                VehicleId::new(id),
                vec![LinkId::new(0), LinkId::new(1)],
                Pcu(1.0),
                Second(at),
            ));
            at += if at < 600 { 1 } else { 4 };
        }
        let run = |entrants: bool| {
            let record = if entrants { run_ltm_recorded_with_entrants } else { run_ltm_recorded };
            record(
                &net,
                &turns,
                &cars,
                Duration(7200.0),
                Duration(60.0),
                FidelityLevel::PointQueue,
                60,
            )
        };
        let (traj_a, exit_a, tables) = run(true);
        let (traj_b, exit_b, plain) = run(false);
        // Keeping the entrants changes nothing else, to the bit.
        assert_eq!((exit_a.clone(), plain.entry.bins()), (exit_b, tables.entry.bins()));
        assert_eq!(plain.entry.pcu_seconds(), tables.entry.pcu_seconds());
        assert!(plain.entry.entrants().is_empty() && !tables.entry.entrants().is_empty());
        assert_eq!(traj_a.len(), traj_b.len());
        let t = LinkTimes::from_tables(&net, &tables).with_reading(LinkTimesReading::Fifo);
        let bins = LinkTimes::from_tables(&net, &tables);
        let mut worst_bins: f64 = 0.0;
        for s in 0..1796 {
            let at = f64::from(s);
            let want = if s < 600 { free + at } else { free + 600.0 - (at - 600.0) / 2.0 };
            assert!(
                (t.link_seconds(0, at) - want).abs() < 1e-9,
                "at {at}: {}",
                t.link_seconds(0, at)
            );
            worst_bins = worst_bins.max((bins.link_seconds(0, at) - want).abs());
        }
        assert!(worst_bins > 25.0, "the bins reading is off by up to 30 s: {worst_bins}");
        for l in 0..net.link_count() {
            let mut last = f64::NEG_INFINITY;
            for s in 0..18000 {
                let at = f64::from(s) * 0.5;
                let exit = at + t.link_seconds(l, at);
                assert!(exit >= last - 1e-9, "link {l} at {at}");
                last = exit;
            }
        }
    }

    /// **Property:** the FIFO reading by the exit seconds relabels with the links: the same traversals filed
    /// under the other link read the same times there, to the bit.
    #[test]
    fn relabelled_links_read_the_same_interpolated_times() {
        let net = line(1800.0);
        let rows = [(0, 10.0, 110.0), (1, 20.0, 150.0), (0, 130.0, 200.0), (1, 140.0, 230.0)];
        let swapped: Vec<(u32, f64, f64)> = rows.iter().map(|&(l, e, x)| (1 - l, e, x)).collect();
        let a = recorded(&net, &rows).with_reading(LinkTimesReading::Fifo);
        let b = recorded(&net, &swapped).with_reading(LinkTimesReading::Fifo);
        for s in 0..1000 {
            let t = f64::from(s) * 0.5;
            for l in 0..2 {
                assert_eq!(a.link_seconds(l, t).to_bits(), b.link_seconds(1 - l, t).to_bits());
            }
        }
    }

    /// A table with its entrants is read FIFO by them (the hand-made table above, 10 s after its
    /// last entrant: behind its exit at 260 s, 80 s), one without by the floor (the 300 / 100-s
    /// table of the first test: 150 s at 450 s).
    #[test]
    fn a_table_without_entrants_reads_the_fifo_floor() {
        let net = line(1800.0);
        let traversals = [(0, 10.0, 110.0), (0, 130.0, 200.0), (0, 170.0, 260.0)];
        let with = recorded(&net, &traversals).with_reading(LinkTimesReading::Fifo);
        assert!(close(with.link_seconds(0, 180.0), 80.0));
        let t = times(&net, 300, &[(0, 0, 300.0), (0, 1, 100.0)]);
        assert_eq!(
            t.clone().with_reading(LinkTimesReading::Fifo).reading(),
            LinkTimesReading::Fifo
        );
        assert!(close(t.with_reading(LinkTimesReading::Fifo).link_seconds(0, 450.0), 150.0));
    }

    #[test]
    fn the_readings_are_named() {
        assert_eq!(LinkTimesReading::default(), LinkTimesReading::Bins);
        for r in LinkTimesReading::ALL {
            assert_eq!(LinkTimesReading::from_name(r.name()), Some(r));
        }
        assert_eq!(LinkTimesReading::from_name("FIFO"), None);
        assert_eq!(LinkTimesReading::ALL.len(), 2);
    }
}
