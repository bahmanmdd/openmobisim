//! Places, legs and chains: one representation for every door-to-door alternative.
//!
//! A **place** is where a leg begins or ends: a node of the road network that a trip end was
//! snapped to (a transfer point would be a further kind of place).
//!
//! A **leg** is one mode between two places, departing at a time, with its
//! [attributes](crate::Attributes) and the links it uses. A **chain** is a traveller's
//! door-to-door alternative: a sequence of legs. A single-mode trip is a chain of one leg.
//!
//! **Identity.** A chain carries an identity, a 64-bit FNV-1a hash of the mode and the link
//! sequence of each of its legs, so the same alternative found in two assignment iterations is
//! recognised as the same object (a choice set never holds it twice). Departure times and
//! attributes are not part of the identity: they change from one iteration to the next as the
//! link travel times change, while the alternative stays the same. Two chains are the same
//! alternative when their identities are equal *and* their modes and links are equal, so a hash
//! collision can never merge two different chains.

use openmobisim_core_demand::Mode;
use openmobisim_core_types::hash::Fnv1a;
use openmobisim_core_types::ids::{EntityId, LinkId, NodeId};

use crate::attributes::Attributes;

/// Where a leg begins or ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Place {
    /// A node of the road network (a trip end snapped to the nearest street node).
    Road(NodeId),
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
    /// What the leg is made of, at the link travel times it was last priced on.
    pub attributes: Attributes,
    /// The road links the leg uses, from its upstream end to its downstream end, in driving
    /// order (empty for a leg that does not use the road network, or one whose two places are
    /// the same node).
    pub links: Vec<LinkId>,
}

/// A door-to-door alternative: a sequence of legs, their summed attributes and an identity.
#[derive(Clone, Debug, PartialEq)]
pub struct Chain {
    legs: Vec<Leg>,
    total: Attributes,
    identity: u64,
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
                .all(|(a, b)| a.mode == b.mode && a.links == b.links)
    }

    /// Every road link of the chain, leg after leg, as raw link numbers.
    #[must_use]
    pub fn road_links(&self) -> Vec<u32> {
        self.legs.iter().flat_map(|leg| leg.links.iter().map(|l| l.raw())).collect()
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
