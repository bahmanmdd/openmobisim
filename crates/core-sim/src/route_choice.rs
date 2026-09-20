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

use openmobisim_core_choice::{ChoiceBatch, ChoiceError, ChoiceModel};
use openmobisim_core_demand::{Travellers, Trips};
use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_routes::{RouteAttributes, RouteKey, RouteSets};
use openmobisim_core_types::hash::Fnv1a;
use openmobisim_core_types::ids::{EntityId, TripId};

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
    pub route_sets: &'a RouteSets,
    pub model: &'a dyn ChoiceModel,
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

/// The first bit of every random key that marks a **floor** draw (below): a
/// fresh sample from the model's probabilities, independent of every draw the
/// run itself uses.
const FLOOR_KEY: u32 = 0x8000_0000;

/// What one update of the assignment found.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Assessment {
    pub reselected_share: f64,
    pub changed_share: f64,
    pub gap_flow: f64,
    pub gap_flow_floor: f64,
    pub gap_cost: f64,
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
}

/// The parts of route choice that do not change from one iteration to the next:
/// which attributes to fill, the routes' lengths and path sizes, their
/// identities.
pub(crate) struct Chooser<'a> {
    inputs: &'a Inputs<'a>,
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
    pub(crate) fn new(inputs: &'a Inputs<'a>) -> Result<Self, ChoiceError> {
        let (route_sets, trips, travellers, network) =
            (inputs.route_sets, inputs.trips, inputs.travellers, inputs.network);
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
        Ok(Self { inputs, wanted, attributes, identity, weight })
    }

    /// Put trips `from..to` that have a set into `batch`, with `times` as their cost
    /// (free flow if `None`). `first_route[s]` is the store index of situation `s`'s
    /// route 0, and `trip_of[s]` its trip.
    fn fill(
        &self,
        batch: &mut ChoiceBatch,
        first_route: &mut Vec<usize>,
        trip_of: &mut Vec<usize>,
        range: core::ops::Range<usize>,
        times: Option<&LinkTimes>,
    ) {
        let Inputs { trips, trip_keys, route_sets, .. } = *self.inputs;
        batch.clear();
        first_route.clear();
        trip_of.clear();
        let mut row = vec![0.0; self.wanted.len()];
        let mut seconds: Vec<f64> = Vec::new();
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
            seconds.clear();
            seconds.extend(set.clone().map(|r| {
                let view = route_sets.route(r);
                times.map_or_else(
                    || f64::from(view.cost),
                    |t| t.route_seconds(view.links, departure),
                )
            }));
            let best = seconds.iter().copied().fold(f64::INFINITY, f64::min);
            for (a, r) in set.clone().enumerate() {
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
            }
            first_route.push(set.start);
            trip_of.push(i);
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
        let (mut first_route, mut trip_of) = (Vec::new(), Vec::new());
        let mut i = 0;
        while i < total {
            let end = (i + CHUNK).min(total);
            self.fill(&mut batch, &mut first_route, &mut trip_of, i..end, None);
            i = end;
            if trip_of.is_empty() {
                continue;
            }
            batch.validate()?;
            let choices = self.inputs.model.choose(&batch, rng)?;
            choices.validate(&batch)?;
            for (s, &t) in trip_of.iter().enumerate() {
                out.route[t] = u32::try_from(first_route[s] + choices.chosen[s] as usize)
                    .expect("route ids fit in 32 bits");
                out.alternatives[t] =
                    u32::try_from(batch.range(s).len()).expect("few alternatives");
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
    /// is of `current` as it was loaded.
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
    ) -> Result<Update, ChoiceError> {
        let total = self.inputs.trips.len() as usize;
        let sets = self.inputs.route_sets;
        let mut batch = ChoiceBatch::new(iteration, &self.wanted);
        let (mut first_route, mut trip_of) = (Vec::new(), Vec::new());
        let routes = sets.route_count();
        let (mut observed, mut expected, mut sample) =
            (vec![0.0; routes], vec![0.0; routes], vec![0.0; routes]);
        let (mut w_all, mut w_reselected, mut w_changed) = (0.0, 0.0, 0.0);
        let (mut cost_chosen, mut cost_expected) = (0.0, 0.0);
        let mut measured = true;
        let mut changes: Vec<Change> = Vec::new();
        let mut i = 0;
        while i < total {
            let end = (i + CHUNK).min(total);
            self.fill(&mut batch, &mut first_route, &mut trip_of, i..end, Some(times));
            i = end;
            if trip_of.is_empty() {
                continue;
            }
            batch.validate()?;
            let time_min = batch.attribute("time_min");
            let probabilities = self.inputs.model.probabilities(&batch)?;
            measured &= probabilities.is_some();
            let mut moving: Vec<usize> = Vec::new();
            for (s, &t) in trip_of.iter().enumerate() {
                let w = f64::from(current.weight[t]);
                w_all += w;
                let range = batch.range(s);
                let taken = current.route[t] as usize;
                if let Some(p) = &probabilities {
                    observed[taken] += w;
                    let (mut chosen_cost, mut mean_cost) = (0.0, 0.0);
                    let u = floor_draw(rng, &batch, s, iteration);
                    let mut cumulative = 0.0;
                    let mut drawn = false;
                    for (a, r) in range.clone().enumerate() {
                        expected[first_route[s] + a] += w * p[r];
                        cumulative += p[r];
                        if !drawn && (u < cumulative || a + 1 == range.len()) {
                            sample[first_route[s] + a] += w;
                            drawn = true;
                        }
                        if let Some(time) = time_min {
                            mean_cost += p[r] * time[r];
                            if first_route[s] + a == taken {
                                chosen_cost = time[r];
                            }
                        }
                    }
                    cost_chosen += w * chosen_cost;
                    cost_expected += w * mean_cost;
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
                    let t = trip_of[s];
                    let w = f64::from(current.weight[t]);
                    let new = u32::try_from(first_route[s] + choices.chosen[m] as usize)
                        .expect("route ids fit in 32 bits");
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
        let assessment = Assessment {
            reselected_share: share(w_reselected),
            changed_share: share(w_changed),
            gap_flow: if measured { share(tv(&observed)) } else { f64::NAN },
            gap_flow_floor: if measured { share(tv(&sample)) } else { f64::NAN },
            gap_cost: if measured && cost_chosen > 0.0 {
                (cost_chosen - cost_expected) / cost_chosen
            } else {
                f64::NAN
            },
        };
        Ok(Update { assessment, changes })
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
