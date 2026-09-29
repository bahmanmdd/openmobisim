//! The defaults of scheduled public transport (S199): part of the defaults
//! table (they joined it at version 4), so a change is a modelling change and
//! bumps [`openmobisim_core_graph::DEFAULTS_VERSION`].

/// Transit's defaults: boarding, walking to and between stops, and buses.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct TransitDefaults {
    /// How long before a run leaves a passenger must be at the stop to board
    /// it, in seconds, at every boarding.
    ///
    /// *An assumption, and a common routing-engine convention: one minute to
    /// reach the platform and the door, first boarding and transfers alike.
    /// CITATION OWED.*
    pub board_slack_s: u32,
    /// The most vehicles one journey takes.
    ///
    /// *A resource limit, not behaviour: at eight, seven changes, far beyond the
    /// journeys anyone makes in a city.*
    pub max_rides: u32,
    /// The longest walk from an origin to a stop, or from a stop to a
    /// destination, in seconds (at walking speed, 900 s is 1.2 km).
    ///
    /// *An assumption. CITATION OWED: the catchment of urban stops in access
    /// studies is 400–800 m for buses and up to 1.2 km for rail.*
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
    /// *CITATION OWED: dwell times of urban buses are typically 10–40 s a stop
    /// and depend on boardings; 20 s stands for a typical stop until
    /// boarding-dependent dwell arrives.*
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
        access_walk_max_s: 900.0,
        transfer_walk_max_s: 300.0,
        stop_walk_snap_m: 300.0,
        stop_transfer_s: 0.0,
        bus_dwell_s: 20.0,
        bus_pcu: 2.0,
        bus_plausibility_ratio: 1.6,
        bus_stop_snap_m: 300.0,
    };
}

impl Default for TransitDefaults {
    fn default() -> Self {
        Self::SHIPPED
    }
}
