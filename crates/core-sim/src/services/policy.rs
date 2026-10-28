//! Operators: the code that steers the services during a day.
//!
//! A **policy** (an operator, in code) is called by the fleet engine every [`Policy::step_s`]
//! seconds, the first at the window's start, with a read-only [`View`] of the state; it returns
//! [`Decision`]s, which the engine checks, refuses with a reason when they cannot be carried
//! out (the day goes on), and carries out otherwise. A policy never changes the simulation
//! itself. One policy may manage every service at once (a **hub manager**): the view holds every
//! service's section.
//!
//! A policy's own failure (`Err` from [`Policy::step`]) stops the day: the engine returns the
//! message.
//!
//! **Determinism**: a policy's own draws come from [`View::rng_seed`], one number per step from
//! the stream `fleet_policy` keyed by the day's seed and the step; the same day, policy and seed
//! give the same decisions. A copied engine copies its policy ([`PolicyClone`]), so the copy
//! continues as the original would.

use std::collections::BTreeMap;
use std::fmt;

use super::decisions::Decision;
use super::engine::VehicleState;
use super::view::View;
use super::{ServiceParts, ServiceSetup};

/// What a policy is, for the run's fingerprint and manifest: its name and parameters.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PolicyDescription {
    /// Its name (`"noop"` for [`NoOp`]).
    pub name: String,
    /// Its parameters, by name, in the order it gives them.
    pub parameters: Vec<(String, f64)>,
}

/// What a policy is told when a day begins.
#[derive(Clone, Copy, Debug)]
pub struct DayInfo<'a> {
    /// The window's start and end, in seconds from midnight.
    pub window: (f64, f64),
    /// The day's seed.
    pub seed: u64,
    /// The services, in the run's order (a decision names a service by its index here).
    pub services: &'a [ServiceSetup],
}

/// An operator: see the [module docs](self).
pub trait Policy: Send + Sync + PolicyClone {
    /// Its name and parameters.
    fn describe(&self) -> PolicyDescription;

    /// The seconds between its steps, the first at the window's start (900 by convention). A
    /// value that is not a positive number stops the day before its first step.
    fn step_s(&self) -> f64;

    /// Its decisions at this step, appended to `out`.
    ///
    /// # Errors
    ///
    /// Its own failure, which stops the day with the message.
    fn step(&mut self, view: &View<'_>, out: &mut Vec<Decision>) -> Result<(), String>;

    /// A new day begins (an operator with state resets here). Called once, when the engine is
    /// built.
    fn day_start(&mut self, day: &DayInfo<'_>) {
        let _ = day;
    }
}

/// A copy of a policy, its state included, for a copied engine. Every `Clone` policy has it.
pub trait PolicyClone {
    /// A boxed copy.
    fn clone_box(&self) -> Box<dyn Policy>;
}

impl<T: Policy + Clone + 'static> PolicyClone for T {
    fn clone_box(&self) -> Box<dyn Policy> {
        Box::new(self.clone())
    }
}

/// The policy that decides nothing: the day is the day without an operator, with steps.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NoOp {
    /// The seconds between steps.
    pub step_s: f64,
}

impl NoOp {
    /// Steps every 900 s.
    pub const DEFAULT: Self = Self { step_s: 900.0 };

    /// Steps every `step_s` seconds.
    #[must_use]
    pub const fn every(step_s: f64) -> Self {
        Self { step_s }
    }
}

impl Default for NoOp {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl Policy for NoOp {
    fn describe(&self) -> PolicyDescription {
        PolicyDescription {
            name: "noop".to_string(),
            parameters: vec![("step_s".into(), self.step_s)],
        }
    }

    fn step_s(&self) -> f64 {
        self.step_s
    }

    fn step(&mut self, _: &View<'_>, _: &mut Vec<Decision>) -> Result<(), String> {
        Ok(())
    }
}

/// Every docked service's docks back toward their start-of-day counts, every step from a
/// second of the day on.
///
/// At each step at or after [`Self::from_s`], every dock whose bikes differ from its count at
/// the start of the day is given that count as its [`Decision::Target`]; the engine's target
/// solver ([`super::decisions::plan_moves`]) moves bikes from the docks above their count to
/// those below it, nearest first, at most the service's `van_bikes` a move. A dock at its count
/// neither gives nor takes. Bikes still in a van at the next step are not counted at their
/// destination, so a dock waiting for them may be served twice when vans take longer than
/// [`Self::step_s`].
#[derive(Clone, Debug, PartialEq)]
pub struct Reset {
    /// The seconds between steps.
    pub step_s: f64,
    /// The first second of the day (from midnight) it acts at; before it, it decides nothing.
    pub from_s: f64,
    /// Per service, its docks' counts at the start of the day (`None`: not a docked service).
    start: Vec<Option<Vec<u32>>>,
}

impl Reset {
    /// Steps every `step_s` seconds, acting from second `from_s` of the day on.
    #[must_use]
    pub fn new(step_s: f64, from_s: f64) -> Self {
        Self { step_s, from_s, start: Vec::new() }
    }
}

impl Policy for Reset {
    fn describe(&self) -> PolicyDescription {
        PolicyDescription {
            name: "reset".to_string(),
            parameters: vec![("step_s".into(), self.step_s), ("from_s".into(), self.from_s)],
        }
    }

