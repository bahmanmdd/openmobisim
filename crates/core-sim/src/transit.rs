//! Scheduled public transport in a run (S199): stops as hubs, walking to, from
//! and between them, and passengers routed by RAPTOR.
//!
//! **Stops are hubs** (design §18.4): every stop of the timetable is a `stop`
//! hub with a transit access point (the stop), a walk access point (the walk
//! layer's nearest node, if within [`TransitDefaults::stop_walk_snap_m`]) and a
//! **bike access point** where the bike layer reaches it likewise (S193), so
//! bike-and-ride (M4) needs nothing more. A stop no pedestrian can reach is kept
//! in the timetable (its runs still run) but boards and alights nobody.
//!
//! **Walking** is on the walk layer, by its travel times: to the stops within
//! [`TransitDefaults::access_walk_max_s`] of the origin, from those around the
//! destination, and between stops within [`TransitDefaults::transfer_walk_max_s`]
//! of each other (`transfers.txt`'s minimum times replace the walk where the
//! feed gives them). A walk's time is floored to whole seconds once, as a
//! static-layer trip's is (S88).
//!
//! **A transit trip** (`walk · transit · walk`) leaves at its departure, walks
//! to a stop, rides one or more runs and walks to its destination, by the
//! earliest-arriving journey RAPTOR finds on the times of the day — the
//! schedule, or the loading's realised times for the runs that ride the roads —
//! with the fewest vehicles among the earliest. It takes at least one vehicle.
//!
//! **Cost:** per stop, a hub (about 40 bytes) and its footpaths; per trip, two
//! bounded walks and one RAPTOR query, in parallel over trips in fixed chunks, so
//! the result does not depend on the thread count.

use std::collections::HashMap;
use std::sync::Arc;

use openmobisim_core_graph::geometry::ground_distance_metres;
use openmobisim_core_graph::hubs::{AccessPoint, HubKind, HubSet, HubSpec};
use openmobisim_core_graph::layers::{Layer, StaticNetwork};
use openmobisim_core_routes::{NodeSnapper, Reach};
use openmobisim_core_transit::{
    CallTimes, Footpaths, Journey, JourneyLeg, Raptor, RaptorData, Timetable, TransitDefaults,
};
use openmobisim_core_types::ids::{EntityId, LinkId, NULL_ID, NodeId};

use crate::layers::LayerSetup;

#[cfg(feature = "parallel")]
use rayon::prelude::*;

/// Items per unit of parallel work: fixed, so the split never depends on the
/// thread count.
const CHUNK: usize = 16;

/// Run `work` on every item, each thread with its own scratch from `init`, and
/// return the results in the order of the items: the same for any number of
/// threads, provided `work` depends only on its item.
pub(crate) fn par_map<T, R, S, I, F>(items: &[T], init: I, work: F) -> Vec<R>
where
    T: Sync,
    R: Send,
    I: Fn() -> S + Sync + Send,
    F: Fn(&mut S, &T) -> R + Sync + Send,
{
    #[cfg(feature = "parallel")]
    {
        let chunks: Vec<Vec<R>> = items
            .par_chunks(CHUNK)
            .map_init(&init, |scratch, chunk| chunk.iter().map(|i| work(scratch, i)).collect())
            .collect();
        chunks.into_iter().flatten().collect()
    }
    #[cfg(not(feature = "parallel"))]
    {
        let mut scratch = init();
        items.iter().map(|i| work(&mut scratch, i)).collect()
    }
}

/// A timetable linked to a scenario's walk and bike layers: see the
/// [module docs](self).
#[derive(Debug)]
pub struct TransitSetup {
    timetable: Arc<Timetable>,
    defaults: TransitDefaults,
    walk: Arc<StaticNetwork>,
    walk_seconds: Vec<f64>,
    walk_snapper: NodeSnapper,
    hubs: HubSet,
    /// Per stop: its walk and bike layer nodes, or null.
    stop_walk: Vec<NodeId>,
    stop_bike: Vec<NodeId>,
    /// Walk node → the stops there, CSR.
    node_stops_start: Vec<u32>,
    node_stops: Vec<NodeId>,
    /// Walk nodes with a stop.
    has_stop: Vec<bool>,
    footpaths: Footpaths,
    scheduled: RaptorData,
}

