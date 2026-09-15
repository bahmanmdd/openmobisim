//! S85's vehicles-on-curves, run: the iterative LTM (S84) plus the node
//! model (S48/S77) plus vehicle hand-over, stepped on the S88 loading grid.
//!
//! This is **Phase 2 item 2's opening prototype** (S85's flagged risk):
//! proving the within-step, multi-link vehicle
//! hand-over is correct on a small network before it has to run on a real
//! one. It is deliberately narrower than the eventual production motor:
//!
//! - **No warm start, no change-restricted work.** Himpe/Corthout/Tampère's
//!   "iterative" LTM (S84's citation) restricts each step's recomputation to
//!   the parts of the network that changed; this prototype recomputes every
//!   link and every node every step. That is the performance work item 2's
//!   cold-run benchmark still owes, not a correctness gap.
//! - **Only completed trajectories are reported.** `core-sim`'s truncation
//!   accounting (S57) is not wired up here — that integration is separate,
//!   later work; this module is proven and tested standing alone.
//! - **Departures are coarsened to the step grid** (S88's "only the road
//!   loading is coarse"): a vehicle departing inside step *s* is injected at
//!   step *s*'s boundary, not at its exact second.
//! - **A chained vehicle's exit time is linearly interpolated across its
//!   link's whole-step advance**, even when the vehicle joined the queue
//!   partway through the step (clamped to never precede its own entry). This
//!   is the one place sub-step precision is traded for simplicity; it does
//!   not affect conservation, only the reported instant within a step.
//!
//! # The algorithm, per step
//!
//! 1. **Sending and receiving flow**, per link, from [`LinkCurves`] at the
//!    step's start — never touches this step's own commits (S84's
//!    no-stability-limit property).
//! 2. **Per-turn demand**, read by front-loading each link's FIFO queue up to
//!    its sending-flow budget and grouping by next link (S85).
//! 3. **The node model** (S48/S77), once per node, Jacobi — every node reads
//!    only the demand/supply captured in steps 1–2, so node order cannot
//!    change the result.
//! 4. **Commit every link's curves** — `N_up` by the accepted turns arriving,
//!    `N_dn` by the incoming link's full-blocking factor (S77) applied to its
//!    *entire* sending flow, arrivals included.
//! 5. **Vehicle hand-over**, a fixed point over the now-fixed step: pop any
//!    queue-front vehicle whose position the committed `N_dn` has passed,
//!    hand it to its next link (which may in turn let it pop again in the
//!    same pass) or finish its trajectory. Repeated passes are what let a
//!    vehicle cross several links in one step — the risk S85 names.

use std::collections::{HashMap, VecDeque};

use openmobisim_core_graph::defaults::LinkParameters;
use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_types::ids::{EntityId, LinkId, NodeId, VehicleId};
use openmobisim_core_types::time::Second;
use openmobisim_core_types::units::{Duration, Flow, Pcu, Speed};

use crate::curves::LinkCurves;
use crate::level0::{LinkTraversal, Trajectory};
use crate::node_model::{TurnDemand, solve_node};
use crate::vehicle::Vehicle;

/// Which term of the triangular diagram this run keeps — design §10.1's
/// nesting: levels 2–4 are **the same code**, called with a limiting
/// parameter overridden (S76). Levels 0 and 1 are separate mechanisms
/// (free-flow traversal, `crate::level0`; volume-delay, not yet built) and
/// are not represented here.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum FidelityLevel {
    /// Level 2, *point queue*: infinite storage. Nothing a link holds can
    /// ever block its upstream neighbour — only the link's own discharge
    /// capacity limits it. Loses queue length / physical extent.
    PointQueue,
    /// Level 3, *spatial queue*: infinite backward wave speed. Storage is
    /// real (spillback happens), but the "room freed up" signal reaches the
    /// upstream end instantly rather than at a finite shockwave speed.
    /// Loses discharge-timing accuracy.
    SpatialQueue,
    /// Level 4, the full triangular diagram, as measured/configured. What
    /// both shipped presets run (design §10.1).
    #[default]
    Full,
}