    fn step_s(&self) -> f64 {
        self.step_s
    }

    fn day_start(&mut self, day: &DayInfo<'_>) {
        self.start = day
            .services
            .iter()
            .map(|s| match s.parts() {
                ServiceParts::BikeDocked(docks, _) => Some(docks.start.clone()),
                ServiceParts::Taxi(..) => None,
            })
            .collect();
    }

    fn step(&mut self, view: &View<'_>, out: &mut Vec<Decision>) -> Result<(), String> {
        if view.t < self.from_s {
            return Ok(());
        }
        for (service, start) in (0_u16..).zip(&self.start) {
            let (Some(start), Some(d)) = (start, view.docked(service)) else { continue };
            for (dock, (&now, &want)) in (0_u32..).zip(d.bikes.iter().zip(start)) {
                if now != want {
                    out.push(Decision::Target { service, dock, level: want });
                }
            }
        }
        Ok(())
    }
}

/// Docks running empty topped up from docks running full, every step.
///
/// At each step, in every docked service, a dock holding fewer bikes than [`Self::low`] of its
/// slots is given a [`Decision::Target`] of the middle, `slots × (low + high) / 2` rounded
/// down, and one holding more than [`Self::high`] of its slots a target of the middle rounded
/// up; the others none. The engine's target solver moves bikes from the second kind to the
/// first, nearest first, at most the service's `van_bikes` a move.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Threshold {
    /// Below this share of its slots, a dock is topped up.
    pub low: f64,
    /// Above this share of its slots, a dock gives bikes away.
    pub high: f64,
    /// The seconds between steps.
    pub step_s: f64,
}

impl Threshold {
    /// The rule for `0 ≤ low < high ≤ 1`.
    ///
    /// # Errors
    ///
    /// If not `0 ≤ low < high ≤ 1`.
    pub fn new(low: f64, high: f64, step_s: f64) -> Result<Self, String> {
        if (0.0..=1.0).contains(&low) && (0.0..=1.0).contains(&high) && low < high {
            Ok(Self { low, high, step_s })
        } else {
            Err(format!("threshold needs 0 <= low < high <= 1, got {low} and {high}"))
        }
    }

    /// Dock `slots`' target holding `bikes`, if it is past a threshold.
    #[must_use]
    pub fn target(&self, bikes: u32, slots: u32) -> Option<u32> {
        let (b, n) = (f64::from(bikes), f64::from(slots));
        let mid = n * (self.low + self.high) / 2.0;
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "between 0 and the dock's slots"
        )]
        let level = |x: f64| x.clamp(0.0, n) as u32;
        if b < self.low * n {
            Some(level(mid.floor()))
        } else if b > self.high * n {
            Some(level(mid.ceil()))
        } else {
            None
        }
    }
}

impl Policy for Threshold {
    fn describe(&self) -> PolicyDescription {
        PolicyDescription {
            name: "threshold".to_string(),
            parameters: vec![
                ("low".into(), self.low),
                ("high".into(), self.high),
                ("step_s".into(), self.step_s),
            ],
        }
    }

    fn step_s(&self) -> f64 {
        self.step_s
    }

    fn step(&mut self, view: &View<'_>, out: &mut Vec<Decision>) -> Result<(), String> {
        for service in (0_u16..).take(view.services.len()) {
            let Some(d) = view.docked(service) else { continue };
            for (dock, (&bikes, &slots)) in (0_u32..).zip(d.bikes.iter().zip(d.slots)) {
                if let Some(level) = self.target(bikes, slots) {
                    out.push(Decision::Target { service, dock, level });
                }
            }
        }
        Ok(())
    }
}

/// Idle taxis low on charge sent to charge, every step.
///
/// At each step, in every taxi fleet, an idle vehicle charged below [`Self::charge_below`] of
/// its battery is sent ([`Decision::Charge`]) to the charger nearest its node by free-flow time
/// (within the fleet's `charger_reach_s`; a vehicle with none in reach stays), to charge to
/// [`Self::charge_to`]. It acts besides the fleet's own charging rule (its options
/// `charge_below` and `charge_to`, applied when a vehicle drops a passenger).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChargeRule {
    /// Below this share of its battery, an idle vehicle is sent to charge.
    pub charge_below: f64,
    /// The share it charges to.
    pub charge_to: f64,
    /// The seconds between steps.
    pub step_s: f64,
}

