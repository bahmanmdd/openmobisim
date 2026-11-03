//! The chain search: the feasible chains of a trip at the current state.
//!
//! For each trip, [`ChainSearch::search`] enumerates the chains a traveller could take from
//! the trip's origin to its destination, prices each leg through the planner of its mode
//! (reading the cost where [`Params::cost_source`] says), keeps the ones [`feasible`] accepts
//! and returns them, best first. The loop adds those not yet in the trip's choice set.
//!
//! **What it enumerates for now:** one chain, a single car leg from the origin's road node to
//! the destination's, departing at the trip's departure time: the route of least cost in utility
//! units at the current link travel times (see [`crate::planner`]). A trip whose mode is given
//! as something other than the car has no chain. Another mode is another planner, and chains of
//! several legs another shape, behind the same function.
//!
//! **Trip ends** are snapped once, when the search is built, to the nearest node with a street
//! (a road a car may enter that is not a motorway or trunk road), unless the point lies within
//! [`Params::motorway_snap_metres`] of a node that has only such roads
//! ([`TripSnapper`]).
//!
//! **Feasibility** is one function, [`feasible`], in one place: every chain the search returns
//! passes it, and nothing else decides whether a chain can be taken.

use openmobisim_core_demand::{Mode, Travellers, Trips};
use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_routes::snap::TripSnapper;
use openmobisim_core_types::ids::{EntityId, TravellerId, TripId};

use crate::assign::Params;
use crate::chain::{Chain, Place};
use crate::planner::{CarPlanner, LegPlanner};
use crate::state::State;

/// The run's chain search: the planners, the demand and every trip's snapped ends.
#[derive(Debug)]
pub struct ChainSearch<'a> {
    travellers: &'a Travellers,
    trips: &'a Trips,
    car: CarPlanner<'a>,
    /// Per trip, its origin and destination as places.
    ends: Vec<(Place, Place)>,
}

impl<'a> ChainSearch<'a> {
    /// The search over `network` for `travellers`' `trips`, with the `car` planner; trip ends
    /// snapped as [`Params::motorway_snap_metres`] says.
    #[must_use]
    pub fn new(
        network: &RoadNetwork,
        travellers: &'a Travellers,
        trips: &'a Trips,
        car: CarPlanner<'a>,
        params: &Params,
    ) -> Self {
        let snapper = TripSnapper::new(network, params.motorway_snap_metres);
        let ends = (0..trips.len())
            .map(|t| {
                let trip = TripId::new(t);
                let at = |p| Place::Road(snapper.nearest(network, p));
                (at(trips.origin(trip)), at(trips.destination(trip)))
            })
            .collect();
        Self { travellers, trips, car, ends }
    }

    /// The car planner.
    #[must_use]
    pub fn car(&self) -> &CarPlanner<'a> {
        &self.car
    }

    /// A trip's origin and destination, as the places its chains run between.
    ///
    /// # Panics
    ///
    /// Panics if `trip` is not one of the search's trips.
    #[must_use]
    pub fn ends(&self, trip: TripId) -> (Place, Place) {
        self.ends[trip.index()]
    }

    /// The planner of `mode`'s legs, if the run has one.
    pub(crate) fn planner(&self, mode: Mode) -> Option<&dyn LegPlanner> {
        match mode {
            Mode::Car => Some(&self.car),
            _ => None,
        }
    }

    /// The feasible chains of `traveller`'s `trip` at `state`, best first (see the
    /// [module docs](self) for what is enumerated).
    ///
    /// # Panics
    ///
    /// Panics if `trip` is not one of the search's trips or `traveller` not one of its
    /// travellers.
    #[must_use]
    pub fn search(
        &self,
        traveller: TravellerId,
        trip: TripId,
        state: &State,
        params: &Params,
    ) -> Vec<Chain> {
        let mode = self.trips.mode(trip);
        if self.trips.mode_given(trip) && mode != Mode::Car {
            return Vec::new();
        }
        let (from, to) = self.ends(trip);
        let departure = f64::from(self.trips.departure(trip).get());
        let class = self.travellers.user_class(traveller);
        self.car
            .plan(from, to, departure, class, state, params.cost_source)
            .map(|leg| Chain::new(vec![leg]))
            .filter(|chain| feasible(state, traveller, chain))
            .into_iter()
            .collect()
    }
}

/// Whether `traveller` can take `chain` at `state`: the one feasibility rule of the run.
///
/// For now: a chain is feasible when it has at least one leg, its legs follow on from one
/// another (each begins where the last ended), every leg is a car leg, and the traveller has a
/// car. Where the car is during the day is not tracked: a traveller with a car may drive it on
/// every trip.
#[must_use]
pub fn feasible(state: &State, traveller: TravellerId, chain: &Chain) -> bool {
    let legs = chain.legs();
    !legs.is_empty()
        && legs.windows(2).all(|w| w[0].to == w[1].from)
        && legs.iter().all(|leg| leg.mode == Mode::Car)
        && state.has_car(traveller)
}