/// One vehicle, queued on the link it currently occupies.
#[derive(Clone, Copy)]
struct QueuedVehicle<'a> {
    vehicle: &'a Vehicle,
    link_index: usize,
    /// This vehicle exits its current link once that link's `N_dn` reaches
    /// this value (S85: "leaves when the downstream curve passes its
    /// position").
    exit_boundary: Pcu,
    enter_time: Duration,
}

impl<'a> QueuedVehicle<'a> {
    fn next_link(&self) -> Option<LinkId> {
        self.vehicle.route.get(self.link_index + 1).copied()
    }
}

/// The running state of a loading: every link's curves and queue.
pub struct LtmNetwork<'a> {
    network: &'a RoadNetwork,
    turns: &'a TurnTable,
    curves: Vec<LinkCurves>,
    queues: Vec<VecDeque<QueuedVehicle<'a>>>,
    in_progress: HashMap<VehicleId, Vec<LinkTraversal>>,
    level: FidelityLevel,
}

impl<'a> LtmNetwork<'a> {
    /// An empty network at level 4 (the full diagram): every link starts
    /// with no traffic.
    #[must_use]
    pub fn new(network: &'a RoadNetwork, turns: &'a TurnTable) -> Self {
        let n = network.link_count() as usize;
        Self {
            network,
            turns,
            curves: (0..n).map(|_| LinkCurves::new()).collect(),
            queues: (0..n).map(|_| VecDeque::new()).collect(),
            in_progress: HashMap::new(),
            level: FidelityLevel::Full,
        }
    }

    /// The same network, run at `level` instead of the default (S76's
    /// nesting: the rest of the engine is unchanged).
    #[must_use]
    pub fn with_level(mut self, level: FidelityLevel) -> Self {
        self.level = level;
        self
    }

    /// One link's cumulative curves, for tests and diagnostics.
    #[must_use]
    pub fn curve(&self, link: LinkId) -> &LinkCurves {
        &self.curves[link.index()]
    }

    /// How many vehicles are currently queued on a link.
    #[must_use]
    pub fn queue_len(&self, link: LinkId) -> usize {
        self.queues[link.index()].len()
    }

    /// Put a vehicle onto the first link of its route, at `now`.
    fn depart(&mut self, vehicle: &'a Vehicle, now: Duration) {
        let first = vehicle.route[0];
        self.curves[first.index()].inject_up(vehicle.pcu);
        let boundary = self.curves[first.index()].up_now();
        self.queues[first.index()].push_back(QueuedVehicle {
            vehicle,
            link_index: 0,
            exit_boundary: boundary,
            enter_time: now,
        });
        self.in_progress.insert(vehicle.id, Vec::with_capacity(vehicle.route.len()));
    }

    /// The smallest link free-flow time in the network (including S90's
    /// control delay, which only ever makes this bound safer).
    ///
    /// Bounds the sweep granularity [`Self::step`] subdivides `dt` into:
    /// evaluating sending/receiving flow and committing a step no coarser
    /// than this cannot smear a single vehicle's release across a link it
    /// crosses in far less time than that — the failure mode a step run
    /// straight at the macro `dt` produces (see the module docs).
    fn min_free_flow_time(&self) -> Duration {
        let mut min = Duration::INFINITE;
        for idx in 0..self.network.link_count() {
            let t = self.network.free_flow_time(LinkId::from_index(idx as usize));
            if t.get() > 0.0 && t < min {
                min = t;
            }
        }
        if min.is_finite() { min } else { Duration(1.0) }
    }

