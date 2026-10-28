//! Shared-mobility services in a run, and the hubs they and every other subsystem share.
//!
//! A **service** is a shared-mobility system a scenario adds: docked bike sharing
//! ([`ServiceKind::BikeDocked`], bikes counted per dock) or electric taxis
//! ([`ServiceKind::Taxi`], vehicles tracked one by one, with a battery). Its docks, stands and
//! chargers are resources of the run's one hub set ([`hubs::RunHubs`]); its day is simulated by
//! the fleet engine ([`engine::FleetEngine`]), one clock for every service.
//!
//! | Module | What it owns |
//! |---|---|
//! | [`hubs`] | [`hubs::RunHubs`] — the run's one hub set: stops, parkings, docks, chargers and stands, joined by hub id or by distance |
//! | [`engine`] | [`engine::FleetEngine`] — the services' day: legs, vehicles and docks, events in time order |
//! | [`outcome`] | [`outcome::FleetOutcome`] — what became of each leg, and the day's counts |
//! | [`docks`] | [`docks::DockBins`] — docked bikes' pickups, returns and redirections on the engine, bin by bin |
//! | [`costs`] | [`costs::DockCosts`] — what a dock is expected to cost a pickup and a return, per bin |
//!
//! **Cost:** per dock about 60 bytes and its neighbours, found by one bounded walk search per
//! dock, once per run; per vehicle and charger about 40 bytes.

pub mod costs;
pub mod docks;
pub mod engine;
pub mod hubs;
pub mod outcome;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;

use openmobisim_core_graph::geometry::{LonLat, ground_distance_metres};
use openmobisim_core_graph::hubs::{AccessPoint, Resource, ResourceKind};
use openmobisim_core_graph::layers::Layer;
use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_routes::{NodeSnapper, Reach};
use openmobisim_core_types::ids::{EntityId, NodeId, ResourceId};

use crate::layers::LayerSetup;
use crate::transit::par_map;
use hubs::{ResourceRow, RunHubs};

/// Named options, as a service's `options`.
pub type Options = BTreeMap<String, f64>;

/// What kind of shared mobility a service is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ServiceKind {
    /// Bikes at docks, counted per dock; a leg walks to a dock, rides, returns at a dock.
    BikeDocked,
    /// Electric taxis, tracked one by one; a leg waits for a vehicle and rides.
    Taxi,
}

impl ServiceKind {
    /// The stable snake_case name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            ServiceKind::BikeDocked => "bike_docked",
            ServiceKind::Taxi => "taxi",
        }
    }
}

// --- Docked bikes ------------------------------------------------------------------------------

/// One dock, as a table gives it (a GBFS station, an OSM element).
#[derive(Clone, Debug, PartialEq)]
pub struct DockRow {
    /// Its id, unique in the table (a GBFS `station_id`, an OSM element).
    pub dock_id: String,
    /// Its name, if it has one.
    pub name: Option<String>,
    /// The hub it belongs to, if its row names one; else the nearest stop's, or its own.
    pub hub_id: Option<String>,
    /// Where it is.
    pub position: LonLat,
    /// How many bikes it holds: its slots.
    pub capacity: u32,
    /// How many bikes are docked at the start of the day, if known (a GBFS status); else the
    /// service's share of the slots ([`DockDefaults::start_share`]).
    pub bikes: Option<u32>,
}

