//! Leg planners: the best leg of one mode between two places at a departure time.
//!
//! A [`LegPlanner`] answers one question: given where a leg begins and ends, when it departs,
//! the traveller's user class and the current [`State`], which leg of its mode is cheapest,
//! with its attributes. It is deterministic: the same inputs give the same leg, to the bit.
//! The same planner also **re-prices** a leg it planned on new link travel times (the measure
//! stage), with the one pricing function it plans with.
//!
//! # The car planner
//!
//! [`CarPlanner`] searches the road network for the route of least cost in utility units,
//! time-dependently: entering link `a` at clock time `t` costs
//!
//! ```text
//! c_a(t) = β_ivt / 60 · τ_a(t) + β_money · m_a + β_km · ℓ_a / 1000
//! ```
//!
//! where `τ_a(t)` is the link's travel time in seconds when entered at `t` (from the state's
//! [`LinkTimes`]), `m_a` the money charged on the link and `ℓ_a` its length in metres; the clock
//! advances by `τ_a(t)` along the route (first-in-first-out link times keep this exact).
//!
//! **The wait at the origin.** A vehicle setting out at `t₀` onto first link `a` may have to
//! wait `w_a(t₀)` outside the network before the link takes it (a loading that meters entries
//! from origins records this wait by the bin of departure,
//! [`LinkTimes::origin_wait_seconds`]); the wait is time spent in the vehicle, so it costs
//! `β_ivt / 60` per second like a link's time, and the first link is entered at `t₀ + w_a(t₀)`.
//! The search counts it for every first link it tries, so a route is never chosen blind to a
//! queue at its origin. (The classic point queue records no such wait: its storage is infinite
//! and a departing vehicle enters its first link at once, queueing at that link's downstream
//! end instead.)
//!
//! The search itself is the label-setting search over links of `core-routes`
//! ([`Search::cheapest_routes_to`] with one target), which minimises
//! `w_a(t₀) + Σ (τ_a(t) + e_a)` with a static extra `e_a` per link; the planner hands it
//! `e_a = (β_money · m_a + β_km · ℓ_a / 1000) · 60 / β_ivt`, so the route it returns minimises
//! the cost above (divided by the positive `β_ivt / 60`). A run without tolls has `m_a = 0`
//! everywhere: the same search, the same code.
//!
//! The leg's attributes are then priced from its links: the in-vehicle seconds are the route
//! driven link by link from the vehicle's departure on the state's link times
//! ([`LinkTimes::route_seconds`], the wait to get onto the first link included, read the way
//! the search read it), the money and the length are summed over the links. So the time the
//! search minimised and the time the leg is priced at are one number.
//!
//! **The fixed times.** A car leg carries the planner's [`FixedTimes`]
//! ([`CarPlanner::with_fixed_times`]; none by default): the car sets off the access time after
//! the leg's departure (reaching the car), and the leg ends the egress time after the car
//! arrives (parking and leaving it). The search and the pricing of the drive run from the
//! car's departure, `t₀ = departure + access`; the attributes then add the access time as
//! walking time and the egress time as parking time ([`FixedTimes::added_to`]). **The search
//! is unchanged by them:** every route of the leg costs the same fixed amount more,
//! `β_walk / 60 · access + β_park / 60 · egress`, which does not depend on the route, so the
//! route of least cost from `t₀` is the route of least cost of the whole leg. A leg whose two
//! ends are one node uses no link, leaves no car, and carries no fixed time.
//!
//! **Ties.** Of two routes of exactly equal cost, the search keeps the one its tie rule settles
//! first (labels of equal cost are settled in order of link number); which one that is depends
//! on the numbering, never on chance.
//!
//! **Cost of a plan.** One search over the links it touches, plus a fresh search scratch of
//! about 40 bytes per link and one byte per node of the network.
//!
//! # Shared searches
//!
//! The chain search asks the car planner for many legs at once: the trips that leave one
//! origin node, of one user class, within one departure window of
//! [`SHARED_SEARCH_WINDOW_SECONDS`] (`CarPlanner::routes_from`). It answers them with as few
//! searches as it can **without changing any answer**: each leg gets, link for link, the route
//! its own search from the origin at its own departure time would return.
//!
//! One search from the origin at the earliest departure `t₁` not yet answered, towards every
//! destination of the trips not yet answered, reads back one route per destination (the label
//! of each link is settled once and never changes afterwards, so a route read when the search
//! ends is the one read when its destination was reached, where a search towards that
//! destination alone would have stopped). That search is also the search of a trip leaving at
//! `t₁ + δ`, operation for operation, if every link time it read is the same at a clock `δ`
//! later: the link travel times are constant within each of their bins of entry time, so the
//! search records, over every link time it reads at clock `t`, the time until that link's time
//! next changes, and the least of these, `s`, bounds the `δ` it answers for. A trip leaving at
//! `t₁ + δ` takes the route found for its destination if `δ ≤ s − 0.001 s` (the margin covers
//! the rounding of the clock sums) and the wait to get onto every first link the search tries is
//! the same at its departure as at `t₁`; the trips left over get the next search, from the
//! earliest of them. A trip leaving at `t₁` itself is always answered. At free flow nothing
//! changes, and one search answers every trip of the window; in a queue whose time changes from
//! bin to bin, a search answers the trips until the first such change on its tree.
//!
//! The argument needs every step of the search to cost zero or more (true of a link's time plus
//! the class's extra whenever the extras are not negative). A search that meets a negative step
//! answers nothing; its trips get a search each, as [`LegPlanner::plan`] makes it.
//!
//! `TimeChanges` says when each link's time changes. It reads the link times as the table of
//! their bins of entry time ([`LinkTimes::rows`]), which is how a loading's link times and
//! the free-flow times are made; if a time read during a search is not the table's value for
//! its bin (a table of times built otherwise), that search answers only the trips leaving at
//! `t₁`.
//!
//! # Cost source
//!
//! [`CostSource`] says where a leg's cost is read: `Exact`, by searching at the leg's own
//! departure time on the state's link times, or `Skim`, from a table of expected costs between
//! places by departure bin (a skim). The skim table does not exist yet: a `Skim` request is
//! answered by the same search on the same link times, through the same code, so the two sources
//! give the same leg; when the table is added, it changes where the costs are read and nothing
//! else.