    /// Advance every link by `dt`, internally subdivided into sweeps no
    /// coarser than the network's smallest link free-flow time (S84: "solved
    /// by Jacobi sweeps"). Returns every trajectory that reached the end of
    /// its route during `dt`. `dt` itself carries no stability limit (S84) —
    /// only the sweep granularity this method chooses internally does.
    pub fn step(&mut self, dt: Duration) -> Vec<Trajectory> {
        if self.network.link_count() == 0 {
            return Vec::new();
        }
        let sweep_dt = self.min_free_flow_time().min(dt).max(Duration(1.0));
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "dt and sweep_dt are finite and positive"
        )]
        let n_sweeps = (dt.get() / sweep_dt.get()).ceil().max(1.0) as usize;
        #[allow(clippy::cast_precision_loss, reason = "n_sweeps is a small sweep count")]
        let sweep_dt = dt / (n_sweeps as f64);
        let mut completed = Vec::new();
        for _ in 0..n_sweeps {
            completed.extend(self.sweep(sweep_dt));
        }
        completed
    }

    /// One Jacobi sweep of the node model over the whole network, at
    /// granularity `dt` — see the module docs for the five-step algorithm.
    fn sweep(&mut self, dt: Duration) -> Vec<Trajectory> {
        let link_count = self.network.link_count() as usize;
        if link_count == 0 {
            return Vec::new();
        }
        let step_start = self.curves[0].now();

        // 1–2. Sending/receiving flow and per-link turn demand.
        let mut sending = vec![Flow::ZERO; link_count];
        let mut receiving = vec![Flow::ZERO; link_count];
        let mut turn_pcu: Vec<Vec<(LinkId, Pcu)>> = vec![Vec::new(); link_count];
        for idx in 0..link_count {
            let link = LinkId::from_index(idx);
            let params = self.network.link_parameters(link);
            let length = self.network.link_length(link);
            // S76: levels 2-4 are this same code, with one term of the
            // diagram overridden to its limiting value.
            let storage = match self.level {
                FidelityLevel::PointQueue => Pcu::INFINITE,
                FidelityLevel::SpatialQueue | FidelityLevel::Full => self.network.storage(link),
            };
            let receiving_params = match self.level {
                FidelityLevel::SpatialQueue => {
                    LinkParameters { wave_speed: Speed::INFINITE, ..params }
                }
                FidelityLevel::PointQueue | FidelityLevel::Full => params,
            };
            sending[idx] = self.curves[idx].sending_flow(dt, params, length);
            receiving[idx] = self.curves[idx].receiving_flow(dt, receiving_params, length, storage);

            let budget = sending[idx] * dt;
            let mut used = Pcu::ZERO;
            for qv in &self.queues[idx] {
                if used >= budget {
                    break;
                }
                used += qv.vehicle.pcu;
                if let Some(next) = qv.next_link() {
                    match turn_pcu[idx].iter_mut().find(|(l, _)| *l == next) {
                        Some((_, p)) => *p += qv.vehicle.pcu,
                        None => turn_pcu[idx].push((next, qv.vehicle.pcu)),
                    }
                }
            }
        }

        // 3. The node model, once per node, Jacobi (reads only the arrays above).
        let mut up_gain = vec![Pcu::ZERO; link_count];
        let mut alpha = vec![1.0f64; link_count];
        for node in NodeId::iter_space(self.network.node_count()) {
            let node_turns = self.turns.turns_at(node);
            if node_turns.is_empty() {
                continue;
            }
            let demands: Vec<TurnDemand> = node_turns
                .iter()
                .map(|&t| {
                    let in_link = self.turns.incoming(t);
                    let out_link = self.turns.outgoing(t);
                    let pcu = turn_pcu[in_link.index()]
                        .iter()
                        .find(|(l, _)| *l == out_link)
                        .map_or(Pcu::ZERO, |(_, p)| *p);
                    TurnDemand { in_link, out_link, demand: pcu / dt }
                })
                .collect();
            let accepted = solve_node(&demands, |l| receiving[l.index()]);
            for (d, &a) in demands.iter().zip(&accepted) {
                up_gain[d.out_link.index()] += a * dt;
                if d.demand.get() > 0.0 {
                    let factor = (a / d.demand).clamp(0.0, 1.0);
                    let slot = &mut alpha[d.in_link.index()];
                    *slot = slot.min(factor);
                }
            }
        }

        // 4. Commit every link's curves.
        let mut dn_before = vec![Pcu::ZERO; link_count];
        let mut dn_after = vec![Pcu::ZERO; link_count];
        for idx in 0..link_count {
            dn_before[idx] = self.curves[idx].dn_now();
            let dn_gain = (sending[idx] * dt) * alpha[idx];
            self.curves[idx].commit(dt, up_gain[idx], dn_gain);
            dn_after[idx] = self.curves[idx].dn_now();
        }

        // 5. Vehicle hand-over: a fixed point over this step's fixed curves.
        let mut completed = Vec::new();
        let safety_cap = link_count.saturating_mul(4) + 16;
        for _ in 0..safety_cap {
            let mut moved = false;
            for idx in 0..link_count {
                while let Some(front) = self.queues[idx].front() {
                    if front.exit_boundary > dn_after[idx] {
                        break;
                    }
                    let qv = self.queues[idx].pop_front().expect("front was just checked");
                    moved = true;
                    let exit_time = interpolate_exit_time(
                        dn_before[idx],
                        dn_after[idx],
                        step_start,
                        dt,
                        qv.exit_boundary,
                    )
                    .max(qv.enter_time);
                    let link = LinkId::from_index(idx);
                    self.hand_over(&qv, link, exit_time, &mut completed);
                }
            }
            if !moved {
                break;
            }
        }
        completed
    }

    /// Record `qv`'s traversal of `link` and either chain it onto its next
    /// link or finish its trajectory.
    fn hand_over(
        &mut self,
        qv: &QueuedVehicle<'a>,
        link: LinkId,
        exit_time: Duration,
        completed: &mut Vec<Trajectory>,
    ) {
        let traversal = LinkTraversal {
            link,
            enter: floor_to_second(qv.enter_time),
            exit: floor_to_second(exit_time),
        };
        let segments = self
            .in_progress
            .get_mut(&qv.vehicle.id)
            .expect("a queued vehicle is always in progress");
        segments.push(traversal);

        match qv.next_link() {
            Some(next) => {
                self.curves[next.index()].inject_up(qv.vehicle.pcu);
                let boundary = self.curves[next.index()].up_now();
                self.queues[next.index()].push_back(QueuedVehicle {
                    vehicle: qv.vehicle,
                    link_index: qv.link_index + 1,
                    exit_boundary: boundary,
                    enter_time: exit_time,
                });
            }
            None => {
                let links = self.in_progress.remove(&qv.vehicle.id).expect("just pushed to above");
                completed.push(Trajectory { vehicle: qv.vehicle.id, links });
            }
        }
    }
}