fn floor_seconds(s: f64) -> u32 {
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a walk of at most an hour, non-negative"
    )]
    let t = s.max(0.0).floor() as u32;
    t
}

impl TransitSetup {
    /// Link `timetable` to the walk layer `walk` and, if there is one, the bike
    /// layer `bike`, under `defaults`.
    ///
    /// # Panics
    ///
    /// Panics if the timetable has more than `u32::MAX` stops or footpaths.
    #[must_use]
    pub fn new(
        timetable: Arc<Timetable>,
        walk: &LayerSetup,
        bike: Option<&LayerSetup>,
        defaults: TransitDefaults,
    ) -> Self {
        let walk_net = walk.network().clone();
        let walk_graph = walk_net.network();
        let walk_snapper = NodeSnapper::every_node(walk_graph);
        let n = timetable.stop_count() as usize;
        let snap = |layer: &StaticNetwork, snapper: &NodeSnapper| -> Vec<NodeId> {
            let graph = layer.network();
            (0..n)
                .map(|s| {
                    let at = timetable.stop_position(NodeId::from_index(s));
                    let node = snapper.nearest(graph, at);
                    if ground_distance_metres(at, graph.node_lonlat(node))
                        <= defaults.stop_walk_snap_m
                    {
                        node
                    } else {
                        NodeId::from_raw(NULL_ID)
                    }
                })
                .collect()
        };
        let stop_walk = snap(&walk_net, &walk_snapper);
        let stop_bike = match bike {
            Some(b) => snap(b.network(), &NodeSnapper::every_node(b.network().network())),
            None => vec![NodeId::from_raw(NULL_ID); n],
        };

        let hubs = HubSet::build(
            (0..n)
                .map(|s| {
                    let stop = NodeId::from_index(s);
                    let mut access_points = vec![AccessPoint { layer: Layer::Transit, node: stop }];
                    for (layer, node) in [(Layer::Walk, stop_walk[s]), (Layer::Bike, stop_bike[s])]
                    {
                        if !node.is_null() {
                            access_points.push(AccessPoint { layer, node });
                        }
                    }
                    HubSpec {
                        external_id: timetable.stop_ids().external(stop.raw()).to_string(),
                        kind: HubKind::Stop,
                        position: timetable.stop_position(stop),
                        access_points,
                        transfer_seconds: defaults.stop_transfer_s,
                    }
                })
                .collect(),
        );

        let nodes = walk_graph.node_count() as usize;
        let mut per_node: Vec<Vec<NodeId>> = vec![Vec::new(); nodes];
        for (s, node) in stop_walk.iter().enumerate() {
            if !node.is_null() {
                per_node[node.index()].push(NodeId::from_index(s));
            }
        }
        let mut node_stops_start = Vec::with_capacity(nodes + 1);
        node_stops_start.push(0u32);
        let mut node_stops = Vec::new();
        for list in &per_node {
            node_stops.extend_from_slice(list);
            node_stops_start.push(u32::try_from(node_stops.len()).expect("stops fit u32"));
        }
        let has_stop: Vec<bool> = per_node.iter().map(|l| !l.is_empty()).collect();

        let walk_seconds = walk.seconds().to_vec();
        let footpaths = {
            let mut reach = Reach::new(walk_graph, &walk_seconds);
            let mut walks: HashMap<(u32, u32), u32> = HashMap::new();
            for (s, &from) in stop_walk.iter().enumerate() {
                if from.is_null() {
                    continue;
                }
                for (node, secs) in reach.forward(from, defaults.transfer_walk_max_s, &has_stop) {
                    let (a, b) = (
                        node_stops_start[node.index()] as usize,
                        node_stops_start[node.index() + 1] as usize,
                    );
                    for &to in &node_stops[a..b] {
                        if to.index() != s {
                            let s32 = u32::try_from(s).expect("stops fit u32");
                            walks.insert((s32, to.raw()), floor_seconds(secs));
                        }
                    }
                }
            }
            // The feed's minimum transfer times replace the walk where it gives them.
            for t in timetable.transfers() {
                if !stop_walk[t.from.index()].is_null() && !stop_walk[t.to.index()].is_null() {
                    walks.insert((t.from.raw(), t.to.raw()), t.seconds);
                }
            }
            let mut list: Vec<(NodeId, NodeId, u32)> =
                walks.into_iter().map(|((a, b), s)| (NodeId::new(a), NodeId::new(b), s)).collect();
            list.sort_unstable();
            Footpaths::new(timetable.stop_count(), list)
        };
        let scheduled = RaptorData::new(
            &timetable,
            timetable.scheduled(),
            footpaths.clone(),
            defaults.board_slack_s,
        );
        Self {
            timetable,
            defaults,
            walk: walk_net,
            walk_seconds,
            walk_snapper,
            hubs,
            stop_walk,
            stop_bike,
            node_stops_start,
            node_stops,
            has_stop,
            footpaths,
            scheduled,
        }
    }

