//! The master clock, the loading step grid and the event-queue total order
//! (Foundations §2).
//!
//! # One clock, integer seconds
//!
//! The master clock is a **`u32` count of seconds** from the scenario origin
//! (`period.start` of day one). Never a float clock: a float clock accumulates
//! representation error, makes equality tests unsafe, and makes tie-breaking
//! between simultaneous events depend on rounding. `u32` seconds covers 136
//! years, which is enough.
//!
//! # Two time resolutions, deliberately
//!
//! * **Events** happen at a [`Second`].
//! * **Loading** runs on a coarse [`StepGrid`], `step_seconds` (default 300).
//!
//! The iterative link transmission model has no stability limit on the step,
//! so the step is a fidelity dial rather than a numerical constraint. Vehicle
//! link-exit times come from the cumulative curves by linear interpolation
//! *within* a step and are floored to whole seconds — deterministic, and fine
//! enough for transfers and boarding.

use core::fmt;
use core::num::NonZeroU32;

use crate::ids::EntityKind;

/// A point on the master clock: whole seconds from the scenario origin.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[repr(transparent)]
pub struct Second(pub u32);

impl Second {
    /// The scenario origin.
    pub const ZERO: Second = Second(0);
    /// The largest representable instant.
    pub const MAX: Second = Second(u32::MAX);

    /// Seconds since the scenario origin.
    #[inline]
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }

    /// `self + delta`, saturating at [`Second::MAX`] rather than wrapping.
    ///
    /// Saturating is the right behaviour here: a simulation that would
    /// schedule past the end of the clock must still complete (brief §3c), and
    /// the window bound will discard the event anyway.
    #[inline]
    #[must_use]
    pub const fn saturating_add(self, delta: u32) -> Second {
        Second(self.0.saturating_add(delta))
    }

    /// `self - earlier`, or zero if `earlier` is later than `self`.
    #[inline]
    #[must_use]
    pub const fn saturating_sub(self, earlier: Second) -> u32 {
        self.0.saturating_sub(earlier.0)
    }

    /// Build from an hour-of-day, for readable scenario fixtures.
    #[inline]
    #[must_use]
    pub const fn from_hours(hours: u32) -> Second {
        Second(hours * 3600)
    }

    /// Build from a minute-of-day.
    #[inline]
    #[must_use]
    pub const fn from_minutes(minutes: u32) -> Second {
        Second(minutes * 60)
    }
}

impl fmt::Debug for Second {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Second({})", self.0)
    }
}

impl fmt::Display for Second {
    /// `HH:MM:SS`, with hours running past 24 for multi-day scenarios.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (h, m, s) = (self.0 / 3600, (self.0 % 3600) / 60, self.0 % 60);
        write!(f, "{h:02}:{m:02}:{s:02}")
    }
}

/// An index into the loading step grid.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
#[repr(transparent)]
pub struct StepIndex(pub u32);

impl StepIndex {
    /// The first step.
    pub const ZERO: StepIndex = StepIndex(0);

    /// The raw index.
    #[inline]
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }

    /// The raw index as an array subscript.
    #[inline]
    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

impl fmt::Display for StepIndex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Division of a `u32` by a value fixed at build time, without a `div`
/// instruction.
///
/// `second / step_seconds` is evaluated for every vehicle, every event and
/// every curve lookup, and `step_seconds` is a runtime value the compiler
/// cannot turn into a shift. Hardware integer division is an order of
/// magnitude slower than a multiply, so this precomputes the reciprocal once
/// and spends a 64×64→128 multiply-high instead.
///
/// The method is Lemire's: for divisor `d`, `M = floor(2^64 / d) + 1` gives
/// `floor(n / d) = (M · n) >> 64` for every `n < 2^32`. `d == 1` is the one
/// case `M` cannot represent in 64 bits, and is branched around; the branch is
/// perfectly predicted because `d` never changes.
///
/// Correctness against `/` and `%` is asserted by a property test over the
/// whole `u32` input range.
///
/// # Examples
///
/// ```
/// use openmobisim_core_types::time::FastDivU32;
///
/// let d = FastDivU32::new(300);
/// assert_eq!(d.div(1_234), 1_234 / 300);
/// assert_eq!(d.rem(1_234), 1_234 % 300);
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FastDivU32 {
    magic: u64,
    divisor: u32,
}

