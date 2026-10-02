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
//! **Buses ride the roads** ([`TransitSetup::with_roads`]): every bus pattern is
//! routed stop to stop at free flow on the roads cars use and on **busways**, which
//! only buses use (rung 2 of design §18.5), each stop at the nearest node of such a
//! link within
//! [`TransitDefaults::bus_stop_snap_m`]. A pattern with a stop off the roads, a
//! stop the roads do not connect to the next, or a free-flow time more than
//! [`TransitDefaults::bus_plausibility_ratio`] times its schedule's is **run by
//! the schedule** instead (rung 3; counted in [`BusReport`]). In a loading each
//! bus run is a chain of stop-to-stop vehicles of [`TransitDefaults::bus_pcu`]:
//! a leg leaves its stop [`TransitDefaults::bus_dwell_s`] after the leg before
//! arrives, and not before its scheduled departure (`core-loading`'s chained
//! vehicles), so a bus that falls behind stays behind and one that is early waits.
//! The loading's times replace the schedule's for those runs, and passengers
//! route on them.
//!
//! **Cost:** per stop, a hub (about 40 bytes) and its footpaths; per bus pattern,
//! a shortest-path search per pair of stops (once); per trip, two bounded walks
//! and one RAPTOR query, in parallel over trips in fixed chunks, so the result
//! does not depend on the thread count.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::OnceLock;

use openmobisim_core_graph::defaults::SignalDefaults;
use openmobisim_core_graph::geometry::ground_distance_metres;
use openmobisim_core_graph::hubs::{AccessPoint, HubKind, HubSet, HubSpec};
use openmobisim_core_graph::layers::{Layer, StaticNetwork};
use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_loading::{Trajectory, Vehicle};
use openmobisim_core_routes::{NodeSnapper, Reach, Search, SearchContext};
use openmobisim_core_transit::{
    CallTimes, Footpaths, RaptorData, Timetable, TransitDefaults, UNKNOWN_TIME,
};
use openmobisim_core_types::ids::{EntityId, LinkId, NULL_ID, NodeId, TransitRunId, VehicleId};
use openmobisim_core_types::time::Second;
use openmobisim_core_types::units::Pcu;

use crate::layers::LayerSetup;

#[cfg(feature = "parallel")]
use rayon::prelude::*;

/// Run `work` on every item, in chunks of `chunk` items, each thread with its own
/// scratch from `init` (made at least once per chunk it takes: a large scratch wants
/// large chunks), and return the results in the order of the items: the same for any
/// number of threads and any `chunk`, provided `work` depends only on its item.
pub(crate) fn par_map<T, R, S, I, F>(items: &[T], chunk: usize, init: I, work: F) -> Vec<R>
where
    T: Sync,
    R: Send,
    I: Fn() -> S + Sync + Send,
    F: Fn(&mut S, &T) -> R + Sync + Send,
{
    #[cfg(feature = "parallel")]
    {
        let chunks: Vec<Vec<R>> = items
            .par_chunks(chunk.max(1))
            .map_init(&init, |scratch, chunk| chunk.iter().map(|i| work(scratch, i)).collect())
            .collect();
        chunks.into_iter().flatten().collect()
    }
    #[cfg(not(feature = "parallel"))]
    {
        let _ = chunk;
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
    /// How the buses ride the roads, if they do.
    buses: Option<BusPlan>,
    /// Per walk node, filled on first use: the stops a passenger reaches walking from it
    /// (access) and the stops they walk to it from (egress), each with the walk plus the
    /// stop's transfer. The walk layer is static, so a node's lists never change in a run
    /// and are searched once, not once per trip and iteration (S209). Whichever thread
    /// fills an entry, it fills the same list.
    access_memo: Vec<StopWalks>,
    egress_memo: Vec<StopWalks>,
}

/// A walk node's stops and the walk to each, filled once (see [`TransitSetup`]'s memos).
type StopWalks = OnceLock<Box<[(NodeId, u32)]>>;

/// How a timetable's buses ride a road network: see the [module docs](self).
#[derive(Debug)]
struct BusPlan {
    road: Arc<RoadNetwork>,
    /// Per pattern group: the road links between each call and the next, or
    /// `None` if the group runs by the schedule.
    hops: Vec<Option<Vec<Vec<LinkId>>>>,
    report: BusReport,
}

/// How the bus patterns were put on the roads (S199).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BusReport {
    /// Bus pattern groups that ride the roads.
    pub groups_on_roads: u32,
    /// Their runs.
    pub runs_on_roads: u32,
    /// Bus groups run by the schedule because a stop is too far from the roads.
    pub by_schedule_off_road: u32,
    /// Because the roads do not connect a stop to the next.
    pub by_schedule_no_route: u32,
    /// Because their free-flow time on the roads is implausible against the schedule.
    pub by_schedule_implausible: u32,
}