use std::cell::{Cell, RefCell};

use openmobisim_core_demand::Mode;
use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_loading::LinkTimes;
use openmobisim_core_routes::search::{Extras, Search, SearchContext};
use openmobisim_core_types::ids::{EntityId, LinkId, NodeId, UserClassId};

use crate::attributes::{Attributes, Coefficients};
use crate::chain::{FixedTimes, Leg, Place};
use crate::error::AssignError;
use crate::state::State;

/// Where a leg's cost is read. See the [module docs](self#cost-source).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CostSource {
    /// Plan the leg at its own departure time on the state's link travel times.
    #[default]
    Exact,
    /// Read the leg's cost from the table of expected costs between places. Until that table
    /// exists, answered exactly as [`CostSource::Exact`], by the same code.
    Skim,
}

/// A planner of legs of one mode. See the [module docs](self).
pub trait LegPlanner {
    /// The mode of the legs this planner makes.
    fn mode(&self) -> Mode;

    /// The cheapest leg of this planner's mode from `from` to `to` departing at `departure`
    /// (seconds after midnight) for a traveller of `class`, on `state`, its cost read from
    /// `cost_source`; `None` if there is none.
    fn plan(
        &self,
        from: Place,
        to: Place,
        departure: f64,
        class: UserClassId,
        state: &State,
        cost_source: CostSource,
    ) -> Option<Leg>;

    /// Price `leg` (one this planner made) again at its departure time on `state`: its
    /// attributes become what the same links cost now.
    fn reprice(&self, leg: &mut Leg, state: &State);
}

