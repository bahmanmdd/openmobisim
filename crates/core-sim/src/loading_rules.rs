//! What the link transmission model does at junctions and for vehicles that are stuck,
//! by name: priority by road hierarchy at merges, en-route rerouting, and turn pockets; and two
//! rules of entry: the inflow headway at nodes at the point queue, and at origins.
//!
//! The first three are remedies for gridlock: a merge that shares room in a fixed ratio lets
//! traffic on a closed loop destroy itself, and vehicles that never leave their route cannot get
//! out of it (Daganzo 1996, *The nature of freeway gridlock and how to prevent it*); priority to
//! the major stream prevents the first, drivers who change course escape the second. Turn
//! pockets keep a vehicle waiting to turn into a full street from holding up those turning
//! elsewhere.

use std::collections::BTreeMap;

use openmobisim_core_loading::FidelityLevel;

/// Whether en-route rerouting is on: by the level, or as asked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RerouteSetting {
    /// On at the levels with spillback (3 and 4), where a vehicle can be held for room, and at
    /// the point queue (2) in a loading with road disruptions, where a closed or narrowed link
    /// ahead holds one; off at the point queue otherwise, where nothing is ever held. The
    /// shipped setting.
    Auto,
    /// Off at every level.
    Off,
    /// On at every level (at the point queue it acts only behind a closed or narrowed link).
    On,
}

