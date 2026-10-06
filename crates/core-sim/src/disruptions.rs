//! Disruptions at a time of day (S238, roadmap I-az: timed events; design §25): a road narrowed
//! or closed for a while, a line's runs delayed or cancelled.
//!
//! **Known or not.** Travellers either **know** of the disruptions — every loading has them, so
//! routes, modes and itineraries adapt over the iterations as to anything that happens every
//! day — or **do not** (the default, design §25.1's zero-adaptation reading, S78): the run
//! reaches its equilibrium without them, then loads the day **once more with them**, choices
//! fixed. Only what happens within that day reacts: en-route rerouting of cars stuck at levels
//! 3–4 (S213), and passengers whose run never comes, who plan again from where they are (M4).
//! That last loading is the run's result; the convergence report stays the undisrupted one's.
//!
//! **Roads**: [`CapacityChange`]s the loading makes when its clock reaches them, a link's
//! factors multiplied where disruptions overlap; at free flow (`flow_level` 0) capacity plays
//! no part, so a road disruption changes nothing there. **Transit**: the runs of a line whose
//! first departure falls in the window; a delayed bus riding the roads leaves its first stop
//! late and drives on among the cars, a delayed run by the schedule keeps its times shifted; a
//! cancelled run does not run, and no one boards it.
//!
//! **Cost:** nothing when there is none; one comparison per movement of the loading when there
//! are road disruptions; one more loading when they are not known.

use std::collections::BTreeMap;

use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_loading::CapacityChange;
use openmobisim_core_transit::{CallTimes, Timetable, UNKNOWN_TIME};
use openmobisim_core_types::ids::{EntityId, LinkId, TransitRunId};

/// A road's capacity multiplied for a while.
#[derive(Clone, Debug, PartialEq)]
pub struct RoadDisruption {
    /// The links (each direction of a road is a link of its own).
    pub links: Vec<LinkId>,
    /// The share of capacity left: 0 closes them, 0.5 halves it.
    pub factor: f64,
    /// From this second of the day …
    pub from_s: f64,
    /// … to this one.
    pub to_s: f64,
}

/// What happens to a line's runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransitEffect {
    /// Late by this many seconds, every call.
    Delay(u32),
    /// Not run.
    Cancel,
}

/// A line's runs disrupted: those whose first departure falls in the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TransitDisruption {
    /// The line (route), by index.
    pub route: u32,
    /// Delayed or cancelled.
    pub effect: TransitEffect,
    /// Runs leaving their first stop from this second …
    pub from_s: u32,
    /// … to before this one.
    pub to_s: u32,
}

/// A run's disruptions: see the [module docs](self).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Disruptions {
    /// Roads.
    pub road: Vec<RoadDisruption>,
    /// Lines.
    pub transit: Vec<TransitDisruption>,
    /// Whether travellers know of them in advance.
    pub known: bool,
}