    /// The timetable.
    #[must_use]
    pub fn timetable(&self) -> &Arc<Timetable> {
        &self.timetable
    }

    /// The defaults it was built with.
    #[must_use]
    pub fn defaults(&self) -> TransitDefaults {
        self.defaults
    }

    /// The walk layer it walks on.
    #[must_use]
    pub fn walk(&self) -> &Arc<StaticNetwork> {
        &self.walk
    }

    /// The stop hubs, one per stop, by stop id.
    #[must_use]
    pub fn hubs(&self) -> &HubSet {
        &self.hubs
    }

    /// A stop's node on the walk layer, if a pedestrian can reach it.
    #[must_use]
    pub fn stop_walk_node(&self, stop: NodeId) -> Option<NodeId> {
        let n = self.stop_walk[stop.index()];
        (!n.is_null()).then_some(n)
    }

    /// A stop's node on the bike layer, if the bike layer reaches it.
    #[must_use]
    pub fn stop_bike_node(&self, stop: NodeId) -> Option<NodeId> {
        let n = self.stop_bike[stop.index()];
        (!n.is_null()).then_some(n)
    }

    /// The walking transfers between stops.
    #[must_use]
    pub fn footpaths(&self) -> &Footpaths {
        &self.footpaths
    }

    /// RAPTOR's form of the timetable on its scheduled times.
    #[must_use]
    pub fn scheduled(&self) -> &RaptorData {
        &self.scheduled
    }

    /// RAPTOR's form of the timetable on other times (a loading's realised ones).
    #[must_use]
    pub fn on_times(&self, times: &CallTimes) -> RaptorData {
        RaptorData::new(&self.timetable, times, self.footpaths.clone(), self.defaults.board_slack_s)
    }

    /// The walk layer node nearest a point.
    #[must_use]
    pub fn walk_node(&self, at: openmobisim_core_graph::geometry::LonLat) -> NodeId {
        self.walk_snapper.nearest(self.walk.network(), at)
    }

    fn stops_at(&self, node: NodeId) -> &[NodeId] {
        let (a, b) = (
            self.node_stops_start[node.index()] as usize,
            self.node_stops_start[node.index() + 1] as usize,
        );
        &self.node_stops[a..b]
    }

    /// Route every request (see [`TransitRequest`]) on `data`, in order.
    pub(crate) fn route_all(
        &self,
        data: &RaptorData,
        requests: &[TransitRequest],
        want_walks: bool,
    ) -> Vec<Option<TransitTravel>> {
        let graph = self.walk.network();
        let seconds = &self.walk_seconds;
        let rides = self.defaults.max_rides as usize;
        par_map(
            requests,
            || (Reach::new(graph, seconds), Reach::new(graph, seconds), Raptor::new(data, rides)),
            |(out, into, raptor), request| self.route(out, into, raptor, request, want_walks),
        )
    }

