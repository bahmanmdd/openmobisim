//! What a policy asks for: the decisions, their checks, refusals and execution, and the record
//! of each.
//!
//! | Decision | Checked (refused with a reason otherwise) | Carried out |
//! |---|---|---|
//! | [`Decision::Move`] | a docked service; both docks exist and differ; `n ≥ 1`; the service's docks have road nodes; a bike at the first dock; a road between | `min(n, bikes there, van_bikes)` bikes leave at once; the van drives the roads' time at that second between the docks' road nodes, plus `handling_s` per bike; they arrive into the free slots, the rest into the nearest docks with room (the dock's neighbours, nearest first), any left docked above capacity |
//! | [`Decision::Relocate`] | a taxi fleet; the vehicle exists and is idle, not already at the node; a road there; charge for the drive and the reserve | the vehicle drives there empty and waits, idle |
//! | [`Decision::Charge`] | a taxi fleet; the vehicle exists and is idle; the charger exists; `to_soc` above its charge and at most 1; a road there; charge to reach it | the vehicle drives there, queues for a plug, charges to `to_soc` at the plug's power |
//! | [`Decision::Target`] | a docked service; the dock exists; `level` at most its slots; one target per dock and step | at the step's end, every service's targets are turned into moves by the built-in solver ([`plan_moves`]), each carried out as a `Move` |
//!
//! Decisions are taken in the order given, each checked against the state after the ones
//! before it (a vehicle cannot be sent twice). Every decision is recorded
//! ([`DecisionRecord`]), and so is every move the solver makes.

use std::sync::Arc;

use openmobisim_core_graph::geometry::{LonLat, ground_distance_metres};
use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_types::ids::{EntityId, NodeId};

use super::engine::{FleetEngine, ServiceRoads, ServiceState, Van};
use super::{ServiceParts, ServiceSetup};

/// What a policy asks for. Services are named by their index in the run's order, docks,
/// vehicles and chargers by their index in the service.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Decision {
    /// Move up to `n` bikes of a docked service from one dock to another by van.
    Move {
        /// The service.
        service: u16,
        /// The dock the bikes are taken from.
        from_dock: u32,
        /// The dock they are brought to.
        to_dock: u32,
        /// How many: as many as there are, up to this and the van's `van_bikes`.
        n: u32,
    },
    /// Send an idle taxi, empty, to a road node, where it waits.
    Relocate {
        /// The service.
        service: u16,
        /// The vehicle.
        vehicle: u32,
        /// Where it goes.
        node: NodeId,
    },
    /// Send an idle taxi to a charger, to charge to a share of its battery.
    Charge {
        /// The service.
        service: u16,
        /// The vehicle.
        vehicle: u32,
        /// The charger.
        charger: u32,
        /// The charge it charges to, as a share of the battery.
        to_soc: f64,
    },
    /// The bikes a dock should hold: the built-in solver moves bikes to reach it.
    Target {
        /// The service.
        service: u16,
        /// The dock.
        dock: u32,
        /// The bikes it should hold.
        level: u32,
    },
}

impl Decision {
    /// Its kind.
    #[must_use]
    pub const fn kind(&self) -> DecisionKind {
        match self {
            Decision::Move { .. } => DecisionKind::Move,
            Decision::Relocate { .. } => DecisionKind::Relocate,
            Decision::Charge { .. } => DecisionKind::Charge,
            Decision::Target { .. } => DecisionKind::Target,
        }
    }

    /// The service it is about.
    #[must_use]
    pub const fn service(&self) -> u16 {
        match *self {
            Decision::Move { service, .. }
            | Decision::Relocate { service, .. }
            | Decision::Charge { service, .. }
            | Decision::Target { service, .. } => service,
        }
    }

    /// Its parameters by name, the service left out.
    #[must_use]
    pub fn parameters(&self) -> Vec<(&'static str, f64)> {
        match *self {
            Decision::Move { from_dock, to_dock, n, .. } => vec![
                ("from_dock", f64::from(from_dock)),
                ("to_dock", f64::from(to_dock)),
                ("n", f64::from(n)),
            ],
            Decision::Relocate { vehicle, node, .. } => {
                vec![("vehicle", f64::from(vehicle)), ("node", f64::from(node.raw()))]
            }
            Decision::Charge { vehicle, charger, to_soc, .. } => vec![
                ("vehicle", f64::from(vehicle)),
                ("charger", f64::from(charger)),
                ("to_soc", to_soc),
            ],
            Decision::Target { dock, level, .. } => {
                vec![("dock", f64::from(dock)), ("level", f64::from(level))]
            }
        }
    }
}

