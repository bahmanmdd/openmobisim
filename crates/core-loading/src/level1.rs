//! Level 1: the static user-equilibrium assignment (S294).
//!
//! The rung of the fidelity ladder between free flow (level 0) and the point queue (level 2):
//! **no time inside the period**. The demand is one origin–destination table of hourly flows,
//! every link carries one flow for the whole period, and a link's time is a function of its
//! flow alone, the volume-delay function of the Bureau of Public Roads ([`Bpr`]):
//! `t = t0 · (1 + α (v/c)^β)`. The assignment is the deterministic user equilibrium of Wardrop
//! (1952): every used route of a pair has the least time of the pair's routes. Two solvers
//! ([`StaticSolver`]):
//!
//! - **`Msa`**, the method of successive averages: the all-or-nothing flows on the current
//!   times, averaged in with step `1/(k + 1)` at iteration `k` (Sheffi 1985, *Urban
//!   Transportation Networks*, §12.2);
//! - **`FrankWolfe`** (Frank and Wolfe 1956; LeBlanc, Morlok and Pierskalla 1975): the same
//!   direction, with the step that minimises the Beckmann objective along it, found by
//!   bisection on the objective's derivative (the BPR integral is convex, so the derivative
//!   crosses zero once).
//!
//! Both start from the all-or-nothing assignment on free-flow times (iteration 0) and stop at
//! a **relative gap** below the tolerance or at the iteration cap. The gap is
//! `(Σ_a x_a t_a − Σ_od d_od t_min,od) / Σ_od d_od t_min,od`: how much more the flows cost
//! than everyone on the shortest route at the current times, as a share of that; 0 at
//! equilibrium (Boyce, Ralevic-Dekic and Bar-Gera 2004, *Convergence of traffic assignments:
//! how much is enough?*, *Journal of Transportation Engineering* 130(1)).
//!
//! **What is kept besides the link flows:** the route flows of every pair, carried through the
//! iterations (each move scales a pair's route flows by `1 − λ` and adds `λ · d` to the
//! all-or-nothing route), so that a trip can be given one route of its pair with the share of
//! the pair's flow that route carries ([`apportion`]). The decomposition of link flows into
//! route flows is not unique in general; this one is the solver's own, deterministic for a given
//! network and demand.
//!
//! **Determinism.** The shortest-path trees are searched in parallel over origins in fixed
//! chunks ([`openmobisim_core_routes::search_map`]) and every sum is taken serially in link or
//! origin order, so the result is the same for any thread count. **Cost.** Per iteration one
//! Dijkstra tree per origin (the dominant cost on a large network: aggregate the demand to its
//! zones first), one pass over the links for the times and the gap, one pass over every pair's
//! routes for the route flows, and for Frank–Wolfe up to 60 passes over the links for the step.
//!
//! Bikes and the other modes are not assigned here: level 1 is the car layer's static
//! assignment; bike and walk trips keep their own layers, as at level 0.

use std::cmp::Ordering;
use std::collections::HashMap;

use openmobisim_core_graph::defaults::RoadClass;
use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_routes::{Route, SearchContext, search_map};
use openmobisim_core_types::ids::{EntityId, LinkId, NodeId};

/// How many road classes there are: one BPR pair per class.
pub const CLASS_COUNT: usize = RoadClass::ALL.len();

/// The volume-delay function of the Bureau of Public Roads, per road class:
/// `t = t0 · (1 + α (v/c)^β)`, `t0` the free-flow time, `v` the hourly flow, `c` the hourly
/// capacity.
///
/// *Uncalibrated: α 0.15 and β 4 for every class, the values of the Bureau of Public Roads
/// (1964, *Traffic Assignment Manual*, U.S. Department of Commerce, Urban Planning Division),
/// which TNTP's benchmark networks use. The table is keyed by class so that a class can be given
/// its own pair.*
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bpr {
    /// α per road class, in [`RoadClass::ALL`]'s order.
    pub alpha: [f64; CLASS_COUNT],
    /// β per road class, in [`RoadClass::ALL`]'s order.
    pub beta: [f64; CLASS_COUNT],
}

impl Bpr {
    /// The shipped table: α 0.15 and β 4 for every class.
    pub const SHIPPED: Bpr = Bpr { alpha: [0.15; CLASS_COUNT], beta: [4.0; CLASS_COUNT] };