/// Docked bike sharing's defaults: every one an uncalibrated assumption, overridable by name.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DockDefaults {
    /// The longest walk to a dock from the origin, or from a dock to the destination, in
    /// seconds of the walk layer.
    ///
    /// *An assumption: 600 s is about 800 m. CITATION OWED: bike-sharing catchments of
    /// 300–500 m are commonly assumed.*
    pub walk_max_s: f64,
    /// The most dock pairs one trip chooses among; three times as many pairs are estimated.
    ///
    /// *A resource limit, as the parkings'.*
    pub candidates: f64,
    /// The farthest a dock may be from its layers' nearest node, in metres.
    ///
    /// *An assumption, as for a parking.*
    pub snap_m: f64,
    /// The most other docks a traveller tries when the one they reach is empty (about three).
    ///
    /// *An assumption. CITATION OWED.*
    pub redirect_max: f64,
    /// How far from the dock they planned a traveller looks for another, in metres as the
    /// crow flies (about 800 m).
    ///
    /// *An assumption. CITATION OWED.*
    pub redirect_max_m: f64,
    /// The time to take a bike from a dock, in seconds.
    ///
    /// *An assumption. CITATION OWED.*
    pub pickup_s: f64,
    /// The time to dock a bike, in seconds.
    ///
    /// *An assumption. CITATION OWED.*
    pub return_s: f64,
    /// A shared bike's ride time over the bike layer's, for the whole fleet (one speed per
    /// system: 1 rides as one's own bike, an e-bike fleet less).
    ///
    /// *An assumption.*
    pub ride_factor: f64,
    /// The share of a dock's slots holding a bike at the start of the day where its row gives
    /// no count (an OSM dock).
    ///
    /// *An assumption.*
    pub start_share: f64,
    /// What a dock costs a pickup when no dock near it has bikes, in seconds: the expected
    /// cost of finding no bike (walking, or another mode).
    ///
    /// *An assumption. CITATION OWED.*
    pub fail_s: f64,
    /// The length of the time bins counts and costs are kept in, in seconds.
    pub bin_s: f64,
    /// The time an operator's move takes per bike, loading and unloading, in seconds, on top
    /// of the drive.
    ///
    /// *An assumption. CITATION OWED.*
    pub handling_s: f64,
}

impl DockDefaults {
    /// The shipped values.
    pub const SHIPPED: DockDefaults = DockDefaults {
        walk_max_s: 600.0,
        candidates: 3.0,
        snap_m: 300.0,
        redirect_max: 3.0,
        redirect_max_m: 800.0,
        pickup_s: 60.0,
        return_s: 30.0,
        ride_factor: 1.0,
        start_share: 0.5,
        fail_s: 1200.0,
        bin_s: 900.0,
        handling_s: 30.0,
    };

    /// The names of the options, in the order [`Self::values`] gives them.
    pub const NAMES: [&'static str; 12] = [
        "walk_max_s",
        "candidates",
        "snap_m",
        "redirect_max",
        "redirect_max_m",
        "pickup_s",
        "return_s",
        "ride_factor",
        "start_share",
        "fail_s",
        "bin_s",
        "handling_s",
    ];

    /// Every option, by name, in [`Self::NAMES`]' order (the parameter listing, the manifest).
    #[must_use]
    pub fn values(&self) -> Vec<(&'static str, f64)> {
        vec![
            ("walk_max_s", self.walk_max_s),
            ("candidates", self.candidates),
            ("snap_m", self.snap_m),
            ("redirect_max", self.redirect_max),
            ("redirect_max_m", self.redirect_max_m),
            ("pickup_s", self.pickup_s),
            ("return_s", self.return_s),
            ("ride_factor", self.ride_factor),
            ("start_share", self.start_share),
            ("fail_s", self.fail_s),
            ("bin_s", self.bin_s),
            ("handling_s", self.handling_s),
        ]
    }

    /// The shipped values with `options` in place of their namesakes.
    ///
    /// # Errors
    ///
    /// [`ServiceError::UnknownOption`] for a name not in [`Self::NAMES`],
    /// [`ServiceError::BadOption`] for a value that is not finite, is negative, or is out of its
    /// range (`ride_factor` and `bin_s` above 0, `start_share` at most 1).
    pub fn from_options(options: &Options) -> Result<Self, ServiceError> {
        let mut d = Self::SHIPPED;
        for (name, &value) in options {
            let slot = match name.as_str() {
                "walk_max_s" => &mut d.walk_max_s,
                "candidates" => &mut d.candidates,
                "snap_m" => &mut d.snap_m,
                "redirect_max" => &mut d.redirect_max,
                "redirect_max_m" => &mut d.redirect_max_m,
                "pickup_s" => &mut d.pickup_s,
                "return_s" => &mut d.return_s,
                "ride_factor" => &mut d.ride_factor,
                "start_share" => &mut d.start_share,
                "fail_s" => &mut d.fail_s,
                "bin_s" => &mut d.bin_s,
                "handling_s" => &mut d.handling_s,
                _ => {
                    return Err(ServiceError::UnknownOption {
                        kind: ServiceKind::BikeDocked,
                        name: name.clone(),
                        known: Self::NAMES.to_vec(),
                    });
                }
            };
            let positive = matches!(name.as_str(), "ride_factor" | "bin_s");
            if !value.is_finite()
                || value < 0.0
                || (positive && value <= 0.0)
                || (name == "start_share" && value > 1.0)
            {
                return Err(ServiceError::BadOption {
                    kind: ServiceKind::BikeDocked,
                    name: name.clone(),
                    value,
                    why: "out of its range".to_string(),
                });
            }
            *slot = value;
        }
        Ok(d)
    }

