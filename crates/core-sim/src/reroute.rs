//! En-route rerouting's decision (S213): what a vehicle stuck at the front of its link does,
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
//! **Cost:** one bounded search per offer; offers come only to vehicles blocked for
//! [`LoadingOptions::reroute_after_s`], at most [`LoadingOptions::reroute_max`] taken per trip.

use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_loading::{LiveTimes, Reroute};
use openmobisim_core_routes::{Search, SearchContext};
use openmobisim_core_types::ids::{EntityId, LinkId, VehicleId};

use crate::link_times::LinkTimes;
use crate::loading_rules::LoadingOptions;

/// Decides reroutes for one loading.
pub(crate) struct Rerouter<'a> {
    network: &'a RoadNetwork,
    search: Search<'a>,
    expected: Option<&'a LinkTimes>,
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
    ) -> Self {
        Self {
            network: ctx.network,
            search: Search::new(ctx),
            expected,
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
    ) -> Option<Vec<LinkId>> {
        if vehicle.raw() >= self.cars_below {
            return None;
        }
        self.offers += 1;
        let expected = self.expected;
        let seconds = |l: u32, t: f64| {
            let known = expected.map_or(0.0, |e| e.link_seconds(l, t));
            known.max(live.live_seconds(LinkId::new(l)))
        };
        let mut t = now;
        for l in planned {
            t += seconds(l.raw(), t);
        }
        let planned_s = t - now;
        let bound = planned_s * (1.0 - self.min_gain);
        let destination = self.network.link_to(*planned.last()?);
        let (time, links) =
            self.search.fastest_route_after(current, destination, now, bound, &seconds)?;
        (time < bound && !links.is_empty()).then(|| {
            self.changed += 1;
            links
        })
    }
}
