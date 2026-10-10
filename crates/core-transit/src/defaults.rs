//! The defaults of scheduled public transport (S199): part of the defaults
//! table (they joined it at version 4), so a change is a modelling change and
//! bumps [`openmobisim_core_graph::DEFAULTS_VERSION`].

/// Transit's defaults: boarding, walking to and between stops, and buses.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct TransitDefaults {
    /// How long before a run leaves a passenger must be at the stop to board
    /// it, in seconds, at every boarding.
    ///
    /// *An assumption: one minute to reach the platform and the door, first boarding and
    /// transfers alike. Routing engines differ — OpenTripPlanner's `boardSlack` defaults to 0
    /// (<https://docs.opentripplanner.org/en/latest/RouteRequest/>) — and no published value
    /// stands behind it (S259).*
    pub board_slack_s: u32,
    /// The most vehicles one journey takes.
    ///
    /// *A resource limit, not behaviour: at eight, seven changes, far beyond the
    /// journeys anyone makes in a city.*
    pub max_rides: u32,
    /// The longest walk from an origin to a stop, or from a stop to a
    /// destination, in seconds (at walking speed, 1 800 s is 2.4 km).
    ///
    /// *An assumption: a choice-set limit, not a preference (the walk's minutes are
    /// weighed by the choice model). 30 minutes (S235; 15 until then, which left
    /// travellers without a vehicle no way to their destination, S233). In Montréal the 85th
    /// percentile of the walk to a stop is about 524 m for buses and 1 259 m for commuter rail
    /// (El-Geneidy et al. 2014, *Transportation* 41(1) 193–210,
    /// <https://ideas.repec.org/a/kap/transp/v41y2014i1p193-210.html>): the limit leaves room
    /// for the few who walk further.*
    pub access_walk_max_s: f64,
    /// The longest walk between two stops in a change, in seconds (300 s is
    /// 400 m).
    ///
    /// *An assumption.*
    pub transfer_walk_max_s: f64,
    /// The farthest a stop may be from the walk layer's nearest node, in metres,
    /// to be reached on foot; a stop farther away is outside the study area.
    ///
    /// *An assumption: the layers' nodes are junctions (the networks are
    /// contracted), so a stop mid-block can be half a block from the nearest.*
    pub stop_walk_snap_m: f64,
    /// The time a transfer at a stop takes between its access points (walk,
    /// bike and platform), in seconds: zero, since the walk to the platform is
    /// the walk leg's and the margin before boarding is
    /// [`Self::board_slack_s`].
    pub stop_transfer_s: f64,
    /// How long a bus stays at each stop, in seconds (D5: fixed, S195).
    ///
    /// *An assumption: 20 s stands for a typical stop until boarding-dependent dwell arrives;
    /// a bus's dwell depends mostly on its boardings and alightings (Dueker et al. 2004,
    /// *Journal of Public Transportation* 7(1) 21–40, <https://doi.org/10.5038/2375-0901.7.1.2>).*
    pub bus_dwell_s: f64,
    /// A bus's passenger car units on the roads (design §10.4).
    pub bus_pcu: f64,
    /// A bus pattern whose free-flow time on the roads, stop to stop, exceeds
    /// its scheduled time by more than this factor is run by the schedule
    /// instead (the plausibility gate, design §18.5; the interface's
    /// `plausibility_ratio`).
    pub bus_plausibility_ratio: f64,
    /// The farthest a bus stop may be from the road network's nearest node, in
    /// metres, for its buses to ride the roads; a pattern with a stop farther
    /// away is run by the schedule.
    ///
    /// *An assumption, as for [`Self::stop_walk_snap_m`].*
    pub bus_stop_snap_m: f64,
}

impl TransitDefaults {
    /// The shipped values.
    pub const SHIPPED: TransitDefaults = TransitDefaults {
        board_slack_s: 60,
        max_rides: 8,
        access_walk_max_s: 1800.0,
        transfer_walk_max_s: 300.0,
        stop_walk_snap_m: 300.0,
        stop_transfer_s: 0.0,
        bus_dwell_s: 20.0,
        bus_pcu: 2.0,
        bus_plausibility_ratio: 1.6,
        bus_stop_snap_m: 300.0,
    };
}

/// An option [`TransitDefaults::from_options`] refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransitOptionError {
    /// The option.
    pub option: String,
    /// What is wrong with it.
    pub reason: String,
}

impl std::fmt::Display for TransitOptionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "transit option {:?}: {}", self.option, self.reason)
    }
}

impl std::error::Error for TransitOptionError {}

