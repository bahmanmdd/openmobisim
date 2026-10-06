//! Travel-time matrices between points, by mode, after a run: **skims** (S238, roadmap I-az).
//!
//! A skim is the door-to-door time from each origin to each destination, leaving at one
//! second of the day, for one mode — what accessibility measures (cumulative opportunities,
//! gravity) and OD-level outputs are made of. Each mode as the run's trips travel it:
//!
//! - **car**: the earliest arrival on the **last loading's link times** (congestion included,
//!   by time bin, with the wait to get onto the first link), from the nearest drivable node to
//!   the nearest; free-flow times after a run of one loading, which records none;
//! - **bike** and **walk**: the time of the **least-cost route** on the layer (a bike's cost
//!   puts its premium on mixed traffic, as the trips' routes do), node to nearest node;
//! - **transit**: the earliest arrival with at least one vehicle, on the times the runs kept in
//!   the last loading (the schedule for those that do not ride the roads), walking to and from
//!   stops within the run's `access_walk_max_s`, transfers by the timetable's footpaths.
//!
//! Classes' own limits do not apply: a skim belongs to no traveller. Origin and destination at
//! one node take 0. Unreachable within `max_s`: `NaN`.
//!
//! **Cost:** one search per origin, in parallel over origins: a time-dependent one-to-many
//! search for cars, a one-to-many search for bikes and walks, a full RAPTOR search for transit
//! ([`openmobisim_core_transit::Raptor::arrivals`]). Nothing runs during the run itself; the run
//! keeps its last link times ([`crate::Run::final_link_times`]).

use std::sync::{Arc, OnceLock};

use openmobisim_core_demand::Mode;
use openmobisim_core_graph::geometry::LonLat;
use openmobisim_core_graph::layers::StaticLayer;
use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_routes::search::{Search, SearchContext};
use openmobisim_core_routes::{NodeSnapper, Reach};
use openmobisim_core_transit::{CallTimes, Raptor, RaptorData, UNKNOWN_TIME};
use openmobisim_core_types::ids::{EntityId, NodeId};

use crate::layers::{LayerSetup, StaticLayers};
use crate::link_times::LinkTimes;
use crate::transit::{TransitSetup, par_map};

/// How many origins a thread takes at once.
const CHUNK: usize = 8;

/// What a skim needs of a run: see the [module docs](self). The turn table and the timetable
/// on the run's times are made on the first skim that needs them.
pub struct Skimmer {
    road: Arc<RoadNetwork>,
    turns: OnceLock<Arc<TurnTable>>,
    times: Arc<LinkTimes>,
    layers: Arc<StaticLayers>,
    transit: Option<(Arc<TransitSetup>, Option<CallTimes>)>,
    raptor: OnceLock<Arc<RaptorData>>,
}

impl Skimmer {
    /// A skimmer over a run's network, turns (`None`: made from the network when first
    /// needed), last link times (`None`: free flow), layers, and timetable with the times its
    /// runs kept (`None`: the schedule).
    #[must_use]
    pub fn new(
        road: Arc<RoadNetwork>,
        turns: Option<Arc<TurnTable>>,
        times: Option<Arc<LinkTimes>>,
        layers: Arc<StaticLayers>,
        transit: Option<(Arc<TransitSetup>, Option<CallTimes>)>,
    ) -> Self {
        let times = times.unwrap_or_else(|| Arc::new(LinkTimes::free_flow(&road)));
        let cell = OnceLock::new();
        if let Some(t) = turns {
            let _ = cell.set(t);
        }
        Self { road, turns: cell, times, layers, transit, raptor: OnceLock::new() }
    }

    /// The layer the run has for `layer`, if it prepared one.
    #[must_use]
    pub fn layer(&self, layer: StaticLayer) -> Option<&LayerSetup> {
        self.layers.get(layer)
    }

    /// Seconds from each origin to each destination by `mode`, leaving at `departure`
    /// (seconds after midnight), row by origin: `NaN` where nothing arrives within `max_s`.
    /// `layer` stands in for a bike or walk layer the run did not prepare.
    ///
    /// # Errors
    ///
    /// A message for a mode a skim does not cover (park-and-ride, bike-and-ride), a bike or
    /// walk skim without a layer, or a transit skim without a timetable.
    pub fn skim(
        &self,
        mode: Mode,
        origins: &[LonLat],
        destinations: &[LonLat],
        departure: f64,
        max_s: f64,
        layer: Option<&LayerSetup>,
    ) -> Result<Vec<f64>, String> {
        match mode {
            Mode::Car => Ok(self.car(origins, destinations, departure, max_s)),
            Mode::Bike | Mode::Walk => {
                let which = if mode == Mode::Bike { StaticLayer::Bike } else { StaticLayer::Walk };
                let setup = layer
                    .or_else(|| self.layers.get(which))
                    .ok_or_else(|| format!("the run has no {} layer to skim", mode.as_str()))?;
                Ok(static_skim(setup, origins, destinations, max_s))
            }
            Mode::Transit => {
                let (transit, times) = self
                    .transit
                    .as_ref()
                    .ok_or_else(|| "the run has no timetable to skim".to_string())?;
                let data = self.raptor.get_or_init(|| {
                    Arc::new(
                        times
                            .as_ref()
                            .map_or_else(|| transit.scheduled().clone(), |t| transit.on_times(t)),
                    )
                });
                Ok(transit_skim(transit, data, origins, destinations, departure, max_s))
            }
            other => {
                Err(format!("a skim covers car, bike, walk and transit, not {}", other.as_str()))
            }
        }
    }