/// A decision's kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DecisionKind {
    /// [`Decision::Move`].
    Move,
    /// [`Decision::Relocate`].
    Relocate,
    /// [`Decision::Charge`].
    Charge,
    /// [`Decision::Target`].
    Target,
}

impl DecisionKind {
    /// The stable snake_case name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            DecisionKind::Move => "move",
            DecisionKind::Relocate => "relocate",
            DecisionKind::Charge => "charge",
            DecisionKind::Target => "target",
        }
    }
}

/// One decision, as the engine took it.
#[derive(Clone, Debug, PartialEq)]
pub struct DecisionRecord {
    /// The policy step it was asked at (its index; the step at the window's start is 0).
    pub step: u32,
    /// That step's second, from midnight.
    pub t_s: f64,
    /// The service.
    pub service: u16,
    /// What it is.
    pub kind: DecisionKind,
    /// Its parameters by name ([`Decision::parameters`]).
    pub parameters: Vec<(&'static str, f64)>,
    /// Whether it was carried out.
    pub accepted: bool,
    /// Why it was refused (`None` if accepted).
    pub reason: Option<String>,
    /// The second it started (`NaN` if refused).
    pub executed_s: f64,
    /// The second it finished: the van's or vehicle's arrival, the charge's end, the target's
    /// solving (`NaN` if refused, or not finished by the window's end).
    pub finished_s: f64,
    /// A move's bikes in the van (0 for the other kinds).
    pub moved: u32,
    /// The drive's metres (`NaN` if refused or not a drive).
    pub metres: f64,
    /// A move made by the solver for the service's targets at this step.
    pub from_target: bool,
}

impl DecisionRecord {
    pub(crate) fn new(step: u32, t: f64, d: &Decision) -> Self {
        Self {
            step,
            t_s: t,
            service: d.service(),
            kind: d.kind(),
            parameters: d.parameters(),
            accepted: false,
            reason: None,
            executed_s: f64::NAN,
            finished_s: f64::NAN,
            moved: 0,
            metres: f64::NAN,
            from_target: false,
        }
    }
}

/// An operator's decisions as a table: one row per decision, in the order taken, every index
/// turned into its id. The rows of `operator_tasks.parquet` and of `Run.operator_decisions()`.
///
/// A parameter a kind does not have is `None` (an id) or `NaN` (a number); so are the times and
/// metres a refused decision has not. **Cost:** about 150 bytes per decision while held.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OperatorTaskTable {
    /// The policy step's index (0 at the window's start).
    pub step: Vec<u32>,
    /// The step's second, from midnight.
    pub t_s: Vec<f64>,
    /// The service's name.
    pub service: Vec<String>,
    /// [`DecisionKind::as_str`]: `move`, `relocate`, `charge` or `target`.
    pub kind: Vec<&'static str>,
    /// A move's dock the bikes are taken from.
    pub from_dock: Vec<Option<String>>,
    /// A move's dock they are brought to.
    pub to_dock: Vec<Option<String>>,
    /// A move's bikes asked for.
    pub n: Vec<f64>,
    /// A target's dock.
    pub dock: Vec<Option<String>>,
    /// A target's bikes the dock should hold.
    pub level: Vec<f64>,
    /// A relocation's or a charge's vehicle.
    pub vehicle: Vec<Option<String>>,
    /// A relocation's road node.
    pub node: Vec<Option<String>>,
    /// A charge's charger.
    pub charger: Vec<Option<String>>,
    /// A charge's battery share to charge to.
    pub to_soc: Vec<f64>,
    /// Whether it was carried out.
    pub accepted: Vec<bool>,
    /// Why it was refused (`None` if accepted).
    pub reason: Vec<Option<String>>,
    /// The second it started.
    pub executed_s: Vec<f64>,
    /// The second it finished: the van's or vehicle's arrival, the charge's end.
    pub finished_s: Vec<f64>,
    /// A move's bikes in the van.
    pub moved: Vec<u32>,
    /// The drive's metres.
    pub metres: Vec<f64>,
    /// A move made by the solver for the step's targets.
    pub from_target: Vec<bool>,
}