/// The loading's rules, each with an uncalibrated default.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LoadingOptions {
    /// Priority by road hierarchy at unsignalised merges
    /// ([`openmobisim_core_loading::LtmNetwork::with_priority`]): a vehicle gives way to a
    /// higher-priority approach's front vehicle bound for the same link, and departures to every
    /// approach. *Off: with one first-in-first-out queue per link a vehicle giving way
    /// holds up everything behind it, so priority made locks worse on Amsterdam; it waits
    /// for per-turn queues.*
    pub priority: bool,
    /// En-route rerouting: a vehicle stuck at the front of its link re-routes from where it is.
    /// *On at levels 3 and 4 ([`RerouteSetting::Auto`]: it removed the locks of
    /// Amsterdam's morning at 1× and 1.25× and leaves uncongested results as they were), and at
    /// the point queue in a loading with road disruptions (S289: unwarned drivers behind a
    /// closure otherwise wait for it to reopen); off at the point queue otherwise, where
    /// nothing waits for room, unless asked for.* The option `reroute` is 1 (on) or 0 (off) at
    /// every level; [`Self::resolved`] settles the shipped setting for a run.
    pub reroute: RerouteSetting,
    /// How long a vehicle waits at the front of its link, blocked, before it re-routes, in
    /// seconds.
    ///
    /// *Uncalibrated: 300 s. MATSim's stuck time, the nearest device, defaults to 30 s
    /// (Mersini and Ziemke, MATSim User Meeting 2025, slide 6,
    /// <https://matsim.org/conferences/mum2025/presentations/mersini.pdf>) and is 3 600 s in its
    /// Sioux Falls example
    /// (<https://svn.vsp.tu-berlin.de/repos/public-svn/matsim/scenarios/countries/us/sioux-falls/output/output_config.xml>);
    /// Olmos et al. (2018, *PNAS* 115(50) 12654–12661,
    /// <https://pmc.ncbi.nlm.nih.gov/articles/PMC6294880>) let a vehicle stopped about two minutes
    /// (96 steps of 1.2375 s) re-route. No observed diversion delay was found.*
    pub reroute_after_s: f64,
    /// The most times one trip re-routes.
    ///
    /// *Uncalibrated: 3, to keep a vehicle from wandering.*
    pub reroute_max: u32,
    /// How much faster the new route must be, by the costs the vehicle sees, than the rest of
    /// its current one, as a share.
    ///
    /// *Uncalibrated: 0.1. A relative threshold of this kind is the indifference band of
    /// boundedly rational route switching (Mahmassani and Jayakrishnan 1991, *Transportation
    /// Research Part A* 25(5) 293–307, <https://doi.org/10.1016/0191-2607(91)90145-G>); no
    /// observed value was taken from it.*
    pub reroute_min_gain: f64,
    /// Turn pockets on approaches of two lanes or more
    /// ([`openmobisim_core_loading::LtmNetwork::with_pockets`]), in metres per lane: a vehicle at
    /// the end of such a link passes those ahead of it waiting for another movement, as long as
    /// they fit in their pockets. 0 turns them off: every link one first-in-first-out queue.
    ///
    /// **On by default (50 m; S289).** Off (0, the default from S280 to S288) the full model
    /// locked Sioux Falls at its base demand (58 % of trips finished; every build with pockets
    /// finishes them all) and a one-lane street closed for an hour in Delft locked a fifth of
    /// its trips. The known cost of pockets stands: the link times an iterated run averages
    /// over a link's movements misprice the movements that wait, and the entry-delay
    /// attribution that would fix it is not built.
    /// *Uncalibrated: 50 m, split among a link's movements by lane share; an assumption
    /// (design guides size a bay from its expected queue, so no single length was found).
    /// Lanes per movement from OpenStreetMap's `turn:lanes`, where mapped, are for later.*
    pub pocket_length_m: f64,
    /// At the point queue (`flow_level` 2) only, whether a vehicle's entry onto the next link
    /// waits for that link's inflow headway
    /// ([`openmobisim_core_loading::LtmNetwork::with_node_inflow`]): on (1), a vehicle held so
    /// at the end of its link holds up those behind it bound elsewhere, and a bottleneck's queue
    /// forms at the end of the links feeding it; off (0), **the classic point queue**: the next
    /// link takes every vehicle at once, a link's exit is bounded by its own discharge capacity
    /// alone, and a bottleneck's queue sits at its own exit. No effect at levels 3 and 4, where
    /// the headway is part of the receiving condition.
    ///
    /// *Uncalibrated: on, the loading as it was before the option. Off is the point queue of
    /// Vickrey (1969, *Congestion theory and transport investment*, *American Economic Review*
    /// 59(2) 251–260) and of Han, Friesz and Yao (2013, *A partial differential equation
    /// formulation of Vickrey's bottleneck model*, *Transportation Research Part B* 49). Off, a
    /// link closed by a road disruption takes the vehicles bound onto it, which wait at its end,
    /// where no reroute is offered.*
    pub node_inflow: bool,
    /// Whether a departing vehicle enters its first link no faster than the link's inflow
    /// headway allows ([`openmobisim_core_loading::LtmNetwork::with_origin_inflow`]), at every
    /// level: on (1), the origin feeds its first link at the link's capacity at most, as an
    /// upstream link would; off (0), departures enter at any rate, so those given the same
    /// second (demand in departure bins) enter as one platoon.
    ///
    /// *Uncalibrated: off, the loading as it was before the option. On treats the origin as a
    /// centroid connector with the first link's capacity, an assumption.*
    pub origin_inflow: bool,
}

impl LoadingOptions {
    /// The shipped values.
    pub const SHIPPED: LoadingOptions = LoadingOptions {
        priority: false,
        reroute: RerouteSetting::Auto,
        reroute_after_s: 300.0,
        reroute_max: 3,
        reroute_min_gain: 0.1,
        pocket_length_m: 50.0,
        node_inflow: true,
        origin_inflow: false,
    };

    /// Every option's value, by name, in [`Self::NAMES`]' order (the parameter listing);
    /// `priority`, `reroute`, `node_inflow` and `origin_inflow` as 0 or 1 (`reroute` 1 for
    /// [`RerouteSetting::Auto`]: on where it can act; [`Self::resolved`] gives a run's own).
    #[must_use]
    pub fn values(&self) -> Vec<(&'static str, f64)> {
        let flag = |b: bool| if b { 1.0 } else { 0.0 };
        vec![
            ("priority", flag(self.priority)),
            ("reroute", flag(self.reroute != RerouteSetting::Off)),
            ("reroute_after_s", self.reroute_after_s),
            ("reroute_max", f64::from(self.reroute_max)),
            ("reroute_min_gain", self.reroute_min_gain),
            ("pocket_length_m", self.pocket_length_m),
            ("node_inflow", flag(self.node_inflow)),
            ("origin_inflow", flag(self.origin_inflow)),
        ]
    }

