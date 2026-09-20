//! Per-link, per-time-bin results, recorded as the loading runs (S163).
//!
//! What a map, an emissions estimate or a calibration needs from a run is not
//! a trajectory per vehicle but **how much traffic left each link in each time
//! bin, and how long it took**: flow, hence speed and delay. [`LinkBins`] is
//! that table, sparse (only links that saw traffic in a bin have a row), and
//! it counts every traversal that *finished* — including those of vehicles
//! still on their way when the window ends, which a table built from finished
//! trips would silently drop from a congested run.
//!
//! # How it is recorded
//!
//! The loading processes movements in time order, so a link's exit times
//! arrive non-decreasing. [`LinkBinRecorder`] therefore keeps dense scratch
//! space for **one bin only** and flushes it as sparse rows when the clock
//! moves to the next bin. Memory is the scratch (20 bytes per link) plus one
//! 28-byte row per (link, bin) that saw traffic; time is a bin computation and
//! three additions per link crossing. When no recorder is attached the cost is
//! one predictable branch per crossing.
//!
//! A traversal is filed under the bin of its **exit** time.

use std::collections::HashMap;

use openmobisim_core_types::ids::{EntityId, LinkId};

/// The recorded table: one row per (bin, link) that saw traffic, sorted by
/// bin then link. Struct-of-arrays, so a caller can hand each column straight
/// to numpy.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LinkBins {
    bin_seconds: u32,
    bin: Vec<u32>,
    link: Vec<u32>,
    crossings: Vec<u32>,
    pcu: Vec<f64>,
    pcu_seconds: Vec<f64>,
}

impl LinkBins {
    /// The length of one time bin, in seconds.
    #[must_use]
    pub fn bin_seconds(&self) -> u32 {
        self.bin_seconds
    }

    /// How many rows the table has.
    #[must_use]
    pub fn len(&self) -> usize {
        self.bin.len()
    }

    /// Whether no traversal was recorded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bin.is_empty()
    }

    /// Each row's bin index; bin `b` covers `[b, b+1) × bin_seconds` seconds.
    #[must_use]
    pub fn bins(&self) -> &[u32] {
        &self.bin
    }

    /// Each row's link, as a raw [`LinkId`] index.
    #[must_use]
    pub fn links(&self) -> &[u32] {
        &self.link
    }

    /// Each row's number of finished traversals (vehicles, unweighted).
    #[must_use]
    pub fn crossings(&self) -> &[u32] {
        &self.crossings
    }

    /// Each row's traffic that left the link, in PCU (traveller weight and
    /// vehicle size included).
    #[must_use]
    pub fn pcu(&self) -> &[f64] {
        &self.pcu
    }

    /// Each row's PCU-weighted traversal time, in PCU·seconds. Divide by
    /// [`Self::pcu`] for the mean time a unit of traffic took on the link.
    #[must_use]
    pub fn pcu_seconds(&self) -> &[f64] {
        &self.pcu_seconds
    }

    /// The mean traversal time of row `row`, in seconds.
    ///
    /// # Panics
    ///
    /// Panics if `row` is out of range.
    #[must_use]
    pub fn mean_seconds(&self, row: usize) -> f64 {
        self.pcu_seconds[row] / self.pcu[row]
    }

    /// The bytes this table holds.
    #[must_use]
    pub fn bytes(&self) -> usize {
        self.len() * (3 * size_of::<u32>() + 2 * size_of::<f64>())
    }
}

/// What a loading learned about how long a traveller about to set out would take
/// (S170): the two tables an iterated run costs its routes from.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EntryTables {
    /// Traversals by the bin they **entered** their link in: the time to cross a
    /// link when arriving at it (its queue included; the wait to enter it from an
    /// origin is not).
    pub entry: LinkBins,
    /// The wait to get from an origin onto a first link, by the bin the traveller
    /// **departed** in: how long those who set out then waited outside the
    /// network before their first link had room. `crossings`, `pcu` and
    /// `pcu_seconds` are as in any [`LinkBins`], with the wait as the time.
    pub origin_wait: LinkBins,
}

/// Builds a [`LinkBins`] from traversals fed **in exit-time order**.
#[derive(Debug)]
pub struct LinkBinRecorder {
    bin_seconds: f64,
    end: f64,
    current: u32,
    // Dense scratch for the current bin.
    crossings: Vec<u32>,
    pcu: Vec<f64>,
    pcu_seconds: Vec<f64>,
    touched: Vec<u32>,
    out: LinkBins,
    /// If asked for (S170): the same traversals filed under the bin they
    /// **entered** the link in, which is what a traveller about to enter it needs
    /// to know. Sparse: entries arrive out of order.
    entry: Option<HashMap<u64, EntryCell>>,
    /// The waits at origins, by departure bin, when `entry` is on.
    origin_wait: Option<HashMap<u64, EntryCell>>,
}