    fn route(
        &self,
        out: &mut Reach<'_>,
        into: &mut Reach<'_>,
        raptor: &mut Raptor<'_>,
        request: &TransitRequest,
        want_walks: bool,
    ) -> Option<TransitTravel> {
        let d = &self.defaults;
        let transfer = floor_seconds(d.stop_transfer_s);
        let mut access = Vec::new();
        for (node, secs) in out.forward(request.origin, d.access_walk_max_s, &self.has_stop) {
            let at = request.departure.saturating_add(floor_seconds(secs)).saturating_add(transfer);
            access.extend(self.stops_at(node).iter().map(|&s| (s, at)));
        }
        let mut egress = Vec::new();
        for (node, secs) in into.backward(request.destination, d.access_walk_max_s, &self.has_stop)
        {
            let w = floor_seconds(secs).saturating_add(transfer);
            egress.extend(self.stops_at(node).iter().map(|&s| (s, w)));
        }
        if access.is_empty() || egress.is_empty() {
            return None;
        }
        let journey = raptor.earliest(&access, &egress)?;
        let mut walks = Vec::new();
        if want_walks {
            let mut clock = request.departure;
            for leg in &journey.legs {
                match *leg {
                    JourneyLeg::Access { stop, .. } => {
                        let links = out.path(self.stop_walk[stop.index()]);
                        walks.push(Walk { departure: request.departure, links });
                    }
                    JourneyLeg::Ride { arrival, .. } => clock = arrival,
                    JourneyLeg::Transfer { from, to, .. } => {
                        let (a, b) = (self.stop_walk[from.index()], self.stop_walk[to.index()]);
                        // The walk between the two stops, found again (a footpath keeps its
                        // time only); none if the feed's transfer time stood in for it.
                        let found = out.forward(a, d.transfer_walk_max_s + 1.0, &self.has_stop);
                        if found.iter().any(|f| f.0 == b) {
                            walks.push(Walk { departure: clock, links: out.path(b) });
                        }
                    }
                    JourneyLeg::Egress { stop, .. } => {
                        let links = into.path(self.stop_walk[stop.index()]);
                        walks.push(Walk { departure: clock, links });
                    }
                }
            }
            walks.retain(|w| !w.links.is_empty());
        }
        Some(TransitTravel { arrival: journey.arrival, journey, walks })
    }

    /// Bytes held, besides the timetable.
    #[must_use]
    pub fn bytes(&self) -> usize {
        self.hubs.bytes()
            + (self.stop_walk.len() + self.stop_bike.len()) * 4
            + (self.node_stops_start.len() + self.node_stops.len()) * 4
            + self.has_stop.len()
            + self.footpaths.len() * 8
            + self.walk_seconds.len() * 8
    }
}

/// One transit trip to route: from and to walk layer nodes, leaving at a second.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TransitRequest {
    pub origin: NodeId,
    pub destination: NodeId,
    pub departure: u32,
}

/// A walk of a transit trip, on the walk layer.
#[derive(Clone, Debug)]
pub(crate) struct Walk {
    pub departure: u32,
    pub links: Vec<LinkId>,
}

/// How a transit trip went.
#[derive(Clone, Debug)]
pub(crate) struct TransitTravel {
    pub arrival: u32,
    pub journey: Journey,
    pub walks: Vec<Walk>,
}

/// What transit did in a run: the times it ran on and who boarded where.
#[derive(Clone, Debug, PartialEq)]
pub struct TransitResult {
    /// Every call's times in the run: realised for the runs that rode the roads,
    /// scheduled for the rest.
    pub times: CallTimes,
    /// Passengers boarding at each call, weighted by traveller weight.
    pub boardings: Vec<f64>,
    /// Passengers alighting at each call, likewise.
    pub alightings: Vec<f64>,
}

impl TransitResult {
    /// Nobody on board yet, on `times`.
    #[must_use]
    pub fn empty(times: CallTimes) -> Self {
        let n = times.arrival.len();
        Self { times, boardings: vec![0.0; n], alightings: vec![0.0; n] }
    }

    /// Passengers on board as each call's vehicle leaves it, per call: the
    /// running sum of boardings less alightings along each run.
    #[must_use]
    pub fn on_board(&self, timetable: &Timetable) -> Vec<f64> {
        let mut out = vec![0.0; self.boardings.len()];
        for r in 0..timetable.run_count() {
            let mut load = 0.0;
            for c in timetable.run_calls(openmobisim_core_types::ids::TransitRunId::new(r)) {
                load += self.boardings[c] - self.alightings[c];
                out[c] = load;
            }
        }
        out
    }
}