/// Where in `[t_start, t_start + dt)` `N_dn` crosses `boundary`, linearly.
fn interpolate_exit_time(
    dn_before: Pcu,
    dn_after: Pcu,
    t_start: Duration,
    dt: Duration,
    boundary: Pcu,
) -> Duration {
    let span = (dn_after - dn_before).get();
    if span <= 0.0 {
        return t_start + dt;
    }
    let frac = ((boundary - dn_before).get() / span).clamp(0.0, 1.0);
    t_start + dt * frac
}

/// Floor a fractional [`Duration`] onto the whole-second clock (S88).
fn floor_to_second(d: Duration) -> Second {
    debug_assert!(d.get().is_finite(), "an exit time must be finite, got {d:?}");
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "d is finite and non-negative by construction (curve times only advance from \
                  zero); truncating is S88's deliberate floor to a whole second"
    )]
    let whole_seconds = d.get().max(0.0) as u32;
    Second(whole_seconds)
}

/// Run every vehicle to completion (or the end of `window`, whichever comes
/// first): the whole-network entry point.
///
/// Departures are coarsened onto the `step` grid (S88); vehicles still in
/// flight when `window` ends are **not** reported — see the module docs.
/// `level` picks which term of the triangular diagram is in force (S76):
/// pass [`FidelityLevel::Full`] for the diagram as measured/configured —
/// what both shipped presets run (design §10.1).
///
/// # Panics
///
/// Panics if `step` is not positive.
#[must_use]
pub fn run_ltm(
    network: &RoadNetwork,
    turns: &TurnTable,
    vehicles: &[Vehicle],
    window: Duration,
    step: Duration,
    level: FidelityLevel,
) -> Vec<Trajectory> {
    assert!(step.get() > 0.0, "the loading step must be positive, got {step:?}");
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "window and step are finite and non-negative scenario parameters"
    )]
    let n_steps = (window.get() / step.get()).ceil().max(0.0) as usize;

    let mut completed = Vec::new();
    if n_steps > 0 {
        let mut pending: Vec<Vec<&Vehicle>> = vec![Vec::new(); n_steps];
        for vehicle in vehicles {
            let departure = Duration::from_clock(vehicle.departure);
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "departure and step are finite and non-negative"
            )]
            let idx = ((departure.get() / step.get()).floor().max(0.0) as usize).min(n_steps - 1);
            pending[idx].push(vehicle);
        }

        let mut sim = LtmNetwork::new(network, turns).with_level(level);
        for (s, vehicles_this_step) in pending.into_iter().enumerate() {
            #[allow(
                clippy::cast_precision_loss,
                reason = "s is small: a step count, not a byte count"
            )]
            let now = step * (s as f64);
            for vehicle in vehicles_this_step {
                sim.depart(vehicle, now);
            }
            completed.extend(sim.step(step));
        }
    }

    // `n_steps * step` can overshoot `window` when it is not an exact
    // multiple of `step` (S88's coarse grid): only report vehicles that
    // actually arrived within `window` itself, not within the step that
    // happened to contain it.
    completed.retain(|t| Duration::from_clock(t.arrival()) <= window);
    completed
}

