//! What a link costs at a given moment, from the last loading (S170).
//!
//! After a loading, the per-link, per-time-bin table ([`LinkBins`]) says how
//! long traffic took to cross each link in each bin. [`LinkTimes`] reads it back
//! as a **time-dependent cost**: `link_seconds(link, t)` is the mean traversal
//! time in the bin containing `t`, or the link's free-flow time where nobody
//! crossed it. A route's expected time for a traveller leaving at `t` is found by
//! walking it link by link, arriving at each link at the time the last one
//! ended (design §11.1: time-dependent cost is fine under FIFO).
//!
//! **The table is by entry time.** A traveller about to enter a link needs the
//! time taken by those who *entered* when they would, not by those who happened to
//! *leave* then: in a growing queue the two differ by the queue's length, and
//! reading the exit-time table would show the queue late (found in S170 on a
//! hand-made bottleneck, where it hid a third of the disequilibrium). The loading
//! records both ([`openmobisim_core_loading::LinkBinRecorder::with_entry_bins`]);
//! only vehicles that finished the link inside the window count, so in a jam that
//! never clears the last stretch is optimistic.
//!
//! **Cost:** 12 bytes per recorded (link, bin) row plus 8 bytes per link; a lookup
//! is one binary search over that link's few bins, about 30 ns; a route of 150
//! links costs about 5 µs.

use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_loading::{EntryTables, LinkBins};
use openmobisim_core_types::ids::{EntityId, LinkId};

/// Link travel times by time of entry, from a loading's entry-time table.
#[derive(Clone, Debug)]
pub struct LinkTimes {
    bin_seconds: f64,
    /// `link_count + 1` offsets: link `l`'s rows are `start[l]..start[l + 1]`.
    start: Vec<u32>,
    bin: Vec<u32>,
    mean: Vec<f64>,
    free_flow: Vec<f64>,
    /// The wait to get onto each link from an origin, by the bin of departure, laid
    /// out like the times.
    wait_start: Vec<u32>,
    wait_bin: Vec<u32>,
    wait_mean: Vec<f64>,
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

/// The mean in `bin` of a link's rows `rows` of `(bins, means)`, if it has one.
fn lookup(bins: &[u32], means: &[f64], rows: core::ops::Range<usize>, wanted: u32) -> Option<f64> {
    bins[rows.clone()].binary_search(&wanted).ok().map(|i| means[rows.start + i])
}

impl LinkTimes {
    /// Times from a loading's [`EntryTables`]: link times by the bin of entry and
    /// the wait at an origin by the bin of departure, with free flow (and no wait)
    /// where nobody was recorded.
    #[must_use]
    pub fn from_tables(network: &RoadNetwork, tables: &EntryTables) -> Self {
        let links = network.link_count() as usize;
        let (start, bin, mean) = by_link(links, &tables.entry);
        let (wait_start, wait_bin, wait_mean) = by_link(links, &tables.origin_wait);
        Self {
            bin_seconds: f64::from(tables.entry.bin_seconds()),
            start,
            bin,
            mean,
            free_flow: (0..network.link_count())
                .map(|i| network.free_flow_time(LinkId::new(i)).get())
                .collect(),
            wait_start,
            wait_bin,
            wait_mean,
        }
    }

    /// Free-flow times everywhere: what the first assignment sees.
    #[must_use]
    pub fn free_flow(network: &RoadNetwork) -> Self {
        let links = network.link_count() as usize;
        Self {
            bin_seconds: 1.0,
            start: vec![0; links + 1],
            bin: Vec::new(),
            mean: Vec::new(),
            free_flow: (0..network.link_count())
                .map(|i| network.free_flow_time(LinkId::new(i)).get())
                .collect(),
            wait_start: vec![0; links + 1],
            wait_bin: Vec::new(),
            wait_mean: Vec::new(),
        }
    }

    /// The bin length the table was recorded with, in seconds.
    #[must_use]
    pub fn bin_seconds(&self) -> f64 {
        self.bin_seconds
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

    /// The time to cross `link` when entering it at second `at`.
    #[must_use]
    pub fn link_seconds(&self, link: u32, at: f64) -> f64 {
        let l = link as usize;
        let rows = self.start[l] as usize..self.start[l + 1] as usize;
        lookup(&self.bin, &self.mean, rows, self.bin_of(at)).unwrap_or(self.free_flow[l])
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
/// earlier if there is none). Zero when nothing moved; the stability number of
/// design §11.2.
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
