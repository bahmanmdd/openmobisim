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
use openmobisim_core_routes::{RouteKey, RouteSets};
use openmobisim_core_types::hash::Fnv1a;
use openmobisim_core_types::ids::{EntityId, TripId};
use openmobisim_core_types::rng::StreamRng;

/// The attributes a route carries, in the order a batch holds them.
pub const ROUTE_ATTRIBUTES: [&str; 6] =
    ["time_min", "length_km", "detour", "overlap", "ln_path_size", "n_links"];

/// "No route": a trip with no set, or whose origin and destination are one node.
pub const NO_ROUTE: u32 = u32::MAX;

/// How many trips are asked about at once. Bounds the batch's memory and is the
/// granularity of a Python model's call; results do not depend on it.
const CHUNK: usize = 32_768;

/// Each trip's route choice, indexed by trip.
#[derive(Clone, Debug, Default, PartialEq)]
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
    pub rng: &'a StreamRng,
    pub iteration: u32,
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

/// Choose a route for every trip that has more than one node to travel between.
///
/// # Errors
///
/// [`ChoiceError`] if the model needs an attribute routes do not carry, is
/// given a malformed batch, or fails or answers wrongly.
pub(crate) fn choose_routes(inputs: &Inputs<'_>) -> Result<RouteChoices, ChoiceError> {
    let Inputs { network, travellers, trips, trip_keys, route_sets, model, rng, iteration } =
        *inputs;
    let total = trips.len() as usize;

    // The attributes to fill: what the model reads, or all of them.
    let wanted: Vec<&str> = match model.required_attributes() {
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
    let identity: Vec<u32> = {
        let mut ids: Vec<u32> = (0..route_sets.route_count())
            .map(|r| route_identity(route_sets.route(r).links))
            .collect();
        // Within a set, identities must differ: nudge a (vanishingly rare) clash.
        for key in 0..route_sets.keys().len() {
            let range = route_sets.route_range(key);
            for r in range.clone() {
                while ids[range.start..r].contains(&ids[r]) {
                    ids[r] = ids[r].wrapping_add(1);
                }
            }
        }
        ids
    };

    let mut out = RouteChoices {
        route: vec![NO_ROUTE; total],
        alternatives: vec![0; total],
        probability: vec![f64::NAN; total],
        weight: (0..total)
            .map(|i| travellers.weight(trips.traveller(TripId::from_index(i))))
            .collect(),
    };

    let mut batch = ChoiceBatch::new(iteration, &wanted);
    let mut first_route: Vec<usize> = Vec::new(); // per situation: the store index of its route 0
    let mut trip_of: Vec<usize> = Vec::new(); // per situation: the trip
    let mut row = vec![0.0; wanted.len()];

    let mut i = 0;
    while i < total {
        batch.clear();
        first_route.clear();
        trip_of.clear();
        while i < total && trip_of.len() < CHUNK {
            let key = trip_keys[i];
            let trip = TripId::from_index(i);
            if key.origin != key.destination {
                if let Some(k) = route_sets.key_index(key) {
                    let range = route_sets.route_range(k);
                    if !range.is_empty() {
                        batch.begin_situation(trips.traveller(trip).raw(), trip.raw());
                        let best = f64::from(route_sets.route(range.start).cost);
                        for r in range.clone() {
                            let view = route_sets.route(r);
                            for (slot, name) in row.iter_mut().zip(&wanted) {
                                *slot = match *name {
                                    "time_min" => f64::from(view.cost) / 60.0,
                                    "length_km" => {
                                        attributes.as_ref().map_or(0.0, |a| a.length_m[r]) / 1000.0
                                    }
                                    "detour" => (f64::from(view.cost) / best - 1.0).max(0.0),
                                    "overlap" => f64::from(view.overlap),
                                    "ln_path_size" => {
                                        attributes.as_ref().map_or(1.0, |a| a.path_size[r]).ln()
                                    }
                                    _ => f64::from(
                                        u32::try_from(view.links.len()).unwrap_or(u32::MAX),
                                    ), // "n_links"
                                };
                            }
                            batch.push_alternative(identity[r], &row);
                        }
                        first_route.push(range.start);
                        trip_of.push(i);
                    }
                }
            }
            i += 1;
        }
        if trip_of.is_empty() {
            continue;
        }
        batch.validate()?;
        let choices = model.choose(&batch, rng)?;
        choices.validate(&batch)?;
        for (s, &t) in trip_of.iter().enumerate() {
            out.route[t] = u32::try_from(first_route[s] + choices.chosen[s] as usize)
                .expect("route ids fit in 32 bits");
            out.alternatives[t] = u32::try_from(batch.range(s).len()).expect("few alternatives");
            out.probability[t] = choices.probability[s];
        }
    }
    Ok(out)
}
