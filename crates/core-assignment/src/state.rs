//! The state the assignment carries from one iteration to the next.
//!
//! The state is a **value**: the loop takes it, transforms it stage by stage and returns it;
//! nothing else holds anything between assignment iterations. It holds:
//!
//! - the **link travel times by time of day** of the car layer ([`LinkTimes`]): free-flow
//!   times before the first loading, then the times the last loading measured;
//! - per trip, its **choice set** (the chains found for it so far, in the order they were found),
//!   the **current chain** (the one the traveller takes now, an index into the set) and the
//!   **probabilities of the last choice** (what the choice model gave each chain of the set
//!   when the trip last faced a choice);
//! - per traveller, the **vehicles** they can use: for now, whether they have a car (from their
//!   class's ownership). Where each vehicle is during the day is not tracked;
//! - the number of **assignment iterations completed** on the way to it, which sets the
//!   averaging rule's step: a run continued from a state (`Inputs::initial`) goes on with the
//!   next iteration's step, not with the first one's.
//!
//! A state can be rebuilt from what a run wrote ([`crate::restore`]): every trip's set (each
//! chain's links), current chain and last probabilities, and the count of iterations; its link
//! travel times are those the loading of the current chains measures, which is how the run that
//! wrote them made its own.
//!
//! **Sets only grow.** A chain, once in a set, keeps its position, so its index is a stable
//! identity across iterations: the choice model's random draws are addressed by it, and a
//! traveller who keeps a chain keeps the same draw.
//!
//! **Choice situations are trips.** Each trip of a traveller is its own choice situation with
//! its own set; the averaging rule decides per traveller (all of a traveller's trips re-choose
//! together or none does).

use openmobisim_core_demand::{Travellers, Trips};
use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_loading::LinkTimes;
use openmobisim_core_types::ids::{EntityId, TravellerId, TripId};

use crate::chain::Chain;

/// One trip's choice set, current chain and last choice probabilities.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TripState {
    /// The chains found for the trip, in the order they were found.
    pub set: Vec<Chain>,
    /// The chain the traveller takes now, as an index into [`Self::set`]; `None` before the
    /// trip's first choice (or if no chain was ever found for it).
    pub current: Option<usize>,
    /// The probability the choice model gave each chain of the set at the trip's last choice,
    /// one per chain of the set at that time (empty before the first choice).
    pub probabilities: Vec<f64>,
}

impl TripState {
    /// The chain the traveller takes now.
    #[must_use]
    pub fn current_chain(&self) -> Option<&Chain> {
        self.current.map(|c| &self.set[c])
    }

    /// Add `chain` to the set if no chain of the set is the same alternative; whether it was
    /// added.
    pub fn add(&mut self, chain: Chain) -> bool {
        if self.set.iter().any(|c| c.same_alternative(&chain)) {
            return false;
        }
        self.set.push(chain);
        true
    }
}

/// The assignment's state: link travel times, and per trip its set, current chain and last
/// choice probabilities. See the [module docs](self).
#[derive(Clone, Debug)]
pub struct State {
    link_times: LinkTimes,
    /// How many links the network has (what the link times are compared over).
    links: u32,
    trips: Vec<TripState>,
    has_car: Vec<bool>,
    iterations: u32,
}

impl State {
    /// The state before the first iteration: free-flow link times, every set empty, and each
    /// traveller's car as their ownership says.
    #[must_use]
    pub fn initial(network: &RoadNetwork, travellers: &Travellers, trips: &Trips) -> Self {
        Self {
            link_times: LinkTimes::free_flow(network),
            links: network.link_count(),
            trips: vec![TripState::default(); trips.len() as usize],
            has_car: (0..travellers.len())
                .map(|t| travellers.ownership(TravellerId::new(t)).car)
                .collect(),
            iterations: 0,
        }
    }

    /// The link travel times of the car layer.
    #[must_use]
    pub fn link_times(&self) -> &LinkTimes {
        &self.link_times
    }

    /// Replace the link travel times (what the measure stage does after a loading).
    pub fn set_link_times(&mut self, link_times: LinkTimes) {
        self.link_times = link_times;
    }

    /// One trip's set, current chain and last choice probabilities.
    ///
    /// # Panics
    ///
    /// Panics if `trip` is not a trip of the demand the state was made for.
    #[must_use]
    pub fn trip(&self, trip: TripId) -> &TripState {
        &self.trips[trip.index()]
    }

    /// One trip's entry, to change (to give a trip a set before the first iteration, say).
    ///
    /// # Panics
    ///
    /// Panics if `trip` is not a trip of the demand the state was made for.
    pub fn trip_mut(&mut self, trip: TripId) -> &mut TripState {
        &mut self.trips[trip.index()]
    }

    /// Every trip's entry, by trip.
    #[must_use]
    pub fn trips(&self) -> &[TripState] {
        &self.trips
    }

    pub(crate) fn trips_mut(&mut self) -> &mut Vec<TripState> {
        &mut self.trips
    }

    /// How many assignment iterations were completed on the way to this state (0 for
    /// [`Self::initial`]); a run started from it numbers its first iteration so.
    #[must_use]
    pub fn iterations(&self) -> u32 {
        self.iterations
    }

    /// Count one more completed iteration.
    pub(crate) fn complete_iteration(&mut self) {
        self.iterations += 1;
    }

    /// Set the count of completed iterations (a state rebuilt from what a run wrote).
    pub(crate) fn set_iterations(&mut self, iterations: u32) {
        self.iterations = iterations;
    }

    /// Whether `traveller` has a car to drive.
    ///
    /// # Panics
    ///
    /// Panics if `traveller` is not one of the demand the state was made for.
    #[must_use]
    pub fn has_car(&self, traveller: TravellerId) -> bool {
        self.has_car[traveller.index()]
    }
}

/// Whether two tables of link times over `links` links give every link the same time: the
/// same bin length, the same recorded rows, and the same time where no row applies (read far
/// past any recorded bin, where a link takes its free-flow time).
fn same_link_times(a: &LinkTimes, b: &LinkTimes, links: u32) -> bool {
    let free_flow = |t: &LinkTimes| -> Vec<u64> {
        (0..links).map(|l| t.link_seconds(l, f64::MAX).to_bits()).collect()
    };
    let rows = |t: &LinkTimes| -> Vec<(u32, u32, u64)> {
        t.rows().into_iter().map(|(l, k, s)| (l, k, s.to_bits())).collect()
    };
    a.bin_seconds().to_bits() == b.bin_seconds().to_bits()
        && rows(a) == rows(b)
        && free_flow(a) == free_flow(b)
}

impl PartialEq for State {
    /// Equal sets, current chains, probabilities and vehicles, and link times equal to the bit;
    /// the count of iterations completed is not compared (two states are the same assignment
    /// however many iterations led to them).
    fn eq(&self, other: &Self) -> bool {
        self.links == other.links
            && self.trips == other.trips
            && self.has_car == other.has_car
            && same_link_times(&self.link_times, &other.link_times, self.links)
    }
}