impl FastDivU32 {
    /// Precompute the reciprocal of `divisor`.
    ///
    /// # Panics
    ///
    /// Panics if `divisor` is zero.
    #[must_use]
    pub const fn new(divisor: u32) -> Self {
        assert!(divisor != 0, "FastDivU32 divisor must be non-zero");
        // For divisor == 1 the magic is unrepresentable; `div` branches on it.
        let magic = if divisor == 1 { 0 } else { (u64::MAX / divisor as u64) + 1 };
        Self { magic, divisor }
    }

    /// The divisor this was built for.
    #[inline]
    #[must_use]
    pub const fn divisor(self) -> u32 {
        self.divisor
    }

    /// `n / divisor`.
    #[inline]
    #[must_use]
    pub const fn div(self, n: u32) -> u32 {
        if self.divisor == 1 {
            return n;
        }
        #[allow(
            clippy::cast_possible_truncation,
            reason = "Lemire's bound: the high 64 bits of magic * n are below 2^32 for n < 2^32"
        )]
        let q = ((self.magic as u128 * n as u128) >> 64) as u32;
        q
    }

    /// `n % divisor`.
    #[inline]
    #[must_use]
    pub const fn rem(self, n: u32) -> u32 {
        n - self.div(n) * self.divisor
    }

    /// `(n / divisor, n % divisor)` — one multiply for both.
    #[inline]
    #[must_use]
    pub const fn div_rem(self, n: u32) -> (u32, u32) {
        let q = self.div(n);
        (q, n - q * self.divisor)
    }
}

/// The coarse time grid the network loading runs on.
///
/// Steps are half-open: step `i` covers `[origin + i·step, origin + (i+1)·step)`.
/// The grid is built once per run and shared; it carries a precomputed
/// reciprocal because [`step_of`](StepGrid::step_of) is called constantly.
///
/// # Examples
///
/// ```
/// use openmobisim_core_types::time::{Second, StepGrid, StepIndex};
///
/// // A four-hour window from 07:00, at the default 300-second step.
/// let grid = StepGrid::new(Second::from_hours(7), 300, 4 * 3600).unwrap();
///
/// assert_eq!(grid.n_steps(), 48);
/// assert_eq!(grid.step_of(Second::from_hours(7)), StepIndex(0));
/// assert_eq!(grid.step_of(Second::from_hours(8)), StepIndex(12));
/// assert_eq!(grid.step_start(StepIndex(12)), Second::from_hours(8));
///
/// // Halfway through step 12.
/// let t = Second(Second::from_hours(8).get() + 150);
/// assert!((grid.fraction_into_step(t) - 0.5).abs() < 1e-12);
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct StepGrid {
    origin: Second,
    step_seconds: NonZeroU32,
    n_steps: u32,
    recip: FastDivU32,
}

/// Why a [`StepGrid`] could not be built.
///
/// These are scenario-validation failures, rejected at build time before any
/// run starts — not run-time conditions.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StepGridError {
    /// `step_seconds` was zero.
    ZeroStep,
    /// The window was not a whole number of steps long.
    WindowNotWholeSteps {
        /// The window length that was offered, in seconds.
        window_seconds: u32,
        /// The step length it was offered against.
        step_seconds: u32,
    },
    /// The window would run past the end of the `u32` clock.
    WindowOverflowsClock {
        /// The origin the window was offered from.
        origin: u32,
        /// The window length that was offered, in seconds.
        window_seconds: u32,
    },
}