    /// The time of a link of `class` carrying `flow` (PCU/h) against `capacity` (PCU/h), from
    /// its free-flow time `t0` (seconds); `t0` itself at a flow of 0, infinite at a capacity
    /// of 0 with any flow.
    #[inline]
    #[must_use]
    pub fn time(&self, class: RoadClass, t0: f64, flow: f64, capacity: f64) -> f64 {
        if flow <= 0.0 {
            return t0;
        }
        if capacity <= 0.0 {
            return f64::INFINITY;
        }
        let i = class as usize;
        t0 * (1.0 + self.alpha[i] * (flow / capacity).powf(self.beta[i]))
    }

    /// `Some((α, β))` if every class has the same pair, else `None`.
    #[must_use]
    pub fn uniform(&self) -> Option<(f64, f64)> {
        let (a, b) = (self.alpha[0], self.beta[0]);
        (self.alpha.iter().all(|&x| x.to_bits() == a.to_bits())
            && self.beta.iter().all(|&x| x.to_bits() == b.to_bits()))
        .then_some((a, b))
    }
}

impl Default for Bpr {
    fn default() -> Self {
        Self::SHIPPED
    }
}

/// Which solver finds the equilibrium.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StaticSolver {
    /// The method of successive averages: step `1/(k + 1)` at iteration `k`.
    Msa,
    /// Frank–Wolfe: the step that minimises the Beckmann objective along the direction.
    FrankWolfe,
}

/// What the solver is given besides the network and the demand.
#[derive(Clone, Debug, PartialEq)]
pub struct StaticOptions {
    /// The volume-delay table.
    pub bpr: Bpr,
    /// The solver.
    pub solver: StaticSolver,
    /// The most iterations (flow patterns assessed), at least 1: with 1, the all-or-nothing
    /// assignment on free-flow times, its gap measured.
    pub max_iterations: u32,
    /// Stop once the relative gap is below this; 0 never stops early.
    pub tolerance: f64,
    /// Capacity factors in force for the whole period (a road disruption): 0 closes the link.
    pub capacity_factor: Vec<(LinkId, f64)>,
}

/// The origin–destination table: per origin node, its destinations with their hourly flow in
/// PCU/h. A destination equal to its origin, a repeated destination (its flow is added to the
/// first) or a flow that is not above 0 is not assigned (see [`StaticResult::routes`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StaticDemand {
    /// `(origin, [(destination, flow)])`, in the order the pairs are numbered.
    pub origins: Vec<(NodeId, Vec<(NodeId, f64)>)>,
}

impl StaticDemand {
    /// How many pairs the table has (the length of [`StaticResult::routes`]).
    #[must_use]
    pub fn pair_count(&self) -> usize {
        self.origins.iter().map(|(_, d)| d.len()).sum()
    }
}

/// What one iteration showed: the flows assessed at iteration `iteration`, and how they were
/// reached.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StaticIteration {
    /// 0 is the all-or-nothing assignment on free-flow times.
    pub iteration: u32,
    /// The step λ that made these flows from the last (1 at iteration 0).
    pub step: f64,
    /// Σ flow × time over the links, in PCU/h × seconds (times the period in hours: PCU·s).
    pub total_time: f64,
    /// Σ demand × shortest-route time at these times, in the same unit.
    pub shortest_time: f64,
    /// The relative gap, `(total_time − shortest_time) / shortest_time`.
    pub gap: f64,
    /// Σ flow × |time − last time| / Σ flow × last time: how much the link times moved since
    /// the last iteration; `NaN` at iteration 0.
    pub time_change: f64,
}

/// What the solver found.
#[derive(Clone, Debug, PartialEq)]
pub struct StaticResult {
    /// The equilibrium flow of every link, PCU/h.
    pub flows: Vec<f64>,
    /// The time of every link at those flows, seconds: the free-flow time where nothing drives,
    /// infinite on a link a car cannot use or a closed one.
    pub times_s: Vec<f64>,
    /// Every pair's routes with their flow, PCU/h, by the pair's number in the demand (in the
    /// order they were first found, the free-flow route first). A pair not assigned (its
    /// origin is its destination, it repeats an earlier pair, or its flow is not above 0) and a
    /// pair with no route have none.
    pub routes: Vec<Vec<(Vec<LinkId>, f64)>>,
    /// Every iteration, in order.
    pub iterations: Vec<StaticIteration>,
    /// Whether the final relative gap is below the tolerance (never with a tolerance of 0).
    pub converged: bool,
    /// The pairs (by number) the network has no route for; their flow is not assigned.
    pub unreachable: Vec<usize>,
}