    fn car(
        &self,
        origins: &[LonLat],
        destinations: &[LonLat],
        departure: f64,
        max_s: f64,
    ) -> Vec<f64> {
        let road = &*self.road;
        let snapper = NodeSnapper::new(road);
        let turns = self.turns.get_or_init(|| Arc::new(TurnTable::build(road, road.signals())));
        let ctx = SearchContext::new(road, turns);
        let times = &*self.times;
        let wait = |l: u32, t: f64| times.origin_wait_seconds(l, t);
        let seconds = |l: u32, t: f64| times.link_seconds(l, t);
        let targets_of: Vec<NodeId> =
            destinations.iter().map(|&d| snapper.nearest(road, d)).collect();
        let mut targets = vec![false; road.node_count() as usize];
        for &n in &targets_of {
            targets[n.index()] = true;
        }
        let wanted = targets.iter().filter(|&&t| t).count();
        let rows = par_map(
            origins,
            CHUNK,
            || (Search::new(&ctx), vec![f64::NAN; road.node_count() as usize]),
            |(search, at), &o| {
                let origin = snapper.nearest(road, o);
                let found = search
                    .fastest_routes_to(origin, &targets, wanted, departure, max_s, &wait, &seconds);
                for &(node, t, _) in &found {
                    at[node.index()] = t;
                }
                let row: Vec<f64> = targets_of.iter().map(|n| at[n.index()]).collect();
                for &(node, _, _) in &found {
                    at[node.index()] = f64::NAN;
                }
                row
            },
        );
        rows.concat()
    }
}

/// A bike or walk skim on `setup`: the least-cost route's time.
fn static_skim(
    setup: &LayerSetup,
    origins: &[LonLat],
    destinations: &[LonLat],
    max_s: f64,
) -> Vec<f64> {
    let graph = setup.network().network();
    let snapper = NodeSnapper::every_node(graph);
    let ctx = SearchContext::with_costs(graph, setup.turns(), setup.costs().to_vec());
    let (costs, link_seconds) = (setup.costs(), setup.seconds());
    // A leg of `max_s` seconds costs at most that times the layer's largest cost per second.
    let per_second = costs
        .iter()
        .zip(link_seconds)
        .filter(|&(c, t)| c.is_finite() && *t > 0.0)
        .fold(1.0_f64, |m, (c, t)| m.max(c / t));
    let bound = max_s * per_second;
    let targets_of: Vec<NodeId> = destinations.iter().map(|&d| snapper.nearest(graph, d)).collect();
    let mut targets = vec![false; graph.node_count() as usize];
    for &n in &targets_of {
        targets[n.index()] = true;
    }
    let wanted = targets.iter().filter(|&&t| t).count();
    let no_wait = |_: u32, _: f64| 0.0;
    let cost = |l: u32, _: f64| costs[l as usize];
    let rows = par_map(
        origins,
        CHUNK,
        || (Search::new(&ctx), vec![f64::NAN; graph.node_count() as usize]),
        |(search, at), &o| {
            let origin = snapper.nearest(graph, o);
            let found =
                search.fastest_routes_to(origin, &targets, wanted, 0.0, bound, &no_wait, &cost);
            for (node, _, links) in &found {
                // `+ 0.0`: an empty sum is −0.0.
                let seconds: f64 = links.iter().map(|l| link_seconds[l.index()]).sum::<f64>() + 0.0;
                at[node.index()] = if seconds <= max_s { seconds } else { f64::NAN };
            }
            let row: Vec<f64> = targets_of.iter().map(|n| at[n.index()]).collect();
            for (node, _, _) in &found {
                at[node.index()] = f64::NAN;
            }
            row
        },
    );
    rows.concat()
}

/// A transit skim: walk, ride at least once, walk.
fn transit_skim(
    transit: &TransitSetup,
    data: &RaptorData,
    origins: &[LonLat],
    destinations: &[LonLat],
    departure: f64,
    max_s: f64,
) -> Vec<f64> {
    let walk = transit.walk().network();
    let seconds = transit.walk_link_seconds();
    let limit = transit.defaults().access_walk_max_s;
    let max_rides = transit.defaults().max_rides as usize;
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a second of the day"
    )]
    let leave = departure.max(0.0) as u32;
    let to_nodes: Vec<NodeId> = destinations.iter().map(|&d| transit.walk_node(d)).collect();
    let mut reach = Reach::new(walk, seconds);
    let egress: Vec<Vec<(NodeId, u32)>> =
        to_nodes.iter().map(|&n| transit.egress(&mut reach, n, limit)).collect();
    let rows = par_map(
        origins,
        CHUNK,
        || (Reach::new(walk, seconds), Raptor::new(data, max_rides)),
        |(reach, raptor), &o| {
            let from = transit.walk_node(o);
            let access = transit.access(reach, from, leave, limit);
            let arrivals = if access.is_empty() { Vec::new() } else { raptor.arrivals(&access) };
            to_nodes
                .iter()
                .zip(&egress)
                .map(|(&to, out)| {
                    if to == from {
                        return 0.0;
                    }
                    let best = out
                        .iter()
                        .filter_map(|&(stop, w)| {
                            let t = *arrivals.get(stop.index())?;
                            (t != UNKNOWN_TIME).then(|| f64::from(t.saturating_add(w)))
                        })
                        .fold(f64::INFINITY, f64::min);
                    let s = best - f64::from(leave);
                    if s.is_finite() && s <= max_s { s } else { f64::NAN }
                })
                .collect::<Vec<f64>>()
        },
    );
    rows.concat()
}
