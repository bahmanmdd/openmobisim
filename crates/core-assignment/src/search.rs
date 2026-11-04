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
//!
//! **The search stage** asks for every trip at once ([`ChainSearch::search_all`] answers the
//! same question without the choice sets). The trips are
//! grouped once, when the search is built, by origin node, user class and departure window of
//! [`SHARED_SEARCH_WINDOW_SECONDS`]; each group's car legs come from the car planner's shared
//! searches ([`CarPlanner`], *shared searches*), which give every trip the route its own search
//! would give. The groups are worked on in parallel, in fixed chunks, and each trip's answer
//! depends on nothing but the trip and the state, so the answers are the same for any number of
//! threads. A route already in the trip's choice set is not priced again.
//!
//! [`SHARED_SEARCH_WINDOW_SECONDS`]: crate::planner::SHARED_SEARCH_WINDOW_SECONDS

use openmobisim_core_demand::{Mode, Travellers, Trips};
use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_routes::snap::TripSnapper;
use openmobisim_core_types::ids::{EntityId, LinkId, NodeId, TravellerId, TripId, UserClassId};

use crate::assign::Params;
use crate::chain::{Chain, Place};
use crate::parallel;
use crate::planner::{CarPlanner, LegPlanner, SHARED_SEARCH_WINDOW_SECONDS};
use crate::state::State;

/// The groups of shared searches a thread takes at a time: enough to keep the scratch (one
/// search state per thread) worth making.
const GROUPS_PER_CHUNK: usize = 4;

/// The car trips of one origin node and user class leaving within one departure window, whose
/// routes come from shared searches.
#[derive(Debug)]
struct Group {
    origin: NodeId,
    class: UserClassId,
    /// The trips, by departure, then by trip.
    trips: Vec<TripId>,
}

/// The run's chain search: the planners, the demand and every trip's snapped ends.
#[derive(Debug)]
pub struct ChainSearch<'a> {
    travellers: &'a Travellers,
    trips: &'a Trips,
    car: CarPlanner<'a>,
    /// Per trip, its origin and destination as places.
    ends: Vec<(Place, Place)>,
    /// The trips that may drive, grouped for shared searches, in order of origin, class and
    /// window.
    groups: Vec<Group>,
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
        let ends: Vec<(Place, Place)> = (0..trips.len())
            .map(|t| {
                let trip = TripId::new(t);
                let at = |p| Place::Road(snapper.nearest(network, p));
                (at(trips.origin(trip)), at(trips.destination(trip)))
            })
            .collect();
        // (origin, class, window, departure, trip) of every trip that may drive.
        let mut keyed: Vec<(u32, u32, u32, u32, u32)> = (0..trips.len())
            .map(TripId::new)
            .filter(|&trip| !(trips.mode_given(trip) && trips.mode(trip) != Mode::Car))
            .map(|trip| {
                let (Place::Road(origin), _) = ends[trip.index()];
                let class = travellers.user_class(trips.traveller(trip));
                let departure = trips.departure(trip).get();
                let window = departure / SHARED_SEARCH_WINDOW_SECONDS;
                (origin.raw(), class.raw(), window, departure, trip.raw())
            })
            .collect();
        keyed.sort_unstable();
        let mut groups: Vec<Group> = Vec::new();
        let mut last = None;
        for &(origin, class, window, _, trip) in &keyed {
            match groups.last_mut() {
                Some(group) if last == Some((origin, class, window)) => {
                    group.trips.push(TripId::new(trip));
                }
                _ => {
                    groups.push(Group {
                        origin: NodeId::new(origin),
                        class: UserClassId::new(class),
                        trips: vec![TripId::new(trip)],
                    });
                    last = Some((origin, class, window));
                }
            }
        }
        Self { travellers, trips, car, ends, groups }
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

impl ChainSearch<'_> {
    /// The feasible chains of every trip at `state`, by trip, each best first: for every trip
    /// what [`Self::search`] returns for it, worked out with shared searches (see the
    /// [module docs](self)).
    #[must_use]
    pub fn search_all(&self, state: &State, params: &Params) -> Vec<Vec<Chain>> {
        self.by_groups(state, params, |_, _| true)
            .into_iter()
            .map(Option::into_iter)
            .map(Vec::from_iter)
            .collect()
    }

    /// The search stage's question for every trip at `state`: per trip, the first chain
    /// [`Self::search`] returns that is not yet in the trip's choice set, if any. A route
    /// already in the set is not priced.
    pub(crate) fn fresh_chains(&self, state: &State, params: &Params) -> Vec<Option<Chain>> {
        self.by_groups(state, params, |trip, links| {
            !state.trip(trip).set.iter().any(|c| c.is_single_leg(Mode::Car, links))
        })
    }

    /// Per trip, the car chain along the route its search finds at `state`, if feasible and
    /// `wanted` for that trip and route; one group of shared searches after another, in
    /// parallel.
    fn by_groups<W>(&self, state: &State, params: &Params, wanted: W) -> Vec<Option<Chain>>
    where
        W: Fn(TripId, &[LinkId]) -> bool + Sync,
    {
        let changes = self.car.changes(state, params.cost_source);
        let found = parallel::map(
            &self.groups,
            GROUPS_PER_CHUNK,
            || self.car.scratch(),
            |scratch, group| {
                let requests: Vec<(f64, NodeId)> = group
                    .trips
                    .iter()
                    .map(|&trip| {
                        let (_, Place::Road(to)) = self.ends(trip);
                        (f64::from(self.trips.departure(trip).get()), to)
                    })
                    .collect();
                let mut routes = Vec::new();
                let answers = self.car.routes_from(
                    group.origin,
                    group.class,
                    &requests,
                    state,
                    params.cost_source,
                    &changes,
                    scratch,
                    &mut routes,
                );
                let mut chains: Vec<(TripId, Chain)> = Vec::new();
                for (&trip, answer) in group.trips.iter().zip(answers) {
                    let Some(r) = answer else { continue };
                    let links = &routes[r];
                    if !wanted(trip, links) {
                        continue;
                    }
                    let (from, to) = self.ends(trip);
                    let departure = f64::from(self.trips.departure(trip).get());
                    let leg = self.car.leg_on(
                        from,
                        to,
                        departure,
                        links.clone(),
                        state,
                        params.cost_source,
                    );
                    let chain = Chain::new(vec![leg]);
                    if feasible(state, self.trips.traveller(trip), &chain) {
                        chains.push((trip, chain));
                    }
                }
                chains
            },
        );
        let mut out: Vec<Option<Chain>> = vec![None; self.trips.len() as usize];
        for (trip, chain) in found.into_iter().flatten() {
            out[trip.index()] = Some(chain);
        }
        out
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