    /// [`Self::candidates`] as a count, at least one.
    #[must_use]
    pub fn candidate_count(&self) -> usize {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "a small count")]
        let n = self.candidates.round().max(1.0) as usize;
        n
    }

    /// [`Self::redirect_max`] as a count.
    #[must_use]
    pub fn redirect_count(&self) -> usize {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "a small count")]
        let n = self.redirect_max.round().max(0.0) as usize;
        n
    }
}

impl Default for DockDefaults {
    fn default() -> Self {
        Self::SHIPPED
    }
}

// --- Taxis -------------------------------------------------------------------------------------

/// One vehicle of a fleet, as a table gives it.
#[derive(Clone, Debug, PartialEq)]
pub struct VehicleRow {
    /// Its id, unique in the table.
    pub vehicle_id: String,
    /// Where it is at the start of the day.
    pub position: LonLat,
    /// Its charge at the start of the day, as a share; else [`TaxiDefaults::soc_start`].
    pub soc: Option<f64>,
}

/// One charger, as a table gives it.
#[derive(Clone, Debug, PartialEq)]
pub struct ChargerRow {
    /// Its id, unique in the table.
    pub charger_id: String,
    /// The hub it belongs to, if its row names one; else the nearest stop's, or its own.
    pub hub_id: Option<String>,
    /// Where it is.
    pub position: LonLat,
    /// How many vehicles it charges at once.
    pub plugs: u32,
    /// Its power per plug, in kW; else [`TaxiDefaults::charger_kw`].
    pub kw: Option<f64>,
}

/// An electric taxi fleet's defaults: every one an uncalibrated assumption, overridable by name.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TaxiDefaults {
    /// A vehicle's battery, in kWh.
    ///
    /// *An assumption: a mid-size electric car's usable battery.*
    pub battery_kwh: f64,
    /// Energy used per kilometre driven, in kWh.
    ///
    /// *An assumption for urban driving of a mid-size electric car.*
    pub kwh_per_km: f64,
    /// The charge, as a share of the battery, of a vehicle whose table gives none.
    ///
    /// *An assumption.*
    pub soc_start: f64,
    /// The charge a vehicle keeps after a trip, as a share: one that would end a request below
    /// it is not sent (room to reach a charger).
    ///
    /// *An assumption.*
    pub reserve_soc: f64,
    /// The default charging rule's threshold: a vehicle that drops a passenger below this share
    /// drives to charge.
    ///
    /// *An assumption.*
    pub charge_below: f64,
    /// The share a vehicle charges to.
    ///
    /// *An assumption: charging slows above about 80 %.*
    pub charge_to: f64,
    /// A charger's power per plug, in kW, where its table gives none.
    ///
    /// *An assumption: a fast charger's.*
    pub charger_kw: f64,
    /// The longest a request waits for a vehicle, in seconds; past it the trip is not made.
    ///
    /// *An assumption.*
    pub max_wait_s: f64,
    /// The time to board at the pickup, in seconds.
    ///
    /// *An assumption.*
    pub board_s: f64,
    /// The farthest a vehicle or charger may be from the road network, in metres.
    ///
    /// *An assumption, as for docks and parkings.*
    pub snap_m: f64,
    /// How far, in seconds of free-flow driving, a vehicle looks for its nearest charger.
    ///
    /// *An assumption: an hour's drive covers a city.*
    pub charger_reach_s: f64,
}

