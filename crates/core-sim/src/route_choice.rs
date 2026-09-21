//! Route choice: which of its pair's routes each trip takes (S169).
//!
//! The route sets say which alternatives exist for each origin-destination
//! pair; a [`ChoiceModel`] says which one each traveller takes. This module is
//! the join: it turns every trip's set into a situation (a
//! [`ChoiceBatch`]) whose alternatives
//! carry the route attributes below, asks the model in batches, and keeps the
//! answers.
//!
//! **The attributes** of a route, all per route and all read by name:
//!
//! | name | meaning |
//! |---|---|
//! | `time_min` | free-flow travel time in minutes, at the time the set was made (turns and signal delay included) |
//! | `length_km` | length in kilometres |
//! | `detour` | `time / best time − 1` within the set: 0 for the best route |
//! | `overlap` | the largest share of its cost shared with a route found before it (0 to 1) |
//! | `ln_path_size` | the natural log of the route's path size (Ben-Akiva and Bierlaire): 0 for a route that shares nothing, negative as it shares more |
//! | `n_links` | how many links |
//!
//! **Alternative identity** is a hash of the route's links, so it does not
//! change when other routes are added to or removed from the set, and the
//! draws of every route that stays are exactly as they were.

use std::sync::Arc;

use openmobisim_core_choice::{ChoiceBatch, ChoiceError, ChoiceModel};
use openmobisim_core_demand::{Travellers, Trips};
use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_routes::{RouteAttributes, RouteKey, RouteSets, Search, SearchContext};
use openmobisim_core_types::hash::Fnv1a;
use openmobisim_core_types::ids::{EntityId, NodeId, TripId};

use crate::equilibration::Equilibration;
use crate::link_times::LinkTimes;
use openmobisim_core_types::rng::{DrawAddress, StreamRng};

/// The attributes a route carries, in the order a batch holds them.
pub const ROUTE_ATTRIBUTES: [&str; 6] =
    ["time_min", "length_km", "detour", "overlap", "ln_path_size", "n_links"];

/// "No route": a trip with no set, or whose origin and destination are one node.
pub const NO_ROUTE: u32 = u32::MAX;

/// How many trips are asked about at once. Bounds the batch's memory and is the
/// granularity of a Python model's call; results do not depend on it.
const CHUNK: usize = 32_768;

/// Each trip's route choice, indexed by trip.
#[derive(Clone, Debug, Default)]
pub struct RouteChoices {
    /// The route taken, as an index into the run's route sets, or [`NO_ROUTE`].
    pub route: Vec<u32>,
    /// How many routes the trip could choose from (0 for none).
    pub alternatives: Vec<u32>,
    /// The probability the model gave the route taken; `NaN` if it gave none.
    pub probability: Vec<f64>,
    /// The traveller's weight: how many people the trip stands for.
    pub weight: Vec<u32>,
}

/// Equal when every entry is, comparing probabilities by their bits so that a
/// model with no probabilities (`NaN`) equals itself.
impl PartialEq for RouteChoices {
    fn eq(&self, other: &Self) -> bool {
        self.route == other.route
            && self.alternatives == other.alternatives
            && self.weight == other.weight
            && self.probability.len() == other.probability.len()
            && self
                .probability
                .iter()
                .zip(&other.probability)
                .all(|(a, b)| a.to_bits() == b.to_bits())
    }
}

impl RouteChoices {
    /// Renumber after the sets grew (S176): `shift` is what [`RouteSets::extended`] returned
    /// for the store `old`, `grown` is the new store and `trip_keys` each trip's pair. A trip
    /// keeps the route it had (now a different number where routes were added before it), and
    /// its number of alternatives is the size of its pair's grown set. Probabilities are what
    /// the model gave the route when it was taken, and stay so.
    pub(crate) fn grown(
        &mut self,
        old: &RouteSets,
        shift: &[u32],
        grown: &RouteSets,
        trip_keys: &[RouteKey],
    ) {
        for t in 0..self.route.len() {
            if self.route[t] == NO_ROUTE {
                continue;
            }
            self.route[t] += shift[old.key_of_route(self.route[t] as usize)];
            let key = grown.key_index(trip_keys[t]).expect("a routed trip has a set");
            self.alternatives[t] =
                u32::try_from(grown.route_range(key).len()).expect("few alternatives");
        }
    }