impl fmt::Display for StepGridError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StepGridError::ZeroStep => f.write_str("step_seconds must be greater than zero"),
            StepGridError::WindowNotWholeSteps { window_seconds, step_seconds } => write!(
                f,
                "window of {window_seconds} s is not a whole number of {step_seconds} s steps"
            ),
            StepGridError::WindowOverflowsClock { origin, window_seconds } => write!(
                f,
                "window of {window_seconds} s from second {origin} runs past the end of the u32 clock"
            ),
        }
    }
}

impl core::error::Error for StepGridError {}

impl StepGrid {
    /// The default loading step, in seconds.
    ///
    /// Chosen in the design, not here; this constant is the single place the
    /// number appears in code.
    pub const DEFAULT_STEP_SECONDS: u32 = 300;

    /// Build a grid covering `window_seconds` from `origin`.
    ///
    /// # Errors
    ///
    /// Returns [`StepGridError`] if the step is zero, if the window is not a
    /// whole number of steps, or if the window would run past the end of the
    /// clock. A partial trailing step is rejected rather than rounded: a
    /// silently shortened or lengthened window would change every rate in the
    /// run, and the scenario file is the place to fix it.
    pub const fn new(
        origin: Second,
        step_seconds: u32,
        window_seconds: u32,
    ) -> Result<Self, StepGridError> {
        let Some(step) = NonZeroU32::new(step_seconds) else {
            return Err(StepGridError::ZeroStep);
        };
        if window_seconds % step_seconds != 0 {
            return Err(StepGridError::WindowNotWholeSteps { window_seconds, step_seconds });
        }
        if origin.0.checked_add(window_seconds).is_none() {
            return Err(StepGridError::WindowOverflowsClock { origin: origin.0, window_seconds });
        }
        Ok(Self {
            origin,
            step_seconds: step,
            n_steps: window_seconds / step_seconds,
            recip: FastDivU32::new(step_seconds),
        })
    }

    /// The instant step zero starts at.
    #[inline]
    #[must_use]
    pub const fn origin(self) -> Second {
        self.origin
    }

    /// The step length, in seconds.
    #[inline]
    #[must_use]
    pub const fn step_seconds(self) -> u32 {
        self.step_seconds.get()
    }

    /// The step length as a float, for rate arithmetic.
    #[inline]
    #[must_use]
    pub fn step_seconds_f64(self) -> f64 {
        f64::from(self.step_seconds.get())
    }

    /// How many steps the window holds.
    #[inline]
    #[must_use]
    pub const fn n_steps(self) -> u32 {
        self.n_steps
    }

    /// The first instant after the window.
    #[inline]
    #[must_use]
    pub const fn end(self) -> Second {
        Second(self.origin.0 + self.n_steps * self.step_seconds.get())
    }

    /// Whether `t` falls inside the window.
    #[inline]
    #[must_use]
    pub const fn contains(self, t: Second) -> bool {
        t.0 >= self.origin.0 && t.0 < self.end().0
    }

    /// The step containing `t`.
    ///
    /// Instants before the origin map to step zero and instants at or after
    /// the end map to the last step. Clamping rather than failing is
    /// deliberate: the simulation never stops (brief §3c), and a traveller who
    /// is still moving at the end of the window is counted as truncated by the
    /// completion statistics, not by a panic here.
    #[inline]
    #[must_use]
    pub const fn step_of(self, t: Second) -> StepIndex {
        if t.0 <= self.origin.0 {
            return StepIndex(0);
        }
        let q = self.recip.div(t.0 - self.origin.0);
        if q >= self.n_steps { StepIndex(self.n_steps - 1) } else { StepIndex(q) }
    }

    /// The step containing `t`, or `None` if `t` is outside the window.
    ///
    /// Use this where falling outside the window is a condition to record
    /// rather than to clamp.
    #[inline]
    #[must_use]
    pub const fn try_step_of(self, t: Second) -> Option<StepIndex> {
        if self.contains(t) { Some(StepIndex(self.recip.div(t.0 - self.origin.0))) } else { None }
    }