impl TaxiDefaults {
    /// The shipped values.
    pub const SHIPPED: Self = Self {
        battery_kwh: 60.0,
        kwh_per_km: 0.18,
        soc_start: 0.8,
        reserve_soc: 0.15,
        charge_below: 0.2,
        charge_to: 0.8,
        charger_kw: 50.0,
        max_wait_s: 600.0,
        board_s: 60.0,
        snap_m: 300.0,
        charger_reach_s: 3600.0,
    };

    /// Every option, by name, with its value.
    #[must_use]
    pub fn values(&self) -> Vec<(&'static str, f64)> {
        vec![
            ("battery_kwh", self.battery_kwh),
            ("kwh_per_km", self.kwh_per_km),
            ("soc_start", self.soc_start),
            ("reserve_soc", self.reserve_soc),
            ("charge_below", self.charge_below),
            ("charge_to", self.charge_to),
            ("charger_kw", self.charger_kw),
            ("max_wait_s", self.max_wait_s),
            ("board_s", self.board_s),
            ("snap_m", self.snap_m),
            ("charger_reach_s", self.charger_reach_s),
        ]
    }

    /// The shipped values with `options` replacing them by name.
    ///
    /// # Errors
    ///
    /// [`ServiceError::UnknownOption`] naming the known ones; [`ServiceError::BadOption`] for a
    /// value out of its range (shares in 0..=1 with the reserve at most the threshold and the
    /// threshold below the target; energies, powers and the battery above 0; times and
    /// distances not negative).
    pub fn from_options(options: &Options) -> Result<Self, ServiceError> {
        let mut d = Self::SHIPPED;
        for (name, &value) in options {
            let slot = match name.as_str() {
                "battery_kwh" => &mut d.battery_kwh,
                "kwh_per_km" => &mut d.kwh_per_km,
                "soc_start" => &mut d.soc_start,
                "reserve_soc" => &mut d.reserve_soc,
                "charge_below" => &mut d.charge_below,
                "charge_to" => &mut d.charge_to,
                "charger_kw" => &mut d.charger_kw,
                "max_wait_s" => &mut d.max_wait_s,
                "board_s" => &mut d.board_s,
                "snap_m" => &mut d.snap_m,
                "charger_reach_s" => &mut d.charger_reach_s,
                _ => {
                    return Err(ServiceError::UnknownOption {
                        kind: ServiceKind::Taxi,
                        name: name.clone(),
                        known: Self::SHIPPED.values().iter().map(|(n, _)| *n).collect(),
                    });
                }
            };
            *slot = value;
        }
        let bad = |name: &str, value: f64, why: &str| {
            Err(ServiceError::BadOption {
                kind: ServiceKind::Taxi,
                name: name.to_string(),
                value,
                why: why.to_string(),
            })
        };
        for (name, value) in [
            ("battery_kwh", d.battery_kwh),
            ("kwh_per_km", d.kwh_per_km),
            ("charger_kw", d.charger_kw),
        ] {
            if !(value.is_finite() && value > 0.0) {
                return bad(name, value, "must be above 0");
            }
        }
        for (name, value) in [
            ("max_wait_s", d.max_wait_s),
            ("board_s", d.board_s),
            ("snap_m", d.snap_m),
            ("charger_reach_s", d.charger_reach_s),
        ] {
            if !(value.is_finite() && value >= 0.0) {
                return bad(name, value, "must be 0 or more");
            }
        }
        for (name, value) in [
            ("soc_start", d.soc_start),
            ("reserve_soc", d.reserve_soc),
            ("charge_below", d.charge_below),
            ("charge_to", d.charge_to),
        ] {
            if !(0.0..=1.0).contains(&value) {
                return bad(name, value, "must be a share, from 0 to 1");
            }
        }
        if d.reserve_soc > d.charge_below || d.charge_below >= d.charge_to {
            return bad(
                "charge_below",
                d.charge_below,
                "needs reserve_soc <= charge_below < charge_to",
            );
        }
        Ok(d)
    }
}

impl Default for TaxiDefaults {
    fn default() -> Self {
        Self::SHIPPED
    }
}

// --- Errors ------------------------------------------------------------------------------------

