//! The quota allocation: a group's travellers allocated in the choice model's proportions.
//!
//! **Why.** In the classical method of successive averages, applied to flows, the flow of a
//! group of `N` trips on chain `k` moves as `x_k ← (1 − α) x_k + α N P_k` exactly, `P_k` the
//! choice model's probability of the chain and `α` the averaging step. Applied per traveller
//! with random draws (the [`Allocation::Draws`] of the chooser), it does so only in expectation:
//! who reconsiders is a draw (about `α N` of the group, binomially), and what each re-chooser
//! takes is a draw from `P`. The quota removes both draws while keeping the per-traveller form.
//!
//! **The group.** Trips face the same choice when they have the same user class, the same choice
//! set (the same chains, as a set, whatever the order each trip found them in) and their
//! vehicles set off in the same bin of the link travel times the costs are read from (the last
//! loading's bins, 300 s by default; one second for the free-flow table of the first
//! iteration; a car sets off its access time after the trip's departure, the same for every
//! trip of a group, whose chains are the same): they read the same costs, up to where in the
//! bin each sets off. A group's trips are taken in trip order,
//! and its chains in the order of its first trip's set; no link or node id decides anything.
//!
//! **Form 1, the choice quota** ([`pick`]). Among a group's `N` re-choosers of an iteration, the
//! `n`-th (from 0, in trip order) takes the chain at which its cumulative probability, the chains
//! taken in the group's order, first passes `u_n = (σ_N(n) + φ) / N`, with `φ` the group's
//! choice turn in assignment iteration `i` ([`choice_turn`]) and
//! `σ_N(n) = (s_N(n) + ⌊θ N⌋) mod N` a permutation of `0 … N − 1` ([`ranks`]): `s_N` spreads
//! neighbours in trip order apart ([`spread`]) and the rotation by `⌊θ N⌋`, `θ` the group's
//! rotation turn ([`rotation_turn`]), moves which trip receives which point from one iteration
//! to the next: no random draw. **The turns** are fractional parts of `i · a + g · b`, `g` the
//! index of the group's first trip and `a`, `b` square roots less their whole part, one pair per
//! turn: they move from one iteration to the next (so a group of one re-chooser is still spread
//! over its chains in their proportions over the iterations), and they differ from one group to
//! the next in the same iteration. The second part matters as much as the first: with one turn
//! for every group of an iteration, every group of one trip reconsiders in the same iterations
//! (form 2) and every lone re-chooser takes the same point (form 1), so on a network of many
//! small groups (DOTNET: most groups hold one trip) everyone moves at once and the averaging is
//! lost (measured: the mean trip 46 % above that of the draws after 200 iterations). The
//! rotation and the spread matter too. With the points in trip order
//! (`σ_N(n) = n`) the first re-choosers of a bin would always take the first chains and the last
//! ones the last chains, so over the iterations a bin's early departures gather on one route and
//! its late ones on another (measured on the two-route bottleneck: 88 % against 11 % of the two
//! halves of a bin on the short route), which moves the queue from one bin to the next; the
//! spread alone leaves the first re-chooser of a group always at the lowest point (`s_N(0) = 0`),
//! which late in a run, with one to three re-choosers a group, is still a bias (75 % against
//! 57 %). With the rotation each trip's point is spread over `[0, 1)` across the iterations, so
//! each trip takes a chain in its model's proportion. When the
//! group's trips have the same probabilities (they read the same costs) and every `N P_k` is
//! whole, chain `k` receives exactly `N P_k` trips; otherwise each chain receives `N P_k`
//! rounded up or down. Each trip reads its own probabilities at the group's point: where the members'
//! probabilities differ a little (a trip departing late in a bin reads later bins downstream),
//! no trip ever takes a chain its own probabilities give 0, which keeps the deterministic
//! choice within its indifference band.
//!
//! **Form 2, the re-selection quota** ([`reselected`]). Among a group's `N` trips that have a
//! chain, in trip order, the `r`-th (from 0) reconsiders when
//! `⌊(r + 1) α + ψ⌋ > ⌊r α + ψ⌋`, `ψ` the group's selection turn ([`selection_turn`]): exactly
//! `⌊α N + ψ⌋` of them, that is `⌊α N⌋` or `⌈α N⌉`, spread evenly through the group, the turn
//! moving which ones from one iteration to the next. A trip with no chain always chooses.
//!
//! **A traveller's trips reconsider together** (the averaging rule's unit is the traveller). Under
//! form 2 a traveller is represented in the quota by their first trip with a chain (lowest trip
//! id): that trip is counted in its group, and the traveller's other trips reconsider exactly when
//! it does, without being counted in their own groups. With one trip per traveller (the
//! validation cases, DOTNET and Sioux Falls) every trip is counted.
//!
//! **Nothing is car-specific.** The group is defined by its chains' identities and a chain may hold
//! legs of any mode: a bicycle-only group is the trips whose sets hold the same bicycle routes, a
//! mixed group the trips whose sets hold the same car and bicycle chains.

