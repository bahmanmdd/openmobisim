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
