//! Places, legs and chains: one representation for every door-to-door alternative.
//!
//! A **place** is where a leg begins or ends: a node of the network the leg's mode travels on (its
//! **layer**: the road network for a car, the bike layer for a bike, the walk layer for walking)
//! that a trip end was snapped to (a transfer point would be a further kind of place).
//!
//! A **leg** is one mode between two places, departing at a time, with its
//! [attributes](crate::Attributes) and the links it uses, numbered as its layer numbers them. A
//! **chain** is a traveller's door-to-door alternative: a sequence of legs. A single-mode trip is
//! a chain of one leg.
//!
//! A **transit leg** runs between the trip's ends snapped to the walk layer: the walk to the first
//! stop, the [rides](Ride) with the waits and transfer walks between them, and the walk from the
//! last stop, all on the timetable (see [`crate::transit`]); it uses no link of any layer.
//!
//! **Which legs are loaded.** Only a car leg that uses a road link puts a vehicle on the road
//! network ([`Leg::is_loaded`]); a bike or walking leg is timed on its layer at the layer's
//! static speeds and loads nothing (see [`crate::planner`], *static planners*).
//!
//! **Fixed times.** A leg in a vehicle of the traveller's own (a car, a bike) carries
//! [`FixedTimes`]: the seconds before the vehicle sets off (reaching it: its **access time**)
//! and after it arrives (parking and leaving it: its **egress time**). The leg departs at its
//! departure, the vehicle enters the road network at the departure plus the access time
//! ([`Leg::vehicle_departure`]), and the leg ends at the vehicle's arrival plus the egress
//! time. In the leg's attributes the access time is walking time and the egress time parking
//! time, each weighed by the class's coefficient of its kind
//! ([`crate::Coefficients::walk_per_minute`], [`crate::Coefficients::parking_per_minute`]). The
//! mechanism is one for every such vehicle; each mode's planner has its own times. A leg that
//! uses no road link (both ends on one node) leaves no vehicle and has none.
//!
//! **Identity.** A chain carries an identity, a 64-bit FNV-1a hash of the mode and the link
//! sequence of each of its legs (and, for a transit leg, the line, boarding stop and alighting stop
//! of each ride, not the run, as the other engine keys its itineraries), so the same alternative found in two assignment iterations is
//! recognised as the same object (a choice set never holds it twice). Departure times and
//! attributes are not part of the identity: they change from one iteration to the next as the
//! link travel times change, while the alternative stays the same. Two chains are the same
//! alternative when their identities are equal *and* their modes and links are equal, so a hash
//! collision can never merge two different chains.

use openmobisim_core_demand::Mode;
use openmobisim_core_types::hash::Fnv1a;
use openmobisim_core_types::ids::{EntityId, LinkId, NodeId, TransitRunId};

use crate::attributes::Attributes;

/// Where a leg begins or ends: a node of one layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Place {
    /// A node of the road network (a trip end snapped to the nearest street node).
    Road(NodeId),
    /// A node of the bike layer (a trip end snapped to the layer's nearest node).
    Bike(NodeId),
    /// A node of the walk layer (a trip end snapped to the layer's nearest node).
    Walk(NodeId),
}

impl Place {
    /// The node, on the place's own layer.
    #[must_use]
    pub fn node(self) -> NodeId {
        match self {
            Place::Road(n) | Place::Bike(n) | Place::Walk(n) => n,
        }
    }
}

/// The fixed seconds of a leg in a vehicle of the traveller's own: before the vehicle sets off
/// and after it arrives (see the [module docs](self)).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FixedTimes {
    /// Seconds from the leg's departure until the vehicle sets off (reaching the vehicle).
    pub access_seconds: f64,
    /// Seconds from the vehicle's arrival until the leg ends (parking and leaving it).
    pub egress_seconds: f64,
}

impl FixedTimes {
    /// No fixed time before or after the vehicle.
    pub const NONE: Self = Self { access_seconds: 0.0, egress_seconds: 0.0 };