impl ChargeRule {
    /// The rule for `0 ≤ charge_below < charge_to ≤ 1`.
    ///
    /// # Errors
    ///
    /// If not `0 ≤ charge_below < charge_to ≤ 1`.
    pub fn new(charge_below: f64, charge_to: f64, step_s: f64) -> Result<Self, String> {
        if (0.0..=1.0).contains(&charge_below)
            && (0.0..=1.0).contains(&charge_to)
            && charge_below < charge_to
        {
            Ok(Self { charge_below, charge_to, step_s })
        } else {
            Err(format!(
                "charge_rule needs 0 <= charge_below < charge_to <= 1, got {charge_below} and \
                 {charge_to}"
            ))
        }
    }
}

impl Policy for ChargeRule {
    fn describe(&self) -> PolicyDescription {
        PolicyDescription {
            name: "charge_rule".to_string(),
            parameters: vec![
                ("charge_below".into(), self.charge_below),
                ("charge_to".into(), self.charge_to),
                ("step_s".into(), self.step_s),
            ],
        }
    }

    fn step_s(&self) -> f64 {
        self.step_s
    }

    fn step(&mut self, view: &View<'_>, out: &mut Vec<Decision>) -> Result<(), String> {
        for service in (0_u16..).take(view.services.len()) {
            let Some(t) = view.taxi(service) else { continue };
            for (vehicle, ((&state, &soc), &node)) in
                (0_u32..).zip(t.state.iter().zip(t.soc).zip(t.node))
            {
                if state != VehicleState::Idle || soc >= self.charge_below {
                    continue;
                }
                if let Some(charger) = t.nearest_charger(node) {
                    out.push(Decision::Charge {
                        service,
                        vehicle,
                        charger,
                        to_soc: self.charge_to,
                    });
                }
            }
        }
        Ok(())
    }
}

/// The built-in policies' names, for [`policy_builtin`].
pub const POLICY_BUILTINS: [&str; 4] = ["noop", "reset", "threshold", "charge_rule"];

/// A built-in policy by name, with its parameters by name (those not given take their
/// defaults): `"noop"` (`step_s` 900), `"reset"` (`step_s` 900, `from_s` 0), `"threshold"`
/// (`low` 0.2, `high` 0.8, `step_s` 900), `"charge_rule"` (`charge_below` 0.2, `charge_to` 0.8,
/// `step_s` 900). The defaults are assumptions, not calibrated.
///
/// # Errors
///
/// An unknown name or parameter (with the list), or parameters a policy refuses.
pub fn policy_builtin(
    name: &str,
    parameters: &BTreeMap<String, f64>,
) -> Result<Box<dyn Policy>, String> {
    let known: &[(&str, f64)] = match name {
        "noop" => &[("step_s", 900.0)],
        "reset" => &[("step_s", 900.0), ("from_s", 0.0)],
        "threshold" => &[("low", 0.2), ("high", 0.8), ("step_s", 900.0)],
        "charge_rule" => &[("charge_below", 0.2), ("charge_to", 0.8), ("step_s", 900.0)],
        _ => {
            return Err(format!(
                "unknown policy {name:?}; the built-in ones are {}",
                POLICY_BUILTINS.join(", ")
            ));
        }
    };
    if let Some(bad) = parameters.keys().find(|k| known.iter().all(|(n, _)| n != k)) {
        let names: Vec<&str> = known.iter().map(|(n, _)| *n).collect();
        return Err(format!("unknown {name} parameter {bad:?}; it has {}", names.join(", ")));
    }
    let p = |key: &str| {
        parameters
            .get(key)
            .copied()
            .unwrap_or_else(|| known.iter().find(|(n, _)| *n == key).map_or(f64::NAN, |&(_, v)| v))
    };
    Ok(match name {
        "noop" => Box::new(NoOp::every(p("step_s"))),
        "reset" => Box::new(Reset::new(p("step_s"), p("from_s"))),
        "threshold" => Box::new(Threshold::new(p("low"), p("high"), p("step_s"))?),
        _ => Box::new(ChargeRule::new(p("charge_below"), p("charge_to"), p("step_s"))?),
    })
}

/// An engine's policy: a box that copies and prints.
pub(crate) struct PolicySlot(pub(crate) Box<dyn Policy>);

impl Clone for PolicySlot {
    fn clone(&self) -> Self {
        Self(self.0.clone_box())
    }
}

impl fmt::Debug for PolicySlot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Policy({:?})", self.0.describe())
    }
}