#[cfg(test)]
mod tests {
    use openmobisim_core_graph::defaults::{GlobalMultipliers, RoadClass, SignalDefaults};
    use openmobisim_core_graph::geometry::LonLat;
    use openmobisim_core_graph::network::{LinkSpec, RoadNetworkBuilder};
    use openmobisim_core_types::diagnostics::Diagnostics;
    use openmobisim_core_types::ids::EntityId;

    use super::*;

    fn line_network() -> RoadNetwork {
        let mut b = RoadNetworkBuilder::new();
        b.add_node("a", LonLat::new(4.800, 45.700));
        b.add_node("b", LonLat::new(4.801, 45.700));
        b.add_node("c", LonLat::new(4.802, 45.700));
        b.add_link("ab", "a", "b", LinkSpec::new(RoadClass::Residential));
        b.add_link("bc", "b", "c", LinkSpec::new(RoadClass::Residential));
        b.add_link("ba", "b", "a", LinkSpec::new(RoadClass::Residential));
        b.add_link("cb", "c", "b", LinkSpec::new(RoadClass::Residential));
        let mut diagnostics = Diagnostics::new();
        b.build(GlobalMultipliers::default(), SignalDefaults::SHIPPED, &mut diagnostics)
            .expect("buildable")
    }

    fn link(network: &RoadNetwork, external: &str) -> LinkId {
        network.link_external_ids().typed_id_of::<LinkId>(external).expect("known link")
    }