impl OperatorTaskTable {
    /// The table of `records`, taken on `setups` (the services, in the run's order, whose docks,
    /// vehicles and chargers the records name by index) and `road` (the network whose nodes
    /// they name).
    #[must_use]
    pub fn new(records: &[DecisionRecord], setups: &[ServiceSetup], road: &RoadNetwork) -> Self {
        let mut t = Self::default();
        for r in records {
            let setup = &setups[usize::from(r.service)];
            let param = |k: &str| r.parameters.iter().find(|p| p.0 == k).map(|p| p.1);
            let none: &[String] = &[];
            let (docks, vehicles, chargers) = match setup.parts() {
                ServiceParts::BikeDocked(d, _) => (&d.external[..], none, none),
                ServiceParts::Taxi(f, _) => (none, &f.vehicle_ids[..], &f.charger_ids[..]),
            };
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "an index")]
            let id = |ids: &[String], k: &str| param(k).and_then(|x| ids.get(x as usize).cloned());
            t.step.push(r.step);
            t.t_s.push(r.t_s);
            t.service.push(setup.name().to_string());
            t.kind.push(r.kind.as_str());
            t.from_dock.push(id(docks, "from_dock"));
            t.to_dock.push(id(docks, "to_dock"));
            t.n.push(param("n").unwrap_or(f64::NAN));
            t.dock.push(id(docks, "dock"));
            t.level.push(param("level").unwrap_or(f64::NAN));
            t.vehicle.push(id(vehicles, "vehicle"));
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "a node")]
            t.node.push(param("node").map(|x| {
                let n = NodeId::new(x as u32);
                road.node_external_ids().external_of(n).unwrap_or_default().to_string()
            }));
            t.charger.push(id(chargers, "charger"));
            t.to_soc.push(param("to_soc").unwrap_or(f64::NAN));
            t.accepted.push(r.accepted);
            t.reason.push(r.reason.clone());
            t.executed_s.push(r.executed_s);
            t.finished_s.push(r.finished_s);
            t.moved.push(r.moved);
            t.metres.push(r.metres);
            t.from_target.push(r.from_target);
        }
        t
    }

    /// Its rows.
    #[must_use]
    pub fn len(&self) -> usize {
        self.step.len()
    }

    /// Whether it has no row.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.step.is_empty()
    }
}

/// The built-in target solver: moves from docks above their wanted level to docks below it.
///
/// `want` is per dock, `None` for a dock without a target (it neither gives nor takes). A
/// dock's level counts the bikes `incoming` to it in vans already on the road (empty: none), so
/// a step shorter than a van's trip does not ask twice; it spares only the bikes docked. The
/// dock furthest below its level is served first (ties by dock index), from the docks above
/// theirs nearest to it as the crow flies (ties by dock index), one move from each, of at most
/// `van` bikes (a dock left short by the van's size waits for the next step). Returns
/// `(from, to, n)`.
///
/// # Panics
///
/// Never in practice: counts of docks fit `u32`.
#[must_use]
pub fn plan_moves(
    bikes: &[u32],
    incoming: &[u32],
    want: &[Option<u32>],
    position: &[LonLat],
    van: u32,
) -> Vec<(u32, u32, u32)> {
    let n = bikes.len();
    // A dock's bikes once its vans arrive; it spares only bikes docked now.
    let after = |k: usize| bikes[k] + incoming.get(k).copied().unwrap_or(0);
    let mut spare: Vec<u32> =
        (0..n).map(|k| want[k].map_or(0, |w| after(k).saturating_sub(w).min(bikes[k]))).collect();
    let short: Vec<u32> =
        (0..n).map(|k| want[k].map_or(0, |w| w.saturating_sub(after(k)))).collect();
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| short[b].cmp(&short[a]).then(a.cmp(&b)));
    let mut moves = Vec::new();
    for k in order {
        let mut need = short[k];
        if need == 0 {
            break;
        }
        let mut near: Vec<(f64, usize)> = (0..n)
            .filter(|&j| j != k && spare[j] > 0)
            .map(|j| (ground_distance_metres(position[k], position[j]), j))
            .collect();
        near.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        for (_, j) in near {
            let give = spare[j].min(need).min(van.max(1));
            let to_u32 = |i: usize| u32::try_from(i).expect("docks fit u32");
            moves.push((to_u32(j), to_u32(k), give));
            spare[j] -= give;
            need -= give;
            if need == 0 {
                break;
            }
        }
    }
    moves
}