    /// The names of the options.
    pub const NAMES: [&'static str; 8] = [
        "priority",
        "reroute",
        "reroute_after_s",
        "reroute_max",
        "reroute_min_gain",
        "pocket_length_m",
        "node_inflow",
        "origin_inflow",
    ];

    /// The shipped values with `options` in place of their namesakes. `priority`, `reroute`,
    /// `node_inflow` and `origin_inflow` are 0 or 1.
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
                "node_inflow" => d.node_inflow = flag(name, v)?,
                "origin_inflow" => d.origin_inflow = flag(name, v)?,
                "reroute" => {
                    d.reroute =
                        if flag(name, v)? { RerouteSetting::On } else { RerouteSetting::Off };
                }
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
    /// Settle [`RerouteSetting::Auto`] first ([`Self::resolved`]): here it counts as on.
    #[must_use]
    pub fn any(&self) -> bool {
        self.priority || self.reroute != RerouteSetting::Off || self.pocket_length_m > 0.0
    }

    /// Whether this run re-routes, at `level` (`None`: free flow, where nothing is ever held);
    /// `disrupted`: whether the loading has road disruptions, which hold vehicles even at the
    /// point queue.
    #[must_use]
    pub fn reroutes(&self, level: Option<FidelityLevel>, disrupted: bool) -> bool {
        match (self.reroute, level) {
            (_, None) | (RerouteSetting::Off, _) => false,
            (RerouteSetting::On, Some(_)) => true,
            (RerouteSetting::Auto, Some(level)) => level != FidelityLevel::PointQueue || disrupted,
        }
    }

    /// These options with the rerouting setting settled for `level` and a run with or without
    /// road disruptions ([`Self::reroutes`]): what a run hashes and reports, so a point-queue
    /// run and a level-4 run of the shipped options say what each does.
    #[must_use]
    pub fn resolved(&self, level: Option<FidelityLevel>, disrupted: bool) -> Self {
        let reroute =
            if self.reroutes(level, disrupted) { RerouteSetting::On } else { RerouteSetting::Off };
        Self { reroute, ..*self }
    }
}

impl Default for LoadingOptions {
    fn default() -> Self {
        Self::SHIPPED
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(pairs: &[(&str, f64)]) -> Result<LoadingOptions, String> {
        LoadingOptions::from_options(&pairs.iter().map(|&(n, v)| (n.to_string(), v)).collect())
    }

    /// **Property:** the two rules of entry (S293) are flags, shipped as the loading was before
    /// them (the headway at nodes kept, departures at any rate), read by name, listed with the
    /// others, and refused, with the reason or the list of names, when the value or the name is
    /// wrong.
    #[test]
    fn the_rules_of_entry_are_flags_read_by_name() {
        let shipped = LoadingOptions::SHIPPED;
        assert!(shipped.node_inflow && !shipped.origin_inflow);
        let values = shipped.values();
        assert_eq!(values.iter().map(|v| v.0).collect::<Vec<_>>(), LoadingOptions::NAMES);
        assert!(values.contains(&("node_inflow", 1.0)) && values.contains(&("origin_inflow", 0.0)));
        let read_back = read(&[("node_inflow", 0.0), ("origin_inflow", 1.0)]).expect("valid");
        assert!(!read_back.node_inflow && read_back.origin_inflow);
        assert_eq!(read(&[("node_inflow", 1.0), ("origin_inflow", 0.0)]), Ok(shipped));
        for name in ["node_inflow", "origin_inflow"] {
            let refused = read(&[(name, 2.0)]).expect_err("2 is not a flag");
            assert_eq!(refused, format!("loading_options \"{name}\" must be 0 or 1, got 2"));
        }
        let unknown = read(&[("node_inflows", 0.0)]).expect_err("not an option");
        assert_eq!(
            unknown,
            "loading_options has no \"node_inflows\"; the options are: priority, reroute, \
             reroute_after_s, reroute_max, reroute_min_gain, pocket_length_m, node_inflow, \
             origin_inflow"
        );
    }
}