    /// Fixed times of `access_seconds` and `egress_seconds`.
    #[must_use]
    pub const fn new(access_seconds: f64, egress_seconds: f64) -> Self {
        Self { access_seconds, egress_seconds }
    }

    /// Both times together.
    #[must_use]
    pub fn total_seconds(&self) -> f64 {
        self.access_seconds + self.egress_seconds
    }

    /// Whether both times are finite numbers of at least 0.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        [self.access_seconds, self.egress_seconds].iter().all(|s| s.is_finite() && *s >= 0.0)
    }

    /// `attributes` with the access time added as walking time and the egress time as parking
    /// time: what the times add to a leg's attributes.
    #[must_use]
    pub fn added_to(&self, attributes: Attributes) -> Attributes {
        Attributes {
            walk_seconds: attributes.walk_seconds + self.access_seconds,
            parking_seconds: attributes.parking_seconds + self.egress_seconds,
            ..attributes
        }
    }
}

/// One ride of a transit leg: a run of a line, from the call where it is boarded to the call where it
/// is left, at the timetable's times. Stops, lines, runs and calls are the timetable's ids.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ride {
    /// The line (the timetable's route).
    pub line: u32,
    /// The run (one vehicle's trip on the service day).
    pub run: TransitRunId,
    /// The stop boarded at.
    pub board_stop: NodeId,
    /// The stop left at.
    pub alight_stop: NodeId,
    /// The call boarded at (an index into the timetable's calls).
    pub board_call: u32,
    /// The call left at.
    pub alight_call: u32,
    /// When the run leaves the boarding stop, in seconds after midnight.
    pub departure: u32,
    /// When it reaches the alighting stop.
    pub arrival: u32,
}

/// One mode between two places, departing at a time.
#[derive(Clone, Debug, PartialEq)]
pub struct Leg {
    /// The mode of the leg.
    pub mode: Mode,
    /// Where the leg begins.
    pub from: Place,
    /// Where the leg ends.
    pub to: Place,
    /// When the leg departs, in seconds after midnight.
    pub departure: f64,
    /// What the leg is made of, at the link travel times it was last priced on (its fixed
    /// times included).
    pub attributes: Attributes,
    /// The fixed times before and after the leg's vehicle ([`FixedTimes::NONE`] for a leg
    /// without one).
    pub fixed: FixedTimes,
    /// The links the leg uses on its layer (the road network for a car leg, the bike or walk
    /// layer for a bike or walking leg), from its upstream end to its downstream end, in travel
    /// order (empty for a leg whose two places are the same node).
    pub links: Vec<LinkId>,
    /// The rides of a transit leg, in travel order (empty for every other leg).
    pub rides: Vec<Ride>,
}

impl Leg {
    /// Whether the leg puts a vehicle on the road network: a car leg that uses a link. The load
    /// stage loads exactly these legs, in chain order, trip after trip.
    #[must_use]
    pub fn is_loaded(&self) -> bool {
        self.mode == Mode::Car && !self.links.is_empty()
    }

    /// When the leg's vehicle sets off: the departure plus the access time.
    #[must_use]
    pub fn vehicle_departure(&self) -> f64 {
        self.departure + self.fixed.access_seconds
    }
}

/// A door-to-door alternative: a sequence of legs, their summed attributes and an identity.
#[derive(Clone, Debug, PartialEq)]
pub struct Chain {
    legs: Vec<Leg>,
    total: Attributes,
    identity: u64,
}

/// Whether two legs ride the same lines between the same stops (runs and times aside).
fn same_rides(a: &Leg, b: &Leg) -> bool {
    a.rides.len() == b.rides.len()
        && a.rides.iter().zip(&b.rides).all(|(x, y)| {
            (x.line, x.board_stop, x.alight_stop) == (y.line, y.board_stop, y.alight_stop)
        })
}

