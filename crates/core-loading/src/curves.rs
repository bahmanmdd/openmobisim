//! Cumulative counts per link, kept only as far back as the loading ever
//! reads them (S84, S85; S151, S153).
//!
//! A [`LinkCurves`] records the two cumulative counts of Newell's simplified
//! kinematic wave theory for one link — `N_up`, what has entered it, and
//! `N_dn`, what has left — in PCU, changed **only when a vehicle moves**,
//! never as free-floating flow (S151). A vehicle straddling two links moves
//! part of its PCU at a time (S153), so a link's counts always equal the parts
//! of the vehicles on it.
//!
//! The only backward-looking read the loading needs is `N_dn(t − τ_w)`, the
//! downstream count one backward-wave travel time ago, for the receiving
//! condition. So a link keeps its exits only as far back as its own `τ_w`
//! plus one loading step, and forgets the rest ([`LinkCurves::forget_before`]):
//! memory per link is bounded by its discharge in that window, not by the
//! length of the run.

use std::collections::VecDeque;

use openmobisim_core_types::units::{Duration, Pcu};

/// PCU below which a quantity of room is treated as none: absorbs
/// floating-point noise in the cumulative sums.
pub const ROOM_EPSILON: f64 = 1e-9;

/// One link's cumulative counts and its recent exits.
#[derive(Clone, Debug)]
pub struct LinkCurves {
    cumulative_in: f64,
    cumulative_out: f64,
    /// `N_dn` before the oldest retained exit.
    out_before_retained: f64,
    /// `(exit time, N_dn just after that exit)`, oldest first. Times are
    /// non-decreasing.
    exits: VecDeque<(f64, f64)>,
    last_entry: f64,
    last_exit: f64,
}

impl LinkCurves {
    /// An empty link: nothing has entered or left.
    #[must_use]
    pub fn new() -> Self {
        Self {
            cumulative_in: 0.0,
            cumulative_out: 0.0,
            out_before_retained: 0.0,
            exits: VecDeque::new(),
            last_entry: f64::NEG_INFINITY,
            last_exit: f64::NEG_INFINITY,
        }
    }

    /// `N_up` now.
    #[inline]
    #[must_use]
    pub fn cumulative_in(&self) -> Pcu {
        Pcu(self.cumulative_in)
    }

    /// `N_dn` as committed.
    #[inline]
    #[must_use]
    pub fn cumulative_out(&self) -> Pcu {
        Pcu(self.cumulative_out)
    }

    /// `N_dn(t)`: what had left by time `t`, as a step function of the retained
    /// exits. Exact for any `t` no earlier than the last
    /// [`Self::forget_before`] cut-off.
    #[must_use]
    pub fn out_at(&self, t: Duration) -> Pcu {
        let t = t.get();
        let k = self.exits.partition_point(|&(time, _)| time <= t);
        if k == 0 { Pcu(self.out_before_retained) } else { Pcu(self.exits[k - 1].1) }
    }

    /// The first retained exit strictly after `t`, if any.
    #[must_use]
    pub fn first_exit_after(&self, t: Duration) -> Option<Duration> {
        let t = t.get();
        let k = self.exits.partition_point(|&(time, _)| time <= t);
        self.exits.get(k).map(|&(time, _)| Duration(time))
    }

    /// When the last vehicle's front entered from an upstream link, or `-∞`.
    #[inline]
    #[must_use]
    pub fn last_entry(&self) -> Duration {
        Duration(self.last_entry)
    }

    /// When the last vehicle's front passed the downstream end, or `-∞`.
    #[inline]
    #[must_use]
    pub fn last_exit(&self) -> Duration {
        Duration(self.last_exit)
    }

    /// `pcu` moved onto the link.
    pub fn record_in(&mut self, pcu: Pcu) {
        self.cumulative_in += pcu.get();
    }

    /// `pcu` left the link at `t`. Exits are recorded in time order.
    ///
    /// # Panics
    ///
    /// Panics in debug builds if `t` is earlier than the last recorded exit.
    pub fn record_out(&mut self, t: Duration, pcu: Pcu) {
        debug_assert!(
            t.get() >= self.exits.back().map_or(f64::NEG_INFINITY, |e| e.0),
            "exits must be recorded in time order"
        );
        self.cumulative_out += pcu.get();
        self.exits.push_back((t.get(), self.cumulative_out));
    }

    /// A vehicle's front entered from an upstream link at `t`, using the link's
    /// inflow capacity.
    pub fn set_last_entry(&mut self, t: Duration) {
        self.last_entry = t.get();
    }

    /// A vehicle's front passed the downstream end at `t`: the discharge
    /// headway clock moves there.
    pub fn set_last_exit(&mut self, t: Duration) {
        self.last_exit = t.get();
    }