impl TransitDefaults {
    /// Every option's value, by name, in [`Self::NAMES`]' order (S230: the parameter listing).
    #[must_use]
    pub fn values(&self) -> Vec<(&'static str, f64)> {
        vec![
            ("board_slack_s", f64::from(self.board_slack_s)),
            ("max_rides", f64::from(self.max_rides)),
            ("access_walk_max_s", self.access_walk_max_s),
            ("transfer_walk_max_s", self.transfer_walk_max_s),
            ("stop_walk_snap_m", self.stop_walk_snap_m),
            ("stop_transfer_s", self.stop_transfer_s),
            ("bus_dwell_s", self.bus_dwell_s),
            ("bus_pcu", self.bus_pcu),
            ("bus_plausibility_ratio", self.bus_plausibility_ratio),
            ("bus_stop_snap_m", self.bus_stop_snap_m),
        ]
    }

    /// The names of the options, as [`Self::from_options`] takes them: the fields'.
    pub const NAMES: [&'static str; 10] = [
        "board_slack_s",
        "max_rides",
        "access_walk_max_s",
        "transfer_walk_max_s",
        "stop_walk_snap_m",
        "stop_transfer_s",
        "bus_dwell_s",
        "bus_pcu",
        "bus_plausibility_ratio",
        "bus_stop_snap_m",
    ];

    /// The shipped values with `options` in place of their namesakes (S202: every
    /// parameter overridable by name, for calibration).
    ///
    /// # Errors
    ///
    /// [`TransitOptionError`] for a name that is not one of [`Self::NAMES`], or a
    /// value that is not a finite non-negative number (`max_rides` a whole number
    /// of at least 1, `board_slack_s` a whole number).
    pub fn from_options(
        options: &std::collections::BTreeMap<String, f64>,
    ) -> Result<Self, TransitOptionError> {
        let mut d = Self::SHIPPED;
        for (name, &v) in options {
            let bad = |reason: &str| TransitOptionError {
                option: name.clone(),
                reason: reason.to_string(),
            };
            if !(v.is_finite() && v >= 0.0) {
                return Err(bad(&format!("must be a finite number of at least 0, got {v}")));
            }
            let whole = || {
                #[allow(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "checked"
                )]
                let n = v as u32;
                #[allow(clippy::float_cmp, reason = "a whole number is exactly itself")]
                let exact = f64::from(n) == v;
                exact.then_some(n)
            };
            match name.as_str() {
                "board_slack_s" => {
                    d.board_slack_s =
                        whole().ok_or_else(|| bad("must be a whole number of seconds"))?
                }
                "max_rides" => {
                    d.max_rides = whole()
                        .filter(|&n| n >= 1)
                        .ok_or_else(|| bad("must be a whole number of at least 1"))?;
                }
                "access_walk_max_s" => d.access_walk_max_s = v,
                "transfer_walk_max_s" => d.transfer_walk_max_s = v,
                "stop_walk_snap_m" => d.stop_walk_snap_m = v,
                "stop_transfer_s" => d.stop_transfer_s = v,
                "bus_dwell_s" => d.bus_dwell_s = v,
                "bus_pcu" => d.bus_pcu = v,
                "bus_plausibility_ratio" => d.bus_plausibility_ratio = v,
                "bus_stop_snap_m" => d.bus_stop_snap_m = v,
                _ => {
                    return Err(bad(&format!(
                        "no such option; the options are: {}",
                        Self::NAMES.join(", ")
                    )));
                }
            }
        }
        Ok(d)
    }
}

impl Default for TransitDefaults {
    fn default() -> Self {
        Self::SHIPPED
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn options_replace_their_namesakes_and_a_wrong_name_or_value_is_refused() {
        let o = |pairs: &[(&str, f64)]| {
            pairs.iter().map(|&(k, v)| (k.to_string(), v)).collect::<BTreeMap<_, _>>()
        };
        let d = TransitDefaults::from_options(&o(&[("bus_dwell_s", 30.0), ("max_rides", 3.0)]))
            .unwrap();
        assert_eq!((d.bus_dwell_s, d.max_rides), (30.0, 3));
        assert_eq!(d.board_slack_s, TransitDefaults::SHIPPED.board_slack_s);
        let e = TransitDefaults::from_options(&o(&[("dwell", 30.0)])).unwrap_err().to_string();
        assert!(e.contains("dwell") && e.contains("bus_dwell_s"), "{e}");
        assert!(TransitDefaults::from_options(&o(&[("max_rides", 0.0)])).is_err());
        assert!(TransitDefaults::from_options(&o(&[("board_slack_s", 1.5)])).is_err());
        assert!(TransitDefaults::from_options(&o(&[("bus_pcu", -1.0)])).is_err());
    }
}