/// The car planner: the route of least cost in utility units at the departure time. See the
/// [module docs](self#the-car-planner).
#[derive(Debug)]
pub struct CarPlanner<'a> {
    network: &'a RoadNetwork,
    context: SearchContext<'a>,
    money: &'a [f64],
    /// Per user class, the static extra per link the search adds to each link's travel time.
    extras: Vec<Extras>,
    /// The fixed times of every car leg that uses a link.
    fixed: FixedTimes,
}

impl<'a> CarPlanner<'a> {
    /// A planner over `network` and its `turns`, for user classes weighing attributes by
    /// `classes` (indexed by user class), with `money` charged on each link (one value per link
    /// of the network, zeros where nothing is charged).
    ///
    /// # Errors
    ///
    /// [`AssignError::Input`] if `money` is not one value per link or holds a value that is not
    /// a finite number; [`AssignError::Coefficients`] if a class's cost of a minute in a vehicle
    /// is not a positive finite number, or makes some link cost less than nothing at free flow.
    ///
    /// # Panics
    ///
    /// Never in practice: the number of classes fits `u32`.
    pub fn new(
        network: &'a RoadNetwork,
        turns: &'a TurnTable,
        classes: &[Coefficients],
        money: &'a [f64],
    ) -> Result<Self, AssignError> {
        let links = network.link_count() as usize;
        if money.len() != links {
            return Err(AssignError::Input(format!(
                "the money per link has {} values for {links} links",
                money.len()
            )));
        }
        if let Some(l) = money.iter().position(|m| !m.is_finite()) {
            return Err(AssignError::Input(format!(
                "the money on link {l} is not a finite number"
            )));
        }
        let context = SearchContext::new(network, turns);
        let mut extras = Vec::with_capacity(classes.len());
        for (c, k) in classes.iter().enumerate() {
            let class = u32::try_from(c).expect("fewer than 2^32 classes");
            let per_second = k.in_vehicle_per_minute / 60.0;
            if !(per_second.is_finite() && per_second > 0.0) {
                return Err(AssignError::Coefficients {
                    class,
                    reason: format!(
                        "the cost of a minute in a vehicle is {}, not a positive number",
                        k.in_vehicle_per_minute
                    ),
                });
            }
            let seconds: Vec<f64> = (0..network.link_count())
                .map(|l| {
                    let link = LinkId::new(l);
                    let km = network.link_length(link).get() / 1000.0;
                    (k.per_money * money[link.index()] + k.per_kilometre * km) / per_second
                })
                .collect();
            for (l, &e) in seconds.iter().enumerate() {
                let free = context.link_cost(LinkId::from_index(l));
                if free.is_finite() && !(e.is_finite() && free + e >= 0.0) {
                    return Err(AssignError::Coefficients {
                        class,
                        reason: format!("link {l} would cost {} seconds-equivalent", free + e),
                    });
                }
            }
            extras.push(context.extras(seconds));
        }
        Ok(Self { network, context, money, extras, fixed: FixedTimes::NONE })
    }

    /// The planner with `fixed` times before and after every car leg that uses a link (see the
    /// [module docs](self#the-car-planner)).
    ///
    /// # Errors
    ///
    /// [`AssignError::Input`] if a time is not a finite number of at least 0.
    pub fn with_fixed_times(mut self, fixed: FixedTimes) -> Result<Self, AssignError> {
        if !fixed.is_valid() {
            return Err(AssignError::Input(format!(
                "the car's fixed times must be finite numbers of seconds of at least 0, not \
                 {} s before and {} s after the drive",
                fixed.access_seconds, fixed.egress_seconds
            )));
        }
        self.fixed = fixed;
        Ok(self)
    }

    /// The fixed times of every car leg that uses a link.
    #[must_use]
    pub fn fixed_times(&self) -> FixedTimes {
        self.fixed
    }