/// One pair the solver assigns: where it sits in the demand, its destination and its flow.
struct Pair {
    index: usize,
    destination: NodeId,
    flow: f64,
}

/// The equilibrium assignment of `demand` on `network`. See the [module docs](self).
///
/// # Panics
///
/// Panics if `options.max_iterations` is 0, or a capacity factor names a link the network
/// does not have.
#[must_use]
pub fn solve_static(
    network: &RoadNetwork,
    turns: &TurnTable,
    demand: &StaticDemand,
    options: &StaticOptions,
) -> StaticResult {
    assert!(options.max_iterations >= 1, "at least one iteration");
    let n = network.link_count() as usize;
    // Per link: the capacity in force (PCU/h), the free-flow time and the class; a link a car
    // cannot use, a closed one or one without capacity is never searched.
    let mut capacity: Vec<f64> = (0..n)
        .map(|i| network.link_parameters(LinkId::from_index(i)).capacity.get() * 3600.0)
        .collect();
    for &(link, factor) in &options.capacity_factor {
        capacity[link.index()] *= factor;
    }
    let usable: Vec<bool> =
        (0..n).map(|i| network.is_drivable(LinkId::from_index(i)) && capacity[i] > 0.0).collect();
    let t0: Vec<f64> =
        (0..n).map(|i| network.free_flow_time(LinkId::from_index(i)).get()).collect();
    let class: Vec<RoadClass> = (0..n).map(|i| network.link_class(LinkId::from_index(i))).collect();
    let times_of = |flows: &[f64]| -> Vec<f64> {
        (0..n)
            .map(|i| {
                if usable[i] {
                    options.bpr.time(class[i], t0[i], flows[i], capacity[i])
                } else {
                    f64::INFINITY
                }
            })
            .collect()
    };

    // The pairs assigned: per origin its distinct destinations (not the origin itself) with a
    // flow above 0, a repeated destination's flow added to its first.
    let mut origins: Vec<(NodeId, Vec<Pair>)> = Vec::with_capacity(demand.origins.len());
    let mut index = 0_usize;
    for (o, dests) in &demand.origins {
        let mut first: HashMap<u32, usize> = HashMap::new();
        let mut kept: Vec<Pair> = Vec::new();
        for &(d, flow) in dests {
            if d != *o && flow > 0.0 {
                match first.get(&d.raw()) {
                    Some(&k) => kept[k].flow += flow,
                    None => {
                        first.insert(d.raw(), kept.len());
                        kept.push(Pair { index, destination: d, flow });
                    }
                }
            }
            index += 1;
        }
        if !kept.is_empty() {
            origins.push((*o, kept));
        }
    }
    let mut routes: Vec<Vec<(Vec<LinkId>, f64)>> = vec![Vec::new(); demand.pair_count()];

    // All-or-nothing on `times`: the link flows, Σ d × t_min, and each assigned pair's route
    // (in the order of `origins`, then of their pairs).
    let all_or_nothing = |times: &[f64]| {
        let ctx = SearchContext::with_costs(network, turns, times.to_vec());
        let trees: Vec<Vec<Option<Route>>> = search_map(&ctx, &origins, |search, (o, pairs)| {
            let destinations: Vec<NodeId> = pairs.iter().map(|p| p.destination).collect();
            search.shortest_tree(*o, &destinations, f64::INFINITY)
        });
        let mut y = vec![0.0_f64; n];
        let mut shortest = 0.0_f64;
        let mut found: Vec<Option<Vec<LinkId>>> = Vec::new();
        for (tree, (_, pairs)) in trees.into_iter().zip(&origins) {
            for (route, pair) in tree.into_iter().zip(pairs) {
                if let Some(r) = &route {
                    for &l in &r.links {
                        y[l.index()] += pair.flow;
                    }
                    shortest += pair.flow * r.cost;
                }
                found.push(route.map(|r| r.links));
            }
        }
        (y, shortest, found)
    };
    // Every assigned pair, in the order `all_or_nothing` returns their routes.
    let pairs: Vec<&Pair> = origins.iter().flat_map(|(_, p)| p).collect();
    // Move the route flows of every pair: scale by 1 − λ, add λ·d to the all-or-nothing route.
    let move_routes =
        |routes: &mut Vec<Vec<(Vec<LinkId>, f64)>>, found: Vec<Option<Vec<LinkId>>>, step: f64| {
            for (pair, route) in pairs.iter().zip(found) {
                let Some(links) = route else { continue };
                let set = &mut routes[pair.index];
                for (_, f) in set.iter_mut() {
                    *f *= 1.0 - step;
                }
                let add = step * pair.flow;
                match set.iter_mut().find(|(r, _)| *r == links) {
                    Some((_, f)) => *f += add,
                    None => set.push((links, add)),
                }
            }
        };

    // Iteration 0: everyone on the free-flow route. A pair without one is unreachable at any
    // flow (a link's time only grows with its flow; unusable links are never searched).
    let (mut x, _, found) = all_or_nothing(&times_of(&vec![0.0; n]));
    let unreachable: Vec<usize> =
        pairs.iter().zip(&found).filter(|(_, r)| r.is_none()).map(|(p, _)| p.index).collect();
    move_routes(&mut routes, found, 1.0);

    let mut iterations: Vec<StaticIteration> = Vec::new();
    let mut step = 1.0;
    let mut last_times: Option<Vec<f64>> = None;
    let mut converged = false;
    let mut k = 0_u32;
    let times = loop {
        let times = times_of(&x);
        let (y, shortest, found) = all_or_nothing(&times);
        let total: f64 = (0..n).filter(|&i| x[i] > 0.0).map(|i| x[i] * times[i]).sum();
        let gap = if shortest > 0.0 { (total - shortest) / shortest } else { 0.0 };
        let time_change = last_times.as_ref().map_or(f64::NAN, |last| {
            let moved: f64 =
                (0..n).filter(|&i| x[i] > 0.0).map(|i| x[i] * (times[i] - last[i]).abs()).sum();
            let was: f64 = (0..n).filter(|&i| x[i] > 0.0).map(|i| x[i] * last[i]).sum();
            if was > 0.0 { moved / was } else { 0.0 }
        });
        iterations.push(StaticIteration {
            iteration: k,
            step,
            total_time: total,
            shortest_time: shortest,
            gap,
            time_change,
        });
        if options.tolerance > 0.0 && gap < options.tolerance {
            converged = true;
            break times;
        }
        if k + 1 >= options.max_iterations {
            break times;
        }
        let direction: Vec<f64> = (0..n).map(|i| y[i] - x[i]).collect();
        step = match options.solver {
            StaticSolver::Msa => 1.0 / (f64::from(k) + 2.0),
            StaticSolver::FrankWolfe => line_search(|lambda| {
                (0..n)
                    .filter(|&i| direction[i] != 0.0)
                    .map(|i| {
                        let flow = x[i] + lambda * direction[i];
                        direction[i] * options.bpr.time(class[i], t0[i], flow, capacity[i])
                    })
                    .sum()
            }),
        };
        for i in 0..n {
            x[i] += step * direction[i];
        }
        move_routes(&mut routes, found, step);
        last_times = Some(times);
        k += 1;
    };
    StaticResult { flows: x, times_s: times, routes, iterations, converged, unreachable }
}