impl FleetEngine {
    /// Check and carry out `decisions` at the clock, as asked at step `step`, then the
    /// services' targets; every one recorded. Per decision given, why it was refused.
    pub(super) fn apply_at(
        &mut self,
        step: u32,
        decisions: &[Decision],
        roads: &dyn ServiceRoads,
    ) -> Vec<Option<String>> {
        let t = self.now();
        let mut out = Vec::with_capacity(decisions.len());
        // Per service, its docks' targets at this step.
        let mut targets: Vec<Option<Vec<Option<u32>>>> = vec![None; self.setups.len()];
        for d in decisions {
            let mut record = DecisionRecord::new(step, t, d);
            let result = match *d {
                Decision::Move { service, from_dock, to_dock, n } => {
                    self.move_bikes(service, from_dock, to_dock, n, &mut record, roads)
                }
                Decision::Relocate { service, vehicle, node } => {
                    self.taxi_decision(service, vehicle, &mut record, roads, |taxis| {
                        taxis.relocate(vehicle, node, t)
                    })
                }
                Decision::Charge { service, vehicle, charger, to_soc } => {
                    self.taxi_decision(service, vehicle, &mut record, roads, |taxis| {
                        taxis.charge(vehicle, charger, to_soc, t)
                    })
                }
                Decision::Target { service, dock, level } => {
                    self.check_target(service, dock, level, &mut targets).map(|()| {
                        record.executed_s = t;
                        record.finished_s = t;
                    })
                }
            };
            record.accepted = result.is_ok();
            record.reason = result.err();
            out.push(record.reason.clone());
            self.records.push(record);
        }
        for (s, want) in targets.into_iter().enumerate() {
            let Some(want) = want else { continue };
            let service = u16::try_from(s).expect("services fit u16");
            let setups = Arc::clone(&self.setups);
            let ServiceParts::BikeDocked(docks, defaults) = setups[s].parts() else { continue };
            let ServiceState::BikeDocked { bikes, .. } = &self.state.services[s] else { continue };
            let incoming = &self.docks[s].as_ref().expect("a docked service's run").incoming;
            for (from_dock, to_dock, n) in
                plan_moves(bikes, incoming, &want, &docks.position, defaults.van_count())
            {
                let d = Decision::Move { service, from_dock, to_dock, n };
                let mut record = DecisionRecord::new(step, t, &d);
                record.from_target = true;
                let result = self.move_bikes(service, from_dock, to_dock, n, &mut record, roads);
                record.accepted = result.is_ok();
                record.reason = result.err();
                self.records.push(record);
            }
        }
        out
    }

    /// A move: checked, the bikes taken and the van sent.
    fn move_bikes(
        &mut self,
        service: u16,
        from: u32,
        to: u32,
        n: u32,
        record: &mut DecisionRecord,
        roads: &dyn ServiceRoads,
    ) -> Result<(), String> {
        let t = self.now();
        let s = usize::from(service);
        let setups = Arc::clone(&self.setups);
        let docks = match setups.get(s).map(super::ServiceSetup::parts) {
            None => return Err(format!("no service {service}")),
            Some(ServiceParts::Taxi(..)) => {
                return Err(format!("service {} is not a docked service", self.setups[s].name()));
            }
            Some(ServiceParts::BikeDocked(docks, d)) => (docks, d),
        };
        let (docks, defaults) = docks;
        let count = docks.len();
        if from as usize >= count || to as usize >= count {
            return Err(format!("no dock {}", if from as usize >= count { from } else { to }));
        }
        if from == to {
            return Err(format!("dock {} is both ends", docks.external[from as usize]));
        }
        if n == 0 {
            return Err("no bike asked for".to_string());
        }
        let ServiceState::BikeDocked { bikes, moving, .. } = &mut self.state.services[s] else {
            return Err(format!("service {} is not a docked service", self.setups[s].name()));
        };
        let there = bikes[from as usize];
        if there == 0 {
            return Err(format!("no bike at dock {}", docks.external[from as usize]));
        }
        if docks.road_node.len() != count {
            return Err("no roads".to_string());
        }
        let (drive_s, metres) =
            roads.drive(docks.road_node[from as usize], docks.road_node[to as usize], t);
        if !drive_s.is_finite() {
            return Err(format!(
                "no road from dock {} to dock {}",
                docks.external[from as usize], docks.external[to as usize]
            ));
        }
        let moved = n.min(there).min(defaults.van_count());
        let run = self.docks[s].as_mut().expect("a docked service's run");
        run.van_out(docks, bikes, from as usize, moved, t);
        run.incoming[to as usize] += moved;
        *moving += u64::from(moved);
        let arrival = t + drive_s + defaults.handling_s * f64::from(moved);
        let van = u32::try_from(self.vans.len()).expect("vans fit u32");
        let index = u32::try_from(self.records.len()).expect("records fit u32");
        self.vans.push(Van { service, to, n: moved, record: index });
        self.schedule_van(arrival, van, service);
        record.executed_s = t;
        record.moved = moved;
        record.metres = metres;
        Ok(())
    }

