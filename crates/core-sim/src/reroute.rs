//! En-route rerouting's decision: what a vehicle stuck at the front of its link does,
//! asked by the loading ([`openmobisim_core_loading::Reroute`]).
//!
//! **The costs a vehicle sees:** each link's time is the larger of what the trip planned on —
//! the time expected from the last loading ([`LinkTimes`]; free flow at the first) — and the
//! loading's **live** estimate now (free-flow time, the queue's clearance time, how long the
//! link's front has been blocked; [`openmobisim_core_loading::LiveTimes`]). So a vehicle avoids
//! what it can see jammed now and what it knows is usually slow.
//!
//! **The decision:** the rest of its planned route is timed on those costs; the earliest
//! arrival from the end of its link, through its legal turns, is searched (A\*, bounded by the
//! planned time less the required gain); a route that beats the plan by at least
//! [`LoadingOptions::reroute_min_gain`] is taken. Buses never re-route: their line is their
//! route.
//!
//! **By the route cost** (S299): where the run's route cost is not time alone, a car is
//! re-routed by its trip's coefficient vector: the plan's cost and the search's are the time on
//! those costs plus the links' extras ([`openmobisim_core_routes::Search::cheapest_route_after`]),
//! and a route beating the plan's generalised cost by the required gain is taken.
//!
//! **Cost:** one bounded search per offer; offers come only to vehicles blocked for
//! [`LoadingOptions::reroute_after_s`], at most [`LoadingOptions::reroute_max`] taken per trip.

use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_loading::{LiveTimes, Reroute, RerouteReason};
use openmobisim_core_routes::{Extras, Search, SearchContext};
use openmobisim_core_types::ids::{EntityId, LinkId, VehicleId};

use crate::link_times::LinkTimes;
use crate::loading_rules::LoadingOptions;

/// What a run's re-routing searches by where its route cost is not time alone (S299): each
/// coefficient vector's extras prepared for the car's context, and each trip's vector by trip
/// (a car's vehicle id is its trip's).
#[derive(Debug)]
pub(crate) struct RerouteCost {
    pub extras: Vec<Option<Extras>>,
    pub trip_vector: Vec<u32>,
}

/// Decides reroutes for one loading.
pub(crate) struct Rerouter<'a> {
    network: &'a RoadNetwork,
    search: Search<'a>,
    expected: Option<&'a LinkTimes>,
    cost: Option<&'a RerouteCost>,
    min_gain: f64,
    /// Vehicles with an id below this are trips' cars; the rest are buses.
    cars_below: u32,
    /// Offers answered, and routes changed.
    pub offers: u32,
    pub changed: u32,
}

impl<'a> Rerouter<'a> {
    pub(crate) fn new(
        ctx: &'a SearchContext<'a>,
        expected: Option<&'a LinkTimes>,
        options: &LoadingOptions,
        cars_below: u32,
        cost: Option<&'a RerouteCost>,
    ) -> Self {
        Self {
            network: ctx.network,
            search: Search::new(ctx),
            expected,
            cost,
            min_gain: options.reroute_min_gain,
            cars_below,
            offers: 0,
            changed: 0,
        }
    }
}

impl Reroute for Rerouter<'_> {
    fn reroute(
        &mut self,
        vehicle: VehicleId,
        current: LinkId,
        planned: &[LinkId],
        now: f64,
        live: &dyn LiveTimes,
        _reason: RerouteReason,
    ) -> Option<Vec<LinkId>> {
        // Every driver re-routes the same way when stuck, whatever the reason (only "stuck"
        // exists yet). An informed share, alerts and disruptions (S215) decide here.
        if vehicle.raw() >= self.cars_below {
            return None;
        }
        self.offers += 1;
        let expected = self.expected;
        let seconds = |l: u32, t: f64| {
            let known = expected.map_or(0.0, |e| e.link_seconds(l, t));
            known.max(live.live_seconds(LinkId::new(l)))
        };
        let destination = self.network.link_to(*planned.last()?);
        // By the trip's vector's generalised cost where it has extras (S299).
        let extras = self.cost.and_then(|c| {
            let v = c.trip_vector.get(vehicle.index()).copied().unwrap_or(0);
            c.extras.get(v as usize).and_then(Option::as_ref)
        });
        if let Some(e) = extras {
            let mut t = now;
            let mut planned_cost = 0.0;
            for l in planned {
                let s = seconds(l.raw(), t);
                t += s;
                planned_cost += s + e.seconds()[l.index()];
            }
            let bound = planned_cost * (1.0 - self.min_gain);
            self.search.clear_route_state();
            self.search.set_extras(Some(e.clone()));
            let found =
                self.search.cheapest_route_after(current, destination, now, bound, &seconds);
            self.search.clear_route_state();
            let (cost, _, links) = found?;
            return (cost < bound && !links.is_empty()).then(|| {
                self.changed += 1;
                links
            });
        }
        let mut t = now;
        for l in planned {
            t += seconds(l.raw(), t);
        }
        let planned_s = t - now;
        let bound = planned_s * (1.0 - self.min_gain);
        let (time, links) =
            self.search.fastest_route_after(current, destination, now, bound, &seconds)?;
        (time < bound && !links.is_empty()).then(|| {
            self.changed += 1;
            links
        })
    }
}