/// The step in `[0, 1]` that minimises the Beckmann objective along a direction, given the
/// objective's derivative `slope(λ)` (non-decreasing in λ): 1 if the derivative is still
/// negative there, else its zero by bisection, 60 halvings at most or a bracket below 1e-14.
fn line_search(slope: impl Fn(f64) -> f64) -> f64 {
    if slope(1.0) <= 0.0 {
        return 1.0;
    }
    let (mut lo, mut hi) = (0.0_f64, 1.0_f64);
    for _ in 0..60 {
        let mid = 0.5 * (lo + hi);
        if slope(mid) > 0.0 {
            hi = mid;
        } else {
            lo = mid;
        }
        if hi - lo < 1e-14 {
            break;
        }
    }
    0.5 * (lo + hi)
}

/// Give each of `count` trips, in order, one of a pair's routes so that route `r` gets as near
/// to `shares[r] / Σ shares × count` trips as whole trips allow: the largest-remainder
/// (Hamilton) apportionment of `count` among the shares, ties to the earlier route, the trips
/// given to the routes in route order. Returns each trip's route index; empty if there is no
/// route, no trip or no share above 0. Trips are counted, not weighed.
#[must_use]
pub fn apportion(shares: &[f64], count: usize) -> Vec<usize> {
    let total: f64 = shares.iter().sum();
    if shares.is_empty() || count == 0 || total.partial_cmp(&0.0) != Some(Ordering::Greater) {
        return Vec::new();
    }
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss,
        reason = "counts of trips"
    )]
    let quotas: Vec<usize> = {
        let exact: Vec<f64> = shares.iter().map(|&s| s.max(0.0) / total * count as f64).collect();
        let mut whole: Vec<usize> = exact.iter().map(|q| q.floor() as usize).collect();
        let given: usize = whole.iter().sum();
        let mut left = count.saturating_sub(given);
        // The largest remainders first; ties to the earlier route.
        let mut order: Vec<usize> = (0..shares.len()).collect();
        order.sort_by(|&a, &b| {
            let ra = exact[a] - exact[a].floor();
            let rb = exact[b] - exact[b].floor();
            rb.total_cmp(&ra).then(a.cmp(&b))
        });
        for &r in order.iter().cycle() {
            if left == 0 {
                break;
            }
            whole[r] += 1;
            left -= 1;
        }
        whole
    };
    let mut out = Vec::with_capacity(count);
    for (r, &q) in quotas.iter().enumerate() {
        out.extend(std::iter::repeat_n(r, q));
    }
    out.truncate(count);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bpr_is_the_free_flow_time_at_no_flow_and_the_formula_above_it() {
        let bpr = Bpr::SHIPPED;
        assert!((bpr.time(RoadClass::Primary, 60.0, 0.0, 1000.0) - 60.0).abs() < 1e-12);
        let t = bpr.time(RoadClass::Primary, 60.0, 1000.0, 1000.0);
        assert!((t - 69.0).abs() < 1e-12, "{t}: 60 × (1 + 0.15) at v/c 1");
        let t = bpr.time(RoadClass::Primary, 60.0, 2000.0, 1000.0);
        assert!((t - 60.0 * (1.0 + 0.15 * 16.0)).abs() < 1e-9, "{t}: v/c 2 costs 2.4 t0 extra");
        assert!(bpr.time(RoadClass::Primary, 60.0, 1.0, 0.0).is_infinite());
        assert_eq!(bpr.uniform(), Some((0.15, 4.0)));
        let mut own = bpr;
        own.beta[RoadClass::Motorway as usize] = 6.0;
        assert_eq!(own.uniform(), None);
        // A class's own pair acts on its links alone.
        let t = own.time(RoadClass::Motorway, 60.0, 2000.0, 1000.0);
        assert!((t - 60.0 * (1.0 + 0.15 * 64.0)).abs() < 1e-9, "{t}");
        assert!((own.time(RoadClass::Primary, 60.0, 2000.0, 1000.0) - 204.0).abs() < 1e-9);
    }

    #[test]
    fn the_line_search_finds_the_zero_of_an_increasing_slope() {
        // slope(λ) = λ − 0.25: zero at 0.25.
        let step = line_search(|l| l - 0.25);
        assert!((step - 0.25).abs() < 1e-12, "{step}");
        assert!((line_search(|l| l - 2.0) - 1.0).abs() < f64::EPSILON, "still descending at 1");
    }

    #[test]
    fn apportionment_gives_whole_trips_nearest_the_shares_in_route_order() {
        assert_eq!(apportion(&[0.5, 0.5], 3), vec![0, 0, 1], "the earlier route takes the odd");
        assert_eq!(apportion(&[0.1, 0.9], 10), vec![0, 1, 1, 1, 1, 1, 1, 1, 1, 1]);
        assert_eq!(apportion(&[2.0, 1.0], 1), vec![0]);
        assert_eq!(apportion(&[], 4), Vec::<usize>::new());
        assert_eq!(apportion(&[1.0], 0), Vec::<usize>::new());
        assert_eq!(apportion(&[0.0, 0.0], 3), Vec::<usize>::new());
        let many = apportion(&[0.333, 0.333, 0.334], 100);
        assert_eq!(many.len(), 100);
        assert_eq!(many.iter().filter(|&&r| r == 2).count(), 34);
        // A share worn down to almost nothing gets no trip.
        assert_eq!(apportion(&[999.999, 0.001], 7), vec![0; 7]);
    }
}
