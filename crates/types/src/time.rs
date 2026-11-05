//! The clock in whole seconds, the grid of time steps over a run's window, and the total order
//! of events.
//!
//! The clock is a `u32` count of seconds from the run's origin (136 years of range). It is never
//! a float: a float clock accumulates representation error and makes ties between simultaneous
//! events depend on rounding.

use core::fmt;
use core::num::NonZeroU32;

use crate::ids::EntityKind;

/// A point on the clock: whole seconds from the run's origin.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[repr(transparent)]
pub struct Second(pub u32);

impl Second {
    /// The origin.
    pub const ZERO: Second = Second(0);
    /// The last representable instant.
    pub const MAX: Second = Second(u32::MAX);

    /// Seconds since the origin.
    #[inline]
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }

    /// `self + delta` seconds, saturating at [`Second::MAX`] rather than wrapping.
    #[inline]
    #[must_use]
    pub const fn saturating_add(self, delta: u32) -> Second {
        Second(self.0.saturating_add(delta))
    }

    /// Seconds from `earlier` to `self`, or zero if `earlier` is later.
    #[inline]
    #[must_use]
    pub const fn saturating_sub(self, earlier: Second) -> u32 {
        self.0.saturating_sub(earlier.0)
    }

    /// The instant `hours` hours after the origin.
    ///
    /// # Panics
    ///
    /// If the result overflows the `u32` clock.
    #[inline]
    #[must_use]
    pub const fn from_hours(hours: u32) -> Second {
        match hours.checked_mul(3600) {
            Some(s) => Second(s),
            None => panic!("hours overflow the u32 clock"),
        }
    }

    /// The instant `minutes` minutes after the origin.
    ///
    /// # Panics
    ///
    /// If the result overflows the `u32` clock.
    #[inline]
    #[must_use]
    pub const fn from_minutes(minutes: u32) -> Second {
        match minutes.checked_mul(60) {
            Some(s) => Second(s),
            None => panic!("minutes overflow the u32 clock"),
        }
    }
}

impl fmt::Debug for Second {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Second({})", self.0)
    }
}

impl fmt::Display for Second {
    /// `HH:MM:SS`, the hours running past 24.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (h, m, s) = (self.0 / 3600, (self.0 % 3600) / 60, self.0 % 60);
        write!(f, "{h:02}:{m:02}:{s:02}")
    }
}

/// An index into a [`StepGrid`].
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

/// Half-open time steps of equal length over a run's window.
///
/// Step `i` covers `[origin + i·step, origin + (i + 1)·step)`. The window is a whole number of
/// steps; its end belongs to no step.
///
/// # Examples
///
/// ```
/// use openmobisim_types::time::{Second, StepGrid, StepIndex};
///
/// // 07:30 to 09:30 in 600-second steps.
/// let grid = StepGrid::new(Second::from_minutes(450), 600, 7_200).unwrap();
/// assert_eq!(grid.n_steps(), 12);
/// assert_eq!(grid.step_of(Second::from_hours(8)), Some(StepIndex(3)));
/// assert_eq!(grid.step_of(grid.end()), None);
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct StepGrid {
    origin: Second,
    step_seconds: NonZeroU32,
    n_steps: u32,
}

/// Why a [`StepGrid`] could not be built.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StepGridError {
    /// The step was zero seconds long.
    ZeroStep,
    /// The window was zero seconds long.
    ZeroWindow,
    /// The window was not a whole number of steps long.
    WindowNotWholeSteps {
        /// The window length offered, in seconds.
        window_seconds: u32,
        /// The step length offered, in seconds.
        step_seconds: u32,
    },
    /// The window would run past the end of the `u32` clock.
    WindowOverflowsClock {
        /// The origin offered, in seconds.
        origin: u32,
        /// The window length offered, in seconds.
        window_seconds: u32,
    },
}

