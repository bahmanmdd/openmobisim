//! What the link transmission model does at junctions and for vehicles that are stuck (S213),
//! by name: priority by road hierarchy at merges, en-route rerouting, and turn pockets.
//!
//! All three are remedies for gridlock: a merge that shares room in a fixed ratio lets traffic on
//! a closed loop destroy itself, and vehicles that never leave their route cannot get out of it
//! (Daganzo 1996, *The nature of freeway gridlock and how to prevent it*); priority to the major
//! stream prevents the first, drivers who change course escape the second. Turn pockets (S217)
//! keep a vehicle waiting to turn into a full street from holding up those turning elsewhere.

use std::collections::BTreeMap;

/// The loading's rules, each with an uncalibrated default (S202).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LoadingOptions {
    /// Priority by road hierarchy at unsignalised merges
    /// ([`openmobisim_core_loading::LtmNetwork::with_priority`]): a vehicle gives way to a
    /// higher-priority approach's front vehicle bound for the same link, and departures to every
    /// approach. *Off (S215): with one first-in-first-out queue per link a vehicle giving way
    /// holds up everything behind it, so priority made locks worse on Amsterdam (S214); it waits
    /// for per-turn queues.*
    pub priority: bool,
    /// En-route rerouting: a vehicle stuck at the front of its link re-routes from where it is.
    /// *On (S215): it removed the locks of Amsterdam's morning at 1× and 1.25× and leaves
    /// uncongested results as they were (S214).*
    pub reroute: bool,
    /// How long a vehicle waits at the front of its link, blocked, before it re-routes, in
    /// seconds.
    ///
    /// *Uncalibrated: 300 s. MATSim's stuck time, the nearest device, ranges from 10 s (its
    /// default) to 3 600 s in its own examples; Olmos et al. (2018) re-route after 120 s.
    /// CITATION OWED (observed diversion under congestion).*
    pub reroute_after_s: f64,
    /// The most times one trip re-routes.
    ///
    /// *Uncalibrated: 3, to keep a vehicle from wandering.*
    pub reroute_max: u32,
    /// How much faster the new route must be, by the costs the vehicle sees, than the rest of
    /// its current one, as a share.
    ///
    /// *Uncalibrated: 0.1. CITATION OWED (diversion thresholds in route-guidance studies).*
    pub reroute_min_gain: f64,
    /// Turn pockets on approaches of two lanes or more
    /// ([`openmobisim_core_loading::LtmNetwork::with_pockets`]), in metres per lane: a vehicle at
    /// the end of such a link passes those ahead of it waiting for another movement, as long as
    /// they fit in their pockets. 0 turns them off: every link one first-in-first-out queue.
    ///
    /// *Uncalibrated: 50 m, a typical turn bay, split among a link's movements by lane share.
    /// CITATION OWED (turn-bay lengths in design guides). Lanes per movement from OpenStreetMap's
    /// `turn:lanes`, where mapped, are for later (S217).*
    pub pocket_length_m: f64,
}

impl LoadingOptions {
    /// The shipped values.
    pub const SHIPPED: LoadingOptions = LoadingOptions {
        priority: false,
        reroute: true,
        reroute_after_s: 300.0,
        reroute_max: 3,
        reroute_min_gain: 0.1,
        pocket_length_m: 50.0,
    };

    /// Every option's value, by name, in [`Self::NAMES`]' order (S230: the parameter listing);
    /// `priority` and `reroute` as 0 or 1.
    #[must_use]
    pub fn values(&self) -> Vec<(&'static str, f64)> {
        let flag = |b: bool| if b { 1.0 } else { 0.0 };
        vec![
            ("priority", flag(self.priority)),
            ("reroute", flag(self.reroute)),
            ("reroute_after_s", self.reroute_after_s),
            ("reroute_max", f64::from(self.reroute_max)),
            ("reroute_min_gain", self.reroute_min_gain),
            ("pocket_length_m", self.pocket_length_m),
        ]
    }

    /// The names of the options.
    pub const NAMES: [&'static str; 6] = [
        "priority",
        "reroute",
        "reroute_after_s",
        "reroute_max",
        "reroute_min_gain",
        "pocket_length_m",
    ];

    /// The shipped values with `options` in place of their namesakes. `priority` and `reroute`
    /// are 0 or 1.
    ///
    /// # Errors
    ///
    /// The name and the list of known names for an unknown option; the reason for a value out
    /// of range.
    pub fn from_options(options: &BTreeMap<String, f64>) -> Result<Self, String> {
        let mut d = Self::SHIPPED;
        #[allow(clippy::float_cmp, reason = "a flag given as exactly 0 or 1")]
        let flag = |name: &str, v: f64| -> Result<bool, String> {
            if v == 0.0 || v == 1.0 {
                Ok(v == 1.0)
            } else {
                Err(format!("loading_options {name:?} must be 0 or 1, got {v}"))
            }
        };
        for (name, &v) in options {
            match name.as_str() {
                "priority" => d.priority = flag(name, v)?,
                "reroute" => d.reroute = flag(name, v)?,
                "reroute_after_s" => {
                    if v.is_nan() || v < 0.0 || v.is_infinite() {
                        return Err(format!(
                            "loading_options \"reroute_after_s\" must be a finite number of \
                             seconds, at least 0, got {v}"
                        ));
                    }
                    d.reroute_after_s = v;
                }
                "reroute_max" => {
                    if !(v.fract() == 0.0 && (0.0..=255.0).contains(&v)) {
                        return Err(format!(
                            "loading_options \"reroute_max\" must be a whole number from 0 to \
                             255, got {v}"
                        ));
                    }
                    #[allow(
                        clippy::cast_possible_truncation,
                        clippy::cast_sign_loss,
                        reason = "checked to be a small whole number"
                    )]
                    let n = v as u32;
                    d.reroute_max = n;
                }
                "reroute_min_gain" => {
                    if !(0.0..1.0).contains(&v) {
                        return Err(format!(
                            "loading_options \"reroute_min_gain\" must be from 0 to below 1, \
                             got {v}"
                        ));
                    }
                    d.reroute_min_gain = v;
                }
                "pocket_length_m" => {
                    if v.is_nan() || v < 0.0 || v.is_infinite() {
                        return Err(format!(
                            "loading_options \"pocket_length_m\" must be a finite number of \
                             metres, at least 0, got {v}"
                        ));
                    }
                    d.pocket_length_m = v;
                }
                _ => {
                    return Err(format!(
                        "loading_options has no {name:?}; the options are: {}",
                        Self::NAMES.join(", ")
                    ));
                }
            }
        }
        Ok(d)
    }

    /// Whether any rule is on: a run with every rule off hashes as runs did before the rules.
    #[must_use]
    pub fn any(&self) -> bool {
        self.priority || self.reroute || self.pocket_length_m > 0.0
    }
}

impl Default for LoadingOptions {
    fn default() -> Self {
        Self::SHIPPED
    }
}