/// How the travellers who reconsider in an iteration are selected and allocated over their
/// chains ([`crate::Params::allocation`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Allocation {
    /// Per traveller, by seeded random draws: each reconsiders with probability `α`, and a
    /// re-chooser takes the choice model's choice (a draw for the logit; for the deterministic
    /// choice, its even split of a tie). The default.
    #[default]
    Draws,
    /// Form 1: who reconsiders by the draws; what the re-choosers of a group take by the choice
    /// quota ([`pick`]), in the model's proportions.
    ChoiceQuota,
    /// Forms 1 and 2: who reconsiders by the re-selection quota ([`reselected`]) and what they
    /// take by the choice quota. No random draw at all: the seed does not enter the choice.
    Quota,
}

impl Allocation {
    /// Every allocation, in the order of its names.
    pub const ALL: [Self; 3] = [Self::Draws, Self::ChoiceQuota, Self::Quota];

    /// The allocation's name, as the Python option gives it.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Draws => "draws",
            Self::ChoiceQuota => "choice_quota",
            Self::Quota => "quota",
        }
    }

    /// The allocation named `name`, if any.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|a| a.name() == name)
    }

    /// Whether the re-choosers' chains are allocated by the choice quota.
    #[must_use]
    pub fn quota_choice(self) -> bool {
        matches!(self, Self::ChoiceQuota | Self::Quota)
    }

    /// Whether who reconsiders is selected by the re-selection quota.
    #[must_use]
    pub fn quota_selection(self) -> bool {
        matches!(self, Self::Quota)
    }
}

/// The steps of the turns: per assignment iteration (`(√5 − 1)/2`, `√2 − 1`, `√3 − 1`) and per
/// group (`√7 − 2`, `√11 − 3`, `√13 − 3`), for the choice point, the re-selection and the
/// rotation. Square roots of distinct square-free numbers are rationally independent of each
/// other and of 1, so over the iterations a group's three turns are spread over the unit cube
/// together, and in one iteration the groups' turns are spread apart: no turn follows another.
const CHOICE_STEP: f64 = 0.618_033_988_749_894_9;
const SELECTION_STEP: f64 = 0.414_213_562_373_095_1;
const ROTATION_STEP: f64 = 0.732_050_807_568_877_3;
const CHOICE_GROUP_STEP: f64 = 0.645_751_311_064_590_6;
const SELECTION_GROUP_STEP: f64 = 0.316_624_790_355_399_8;
const ROTATION_GROUP_STEP: f64 = 0.605_551_275_463_989_3;

/// `frac(iteration · a + group · b)`.
fn turn(iteration: u32, group: usize, a: f64, b: f64) -> f64 {
    #[allow(clippy::cast_precision_loss, reason = "a trip index, exact in a double")]
    let g = group as f64;
    (f64::from(iteration) * a + (g * b).fract()).fract()
}

/// The choice quota's turn `φ` in `iteration` for the group whose first member is trip
/// `group` (its index): `frac(i · (√5 − 1)/2 + g · (√7 − 2))`, in `[0, 1)`.
#[must_use]
pub fn choice_turn(iteration: u32, group: usize) -> f64 {
    turn(iteration, group, CHOICE_STEP, CHOICE_GROUP_STEP)
}

/// The choice quota's rotation turn `θ`: `frac(i · (√3 − 1) + g · (√13 − 3))`, in `[0, 1)`.
#[must_use]
pub fn rotation_turn(iteration: u32, group: usize) -> f64 {
    turn(iteration, group, ROTATION_STEP, ROTATION_GROUP_STEP)
}

/// The re-selection quota's turn `ψ`: `frac(i · (√2 − 1) + g · (√11 − 3))`, in `[0, 1)`.
#[must_use]
pub fn selection_turn(iteration: u32, group: usize) -> f64 {
    turn(iteration, group, SELECTION_STEP, SELECTION_GROUP_STEP)
}

