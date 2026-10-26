//! What a route is, beyond its links and its cost: the numbers a choice model
//! reads.
//!
//! A route's **length** is kept by the store when the route is added
//! ([`RouteSets::lengths_m`](crate::RouteSets::lengths_m)); a length never changes.
//!
//! **Path size** (Ben-Akiva and Bierlaire, 1999) measures how much of a route is
//! its own: `PS_i = Σ_{a ∈ i} (length_a / length_i) / N_a`, where `N_a` is how
//! many routes of the same set use link `a`. A route that shares none of its
//! links has 1; two identical routes have ½ each. A logit that adds `ln PS` to
//! each route's utility stops treating overlapping routes as independent
//! alternatives, which is the standard repair for route choice.
//!
//! [`path_sizes`] computes it over exactly the routes it is given: a choice
//! reads the path size over the routes **offered**, not over the whole stored
//! set, which includes routes nobody is offered.

/// The path size of each of `routes` over exactly these routes (Ben-Akiva and Bierlaire,
/// 1999): `PS_i = Σ_{a ∈ i} (length_a / length_i) / N_a`, `N_a` the number of the given
/// routes that use link `a`, **by length** (a link a route uses twice counts twice). A choice set's path size is the one over the routes the traveller is
/// offered; the project extends it to itineraries by time share
/// (`itinerary_choice`). A route of no length, or a single route, has 1. The sums run in the
/// order of each route's links.
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

/// [`path_sizes`], counting each link's users in `users`, a scratch indexed by link that grows
/// as needed and is left all zeros: the same numbers to the last bit (each link's count is the
/// same integer, and the sums run in the same order), without the sort. For a caller costing
/// many sets in a row (a route choice, S294).
///
/// **Cost:** two passes over the routes' links; `users` holds 4 bytes per link up to the
/// largest seen.
#[must_use]
pub fn path_sizes_counted(
    routes: &[&[u32]],
    link_length: impl Fn(u32) -> f64,
    users: &mut Vec<u32>,
) -> Vec<f64> {
    let mut size = vec![1.0; routes.len()];
    if routes.len() < 2 {
        return size;
    }
    for r in routes {
        for &l in *r {
            let l = l as usize;
            if l >= users.len() {
                users.resize(l + 1, 0);
            }
            users[l] += 1;
        }
    }
    for (i, r) in routes.iter().enumerate() {
        let (mut sum, mut shared) = (0.0, 0.0);
        for &l in *r {
            let length = link_length(l);
            sum += length;
            shared += length / f64::from(users[l as usize]);
        }
        if sum > 0.0 {
            size[i] = shared / sum;
        }
    }
    for r in routes {
        for &l in *r {
            users[l as usize] = 0;
        }
    }
    size
}
