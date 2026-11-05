//! Which modes a trip's chains may have: a mode stated by the trip, or a choice among modes.
//!
//! **A trip's modes** ([`TripModes`]). A trip whose mode is stated keeps it: its chains are of
//! that mode, whatever their length. A trip whose mode is not stated takes the modes **mode
//! choice** offers its user class ([`ClassModes`], one per class in
//! [`crate::Inputs::mode_choice`]): with two or more of the car, the bike and walking it
//! **chooses its mode**, and its choice set holds a chain of each offered mode that can serve
//! it; with one, it is a trip stated of that mode (one mode is no choice); with none offered
//! (no mode choice in the run), it is a car trip.
//!
//! **Mode choice is a choice set whose chains have different modes**, nothing else: the chooser
//! sees one set per trip, its car chains (the chain search adds one per iteration, as for any
//! car trip) beside its bike chain (one per route of its pair with sampled bike route sets,
//! [`crate::BikeRoutes`]) and its walking chain (each planned once, at iteration 0, on
//! its layer at its static speeds), every chain priced in the one vocabulary of attributes and
//! weighed by the class's coefficients, the mode's constant among them
//! ([`crate::Coefficients::chain_cost`]). Under the nested logit each mode is a nest
//! ([`crate::chooser`]).
//!
//! **A mode that cannot serve a trip choosing its mode is left out of its set**, and counted
//! ([`crate::left_out`]): the car or the bike when the traveller has none (from their class's or
//! their own ownership); the car when no route on the road network reaches the destination; the
//! bike or walking when no route on its layer does, or when the ride or the walk would take
//! longer than the class's limit ([`ClassModes::bike_max_seconds`],
//! [`ClassModes::walk_max_seconds`], compared with the leg's seconds on its layer, its fixed
//! times not counted). A trip whose every offered mode is left out has no chain and does not
//! travel.

use openmobisim_core_demand::{Mode, Travellers, Trips};
use openmobisim_core_types::ids::{EntityId, TripId};

/// A set of the modes a chain of the one-loop engine can have: the car, the bike, walking.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct ModeSet(u8);

/// The modes a [`ModeSet`] can hold, in the order the chains of a set are added.
pub const CHAIN_MODES: [Mode; 3] = [Mode::Car, Mode::Bike, Mode::Walk];

impl ModeSet {
    /// No mode.
    pub const EMPTY: Self = Self(0);

    fn bit(mode: Mode) -> u8 {
        match mode {
            Mode::Car => 1,
            Mode::Bike => 2,
            Mode::Walk => 4,
            _ => 0,
        }
    }

    /// The set of `modes` (a mode other than the car, the bike and walking is not held).
    #[must_use]
    pub fn of(modes: &[Mode]) -> Self {
        Self(modes.iter().fold(0, |acc, &m| acc | Self::bit(m)))
    }

    /// Whether `mode` is in the set.
    #[must_use]
    pub fn contains(self, mode: Mode) -> bool {
        let b = Self::bit(mode);
        b != 0 && self.0 & b != 0
    }

    /// How many modes the set holds.
    #[must_use]
    pub fn len(self) -> usize {
        self.0.count_ones() as usize
    }

    /// Whether the set holds no mode.
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// The set's modes, in the order of [`CHAIN_MODES`].
    pub fn iter(self) -> impl Iterator<Item = Mode> {
        CHAIN_MODES.into_iter().filter(move |&m| self.contains(m))
    }
}

/// What mode choice offers the trips of one user class that do not state their mode.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClassModes {
    /// The modes offered (two or more: such a trip chooses its mode; one: such a trip is a trip
    /// of that mode, stated, and the limits below do not apply to it; none: a car trip).
    pub modes: ModeSet,
    /// The longest walk offered as a walking alternative, in seconds of walking on the walk
    /// layer (`f64::INFINITY`: every walk).
    pub walk_max_seconds: f64,
    /// The longest ride offered as a bike alternative, in seconds of riding on the bike layer,
    /// the fixed times not counted (`f64::INFINITY`: every ride).
    pub bike_max_seconds: f64,
}

impl ClassModes {
    /// `modes` offered with no limit on the walk or the ride.
    #[must_use]
    pub fn unlimited(modes: &[Mode]) -> Self {
        Self {
            modes: ModeSet::of(modes),
            walk_max_seconds: f64::INFINITY,
            bike_max_seconds: f64::INFINITY,
        }
    }

    /// The longest leg of `mode` offered, in seconds on its layer (no limit for the car).
    #[must_use]
    pub fn max_seconds(&self, mode: Mode) -> f64 {
        match mode {
            Mode::Bike => self.bike_max_seconds,
            Mode::Walk => self.walk_max_seconds,
            _ => f64::INFINITY,
        }
    }
}

/// The modes of one trip's chains (see the [module docs](self)).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TripModes {
    /// One mode, kept whatever the chain's length: the trip's stated mode, or the one mode its
    /// class is offered, or the car. The class's longest walk and ride
    /// ([`ClassModes::walk_max_seconds`], [`ClassModes::bike_max_seconds`]) do not apply to a
    /// stated mode: they limit only the alternatives of a trip choosing its mode.
    Stated(Mode),
    /// The trip chooses its mode among these (two or more), each within its class's limits.
    Choice(ClassModes),
}

impl TripModes {
    /// Whether the trip may have a chain of `mode`.
    #[must_use]
    pub fn allows(&self, mode: Mode) -> bool {
        match self {
            Self::Stated(m) => *m == mode,
            Self::Choice(c) => c.modes.contains(mode),
        }
    }

    /// Whether the trip chooses its mode.
    #[must_use]
    pub fn chooses_mode(&self) -> bool {
        matches!(self, Self::Choice(_))
    }

    /// Whether the trip could change its choice in response to the iterations: a car trip (its
    /// route) or a trip choosing its mode. A trip of a stated bike or walking mode has its chains
    /// planned once on static costs (one chain, or with sampled bike route sets one per route of
    /// its pair): its choice probabilities never change, so any switch it makes is chance alone.
    #[must_use]
    pub fn could_change(&self) -> bool {
        !matches!(self, Self::Stated(Mode::Bike | Mode::Walk))
    }

    /// The longest leg of `mode` offered, in seconds on its layer: no limit for a stated mode.
    #[must_use]
    pub fn max_seconds(&self, mode: Mode) -> f64 {
        match self {
            Self::Stated(_) => f64::INFINITY,
            Self::Choice(c) => c.max_seconds(mode),
        }
    }
}

/// Every trip's modes: its stated mode, else what `classes` offers its user class (see the
/// [module docs](self)); `classes` empty, or shorter than the class's index, offers nothing.
#[must_use]
pub fn trip_modes(
    travellers: &Travellers,
    trips: &Trips,
    classes: &[ClassModes],
) -> Vec<TripModes> {
    (0..trips.len())
        .map(TripId::new)
        .map(|trip| {
            if trips.mode_given(trip) {
                return TripModes::Stated(trips.mode(trip));
            }
            let class = travellers.user_class(trips.traveller(trip)).index();
            match classes.get(class) {
                Some(c) if c.modes.len() >= 2 => TripModes::Choice(*c),
                Some(c) if c.modes.len() == 1 => {
                    TripModes::Stated(c.modes.iter().next().unwrap_or(Mode::Car))
                }
                _ => TripModes::Stated(Mode::Car),
            }
        })
        .collect()
}
