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
//!
//! **Level 1** (the static assignment, S294) reads two more: the volume-delay function per road
//! class ([`LoadingOptions::bpr`]) and the length of the period its hourly flows are of
//! ([`LoadingOptions::period_s`]); no other level reads them.

use std::collections::BTreeMap;

use openmobisim_core_graph::defaults::RoadClass;
use openmobisim_core_loading::{Bpr, FidelityLevel};

/// Whether en-route rerouting is on: by the level, or as asked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RerouteSetting {
    /// On at the levels with spillback (3 and 4), where a vehicle can be held for room; off at
    /// the point queue (2), with or without road disruptions. The shipped setting.
    ///
    /// From S289 to S293 it was on at the point queue in a loading with road disruptions too.
    /// That made the model itself depend on the disruption: a run with a disruption re-routed
    /// and the same run without it did not, so the two were not compared on equal terms; and
    /// iterated with it on, a run of Delft at 80 % of its capacity never converged, where with
    /// it off it converged steadily (S293). A driver who does not know of a closure now waits
    /// for it to reopen; [`Self::On`] lets them re-route.
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
    /// Amsterdam's morning at 1× and 1.25× and leaves uncongested results as they were); off at
    /// the point queue, where nothing waits for room, with or without road disruptions, unless
    /// asked for (S293; why: [`RerouteSetting::Auto`]).* The option `reroute` is 1 (on) or 0
    /// (off) at every level; [`Self::resolved`] settles the shipped setting for a run.
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
    /// At the point queue only (`flow_level` 2, and the point-queue warm-up loadings of levels 3
    /// and 4), whether a vehicle's entry onto the next link waits for that link's inflow headway
    /// ([`openmobisim_core_loading::LtmNetwork::with_node_inflow`]): off (0), **the classic
    /// point queue**: the next link takes every vehicle at once, a link's exit is bounded by its
    /// own discharge capacity alone, and a bottleneck's queue sits at its own exit; on (1), a
    /// vehicle held so at the end of its link holds up those behind it bound elsewhere, and a
    /// bottleneck's queue forms at the end of the links feeding it. Either way a link closed by
    /// a road disruption refuses entry: the vehicles bound onto it wait at the end of the link
    /// before it, where one that may re-route is offered a new route; a narrowed link takes them
    /// and queues them at its own exit. No effect on a loading at level 3 or 4, where the
    /// headway is part of the receiving condition.
    ///
    /// *Uncalibrated: off since S293, the point queue of Vickrey (1969, *Congestion theory and
    /// transport investment*, *American Economic Review* 59(2) 251–260) and of Han, Friesz and
    /// Yao (2013, *A partial differential equation formulation of Vickrey's bottleneck model*,
    /// *Transportation Research Part B* 49); on, the loading as it was until then.*
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
    /// **Level 1 only:** the volume-delay function, `t = t0 · (1 + α (v/c)^β)`, one `(α, β)` per
    /// road class, by name `bpr_alpha` and `bpr_beta` for every class and `<class>.bpr_alpha`,
    /// `<class>.bpr_beta` for one (`<class>` as OpenStreetMap writes it: `motorway`,
    /// `residential` …; the classes cars use).
    ///
    /// *Uncalibrated: α 0.15 and β 4 for every class, the Bureau of Public Roads' (1964, *Traffic
    /// Assignment Manual*), which TNTP's benchmark networks use.*
    pub bpr: Bpr,
    /// **Level 1 only:** the length of the period the demand departs over, in seconds, which its
    /// hourly flows are of: a link's flow is the PCU routed over it divided by this period in
    /// hours. 0 (the default) reads it from the departures, before they are moved to the
    /// window's start: from the earliest rounded down to the quarter hour to the end of the
    /// quarter hour of the latest (a demand departing over one hour is one hour of flow,
    /// whatever the window). *The scenario layer settles it into the run's flow motor.*
    pub period_s: u32,
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
        node_inflow: false,
        origin_inflow: false,
        bpr: Bpr::SHIPPED,
        period_s: 0,
    };

    /// Every option's value, by name, in [`Self::NAMES`]' order (the parameter listing): the
    /// BPR pair as `bpr_alpha` and `bpr_beta` while every class has the same (as shipped),
    /// otherwise each class's `<class>.bpr_alpha` and `<class>.bpr_beta`, in [`RoadClass::ALL`]'s
    /// order, for the classes cars use; `priority`, `reroute`, `node_inflow` and
    /// `origin_inflow` as 0 or 1 (`reroute` 1 for [`RerouteSetting::Auto`]: on where it can act;
    /// [`Self::resolved`] gives a run's own).
    #[must_use]
    pub fn values(&self) -> Vec<(String, f64)> {
        let flag = |b: bool| if b { 1.0 } else { 0.0 };
        let mut out: Vec<(String, f64)> = [
            ("priority", flag(self.priority)),
            ("reroute", flag(self.reroute != RerouteSetting::Off)),
            ("reroute_after_s", self.reroute_after_s),
            ("reroute_max", f64::from(self.reroute_max)),
            ("reroute_min_gain", self.reroute_min_gain),
            ("pocket_length_m", self.pocket_length_m),
            ("node_inflow", flag(self.node_inflow)),
            ("origin_inflow", flag(self.origin_inflow)),
            ("period_s", f64::from(self.period_s)),
        ]
        .into_iter()
        .map(|(n, v)| (n.to_string(), v))
        .collect();
        let uniform = BPR_CLASSES.iter().all(|&c| {
            let (i, j) = (c as usize, BPR_CLASSES[0] as usize);
            self.bpr.alpha[i].to_bits() == self.bpr.alpha[j].to_bits()
                && self.bpr.beta[i].to_bits() == self.bpr.beta[j].to_bits()
        });
        if uniform {
            let first = BPR_CLASSES[0] as usize;
            out.push(("bpr_alpha".to_string(), self.bpr.alpha[first]));
            out.push(("bpr_beta".to_string(), self.bpr.beta[first]));
        } else {
            for class in BPR_CLASSES {
                let i = class as usize;
                out.push((format!("{}.bpr_alpha", class.as_str()), self.bpr.alpha[i]));
                out.push((format!("{}.bpr_beta", class.as_str()), self.bpr.beta[i]));
            }
        }
        out
    }

    /// The names of the options (`<class>` standing for a road class's name).
    pub const NAMES: [&'static str; 13] = [
        "priority",
        "reroute",
        "reroute_after_s",
        "reroute_max",
        "reroute_min_gain",
        "pocket_length_m",
        "node_inflow",
        "origin_inflow",
        "period_s",
        "bpr_alpha",
        "bpr_beta",
        "<class>.bpr_alpha",
        "<class>.bpr_beta",
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
                "period_s" => {
                    if !(v.fract() == 0.0 && (0.0..=86_400.0).contains(&v)) {
                        return Err(format!(
                            "loading_options \"period_s\" must be a whole number of seconds from \
                             0 (read from the departures) to 86 400, got {v}"
                        ));
                    }
                    #[allow(
                        clippy::cast_possible_truncation,
                        clippy::cast_sign_loss,
                        reason = "checked to be a whole number of seconds in a day"
                    )]
                    let n = v as u32;
                    d.period_s = n;
                }
                "bpr_alpha" | "bpr_beta" => {
                    let v = bpr_value(name, v)?;
                    for class in BPR_CLASSES {
                        set_bpr(&mut d.bpr, class, name, v);
                    }
                }
                _ => {
                    let class = name
                        .strip_suffix(".bpr_alpha")
                        .or_else(|| name.strip_suffix(".bpr_beta"))
                        .and_then(|c| BPR_CLASSES.into_iter().find(|k| k.as_str() == c));
                    let Some(class) = class else {
                        return Err(format!(
                            "loading_options has no {name:?}; the options are: {} (<class> one \
                             of {})",
                            Self::NAMES.join(", "),
                            BPR_CLASSES.map(RoadClass::as_str).join(", ")
                        ));
                    };
                    let leaf = &name[class.as_str().len() + 1..];
                    set_bpr(&mut d.bpr, class, leaf, bpr_value(name, v)?);
                }
            }
        }
        // A class's own pair wins over the one for every class, whatever the order given.
        for (name, &v) in options {
            if let Some((class, leaf)) = name.split_once('.') {
                if let Some(class) = BPR_CLASSES.into_iter().find(|k| k.as_str() == class) {
                    set_bpr(&mut d.bpr, class, leaf, v);
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

    /// Whether this run re-routes, at `level` (`None`: free flow, where nothing is ever held).
    /// The same with or without road disruptions (S293).
    #[must_use]
    pub fn reroutes(&self, level: Option<FidelityLevel>) -> bool {
        match (self.reroute, level) {
            (_, None) | (RerouteSetting::Off, _) => false,
            (RerouteSetting::On, Some(_)) => true,
            (RerouteSetting::Auto, Some(level)) => level != FidelityLevel::PointQueue,
        }
    }

    /// These options with the rerouting setting settled for `level` ([`Self::reroutes`]): what a
    /// run hashes and reports, so a point-queue run and a level-4 run of the shipped options say
    /// what each does.
    #[must_use]
    pub fn resolved(&self, level: Option<FidelityLevel>) -> Self {
        let reroute = if self.reroutes(level) { RerouteSetting::On } else { RerouteSetting::Off };
        Self { reroute, ..*self }
    }
}

impl Default for LoadingOptions {
    fn default() -> Self {
        Self::SHIPPED
    }
}

/// The road classes cars use: those a BPR pair can be given for by name.
pub const BPR_CLASSES: [RoadClass; 14] = [
    RoadClass::Motorway,
    RoadClass::MotorwayLink,
    RoadClass::Trunk,
    RoadClass::TrunkLink,
    RoadClass::Primary,
    RoadClass::PrimaryLink,
    RoadClass::Secondary,
    RoadClass::SecondaryLink,
    RoadClass::Tertiary,
    RoadClass::TertiaryLink,
    RoadClass::Unclassified,
    RoadClass::Residential,
    RoadClass::LivingStreet,
    RoadClass::Service,
];

/// A BPR coefficient given by `name`: α at least 0, β at least 1 (the curve convex, so the
/// equilibrium is unique in link flows), finite.
fn bpr_value(name: &str, v: f64) -> Result<f64, String> {
    let beta = name.ends_with("bpr_beta");
    let least = if beta { 1.0 } else { 0.0 };
    if v.is_finite() && v >= least {
        Ok(v)
    } else {
        Err(format!("loading_options {name:?} must be a finite number, at least {least}, got {v}"))
    }
}

/// Set `class`'s α (`leaf` ending `bpr_alpha`) or β.
fn set_bpr(bpr: &mut Bpr, class: RoadClass, leaf: &str, v: f64) {
    let i = class as usize;
    if leaf.ends_with("bpr_alpha") {
        bpr.alpha[i] = v;
    } else {
        bpr.beta[i] = v;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(pairs: &[(&str, f64)]) -> Result<LoadingOptions, String> {
        LoadingOptions::from_options(&pairs.iter().map(|&(n, v)| (n.to_string(), v)).collect())
    }

    /// **Property:** the two rules of entry (S293) are flags, shipped as the classic point queue
    /// (no headway at nodes) with departures at any rate, read by name, listed with the others,
    /// and refused, with the reason or the list of names, when the value or the name is wrong.
    #[test]
    fn the_rules_of_entry_are_flags_read_by_name() {
        let shipped = LoadingOptions::SHIPPED;
        assert!(!shipped.node_inflow && !shipped.origin_inflow);
        let values = shipped.values();
        let names: Vec<&str> = values.iter().map(|v| v.0.as_str()).take(9).collect();
        assert_eq!(names, LoadingOptions::NAMES[..9]);
        let has = |name: &str, value: f64| values.contains(&(name.to_string(), value));
        assert!(has("node_inflow", 0.0) && has("origin_inflow", 0.0));
        let read_back = read(&[("node_inflow", 1.0), ("origin_inflow", 1.0)]).expect("valid");
        assert!(read_back.node_inflow && read_back.origin_inflow);
        assert_eq!(read(&[("node_inflow", 0.0), ("origin_inflow", 0.0)]), Ok(shipped));
        for name in ["node_inflow", "origin_inflow"] {
            let refused = read(&[(name, 2.0)]).expect_err("2 is not a flag");
            assert_eq!(refused, format!("loading_options \"{name}\" must be 0 or 1, got 2"));
        }
        let unknown = read(&[("node_inflows", 0.0)]).expect_err("not an option");
        assert!(
            unknown.starts_with(
                "loading_options has no \"node_inflows\"; the options are: priority, reroute, \
                 reroute_after_s, reroute_max, reroute_min_gain, pocket_length_m, node_inflow, \
                 origin_inflow, period_s, bpr_alpha, bpr_beta, <class>.bpr_alpha, \
                 <class>.bpr_beta (<class> one of motorway, motorway_link,"
            ),
            "{unknown}"
        );
    }

    /// **Property (S294):** level 1's volume-delay function is read by name, for every class
    /// or for one (a class's own wins, whatever the order), listed per class, and refused out of
    /// range or for a class cars do not use; the period is whole seconds, 0 read from the
    /// departures.
    #[test]
    fn the_bpr_table_and_the_period_are_read_by_name() {
        let shipped = LoadingOptions::SHIPPED;
        assert_eq!(shipped.bpr, Bpr::SHIPPED);
        assert_eq!(shipped.period_s, 0);
        let values = shipped.values();
        let names: Vec<&str> = values.iter().map(|v| v.0.as_str()).collect();
        assert_eq!(names, LoadingOptions::NAMES[..11], "one pair for every class, as shipped");
        assert!(values.contains(&("bpr_alpha".to_string(), 0.15)));
        assert!(values.contains(&("bpr_beta".to_string(), 4.0)));
        let o = read(&[("bpr_alpha", 0.5), ("motorway.bpr_alpha", 0.8), ("bpr_beta", 5.0)])
            .expect("valid");
        let listed = o.values();
        assert!(listed.contains(&("motorway.bpr_alpha".to_string(), 0.8)), "each class listed");
        assert!(listed.contains(&("service.bpr_beta".to_string(), 5.0)));
        assert!(!listed.iter().any(|(n, _)| n.starts_with("cycleway.")));
        assert!((o.bpr.alpha[RoadClass::Motorway as usize] - 0.8).abs() < f64::EPSILON);
        assert!((o.bpr.alpha[RoadClass::Primary as usize] - 0.5).abs() < f64::EPSILON);
        assert!((o.bpr.beta[RoadClass::Residential as usize] - 5.0).abs() < f64::EPSILON);
        assert_eq!(read(&[("period_s", 3600.0)]).expect("valid").period_s, 3600);
        assert!(read(&[("period_s", 0.5)]).is_err());
        assert!(read(&[("bpr_beta", 0.5)]).expect_err("below 1").contains("at least 1"));
        assert!(read(&[("bpr_alpha", -0.1)]).is_err());
        assert!(
            read(&[("cycleway.bpr_alpha", 0.1)]).expect_err("not a car class").contains("has no")
        );
    }

    /// **Property (S293):** rerouting is settled by the setting and the level alone: `Auto` is
    /// on at levels 3 and 4 and off at the point queue and at free flow, `On` on and `Off` off at
    /// every level that loads, and nothing re-routes at free flow; the settled options say the
    /// same, and keep every other option.
    #[test]
    fn rerouting_is_settled_by_the_setting_and_the_level_alone() {
        let levels = [
            None,
            Some(FidelityLevel::PointQueue),
            Some(FidelityLevel::SpatialQueue),
            Some(FidelityLevel::Full),
        ];
        for setting in [RerouteSetting::Auto, RerouteSetting::Off, RerouteSetting::On] {
            let o = LoadingOptions { reroute: setting, ..LoadingOptions::SHIPPED };
            for level in levels {
                let expected = match (setting, level) {
                    (_, None) | (RerouteSetting::Off, _) => false,
                    (RerouteSetting::On, Some(_)) => true,
                    (RerouteSetting::Auto, Some(l)) => {
                        matches!(l, FidelityLevel::SpatialQueue | FidelityLevel::Full)
                    }
                };
                assert_eq!(o.reroutes(level), expected, "{setting:?} at {level:?}");
                let settled = o.resolved(level);
                let want = if expected { RerouteSetting::On } else { RerouteSetting::Off };
                assert_eq!(
                    settled,
                    LoadingOptions { reroute: want, ..o },
                    "{setting:?} at {level:?}"
                );
                assert_eq!(settled.reroutes(level), expected, "settled once, settled for good");
            }
        }
    }
}