    /// The fixed times of a leg along `links`: the planner's, or none for a leg of no link.
    fn fixed_of(&self, links: &[LinkId]) -> FixedTimes {
        if links.is_empty() { FixedTimes::NONE } else { self.fixed }
    }

    /// The link travel times a leg's cost is read from under `cost_source`. Both sources read
    /// the state's link times until the skim table exists (see the [module docs](self)).
    fn times(state: &State, cost_source: CostSource) -> &LinkTimes {
        match cost_source {
            CostSource::Exact | CostSource::Skim => state.link_times(),
        }
    }

    /// The attributes of a car leg along `links` departing at second `departure`, on `times`:
    /// the drive from the car's departure (`departure` plus the access time), and the fixed
    /// times as walking and parking time.
    fn price(&self, links: &[LinkId], departure: f64, times: &LinkTimes) -> Attributes {
        let fixed = self.fixed_of(links);
        let setting_off = departure + fixed.access_seconds;
        // The link numbers in a buffer kept by the thread: one allocation per thread, not per leg.
        let in_vehicle_seconds = RAW_LINKS.with(|raw| {
            let mut raw = raw.borrow_mut();
            raw.clear();
            raw.extend(links.iter().map(|l| l.raw()));
            times.route_seconds(&raw, setting_off)
        });
        fixed.added_to(Attributes {
            in_vehicle_seconds,
            money: links.iter().map(|l| self.money[l.index()]).sum(),
            length_metres: links.iter().map(|&l| self.network.link_length(l).get()).sum(),
            ..Attributes::default()
        })
    }

    /// The car leg along `links` departing at `departure`, with `attributes`.
    fn leg(
        &self,
        from: Place,
        to: Place,
        departure: f64,
        links: Vec<LinkId>,
        attributes: Attributes,
    ) -> Leg {
        let fixed = self.fixed_of(&links);
        Leg { mode: Mode::Car, from, to, departure, attributes, fixed, links }
    }

    /// The car leg along `links` (a route from `from` to `to`) departing at `departure`,
    /// priced on the state's link times: what the planner returns for a route it found, and
    /// how a route found elsewhere (given by hand, say) becomes a leg.
    #[must_use]
    pub fn leg_along(
        &self,
        from: Place,
        to: Place,
        departure: f64,
        links: Vec<LinkId>,
        state: &State,
    ) -> Leg {
        let attributes = self.price(&links, departure, state.link_times());
        self.leg(from, to, departure, links, attributes)
    }

    /// The car leg along `links` from `from` to `to` departing at `departure`, priced where
    /// `cost_source` says: what [`LegPlanner::plan`] returns once it has found `links`.
    pub(crate) fn leg_on(
        &self,
        from: Place,
        to: Place,
        departure: f64,
        links: Vec<LinkId>,
        state: &State,
        cost_source: CostSource,
    ) -> Leg {
        let attributes = self.price(&links, departure, Self::times(state, cost_source));
        self.leg(from, to, departure, links, attributes)
    }