/// What one (entry bin, link) cell has seen so far.
#[derive(Clone, Copy, Debug, Default)]
struct EntryCell {
    crossings: u32,
    pcu: f64,
    pcu_seconds: f64,
}

impl LinkBinRecorder {
    /// A recorder over `link_count` links, with bins of `bin_seconds`,
    /// ignoring any traversal that finishes at or after `end` (the window's
    /// end, in seconds).
    ///
    /// # Panics
    ///
    /// Panics if `bin_seconds` is zero.
    #[must_use]
    pub fn new(link_count: usize, bin_seconds: u32, end: f64) -> Self {
        assert!(bin_seconds > 0, "a time bin must be at least one second long");
        Self {
            bin_seconds: f64::from(bin_seconds),
            end,
            current: 0,
            crossings: vec![0; link_count],
            pcu: vec![0.0; link_count],
            pcu_seconds: vec![0.0; link_count],
            touched: Vec::new(),
            out: LinkBins { bin_seconds, ..LinkBins::default() },
            entry: None,
            origin_wait: None,
        }
    }

    /// The same recorder, also filing every traversal under the bin it
    /// **entered** in (S170), for [`Self::finish_with_entry`].
    ///
    /// The exit-time table says how much traffic left a link in each bin, which is
    /// what a flow map draws; a traveller about to enter the link needs the time
    /// taken by those who *entered* when they would, and in a growing queue the two
    /// differ by the queue's length. Costs a hash-map insert per traversal (about
    /// 30 ns) and 40 bytes per (link, bin) cell that saw traffic. It also files the
    /// wait of every vehicle at its origin ([`Self::record_origin_wait`]).
    #[must_use]
    pub fn with_entry_bins(mut self) -> Self {
        self.entry = Some(HashMap::new());
        self.origin_wait = Some(HashMap::new());
        self
    }