impl fmt::Display for StepGridError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StepGridError::ZeroStep => f.write_str("the step must be longer than zero seconds"),
            StepGridError::ZeroWindow => f.write_str("the window must be longer than zero seconds"),
            StepGridError::WindowNotWholeSteps { window_seconds, step_seconds } => write!(
                f,
                "a window of {window_seconds} s is not a whole number of {step_seconds} s steps"
            ),
            StepGridError::WindowOverflowsClock { origin, window_seconds } => write!(
                f,
                "a window of {window_seconds} s from second {origin} runs past the end of the clock"
            ),
        }
    }
}

impl core::error::Error for StepGridError {}

impl StepGrid {
    /// The grid of `step_seconds`-long steps covering `window_seconds` from `origin`.
    ///
    /// # Errors
    ///
    /// [`StepGridError`] if the step or the window is zero, if the window is not a whole number
    /// of steps (a partial last step is refused, not rounded), or if the window runs past the
    /// end of the clock.
    pub const fn new(
        origin: Second,
        step_seconds: u32,
        window_seconds: u32,
    ) -> Result<Self, StepGridError> {
        let Some(step) = NonZeroU32::new(step_seconds) else {
            return Err(StepGridError::ZeroStep);
        };
        if window_seconds == 0 {
            return Err(StepGridError::ZeroWindow);
        }
        if window_seconds % step_seconds != 0 {
            return Err(StepGridError::WindowNotWholeSteps { window_seconds, step_seconds });
        }
        if origin.0.checked_add(window_seconds).is_none() {
            return Err(StepGridError::WindowOverflowsClock { origin: origin.0, window_seconds });
        }
        Ok(Self { origin, step_seconds: step, n_steps: window_seconds / step_seconds })
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

    /// The step containing `t`, or `None` if `t` is outside the window.
    ///
    /// An instant after the window is not filed in the last step: what happens after the window
    /// is the caller's to count (as unfinished, for a trip still travelling).
    #[inline]
    #[must_use]
    pub const fn step_of(self, t: Second) -> Option<StepIndex> {
        if self.contains(t) {
            Some(StepIndex((t.0 - self.origin.0) / self.step_seconds.get()))
        } else {
            None
        }
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

    /// Every step of the window, in order.
    pub fn steps(self) -> impl Iterator<Item = StepIndex> {
        (0..self.n_steps).map(StepIndex)
    }
}

/// The key that totally orders events: lexicographic on `(second, kind, entity)`.
///
/// Simultaneous events therefore resolve the same way on every run. `BinaryHeap` is a max-heap,
/// so a queue pops the earliest event by storing [`core::cmp::Reverse<EventKey>`].
///
/// # Examples
///
/// ```
/// use core::cmp::Reverse;
/// use std::collections::BinaryHeap;
/// use openmobisim_types::ids::EntityKind;
/// use openmobisim_types::time::{EventKey, Second};
///
/// let mut q = BinaryHeap::new();
/// q.push(Reverse(EventKey::new(Second(20), EntityKind::Trip, 1)));
/// q.push(Reverse(EventKey::new(Second(10), EntityKind::Vehicle, 9)));
/// q.push(Reverse(EventKey::new(Second(10), EntityKind::Trip, 3)));
///
/// // Earliest first; at equal seconds, the lower entity kind first.
/// assert_eq!(q.pop().unwrap().0.kind, EntityKind::Trip);
/// assert_eq!(q.pop().unwrap().0.kind, EntityKind::Vehicle);
/// assert_eq!(q.pop().unwrap().0.second, Second(20));
/// ```
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct EventKey {
    /// When the event fires.
    pub second: Second,
    /// The kind of entity the event belongs to: the first tie-break.
    pub kind: EntityKind,
    /// The entity's raw identifier: the second tie-break.
    pub entity: u32,
}

impl EventKey {
    /// The key of an event of entity `entity` of kind `kind` at `second`.
    #[inline]
    #[must_use]
    pub const fn new(second: Second, kind: EntityKind, entity: u32) -> Self {
        Self { second, kind, entity }
    }
}