    /// Forget exits strictly before `t`; `N_dn` at those times is kept as a
    /// single number. Every later [`Self::out_at`] read must be at or after `t`.
    pub fn forget_before(&mut self, t: Duration) {
        let t = t.get();
        while let Some(&(time, count)) = self.exits.front() {
            if time >= t {
                break;
            }
            self.out_before_retained = count;
            self.exits.pop_front();
        }
    }

    /// How many exits are retained — the memory this link holds beyond its
    /// fixed fields.
    #[inline]
    #[must_use]
    pub fn retained(&self) -> usize {
        self.exits.len()
    }
}

impl Default for LinkCurves {
    fn default() -> Self {
        Self::new()
    }
}

/// The room a link has at time `t`: its storage, plus what had left it one
/// backward-wave travel time earlier, minus what has entered.
///
/// This is the LTM receiving condition (S84, Yperman's formulation) in vehicle
/// form. **S76's nesting is visible in the formula:** infinite `storage` is the
/// point queue (level 2); a zero `wave_lag` is the spatial queue (level 3).
#[must_use]
pub fn room_at(curves: &LinkCurves, storage: Pcu, wave_lag: Duration, t: Duration) -> Pcu {
    if !storage.is_finite() {
        return Pcu::INFINITE;
    }
    storage + curves.out_at(t - wave_lag) - curves.cumulative_in()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filled(entered: f64, exits: &[(f64, f64)]) -> LinkCurves {
        let mut c = LinkCurves::new();
        c.record_in(Pcu(entered));
        for &(t, p) in exits {
            c.record_out(Duration(t), Pcu(p));
        }
        c
    }

    #[test]
    fn a_fresh_link_is_empty() {
        let c = LinkCurves::new();
        assert_eq!(c.cumulative_in(), Pcu::ZERO);
        assert_eq!(c.cumulative_out(), Pcu::ZERO);
        assert_eq!(c.out_at(Duration(1e9)), Pcu::ZERO);
        assert_eq!(c.first_exit_after(Duration::ZERO), None);
    }

    #[test]
    fn out_at_is_a_step_function_of_exits() {
        let c = filled(3.0, &[(10.0, 1.0), (20.0, 2.0)]);
        assert_eq!(c.out_at(Duration(9.9)), Pcu(0.0));
        assert_eq!(c.out_at(Duration(10.0)), Pcu(1.0));
        assert_eq!(c.out_at(Duration(19.0)), Pcu(1.0));
        assert_eq!(c.out_at(Duration(25.0)), Pcu(3.0));
        assert_eq!(c.first_exit_after(Duration(5.0)), Some(Duration(10.0)));
        assert_eq!(c.first_exit_after(Duration(10.0)), Some(Duration(20.0)));
        assert_eq!(c.first_exit_after(Duration(20.0)), None);
    }

    #[test]
    fn forgetting_keeps_every_later_read_exact() {
        let mut c = filled(5.0, &[(10.0, 1.0), (20.0, 1.0), (30.0, 1.0), (40.0, 1.0)]);
        let reads = [25.0, 30.0, 35.0, 50.0];
        let before: Vec<Pcu> = reads.iter().map(|&t| c.out_at(Duration(t))).collect();
        c.forget_before(Duration(25.0));
        assert_eq!(c.retained(), 2);
        let after: Vec<Pcu> = reads.iter().map(|&t| c.out_at(Duration(t))).collect();
        assert_eq!(before, after);
    }

    /// S76, first exact assertion, at the formula: **infinite storage is the
    /// point queue** — a jammed link with real storage has no room, the same
    /// link with infinite storage always has room.
    #[test]
    fn infinite_storage_reproduces_the_point_queue_exactly() {
        let jammed = filled(1000.0, &[]);
        let at = Duration(100.0);
        assert!(room_at(&jammed, Pcu(50.0), Duration(20.0), at).get() <= 0.0);
        assert_eq!(room_at(&jammed, Pcu::INFINITE, Duration(20.0), at), Pcu::INFINITE);
    }

    /// S76, second exact assertion, at the formula: **zero wave lag is the
    /// spatial queue** — the lagged read misses exactly the PCU that left
    /// during the lag, and nothing else.
    #[test]
    fn zero_wave_lag_reproduces_the_spatial_queue_exactly() {
        let c = filled(20.0, &[(20.0, 20.0)]);
        let storage = Pcu(100.0);
        let lagged = room_at(&c, storage, Duration(20.0), Duration(30.0));
        let instant = room_at(&c, storage, Duration::ZERO, Duration(30.0));
        assert_eq!(lagged, Pcu(80.0));
        assert_eq!(instant, Pcu(100.0));
        assert_eq!(instant - lagged, Pcu(20.0));
    }
}