    /// A thread's scratch for [`Self::routes_from`].
    pub(crate) fn scratch(&self) -> RouteScratch<'_> {
        let nodes = self.network.node_count() as usize;
        RouteScratch {
            search: Search::new(&self.context),
            targets: vec![false; nodes],
            slot: vec![NO_SLOT; nodes],
            waits: Vec::new(),
        }
    }

    /// When each link's time changes on the link times `cost_source` reads from `state`.
    pub(crate) fn changes(&self, state: &State, cost_source: CostSource) -> TimeChanges {
        TimeChanges::new(Self::times(state, cost_source), self.network.link_count())
    }

    /// The route of least cost from `origin` to `destination` leaving at `departure` for a
    /// class with `extras`, on `times`: one search towards the destination alone, the search
    /// [`LegPlanner::plan`] makes.
    fn route(
        search: &mut Search<'_>,
        extras: &Extras,
        origin: NodeId,
        destination: NodeId,
        departure: f64,
        times: &LinkTimes,
        target: &mut [bool],
    ) -> Option<Vec<LinkId>> {
        search.set_extras(Some(extras.clone()));
        let wait = |l: u32, t: f64| times.origin_wait_seconds(l, t);
        let seconds = |l: u32, t: f64| times.link_seconds(l, t);
        target[destination.index()] = true;
        let found =
            search.cheapest_routes_to(origin, target, 1, departure, f64::INFINITY, &wait, &seconds);
        target[destination.index()] = false;
        found.into_iter().next_back().map(|(_, _, _, links)| links)
    }

    /// The routes of legs from `origin` for a traveller of `class`, by shared searches (see the
    /// [module docs](self#shared-searches)): `requests` holds each car's departure (seconds after
    /// midnight: the leg's departure plus the access time, [`Self::fixed_times`]) and
    /// destination, in order of departure; the routes found are appended to `routes`, and the
    /// answer per request is the index of its route there, `None` where no route exists. Each
    /// answer is the route [`LegPlanner::plan`] finds for the leg of that request.
    ///
    /// # Panics
    ///
    /// Panics if `class` has no coefficients or a destination is not a node of the network.
    #[allow(
        clippy::too_many_arguments,
        reason = "the planner's question, the state, and where the answers and the scratch go"
    )]
    pub(crate) fn routes_from(
        &self,
        origin: NodeId,
        class: UserClassId,
        requests: &[(f64, NodeId)],
        state: &State,
        cost_source: CostSource,
        changes: &TimeChanges,
        scratch: &mut RouteScratch<'_>,
        routes: &mut Vec<Vec<LinkId>>,
    ) -> Vec<Option<usize>> {
        let extras = &self.extras[class.index()];
        let extra = extras.seconds();
        let times = Self::times(state, cost_source);
        let RouteScratch { search, targets, slot, waits } = scratch;
        let mut answer: Vec<Option<usize>> = vec![None; requests.len()];
        let mut answered = vec![false; requests.len()];
        let mut first = 0;
        // How far after its departure the next search looks for destinations: twice what the
        // last search answered for (a search that looks for fewer reads fewer link times and
        // answers for longer); everything at first.
        let mut horizon = f64::INFINITY;
        loop {
            while first < requests.len() && answered[first] {
                first += 1;
            }
            let Some(&(t1, _)) = requests.get(first) else { break };
            let mut wanted = 0;
            for (j, &(departure, to)) in requests.iter().enumerate().skip(first) {
                if j > first && departure - t1 > horizon {
                    break;
                }
                if !answered[j] && !targets[to.index()] {
                    targets[to.index()] = true;
                    wanted += 1;
                }
            }
            // The least time, over every link time the search reads, until that time changes;
            // and whether some step costs less than nothing.
            let slack = Cell::new(f64::INFINITY);
            let negative = Cell::new(false);
            let wait = |l: u32, t: f64| times.origin_wait_seconds(l, t);
            let seconds = |l: u32, t: f64| {
                let s = times.link_seconds(l, t);
                slack.set(slack.get().min(changes.seconds_until_change(l, t, s)));
                if s + extra[l as usize] < 0.0 {
                    negative.set(true);
                }
                s
            };
            search.set_extras(Some(extras.clone()));
            let found = search.cheapest_routes_to(
                origin,
                targets,
                wanted,
                t1,
                f64::INFINITY,
                &wait,
                &seconds,
            );
            if negative.get() {
                for &(_, to) in &requests[first..] {
                    targets[to.index()] = false;
                }
                // Answered one by one, by the search each would have alone.
                for (j, &(departure, to)) in requests.iter().enumerate().skip(first) {
                    if !answered[j] {
                        let r = Self::route(search, extras, origin, to, departure, times, targets);
                        answer[j] = r.map(|links| {
                            routes.push(links);
                            routes.len() - 1
                        });
                        answered[j] = true;
                    }
                }
                break;
            }
            for (node, _, _, links) in found {
                slot[node.index()] = u32::try_from(routes.len()).expect("fewer than 2^32 routes");
                routes.push(links);
            }
            // The wait to get onto each first link the search tried, at `t₁`.
            waits.clear();
            for &l in self.network.out_links(origin) {
                if self.context.link_cost(l).is_finite() {
                    waits.push((l.raw(), times.origin_wait_seconds(l.raw(), t1).to_bits()));
                }
            }
            let limit = slack.get() - SHARED_SEARCH_MARGIN_SECONDS;
            horizon = (2.0 * slack.get()).max(0.0);
            for (j, &(departure, to)) in requests.iter().enumerate().skip(first) {
                if answered[j] {
                    continue;
                }
                let at_t1 = departure.to_bits() == t1.to_bits();
                if !at_t1 && departure - t1 > limit {
                    // Later requests leave later still: none of them is answered either.
                    break;
                }
                // A request whose destination the search did not look for waits for a later one.
                if !targets[to.index()] {
                    continue;
                }
                let same = at_t1
                    || waits
                        .iter()
                        .all(|&(l, w)| times.origin_wait_seconds(l, departure).to_bits() == w);
                if same {
                    let at = slot[to.index()];
                    answer[j] = (at != NO_SLOT).then_some(at as usize);
                    answered[j] = true;
                }
            }
            for &(_, to) in &requests[first..] {
                slot[to.index()] = NO_SLOT;
                targets[to.index()] = false;
            }
        }
        answer
    }
}