/// The identity of a sequence of legs: FNV-1a over each leg's mode, link count and links.
fn identity_of(legs: &[Leg]) -> u64 {
    let mut h = Fnv1a::new();
    h.write_u32(u32::try_from(legs.len()).expect("fewer than 2^32 legs"));
    for leg in legs {
        h.write_u8(leg.mode as u8);
        h.write_u32(u32::try_from(leg.links.len()).expect("fewer than 2^32 links"));
        for link in &leg.links {
            h.write_u32(link.raw());
        }
        // Only a transit leg has rides, so every other chain keeps its identity.
        if leg.mode == Mode::Transit {
            h.write_u32(u32::try_from(leg.rides.len()).expect("fewer than 2^32 rides"));
            for r in &leg.rides {
                h.write_u32(r.line);
                h.write_u32(r.board_stop.raw());
                h.write_u32(r.alight_stop.raw());
            }
        }
    }
    h.finish()
}

impl Chain {
    /// A chain of `legs`, in travel order.
    #[must_use]
    pub fn new(legs: Vec<Leg>) -> Self {
        let identity = identity_of(&legs);
        let total = Self::sum(&legs);
        Self { legs, total, identity }
    }

    fn sum(legs: &[Leg]) -> Attributes {
        legs.iter().fold(Attributes::default(), |acc, leg| acc.plus(&leg.attributes))
    }

    /// The legs, in travel order.
    #[must_use]
    pub fn legs(&self) -> &[Leg] {
        &self.legs
    }

    /// The chain's attributes: the sum of its legs'.
    #[must_use]
    pub fn total(&self) -> &Attributes {
        &self.total
    }

    /// The chain's mode: the mode of its first leg (every chain the engine builds so far is of
    /// one mode; a chain of several modes will have a mode of its own). The car for a chain of
    /// no leg.
    #[must_use]
    pub fn mode(&self) -> Mode {
        self.legs.first().map_or(Mode::Car, |l| l.mode)
    }

    /// The chain's identity (see the [module docs](self)).
    #[must_use]
    pub fn identity(&self) -> u64 {
        self.identity
    }

    /// Whether `other` is the same alternative: same identity, same modes, same links.
    #[must_use]
    pub fn same_alternative(&self, other: &Self) -> bool {
        self.identity == other.identity
            && self.legs.len() == other.legs.len()
            && self
                .legs
                .iter()
                .zip(&other.legs)
                .all(|(a, b)| a.mode == b.mode && a.links == b.links && same_rides(a, b))
    }

    /// Whether the chain is one leg of `mode` along `links`: [`Self::same_alternative`] as the
    /// chain that leg would make, without making it.
    #[must_use]
    pub fn is_single_leg(&self, mode: Mode, links: &[LinkId]) -> bool {
        matches!(self.legs.as_slice(), [leg] if leg.mode == mode && leg.links == links)
    }

    /// Whether the chain takes time to travel: a leg of it uses a link or rides a vehicle of the
    /// timetable (a chain of neither arrives as it sets out).
    #[must_use]
    pub fn moves(&self) -> bool {
        self.legs.iter().any(|l| !l.links.is_empty() || !l.rides.is_empty())
    }

    /// Every road link of the chain (the links of its car legs), leg after leg, as raw link
    /// numbers.
    #[must_use]
    pub fn road_links(&self) -> Vec<u32> {
        self.legs
            .iter()
            .filter(|leg| leg.mode == Mode::Car)
            .flat_map(|leg| leg.links.iter().map(|l| l.raw()))
            .collect()
    }

    /// The legs, for re-pricing in place; the total is brought up to date with
    /// [`Self::refresh_total`].
    pub(crate) fn legs_mut(&mut self) -> &mut [Leg] {
        &mut self.legs
    }

    /// Recompute the total after the legs' attributes changed (the identity cannot change:
    /// modes and links are never edited in place).
    pub(crate) fn refresh_total(&mut self) {
        self.total = Self::sum(&self.legs);
    }
}