/// Why a service could not be set up.
#[derive(Clone, Debug, PartialEq)]
pub enum ServiceError {
    /// An option the service's kind does not have.
    UnknownOption {
        /// The service's kind.
        kind: ServiceKind,
        /// The name given.
        name: String,
        /// The names that exist.
        known: Vec<&'static str>,
    },
    /// An option's value out of its range.
    BadOption {
        /// The service's kind.
        kind: ServiceKind,
        /// The option.
        name: String,
        /// The value given.
        value: f64,
        /// What it must be.
        why: String,
    },
    /// A row that cannot be used: a repeated id, a charge out of 0..=1, a power not above 0.
    BadRow(String),
    /// Nothing of the service is left near its layers.
    Empty(String),
}

impl fmt::Display for ServiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownOption { kind, name, known } => write!(
                f,
                "unknown {} option `{name}`; the options are {}",
                kind.as_str(),
                known.join(", ")
            ),
            Self::BadOption { kind, name, value, why } => {
                write!(f, "{} option `{name}` = {value}: {why}", kind.as_str())
            }
            Self::BadRow(why) | Self::Empty(why) => write!(f, "{why}"),
        }
    }
}

impl std::error::Error for ServiceError {}

// --- The setups --------------------------------------------------------------------------------

/// What setting a service up kept and left out.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ServiceSetupReport {
    /// Rows given (docks, or vehicles and chargers).
    pub rows: u32,
    /// Kept.
    pub kept: u32,
    /// Left out: farther than the service's `snap_m` from its layers.
    pub off_layer: u32,
    /// Left out: a dock without slots, a charger without plugs.
    pub empty: u32,
}

/// A docked service's docks, snapped to the walk and bike layers, in dock id order.
#[derive(Clone, Debug, Default)]
pub struct Docks {
    /// Per dock: its id, name, hub named by its row, position, walk and bike nodes, slots and
    /// bikes at the start of the day.
    pub external: Vec<String>,
    /// See [`Self::external`].
    pub name: Vec<Option<String>>,
    /// See [`Self::external`].
    pub hub_id: Vec<Option<String>>,
    /// See [`Self::external`].
    pub position: Vec<LonLat>,
    /// See [`Self::external`].
    pub walk_node: Vec<NodeId>,
    /// See [`Self::external`].
    pub bike_node: Vec<NodeId>,
    /// See [`Self::external`].
    pub capacity: Vec<u32>,
    /// See [`Self::external`].
    pub start: Vec<u32>,
    /// Per dock, CSR: the other docks within [`DockDefaults::redirect_max_m`] and their walk in
    /// seconds, nearest first.
    pub neighbours_start: Vec<u32>,
    /// See [`Self::neighbours_start`].
    pub neighbours: Vec<(u32, f64)>,
}

impl Docks {
    /// How many docks.
    #[must_use]
    pub fn len(&self) -> usize {
        self.external.len()
    }

    /// Whether there is none.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.external.is_empty()
    }

    /// Dock `dock`'s neighbours and the walk to each, in seconds, nearest first (none if the
    /// docks were given without their neighbours).
    #[must_use]
    pub fn neighbours(&self, dock: u32) -> &[(u32, f64)] {
        let d = dock as usize;
        match (self.neighbours_start.get(d), self.neighbours_start.get(d + 1)) {
            (Some(&a), Some(&b)) => &self.neighbours[a as usize..b as usize],
            _ => &[],
        }
    }

    /// The bikes docked at the start of the day: the fleet.
    #[must_use]
    pub fn fleet(&self) -> u64 {
        self.start.iter().map(|&b| u64::from(b)).sum()
    }
}

/// A taxi fleet's vehicles and chargers, snapped to the road network, in table order.
#[derive(Clone, Debug, Default)]
pub struct Fleet {
    /// Per vehicle: its id, its road node and its charge at the start of the day.
    pub vehicle_ids: Vec<String>,
    /// See [`Self::vehicle_ids`].
    pub vehicle_node: Vec<NodeId>,
    /// See [`Self::vehicle_ids`].
    pub vehicle_soc: Vec<f64>,
    /// Per charger: its id, the hub named by its row, its road node, position, plugs and power.
    pub charger_ids: Vec<String>,
    /// See [`Self::charger_ids`].
    pub charger_hub_id: Vec<Option<String>>,
    /// See [`Self::charger_ids`].
    pub charger_node: Vec<NodeId>,
    /// See [`Self::charger_ids`].
    pub charger_position: Vec<LonLat>,
    /// See [`Self::charger_ids`].
    pub charger_plugs: Vec<u32>,
    /// See [`Self::charger_ids`].
    pub charger_kw: Vec<f64>,
}

