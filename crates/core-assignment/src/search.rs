//! The chain search: the feasible chains of a trip at the current state.
//!
//! For each trip, [`ChainSearch::search`] enumerates the chains a traveller could take from
//! the trip's origin to its destination, prices each leg through the planner of its mode
//! (reading the cost where [`Params::cost_source`] says), keeps the ones [`feasible`] accepts
//! and returns them, best first. The loop adds those not yet in the trip's choice set.
//!
//! **What it enumerates for now:** chains of one leg, one per mode the trip may use
//! ([`crate::modes`]: its stated mode, or the modes it chooses among), in the order car, bike,
//! walking. A **car** leg runs from the origin's road node to the destination's, departing at the
//! trip's departure time, its car setting off the access time later: the route of least cost in
//! utility units at the current link travel times, with the car's fixed times (see
//! [`crate::planner`]); it is searched again at every search stage. A **bike** or **walking** leg
//! runs on its layer, between the trip's ends snapped to that layer ([`crate::layer_planner`]):
//! its times are static, so it is **planned once**, when the search is given its static planners
//! ([`ChainSearch::with_static`]), and offered to the trip's set at the first search stage; from
//! then on it is in the set and nothing new is found for it. With sampled bike route sets
//! ([`crate::BikeRoutes::Sampled`]) a trip has one bike chain per route of its pair's set, all
//! planned once and offered together. For a trip choosing its mode, a ride
//! or a walk longer than its class's limit is not planned into its set
//! ([`crate::ClassModes`]). A **transit** trip (its mode stated as transit) has one chain per
//! journey of the timetable kept for it, all planned once, before the first search, and offered
//! together at the first search ([`ChainSearch::with_transit`], [`crate::transit`]). A trip whose
//! mode is given as another mode has no chain. Another mode
//! is another planner, and chains of several legs another shape, behind the same function.
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
use openmobisim_core_routes::{RouteKey, RouteSets};
use openmobisim_core_types::ids::{EntityId, LinkId, NodeId, TravellerId, TripId, UserClassId};

use crate::assign::Params;
use crate::chain::{Chain, Place};
use crate::layer_planner::StaticPlanner;
use crate::modes::{ClassModes, TripModes, trip_modes};
use crate::parallel;
use crate::planner::{CarPlanner, LegPlanner, SHARED_SEARCH_WINDOW_SECONDS};
use crate::state::State;
use crate::transit::TransitPlanner;

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
    /// Per trip, the modes its chains may have.
    modes: Vec<TripModes>,
    /// Per trip, its bike and walking chains (in that order; several bike chains with sampled
    /// route sets), planned once ([`Self::with_static`]); empty for a trip of neither mode, and
    /// where no route exists or a trip choosing its mode would ride or walk longer than its
    /// limit.
    fixed_chains: Vec<Vec<Chain>>,
    /// Per trip choosing its mode, each bike or walking mode it is offered that has no chain
    /// planned, and why.
    unplanned: Vec<Vec<(Mode, Unplanned)>>,
    /// The planner of transit legs, if the run has a timetable.
    transit: Option<TransitPlanner<'a>>,
}

/// Why a bike or walking mode offered to a trip choosing its mode has no chain in its set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Unplanned {
    /// No route on the layer from the trip's origin to its destination.
    NoRoute,
    /// The ride or the walk would take longer than the class's limit.
    TooLong,
}

/// The static modes (bike, walking) whose chains `modes` allows, in that order.
fn static_modes(modes: &TripModes) -> impl Iterator<Item = Mode> + '_ {
    [Mode::Bike, Mode::Walk].into_iter().filter(|&m| modes.allows(m))
}

impl<'a> ChainSearch<'a> {
    /// The search over `network` for `travellers`' `trips`, with the `car` planner; trip ends
    /// snapped as [`Params::motorway_snap_metres`] says; no trip chooses its mode (a trip whose
    /// mode is not given is a car trip).
    #[must_use]
    pub fn new(
        network: &RoadNetwork,
        travellers: &'a Travellers,
        trips: &'a Trips,
        car: CarPlanner<'a>,
        params: &Params,
    ) -> Self {
        Self::with_modes(network, travellers, trips, car, params, &[])
    }