    #[test]
    fn a_lone_vehicle_arrives_close_to_free_flow_time() {
        let network = line_network();
        let turns = TurnTable::build(&network, SignalDefaults::SHIPPED);
        let (ab, bc) = (link(&network, "ab"), link(&network, "bc"));
        let vehicle = Vehicle::new(VehicleId::new(0), vec![ab, bc], Pcu(1.0), Second(0));

        let trajectories = run_ltm(
            &network,
            &turns,
            &[vehicle],
            Duration(600.0),
            Duration(300.0),
            FidelityLevel::Full,
        );

        assert_eq!(trajectories.len(), 1);
        let t = &trajectories[0];
        assert_eq!(t.links.len(), 2);
        let expected_ff = network.free_flow_time(ab).get() + network.free_flow_time(bc).get();
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "test assertion"
        )]
        let expected_ff_s = expected_ff as u32;
        // A lone vehicle on empty links experiences (close to) free-flow time;
        // the loading step's coarsening can add up to one step of slack.
        assert!(t.arrival().get() >= expected_ff_s, "cannot arrive before free-flow time");
        assert!(
            t.arrival().get() <= expected_ff_s + 300,
            "a lone vehicle should not be delayed by more than one loading step, got {t:?}"
        );
    }

    #[test]
    fn a_vehicle_can_cross_several_links_within_one_step() {
        // Three 100 m residential links chained: free-flow time per link is
        // a few seconds, so a 300 s step must let a lone vehicle cross all
        // three — the exact risk S85 flags.
        let mut b = RoadNetworkBuilder::new();
        b.add_node("a", LonLat::new(4.8000, 45.700));
        b.add_node("b", LonLat::new(4.8012, 45.700));
        b.add_node("c", LonLat::new(4.8024, 45.700));
        b.add_node("d", LonLat::new(4.8036, 45.700));
        b.add_link("ab", "a", "b", LinkSpec::new(RoadClass::Residential));
        b.add_link("bc", "b", "c", LinkSpec::new(RoadClass::Residential));
        b.add_link("cd", "c", "d", LinkSpec::new(RoadClass::Residential));
        let mut diagnostics = Diagnostics::new();
        let network = b
            .build(GlobalMultipliers::default(), SignalDefaults::SHIPPED, &mut diagnostics)
            .expect("buildable");
        let turns = TurnTable::build(&network, SignalDefaults::SHIPPED);
        let (ab, bc, cd) = (link(&network, "ab"), link(&network, "bc"), link(&network, "cd"));
        let vehicle = Vehicle::new(VehicleId::new(0), vec![ab, bc, cd], Pcu(1.0), Second(0));

        let trajectories = run_ltm(
            &network,
            &turns,
            &[vehicle],
            Duration(300.0),
            Duration(300.0),
            FidelityLevel::Full,
        );

        assert_eq!(trajectories.len(), 1, "the vehicle must complete within the one step given");
        assert_eq!(trajectories[0].links.len(), 3);
    }

    #[test]
    fn vehicle_count_is_conserved_across_a_congested_run() {
        let network = line_network();
        let turns = TurnTable::build(&network, SignalDefaults::SHIPPED);
        let (ab, bc) = (link(&network, "ab"), link(&network, "bc"));
        let vehicles: Vec<Vehicle> = (0..40)
            .map(|i| Vehicle::new(VehicleId::new(i), vec![ab, bc], Pcu(1.0), Second(i * 5)))
            .collect();

        let trajectories = run_ltm(
            &network,
            &turns,
            &vehicles,
            Duration(3600.0),
            Duration(300.0),
            FidelityLevel::Full,
        );

        assert!(
            trajectories.len() <= 40,
            "cannot report more completed trajectories than vehicles departed"
        );
        let mut seen: Vec<u32> = trajectories.iter().map(|t| t.vehicle.raw()).collect();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), trajectories.len(), "no vehicle is reported twice");
        for t in &trajectories {
            assert_eq!(t.links.len(), 2, "every completed trip has both links");
            assert!(
                t.links[1].exit >= t.links[0].exit,
                "exit times are non-decreasing along a route"
            );
        }
    }

    #[test]
    fn curves_are_monotone_and_conservative() {
        let network = line_network();
        let turns = TurnTable::build(&network, SignalDefaults::SHIPPED);
        let (ab, bc) = (link(&network, "ab"), link(&network, "bc"));
        let vehicles: Vec<Vehicle> = (0..20)
            .map(|i| Vehicle::new(VehicleId::new(i), vec![ab, bc], Pcu(1.0), Second(i * 30)))
            .collect();

        let mut sim = LtmNetwork::new(&network, &turns);
        for s in 0..12u32 {
            for v in &vehicles {
                if v.departure.get() / 300 == s {
                    sim.depart(v, Duration(f64::from(s) * 300.0));
                }
            }
            let _ = sim.step(Duration(300.0));
        }
        for idx in 0..network.link_count() {
            let link = LinkId::from_index(idx as usize);
            let c = sim.curve(link);
            assert!(
                c.up_now().get() >= c.dn_now().get() - 1e-9,
                "a link cannot discharge more than it received"
            );
            assert!(c.dn_now().get() >= 0.0);
        }
    }

    /// S76's exact-assertion pair, exercised through the whole engine (the
    /// unit tests in `crate::curves` prove the formulas directly; this
    /// proves the [`FidelityLevel`] wiring actually reaches the node model
    /// and changes behaviour end to end).
    ///
    /// `bc` is short (tiny real storage) and low-capacity, downstream of
    /// higher-capacity `ab`: a burst of demand overwhelms `bc` and, under
    /// the full diagram, spillback throttles `ab`'s own discharge (S77).
    /// Under [`FidelityLevel::PointQueue`] (infinite storage) that cannot
    /// happen — `bc` never blocks upstream, only its own capacity limits
    /// it — so `ab` must discharge at least as much in the same time, and
    /// strictly more once spillback has genuinely bound under level 4.
    #[test]
    fn infinite_storage_discharges_at_least_as_much_as_real_spillback() {
        let mut b = RoadNetworkBuilder::new();
        b.add_node("a", LonLat::new(4.8000, 45.700));
        b.add_node("b", LonLat::new(4.8012, 45.700));
        b.add_node("c", LonLat::new(4.8014, 45.700)); // "bc" ~15 m: tiny storage
        b.add_node("d", LonLat::new(4.8026, 45.700));
        b.add_link("ab", "a", "b", LinkSpec::new(RoadClass::Secondary));
        b.add_link("bc", "b", "c", LinkSpec::new(RoadClass::Service));
        b.add_link("cd", "c", "d", LinkSpec::new(RoadClass::Service));
        let mut diagnostics = Diagnostics::new();
        let network = b
            .build(GlobalMultipliers::default(), SignalDefaults::SHIPPED, &mut diagnostics)
            .expect("buildable");
        let turns = TurnTable::build(&network, SignalDefaults::SHIPPED);
        let (ab, bc, cd) = (link(&network, "ab"), link(&network, "bc"), link(&network, "cd"));
        assert!(
            network.storage(bc).get() < 5.0,
            "the fixture needs bc's real storage to be small enough to saturate quickly, got {:?}",
            network.storage(bc)
        );

        let vehicles: Vec<Vehicle> = (0..200)
            .map(|i| Vehicle::new(VehicleId::new(i), vec![ab, bc, cd], Pcu(1.0), Second(0)))
            .collect();

        let ab_discharge_after = |level: FidelityLevel| -> f64 {
            let mut sim = LtmNetwork::new(&network, &turns).with_level(level);
            for v in &vehicles {
                sim.depart(v, Duration::ZERO);
            }
            for _ in 0..10 {
                let _ = sim.step(Duration(300.0));
            }
            sim.curve(ab).dn_now().get()
        };

        let full = ab_discharge_after(FidelityLevel::Full);
        let point_queue = ab_discharge_after(FidelityLevel::PointQueue);

        assert!(
            point_queue > full + 1e-6,
            "infinite storage must discharge strictly more than real spillback once it has \
             bound: full={full}, point_queue={point_queue}"
        );
    }
}