/// What a service holds, by kind.
#[derive(Clone, Debug)]
pub enum ServiceParts {
    /// A docked bike service: its docks and defaults.
    BikeDocked(Docks, DockDefaults),
    /// A taxi fleet: its vehicles, chargers and defaults.
    Taxi(Fleet, TaxiDefaults),
}

/// One service of a run: its name, kind, the mode its legs are, its parts snapped to the
/// layers and, once the run's hub set is built, its resources there.
#[derive(Clone, Debug)]
pub struct ServiceSetup {
    name: String,
    parts: ServiceParts,
    report: ServiceSetupReport,
    /// Per dock or charger, its resource in the run's hub set (empty until [`Self::link`]).
    resources: Vec<ResourceId>,
}

impl ServiceSetup {
    /// A docked bike service named `name`: `rows` snapped to `walk` and `bike` (a dock farther
    /// than `snap_m` from either, or without slots, is left out and counted), kept in dock id
    /// order, each with its neighbours for redirections.
    ///
    /// # Errors
    ///
    /// [`ServiceError::BadRow`] if two rows share a `dock_id`; [`ServiceError::Empty`] if no
    /// dock is left.
    ///
    /// # Panics
    ///
    /// Never in practice: counts of docks fit `u32`.
    pub fn bike_docked(
        name: &str,
        rows: &[DockRow],
        walk: &LayerSetup,
        bike: &LayerSetup,
        defaults: DockDefaults,
    ) -> Result<Self, ServiceError> {
        let mut seen = HashSet::new();
        for r in rows {
            if !seen.insert(r.dock_id.as_str()) {
                return Err(ServiceError::BadRow(format!("two docks have the id {:?}", r.dock_id)));
            }
        }
        let mut report = ServiceSetupReport {
            rows: u32::try_from(rows.len()).expect("docks fit u32"),
            ..ServiceSetupReport::default()
        };
        let walk_graph = walk.network().network();
        let bike_graph = bike.network().network();
        let walk_snapper = NodeSnapper::every_node(walk_graph);
        let bike_snapper = NodeSnapper::every_node(bike_graph);
        // Rows in id order, so the result does not depend on the table's order.
        let mut order: Vec<&DockRow> = rows.iter().collect();
        order.sort_by(|a, b| a.dock_id.cmp(&b.dock_id));
        let mut docks = Docks { neighbours_start: vec![0], ..Docks::default() };
        for row in order {
            if row.capacity == 0 {
                report.empty += 1;
                continue;
            }
            let w = walk_snapper.nearest(walk_graph, row.position);
            let b = bike_snapper.nearest(bike_graph, row.position);
            let near = |at: LonLat| ground_distance_metres(row.position, at) <= defaults.snap_m;
            if !(near(walk_graph.node_lonlat(w)) && near(bike_graph.node_lonlat(b))) {
                report.off_layer += 1;
                continue;
            }
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "a count")]
            let start = row
                .bikes
                .unwrap_or_else(|| (f64::from(row.capacity) * defaults.start_share).round() as u32);
            docks.external.push(row.dock_id.clone());
            docks.name.push(row.name.clone());
            docks.hub_id.push(row.hub_id.clone());
            docks.position.push(row.position);
            docks.walk_node.push(w);
            docks.bike_node.push(b);
            docks.capacity.push(row.capacity);
            docks.start.push(start.min(row.capacity));
            report.kept += 1;
        }
        if docks.external.is_empty() {
            return Err(ServiceError::Empty(format!("service `{name}`: no dock near the layers")));
        }
        // Each dock's neighbours: one walk search bounded at a slow walker's time over the
        // redirection distance (1 m/s), then those within it as the crow flies.
        let mut has_dock = vec![false; walk_graph.node_count() as usize];
        let mut at_walk: HashMap<u32, Vec<u32>> = HashMap::new();
        for (d, w) in docks.walk_node.iter().enumerate() {
            has_dock[w.index()] = true;
            at_walk.entry(w.raw()).or_default().push(u32::try_from(d).expect("docks fit u32"));
        }
        let ids: Vec<u32> =
            (0..u32::try_from(docks.external.len()).expect("docks fit u32")).collect();
        let lists = par_map(
            &ids,
            64,
            || Reach::new(walk_graph, walk.seconds()),
            |reach, &d| {
                let mut out: Vec<(u32, f64)> = Vec::new();
                for (node, secs) in
                    reach.forward(docks.walk_node[d as usize], defaults.redirect_max_m, &has_dock)
                {
                    for &other in &at_walk[&node.raw()] {
                        let far = ground_distance_metres(
                            docks.position[d as usize],
                            docks.position[other as usize],
                        );
                        if other != d && far <= defaults.redirect_max_m {
                            out.push((other, secs));
                        }
                    }
                }
                out
            },
        );
        for list in lists {
            docks.neighbours.extend(list);
            docks.neighbours_start.push(u32::try_from(docks.neighbours.len()).expect("fits u32"));
        }
        Ok(Self {
            name: name.to_string(),
            parts: ServiceParts::BikeDocked(docks, defaults),
            report,
            resources: Vec::new(),
        })
    }

    /// A taxi fleet named `name`: `vehicles` and `chargers` snapped to `road`'s nearest nodes;
    /// those farther than `snap_m`, and chargers without plugs, are left out and counted. Kept
    /// in table order.
    ///
    /// # Errors
    ///
    /// [`ServiceError::BadRow`] for a repeated id, a charge out of 0..=1 or a power not above
    /// 0; [`ServiceError::Empty`] if no vehicle is left.
    ///
    /// # Panics
    ///
    /// Never in practice: counts of vehicles and chargers fit `u32`.
    pub fn taxi(
        name: &str,
        road: &RoadNetwork,
        vehicles: &[VehicleRow],
        chargers: &[ChargerRow],
        defaults: TaxiDefaults,
    ) -> Result<Self, ServiceError> {
        let snapper = NodeSnapper::new(road);
        let far = |node: NodeId, at: LonLat| {
            ground_distance_metres(road.node_lonlat(node), at) > defaults.snap_m
        };
        let mut fleet = Fleet::default();
        let mut report = ServiceSetupReport {
            rows: u32::try_from(vehicles.len() + chargers.len()).expect("rows fit u32"),
            ..ServiceSetupReport::default()
        };
        let mut seen = HashSet::new();
        for v in vehicles {
            if !seen.insert(v.vehicle_id.as_str()) {
                return Err(ServiceError::BadRow(format!(
                    "vehicle `{}` appears twice",
                    v.vehicle_id
                )));
            }
            let soc = v.soc.unwrap_or(defaults.soc_start);
            if !(0.0..=1.0).contains(&soc) {
                return Err(ServiceError::BadRow(format!(
                    "vehicle `{}`: soc {soc} is not a share from 0 to 1",
                    v.vehicle_id
                )));
            }
            let node = snapper.nearest(road, v.position);
            if far(node, v.position) {
                report.off_layer += 1;
                continue;
            }
            fleet.vehicle_ids.push(v.vehicle_id.clone());
            fleet.vehicle_node.push(node);
            fleet.vehicle_soc.push(soc);
            report.kept += 1;
        }
        let mut seen = HashSet::new();
        for c in chargers {
            if !seen.insert(c.charger_id.as_str()) {
                return Err(ServiceError::BadRow(format!(
                    "charger `{}` appears twice",
                    c.charger_id
                )));
            }
            let kw = c.kw.unwrap_or(defaults.charger_kw);
            if !(kw.is_finite() && kw > 0.0) {
                return Err(ServiceError::BadRow(format!(
                    "charger `{}`: kw {kw} must be above 0",
                    c.charger_id
                )));
            }
            if c.plugs == 0 {
                report.empty += 1;
                continue;
            }
            let node = snapper.nearest(road, c.position);
            if far(node, c.position) {
                report.off_layer += 1;
                continue;
            }
            fleet.charger_ids.push(c.charger_id.clone());
            fleet.charger_hub_id.push(c.hub_id.clone());
            fleet.charger_node.push(node);
            fleet.charger_position.push(c.position);
            fleet.charger_plugs.push(c.plugs);
            fleet.charger_kw.push(kw);
            report.kept += 1;
        }
        if fleet.vehicle_ids.is_empty() {
            return Err(ServiceError::Empty(format!(
                "service `{name}`: no vehicle near the road network"
            )));
        }
        Ok(Self {
            name: name.to_string(),
            parts: ServiceParts::Taxi(fleet, defaults),
            report,
            resources: Vec::new(),
        })
    }

    /// A service from parts already snapped to the layers (a replay's, a test's).
    #[must_use]
    pub fn from_parts(name: &str, parts: ServiceParts) -> Self {
        Self {
            name: name.to_string(),
            parts,
            report: ServiceSetupReport::default(),
            resources: Vec::new(),
        }
    }

    /// Its name, unique in the run.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Its kind.
    #[must_use]
    pub fn kind(&self) -> ServiceKind {
        match self.parts {
            ServiceParts::BikeDocked(..) => ServiceKind::BikeDocked,
            ServiceParts::Taxi(..) => ServiceKind::Taxi,
        }
    }

    /// Its parts.
    #[must_use]
    pub fn parts(&self) -> &ServiceParts {
        &self.parts
    }

    /// What setting it up kept and left out.
    #[must_use]
    pub fn report(&self) -> ServiceSetupReport {
        self.report
    }

    /// Its option values, by name.
    #[must_use]
    pub fn option_values(&self) -> Vec<(&'static str, f64)> {
        match &self.parts {
            ServiceParts::BikeDocked(_, d) => d.values(),
            ServiceParts::Taxi(_, d) => d.values(),
        }
    }

    /// Its docks or chargers as resources for the run's hub set ([`RunHubs::build`]), in dock
    /// or charger order. A taxi's vehicles wait on the roads, so it brings no stand here.
    #[must_use]
    pub fn resource_rows(&self) -> Vec<ResourceRow> {
        let service = Some(self.name.clone());
        match &self.parts {
            ServiceParts::BikeDocked(docks, _) => (0..docks.external.len())
                .map(|d| ResourceRow {
                    resource: Resource {
                        external_id: docks.external[d].clone(),
                        kind: ResourceKind::Dock,
                        capacity: docks.capacity[d],
                        initial: docks.start[d],
                        kw: 0.0,
                        service: service.clone(),
                    },
                    hub_id: docks.hub_id[d].clone(),
                    position: docks.position[d],
                    access_points: vec![
                        AccessPoint { layer: Layer::Walk, node: docks.walk_node[d] },
                        AccessPoint { layer: Layer::Bike, node: docks.bike_node[d] },
                    ],
                })
                .collect(),
            ServiceParts::Taxi(fleet, _) => (0..fleet.charger_ids.len())
                .map(|c| ResourceRow {
                    resource: Resource {
                        external_id: fleet.charger_ids[c].clone(),
                        kind: ResourceKind::Charger,
                        capacity: fleet.charger_plugs[c],
                        initial: 0,
                        kw: fleet.charger_kw[c],
                        service: service.clone(),
                    },
                    hub_id: fleet.charger_hub_id[c].clone(),
                    position: fleet.charger_position[c],
                    access_points: vec![AccessPoint {
                        layer: Layer::Car,
                        node: fleet.charger_node[c],
                    }],
                })
                .collect(),
        }
    }

    /// Record its resources in the run's hub set: `hubs` was built with this service's
    /// [`Self::resource_rows`] starting at row `first`.
    pub fn link(&mut self, hubs: &RunHubs, first: usize) {
        let n = match &self.parts {
            ServiceParts::BikeDocked(docks, _) => docks.external.len(),
            ServiceParts::Taxi(fleet, _) => fleet.charger_ids.len(),
        };
        self.resources = (first..first + n).map(|i| hubs.row_resource(i)).collect();
    }

    /// A dock's or charger's resource in the run's hub set, once [`Self::link`] has run.
    #[must_use]
    pub fn resource(&self, index: u32) -> Option<ResourceId> {
        self.resources.get(index as usize).copied()
    }
}