    /// The search of [`Self::new`], with the modes mode choice offers each user class's trips
    /// that state no mode (`classes`, by user class; see [`crate::modes`]).
    #[must_use]
    pub fn with_modes(
        network: &RoadNetwork,
        travellers: &'a Travellers,
        trips: &'a Trips,
        car: CarPlanner<'a>,
        params: &Params,
        classes: &[ClassModes],
    ) -> Self {
        let modes = trip_modes(travellers, trips, classes);
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
            .filter(|&trip| modes[trip.index()].allows(Mode::Car))
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
            fixed_chains: vec![Vec::new(); trips.len() as usize],
            unplanned: vec![Vec::new(); trips.len() as usize],
            modes,
            transit: None,
        }
    }

    /// The modes of `trip`'s chains.
    ///
    /// # Panics
    ///
    /// Panics if `trip` is not one of the search's trips.
    #[must_use]
    pub fn trip_modes(&self, trip: TripId) -> TripModes {
        self.modes[trip.index()]
    }

    /// Every trip's modes, by trip.
    pub(crate) fn modes(&self) -> &[TripModes] {
        &self.modes
    }

    /// Per trip choosing its mode, the bike and walking modes it is offered that have no chain,
    /// and why.
    pub(crate) fn unplanned(&self, trip: TripId) -> &[(Mode, Unplanned)] {
        &self.unplanned[trip.index()]
    }

    /// The search with the planners of bike and walking legs (each `None` where the run has no
    /// such layer): every trip that may ride or walk (its mode given as the bike or walking, or
    /// offered them by mode choice) is planned now, once per mode, its ends snapped to the mode's
    /// layer, departing at its departure (see the [module docs](self)). A bike planner with
    /// sampled route sets ([`crate::BikeRoutes::Sampled`]) makes one set per pair of the trips'
    /// snapped ends first, and each route of its trip's pair becomes one of its bike chains, in
    /// the set's order; a trip choosing its mode keeps those within its class's longest ride.
    /// The trips are planned in parallel, each by its own search, so the legs do not depend on
    /// the number of threads.
    #[must_use]
    pub fn with_static(
        mut self,
        bike: Option<StaticPlanner<'a>>,
        walk: Option<StaticPlanner<'a>>,
    ) -> Self {
        self.bike = bike;
        self.walk = walk;
        let wanted: Vec<(TripId, Mode)> = (0..self.trips.len())
            .map(TripId::new)
            .flat_map(|t| static_modes(&self.modes[t.index()]).map(move |m| (t, m)))
            .collect();
        let sets = self.bike_route_sets(&wanted);
        let planned = parallel::map(
            &wanted,
            STATIC_TRIPS_PER_CHUNK,
            Vec::new,
            |searches: &mut Vec<Option<_>>, &(trip, mode)| {
                // A run without the layer is refused before (`assign`'s check); a search built
                // without it plans no chain of the mode.
                let Some(planner) = self.static_planner(mode) else {
                    return Err(Unplanned::NoRoute);
                };
                let chains = match (&sets, mode) {
                    (Some(sets), Mode::Bike) => self.chains_of_set(trip, planner, sets),
                    _ => self.plan_static(trip, mode, planner, searches).into_iter().collect(),
                };
                if chains.is_empty() {
                    return Err(Unplanned::NoRoute);
                }
                // The leg's seconds on its layer, its fixed times not counted: the ride, or the
                // walk.
                let limit = self.modes[trip.index()].max_seconds(mode);
                let within: Vec<Chain> = chains
                    .into_iter()
                    .filter(|chain| {
                        let a = chain.legs()[0].attributes;
                        let seconds =
                            if mode == Mode::Bike { a.in_vehicle_seconds } else { a.walk_seconds };
                        seconds <= limit
                    })
                    .collect();
                if within.is_empty() {
                    return Err(Unplanned::TooLong);
                }
                Ok(within)
            },
        );
        for ((trip, mode), chains) in wanted.into_iter().zip(planned) {
            match chains {
                Ok(chains) => self.fixed_chains[trip.index()].extend(chains),
                Err(why) if self.modes[trip.index()].chooses_mode() => {
                    self.unplanned[trip.index()].push((mode, why));
                }
                Err(_) => {}
            }
        }
        self
    }

    /// The search with the planner of transit legs: every trip whose mode is stated as transit is
    /// planned now, once, its ends snapped to the walk layer, leaving at its departure; each
    /// journey kept becomes one of its chains, fewest vehicles first ([`crate::transit`]). The
    /// walks to and from stops are searched once per walk node, the journeys once per trip, in
    /// parallel, each by its own search, so the chains do not depend on the number of threads.
    ///
    /// # Panics
    ///
    /// Never: every trip end's walks are searched before its journeys.
    #[must_use]
    pub fn with_transit(mut self, planner: TransitPlanner<'a>) -> Self {
        let wanted: Vec<TripId> = (0..self.trips.len())
            .map(TripId::new)
            .filter(|t| self.modes[t.index()] == TripModes::Stated(Mode::Transit))
            .collect();
        let ends: Vec<(Place, Place)> = wanted
            .iter()
            .map(|&t| {
                (planner.place(self.trips.origin(t)), planner.place(self.trips.destination(t)))
            })
            .collect();
        // The walks of every distinct origin node (to stops) and destination node (from stops).
        let mut nodes: Vec<(NodeId, bool)> =
            ends.iter().flat_map(|(o, d)| [(o.node(), false), (d.node(), true)]).collect();
        nodes.sort_unstable_by_key(|&(n, b)| (n.raw(), b));
        nodes.dedup();
        let walks = parallel::map(
            &nodes,
            STATIC_TRIPS_PER_CHUNK,
            || planner.reach(),
            |reach, &(n, b)| planner.walks(reach, n, b),
        );
        let walk_of = |n: NodeId, b: bool| {
            let i = nodes.binary_search_by_key(&(n.raw(), b), |&(m, c)| (m.raw(), c));
            &walks[i.expect("every end's walks were searched")]
        };
        let items: Vec<usize> = (0..wanted.len()).collect();
        let planned = parallel::map(
            &items,
            STATIC_TRIPS_PER_CHUNK,
            || planner.raptor(),
            |raptor, &i| {
                let (from, to) = ends[i];
                let departure = self.trips.departure(wanted[i]).get();
                let legs = planner.legs(
                    raptor,
                    (from, to),
                    departure,
                    walk_of(from.node(), false),
                    walk_of(to.node(), true),
                );
                legs.into_iter().map(|leg| Chain::new(vec![leg])).collect::<Vec<Chain>>()
            },
        );
        for (trip, chains) in wanted.into_iter().zip(planned) {
            self.fixed_chains[trip.index()].extend(chains);
        }
        self.transit = Some(planner);
        self
    }

    /// The planner of transit legs, if the run has a timetable.
    #[must_use]
    pub fn transit(&self) -> Option<&TransitPlanner<'a>> {
        self.transit.as_ref()
    }

    /// The bike planner's sampled route sets for the bike trips of `wanted`, one per pair of
    /// their ends snapped to the bike layer; `None` without a bike planner that makes them.
    fn bike_route_sets(&self, wanted: &[(TripId, Mode)]) -> Option<RouteSets> {
        let planner = self.bike.as_ref().filter(|p| p.has_route_sets())?;
        let keys: Vec<RouteKey> = wanted
            .iter()
            .filter(|&&(_, mode)| mode == Mode::Bike)
            .map(|&(trip, _)| {
                let (from, to) = self.static_places(trip, planner);
                RouteKey::new(from.node(), to.node())
            })
            .collect();
        planner.route_sets(&keys)
    }

    /// `trip`'s ends snapped to `planner`'s layer.
    fn static_places(&self, trip: TripId, planner: &StaticPlanner<'a>) -> (Place, Place) {
        (planner.place(self.trips.origin(trip)), planner.place(self.trips.destination(trip)))
    }

    /// The bike chains of `trip` from the sampled `sets`: one per route of its pair, in the set's
    /// order; one chain of no link where its two ends are one node; none where the layer has no
    /// route between them.
    fn chains_of_set(
        &self,
        trip: TripId,
        planner: &StaticPlanner<'a>,
        sets: &RouteSets,
    ) -> Vec<Chain> {
        let (from, to) = self.static_places(trip, planner);
        let departure = f64::from(self.trips.departure(trip).get());
        if from.node() == to.node() {
            return vec![Chain::new(vec![planner.leg_along(from, to, departure, Vec::new())])];
        }
        let Some(k) = sets.key_index(RouteKey::new(from.node(), to.node())) else {
            return Vec::new();
        };
        sets.routes(k)
            .map(|route| {
                let links = route.links.iter().map(|&l| LinkId::new(l)).collect();
                Chain::new(vec![planner.leg_along(from, to, departure, links)])
            })
            .collect()
    }

    /// The static planner of `mode`, if the run has its layer.
    fn static_planner(&self, mode: Mode) -> Option<&StaticPlanner<'a>> {
        match mode {
            Mode::Bike => self.bike.as_ref(),
            Mode::Walk => self.walk.as_ref(),
            _ => None,
        }
    }

    /// The `mode` chain of `trip` (bike or walking): one leg of that mode on its layer, by
    /// `planner`'s search of its class (kept in `searches`, by the planner's mode and search
    /// context, for the next trip the thread plans).
    fn plan_static<'s>(
        &'s self,
        trip: TripId,
        mode: Mode,
        planner: &'s StaticPlanner<'a>,
        searches: &mut Vec<Option<(Mode, usize, openmobisim_core_routes::search::Search<'s>)>>,
    ) -> Option<Chain> {
        let class = self.travellers.user_class(self.trips.traveller(trip));
        let context = planner.context_of(class)?;
        let (from, to) = self.static_places(trip, planner);
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
            Mode::Transit => self.transit.as_ref().map(|p| p as &dyn LegPlanner),
            _ => None,
        }
    }

    /// The bike or walking planner of `mode`, if the run has its layer.
    #[must_use]
    pub fn layer_planner(&self, mode: Mode) -> Option<&StaticPlanner<'a>> {
        self.static_planner(mode)
    }

    /// The feasible chains of `traveller`'s `trip` at `state`, one per mode it may use, in the
    /// order car, bike, walking (see the [module docs](self) for what is enumerated).
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
        let mut out = Vec::new();
        if self.modes[trip.index()].allows(Mode::Car) {
            let (from, to) = self.ends(trip);
            let departure = f64::from(self.trips.departure(trip).get());
            let class = self.travellers.user_class(traveller);
            out.extend(
                self.car
                    .plan(from, to, departure, class, state, params.cost_source)
                    .map(|leg| Chain::new(vec![leg]))
                    .filter(|chain| feasible(state, traveller, chain)),
            );
        }
        out.extend(
            self.fixed_chains[trip.index()]
                .iter()
                .filter(|chain| feasible(state, traveller, chain))
                .cloned(),
        );
        out
    }
}

