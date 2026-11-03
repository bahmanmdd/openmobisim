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
//! driven link by link from the departure time on the state's link times
//! ([`LinkTimes::route_seconds`], the wait to get onto the first link included, read the way
//! the search read it), the money and the length are summed over the links. So the time the
//! search minimised and the time the leg is priced at are one number.
//!
//! **Ties.** Of two routes of exactly equal cost, the search keeps the one its tie rule settles
//! first (labels of equal cost are settled in order of link number); which one that is depends
//! on the numbering, never on chance.
//!
//! **Cost of a plan.** One search over the links it touches, plus a fresh search scratch of
//! about 40 bytes per link and one byte per node of the network. A planner may keep searches
//! between plans (one tree per origin and departure bin) without changing any answer.
//!
//! # Cost source
//!
//! [`CostSource`] says where a leg's cost is read: `Exact`, by searching at the leg's own
//! departure time on the state's link times, or `Skim`, from a table of expected costs between
//! places by departure bin (a skim). The skim table does not exist yet: a `Skim` request is
//! answered by the same search on the same link times, through the same code, so the two sources
//! give the same leg; when the table is added, it changes where the costs are read and nothing
//! else.

use openmobisim_core_demand::Mode;
use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_loading::LinkTimes;
use openmobisim_core_routes::search::{Extras, Search, SearchContext};
use openmobisim_core_types::ids::{EntityId, LinkId, UserClassId};

use crate::attributes::{Attributes, Coefficients};
use crate::chain::{Leg, Place};
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
        Ok(Self { network, context, money, extras })
    }

    /// The link travel times a leg's cost is read from under `cost_source`. Both sources read
    /// the state's link times until the skim table exists (see the [module docs](self)).
    fn times(state: &State, cost_source: CostSource) -> &LinkTimes {
        match cost_source {
            CostSource::Exact | CostSource::Skim => state.link_times(),
        }
    }

    /// The attributes of driving `links` from second `departure` on `times`.
    fn price(&self, links: &[LinkId], departure: f64, times: &LinkTimes) -> Attributes {
        let raw: Vec<u32> = links.iter().map(|l| l.raw()).collect();
        Attributes {
            in_vehicle_seconds: times.route_seconds(&raw, departure),
            money: links.iter().map(|l| self.money[l.index()]).sum(),
            length_metres: links.iter().map(|&l| self.network.link_length(l).get()).sum(),
            ..Attributes::default()
        }
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
        Leg { mode: Mode::Car, from, to, departure, attributes, links }
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
        // Extras are shared by handle: this copies a pointer, not the per-link values.
        search.set_extras(Some(extras.clone()));
        let wait = |l: u32, t: f64| times.origin_wait_seconds(l, t);
        let seconds = |l: u32, t: f64| times.link_seconds(l, t);
        let mut target = vec![false; self.network.node_count() as usize];
        target[destination.index()] = true;
        let (_, _, _, links) = search
            .cheapest_routes_to(origin, &target, 1, departure, f64::INFINITY, &wait, &seconds)
            .pop()?;
        let attributes = self.price(&links, departure, times);
        Some(Leg { mode: Mode::Car, from, to, departure, attributes, links })
    }

    fn reprice(&self, leg: &mut Leg, state: &State) {
        leg.attributes = self.price(&leg.links, leg.departure, state.link_times());
    }
}