    /// How many trips.
    #[must_use]
    pub fn len(&self) -> usize {
        self.route.len()
    }

    /// Whether there are no trips.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.route.is_empty()
    }
}

/// Everything a route choice reads.
pub(crate) struct Inputs<'a> {
    pub network: &'a RoadNetwork,
    pub travellers: &'a Travellers,
    pub trips: &'a Trips,
    pub trip_keys: &'a [RouteKey],
    pub model: &'a dyn ChoiceModel,
    pub turns: &'a TurnTable,
    /// An alternative is offered only if its expected time is within this share of the best's
    /// (0: all of them; S178).
    pub detour_limit: f64,
}

/// The identity of a route: a 32-bit hash of its links.
fn route_identity(links: &[u32]) -> u32 {
    let mut h = Fnv1a::new();
    for &l in links {
        h.write_u32(l);
    }
    let v = h.finish();
    #[allow(clippy::cast_possible_truncation, reason = "folding 64 bits into 32 on purpose")]
    let folded = (v ^ (v >> 32)) as u32;
    folded
}

/// The key that marks a draw made to pick the trips for the network-wide gap.
const GAP_SAMPLE_KEY: u32 = u32::MAX;

/// The first bit of every random key that marks a **floor** draw (below): a
/// fresh sample from the model's probabilities, independent of every draw the
/// run itself uses.
const FLOOR_KEY: u32 = 0x8000_0000;

/// What one update of the assignment found.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Assessment {
    pub reselected_share: f64,
    pub changed_share: f64,
    /// The relative gap to the least-cost route of the choice set (S171), over the trips that
    /// finished (S178), the gap the model itself expects, and the difference.
    pub gap: f64,
    pub gap_expected: f64,
    pub gap_excess: f64,
    /// The share of travellers (by weight) left out because their trip had not finished.
    pub incomplete_share: f64,
    pub gap_flow: f64,
    pub gap_flow_floor: f64,
}

/// A trip that chose again, and what it chose.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Change {
    pub trip: usize,
    pub route: u32,
    pub probability: f64,
}

/// What [`Chooser::update`] found and would change.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Update {
    pub assessment: Assessment,
    pub changes: Vec<Change>,
    /// Each trip's expected travel time under the model at these times, in seconds (`NaN`
    /// for a trip with no set, or if the model gives no probabilities): what
    /// [`Chooser::network_gap`] needs to tell the model's dispersion from disequilibrium.
    pub expected_seconds: Vec<f64>,
}

/// What [`Chooser::fill`] leaves next to the batch: per situation (one trip's choice among its
/// pair's routes) and per alternative offered.
#[derive(Default)]
struct Filled {
    /// Per situation: its trip.
    trip_of: Vec<usize>,
    /// Per situation: the store index of its set's route 0, and how many routes the set has.
    set_first: Vec<usize>,
    set_len: Vec<usize>,
    /// Per situation: where its set's expected seconds start in `set_seconds`.
    set_at: Vec<usize>,
    /// The expected seconds of every route of every situation's set, offered or not, in order.
    set_seconds: Vec<f64>,
    /// Per alternative offered, as the batch holds them: its route's store index, and its expected seconds.
    alt_route: Vec<u32>,
    alt_seconds: Vec<f64>,
}

impl Filled {
    fn clear(&mut self) {
        self.trip_of.clear();
        self.set_first.clear();
        self.set_len.clear();
        self.set_at.clear();
        self.set_seconds.clear();
        self.alt_route.clear();
        self.alt_seconds.clear();
    }
}

/// The parts of route choice that do not change from one iteration to the next:
/// which attributes to fill, the routes' lengths and path sizes, their
/// identities. **It belongs to one state of the route sets**: when they grow (S176) the
/// run makes a new one, which costs a pass over the store (~50 ms at 53 000 routes) and is
/// done only then.
pub(crate) struct Chooser<'a> {
    inputs: &'a Inputs<'a>,
    route_sets: Arc<RouteSets>,
    wanted: Vec<&'static str>,
    attributes: Option<RouteAttributes>,
    identity: Vec<u32>,
    weight: Vec<u32>,
}

