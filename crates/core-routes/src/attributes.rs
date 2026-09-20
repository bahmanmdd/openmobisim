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
    /// Each route's path size, in `(0, 1]`.
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
