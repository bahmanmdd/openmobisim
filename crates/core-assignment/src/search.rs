//! The chain search: the feasible chains of a trip at the current state.
//!
//! For each trip, [`ChainSearch::search`] enumerates the chains a traveller could take from
//! the trip's origin to its destination, prices each leg through the planner of its mode
//! (reading the cost where [`Params::cost_source`] says), keeps the ones [`feasible`] accepts
//! and returns them, best first. The loop adds those not yet in the trip's choice set.
//!
//! **What it enumerates for now:** one chain of one leg, of the trip's mode. A trip whose mode is
//! not given, or given as the car, gets a single car leg from the origin's road node to the
//! destination's, departing at the trip's departure time, its car setting off the access time
//! later: the route of least cost in utility units at the current link travel times, with the
//! car's fixed times (see [`crate::planner`]). A trip whose mode is given as the bike or walking
//! gets a single leg of that mode on its layer, between its ends snapped to that layer
//! ([`crate::layer_planner`]): its times are static, so it is **planned once**, when the search is
//! given its static planners ([`ChainSearch::with_static`]), and offered to the trip's set at the
//! first search stage; from then on it is in the set and nothing new is found for the trip. A trip
//! whose mode is given as another mode has no chain. Another mode is another planner, and chains
//! of several legs another shape, behind the same function.
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
use crate::layer_planner::StaticPlanner;
use crate::parallel;
use crate::planner::{CarPlanner, LegPlanner, SHARED_SEARCH_WINDOW_SECONDS};
use crate::state::State;

/// The bike and walking trips a thread plans at a time: one search each, a scratch per thread.
const STATIC_TRIPS_PER_CHUNK: usize = 256;

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
    /// The planners of bike and walking legs, if the run has their layers.
    bike: Option<StaticPlanner<'a>>,
    walk: Option<StaticPlanner<'a>>,
    /// Per trip whose mode is given as the bike or walking: its one chain, planned once
    /// ([`Self::with_static`]); `None` for every other trip, and where no route exists.
    fixed_chains: Vec<Option<Chain>>,
}