/// The departure window of the car planner's shared searches, in seconds: the trips of one
/// origin and user class leaving within one such window of the day (counted from midnight) are
/// answered together (by shared searches, see the [module docs](self#shared-searches)). It bounds how many destinations one search
/// looks for; it changes no answer.
pub const SHARED_SEARCH_WINDOW_SECONDS: u32 = 300;

/// The margin, in seconds, a shared search keeps from the next change of a link time it read:
/// far above the rounding of the clock sums along any route (about 10⁻¹¹ s per link at a clock
/// of 10⁵ s) and far below the one second between two departures.
const SHARED_SEARCH_MARGIN_SECONDS: f64 = 1e-3;

/// A node with no route found by the current shared search.
const NO_SLOT: u32 = u32::MAX;

thread_local! {
    /// A thread's buffer of link numbers for pricing a route.
    static RAW_LINKS: RefCell<Vec<u32>> = const { RefCell::new(Vec::new()) };
}

/// What a thread keeps between the car planner's shared searches.
#[derive(Debug)]
pub(crate) struct RouteScratch<'a> {
    search: Search<'a>,
    /// Per node, whether the current search looks for it.
    targets: Vec<bool>,
    /// Per node, the index of the current search's route to it, [`NO_SLOT`] if none.
    slot: Vec<u32>,
    /// The origin's first links and their waits at the current search's departure, as bits.
    waits: Vec<(u32, u64)>,
}

/// When each link's travel time next changes, read from a table of link travel times that are
/// constant within each bin of entry time (see the [module docs](self#shared-searches)).
#[derive(Debug)]
pub(crate) struct TimeChanges {
    bin_seconds: f64,
    /// Per link, its free-flow time: what it takes outside the bins of its span.
    free_flow: Vec<f64>,
    /// Per link, the first bin of its span and its times there,
    /// `span[span_start[l]..span_start[l + 1]]`.
    span_first: Vec<u32>,
    span_start: Vec<u32>,
    span: Vec<f64>,
    /// Per link, the bins at whose start its time differs from the bin before, ascending:
    /// `bins[change_start[l]..change_start[l + 1]]`.
    change_start: Vec<u32>,
    bins: Vec<u32>,
}

