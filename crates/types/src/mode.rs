//! The four modes of travel: car, bike, walk and transit.
//!
//! The four modes are peers: nothing in this type ranks them. The derived order is the order of
//! declaration, there so that a mode can key a sorted collection; it says nothing about which
//! mode comes first in any other sense.

use core::fmt;

/// A mode of travel.
///
/// The discriminant is stable (car 0, bike 1, walk 2, transit 3): it is written to outputs and
/// read back with [`Mode::from_u8`].
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
#[repr(u8)]
pub enum Mode {
    /// A car, loaded on the point queue.
    Car = 0,
    /// A bike, at its free-flow speed.
    Bike = 1,
    /// A walker, at walking speed.
    Walk = 2,
    /// Scheduled transit, on its timetable.
    Transit = 3,
}

impl Mode {
    /// Every mode, in discriminant order.
    pub const ALL: [Mode; 4] = [Mode::Car, Mode::Bike, Mode::Walk, Mode::Transit];

    /// The stable discriminant.
    #[inline]
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    /// The mode of a stable discriminant, or `None` if no mode has it.
    #[must_use]
    pub const fn from_u8(raw: u8) -> Option<Mode> {
        match raw {
            0 => Some(Mode::Car),
            1 => Some(Mode::Bike),
            2 => Some(Mode::Walk),
            3 => Some(Mode::Transit),
            _ => None,
        }
    }
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Mode::Car => "car",
            Mode::Bike => "bike",
            Mode::Walk => "walk",
            Mode::Transit => "transit",
        })
    }
}

/// A set of modes, as four bits (one per discriminant).
///
/// The modes allowed on a link (cars, bikes, walkers; transit runs on its timetable, not on
/// links), or the modes open to a traveller. Iteration is in discriminant order.
///
/// # Examples
///
/// ```
/// use openmobisim_types::mode::{Mode, ModeSet};
///
/// let cycle_street = ModeSet::of(&[Mode::Bike, Mode::Walk]);
/// assert!(cycle_street.contains(Mode::Bike));
/// assert!(!cycle_street.contains(Mode::Car));
/// assert_eq!(cycle_street.iter().collect::<Vec<_>>(), [Mode::Bike, Mode::Walk]);
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct ModeSet(u8);

impl ModeSet {
    /// The empty set.
    pub const EMPTY: ModeSet = ModeSet(0);

    /// The set of the modes listed (a mode listed twice is in it once).
    #[must_use]
    pub const fn of(modes: &[Mode]) -> ModeSet {
        let mut set = ModeSet::EMPTY;
        let mut i = 0;
        while i < modes.len() {
            set = set.with(modes[i]);
            i += 1;
        }
        set
    }

    /// This set with `mode` added.
    #[inline]
    #[must_use]
    pub const fn with(self, mode: Mode) -> ModeSet {
        ModeSet(self.0 | (1 << mode.as_u8()))
    }

    /// Whether `mode` is in the set.
    #[inline]
    #[must_use]
    pub const fn contains(self, mode: Mode) -> bool {
        self.0 & (1 << mode.as_u8()) != 0
    }

    /// Whether the set is empty.
    #[inline]
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// The modes in the set, in discriminant order.
    pub fn iter(self) -> impl Iterator<Item = Mode> {
        Mode::ALL.into_iter().filter(move |&m| self.contains(m))
    }
}

impl fmt::Debug for ModeSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_set().entries(self.iter()).finish()
    }
}
