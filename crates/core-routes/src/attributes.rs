//! What a route is, beyond its links and its cost: the numbers a choice model
//! reads (S169).
//!
//! Computed for the whole store in one pass, from the routes and the network's
//! link lengths, and not stored with it: they are a function of both.
//!
//! **Path size** (Ben-Akiva and Bierlaire, 1999) measures how much of a route is
//! its own: `PS_i = Σ_{a ∈ i} (length_a / length_i) / N_a`, where `N_a` is how
//! many routes of the same set use link `a`. A route that shares none of its
//! links has 1; two identical routes have ½ each. A logit that adds `ln PS` to
//! each route's utility stops treating overlapping routes as independent
//! alternatives, which is the standard repair for route choice.
//!
//! [`path_sizes`] computes the same number over exactly the routes it is given: a choice
//! reads the path size over the routes **offered** (S279, CM-3), not over the whole stored
//! set, which includes routes nobody is offered. The store's [`RouteAttributes::path_size`]
//! stays the whole set's (reports, the interactive map's route panel).
//!
//! **Cost:** one pass over every link of every route, with a scratch counter
//! per network link (2 bytes) that is cleaned up as it goes: about 8 ns per
//! route link, so ~50 ms for Luxembourg's 53 000 routes.

use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_types::ids::{EntityId, LinkId};

use crate::store::RouteSets;

/// Per-route numbers, indexed like the store's routes.
#[derive(Clone, Debug, PartialEq)]
pub struct RouteAttributes {
    /// Each route's length in metres.
    pub length_m: Vec<f64>,
    /// Each route's path size over its **whole stored set**, in `(0, 1]`, by length. A
    /// choice among the routes offered uses [`path_sizes`] over those instead.
    pub path_size: Vec<f64>,
}

impl RouteSets {
    /// The lengths and path sizes of every route in the store.
    #[must_use]
    pub fn attributes(&self, network: &RoadNetwork) -> RouteAttributes {
        let routes = self.route_count();
        let mut length_m = vec![0.0; routes];
        let mut path_size = vec![1.0; routes];
        let mut uses = vec![0_u16; network.link_count() as usize];
        for key in 0..self.keys().len() {
            let range = self.route_range(key);
            for r in range.clone() {
                let route = self.route(r);
                let mut total = 0.0;
                for &l in route.links {
                    total += network.link_length(LinkId::new(l)).get();
                    uses[l as usize] = uses[l as usize].saturating_add(1);
                }
                length_m[r] = total;
            }
            if range.len() > 1 {
                for r in range.clone() {
                    let route = self.route(r);
                    if length_m[r] > 0.0 {
                        let mut shared = 0.0;
                        for &l in route.links {
                            shared += network.link_length(LinkId::new(l)).get()
                                / f64::from(uses[l as usize]);
                        }
                        path_size[r] = shared / length_m[r];
                    }
                }
            }
            for r in range {
                for &l in self.route(r).links {
                    uses[l as usize] = 0;
                }
            }
        }
        RouteAttributes { length_m, path_size }
    }
}

/// The path size of each of `routes` over exactly these routes (Ben-Akiva and Bierlaire,
/// 1999): `PS_i = Σ_{a ∈ i} (length_a / length_i) / N_a`, `N_a` the number of the given
/// routes that use link `a`, **by length** (a link a route uses twice counts twice, as in
/// the store). A choice set's path size is the one over the routes the traveller is
/// offered; the project extends it to itineraries by time share
/// (`itinerary_choice`). A route of no length, or a single route, has 1. The sums run in the
/// order of each route's links, as the store's [`RouteAttributes::path_size`] does, so over
/// the whole set both agree to the last bit.
///
/// `link_length` gives a link's length in metres. **Cost:** one sort of the routes' links.
#[must_use]
pub fn path_sizes(routes: &[&[u32]], link_length: impl Fn(u32) -> f64) -> Vec<f64> {
    let mut size = vec![1.0; routes.len()];
    if routes.len() < 2 {
        return size;
    }
    // Every link occurrence of every route, in route order; `users[o]` is how many
    // occurrences of the same link the routes hold in all.
    let total: usize = routes.iter().map(|r| r.len()).sum();
    let mut order: Vec<(u32, u32)> = Vec::with_capacity(total);
    let mut length = Vec::with_capacity(total);
    for r in routes {
        for &l in *r {
            order.push((l, u32::try_from(order.len()).unwrap_or(u32::MAX)));
            length.push(link_length(l));
        }
    }
    order.sort_unstable();
    let mut users = vec![1.0_f64; total];
    let mut from = 0;
    while from < order.len() {
        let mut to = from + 1;
        while to < order.len() && order[to].0 == order[from].0 {
            to += 1;
        }
        #[allow(clippy::cast_precision_loss, reason = "a handful of routes")]
        let n = (to - from) as f64;
        for &(_, o) in &order[from..to] {
            users[o as usize] = n;
        }
        from = to;
    }
    let mut at = 0;
    for (i, r) in routes.iter().enumerate() {
        let (mut sum, mut shared) = (0.0, 0.0);
        for o in at..at + r.len() {
            sum += length[o];
            shared += length[o] / users[o];
        }
        at += r.len();
        if sum > 0.0 {
            size[i] = shared / sum;
        }
    }
    size
}