    /// The instant step `i` starts at.
    #[inline]
    #[must_use]
    pub const fn step_start(self, i: StepIndex) -> Second {
        Second(self.origin.0 + i.0 * self.step_seconds.get())
    }

    /// The first instant after step `i`.
    #[inline]
    #[must_use]
    pub const fn step_end(self, i: StepIndex) -> Second {
        Second(self.origin.0 + (i.0 + 1) * self.step_seconds.get())
    }

    /// How far into its step `t` sits, in `[0, 1)`.
    ///
    /// This is the interpolation parameter for reading a cumulative curve
    /// between step boundaries.
    #[inline]
    #[must_use]
    pub fn fraction_into_step(self, t: Second) -> f64 {
        let offset = t.0.saturating_sub(self.origin.0);
        f64::from(self.recip.rem(offset)) / self.step_seconds_f64()
    }

    /// Convert a fractional position within step `i` back to a whole second.
    ///
    /// The result is **floored**, which is the rule fixed in Foundations §2:
    /// link-exit times interpolated inside a step become whole seconds, so
    /// that the event queue stays integer and ties break identically.
    ///
    /// `fraction` is clamped to `[0, 1]`, so a caller that computes a
    /// marginally out-of-range value from a curve cannot produce an instant
    /// outside the step.
    #[inline]
    #[must_use]
    pub fn second_within_step(self, i: StepIndex, fraction: f64) -> Second {
        let f = fraction.clamp(0.0, 1.0);
        let offset = (f * self.step_seconds_f64()).floor();
        // `offset` is in [0, step_seconds] by construction, so the cast is exact.
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "offset is clamped to [0, step_seconds] and floored"
        )]
        let offset = offset as u32;
        Second(self.step_start(i).0 + offset.min(self.step_seconds.get() - 1))
    }

    /// Every step index in the window, in order.
    pub fn steps(self) -> impl Iterator<Item = StepIndex> {
        (0..self.n_steps).map(StepIndex)
    }
}

/// The key that gives the event queue its total order.
///
/// Ordering is lexicographic on `(second, kind, entity)` — exactly the tuple
/// fixed in Foundations §2. Simultaneous events therefore resolve identically
/// on every run of the same binary, which is half of what makes a seeded run
/// reproducible. (The other half is [`crate::reduce`].)
///
/// `BinaryHeap` is a max-heap, so a queue pops the *earliest* event by storing
/// [`core::cmp::Reverse<EventKey>`].
///
/// # Examples
///
/// ```
/// use core::cmp::Reverse;
/// use std::collections::BinaryHeap;
/// use openmobisim_core_types::ids::EntityKind;
/// use openmobisim_core_types::time::{EventKey, Second};
///
/// let mut q = BinaryHeap::new();
/// q.push(Reverse(EventKey::new(Second(20), EntityKind::Trip, 1)));
/// q.push(Reverse(EventKey::new(Second(10), EntityKind::Vehicle, 9)));
/// q.push(Reverse(EventKey::new(Second(10), EntityKind::Trip, 3)));
///
/// // Earliest first; at equal seconds, the lower entity kind wins.
/// assert_eq!(q.pop().unwrap().0.kind, EntityKind::Trip);
/// assert_eq!(q.pop().unwrap().0.kind, EntityKind::Vehicle);
/// assert_eq!(q.pop().unwrap().0.second, Second(20));
/// ```
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct EventKey {
    /// When the event fires.
    pub second: Second,
    /// What kind of entity the event belongs to — the first tie-break.
    pub kind: EntityKind,
    /// The entity's raw id — the second tie-break.
    pub entity: u32,
}

impl EventKey {
    /// Build a key.
    #[inline]
    #[must_use]
    pub const fn new(second: Second, kind: EntityKind, entity: u32) -> Self {
        Self { second, kind, entity }
    }
}