/// A day's bus runs as vehicles for one loading (S199).
#[derive(Debug)]
pub(crate) struct BusLoad {
    /// The vehicles, in chain order: a leg after the one it follows.
    pub vehicles: Vec<Vehicle>,
    /// `(vehicle, after, wait, not_before)` by index into `vehicles`.
    pub chains: Vec<(usize, usize, f64, f64)>,
    /// Per road-running run: the vehicle of each hop (call `k` to `k + 1`), or
    /// `u32::MAX` for a hop with no road links.
    legs: Vec<(TransitRunId, Vec<u32>)>,
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
                        parkings: Vec::new(),
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
            buses: None,
            access_memo: (0..nodes).map(|_| OnceLock::new()).collect(),
            egress_memo: (0..nodes).map(|_| OnceLock::new()).collect(),
        }
    }

    /// The same setup, with its buses riding `road` (see the [module docs](self)).
    /// Only a run on this very network loads them; any other runs them by the
    /// schedule.
    ///
    /// # Panics
    ///
    /// Never in practice: every run of a timetable has two calls or more.
    #[must_use]
    pub fn with_roads(mut self, road: Arc<RoadNetwork>) -> Self {
        // Road routes between two nodes, found once: the links and their free-flow time.
        type Found = Option<(Vec<LinkId>, f64)>;
        let d = self.defaults;
        let t = &self.timetable;
        let turns = TurnTable::build(&road, SignalDefaults::SHIPPED);
        // Buses may use busways as well as the roads cars use (S199).
        let costs: Vec<f64> = (0..road.link_count())
            .map(|i| {
                let link = LinkId::new(i);
                if road.link_class(link).carries_buses() {
                    road.free_flow_time(link).get()
                } else {
                    f64::INFINITY
                }
            })
            .collect();
        let ctx = SearchContext::with_costs(&road, &turns, costs);
        let mut search = Search::new(&ctx);
        let snapper = NodeSnapper::of_links(&road, |class| class.carries_buses());
        let mut report = BusReport::default();
        let mut routes: HashMap<(u32, u32), Found> = HashMap::new();
        let mut hops = Vec::with_capacity(t.group_count() as usize);
        for g in 0..t.group_count() {
            let runs = t.group_runs(g);
            let first = runs[0];
            if !t.run_kind(first).rides_road() {
                hops.push(None);
                continue;
            }
            let calls: Vec<usize> = t.run_calls(first).collect();
            let nodes: Option<Vec<NodeId>> = calls
                .iter()
                .map(|&c| {
                    let at = t.stop_position(t.call_stop(c));
                    let node = snapper.nearest(&road, at);
                    (ground_distance_metres(at, road.node_lonlat(node)) <= d.bus_stop_snap_m)
                        .then_some(node)
                })
                .collect();
            let Some(nodes) = nodes else {
                report.by_schedule_off_road += 1;
                hops.push(None);
                continue;
            };
            let mut legs = Vec::with_capacity(nodes.len() - 1);
            let mut free_flow = 0.0;
            for pair in nodes.windows(2) {
                let found = routes.entry((pair[0].raw(), pair[1].raw())).or_insert_with(|| {
                    search.shortest(pair[0], pair[1]).map(|r| (r.links, r.cost))
                });
                match found {
                    Some((links, cost)) => {
                        legs.push(links.clone());
                        free_flow += *cost;
                    }
                    None => break,
                }
            }
            if legs.len() + 1 < nodes.len() {
                report.by_schedule_no_route += 1;
                hops.push(None);
                continue;
            }
            #[allow(clippy::cast_precision_loss, reason = "a handful of stops")]
            let dwells = d.bus_dwell_s * (calls.len().saturating_sub(2)) as f64;
            let scheduled = f64::from(
                t.scheduled().arrival[*calls.last().expect("two calls")]
                    .saturating_sub(t.scheduled().departure[calls[0]]),
            );
            if free_flow + dwells > d.bus_plausibility_ratio * scheduled.max(60.0) {
                report.by_schedule_implausible += 1;
                hops.push(None);
                continue;
            }
            report.groups_on_roads += 1;
            report.runs_on_roads += u32::try_from(runs.len()).expect("runs fit u32");
            hops.push(Some(legs));
        }
        self.buses = Some(BusPlan { road, hops, report });
        self
    }

    /// How the buses were put on the roads, if they ride them.
    #[must_use]
    pub fn bus_report(&self) -> Option<BusReport> {
        self.buses.as_ref().map(|b| b.report)
    }

    /// Whether a run on `road` loads this timetable's buses.
    #[must_use]
    pub fn rides(&self, road: &Arc<RoadNetwork>) -> bool {
        self.buses
            .as_ref()
            .is_some_and(|b| Arc::ptr_eq(&b.road, road) && b.report.runs_on_roads > 0)
    }

    /// The day's road-running runs as chained vehicles, their ids from `first_id` up.
    pub(crate) fn bus_load(&self, first_id: u32) -> BusLoad {
        let d = self.defaults;
        let t = &self.timetable;
        let plan = self.buses.as_ref().expect("only called when the buses ride");
        let mut load = BusLoad { vehicles: Vec::new(), chains: Vec::new(), legs: Vec::new() };
        for (g, hops) in plan.hops.iter().enumerate() {
            let Some(hops) = hops else { continue };
            for &run in t.group_runs(u32::try_from(g).expect("groups fit u32")) {
                let calls: Vec<usize> = t.run_calls(run).collect();
                let sched = t.scheduled();
                // The departure rule for the next leg: max(leader's arrival + wait, not_before);
                // with no leader yet, `not_before` alone.
                let (mut wait, mut not_before) = (0.0, f64::from(sched.departure[calls[0]]));
                let mut leader: Option<usize> = None;
                let mut legs = Vec::with_capacity(hops.len());
                for (k, links) in hops.iter().enumerate() {
                    let last = k + 2 == calls.len();
                    if links.is_empty() {
                        legs.push(u32::MAX);
                    } else {
                        let index = load.vehicles.len();
                        let id = first_id + u32::try_from(index).expect("vehicles fit u32");
                        #[allow(
                            clippy::cast_possible_truncation,
                            clippy::cast_sign_loss,
                            reason = "a scheduled second"
                        )]
                        let departure = Second(not_before.max(0.0) as u32);
                        load.vehicles.push(Vehicle::new(
                            VehicleId::new(id),
                            links.clone(),
                            Pcu(d.bus_pcu),
                            departure,
                        ));
                        if let Some(a) = leader {
                            load.chains.push((index, a, wait, not_before));
                        }
                        leader = Some(index);
                        legs.push(u32::try_from(index).expect("vehicles fit u32"));
                        (wait, not_before) = (0.0, f64::NEG_INFINITY);
                    }
                    // Then the call at the end of the hop: its dwell and its schedule.
                    if !last {
                        let at_next = f64::from(sched.departure[calls[k + 1]]);
                        if leader.is_some() {
                            (wait, not_before) =
                                (wait + d.bus_dwell_s, (not_before + d.bus_dwell_s).max(at_next));
                        } else {
                            not_before = (not_before + d.bus_dwell_s).max(at_next);
                        }
                    }
                }
                load.legs.push((run, legs));
            }
        }
        load
    }

    /// How the road-running runs of `load` kept to the schedule on `times`.
    pub(crate) fn bus_summary(&self, load: &BusLoad, times: &CallTimes) -> BusSummary {
        let t = &self.timetable;
        let (mut arrived, mut delay) = (0u32, 0.0f64);
        for (run, _) in &load.legs {
            let last = t.run_calls(*run).end - 1;
            if times.arrival[last] != UNKNOWN_TIME {
                arrived += 1;
                delay += f64::from(times.arrival[last]) - f64::from(t.scheduled().arrival[last]);
            }
        }
        BusSummary {
            runs_on_roads: u32::try_from(load.legs.len()).expect("runs fit u32"),
            runs_arrived: arrived,
            delay_mean_s: if arrived == 0 { f64::NAN } else { delay / f64::from(arrived) },
        }
    }

    /// Every call's times after a loading: the schedule's, with the road-running
    /// runs' from their legs' trajectories (`trajectory` by index into
    /// `load.vehicles`; `None` for a leg that did not arrive in the window, whose
    /// calls from there on are [`UNKNOWN_TIME`]).
    pub(crate) fn realised<'t>(
        &self,
        load: &BusLoad,
        trajectory: impl Fn(usize) -> Option<&'t Trajectory>,
    ) -> CallTimes {
        let d = self.defaults;
        let t = &self.timetable;
        let mut times = t.scheduled().clone();
        for (run, legs) in &load.legs {
            let calls: Vec<usize> = t.run_calls(*run).collect();
            let sched = t.scheduled();
            let mut known = true;
            for (k, &leg) in legs.iter().enumerate() {
                let (from, to) = (calls[k], calls[k + 1]);
                if !known {
                    times.departure[from] = UNKNOWN_TIME;
                    times.arrival[to] = UNKNOWN_TIME;
                    continue;
                }
                if leg == u32::MAX {
                    if k > 0 {
                        #[allow(
                            clippy::cast_possible_truncation,
                            clippy::cast_sign_loss,
                            reason = "a second"
                        )]
                        let dep = (f64::from(times.arrival[from]) + d.bus_dwell_s)
                            .max(f64::from(sched.departure[from]))
                            as u32;
                        times.departure[from] = dep;
                    }
                    times.arrival[to] = times.departure[from];
                } else if let Some(tr) = trajectory(leg as usize) {
                    times.departure[from] = tr.links[0].enter.get();
                    times.arrival[to] = tr.arrival().get();
                } else {
                    known = false;
                    times.departure[from] = UNKNOWN_TIME;
                    times.arrival[to] = UNKNOWN_TIME;
                }
            }
            let last = *calls.last().expect("two calls");
            times.departure[last] = times.arrival[last];
        }
        times
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

    /// The stops within `bound` seconds' walk of the walk layer node `from`, and
    /// the walk to each in whole seconds (M4: the stops around a parking).
    #[must_use]
    pub fn stops_within(&self, from: NodeId, bound: f64) -> Vec<(NodeId, u32)> {
        let mut reach = Reach::new(self.walk.network(), &self.walk_seconds);
        self.stops_from(&mut reach, from, bound)
    }

    /// [`Self::stops_within`] with the caller's search scratch.
    pub(crate) fn stops_from(
        &self,
        reach: &mut Reach<'_>,
        from: NodeId,
        bound: f64,
    ) -> Vec<(NodeId, u32)> {
        let mut out = Vec::new();
        for (node, secs) in reach.forward(from, bound, &self.has_stop) {
            let s = floor_seconds(secs);
            out.extend(self.stops_at(node).iter().map(|&stop| (stop, s)));
        }
        out
    }

    /// The stops a passenger leaving walk node `origin` at `departure` can walk to,
    /// each with the second they are there (the walk, then the stop's transfer).
    pub(crate) fn access(
        &self,
        reach: &mut Reach<'_>,
        origin: NodeId,
        departure: u32,
    ) -> Vec<(NodeId, u32)> {
        let walks = self.access_memo[origin.index()].get_or_init(|| {
            let transfer = self.transfer_s();
            self.stops_from(reach, origin, self.defaults.access_walk_max_s)
                .into_iter()
                .map(|(stop, w)| (stop, w.saturating_add(transfer)))
                .collect()
        });
        walks.iter().map(|&(stop, w)| (stop, departure.saturating_add(w))).collect()
    }

    /// The stops a passenger can walk from to walk node `destination`, each with the
    /// walk (and the stop's transfer) in seconds.
    pub(crate) fn egress(&self, reach: &mut Reach<'_>, destination: NodeId) -> Vec<(NodeId, u32)> {
        self.egress_memo[destination.index()]
            .get_or_init(|| {
                let transfer = self.transfer_s();
                let mut out = Vec::new();
                for (node, secs) in
                    reach.backward(destination, self.defaults.access_walk_max_s, &self.has_stop)
                {
                    let w = floor_seconds(secs).saturating_add(transfer);
                    out.extend(self.stops_at(node).iter().map(|&stop| (stop, w)));
                }
                out.into_boxed_slice()
            })
            .to_vec()
    }

    /// The links of the shortest walk from walk node `from` to walk node `to`, if it
    /// takes at most `bound` seconds (none if they are one node). `wanted` is the
    /// caller's scratch, one `false` per walk node, left as it was found.
    pub(crate) fn walk_path(
        reach: &mut Reach<'_>,
        wanted: &mut [bool],
        from: NodeId,
        to: NodeId,
        bound: f64,
    ) -> Option<Vec<LinkId>> {
        if from == to {
            return Some(Vec::new());
        }
        wanted[to.index()] = true;
        let found = !reach.forward(from, bound, wanted).is_empty();
        wanted[to.index()] = false;
        found.then(|| reach.path(to))
    }

    /// The walk layer's seconds per link.
    pub(crate) fn walk_link_seconds(&self) -> &[f64] {
        &self.walk_seconds
    }

    /// How many nodes the walk layer has.
    pub(crate) fn walk_node_count(&self) -> usize {
        self.has_stop.len()
    }

    /// The time a transfer at a stop takes, in whole seconds.
    pub(crate) fn transfer_s(&self) -> u32 {
        floor_seconds(self.defaults.stop_transfer_s)
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
    /// How the buses kept to the schedule, if they rode the roads.
    pub buses: Option<BusSummary>,
}

/// How the buses that rode the roads kept to their schedule in a run (S199).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BusSummary {
    /// Runs loaded on the roads.
    pub runs_on_roads: u32,
    /// Of those, runs that reached their last stop within the window.
    pub runs_arrived: u32,
    /// Their mean delay at the last stop, realised minus scheduled, in seconds
    /// (negative: early); NaN if none arrived.
    pub delay_mean_s: f64,
}

impl TransitResult {
    /// Nobody on board yet, on `times`.
    #[must_use]
    pub fn empty(times: CallTimes) -> Self {
        let n = times.arrival.len();
        Self { times, boardings: vec![0.0; n], alightings: vec![0.0; n], buses: None }
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