impl<'a> Chooser<'a> {
    /// Prepare to choose.
    ///
    /// # Errors
    ///
    /// [`ChoiceError::MissingAttribute`] if the model reads an attribute routes
    /// do not carry.
    pub(crate) fn new(
        inputs: &'a Inputs<'a>,
        route_sets: Arc<RouteSets>,
    ) -> Result<Self, ChoiceError> {
        let (trips, travellers, network) = (inputs.trips, inputs.travellers, inputs.network);
        // The attributes to fill: what the model reads, or all of them.
        let wanted: Vec<&'static str> = match inputs.model.required_attributes() {
            None => ROUTE_ATTRIBUTES.to_vec(),
            Some(names) => {
                for name in &names {
                    if !ROUTE_ATTRIBUTES.contains(&name.as_str()) {
                        return Err(ChoiceError::MissingAttribute {
                            name: name.clone(),
                            offered: ROUTE_ATTRIBUTES.iter().map(|s| (*s).to_string()).collect(),
                        });
                    }
                }
                ROUTE_ATTRIBUTES.iter().copied().filter(|a| names.iter().any(|n| n == a)).collect()
            }
        };
        let needs = |name: &str| wanted.contains(&name);
        let attributes =
            (needs("length_km") || needs("ln_path_size")).then(|| route_sets.attributes(network));
        let mut identity: Vec<u32> = (0..route_sets.route_count())
            .map(|r| route_identity(route_sets.route(r).links))
            .collect();
        // Within a set, identities must differ: nudge a (vanishingly rare) clash.
        for key in 0..route_sets.keys().len() {
            let range = route_sets.route_range(key);
            for r in range.clone() {
                while identity[range.start..r].contains(&identity[r]) {
                    identity[r] = identity[r].wrapping_add(1);
                }
            }
        }
        let weight = (0..trips.len())
            .map(|i| travellers.weight(trips.traveller(TripId::from_index(i as usize))))
            .collect();
        Ok(Self { inputs, route_sets, wanted, attributes, identity, weight })
    }

    /// Put trips `from..to` that have a set into `batch`, with `times` as their cost (free flow
    /// if `None`). Each situation offers the routes of its pair's set **whose expected time is
    /// within [`Inputs::detour_limit`] of the best's** (all of them if the limit is 0: the best
    /// route is always offered). What is left besides the batch is in `filled`.
    fn fill(
        &self,
        batch: &mut ChoiceBatch,
        filled: &mut Filled,
        range: core::ops::Range<usize>,
        times: Option<&LinkTimes>,
    ) {
        let Inputs { trips, trip_keys, detour_limit, .. } = *self.inputs;
        let route_sets = &*self.route_sets;
        batch.clear();
        filled.clear();
        let mut row = vec![0.0; self.wanted.len()];
        for i in range {
            let key = trip_keys[i];
            if key.origin == key.destination {
                continue;
            }
            let Some(k) = route_sets.key_index(key) else { continue };
            let set = route_sets.route_range(k);
            if set.is_empty() {
                continue;
            }
            let trip = TripId::from_index(i);
            batch.begin_situation(trips.traveller(trip).raw(), trip.raw());
            let departure = f64::from(trips.departure(trip).get());
            // Each route's expected time: at free flow the store's, later the link
            // times of the last loading walked from this trip's departure.
            let at = filled.set_seconds.len();
            filled.set_seconds.extend(set.clone().map(|r| {
                let view = route_sets.route(r);
                times.map_or_else(
                    || f64::from(view.cost),
                    |t| t.route_seconds(view.links, departure),
                )
            }));
            let seconds = &filled.set_seconds[at..];
            let best = seconds.iter().copied().fold(f64::INFINITY, f64::min);
            let reach =
                if detour_limit > 0.0 { best * (1.0 + detour_limit) } else { f64::INFINITY };
            for (a, r) in set.clone().enumerate() {
                if seconds[a] > reach {
                    continue;
                }
                let view = route_sets.route(r);
                for (slot, name) in row.iter_mut().zip(&self.wanted) {
                    *slot = match *name {
                        "time_min" => seconds[a] / 60.0,
                        "length_km" => {
                            self.attributes.as_ref().map_or(0.0, |x| x.length_m[r]) / 1000.0
                        }
                        "detour" => (seconds[a] / best - 1.0).max(0.0),
                        "overlap" => f64::from(view.overlap),
                        "ln_path_size" => {
                            self.attributes.as_ref().map_or(1.0, |x| x.path_size[r]).ln()
                        }
                        _ => f64::from(u32::try_from(view.links.len()).unwrap_or(u32::MAX)),
                    };
                }
                batch.push_alternative(self.identity[r], &row);
                filled.alt_route.push(u32::try_from(r).expect("route ids fit in 32 bits"));
                filled.alt_seconds.push(seconds[a]);
            }
            filled.set_first.push(set.start);
            filled.set_len.push(set.len());
            filled.set_at.push(at);
            filled.trip_of.push(i);
        }
    }

    /// The gap against the **whole network** (S171), for a keyed sample of `sample` of the
    /// routed trips that **finished** (`unfinished[t]` false; S178): `Σ w t_chosen / Σ w t_fastest − 1`,
    /// where `t_fastest` is the least travel time over *any* route at the times `times` gives,
    /// found by a bounded time-dependent search (no worse than the cheapest of the trip's choice
    /// set). Returns it with the **disequilibrium** version (S178): the same less what the
    /// model itself expects inside the set, `Σ w (t_chosen − t_expected + t_least_in_set −
    /// t_fastest) / Σ w t_fastest`, where `expected_seconds[t]` is trip `t`'s expected time
    /// under the model (`NaN`: the model gave no probabilities; the second number is then `NaN`).
    ///
    /// The sample is the `sample` trips with the smallest keyed draws (a draw on the trip
    /// alone, from the re-selection stream), so it is the same for any thread count and
    /// does not change when others are added. `NaN` if nothing is routed. Costs one
    /// bounded search per sampled trip: about the work of a shortest path, once.
    pub(crate) fn network_gap(
        &self,
        current: &RouteChoices,
        times: &LinkTimes,
        sample: u32,
        rng: &StreamRng,
        unfinished: &[bool],
        expected_seconds: &[f64],
    ) -> (f64, f64) {
        let Inputs { network, trips, trip_keys, turns, .. } = *self.inputs;
        let route_sets = &*self.route_sets;
        let mut keyed: Vec<(f64, usize)> = (0..current.route.len())
            .filter(|&t| current.route[t] != NO_ROUTE && !unfinished[t])
            .map(|t| {
                let key = rng.unit(DrawAddress::from_pair(
                    u32::try_from(t).expect("trip ids fit in 32 bits"),
                    GAP_SAMPLE_KEY,
                ));
                (key, t)
            })
            .collect();
        let take = (sample as usize).min(keyed.len());
        if take == 0 {
            return (f64::NAN, f64::NAN);
        }
        keyed.select_nth_unstable_by(take - 1, |a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        let mut chosen: Vec<usize> = keyed[..take].iter().map(|&(_, t)| t).collect();
        chosen.sort_unstable();

        let ctx = SearchContext::new(network, turns);
        let mut search = Search::new(&ctx);
        let wait = |link: u32, departure: f64| times.origin_wait_seconds(link, departure);
        let seconds = |link: u32, at: f64| times.link_seconds(link, at);
        let (mut paid, mut fastest, mut excess) = (0.0, 0.0, 0.0);
        for t in chosen {
            let key = trip_keys[t];
            let departure = f64::from(trips.departure(TripId::from_index(t)).get());
            let k = route_sets.key_index(key).expect("a routed trip has a set");
            let mut in_set = f64::INFINITY;
            let mut taken = 0.0;
            for r in route_sets.route_range(k) {
                let time = times.route_seconds(route_sets.route(r).links, departure);
                in_set = in_set.min(time);
                if r == current.route[t] as usize {
                    taken = time;
                }
            }
            let found = search.fastest_time(
                NodeId::new(key.origin),
                NodeId::new(key.destination),
                departure,
                in_set,
                &wait,
                &seconds,
            );
            let w = f64::from(current.weight[t]);
            let best = found.map_or(in_set, |f| f.min(in_set));
            paid += w * taken;
            fastest += w * best;
            excess += w * (taken - expected_seconds[t] + in_set - best);
        }
        if fastest > 0.0 {
            ((paid - fastest) / fastest, excess / fastest)
        } else {
            (f64::NAN, f64::NAN)
        }
    }

    fn empty_choices(&self) -> RouteChoices {
        let total = self.inputs.trips.len() as usize;
        RouteChoices {
            route: vec![NO_ROUTE; total],
            alternatives: vec![0; total],
            probability: vec![f64::NAN; total],
            weight: self.weight.clone(),
        }
    }

    /// Every trip chooses, on free-flow costs (iteration 0).
    ///
    /// # Errors
    ///
    /// [`ChoiceError`] if the model is given a malformed batch, or fails or answers
    /// wrongly.
    pub(crate) fn choose_all(
        &self,
        rng: &StreamRng,
        iteration: u32,
    ) -> Result<RouteChoices, ChoiceError> {
        let total = self.inputs.trips.len() as usize;
        let mut out = self.empty_choices();
        let mut batch = ChoiceBatch::new(iteration, &self.wanted);
        let mut filled = Filled::default();
        let mut i = 0;
        while i < total {
            let end = (i + CHUNK).min(total);
            self.fill(&mut batch, &mut filled, i..end, None);
            i = end;
            if filled.trip_of.is_empty() {
                continue;
            }
            batch.validate()?;
            let choices = self.inputs.model.choose(&batch, rng)?;
            choices.validate(&batch)?;
            for (s, &t) in filled.trip_of.iter().enumerate() {
                out.route[t] = filled.alt_route[batch.range(s).start + choices.chosen[s] as usize];
                // How many routes its pair's set holds, however many were offered.
                out.alternatives[t] = u32::try_from(filled.set_len[s]).expect("few alternatives");
                out.probability[t] = choices.probability[s];
            }
        }
        Ok(out)
    }

    /// Assess the assignment `current` against the link `times` its loading
    /// produced, and, if `strategy` is given, work out who would choose again and
    /// what they would choose. **Nothing is changed**: the caller applies
    /// [`Update::changes`] if the run goes on (it may stop first, having converged).
    ///
    /// The assessment (the flow and cost gaps against their floor, design §11.2)
    /// is of `current` as it was loaded. **The cost gaps leave out the trips still under
    /// way when the window ended** (`unfinished[t]`, S178: their times are lower bounds), and
    /// say how many those are; the expected gap is what the model itself expects of the rest.
    ///
    /// # Errors
    ///
    /// [`ChoiceError`] as for [`Self::choose_all`].
    pub(crate) fn update(
        &self,
        current: &RouteChoices,
        times: &LinkTimes,
        iteration: u32,
        strategy: Option<(&dyn Equilibration, &StreamRng)>,
        rng: &StreamRng,
        unfinished: &[bool],
    ) -> Result<Update, ChoiceError> {
        let total = self.inputs.trips.len() as usize;
        let sets = &*self.route_sets;
        let mut batch = ChoiceBatch::new(iteration, &self.wanted);
        let mut filled = Filled::default();
        let routes = sets.route_count();
        let (mut observed, mut expected, mut sample) =
            (vec![0.0; routes], vec![0.0; routes], vec![0.0; routes]);
        let (mut w_all, mut w_reselected, mut w_changed, mut w_unfinished) = (0.0, 0.0, 0.0, 0.0);
        let (mut paid, mut least, mut expected_paid) = (0.0, 0.0, 0.0);
        let mut expected_seconds = vec![f64::NAN; total];
        let mut measured = true;
        let mut changes: Vec<Change> = Vec::new();
        let mut i = 0;
        while i < total {
            let end = (i + CHUNK).min(total);
            self.fill(&mut batch, &mut filled, i..end, Some(times));
            i = end;
            if filled.trip_of.is_empty() {
                continue;
            }
            batch.validate()?;
            let probabilities = self.inputs.model.probabilities(&batch)?;
            measured &= probabilities.is_some();
            let mut moving: Vec<usize> = Vec::new();
            for (s, &t) in filled.trip_of.iter().enumerate() {
                let w = f64::from(current.weight[t]);
                w_all += w;
                let range = batch.range(s);
                let taken = current.route[t] as usize;
                // The gap: what the route taken costs against the cheapest of the set, at the
                // times this loading produced (the whole set, offered or not).
                let set_seconds = &filled.set_seconds[filled.set_at[s]..][..filled.set_len[s]];
                let least_here = set_seconds.iter().copied().fold(f64::INFINITY, f64::min);
                let finished = !unfinished[t];
                if finished {
                    paid += w * set_seconds[taken - filled.set_first[s]];
                    least += w * least_here;
                } else {
                    w_unfinished += w;
                }
                if let Some(p) = &probabilities {
                    observed[taken] += w;
                    let u = floor_draw(rng, &batch, s, iteration);
                    let mut cumulative = 0.0;
                    let mut drawn = false;
                    let mut expected_here = 0.0;
                    for (a, r) in range.clone().enumerate() {
                        let route = filled.alt_route[r] as usize;
                        expected[route] += w * p[r];
                        expected_here += p[r] * filled.alt_seconds[r];
                        cumulative += p[r];
                        if !drawn && (u < cumulative || a + 1 == range.len()) {
                            sample[route] += w;
                            drawn = true;
                        }
                    }
                    expected_seconds[t] = expected_here;
                    if finished {
                        expected_paid += w * expected_here;
                    }
                }
                if let Some((strategy, msa_rng)) = strategy {
                    if strategy.reselects(msa_rng, batch.travellers()[s], iteration) {
                        moving.push(s);
                    }
                }
            }
            if !moving.is_empty() {
                let sub = batch.subset(&moving);
                let choices = self.inputs.model.choose(&sub, rng)?;
                choices.validate(&sub)?;
                for (m, &s) in moving.iter().enumerate() {
                    let t = filled.trip_of[s];
                    let w = f64::from(current.weight[t]);
                    let new = filled.alt_route[batch.range(s).start + choices.chosen[m] as usize];
                    w_reselected += w;
                    if new != current.route[t] {
                        w_changed += w;
                    }
                    changes.push(Change {
                        trip: t,
                        route: new,
                        probability: choices.probability[m],
                    });
                }
            }
        }
        let share = |x: f64| if w_all > 0.0 { x / w_all } else { 0.0 };
        let tv = |a: &[f64]| 0.5 * a.iter().zip(&expected).map(|(x, e)| (x - e).abs()).sum::<f64>();
        let (gap, gap_expected, gap_excess) = if least > 0.0 {
            let gap = (paid - least) / least;
            if measured {
                let expected_gap = (expected_paid - least) / least;
                (gap, expected_gap, gap - expected_gap)
            } else {
                (gap, f64::NAN, f64::NAN)
            }
        } else {
            (f64::NAN, f64::NAN, f64::NAN)
        };
        let assessment = Assessment {
            reselected_share: share(w_reselected),
            changed_share: share(w_changed),
            gap,
            gap_expected,
            gap_excess,
            incomplete_share: share(w_unfinished),
            gap_flow: if measured { share(tv(&observed)) } else { f64::NAN },
            gap_flow_floor: if measured { share(tv(&sample)) } else { f64::NAN },
        };
        Ok(Update { assessment, changes, expected_seconds })
    }
}

/// The uniform draw that picks a route for the floor sample of situation `s`:
/// keyed on (traveller, trip, `FLOOR_KEY | iteration`, 0) from the choice stream,
/// so it is independent of every draw the run itself makes.
fn floor_draw(rng: &StreamRng, batch: &ChoiceBatch, s: usize, iteration: u32) -> f64 {
    rng.unit(DrawAddress::from_quad(
        batch.travellers()[s],
        batch.trips()[s],
        FLOOR_KEY | iteration,
        0,
    ))
}