/// Form 1's ranks of a group of `count` re-choosers in `iteration`, the group's first member
/// being trip `group`: `σ_N(n) = (s_N(n) + ⌊θ N⌋) mod N` ([`spread`], [`rotation_turn`]), a
/// permutation of `0 … count − 1`.
#[must_use]
pub fn ranks(count: usize, iteration: u32, group: usize) -> Vec<usize> {
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss,
        reason = "a share of a count of trips, in [0, count)"
    )]
    let shift =
        ((rotation_turn(iteration, group) * count as f64) as usize).min(count.saturating_sub(1));
    spread(count).into_iter().map(|r| (r + shift) % count).collect()
}

/// The spreading permutation `s_N` of form 1: `s_N(n)` is the rank of `n` when `0 … N − 1` are
/// ordered by the fractional part of `n · (√5 − 1) / 2` (ties, which do not occur in practice, by
/// `n`). Neighbours in trip order get points far apart, and every point `(m + φ) / N`,
/// `m = 0 … N − 1`, is used once, so the counts per chain are those of the points in order.
#[must_use]
pub fn spread(count: usize) -> Vec<usize> {
    #[allow(clippy::cast_precision_loss, reason = "counts of trips, exact in a double")]
    let key = |n: usize| (n as f64 * CHOICE_STEP).fract();
    let mut order: Vec<usize> = (0..count).collect();
    order.sort_by(|&a, &b| key(a).total_cmp(&key(b)).then(a.cmp(&b)));
    let mut rank = vec![0; count];
    for (r, &n) in order.iter().enumerate() {
        rank[n] = r;
    }
    rank
}

/// Form 1: the chain the re-chooser of rank `m` (its [`ranks`] rank) among `count` re-choosers
/// of a group takes, given its `probabilities` in the group's order of chains and the
/// iteration's `turn`: the first chain at which the cumulative probability passes
/// `(m + turn) / count`, as a position in `probabilities`. Rounding that leaves the sum just
/// short of the point gives the last chain of positive probability.
///
/// # Panics
///
/// Panics if `count` is 0 or no probability is positive.
#[must_use]
pub fn pick(m: usize, count: usize, turn: f64, probabilities: &[f64]) -> usize {
    assert!(count > 0, "a group has at least one re-chooser");
    #[allow(clippy::cast_precision_loss, reason = "counts of trips, exact in a double")]
    let point = (m as f64 + turn) / count as f64;
    let mut cumulative = 0.0;
    for (k, &p) in probabilities.iter().enumerate() {
        cumulative += p;
        if p > 0.0 && cumulative > point {
            return k;
        }
    }
    probabilities.iter().rposition(|&p| p > 0.0).expect("a chain of positive probability")
}