impl ChainSearch<'_> {
    /// The feasible chains of every trip at `state`, by trip, each best first: for every trip
    /// what [`Self::search`] returns for it, worked out with shared searches (see the
    /// [module docs](self)).
    #[must_use]
    pub fn search_all(&self, state: &State, params: &Params) -> Vec<Vec<Chain>> {
        let mut all: Vec<Vec<Chain>> = self
            .by_groups(state, params, |_, _| true)
            .into_iter()
            .map(|c| c.into_iter().collect())
            .collect();
        self.add_static(state, &mut all, |_, _| true);
        all
    }

    /// The search stage's question for every trip at `state`: per trip, the chains
    /// [`Self::search`] returns that are not yet in the trip's choice set, in its order. A route
    /// already in the set is not priced; a bike or walking chain, planned once, is new only
    /// until it is in the set.
    pub(crate) fn fresh_chains(&self, state: &State, params: &Params) -> Vec<Vec<Chain>> {
        let mut fresh: Vec<Vec<Chain>> = self
            .by_groups(state, params, |trip, links| {
                !state.trip(trip).set.iter().any(|c| c.is_single_leg(Mode::Car, links))
            })
            .into_iter()
            .map(|c| c.into_iter().collect())
            .collect();
        self.add_static(state, &mut fresh, |trip, chain| {
            !state.trip(trip).set.iter().any(|c| c.same_alternative(chain))
        });
        fresh
    }

    /// Onto `out`, by trip, every bike and walking chain planned once, if feasible at `state`
    /// and `wanted`.
    fn add_static<W>(&self, state: &State, out: &mut [Vec<Chain>], wanted: W)
    where
        W: Fn(TripId, &Chain) -> bool,
    {
        for (t, chains) in self.fixed_chains.iter().enumerate() {
            let trip = TripId::from_index(t);
            for chain in chains {
                if feasible(state, self.trips.traveller(trip), chain) && wanted(trip, chain) {
                    out[t].push(chain.clone());
                }
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
/// another (each begins where the last ended), every leg is a car, bike, walking or transit leg,
/// the traveller has a car for each car leg and a bike for each bike leg (from their class's or
/// their own ownership); walking and transit need nothing (fares are not priced yet, so a
/// transit pass is not read). Where a vehicle is during the day is not
/// tracked: a traveller with a car (a bike) may drive (ride) it on every trip.
#[must_use]
pub fn feasible(state: &State, traveller: TravellerId, chain: &Chain) -> bool {
    let legs = chain.legs();
    !legs.is_empty()
        && legs.windows(2).all(|w| w[0].to == w[1].from)
        && legs.iter().all(|leg| match leg.mode {
            Mode::Car => state.has_car(traveller),
            Mode::Bike => state.has_bike(traveller),
            Mode::Walk | Mode::Transit => true,
            _ => false,
        })
}