    /// Van `van` reaches its dock at second `t`: its bikes docked.
    pub(super) fn van_arrives(&mut self, van: u32, t: f64) {
        let Van { service, to, n, record } = self.vans[van as usize];
        let s = usize::from(service);
        let setups = Arc::clone(&self.setups);
        let ServiceParts::BikeDocked(docks, _) = setups[s].parts() else { return };
        let ServiceState::BikeDocked { bikes, moving, .. } = &mut self.state.services[s] else {
            return;
        };
        *moving -= u64::from(n);
        let run = self.docks[s].as_mut().expect("a docked service's run");
        run.incoming[to as usize] -= n;
        run.van_in(docks, bikes, to, n, t);
        if let Some(r) = self.records.get_mut(record as usize) {
            r.finished_s = t;
        }
    }

    /// A taxi decision: the service checked, then `f` on its handlers; the drive recorded and
    /// the vehicle's decision noted.
    fn taxi_decision(
        &mut self,
        service: u16,
        vehicle: u32,
        record: &mut DecisionRecord,
        roads: &dyn ServiceRoads,
        f: impl FnOnce(&mut super::taxis::Taxis<'_>) -> Result<(f64, f64), String>,
    ) -> Result<(), String> {
        let s = usize::from(service);
        match self.setups.get(s).map(super::ServiceSetup::parts) {
            None => return Err(format!("no service {service}")),
            Some(ServiceParts::BikeDocked(..)) => {
                return Err(format!("service {} is not a taxi fleet", self.setups[s].name()));
            }
            Some(ServiceParts::Taxi(..)) => {}
        }
        let (_, metres) = self.with_taxis(s, roads, f).expect("a taxi fleet")?;
        let index = u32::try_from(self.records.len()).expect("records fit u32");
        if let Some(run) = self.taxis[s].as_mut() {
            run.decision[vehicle as usize] = index;
        }
        record.executed_s = self.now();
        record.metres = metres;
        Ok(())
    }

    /// A target: checked and noted for the solver.
    fn check_target(
        &self,
        service: u16,
        dock: u32,
        level: u32,
        targets: &mut [Option<Vec<Option<u32>>>],
    ) -> Result<(), String> {
        let s = usize::from(service);
        let docks = match self.setups.get(s).map(super::ServiceSetup::parts) {
            None => return Err(format!("no service {service}")),
            Some(ServiceParts::Taxi(..)) => {
                return Err(format!("service {} is not a docked service", self.setups[s].name()));
            }
            Some(ServiceParts::BikeDocked(docks, _)) => docks,
        };
        let Some(&slots) = docks.capacity.get(dock as usize) else {
            return Err(format!("no dock {dock}"));
        };
        if level > slots {
            return Err(format!(
                "level {level} is above dock {}'s {slots} slots",
                docks.external[dock as usize]
            ));
        }
        let want = targets[s].get_or_insert_with(|| vec![None; docks.len()]);
        if want[dock as usize].is_some() {
            return Err(format!(
                "a second target for dock {} at this step",
                docks.external[dock as usize]
            ));
        }
        want[dock as usize] = Some(level);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_solver_serves_the_shortest_dock_from_the_nearest_spare_at_most_a_van_a_move() {
        let at = |x: f64| LonLat::new(4.8 + x, 45.7);
        let position = [at(0.0), at(0.001), at(0.01), at(0.002)];
        // Dock 0 wants 5 more; dock 1 (near) can give 2, dock 2 (far) 6; dock 3 has no target.
        // Vans of 3: 2 from dock 1, then 3 from dock 2.
        let bikes = [0, 4, 8, 9];
        let want = [Some(5), Some(2), Some(2), None];
        assert_eq!(plan_moves(&bikes, &[], &want, &position, 3), [(1, 0, 2), (2, 0, 3)]);
        // Vans of 2: one move from each, dock 0 left one short.
        assert_eq!(plan_moves(&bikes, &[], &want, &position, 2), [(1, 0, 2), (2, 0, 2)]);
        assert!(plan_moves(&bikes, &[], &[None; 4], &position, 2).is_empty(), "no target, no move");
    }
}