/// Form 2: whether the `r`-th trip (from 0) of a group reconsiders at step `alpha` and the
/// iteration's `turn`: `⌊(r + 1) α + turn⌋ > ⌊r α + turn⌋`. Over a group of `N` trips exactly
/// `⌊α N + turn⌋` reconsider.
#[must_use]
pub fn reselected(r: usize, alpha: f64, turn: f64) -> bool {
    #[allow(clippy::cast_precision_loss, reason = "counts of trips, exact in a double")]
    let r = r as f64;
    ((r + 1.0) * alpha + turn).floor() > (r * alpha + turn).floor()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Counts per chain of `count` re-choosers with the same `p` in an iteration of `turn`.
    fn counts(count: usize, turn: f64, p: &[f64]) -> Vec<usize> {
        let mut c = vec![0; p.len()];
        for n in 0..count {
            c[pick(n, count, turn, p)] += 1;
        }
        c
    }

    /// Derived: 10 re-choosers with P = (0.7, 0.3) give 10 · 0.7 = 7 and 10 · 0.3 = 3, whatever
    /// the turn (the points (n + φ)/10 fall seven below 0.7 and three above it).
    #[test]
    fn ten_rechoosers_at_seven_tenths_give_seven_and_three() {
        for i in 0..20 {
            assert_eq!(counts(10, choice_turn(i, 0), &[0.7, 0.3]), vec![7, 3], "iteration {i}");
        }
    }

    /// Derived: 3 re-choosers with P = (0.5, 0.5) give 2 and 1 in iteration 0 (points 0, 1/3,
    /// 2/3) and 1 and 2 in iteration 1 (φ_1 = 0.618: points 0.206, 0.539, 0.873): 3 and 3 in
    /// total.
    #[test]
    fn three_rechoosers_at_one_half_give_three_and_three_over_two_iterations() {
        let a = counts(3, choice_turn(0, 0), &[0.5, 0.5]);
        let b = counts(3, choice_turn(1, 0), &[0.5, 0.5]);
        assert_eq!(a, vec![2, 1]);
        assert_eq!(b, vec![1, 2]);
        assert_eq!([a[0] + b[0], a[1] + b[1]], [3, 3]);
    }

    /// A chain of probability 0 is never taken, even at the points where the cumulative sum
    /// stands still on it; one re-chooser a group, over many iterations, is spread in the
    /// proportions (within one in a hundred over a thousand iterations, the turn being
    /// equidistributed).
    #[test]
    fn no_chain_of_probability_zero_and_a_lone_rechooser_spread_over_iterations() {
        for i in 0..50 {
            assert_ne!(pick(0, 1, choice_turn(i, 0), &[0.5, 0.0, 0.5]), 1);
        }
        let mut c = [0_u32; 2];
        for i in 0..1000 {
            c[pick(0, 1, choice_turn(i, 0), &[0.7, 0.3])] += 1;
        }
        assert!((f64::from(c[0]) / 1000.0 - 0.7).abs() <= 0.01, "{c:?}");
    }

    /// Derived: of N = 100 trips at α = 0.13, ⌊13 + ψ⌋ = 13 reconsider for every turn ψ in
    /// [0, 1); of 7 at α = 0.5, ⌊3.5 + ψ⌋, 3 or 4.
    #[test]
    fn the_reselection_quota_takes_alpha_n_trips() {
        for i in 0..30 {
            let psi = selection_turn(i, 0);
            let n = (0..100).filter(|&r| reselected(r, 0.13, psi)).count();
            assert_eq!(n, 13, "iteration {i}, turn {psi}");
            let m = (0..7).filter(|&r| reselected(r, 0.5, psi)).count();
            assert_eq!(m, if psi < 0.5 { 3 } else { 4 }, "iteration {i}, turn {psi}");
        }
        assert!((0..5).all(|r| reselected(r, 1.0, 0.0)), "α = 1: everyone");
    }

    /// `σ_N` is a permutation, and it spreads: of 10 neighbours in trip order, the first five and
    /// the last five each take points on both sides of the middle.
    #[test]
    fn the_spread_is_a_permutation_that_mixes_early_and_late() {
        for count in [1, 2, 3, 10, 150, 601] {
            let mut r = spread(count);
            r.sort_unstable();
            assert_eq!(r, (0..count).collect::<Vec<_>>());
        }
        let r = spread(10);
        assert!(r[..5].iter().any(|&m| m < 5) && r[..5].iter().any(|&m| m >= 5), "{r:?}");
        assert!(r[5..].iter().any(|&m| m < 5) && r[5..].iter().any(|&m| m >= 5), "{r:?}");
    }

    /// Derived: with the rotation, the first re-chooser of a group of two (`s_2(0) = 0`) takes the
    /// lower point in about half of the iterations and the upper one in the others (rank 0 in
    /// 500 ± 50 of 1 000 iterations), and the ranks are a permutation in every iteration.
    #[test]
    fn the_rotation_moves_the_first_rechooser_over_the_points() {
        let low = (0..1000).filter(|&i| ranks(2, i, 0)[0] == 0).count();
        assert!((450..=550).contains(&low), "{low}");
        for i in 0..50 {
            let mut r = ranks(7, i, 0);
            r.sort_unstable();
            assert_eq!(r, (0..7).collect::<Vec<_>>());
        }
    }

    /// In one iteration the groups' turns differ: of 1 000 groups of one trip at α = 0.1, the
    /// share that reconsiders is 0.1 within 0.02 in every one of 20 iterations (with one turn
    /// for every group it would be 0 or 1).
    #[test]
    fn groups_of_one_reconsider_at_the_step_in_every_iteration() {
        for i in 0..20 {
            let n = (0..1000).filter(|&g| reselected(0, 0.1, selection_turn(i, g))).count();
            assert!((80..=120).contains(&n), "iteration {i}: {n}");
        }
    }

    #[test]
    fn names_round_trip() {
        for a in Allocation::ALL {
            assert_eq!(Allocation::from_name(a.name()), Some(a));
        }
        assert_eq!(Allocation::from_name("quotas"), None);
        assert_eq!(Allocation::default(), Allocation::Draws);
    }
}
