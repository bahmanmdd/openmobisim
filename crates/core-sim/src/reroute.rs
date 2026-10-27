//! En-route rerouting's decision: what a vehicle offered a new route does, asked by the loading
//! ([`openmobisim_core_loading::Reroute`]).
//!
//! **The costs a vehicle sees:** each link's time is the larger of what the trip planned on —
//! the time expected from the last loading ([`LinkTimes`]; free flow at the first) — and the
//! loading's **live** estimate now (free-flow time, the queue's clearance time, how long the
//! link's front has been blocked, the wait until a closed link reopens;
//! [`openmobisim_core_loading::LiveTimes`]). So a vehicle avoids what it can see jammed now and
//! what it knows is usually slow.
//!
//! **The decision:** the rest of its planned route is timed on those costs; the earliest
//! arrival from where it is, through its legal turns (or any link out of its origin), is found;
//! a route that beats the plan by at least [`LoadingOptions::reroute_min_gain`] is taken. Buses
//! never re-route: their line is their route.
//!
//! **One tree per junction (S300):** the vehicles offered a route where they meet a closed link
//! all leave the same place, so one earliest-arrival tree from there, grown for the first,
//! answers each by its own destination for [`LoadingOptions::reroute_tree_s`] seconds, on the
//! link times of the moment it was grown.
//!
//! **By the route cost** (S299): where the run's route cost is not time alone, a car is
//! re-routed by its trip's coefficient vector, one search per offer: the plan's cost and the
//! search's are the time on those costs plus the links' extras
//! ([`openmobisim_core_routes::Search::cheapest_route_after`]), and a route beating the plan's
//! generalised cost by the required gain is taken.
//!
//! **Cost:** one tree per junction and `reroute_tree_s` (a search over the network, kept as a
//! map of the links it reached), or one bounded search per offer by generalised cost; offers come
//! only to vehicles held, at most [`LoadingOptions::reroute_max`] taken per trip.

use std::collections::HashMap;

use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_loading::{LiveTimes, Reroute, RerouteFrom, RerouteReason};
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

/// One earliest-arrival tree from where vehicles are offered routes: grown at `at`, each link it
/// reached with its arrival (a clock time) and the link before it (`u32::MAX` for a first link).
struct Tree {
    from: RerouteFrom,
    at: f64,
    reached: HashMap<u32, (f64, u32)>,
}

/// Decides reroutes for one loading.
pub(crate) struct Rerouter<'a> {
    network: &'a RoadNetwork,
    search: Search<'a>,
    expected: Option<&'a LinkTimes>,
    cost: Option<&'a RerouteCost>,
    min_gain: f64,
    tree_s: f64,
    trees: Vec<Tree>,
    /// Vehicles with an id below this are trips' cars; the rest are buses.
    cars_below: u32,
    /// Offers answered, searches made (trees grown and searches by generalised cost), and
    /// routes changed.
    pub offers: u32,
    pub searches: u32,
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
            tree_s: options.reroute_tree_s,
            trees: Vec::new(),
            cars_below,
            offers: 0,
            searches: 0,
            changed: 0,
        }
    }

    /// The tree from `from` grown at most `tree_s` seconds before `now`, or a new one grown now.
    fn tree(&mut self, from: RerouteFrom, now: f64, seconds: &dyn Fn(u32, f64) -> f64) -> usize {
        let fresh = |t: &Tree| t.from == from && now >= t.at && now - t.at <= self.tree_s;
        if let Some(k) = self.trees.iter().position(fresh) {
            return k;
        }
        self.searches += 1;
        let settled = match from {
            RerouteFrom::EndOf(link) => {
                self.search.fastest_tree_after(link, now, f64::INFINITY, seconds)
            }
            RerouteFrom::Origin(node) => {
                self.search.fastest_tree_from(node, now, f64::INFINITY, seconds)
            }
        };
        let tree = Tree {
            from,
            at: now,
            reached: settled.into_iter().map(|(l, at, pred)| (l, (at, pred))).collect(),
        };
        match self.trees.iter().position(|t| t.from == from) {
            Some(k) => {
                self.trees[k] = tree;
                k
            }
            None => {
                self.trees.push(tree);
                self.trees.len() - 1
            }
        }
    }
}

impl Reroute for Rerouter<'_> {
    fn reroute(
        &mut self,
        vehicle: VehicleId,
        from: RerouteFrom,
        planned: &[LinkId],
        now: f64,
        live: &dyn LiveTimes,
        _reason: RerouteReason,
    ) -> Option<Vec<LinkId>> {
        // Every driver re-routes the same way, whatever the reason. An informed share and
        // alerts (S215) would decide here.
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
        // By the trip's vector's generalised cost where it has extras (S299): one search.
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
            self.searches += 1;
            self.search.clear_route_state();
            self.search.set_extras(Some(e.clone()));
            let found = match from {
                RerouteFrom::EndOf(link) => {
                    self.search.cheapest_route_after(link, destination, now, bound, &seconds)
                }
                RerouteFrom::Origin(node) => {
                    self.search.cheapest_route_from(node, destination, now, bound, &seconds)
                }
            };
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
        let bound = (t - now) * (1.0 - self.min_gain);
        // The earliest arrival at the destination in the junction's tree, and its way there.
        let k = self.tree(from, now, &seconds);
        let tree = &self.trees[k];
        let (arrival, last) = self
            .network
            .in_links(destination)
            .iter()
            .filter_map(|l| tree.reached.get(&l.raw()).map(|&(at, _)| (at, l.raw())))
            .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)))?;
        if arrival - tree.at >= bound {
            return None;
        }
        let mut links = Vec::new();
        let mut at = last;
        while at != u32::MAX {
            links.push(LinkId::new(at));
            at = tree.reached.get(&at).map_or(u32::MAX, |&(_, pred)| pred);
        }
        links.reverse();
        self.changed += 1;
        Some(links)
    }
}