impl TimeChanges {
    /// The changes of `times` over `links` links.
    ///
    /// # Panics
    ///
    /// Never in practice: counts of link-bin cells fit `u32`.
    pub(crate) fn new(times: &LinkTimes, links: u32) -> Self {
        let rows = times.rows();
        let n = links as usize;
        let mut out = Self {
            bin_seconds: times.bin_seconds(),
            free_flow: Vec::with_capacity(n),
            span_first: vec![0; n],
            span_start: vec![0; n + 1],
            span: Vec::with_capacity(rows.len()),
            change_start: vec![0; n + 1],
            bins: Vec::new(),
        };
        let fits = |x: usize| u32::try_from(x).expect("fewer than 2^32 link-bin cells");
        let mut r = 0;
        for l in 0..links {
            // Far past any recorded bin a link takes its free-flow time.
            let free = times.link_seconds(l, f64::MAX);
            out.free_flow.push(free);
            let mut before = free.to_bits();
            let mut next = None;
            while let Some(&(link, bin, seconds)) = rows.get(r).filter(|row| row.0 == l) {
                debug_assert_eq!(link, l);
                if next.is_none() {
                    out.span_first[l as usize] = bin;
                }
                if seconds.to_bits() != before {
                    out.bins.push(bin);
                }
                before = seconds.to_bits();
                out.span.push(seconds);
                next = Some(bin + 1);
                r += 1;
            }
            if let Some(after) = next.filter(|_| before != free.to_bits()) {
                out.bins.push(after);
            }
            out.span_start[l as usize + 1] = fits(out.span.len());
            out.change_start[l as usize + 1] = fits(out.bins.len());
        }
        out
    }

    /// The bin of clock time `at`, as [`LinkTimes`] reads it.
    fn bin_of(&self, at: f64) -> u32 {
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a non-negative time in seconds over a bin length, within the u32 clock"
        )]
        let bin = (at.max(0.0) / self.bin_seconds) as u32;
        bin
    }

    /// The seconds from clock time `at` until `link`'s time next changes, given that it is
    /// `seconds` at `at`; 0 if `seconds` is not the table's time for that bin (times not made
    /// of bins), infinite if it never changes.
    pub(crate) fn seconds_until_change(&self, link: u32, at: f64, seconds: f64) -> f64 {
        let l = link as usize;
        let bin = self.bin_of(at);
        let (a, z) = (self.span_start[l] as usize, self.span_start[l + 1] as usize);
        let k = bin.wrapping_sub(self.span_first[l]) as usize;
        let table = if k < z - a { self.span[a + k] } else { self.free_flow[l] };
        if table.to_bits() != seconds.to_bits() {
            return 0.0;
        }
        let changes = &self.bins[self.change_start[l] as usize..self.change_start[l + 1] as usize];
        match changes.get(changes.partition_point(|&b| b <= bin)) {
            Some(&b) => f64::from(b) * self.bin_seconds - at,
            None => f64::INFINITY,
        }
    }
}

impl LegPlanner for CarPlanner<'_> {
    fn mode(&self) -> Mode {
        Mode::Car
    }

    fn plan(
        &self,
        from: Place,
        to: Place,
        departure: f64,
        class: UserClassId,
        state: &State,
        cost_source: CostSource,
    ) -> Option<Leg> {
        let (Place::Road(origin), Place::Road(destination)) = (from, to);
        let extras = self.extras.get(class.index())?;
        let times = Self::times(state, cost_source);
        let mut search = Search::new(&self.context);
        let mut target = vec![false; self.network.node_count() as usize];
        // Extras are shared by handle: this copies a pointer, not the per-link values.
        // The car sets off once it is reached: the route is searched from then on.
        let setting_off = departure + self.fixed.access_seconds;
        let links =
            Self::route(&mut search, extras, origin, destination, setting_off, times, &mut target)?;
        let attributes = self.price(&links, departure, times);
        Some(self.leg(from, to, departure, links, attributes))
    }

    fn reprice(&self, leg: &mut Leg, state: &State) {
        leg.attributes = self.price(&leg.links, leg.departure, state.link_times());
    }
}