    /// Record that a vehicle of `pcu` PCU that set out at `departure` onto `link` has
    /// waited outside the network until `until`: the second it got onto the link, or
    /// the window's end if it never did (then a lower bound). Only the entry-time
    /// tables see it. Does nothing unless [`Self::with_entry_bins`] was called.
    pub fn record_origin_wait(&mut self, link: LinkId, departure: f64, until: f64, pcu: f64) {
        if let Some(waits) = self.origin_wait.as_mut() {
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "a non-negative time in seconds over a bin length; within the u32 clock"
            )]
            let bin = (departure.max(0.0) / self.bin_seconds) as u32;
            let cell = waits.entry((u64::from(bin) << 32) | u64::from(link.raw())).or_default();
            cell.crossings += 1;
            cell.pcu += pcu;
            cell.pcu_seconds += pcu * (until - departure).max(0.0);
        }
    }

    /// Record a traversal that had **not finished** when the window ended: a
    /// vehicle of `pcu` PCU that entered `link` at `enter` and was still on it, or
    /// waiting to enter it, at `end` (S170). Only the entry-time table sees it, and
    /// with the time it had taken so far, a **lower bound** on the time it will
    /// take: without it, a queue that outlasts the window would show only the
    /// vehicles that got through, and read as if it were short. Does nothing
    /// unless [`Self::with_entry_bins`] was called.
    pub fn record_unfinished(&mut self, link: LinkId, enter: f64, end: f64, pcu: f64) {
        if let Some(entry) = self.entry.as_mut() {
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "a non-negative time in seconds over a bin length; within the u32 clock"
            )]
            let entry_bin = (enter.max(0.0) / self.bin_seconds) as u32;
            let cell =
                entry.entry((u64::from(entry_bin) << 32) | u64::from(link.raw())).or_default();
            cell.crossings += 1;
            cell.pcu += pcu;
            cell.pcu_seconds += pcu * (end - enter).max(0.0);
        }
    }

    /// Record that a vehicle of `pcu` PCU entered `link` at `enter` and left
    /// it at `exit` (seconds). Calls must come in non-decreasing `exit`.
    pub fn record(&mut self, link: LinkId, enter: f64, exit: f64, pcu: f64) {
        if exit >= self.end {
            return;
        }
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a non-negative time in seconds over a bin length; within the u32 clock"
        )]
        let bin = (exit.max(0.0) / self.bin_seconds) as u32;
        debug_assert!(bin >= self.current, "exits must arrive in time order");
        if bin > self.current {
            self.flush();
            self.current = bin;
        }
        let l = link.index();
        if self.crossings[l] == 0 {
            self.touched.push(link.raw());
        }
        self.crossings[l] += 1;
        self.pcu[l] += pcu;
        self.pcu_seconds[l] += pcu * (exit - enter);
        if let Some(entry) = self.entry.as_mut() {
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "a non-negative time in seconds over a bin length; within the u32 clock"
            )]
            let entry_bin = (enter.max(0.0) / self.bin_seconds) as u32;
            let cell =
                entry.entry((u64::from(entry_bin) << 32) | u64::from(link.raw())).or_default();
            cell.crossings += 1;
            cell.pcu += pcu;
            cell.pcu_seconds += pcu * (exit - enter);
        }
    }

    /// Move the current bin's scratch into sorted sparse rows.
    fn flush(&mut self) {
        self.touched.sort_unstable();
        for &link in &self.touched {
            let l = link as usize;
            self.out.bin.push(self.current);
            self.out.link.push(link);
            self.out.crossings.push(std::mem::take(&mut self.crossings[l]));
            self.out.pcu.push(std::mem::take(&mut self.pcu[l]));
            self.out.pcu_seconds.push(std::mem::take(&mut self.pcu_seconds[l]));
        }
        self.touched.clear();
    }

    /// The finished table.
    #[must_use]
    pub fn finish(mut self) -> LinkBins {
        self.flush();
        self.out
    }

    /// The finished table (by exit time), and, if [`Self::with_entry_bins`] asked
    /// for them, the entry-time tables ([`EntryTables`]), each sorted by bin then
    /// link like the first.
    #[must_use]
    pub fn finish_with_entry(mut self) -> (LinkBins, Option<EntryTables>) {
        self.flush();
        let bin_seconds = self.out.bin_seconds;
        let table = |cells: HashMap<u64, EntryCell>| {
            let mut keys: Vec<u64> = cells.keys().copied().collect();
            keys.sort_unstable();
            let mut table = LinkBins { bin_seconds, ..LinkBins::default() };
            for key in keys {
                let c = cells[&key];
                #[allow(clippy::cast_possible_truncation, reason = "the two halves of the key")]
                let (bin, link) = ((key >> 32) as u32, key as u32);
                table.bin.push(bin);
                table.link.push(link);
                table.crossings.push(c.crossings);
                table.pcu.push(c.pcu);
                table.pcu_seconds.push(c.pcu_seconds);
            }
            table
        };
        let tables = match (self.entry.take(), self.origin_wait.take()) {
            (Some(entry), Some(waits)) => {
                Some(EntryTables { entry: table(entry), origin_wait: table(waits) })
            }
            _ => None,
        };
        (self.out, tables)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(i: usize) -> LinkId {
        LinkId::from_index(i)
    }

    #[test]
    fn traversals_are_filed_by_exit_bin_and_sorted() {
        let mut r = LinkBinRecorder::new(5, 60, 600.0);
        r.record(id(3), 0.0, 10.0, 1.0);
        r.record(id(1), 5.0, 20.0, 2.0);
        r.record(id(3), 30.0, 50.0, 1.0);
        r.record(id(1), 50.0, 70.0, 1.0); // exits in bin 1
        let t = r.finish();
        assert_eq!(t.bins(), &[0, 0, 1]);
        assert_eq!(t.links(), &[1, 3, 1], "sorted by bin, then link");
        assert_eq!(t.crossings(), &[1, 2, 1]);
        assert_eq!(t.pcu(), &[2.0, 2.0, 1.0]);
        // link 3, bin 0: 1 PCU for 10 s and 1 PCU for 20 s.
        assert!((t.mean_seconds(1) - 15.0).abs() < 1e-12);
        assert!((t.mean_seconds(0) - 15.0).abs() < 1e-12, "link 1 bin 0: 2 PCU for 15 s");
    }

    #[test]
    fn empty_bins_cost_nothing_and_the_window_end_is_respected() {
        let mut r = LinkBinRecorder::new(2, 10, 100.0);
        r.record(id(0), 0.0, 5.0, 1.0);
        r.record(id(0), 80.0, 95.0, 1.0); // bin 9; bins 1..8 are empty
        r.record(id(0), 90.0, 100.0, 1.0); // at the window end: ignored
        r.record(id(1), 95.0, 120.0, 1.0); // after it: ignored
        let t = r.finish();
        assert_eq!(t.bins(), &[0, 9]);
        assert_eq!(t.len(), 2);
    }

    #[test]
    fn a_row_is_twenty_eight_bytes() {
        let mut r = LinkBinRecorder::new(1, 1, 10.0);
        r.record(id(0), 0.0, 1.0, 1.0);
        assert_eq!(r.finish().bytes(), 28);
    }
}