impl Disruptions {
    /// Whether there is none.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.road.is_empty() && self.transit.is_empty()
    }

    /// Checked against `road` and a timetable of `routes` lines.
    ///
    /// # Errors
    ///
    /// A message for a link or line out of range, a factor below 0, a window that ends before
    /// it starts, or a transit disruption in a run without a timetable.
    pub fn check(&self, road: &RoadNetwork, routes: Option<u32>) -> Result<(), String> {
        for (k, d) in self.road.iter().enumerate() {
            if let Some(l) = d.links.iter().find(|l| l.raw() >= road.link_count()) {
                return Err(format!("road disruption {k}: there is no link {}", l.raw()));
            }
            if !(d.factor.is_finite() && d.factor >= 0.0) {
                return Err(format!("road disruption {k}: a capacity factor of 0 or more"));
            }
            if !(d.from_s.is_finite() && d.to_s.is_finite() && d.from_s < d.to_s) {
                return Err(format!("road disruption {k}: the window ends before it starts"));
            }
        }
        for (k, d) in self.transit.iter().enumerate() {
            let Some(n) = routes else {
                return Err("a transit disruption needs a timetable (transit=)".to_string());
            };
            if d.route >= n {
                return Err(format!("transit disruption {k}: there is no line {}", d.route));
            }
            if d.from_s >= d.to_s {
                return Err(format!("transit disruption {k}: the window ends before it starts"));
            }
        }
        Ok(())
    }

    /// The loading's capacity changes: per link, at every start and end of a disruption, the
    /// product of the factors of those under way then.
    #[must_use]
    pub fn capacity_changes(&self) -> Vec<CapacityChange> {
        let mut by_link: BTreeMap<LinkId, Vec<&RoadDisruption>> = BTreeMap::new();
        for d in &self.road {
            for &l in &d.links {
                by_link.entry(l).or_default().push(d);
            }
        }
        let mut out = Vec::new();
        for (link, ds) in by_link {
            let mut times: Vec<f64> = ds.iter().flat_map(|d| [d.from_s, d.to_s]).collect();
            times.sort_by(f64::total_cmp);
            times.dedup();
            let mut current = 1.0_f64;
            for t in times {
                let factor: f64 =
                    ds.iter().filter(|d| d.from_s <= t && t < d.to_s).map(|d| d.factor).product();
                if factor.to_bits() != current.to_bits() {
                    out.push(CapacityChange { time: t, link, factor });
                    current = factor;
                }
            }
        }
        out
    }

    /// What happens to `run` of `timetable`: a cancellation wins over delays, delays add up.
    #[must_use]
    pub fn effect_on(&self, timetable: &Timetable, run: TransitRunId) -> Option<TransitEffect> {
        if self.transit.is_empty() {
            return None;
        }
        let route = timetable.run_route(run);
        let leaves = timetable.scheduled().departure[timetable.run_calls(run).start];
        let mut delay: Option<u32> = None;
        for d in &self.transit {
            if d.route != route || !(d.from_s..d.to_s).contains(&leaves) {
                continue;
            }
            match d.effect {
                TransitEffect::Cancel => return Some(TransitEffect::Cancel),
                TransitEffect::Delay(s) => delay = Some(delay.unwrap_or(0).saturating_add(s)),
            }
        }
        delay.map(TransitEffect::Delay)
    }

    /// `times` with the disruptions made: cancelled runs' calls unknown (no one boards), and
    /// delayed runs that do not ride the roads (`on_roads`) shifted; a delayed bus on the roads
    /// already carries its delay in its times.
    pub(crate) fn apply_to_times(
        &self,
        timetable: &Timetable,
        times: &mut CallTimes,
        on_roads: &dyn Fn(TransitRunId) -> bool,
    ) {
        for r in 0..timetable.run_count() {
            let run = TransitRunId::new(r);
            match self.effect_on(timetable, run) {
                Some(TransitEffect::Cancel) => {
                    for c in timetable.run_calls(run) {
                        times.arrival[c] = UNKNOWN_TIME;
                        times.departure[c] = UNKNOWN_TIME;
                    }
                }
                Some(TransitEffect::Delay(s)) if !on_roads(run) => {
                    for c in timetable.run_calls(run) {
                        for t in [&mut times.arrival[c], &mut times.departure[c]] {
                            if *t != UNKNOWN_TIME {
                                *t = t.saturating_add(s);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlapping_road_disruptions_multiply_and_end_in_the_link_s_own_capacity() {
        let (a, b) = (LinkId::new(3), LinkId::new(5));
        let d = Disruptions {
            road: vec![
                RoadDisruption { links: vec![a, b], factor: 0.5, from_s: 100.0, to_s: 400.0 },
                RoadDisruption { links: vec![a], factor: 0.0, from_s: 200.0, to_s: 300.0 },
            ],
            ..Disruptions::default()
        };
        let changes: Vec<(f64, u32, f64)> =
            d.capacity_changes().iter().map(|c| (c.time, c.link.raw(), c.factor)).collect();
        assert_eq!(
            changes,
            [
                (100.0, 3, 0.5),
                (200.0, 3, 0.0),
                (300.0, 3, 0.5),
                (400.0, 3, 1.0),
                (100.0, 5, 0.5),
                (400.0, 5, 1.0),
            ]
        );
        assert!(Disruptions::default().capacity_changes().is_empty());
    }
}