/// Whether `trip`'s mode is given as one with a static layer (bike or walking).
fn static_mode(trips: &Trips, trip: TripId) -> Option<Mode> {
    let mode = trips.mode(trip);
    (trips.mode_given(trip) && matches!(mode, Mode::Bike | Mode::Walk)).then_some(mode)
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
                let origin = ends[trip.index()].0.node();
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
        Self {
            travellers,
            trips,
            car,
            ends,
            groups,
            bike: None,
            walk: None,
            fixed_chains: vec![None; trips.len() as usize],
        }
    }

    /// The search with the planners of bike and walking legs (each `None` where the run has no
    /// such layer): every trip whose mode is given as the bike or walking is planned now, once,
    /// its ends snapped to its mode's layer, departing at its departure (see the
    /// [module docs](self)). The trips are planned in parallel, each by its own search, so the
    /// legs do not depend on the number of threads.
    #[must_use]
    pub fn with_static(
        mut self,
        bike: Option<StaticPlanner<'a>>,
        walk: Option<StaticPlanner<'a>>,
    ) -> Self {
        self.bike = bike;
        self.walk = walk;
        let wanted: Vec<TripId> = (0..self.trips.len())
            .map(TripId::new)
            .filter(|&t| static_mode(self.trips, t).is_some_and(|m| self.planner(m).is_some()))
            .collect();
        let planned = parallel::map(
            &wanted,
            STATIC_TRIPS_PER_CHUNK,
            Vec::new,
            |searches: &mut Vec<Option<_>>, &trip| self.plan_static(trip, searches),
        );
        for (trip, chain) in wanted.into_iter().zip(planned) {
            self.fixed_chains[trip.index()] = chain;
        }
        self
    }

    /// The static planner of `mode`, if the run has its layer.
    fn static_planner(&self, mode: Mode) -> Option<&StaticPlanner<'a>> {
        match mode {
            Mode::Bike => self.bike.as_ref(),
            Mode::Walk => self.walk.as_ref(),
            _ => None,
        }
    }

    /// The chain of a bike or walking `trip`: one leg of its mode on its layer, by the search of
    /// its class (kept in `searches`, by the planner's mode and search context, for the next
    /// trip the thread plans).
    fn plan_static<'s>(
        &'s self,
        trip: TripId,
        searches: &mut Vec<Option<(Mode, usize, openmobisim_core_routes::search::Search<'s>)>>,
    ) -> Option<Chain> {
        let mode = static_mode(self.trips, trip)?;
        let planner = self.static_planner(mode)?;
        let class = self.travellers.user_class(self.trips.traveller(trip));
        let context = planner.context_of(class)?;
        let (from, to) =
            (planner.place(self.trips.origin(trip)), planner.place(self.trips.destination(trip)));
        let slot = match searches
            .iter()
            .position(|s| s.as_ref().is_some_and(|(m, c, _)| *m == mode && *c == context))
        {
            Some(i) => i,
            None => {
                searches.push(Some((mode, context, planner.search_in(context))));
                searches.len() - 1
            }
        };
        let (_, _, search) = searches[slot].as_mut().expect("just found or made");
        let links = StaticPlanner::route(search, from.node(), to.node())?;
        let departure = f64::from(self.trips.departure(trip).get());
        Some(Chain::new(vec![planner.leg_along(from, to, departure, links)]))
    }

    /// The car planner.
    #[must_use]
    pub fn car(&self) -> &CarPlanner<'a> {
        &self.car
    }

    /// A trip's origin and destination on the road network, as the places its car chains run
    /// between (a bike or walking chain runs between the ends snapped to its own layer).
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
            Mode::Bike => self.bike.as_ref().map(|p| p as &dyn LegPlanner),
            Mode::Walk => self.walk.as_ref().map(|p| p as &dyn LegPlanner),
            _ => None,
        }
    }

    /// The bike or walking planner of `mode`, if the run has its layer.
    #[must_use]
    pub fn layer_planner(&self, mode: Mode) -> Option<&StaticPlanner<'a>> {
        self.static_planner(mode)
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
        if static_mode(self.trips, trip).is_some() {
            return self.fixed_chains[trip.index()]
                .iter()
                .filter(|chain| feasible(state, traveller, chain))
                .cloned()
                .collect();
        }
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
        let mut all = self.by_groups(state, params, |_, _| true);
        self.add_static(state, &mut all, |_, _| true);
        all.into_iter().map(Option::into_iter).map(Vec::from_iter).collect()
    }

    /// The search stage's question for every trip at `state`: per trip, the first chain
    /// [`Self::search`] returns that is not yet in the trip's choice set, if any. A route
    /// already in the set is not priced; a bike or walking trip's chain, planned once, is new
    /// only until it is in the set.
    pub(crate) fn fresh_chains(&self, state: &State, params: &Params) -> Vec<Option<Chain>> {
        let mut fresh = self.by_groups(state, params, |trip, links| {
            !state.trip(trip).set.iter().any(|c| c.is_single_leg(Mode::Car, links))
        });
        self.add_static(state, &mut fresh, |trip, chain| {
            !state.trip(trip).set.iter().any(|c| c.same_alternative(chain))
        });
        fresh
    }

    /// Into `out`, by trip, every bike or walking trip's chain planned once, if feasible at
    /// `state` and `wanted`.
    fn add_static<W>(&self, state: &State, out: &mut [Option<Chain>], wanted: W)
    where
        W: Fn(TripId, &Chain) -> bool,
    {
        for (t, chain) in self.fixed_chains.iter().enumerate() {
            let Some(chain) = chain else { continue };
            let trip = TripId::from_index(t);
            if feasible(state, self.trips.traveller(trip), chain) && wanted(trip, chain) {
                out[t] = Some(chain.clone());
            }
        }
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
                // Each car sets off the access time after its trip's departure: a constant, so
                // the order of departure is the trips'.
                let access = self.car.fixed_times().access_seconds;
                let requests: Vec<(f64, NodeId)> = group
                    .trips
                    .iter()
                    .map(|&trip| {
                        let to = self.ends(trip).1.node();
                        (f64::from(self.trips.departure(trip).get()) + access, to)
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
/// another (each begins where the last ended), every leg is a car, bike or walking leg, the
/// traveller has a car for each car leg and a bike for each bike leg (from their class's or
/// their own ownership); walking needs nothing. Where a vehicle is during the day is not
/// tracked: a traveller with a car (a bike) may drive (ride) it on every trip.
#[must_use]
pub fn feasible(state: &State, traveller: TravellerId, chain: &Chain) -> bool {
    let legs = chain.legs();
    !legs.is_empty()
        && legs.windows(2).all(|w| w[0].to == w[1].from)
        && legs.iter().all(|leg| match leg.mode {
            Mode::Car => state.has_car(traveller),
            Mode::Bike => state.has_bike(traveller),
            Mode::Walk => true,
            _ => false,
        })
}
