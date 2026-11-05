//! The one-loop assignment (`openmobisim-core-assignment`), exposed to Python.
//!
//! [`run_one_loop`] takes the scenario's parts by name, as `run_pipeline` does, builds the
//! assignment's inputs, runs it and writes its tables to the run folder:
//!
//! | File | What it holds |
//! |---|---|
//! | `trip_results.parquet` | one row per trip of the window: the columns of the other engine's table |
//! | `convergence.parquet` | one row per assignment iteration: the measures of its report |
//! | `chains.parquet` | one row per chain of every trip's choice set: its links (the network's ids, and their indices), whether it was taken |
//! | `link_bins.parquet` | per link and bin of exit time, the traffic and its time (with `link_bin_s`) |
//! | `link_bins_bike.parquet`, `link_bins_walk.parquet` | the same on the bike and walk layers, counting travellers (with `link_bin_s`, when a trip of the mode ran) |
//! | `timings.csv` | the wall-clock seconds of each stage |
//!
//! The manifest is written by the Python package, from what this function returns.
//!
//! **What it runs.** Car trips (a trip without a stated mode is a car trip), loaded at flow level
//! 0 (free flow) or 2 (the point queue), each car leg with its fixed times (`car_access_s` before
//! the drive, reaching the car; `car_egress_s` after it, parking and leaving it; whole seconds,
//! rounded, 0 when not given). **Bike and walking trips whose mode is stated**, each on its own
//! layer of the network (`Network.layer("bike")`, `"walk"`), on the route of least cost there
//! (a bike's by `bike_cost`: `"utility"` by default, `"time"` or `"dedicated"`; a walk's by its
//! time), at the layer's static speeds, a bike's ride with its fixed times (`bike_access_s`
//! before it, reaching and unlocking the bike; `bike_egress_s` after it, parking and locking it;
//! whole seconds, rounded, 0 when not given). They load nothing on the road network. A trip
//! stating another mode is refused. **Trips choosing their mode** among the car, the bike and
//! walking: with `modes` (the run's) and `class_modes` (a class's own, `None` for every mode of the
//! run), a trip that states no mode is offered the modes of both lists; with two or more it
//! chooses among them ([`openmobisim_core_assignment::modes`]): its choice set holds its car chains
//! beside its bike chain and its walking chain, each within the longest walk and ride offered
//! (`walk_max_s` and `bike_max_s`, in seconds on the layer, the fixed times not counted, the run's
//! unless `class_limits` gives a class its own; no limit when not given); with one it is a trip of
//! that mode. The scenario's other parts are checked by the Python package, which refuses by name
//! what this engine does not have before calling it.
//!
//! **A run continued from another** (`start_chains_path`, `start_iterations`): the state is
//! rebuilt from that run's chains table (every trip's set, current chain and probabilities) and
//! its count of iterations, its link travel times by loading its current chains once
//! (`openmobisim_core_assignment::restore`); refused when the table's demand and network
//! fingerprint is not this run's.
//!
//! **The user classes' coefficients.** The scenario states a utility per minute and per unit
//! of each attribute (`beta_time_min` and the like, negative for what travellers dislike); the
//! assignment weighs each attribute by its cost per unit, the opposite sign. Per class, the
//! run's options are overridden by the class's own, by name, and become:
//!
//! - the cost of a minute in the car: `−beta_time_min` (−0.2 for the logit and the nested logit
//!   when not given, their default; 1 for the deterministic choice, whose choice does not depend
//!   on the scale);
//! - of a minute walking, waiting or parking: `−(beta_time_min + beta_walk_min)`,
//!   `−(beta_time_min + beta_wait_min)` and `−(beta_time_min + beta_parking_min)` (each counts
//!   on top of the time; `beta_parking_min` 0 when not given);
//! - of a transfer: `−beta_transfers`; of a unit of money: `−beta_cost_eur`, **0 when no money
//!   coefficient is given** (a run without one is a time-based run); of a kilometre:
//!   `−beta_length_km`, 0 when not given;
//! - of a kilometre ridden by bike in mixed traffic, on top of the ride's time:
//!   `−beta_bike_mixed_km` (`beta_bike_mixed_km` −0.16 for the logit when not given, as the other
//!   engine's logit; 0 for the deterministic choice); of one on a painted lane and on a track or
//!   path of its own: `−beta_bike_lane_km` and `−beta_bike_separated_km` (0 when not given). A
//!   bike's ride counts as time in a vehicle, `−beta_time_min` a minute; a walk's minutes as
//!   walking;
//! - the constant of each mode, what a chain of it costs on top of its attributes:
//!   `−beta_mode_car`, `−beta_mode_bike`, `−beta_mode_walk` (0 when not given: time decides).
//!
//! `beta_ln_path_size` (1 by default) weighs the path-size term of the logit and the nested
//! logit; `mu` (0.5 by default; above 0 and at most 1) is the nested logit's nest scale, one
//! nest per mode. Both are the run's (`choice_options`), never a class's. A car or bike
//! leg's access time counts as walking and its egress time as parking, as the other engine's
//! mode choice counts them (`walk_min` and `parking_min`); it has no waiting or transfer.

use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use arrow_array::builder::{ListBuilder, StringBuilder};
use arrow_array::{
    Array, ArrayRef, BooleanArray, Float64Array, ListArray, RecordBatch, StringArray, UInt32Array,
    types::UInt32Type,
};
use arrow_schema::{DataType, Field, Schema};
use numpy::IntoPyArray;
use parquet::arrow::ArrowWriter;
use parquet::basic::Compression;
use parquet::file::metadata::KeyValue;
use parquet::file::properties::WriterProperties;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyDict;

use openmobisim_core_assignment::{
    Allocation, ClassModes, Coefficients, FixedTimes, FreeFlow, HighestUtility, Inputs,
    IterationReport, Layer, Layers, LeftOut, Loaded, Loader, ModeSet, Params, PerMode, Place,
    PointQueue, SavedTrip, StageHook, State, TripModes, assign, left_out, restore,
};
use openmobisim_core_choice::{ChoiceModel, Logit, NestedLogit, Options};
use openmobisim_core_demand::{
    ClassDefaults, Mode, Ownership, RawTrip, Travellers, Trips, build_travellers,
    check_distinct_keys, read_persons_parquet, read_trips_parquet,
};
use openmobisim_core_graph::layers::{BikeCost, StaticLayer, StaticNetwork};
use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_loading::{
    FidelityLevel, LinkBins, LinkTimes, Vehicle, load_timed_binned, run_ltm_binned,
};
use openmobisim_core_routes::{Route, RouteKey, RouteSets};
use openmobisim_core_sim::{NO_ROUTE, RouteChoices};
use openmobisim_core_types::diagnostics::Diagnostics;
use openmobisim_core_types::hash::{Fnv1a, fingerprint_hex};
use openmobisim_core_types::ids::{EntityId, LinkId, NodeId, TravellerId, TripId, VehicleId};
use openmobisim_core_types::rng::RngKey;
use openmobisim_core_types::time::Second;
use openmobisim_core_types::units::{Duration, Pcu};
use openmobisim_core_types::{CODE_VERSION, VERSION};

use crate::choice::PyRouteChoices;
use crate::network::PyNetwork;
use crate::pipeline::{
    PersonRow, PyLinkBins, TripRow, Window, person_from_row, to_value_error, trip_from_row,
    write_timings,
};
use crate::routes::PyRouteSets;

/// The names of the coefficients a class's options may give, besides `beta_ln_path_size` and
/// `mu`.
const COEFFICIENTS: [&str; 13] = [
    "beta_time_min",
    "beta_walk_min",
    "beta_wait_min",
    "beta_parking_min",
    "beta_transfers",
    "beta_cost_eur",
    "beta_length_km",
    // A bike leg's weight on its kilometres by facility: no car route has any.
    "beta_bike_mixed_km",
    "beta_bike_lane_km",
    "beta_bike_separated_km",
    // The modes' constants.
    "beta_mode_car",
    "beta_mode_bike",
    "beta_mode_walk",
];

/// The options of the run's choice model that are not a class's coefficient.
const MODEL_OPTIONS: [&str; 2] = ["beta_ln_path_size", "mu"];

/// What the fingerprint of a run of this engine covers, in words, for the manifest.
const FINGERPRINT_COVERS: [&str; 16] = [
    "the network: every node's position and signal, every link's ends, class, lanes, length, \
     free-flow time, storage, capacity, jam density, wave speed, control delay and closure",
    "the trips run (those departing inside the window): traveller, place in the day, user class, \
     weight, origin, destination, departure second, stated mode",
    "each traveller's car ownership",
    "each user class's coefficients",
    "the choice model and its path-size weight",
    "the loop's parameters: minimum and maximum iterations, the switch and found tolerances, the \
     indifference band, the averaging offset, the allocation, the cost source, the snapping \
     distance to motorways",
    "the flow level",
    "the loading step",
    "the width of the bins the link travel times are read in",
    "the window",
    "the master seed",
    "the engine and the code version",
    "the fixed times of reaching and leaving the car, and each class's cost of a minute parking, \
     when either time is not 0",
    "the run continued from, if any: its fingerprint and its count of iterations",
    "when a trip states the bike or walking: the bike and walk layers (every node's position, \
     every link's ends, length, speed and bike infrastructure), each traveller's bike ownership, \
     the bike cost, the bike's fixed times and each class's cost of a kilometre ridden in mixed \
     traffic (and on a painted lane and a track of its own, when a class weighs them)",
    "when a trip chooses its mode: the modes each user class is offered, its longest walk and \
     ride, each class's modes' constants and costs of a kilometre by bike facility, and the \
     nested logit's nest scale",
];

/// Why a class's options cannot be read, naming the class.
fn class_error(class: &str, message: &str) -> PyErr {
    PyValueError::new_err(format!("user class {class:?}: {message}"))
}

/// A class's coefficients from the run's options overridden by the class's, as the module docs
/// say; `model` is `"deterministic"`, `"logit"` or `"nested_logit"`.
fn coefficients(
    model: &str,
    run: &BTreeMap<String, f64>,
    class: &BTreeMap<String, f64>,
    name: &str,
) -> PyResult<Coefficients> {
    let mut merged = run.clone();
    for (k, v) in class {
        merged.insert(k.clone(), *v);
    }
    for (k, v) in &merged {
        if MODEL_OPTIONS.contains(&k.as_str()) {
            continue;
        }
        if !COEFFICIENTS.contains(&k.as_str()) {
            return Err(class_error(
                name,
                &format!(
                    "the one-loop engine has no coefficient {k:?}; it reads {}, and the model's \
                     beta_ln_path_size and mu",
                    COEFFICIENTS.join(", ")
                ),
            ));
        }
        if !v.is_finite() {
            return Err(class_error(name, &format!("{k} must be a finite number, got {v}")));
        }
    }
    // The logit's defaults for both logits; the deterministic choice's own.
    let logit = model != "deterministic";
    let given = |k: &str, default: f64| merged.get(k).copied().unwrap_or(default);
    let time = given("beta_time_min", if logit { -0.2 } else { -1.0 });
    let walk = given("beta_walk_min", if logit { -0.13 } else { 0.0 });
    let wait = given("beta_wait_min", if logit { -0.09 } else { 0.0 });
    let parking = given("beta_parking_min", 0.0);
    Ok(Coefficients {
        in_vehicle_per_minute: -time,
        walk_per_minute: -(time + walk),
        wait_per_minute: -(time + wait),
        parking_per_minute: -(time + parking),
        per_transfer: -given("beta_transfers", if logit { -1.0 } else { 0.0 }),
        per_money: -given("beta_cost_eur", 0.0),
        per_kilometre: -given("beta_length_km", 0.0),
        bike_mixed_per_kilometre: -given("beta_bike_mixed_km", if logit { -0.16 } else { 0.0 }),
        bike_lane_per_kilometre: -given("beta_bike_lane_km", 0.0),
        bike_separated_per_kilometre: -given("beta_bike_separated_km", 0.0),
        car_constant: -given("beta_mode_car", 0.0),
        bike_constant: -given("beta_mode_bike", 0.0),
        walk_constant: -given("beta_mode_walk", 0.0),
    })
}

/// The run's choice model: the deterministic choice, the logit with its path-size weight, or
/// the nested logit with its path-size weight and its nest scale `mu` (one nest per mode).
fn model_of(name: &str, options: &BTreeMap<String, f64>) -> PyResult<Box<dyn ChoiceModel>> {
    let mut o: Options = [("beta_utility".to_string(), 1.0)].into_iter().collect();
    if let Some(&v) = options.get("beta_ln_path_size") {
        o.insert("beta_ln_path_size".to_string(), v);
    }
    if name != "nested_logit" && options.contains_key("mu") {
        return Err(PyValueError::new_err(format!(
            "mu is the nested logit's nest scale; the choice model is {name:?}"
        )));
    }
    match name {
        "deterministic" => Ok(Box::new(HighestUtility)),
        "logit" => Ok(Box::new(Logit::from_options(&o).map_err(to_value_error)?)),
        "nested_logit" => {
            o.insert("mu".to_string(), options.get("mu").copied().unwrap_or(0.5));
            Ok(Box::new(NestedLogit::from_options(&o).map_err(to_value_error)?))
        }
        other => Err(PyValueError::new_err(format!(
            "the one-loop engine's choice models are \"deterministic\", \"logit\" and \
             \"nested_logit\", not {other:?}"
        ))),
    }
}

/// Keeps what the last loading was given and returned: the per-trip results are read from it.
#[derive(Default)]
struct LastLoading {
    /// The vehicles loaded, in the order the loop made them.
    vehicles: Vec<Vehicle>,
    /// Each vehicle's trip.
    trip_of: Vec<TripId>,
    /// Each vehicle's leg's fixed times.
    fixed: Vec<FixedTimes>,
    /// Each vehicle's arrival, `None` if unfinished.
    arrival: Vec<Option<f64>>,
}

impl StageHook for LastLoading {
    fn before_load(&mut self, _iteration: u32, state: &State, vehicles: &[Vehicle]) {
        // The loop makes one vehicle per car leg that uses a link, trip after trip.
        self.trip_of.clear();
        self.fixed.clear();
        for (t, entry) in state.trips().iter().enumerate() {
            if let Some(chain) = entry.current_chain() {
                for leg in chain.legs().iter().filter(|l| l.is_loaded()) {
                    self.trip_of.push(TripId::from_index(t));
                    self.fixed.push(leg.fixed);
                }
            }
        }
        self.vehicles = vehicles.to_vec();
    }

    fn after_load(&mut self, _iteration: u32, loaded: &Loaded) {
        self.arrival.clone_from(&loaded.arrival);
    }
}

/// The network (every node and link, by index) and the trips run (with each traveller's car
/// ownership), into `h`: the part of the run's fingerprint that the demand and network
/// fingerprint ([`demand_network_fingerprint`]) is made of.
fn hash_demand_network(
    h: &mut Fnv1a,
    network: &RoadNetwork,
    travellers: &Travellers,
    trips: &Trips,
) {
    h.write_bytes(network.defaults().descriptor().as_bytes());
    for raw in 0..network.node_count() {
        let node = NodeId::new(raw);
        let at = network.node_lonlat(node);
        h.write_f64(at.lon);
        h.write_f64(at.lat);
        h.write_bool(network.is_signalised(node));
    }
    for raw in 0..network.link_count() {
        let link = LinkId::new(raw);
        h.write_bool(network.is_link_signalised(link));
        h.write_u32(network.link_from(link).raw());
        h.write_u32(network.link_to(link).raw());
        h.write_u8(network.link_class(link) as u8);
        h.write_u8(network.link_lanes(link));
        h.write_f64(network.link_length(link).get());
        h.write_f64(network.free_flow_time(link).get());
        h.write_f64(network.storage(link).get());
        let p = network.link_parameters(link);
        h.write_f64(p.capacity.get());
        h.write_f64(p.jam_density.get());
        h.write_f64(p.wave_speed.get());
        h.write_f64(p.control_delay.get());
        h.write_bool(network.is_closed(link));
    }
    let ids = travellers.external_ids();
    let class_ids = travellers.class_external_ids();
    for raw in 0..trips.len() {
        let trip = TripId::new(raw);
        let traveller = trips.traveller(trip);
        h.write_str(ids.external(traveller.raw()));
        h.write_u32(trip.raw() - travellers.first_trip(traveller).raw());
        h.write_str(class_ids.external(travellers.user_class(traveller).raw()));
        h.write_u32(travellers.weight(traveller));
        h.write_bool(travellers.ownership(traveller).car);
        for p in [trips.origin(trip), trips.destination(trip)] {
            h.write_f64(p.lon);
            h.write_f64(p.lat);
        }
        h.write_u32(trips.departure(trip).get());
        h.write_bool(trips.mode_given(trip));
        h.write_u8(trips.mode(trip) as u8);
    }
}

/// The bike and walk layers a run uses (each only when a trip states its mode), and how bikes
/// choose their route.
struct StaticParts {
    bike: Option<Arc<StaticNetwork>>,
    walk: Option<Arc<StaticNetwork>>,
    bike_cost: BikeCost,
}

impl StaticParts {
    /// Whether the run has a static layer (a trip states the bike or walking).
    fn any(&self) -> bool {
        self.bike.is_some() || self.walk.is_some()
    }

    /// The layer of `mode`, if the run has it.
    fn of(&self, mode: Mode) -> Option<&StaticNetwork> {
        match mode {
            Mode::Bike => self.bike.as_deref(),
            Mode::Walk => self.walk.as_deref(),
            _ => None,
        }
    }
}

/// The static layers (every node's position, every link's ends, length, speed and bike
/// infrastructure, by index) and each traveller's bike ownership, into `h`; nothing when the run
/// has no static layer, so a run of car trips keeps its fingerprints.
fn hash_static(h: &mut Fnv1a, parts: &StaticParts, travellers: &Travellers) {
    if !parts.any() {
        return;
    }
    h.write_str("static layers");
    for layer in [&parts.bike, &parts.walk] {
        let Some(layer) = layer else {
            h.write_bool(false);
            continue;
        };
        h.write_bool(true);
        let graph = layer.network();
        h.write_u32(graph.node_count());
        for raw in 0..graph.node_count() {
            let at = graph.node_lonlat(NodeId::new(raw));
            h.write_f64(at.lon);
            h.write_f64(at.lat);
        }
        h.write_u32(graph.link_count());
        for raw in 0..graph.link_count() {
            let link = LinkId::new(raw);
            h.write_u32(graph.link_from(link).raw());
            h.write_u32(graph.link_to(link).raw());
            h.write_f64(graph.link_length(link).get());
            h.write_f64(layer.speed(link));
            h.write_u8(layer.infrastructure(link) as u8);
            h.write_u8(graph.link_class(link) as u8);
        }
    }
    for raw in 0..travellers.len() {
        h.write_bool(travellers.ownership(TravellerId::new(raw)).bike);
    }
}

/// A hash of the network and the trips run alone ([`hash_demand_network`]), with the static
/// layers when the run has them ([`hash_static`]): what a run continued from another must share
/// with it.
fn demand_network_fingerprint(
    network: &RoadNetwork,
    travellers: &Travellers,
    trips: &Trips,
    parts: &StaticParts,
) -> u64 {
    let mut h = Fnv1a::new();
    h.write_str("one-loop demand and network");
    hash_demand_network(&mut h, network, travellers, trips);
    hash_static(&mut h, parts, travellers);
    h.finish()
}

/// A hash of what decides the run's results ([`FINGERPRINT_COVERS`]); `start` is the run
/// continued from, by its fingerprint and count of iterations.
#[allow(clippy::too_many_arguments, reason = "every input of the fingerprint, by name")]
fn fingerprint(
    network: &RoadNetwork,
    travellers: &Travellers,
    trips: &Trips,
    classes: &[Coefficients],
    model: &str,
    path_size: f64,
    params: &Params,
    loading: (u32, u32, u32),
    window: (u32, u32),
    seed: u64,
    start: Option<(&str, u32)>,
    parts: &StaticParts,
    choice: &ModeChoice,
) -> u64 {
    let mut h = Fnv1a::new();
    h.write_str("one-loop");
    h.write_str(VERSION);
    h.write_u32(CODE_VERSION);
    hash_demand_network(&mut h, network, travellers, trips);
    for k in classes {
        for v in [
            k.in_vehicle_per_minute,
            k.walk_per_minute,
            k.wait_per_minute,
            k.per_transfer,
            k.per_money,
            k.per_kilometre,
        ] {
            h.write_f64(v);
        }
    }
    h.write_str(model);
    h.write_f64(path_size);
    h.write_u32(params.min_iterations);
    h.write_u32(params.max_iterations);
    h.write_f64(params.switch_tolerance);
    h.write_f64(params.found_tolerance);
    h.write_f64(params.indifference_band);
    h.write_u32(params.step_offset);
    h.write_str(&format!("{:?}", params.cost_source));
    h.write_f64(params.motorway_snap_metres);
    h.write_u32(loading.0);
    h.write_u32(loading.1);
    h.write_u32(loading.2);
    h.write_u32(window.0);
    h.write_u32(window.1);
    h.write_u64(seed);
    // The allocation, after everything else and only when it is not the default, so a run with
    // the draws keeps the fingerprint it had before the option existed.
    if params.allocation != Allocation::Draws {
        h.write_str(params.allocation.name());
    }
    // Likewise the fixed times, only when either is not 0 (the parking time's cost matters
    // only then), and the run continued from, only when there is one.
    let fixed = params.car_fixed_times;
    if fixed != FixedTimes::NONE {
        h.write_str("car_fixed_times");
        h.write_f64(fixed.access_seconds);
        h.write_f64(fixed.egress_seconds);
        for k in classes {
            h.write_f64(k.parking_per_minute);
        }
    }
    if let Some((print, iterations)) = start {
        h.write_str("start_from");
        h.write_str(print);
        h.write_u32(iterations);
    }
    // The bike and walk layers, only when a trip may ride or walk.
    if parts.any() {
        hash_static(&mut h, parts, travellers);
        h.write_str(parts.bike_cost.as_str());
        h.write_f64(params.bike_fixed_times.access_seconds);
        h.write_f64(params.bike_fixed_times.egress_seconds);
        for k in classes {
            h.write_f64(k.bike_mixed_per_kilometre);
            h.write_f64(k.parking_per_minute);
        }
    }
    // Mode choice, only when a trip chooses its mode; else the coefficients it adds, only when a
    // class gives one (so a run without either keeps its fingerprint).
    let added = |k: &Coefficients| {
        [
            k.car_constant,
            k.bike_constant,
            k.walk_constant,
            k.bike_lane_per_kilometre,
            k.bike_separated_per_kilometre,
        ]
    };
    if choice.any {
        h.write_str("mode choice");
        for c in &choice.classes {
            for mode in [Mode::Car, Mode::Bike, Mode::Walk] {
                h.write_bool(c.modes.contains(mode));
            }
            h.write_f64(c.walk_max_seconds);
            h.write_f64(c.bike_max_seconds);
        }
        for k in classes {
            for v in added(k) {
                h.write_f64(v);
            }
        }
        h.write_f64(choice.mu.unwrap_or(f64::NAN));
    } else if classes.iter().any(|k| added(k).iter().any(|v| *v != 0.0)) {
        h.write_str("modes' constants and bike facilities");
        for k in classes {
            for v in added(k) {
                h.write_f64(v);
            }
        }
    }
    h.finish()
}

/// What mode choice offers each user class (by class index), whether any trip chooses its mode,
/// and the nested logit's nest scale (`None` for another model).
struct ModeChoice {
    classes: Vec<ClassModes>,
    any: bool,
    mu: Option<f64>,
}

/// What mode choice offers each class of `travellers`: the modes of `modes` (the run's) that the
/// class's own list (`class_modes`) also holds, each limited to the car, the bike and walking,
/// with the longest walk and ride (`walk_max_s`, `bike_max_s`, or the class's own in
/// `class_limits`; no limit when neither gives one). Empty when the run offers no modes.
fn mode_choice_of(
    travellers: &Travellers,
    modes: Option<&[String]>,
    class_modes: &HashMap<String, Vec<String>>,
    limits: (Option<f64>, Option<f64>),
    class_limits: &HashMap<String, HashMap<String, f64>>,
) -> PyResult<Vec<ClassModes>> {
    let parse = |names: &[String], whose: &str| -> PyResult<Vec<Mode>> {
        names
            .iter()
            .map(|n| {
                let mode = Mode::from_name(n).map_err(to_value_error)?;
                if matches!(mode, Mode::Car | Mode::Bike | Mode::Walk) {
                    Ok(mode)
                } else {
                    Err(PyValueError::new_err(format!(
                        "the one-loop engine's mode choice is among the car, the bike and walking; \
                         {whose} offers {n:?}"
                    )))
                }
            })
            .collect()
    };
    let Some(modes) = modes else { return Ok(Vec::new()) };
    let run = parse(modes, "the run")?;
    let limit = |name: &str, v: Option<f64>| -> PyResult<f64> {
        let v = v.unwrap_or(f64::INFINITY);
        if v.is_nan() || v <= 0.0 {
            return Err(PyValueError::new_err(format!(
                "{name} must be a positive number of seconds (inf: no limit), got {v}"
            )));
        }
        Ok(v)
    };
    let (walk, bike) = (limit("walk_max_s", limits.0)?, limit("bike_max_s", limits.1)?);
    let ids = travellers.class_external_ids();
    let mut out = Vec::with_capacity(ids.count() as usize);
    for c in 0..ids.count() {
        let name = ids.external(c);
        let own = match class_modes.get(name) {
            Some(list) => parse(list, &format!("user class {name:?}"))?,
            None => run.clone(),
        };
        let offered: Vec<Mode> = run.iter().copied().filter(|m| own.contains(m)).collect();
        let mut class = ClassModes {
            modes: ModeSet::of(&offered),
            walk_max_seconds: walk,
            bike_max_seconds: bike,
        };
        if let Some(l) = class_limits.get(name) {
            for (k, v) in l {
                match k.as_str() {
                    "walk_max_s" => class.walk_max_seconds = limit(k, Some(*v))?,
                    "bike_max_s" => class.bike_max_seconds = limit(k, Some(*v))?,
                    other => {
                        return Err(class_error(
                            name,
                            &format!(
                                "the one-loop engine's class limits are walk_max_s and \
                                 bike_max_s, not {other:?}"
                            ),
                        ));
                    }
                }
            }
        }
        out.push(class);
    }
    Ok(out)
}

/// The per-trip results of the last loading.
struct TripRows {
    traveller_id: Vec<String>,
    trip_seq: Vec<u32>,
    user_class: Vec<String>,
    weight: Vec<u32>,
    origin: Vec<(f64, f64)>,
    destination: Vec<(f64, f64)>,
    departure_s: Vec<u32>,
    /// The mode of the chain taken; a trip choosing its mode that took none has none.
    mode: Vec<Option<&'static str>>,
    /// Whether the trip chose its mode.
    mode_choice: Vec<bool>,
    outcome: Vec<&'static str>,
    arrival_s: Vec<Option<f64>>,
    /// A completed trip's seconds in its own vehicle (the car setting off to arriving, the
    /// bike's ride; 0 for a walk).
    vehicle_s: Vec<Option<f64>>,
    /// A completed trip's fixed seconds around its vehicle (reaching it, parking and leaving it;
    /// 0 for a walk).
    access_egress_s: Vec<Option<f64>>,
}

impl TripRows {
    fn travel_time_s(&self) -> Vec<Option<f64>> {
        self.arrival_s
            .iter()
            .zip(&self.departure_s)
            .map(|(a, &d)| a.map(|a| a - f64::from(d)))
            .collect()
    }
}

/// One trip's last leg's end (its vehicle's arrival plus the egress time), and its seconds in
/// its vehicles and around them, summed over its loaded legs.
#[derive(Clone, Copy)]
struct Ended {
    at: f64,
    vehicle_s: f64,
    fixed_s: f64,
}

/// The mode of `trip`'s chains when it has one mode (stated, or the one its class is offered, or
/// the car); `None` for a trip choosing its mode.
fn trip_mode(modes: &[TripModes], trip: TripId) -> Option<Mode> {
    match modes[trip.index()] {
        TripModes::Stated(m) => Some(m),
        TripModes::Choice(_) => None,
    }
}

/// Each trip's outcome and arrival from the last loading and the final state: a trip whose
/// last leg ends after `end_s` (its vehicle's arrival plus the egress time) is truncated. A bike
/// or walking chain, which loads nothing, ends at the trip's departure plus its time (the bike's
/// access, ride and egress; the walk), truncated likewise. A trip choosing its mode that has no
/// chain had no alternative (`no_alternative`), and no mode.
fn trip_rows(
    travellers: &Travellers,
    trips: &Trips,
    modes: &[TripModes],
    state: &State,
    last: &LastLoading,
    end_s: f64,
) -> PyResult<TripRows> {
    let n = trips.len() as usize;
    // Per trip: `None` if none of its legs was loaded; `Some(None)` if a vehicle had not
    // arrived or its leg ended after the window; `Some(Some(e))` with its last leg's end.
    let mut loaded: Vec<Option<Option<Ended>>> = vec![None; n];
    if last.trip_of.len() != last.arrival.len() || last.fixed.len() != last.arrival.len() {
        return Err(PyValueError::new_err(format!(
            "the last loading returned {} arrivals for {} vehicles",
            last.arrival.len(),
            last.trip_of.len()
        )));
    }
    for (v, (trip, &at)) in last.trip_of.iter().zip(&last.arrival).enumerate() {
        let fixed = last.fixed[v];
        let set_off = f64::from(last.vehicles[v].departure.get());
        let leg = at.map(|a| Ended {
            at: a + fixed.egress_seconds,
            vehicle_s: a - set_off,
            fixed_s: fixed.total_seconds(),
        });
        let leg = leg.filter(|e| e.at <= end_s);
        let slot = &mut loaded[trip.index()];
        *slot = Some(match (*slot, leg) {
            (Some(None), _) | (_, None) => None,
            (Some(Some(a)), Some(b)) => Some(Ended {
                at: a.at.max(b.at),
                vehicle_s: a.vehicle_s + b.vehicle_s,
                fixed_s: a.fixed_s + b.fixed_s,
            }),
            (None, Some(b)) => Some(b),
        });
    }
    let ids = travellers.external_ids();
    let class_ids = travellers.class_external_ids();
    let mut rows = TripRows {
        traveller_id: Vec::with_capacity(n),
        trip_seq: Vec::with_capacity(n),
        user_class: Vec::with_capacity(n),
        weight: Vec::with_capacity(n),
        origin: Vec::with_capacity(n),
        destination: Vec::with_capacity(n),
        departure_s: Vec::with_capacity(n),
        mode: Vec::with_capacity(n),
        mode_choice: Vec::with_capacity(n),
        outcome: Vec::with_capacity(n),
        arrival_s: Vec::with_capacity(n),
        vehicle_s: Vec::with_capacity(n),
        access_egress_s: Vec::with_capacity(n),
    };
    for (t, slot) in loaded.iter().enumerate() {
        let trip = TripId::from_index(t);
        let traveller = trips.traveller(trip);
        rows.traveller_id.push(ids.external(traveller.raw()).to_string());
        rows.trip_seq.push(trip.raw() - travellers.first_trip(traveller).raw());
        rows.user_class
            .push(class_ids.external(travellers.user_class(traveller).raw()).to_string());
        rows.weight.push(travellers.weight(traveller));
        let (o, d) = (trips.origin(trip), trips.destination(trip));
        rows.origin.push((o.lon, o.lat));
        rows.destination.push((d.lon, d.lat));
        let departure = trips.departure(trip).get();
        rows.departure_s.push(departure);
        let entry = state.trip(trip);
        let stated = trip_mode(modes, trip);
        let mode = entry.current_chain().map(openmobisim_core_assignment::Chain::mode).or(stated);
        rows.mode.push(mode.map(Mode::as_str));
        rows.mode_choice.push(stated.is_none());
        let owned = match stated {
            Some(Mode::Car) => state.has_car(traveller),
            Some(Mode::Bike) => state.has_bike(traveller),
            _ => true,
        };
        // A chain that loads nothing but uses links (a bike or walking trip): timed on its layer.
        let statics = entry.current_chain().filter(|c| {
            c.legs().iter().all(|l| !l.is_loaded()) && c.legs().iter().any(|l| !l.links.is_empty())
        });
        let slot = &match statics {
            Some(chain) => {
                let start = f64::from(departure);
                let e = Ended {
                    at: start + chain.total().total_seconds(),
                    vehicle_s: chain.legs().iter().map(|l| l.attributes.in_vehicle_seconds).sum(),
                    fixed_s: chain.legs().iter().map(|l| l.fixed.total_seconds()).sum(),
                };
                Some((e.at <= end_s).then_some(e))
            }
            None => *slot,
        };
        let (outcome, ended) = match (entry.current, slot) {
            (None, _) if stated.is_none() => ("no_alternative", None),
            (None, _) if !owned => ("no_vehicle_available", None),
            (None, _) => ("no_feasible_path", None),
            // A chain whose ends snapped to one node uses no link and leaves no car: it arrives
            // as it sets out.
            (Some(_), None) => (
                "completed",
                Some(Ended { at: f64::from(departure), vehicle_s: 0.0, fixed_s: 0.0 }),
            ),
            (Some(_), Some(Some(e))) => ("completed", Some(*e)),
            (Some(_), Some(None)) => ("truncated", None),
        };
        rows.outcome.push(outcome);
        rows.arrival_s.push(ended.map(|e| e.at));
        rows.vehicle_s.push(ended.map(|e| e.vehicle_s));
        rows.access_egress_s.push(ended.map(|e| e.fixed_s));
    }
    Ok(rows)
}

/// The link bins of the last loading in bins of `bin_seconds`, by exit time: at level 2 the
/// loading replayed (it is deterministic: the replay's arrivals are checked against the
/// loop's), at level 0 each vehicle at free flow, link after link.
fn link_bins(
    network: &RoadNetwork,
    turns: &TurnTable,
    last: &LastLoading,
    level: u32,
    end_s: f64,
    step_s: f64,
    bin_seconds: u32,
) -> PyResult<LinkBins> {
    if level == 2 {
        let (trajectories, bins) = run_ltm_binned(
            network,
            turns,
            &last.vehicles,
            Duration(end_s),
            Duration(step_s),
            FidelityLevel::PointQueue,
            bin_seconds,
        );
        let position: HashMap<_, usize> =
            last.vehicles.iter().enumerate().map(|(i, v)| (v.id, i)).collect();
        let mut replayed = vec![None; last.vehicles.len()];
        for t in &trajectories {
            if let Some(&i) = position.get(&t.vehicle) {
                replayed[i] = Some(f64::from(t.arrival().get()));
            }
        }
        if replayed != last.arrival {
            return Err(PyValueError::new_err(
                "the loading replayed for the link bins gave other arrivals than the run's last \
                 loading: a loading that is not deterministic, please report it",
            ));
        }
        return Ok(bins);
    }
    let times = LinkTimes::free_flow(network);
    let mut cells: BTreeMap<(u32, u32), (u32, f64, f64)> = BTreeMap::new();
    for v in &last.vehicles {
        let mut t = f64::from(v.departure.get());
        for link in &v.route {
            let seconds = times.link_seconds(link.raw(), t);
            t += seconds;
            if t > end_s {
                break;
            }
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "a second of the day over a bin length, within u32"
            )]
            let bin = (t / f64::from(bin_seconds)).floor() as u32;
            let cell = cells.entry((bin, link.raw())).or_insert((0, 0.0, 0.0));
            cell.0 += 1;
            cell.1 += v.pcu.get();
            cell.2 += v.pcu.get() * seconds;
        }
    }
    let rows = cells.into_iter().map(|((b, l), (c, p, s))| (b, l, c, p, s)).collect();
    Ok(LinkBins::from_rows(bin_seconds, rows))
}

/// One bike or walking leg of the final state as the select-link analysis reads it: its trip,
/// when it set off, and its links up to the one it was on when the window ended.
struct LayerLeg {
    trip: u32,
    start_s: u32,
    links: Vec<u32>,
}

/// The bike and walk layers' per-link results of the final state, in bins of `bin_seconds` by
/// exit time, as the other engine makes them (`load_timed_binned`: every traversal at the layer's
/// static time that finishes by `end_s`, its traveller's weight counted): from each trip's current
/// chain's bike (walking) legs that use a link, each setting off at its leg's departure plus its
/// access time. `None` for a layer no such leg used. With them, per layer, the legs binned, each
/// up to the link it was on when the window ended (as the other engine keeps them).
fn layer_bins(
    parts: &StaticParts,
    travellers: &Travellers,
    trips: &Trips,
    state: &State,
    end_s: f64,
    bin_seconds: u32,
) -> ([Option<LinkBins>; 2], [Vec<LayerLeg>; 2]) {
    let mut out = [None, None];
    let mut legs: [Vec<LayerLeg>; 2] = [Vec::new(), Vec::new()];
    for (slot, mode) in [Mode::Bike, Mode::Walk].into_iter().enumerate() {
        let Some(layer) = parts.of(mode) else { continue };
        let mut vehicles = Vec::new();
        for (t, entry) in state.trips().iter().enumerate() {
            let Some(chain) = entry.current_chain() else { continue };
            let trip = TripId::from_index(t);
            let weight = f64::from(travellers.weight(trips.traveller(trip)));
            for leg in chain.legs().iter().filter(|l| l.mode == mode && !l.links.is_empty()) {
                #[allow(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "a whole second of the day, non-negative, within the u32 clock"
                )]
                let second = Second(leg.vehicle_departure().max(0.0).floor() as u32);
                vehicles.push(Vehicle::new(
                    VehicleId::new(trip.raw()),
                    leg.links.clone(),
                    Pcu(weight),
                    second,
                ));
            }
        }
        if vehicles.is_empty() {
            continue;
        }
        let (trajectories, bins) = load_timed_binned(
            &vehicles,
            layer.network(),
            &layer.link_seconds(),
            end_s,
            bin_seconds,
        );
        out[slot] = Some(bins);
        legs[slot] = vehicles
            .into_iter()
            .zip(&trajectories)
            .map(|(v, t)| {
                let on = t.links.iter().position(|x| f64::from(x.exit.get()) >= end_s);
                let keep = on.map_or(v.route.len(), |i| i + 1);
                LayerLeg {
                    trip: v.id.raw(),
                    start_s: v.departure.get(),
                    links: v.route[..keep].iter().map(|l| l.raw()).collect(),
                }
            })
            .collect();
    }
    (out, legs)
}

/// The route sets and route choices of the final state: per pair of snapped trip ends, every
/// car chain any of its trips has in its choice set, in the order first found (trips in order);
/// per trip, the car chain it takes. Car chains only: a bike or walking chain has no road route
/// (its route is in the chains table, on its layer), and a trip choosing its mode that took one
/// takes no route here.
fn routes(
    network: &RoadNetwork,
    travellers: &Travellers,
    trips: &Trips,
    state: &State,
) -> (RouteSets, RouteChoices) {
    let mut by_key: BTreeMap<RouteKey, Vec<Vec<u32>>> = BTreeMap::new();
    // The trip's road pair, from its first car chain.
    let key_of = |t: usize| -> Option<RouteKey> {
        let chain = state.trips()[t].set.iter().find(|c| c.mode() == Mode::Car)?;
        let (first, last) = (chain.legs().first()?, chain.legs().last()?);
        let (Place::Road(o), Place::Road(d)) = (first.from, last.to) else { return None };
        Some(RouteKey::new(o, d))
    };
    for (t, entry) in state.trips().iter().enumerate() {
        let Some(key) = key_of(t) else { continue };
        let routes = by_key.entry(key).or_default();
        for chain in entry.set.iter().filter(|c| c.mode() == Mode::Car) {
            let links = chain.road_links();
            if !routes.contains(&links) {
                routes.push(links);
            }
        }
    }
    let free = |links: &[u32]| -> f64 {
        links.iter().map(|&l| network.free_flow_time(LinkId::new(l)).get()).sum()
    };
    let sets: Vec<(RouteKey, Vec<Route>)> = by_key
        .iter()
        .map(|(k, routes)| {
            let routes = routes
                .iter()
                .map(|links| Route {
                    links: links.iter().map(|&l| LinkId::new(l)).collect(),
                    cost: free(links),
                    overlap: 0.0,
                })
                .collect();
            (*k, routes)
        })
        .collect();
    let store = RouteSets::from_routes(network, "chain_search", "one-loop", sets);
    let n = trips.len() as usize;
    let mut choices = RouteChoices {
        route: vec![NO_ROUTE; n],
        alternatives: vec![0; n],
        probability: vec![f64::NAN; n],
        weight: vec![0; n],
    };
    for (t, entry) in state.trips().iter().enumerate() {
        let trip = TripId::from_index(t);
        choices.weight[t] = travellers.weight(trips.traveller(trip));
        let Some(key) = key_of(t) else { continue };
        let cars = entry.set.iter().filter(|c| c.mode() == Mode::Car).count();
        choices.alternatives[t] = u32::try_from(cars).expect("a set fits u32");
        // A trip on a bike or walking chain takes no road route.
        let Some(current) = entry.current.filter(|&c| entry.set[c].mode() == Mode::Car) else {
            continue;
        };
        let k = store.key_index(key).expect("every trip's pair is in the sets");
        let links = entry.set[current].road_links();
        let range = store.route_range(k);
        let r = range
            .clone()
            .find(|&r| store.route(r).links == links.as_slice())
            .expect("every chain of a set is a route of its pair");
        choices.route[t] = u32::try_from(r).expect("routes fit u32");
        choices.probability[t] = entry.probabilities.get(current).copied().unwrap_or(f64::NAN);
    }
    (store, choices)
}

// --- files ----------------------------------------------------------------------------------

/// Write one batch to `path` with `metadata`, Snappy-compressed.
fn write_batch(
    path: &Path,
    fields: Vec<Field>,
    columns: Vec<ArrayRef>,
    metadata: &[(String, String)],
) -> PyResult<()> {
    let schema = Arc::new(
        Schema::new(fields)
            .with_metadata(metadata.iter().cloned().collect::<HashMap<String, String>>()),
    );
    let batch = RecordBatch::try_new(schema.clone(), columns).map_err(to_value_error)?;
    let file = File::create(path).map_err(to_value_error)?;
    let key_values: Vec<KeyValue> =
        metadata.iter().map(|(k, v)| KeyValue::new(k.clone(), v.clone())).collect();
    let properties = WriterProperties::builder()
        .set_compression(Compression::SNAPPY)
        .set_key_value_metadata(Some(key_values))
        .build();
    let mut writer =
        ArrowWriter::try_new(file, schema, Some(properties)).map_err(to_value_error)?;
    writer.write(&batch).map_err(to_value_error)?;
    writer.close().map_err(to_value_error)?;
    Ok(())
}

fn floats(v: Vec<f64>) -> ArrayRef {
    Arc::new(Float64Array::from(v))
}

fn whole(v: Vec<u32>) -> ArrayRef {
    Arc::new(UInt32Array::from(v))
}

/// `trip_results.parquet`: the other engine's columns; the times as real seconds.
fn write_trip_results(
    path: &Path,
    run_id: &str,
    rows: &TripRows,
    metadata: &[(String, String)],
) -> PyResult<()> {
    let n = rows.traveller_id.len();
    let index = u32::try_from(n).expect("the trips of a run fit u32");
    let nulls = vec![None::<u32>; n];
    let travel = rows.travel_time_s();
    let columns: Vec<ArrayRef> = vec![
        Arc::new(StringArray::from(vec![run_id; n])),
        whole((0..index).collect()),
        Arc::new(StringArray::from(rows.traveller_id.clone())),
        whole(rows.trip_seq.clone()),
        Arc::new(StringArray::from(rows.user_class.clone())),
        whole(rows.weight.clone()),
        floats(rows.origin.iter().map(|p| p.0).collect()),
        floats(rows.origin.iter().map(|p| p.1).collect()),
        floats(rows.destination.iter().map(|p| p.0).collect()),
        floats(rows.destination.iter().map(|p| p.1).collect()),
        whole(rows.departure_s.clone()),
        Arc::new(StringArray::from(rows.mode.clone())),
        Arc::new(BooleanArray::from(rows.mode_choice.clone())),
        Arc::new(StringArray::from(rows.outcome.clone())),
        Arc::new(Float64Array::from(rows.arrival_s.clone())),
        Arc::new(Float64Array::from(travel)),
        Arc::new(UInt32Array::from(nulls)),
        Arc::new(Float64Array::from(rows.vehicle_s.clone())),
        Arc::new(Float64Array::from(rows.access_egress_s.clone())),
    ];
    let field = |name: &str, t: DataType, nullable: bool| Field::new(name, t, nullable);
    let fields = vec![
        field("run_id", DataType::Utf8, false),
        field("trip", DataType::UInt32, false),
        field("traveller_id", DataType::Utf8, false),
        field("trip_seq", DataType::UInt32, false),
        field("user_class", DataType::Utf8, false),
        field("weight", DataType::UInt32, false),
        field("origin_lon", DataType::Float64, false),
        field("origin_lat", DataType::Float64, false),
        field("destination_lon", DataType::Float64, false),
        field("destination_lat", DataType::Float64, false),
        field("departure_s", DataType::UInt32, false),
        field("mode", DataType::Utf8, true),
        field("mode_choice", DataType::Boolean, false),
        field("outcome", DataType::Utf8, false),
        field("arrival_s", DataType::Float64, true),
        field("travel_time_s", DataType::Float64, true),
        field("link_at_end", DataType::UInt32, true),
        field("vehicle_s", DataType::Float64, true),
        field("access_egress_s", DataType::Float64, true),
    ];
    write_batch(path, fields, columns, metadata)
}

/// The convergence table's columns that are real numbers (the iteration and the counts of
/// trips found and added are whole numbers, written apart). A share of an empty group (a mode no
/// trip held, or a run where no trip chooses its mode) is `NaN`.
fn convergence_columns(reports: &[IterationReport]) -> Vec<(&'static str, Vec<f64>)> {
    let col = |f: &dyn Fn(&IterationReport) -> f64| reports.iter().map(f).collect::<Vec<f64>>();
    let nan = |v: Option<f64>| v.unwrap_or(f64::NAN);
    let by_mode = |f: fn(&IterationReport) -> PerMode, m: Mode| {
        reports.iter().map(|r| nan(f(r).of(m))).collect::<Vec<f64>>()
    };
    let (car, bike, walk) = (Mode::Car, Mode::Bike, Mode::Walk);
    vec![
        ("found_share", col(&|r| r.found_share)),
        ("added_share", col(&|r| r.added_share)),
        ("switch_share", col(&|r| r.switch_share)),
        ("switch_floor", col(&|r| r.switch_floor)),
        ("switch_excess", col(&|r| r.switch_excess)),
        ("switch_above_floor", col(&|r| r.switch_above_floor)),
        ("switch_excess_expected", col(&|r| r.switch_excess_expected)),
        ("departed", col(&|r| r.departed)),
        ("arrived", col(&|r| r.arrived)),
        ("unfinished", col(&|r| r.unfinished)),
        ("unfinished_share", col(&|r| r.unfinished_share)),
        ("without_chain", col(&|r| r.without_chain)),
        ("gap", col(&|r| r.duality_gap)),
        ("gap_deterministic", col(&|r| r.gap_deterministic)),
        ("spread", col(&|r| r.on_cheapest_share)),
        ("mean_trip_s", col(&|r| r.mean_trip_seconds)),
        ("choosing", col(&|r| r.choosing)),
        ("found_share_car", by_mode(|r| r.found_share_by_mode, car)),
        ("found_share_bike", by_mode(|r| r.found_share_by_mode, bike)),
        ("found_share_walk", by_mode(|r| r.found_share_by_mode, walk)),
        ("switch_share_car", by_mode(|r| r.switch_share_by_mode, car)),
        ("switch_share_bike", by_mode(|r| r.switch_share_by_mode, bike)),
        ("switch_share_walk", by_mode(|r| r.switch_share_by_mode, walk)),
        ("mode_share_change_expected_car", by_mode(|r| r.mode_share_change_expected, car)),
        ("mode_share_change_expected_bike", by_mode(|r| r.mode_share_change_expected, bike)),
        ("mode_share_change_expected_walk", by_mode(|r| r.mode_share_change_expected, walk)),
        ("mode_change_share", col(&|r| nan(r.mode_change_share))),
        ("mode_share_car", by_mode(|r| r.mode_split, car)),
        ("mode_share_bike", by_mode(|r| r.mode_split, bike)),
        ("mode_share_walk", by_mode(|r| r.mode_split, walk)),
        ("search_s", col(&|r| r.stage_seconds.search)),
        ("choose_s", col(&|r| r.stage_seconds.choose)),
        ("load_s", col(&|r| r.stage_seconds.load)),
        ("measure_s", col(&|r| r.stage_seconds.measure)),
        ("assess_s", col(&|r| r.stage_seconds.assess)),
    ]
}

fn write_convergence(
    path: &Path,
    run_id: &str,
    reports: &[IterationReport],
    metadata: &[(String, String)],
) -> PyResult<()> {
    let n = reports.len();
    let mut fields = vec![
        Field::new("run_id", DataType::Utf8, false),
        Field::new("iteration", DataType::UInt32, false),
        Field::new("found", DataType::UInt32, false),
        Field::new("added", DataType::UInt32, false),
    ];
    let mut columns: Vec<ArrayRef> = vec![
        Arc::new(StringArray::from(vec![run_id; n])),
        whole(reports.iter().map(|r| r.iteration).collect()),
        whole(reports.iter().map(|r| r.found).collect()),
        whole(reports.iter().map(|r| r.added).collect()),
    ];
    for (name, values) in convergence_columns(reports) {
        fields.push(Field::new(name, DataType::Float64, false));
        columns.push(floats(values));
    }
    write_batch(path, fields, columns, metadata)
}

/// One row per chain of every trip's set: `trip`, `chain` (its place in the set), `taken`,
/// `probability` (the model's at the trip's last choice), `cost` (utility units) and
/// `travel_time_s` at the final link travel times (the fixed times included), `mode` (the mode of
/// its leg: `car`, `bike` or `walk`), and its links in travel order, on its leg's layer (the road
/// network for a car, the bike or walk layer), as that layer's ids (`links`) and as indices into
/// that layer's links (`link_index`).
#[allow(clippy::too_many_arguments, reason = "the state, the networks and the demand, by name")]
fn write_chains(
    path: &Path,
    state: &State,
    network: &RoadNetwork,
    parts: &StaticParts,
    travellers: &Travellers,
    trips: &Trips,
    classes: &[Coefficients],
    metadata: &[(String, String)],
) -> PyResult<()> {
    let (mut trip, mut chain, mut taken, mut probability, mut cost, mut seconds, mut links) =
        (vec![], vec![], vec![], vec![], vec![], vec![], vec![]);
    let mut modes: Vec<&'static str> = vec![];
    let mut named = ListBuilder::new(StringBuilder::new());
    for (t, entry) in state.trips().iter().enumerate() {
        let id = TripId::from_index(t);
        let k = classes[travellers.user_class(trips.traveller(id)).index()];
        for (c, ch) in entry.set.iter().enumerate() {
            trip.push(id.raw());
            chain.push(u32::try_from(c).expect("a set fits u32"));
            taken.push(entry.current == Some(c));
            probability.push(entry.probabilities.get(c).copied().unwrap_or(f64::NAN));
            cost.push(k.chain_cost(ch));
            seconds.push(ch.total().total_seconds());
            modes.push(ch.legs().first().map_or(Mode::Car, |l| l.mode).as_str());
            let mut index = Vec::new();
            for leg in ch.legs() {
                let layer = parts.of(leg.mode).map_or(network, StaticNetwork::network);
                let ids = layer.link_external_ids();
                for &l in &leg.links {
                    named.values().append_value(ids.external(l.raw()));
                    index.push(Some(l.raw()));
                }
            }
            named.append(true);
            links.push(Some(index));
        }
    }
    let index = ListArray::from_iter_primitive::<UInt32Type, _, _>(links);
    let named = named.finish();
    let fields = vec![
        Field::new("trip", DataType::UInt32, false),
        Field::new("chain", DataType::UInt32, false),
        Field::new("taken", DataType::Boolean, false),
        Field::new("probability", DataType::Float64, false),
        Field::new("cost", DataType::Float64, false),
        Field::new("travel_time_s", DataType::Float64, false),
        Field::new("links", named.data_type().clone(), true),
        Field::new("link_index", index.data_type().clone(), true),
        Field::new("mode", DataType::Utf8, false),
    ];
    let columns: Vec<ArrayRef> = vec![
        whole(trip),
        whole(chain),
        Arc::new(BooleanArray::from(taken)),
        floats(probability),
        floats(cost),
        floats(seconds),
        Arc::new(named),
        Arc::new(index),
        Arc::new(StringArray::from(modes)),
    ];
    write_batch(path, fields, columns, metadata)
}

/// `link_bins.parquet`: the other engine's columns.
fn write_link_bins(
    path: &Path,
    run_id: &str,
    bins: &LinkBins,
    network: &RoadNetwork,
    metadata: &[(String, String)],
) -> PyResult<()> {
    let n = bins.len();
    let ids = network.link_external_ids();
    let b = bins.bin_seconds();
    let fields = vec![
        Field::new("run_id", DataType::Utf8, false),
        Field::new("bin", DataType::UInt32, false),
        Field::new("start_s", DataType::UInt32, false),
        Field::new("link", DataType::Utf8, false),
        Field::new("link_index", DataType::UInt32, false),
        Field::new("crossings", DataType::UInt32, false),
        Field::new("pcu", DataType::Float64, false),
        Field::new("pcu_seconds", DataType::Float64, false),
    ];
    let columns: Vec<ArrayRef> = vec![
        Arc::new(StringArray::from(vec![run_id; n])),
        whole(bins.bins().to_vec()),
        whole(bins.bins().iter().map(|x| x * b).collect()),
        Arc::new(StringArray::from(
            bins.links().iter().map(|&l| ids.external(l)).collect::<Vec<_>>(),
        )),
        whole(bins.links().to_vec()),
        whole(bins.crossings().to_vec()),
        floats(bins.pcu().to_vec()),
        floats(bins.pcu_seconds().to_vec()),
    ];
    let mut metadata = metadata.to_vec();
    metadata.push(("openmobisim.bin_seconds".to_string(), b.to_string()));
    write_batch(path, fields, columns, &metadata)
}

/// `link_bins_bike.parquet` or `link_bins_walk.parquet`: the other engine's columns for a layer
/// (`travellers` and `traveller_seconds` in place of `pcu` and `pcu_seconds`), the layer's links
/// by its ids and indices.
fn write_layer_link_bins(
    path: &Path,
    run_id: &str,
    layer: StaticLayer,
    bins: &LinkBins,
    graph: &RoadNetwork,
    metadata: &[(String, String)],
) -> PyResult<()> {
    let n = bins.len();
    let ids = graph.link_external_ids();
    let b = bins.bin_seconds();
    let fields = vec![
        Field::new("run_id", DataType::Utf8, false),
        Field::new("bin", DataType::UInt32, false),
        Field::new("start_s", DataType::UInt32, false),
        Field::new("link", DataType::Utf8, false),
        Field::new("link_index", DataType::UInt32, false),
        Field::new("crossings", DataType::UInt32, false),
        Field::new("travellers", DataType::Float64, false),
        Field::new("traveller_seconds", DataType::Float64, false),
    ];
    let columns: Vec<ArrayRef> = vec![
        Arc::new(StringArray::from(vec![run_id; n])),
        whole(bins.bins().to_vec()),
        whole(bins.bins().iter().map(|x| x * b).collect()),
        Arc::new(StringArray::from(
            bins.links().iter().map(|&l| ids.external(l)).collect::<Vec<_>>(),
        )),
        whole(bins.links().to_vec()),
        whole(bins.crossings().to_vec()),
        floats(bins.pcu().to_vec()),
        floats(bins.pcu_seconds().to_vec()),
    ];
    let mut metadata = metadata.to_vec();
    metadata.push(("openmobisim.bin_seconds".to_string(), b.to_string()));
    metadata.push(("openmobisim.layer".to_string(), layer.as_str().to_string()));
    write_batch(path, fields, columns, &metadata)
}

/// The metadata key of the demand and network fingerprint ([`demand_network_fingerprint`]).
const DEMAND_NETWORK_KEY: &str = "openmobisim.demand_network_fingerprint";
/// The metadata key of the run's fingerprint.
const RUN_FINGERPRINT_KEY: &str = "openmobisim.run_fingerprint";

/// What a run continued from another reads of it: every trip's set, current chain and
/// probabilities, and the run's fingerprint its chains table carries.
struct Saved {
    trips: Vec<SavedTrip>,
    run: Option<String>,
}

/// The chains table at `path` (as [`write_chains`] writes it) as every trip's saved set, the
/// links read by their ids on the layer of each trip's mode (`network`, the road network, or a
/// layer of `parts`), for the trips of `demand`; refused unless the table's demand and network
/// fingerprint is `demand_network`. Each chain's mode is the table's `mode` column's.
fn read_chains(
    path: &str,
    network: &RoadNetwork,
    parts: &StaticParts,
    trips: usize,
    demand_network: &str,
) -> PyResult<Saved> {
    use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

    let bad = |what: String| PyValueError::new_err(format!("the chains table {path:?}: {what}"));
    let file = File::open(path).map_err(|e| bad(e.to_string()))?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).map_err(|e| bad(e.to_string()))?;
    let value = |key: &str| -> Option<String> {
        builder
            .metadata()
            .file_metadata()
            .key_value_metadata()?
            .iter()
            .find(|kv| kv.key == key)
            .and_then(|kv| kv.value.clone())
    };
    let (theirs, run) = (value(DEMAND_NETWORK_KEY), value(RUN_FINGERPRINT_KEY));
    if theirs.as_deref() != Some(demand_network) {
        return Err(PyValueError::new_err(format!(
            "the run to continue from is not of this scenario: its demand and network \
             fingerprint is {}, this run's {demand_network} (the network and the trips run, with \
             each traveller's car ownership, must be the same)",
            theirs.as_deref().unwrap_or("not recorded in its chains table")
        )));
    }
    let reader = builder.build().map_err(|e| bad(e.to_string()))?;
    let mut out = vec![SavedTrip::default(); trips];
    for batch in reader {
        let batch = batch.map_err(|e| bad(e.to_string()))?;
        let column = |name: &str| {
            batch.column_by_name(name).ok_or_else(|| bad(format!("no column {name:?}")))
        };
        let (trip, chain, taken, probability, links) = (
            column("trip")?.as_any().downcast_ref::<UInt32Array>(),
            column("chain")?.as_any().downcast_ref::<UInt32Array>(),
            column("taken")?.as_any().downcast_ref::<BooleanArray>(),
            column("probability")?.as_any().downcast_ref::<Float64Array>(),
            column("links")?.as_any().downcast_ref::<ListArray>(),
        );
        let modes = column("mode")?.as_any().downcast_ref::<StringArray>();
        let (Some(trip), Some(chain), Some(taken), Some(probability), Some(links), Some(modes)) =
            (trip, chain, taken, probability, links, modes)
        else {
            return Err(bad("a column of another type than the engine writes".to_string()));
        };
        for i in 0..batch.num_rows() {
            let t = trip.value(i) as usize;
            let entry = out
                .get_mut(t)
                .ok_or_else(|| bad(format!("trip {t}, for a run of {trips} trips")))?;
            if chain.value(i) as usize != entry.chains.len() {
                return Err(bad(format!("trip {t}'s chains are not in their order")));
            }
            let mode =
                Mode::from_name(modes.value(i)).map_err(|e| bad(format!("trip {t}: {e}")))?;
            let ids = parts.of(mode).map_or(network, StaticNetwork::network).link_external_ids();
            let row = links.value(i);
            let names = row
                .as_any()
                .downcast_ref::<StringArray>()
                .ok_or_else(|| bad("links that are not the network's ids".to_string()))?;
            let route = (0..names.len())
                .map(|k| {
                    ids.typed_id_of::<LinkId>(names.value(k)).ok_or_else(|| {
                        bad(format!("trip {t}: link {:?} is not of this network", names.value(k)))
                    })
                })
                .collect::<PyResult<Vec<LinkId>>>()?;
            if taken.value(i) {
                entry.current = Some(entry.chains.len());
            }
            entry.chains.push(route);
            entry.modes.push(mode);
            // The probabilities of the trip's last choice: one per chain it had then.
            let p = probability.value(i);
            if !p.is_nan() && entry.probabilities.len() + 1 == entry.chains.len() {
                entry.probabilities.push(p);
            }
        }
    }
    Ok(Saved { trips: out, run })
}

// --- the function -----------------------------------------------------------------------------

/// Run the one-loop assignment on car trips, bike and walking trips whose mode is stated, and
/// trips choosing their mode among the car, the bike and walking, and write its tables to
/// `output_dir`; returns the run's results as a dict (see the module docs for what is read and
/// written).
///
/// # Errors
///
/// `ValueError` for inputs that cannot be read or do not fit together (a stated or offered mode
/// other than the car, the bike or walking, a flow level other than 0 or 2, a bike cost other than
/// `"utility"`, `"time"` or `"dedicated"`, a coefficient that cannot price a route, a fixed time
/// that is not a finite number of at least 0, a run to continue from whose chains table is not
/// of this demand and network), and if a file cannot be written.
#[pyfunction]
#[pyo3(signature = (
    network, run_id, output_dir, *, trips=None, trips_path=None, persons=None,
    persons_path=None, class_defaults=None, class_options=None, default_weight=1,
    window_start_s=None, window_end_s=None, flow_level=2, flow_step_s=300, cost_bin_s=300,
    link_bin_s=None, master_seed=0, choice_model="logit", choice_options=None,
    min_iterations=None, max_iterations=None, step_offset=None, switch_tolerance=None,
    found_tolerance=None, indifference_band=None, allocation=None, motorway_snap_m=None,
    car_access_s=None, car_egress_s=None, threads=None, start_chains_path=None,
    start_iterations=None, bike_cost=None, bike_access_s=None, bike_egress_s=None, modes=None,
    class_modes=None, walk_max_s=None, bike_max_s=None, class_limits=None
))]
#[allow(clippy::too_many_arguments, reason = "the scenario's parts, by name, as Python gives them")]
#[allow(clippy::too_many_lines, reason = "the steps of one run, in order")]
pub fn run_one_loop(
    py: Python<'_>,
    network: PyRef<'_, PyNetwork>,
    run_id: &str,
    output_dir: &str,
    trips: Option<Vec<TripRow>>,
    trips_path: Option<&str>,
    persons: Option<Vec<PersonRow>>,
    persons_path: Option<&str>,
    class_defaults: Option<HashMap<String, (bool, bool, bool)>>,
    class_options: Option<HashMap<String, HashMap<String, f64>>>,
    default_weight: u32,
    window_start_s: Option<u32>,
    window_end_s: Option<u32>,
    flow_level: u32,
    flow_step_s: u32,
    cost_bin_s: u32,
    link_bin_s: Option<u32>,
    master_seed: u64,
    choice_model: &str,
    choice_options: Option<HashMap<String, f64>>,
    min_iterations: Option<u32>,
    max_iterations: Option<u32>,
    step_offset: Option<u32>,
    switch_tolerance: Option<f64>,
    found_tolerance: Option<f64>,
    indifference_band: Option<f64>,
    allocation: Option<&str>,
    motorway_snap_m: Option<f64>,
    car_access_s: Option<f64>,
    car_egress_s: Option<f64>,
    threads: Option<usize>,
    start_chains_path: Option<&str>,
    start_iterations: Option<u32>,
    bike_cost: Option<&str>,
    bike_access_s: Option<f64>,
    bike_egress_s: Option<f64>,
    modes: Option<Vec<String>>,
    class_modes: Option<HashMap<String, Vec<String>>>,
    walk_max_s: Option<f64>,
    bike_max_s: Option<f64>,
    class_limits: Option<HashMap<String, HashMap<String, f64>>>,
) -> PyResult<Py<PyDict>> {
    let started = Instant::now();
    if flow_level != 0 && flow_level != 2 {
        return Err(PyValueError::new_err(format!(
            "the one-loop engine loads at flow level 0 (free flow) or 2 (the point queue), not \
             {flow_level}"
        )));
    }
    if flow_step_s == 0 || cost_bin_s == 0 || link_bin_s == Some(0) {
        return Err(PyValueError::new_err(
            "flow_step_s, the cost bins and link_bin_s must be positive",
        ));
    }
    // The demand, as the other engine reads it.
    let mut raw: Vec<RawTrip> = match (trips, trips_path) {
        (Some(rows), None) => rows.into_iter().map(trip_from_row).collect::<PyResult<_>>()?,
        (None, Some(path)) => read_trips_parquet(path).map_err(to_value_error)?,
        _ => return Err(PyValueError::new_err("give exactly one of trips or trips_path")),
    };
    check_distinct_keys(&raw).map_err(to_value_error)?;
    let stated: BTreeMap<&str, usize> = raw
        .iter()
        .filter_map(|t| t.mode.filter(|m| !matches!(m, Mode::Car | Mode::Bike | Mode::Walk)))
        .fold(BTreeMap::new(), |mut acc, m| {
            *acc.entry(m.as_str()).or_insert(0) += 1;
            acc
        });
    if !stated.is_empty() {
        let list: Vec<String> = stated.iter().map(|(m, n)| format!("{n} {m}")).collect();
        return Err(PyValueError::new_err(format!(
            "the one-loop engine runs car trips, and bike and walking trips whose mode is stated; \
             the demand states other modes: {} trips",
            list.join(", ")
        )));
    }
    let bike_cost = match bike_cost {
        None => BikeCost::Utility,
        Some(name) => BikeCost::from_name(name).ok_or_else(|| {
            PyValueError::new_err(format!(
                "bike_cost must be \"utility\", \"dedicated\" or \"time\", not {name:?}"
            ))
        })?,
    };
    let raw_persons = match (persons, persons_path) {
        (Some(rows), None) => rows.into_iter().map(person_from_row).collect(),
        (None, Some(path)) => read_persons_parquet(path).map_err(to_value_error)?,
        (None, None) => Vec::new(),
        (Some(_), Some(_)) => {
            return Err(PyValueError::new_err("give at most one of persons or persons_path"));
        }
    };
    let mut defaults = ClassDefaults::new().with_fallback(Ownership::VEHICLES);
    for (class, (car, bike, transit_pass)) in class_defaults.unwrap_or_default() {
        defaults = defaults.with_default(class, Ownership { car, bike, transit_pass });
    }
    let window = Window::resolve(&raw, window_start_s, window_end_s, default_weight)?;
    raw.retain(|t| window.contains(t.departure_time));
    let mut diagnostics = Diagnostics::new();
    let (travellers, trips) =
        build_travellers(raw, raw_persons, &defaults, default_weight, &mut diagnostics)
            .map_err(to_value_error)?;
    let demand_s = started.elapsed().as_secs_f64();
    // What mode choice offers each class, and every trip's modes.
    let offer = mode_choice_of(
        &travellers,
        modes.as_deref(),
        &class_modes.unwrap_or_default(),
        (walk_max_s, bike_max_s),
        &class_limits.unwrap_or_default(),
    )?;
    let trip_modes = openmobisim_core_assignment::modes::trip_modes(&travellers, &trips, &offer);
    // The bike and walk layers, only for a run whose trips may ride or walk.
    let uses = |mode: Mode| trip_modes.iter().any(|m| m.allows(mode));
    let parts = StaticParts {
        bike: if uses(Mode::Bike) { Some(network.static_layer(StaticLayer::Bike)?) } else { None },
        walk: if uses(Mode::Walk) { Some(network.static_layer(StaticLayer::Walk)?) } else { None },
        bike_cost,
    };
    let layer_turns: [Option<TurnTable>; 2] = [&parts.bike, &parts.walk]
        .map(|layer| layer.as_ref().map(|l| TurnTable::build(l.network(), l.network().signals())));

    // The classes' coefficients and the choice model.
    let run_options: BTreeMap<String, f64> =
        choice_options.unwrap_or_default().into_iter().collect();
    let class_options: HashMap<String, BTreeMap<String, f64>> = class_options
        .unwrap_or_default()
        .into_iter()
        .map(|(c, o)| (c, o.into_iter().collect()))
        .collect();
    for (c, o) in &class_options {
        if o.contains_key("beta_ln_path_size") {
            return Err(class_error(
                c,
                "the one-loop engine's path-size weight is the run's (choice_options), not a \
                 class's",
            ));
        }
    }
    let ids = travellers.class_external_ids();
    let mut classes = Vec::with_capacity(ids.count() as usize);
    for c in 0..ids.count() {
        let name = ids.external(c);
        let own = class_options.get(name).cloned().unwrap_or_default();
        classes.push(coefficients(choice_model, &run_options, &own, name)?);
    }
    let path_size = run_options.get("beta_ln_path_size").copied().unwrap_or(1.0);
    let model = model_of(choice_model, &run_options)?;
    let choice = ModeChoice {
        any: trip_modes.iter().any(TripModes::chooses_mode),
        mu: (choice_model == "nested_logit").then(|| run_options.get("mu").copied().unwrap_or(0.5)),
        classes: offer,
    };

    // The loop's parameters, the loading, the hook that keeps the last loading.
    let mut params = Params::default();
    if let Some(v) = min_iterations {
        params.min_iterations = v;
    }
    if let Some(v) = max_iterations {
        if v == 0 {
            return Err(PyValueError::new_err("the most iterations must be at least 1"));
        }
        params.max_iterations = v;
    }
    if let Some(v) = step_offset {
        params.step_offset = v;
    }
    for (name, value) in
        [("switch_tolerance", switch_tolerance), ("found_tolerance", found_tolerance)]
    {
        if let Some(v) = value {
            if !(v.is_finite() && (0.0..=1.0).contains(&v)) {
                return Err(PyValueError::new_err(format!("{name} must be from 0 to 1, got {v}")));
            }
        }
    }
    if let Some(v) = switch_tolerance {
        params.switch_tolerance = v;
    }
    if let Some(v) = found_tolerance {
        params.found_tolerance = v;
    }
    if let Some(v) = indifference_band {
        if !(v.is_finite() && v >= 0.0) {
            return Err(PyValueError::new_err(format!(
                "indifference_band must be a finite number of at least 0, got {v}"
            )));
        }
        params.indifference_band = v;
    }
    if let Some(name) = allocation {
        params.allocation = Allocation::from_name(name).ok_or_else(|| {
            let names: Vec<&str> = Allocation::ALL.iter().map(|a| a.name()).collect();
            PyValueError::new_err(format!(
                "allocation must be one of {}, got {name:?}",
                names.join(", ")
            ))
        })?;
    }
    if let Some(v) = motorway_snap_m {
        params.motorway_snap_metres = v;
    }
    // The car's fixed times, in whole seconds as the loading's clock counts them.
    let mut fixed = [0.0; 2];
    for (slot, (name, value)) in
        fixed.iter_mut().zip([("car_access_s", car_access_s), ("car_egress_s", car_egress_s)])
    {
        let v = value.unwrap_or(0.0);
        if !(v.is_finite() && v >= 0.0) {
            return Err(PyValueError::new_err(format!(
                "{name} must be a finite number of seconds of at least 0, got {v}"
            )));
        }
        *slot = v.round();
    }
    params.car_fixed_times = FixedTimes::new(fixed[0], fixed[1]);
    // The bike's fixed times, likewise.
    let mut fixed = [0.0; 2];
    for (slot, (name, value)) in
        fixed.iter_mut().zip([("bike_access_s", bike_access_s), ("bike_egress_s", bike_egress_s)])
    {
        let v = value.unwrap_or(0.0);
        if !(v.is_finite() && v >= 0.0) {
            return Err(PyValueError::new_err(format!(
                "{name} must be a finite number of seconds of at least 0, got {v}"
            )));
        }
        *slot = v.round();
    }
    params.bike_fixed_times = FixedTimes::new(fixed[0], fixed[1]);
    params.bike_cost = bike_cost;
    if threads == Some(0) {
        return Err(PyValueError::new_err("threads must be at least 1, or None"));
    }
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads.unwrap_or(0))
        .build()
        .map_err(|e| PyValueError::new_err(format!("the run's thread pool: {e}")))?;
    // The run to continue from: its chains table, checked against this demand and network.
    let demand_print =
        fingerprint_hex(demand_network_fingerprint(&network.inner, &travellers, &trips, &parts));
    let saved = match (start_chains_path, start_iterations) {
        (None, None) => None,
        (Some(path), Some(iterations)) => {
            let saved =
                read_chains(path, &network.inner, &parts, trips.len() as usize, &demand_print)?;
            Some((saved, iterations))
        }
        _ => {
            return Err(PyValueError::new_err(
                "give both start_chains_path and start_iterations, or neither",
            ));
        }
    };
    let start_print = saved.as_ref().map(|(s, i)| (s.run.clone().unwrap_or_default(), *i));
    let turns = TurnTable::build(&network.inner, network.inner.signals());
    let end_s = f64::from(window.end_s);
    let step_s = f64::from(flow_step_s);
    let point_queue = PointQueue {
        turns: &turns,
        end_seconds: end_s,
        step_seconds: step_s,
        bin_seconds: cost_bin_s,
    };
    let free_flow = FreeFlow { end_seconds: end_s };
    let loader: &dyn Loader = if flow_level == 2 { &point_queue } else { &free_flow };
    let money = vec![0.0; network.inner.link_count() as usize];
    let print = fingerprint(
        &network.inner,
        &travellers,
        &trips,
        &classes,
        choice_model,
        path_size,
        &params,
        (flow_level, flow_step_s, cost_bin_s),
        (window.start_s, window.end_s),
        master_seed,
        start_print.as_ref().map(|(p, i)| (p.as_str(), *i)),
        &parts,
        &choice,
    );
    fn layer<'l>(l: &'l Option<Arc<StaticNetwork>>, t: &'l Option<TurnTable>) -> Option<Layer<'l>> {
        match (l, t) {
            (Some(network), Some(turns)) => Some(Layer { network: network.as_ref(), turns }),
            _ => None,
        }
    }
    let mut inputs = Inputs {
        network: &network.inner,
        turns: &turns,
        layers: Layers {
            bike: layer(&parts.bike, &layer_turns[0]),
            walk: layer(&parts.walk, &layer_turns[1]),
        },
        travellers: &travellers,
        trips: &trips,
        classes: &classes,
        mode_choice: &choice.classes,
        money_per_link: &money,
        model: model.as_ref(),
        loader,
        seed: RngKey::from_seed(master_seed),
        initial: None,
    };
    let clock = Instant::now();
    let mut last = LastLoading::default();
    let (state, reports) = pool
        .install(|| {
            if let Some((saved, iterations)) = &saved {
                inputs.initial = Some(restore(&inputs, &params, &saved.trips, *iterations)?);
            }
            assign(&inputs, &params, &mut last)
        })
        .map_err(to_value_error)?;
    let assign_s = clock.elapsed().as_secs_f64();
    // The modes left out of the sets of the trips choosing their mode, and why.
    let clock = Instant::now();
    let left = if choice.any {
        pool.install(|| left_out(&inputs, &params, &state)).map_err(to_value_error)?
    } else {
        LeftOut::default()
    };
    let left_s = clock.elapsed().as_secs_f64();

    // The results.
    let clock = Instant::now();
    let rows = trip_rows(&travellers, &trips, &trip_modes, &state, &last, end_s)?;
    let bins = match link_bin_s {
        Some(b) => Some(link_bins(&network.inner, &turns, &last, flow_level, end_s, step_s, b)?),
        None => None,
    };
    let ([bike_bins, walk_bins], layer_legs) = match link_bin_s {
        Some(b) => layer_bins(&parts, &travellers, &trips, &state, end_s, b),
        None => ([None, None], [Vec::new(), Vec::new()]),
    };
    let (sets, choices) = routes(&network.inner, &travellers, &trips, &state);
    let results_s = clock.elapsed().as_secs_f64();

    let clock = Instant::now();
    let dir = Path::new(output_dir);
    std::fs::create_dir_all(dir).map_err(to_value_error)?;
    let print_hex = fingerprint_hex(print);
    let metadata = vec![
        ("openmobisim.engine".to_string(), "one-loop".to_string()),
        (RUN_FINGERPRINT_KEY.to_string(), print_hex.clone()),
        ("openmobisim.master_seed".to_string(), master_seed.to_string()),
        (DEMAND_NETWORK_KEY.to_string(), demand_print.clone()),
    ];
    write_trip_results(&dir.join("trip_results.parquet"), run_id, &rows, &metadata)?;
    write_convergence(&dir.join("convergence.parquet"), run_id, &reports, &metadata)?;
    write_chains(
        &dir.join("chains.parquet"),
        &state,
        &network.inner,
        &parts,
        &travellers,
        &trips,
        &classes,
        &metadata,
    )?;
    let link_bins_path = dir.join("link_bins.parquet");
    if let Some(b) = &bins {
        write_link_bins(&link_bins_path, run_id, b, &network.inner, &metadata)?;
    }
    let mut layer_paths: [Option<String>; 2] = [None, None];
    for (i, (layer, b)) in
        [(StaticLayer::Bike, &bike_bins), (StaticLayer::Walk, &walk_bins)].into_iter().enumerate()
    {
        let (Some(b), Some(graph)) = (b, [&parts.bike, &parts.walk][i]) else { continue };
        let path = dir.join(format!("link_bins_{}.parquet", layer.as_str()));
        write_layer_link_bins(&path, run_id, layer, b, graph.network(), &metadata)?;
        layer_paths[i] = Some(path.to_string_lossy().into_owned());
    }
    let written_s = clock.elapsed().as_secs_f64();
    let mut timings: Vec<(String, Option<u32>, f64)> =
        vec![("demand_read".to_string(), None, demand_s)];
    for r in &reports {
        let s = r.stage_seconds;
        for (stage, seconds) in [
            ("search", s.search),
            ("choose", s.choose),
            ("load", s.load),
            ("measure", s.measure),
            ("assess", s.assess),
        ] {
            timings.push((stage.to_string(), Some(r.iteration), seconds));
        }
    }
    timings.push(("assignment".to_string(), None, assign_s));
    if choice.any {
        timings.push(("modes_left_out".to_string(), None, left_s));
    }
    timings.push(("results_assembled".to_string(), None, results_s));
    timings.push(("results_written".to_string(), None, written_s));
    timings.push(("total".to_string(), None, started.elapsed().as_secs_f64()));
    write_timings(&dir.join("timings.csv"), &timings).map_err(to_value_error)?;

    // What Python reads.
    let out = PyDict::new(py);
    out.set_item("engine", "one-loop")?;
    out.set_item("fingerprint", &print_hex)?;
    out.set_item("demand_network_fingerprint", &demand_print)?;
    out.set_item("start_from", start_print)?;
    out.set_item("fingerprint_covers", FINGERPRINT_COVERS.to_vec())?;
    out.set_item("master_seed", master_seed)?;
    out.set_item("choice_model", choice_model)?;
    let iterations = u32::try_from(reports.len()).expect("few iterations");
    out.set_item("iterations", iterations)?;
    out.set_item("converged", iterations < params.max_iterations)?;
    let p = PyDict::new(py);
    p.set_item("min_iterations", params.min_iterations)?;
    p.set_item("max_iterations", params.max_iterations)?;
    p.set_item("switch_tolerance", params.switch_tolerance)?;
    p.set_item("found_tolerance", params.found_tolerance)?;
    p.set_item("indifference_band", params.indifference_band)?;
    p.set_item("step_offset", params.step_offset)?;
    p.set_item("allocation", params.allocation.name())?;
    p.set_item("cost_source", format!("{:?}", params.cost_source).to_lowercase())?;
    p.set_item("motorway_snap_m", params.motorway_snap_metres)?;
    p.set_item("car_access_s", params.car_fixed_times.access_seconds)?;
    p.set_item("car_egress_s", params.car_fixed_times.egress_seconds)?;
    p.set_item("bike_access_s", params.bike_fixed_times.access_seconds)?;
    p.set_item("bike_egress_s", params.bike_fixed_times.egress_seconds)?;
    p.set_item("bike_cost", params.bike_cost.as_str())?;
    p.set_item("flow_level", flow_level)?;
    p.set_item("flow_step_s", flow_step_s)?;
    p.set_item("cost_bin_s", cost_bin_s)?;
    p.set_item("link_bin_s", link_bin_s)?;
    p.set_item("path_size_weight", path_size)?;
    p.set_item("nest_scale", choice.mu)?;
    // What mode choice offers each class (only when a trip chooses its mode).
    if choice.any {
        let offered = PyDict::new(py);
        for (c, k) in choice.classes.iter().enumerate() {
            let d = PyDict::new(py);
            let names: Vec<&str> = k.modes.iter().map(Mode::as_str).collect();
            d.set_item("modes", names)?;
            d.set_item("walk_max_s", k.walk_max_seconds)?;
            d.set_item("bike_max_s", k.bike_max_seconds)?;
            offered.set_item(ids.external(u32::try_from(c).expect("classes fit u32")), d)?;
        }
        p.set_item("mode_choice", offered)?;
    }
    out.set_item("parameters", p)?;
    let k = PyDict::new(py);
    for c in 0..ids.count() {
        let v = &classes[c as usize];
        let d = PyDict::new(py);
        d.set_item("in_vehicle_per_minute", v.in_vehicle_per_minute)?;
        d.set_item("walk_per_minute", v.walk_per_minute)?;
        d.set_item("wait_per_minute", v.wait_per_minute)?;
        d.set_item("parking_per_minute", v.parking_per_minute)?;
        d.set_item("per_transfer", v.per_transfer)?;
        d.set_item("per_money", v.per_money)?;
        d.set_item("per_kilometre", v.per_kilometre)?;
        d.set_item("bike_mixed_per_kilometre", v.bike_mixed_per_kilometre)?;
        d.set_item("bike_lane_per_kilometre", v.bike_lane_per_kilometre)?;
        d.set_item("bike_separated_per_kilometre", v.bike_separated_per_kilometre)?;
        d.set_item("car_constant", v.car_constant)?;
        d.set_item("bike_constant", v.bike_constant)?;
        d.set_item("walk_constant", v.walk_constant)?;
        k.set_item(ids.external(c), d)?;
    }
    out.set_item("coefficients", k)?;
    out.set_item("window", window.to_dict(py, None)?)?;

    let conv = PyDict::new(py);
    conv.set_item(
        "iteration",
        reports.iter().map(|r| r.iteration).collect::<Vec<_>>().into_pyarray(py),
    )?;
    conv.set_item("found", reports.iter().map(|r| r.found).collect::<Vec<_>>().into_pyarray(py))?;
    conv.set_item("added", reports.iter().map(|r| r.added).collect::<Vec<_>>().into_pyarray(py))?;
    for (name, values) in convergence_columns(&reports) {
        conv.set_item(name, values.into_pyarray(py))?;
    }
    out.set_item("convergence", conv)?;

    let n = u32::try_from(rows.traveller_id.len()).expect("the trips fit u32");
    let maybe =
        |v: &[Option<f64>]| -> Vec<f64> { v.iter().map(|x| x.unwrap_or(f64::NAN)).collect() };
    let travel = rows.travel_time_s();
    let t = PyDict::new(py);
    t.set_item("trip", (0..n).collect::<Vec<u32>>().into_pyarray(py))?;
    t.set_item("traveller_id", rows.traveller_id.clone())?;
    t.set_item("trip_seq", rows.trip_seq.clone().into_pyarray(py))?;
    t.set_item("user_class", rows.user_class.clone())?;
    t.set_item("weight", rows.weight.clone().into_pyarray(py))?;
    t.set_item("origin_lon", rows.origin.iter().map(|p| p.0).collect::<Vec<_>>().into_pyarray(py))?;
    t.set_item("origin_lat", rows.origin.iter().map(|p| p.1).collect::<Vec<_>>().into_pyarray(py))?;
    t.set_item(
        "destination_lon",
        rows.destination.iter().map(|p| p.0).collect::<Vec<_>>().into_pyarray(py),
    )?;
    t.set_item(
        "destination_lat",
        rows.destination.iter().map(|p| p.1).collect::<Vec<_>>().into_pyarray(py),
    )?;
    t.set_item("departure_s", rows.departure_s.clone().into_pyarray(py))?;
    t.set_item("mode", rows.mode.clone())?;
    t.set_item("mode_choice", rows.mode_choice.clone())?;
    t.set_item("outcome", rows.outcome.clone())?;
    t.set_item("arrival_s", maybe(&rows.arrival_s).into_pyarray(py))?;
    t.set_item("travel_time_s", maybe(&travel).into_pyarray(py))?;
    t.set_item("link_at_end", vec![f64::NAN; n as usize].into_pyarray(py))?;
    t.set_item("vehicle_s", maybe(&rows.vehicle_s).into_pyarray(py))?;
    t.set_item("access_egress_s", maybe(&rows.access_egress_s).into_pyarray(py))?;
    out.set_item("trip_results", t)?;

    // The counts of the last loading, by outcome, and its travel time by traveller weight; the
    // same by mode.
    let count = |o: &str| rows.outcome.iter().filter(|x| **x == o).count();
    let (mut total_s, mut people, mut completed_people, mut vehicle_s) = (0.0, 0.0, 0.0, 0.0);
    let by_mode = PyDict::new(py);
    for mode in [Mode::Car, Mode::Bike, Mode::Walk] {
        let name = mode.as_str();
        let of_mode: Vec<usize> =
            (0..rows.mode.len()).filter(|&i| rows.mode[i] == Some(name)).collect();
        if of_mode.is_empty() {
            continue;
        }
        let (mut m_total, mut m_people, mut m_completed) = (0.0, 0.0, 0.0);
        let row = PyDict::new(py);
        for outcome in ["completed", "truncated", "no_vehicle_available", "no_feasible_path"] {
            row.set_item(outcome, of_mode.iter().filter(|&&i| rows.outcome[i] == outcome).count())?;
        }
        row.set_item("mode_not_available", 0)?;
        for &i in &of_mode {
            let w = f64::from(rows.weight[i]);
            m_people += w;
            if let Some(s) = travel[i] {
                m_total += w * s;
                m_completed += w;
            }
        }
        row.set_item("total_travel_time_s", m_total)?;
        row.set_item("people", m_people)?;
        row.set_item("completed_people", m_completed)?;
        by_mode.set_item(name, row)?;
    }
    for (i, w) in rows.weight.iter().enumerate() {
        let w = f64::from(*w);
        people += w;
        if let Some(s) = travel[i] {
            total_s += w * s;
            completed_people += w;
        }
        if let Some(v) = rows.vehicle_s[i] {
            vehicle_s += w * v;
        }
    }
    out.set_item("completion_by_mode", by_mode)?;
    out.set_item("total_vehicle_time_s", vehicle_s)?;
    let [bike_path, walk_path] = layer_paths;
    out.set_item("bike_link_bins", bike_bins.map(PyLinkBins::new))?;
    out.set_item("walk_link_bins", walk_bins.map(PyLinkBins::new))?;
    out.set_item("link_bins_bike_path", bike_path)?;
    out.set_item("link_bins_walk_path", walk_path)?;
    // The legs on the bike and walk layers, for the select-link analysis (only with link bins).
    let legs = PyDict::new(py);
    for (name, of_layer) in ["bike", "walk"].into_iter().zip(&layer_legs) {
        let c = PyDict::new(py);
        let mut offsets: Vec<u32> = vec![0];
        let mut links: Vec<u32> = Vec::new();
        for leg in of_layer {
            links.extend(&leg.links);
            offsets.push(u32::try_from(links.len()).expect("fewer than 2^32 leg links"));
        }
        c.set_item("trip", of_layer.iter().map(|l| l.trip).collect::<Vec<_>>().into_pyarray(py))?;
        c.set_item(
            "start_s",
            of_layer.iter().map(|l| l.start_s).collect::<Vec<_>>().into_pyarray(py),
        )?;
        c.set_item("link_offsets", offsets.into_pyarray(py))?;
        c.set_item("links", links.into_pyarray(py))?;
        legs.set_item(name, c)?;
    }
    out.set_item("layer_legs", legs)?;
    out.set_item("total_trips", n)?;
    out.set_item("completed", count("completed"))?;
    out.set_item("truncated", count("truncated"))?;
    out.set_item("no_vehicle_available", count("no_vehicle_available"))?;
    // A trip choosing its mode with no alternative counts under no feasible path (no mode it is
    // offered reaches its destination), as the other engine counts it.
    out.set_item("no_feasible_path", count("no_feasible_path") + count("no_alternative"))?;
    out.set_item("no_alternative", count("no_alternative"))?;
    out.set_item("mode_not_available", 0)?;
    // The trips choosing their mode, and the modes left out of their sets, by why.
    let m = PyDict::new(py);
    for (key, value) in [
        ("choosing", left.choosing),
        ("no_chain", left.no_chain),
        ("car_no_vehicle", left.car_no_vehicle),
        ("car_no_route", left.car_no_route),
        ("bike_no_vehicle", left.bike_no_vehicle),
        ("bike_no_route", left.bike_no_route),
        ("bike_too_long", left.bike_too_long),
        ("walk_no_route", left.walk_no_route),
        ("walk_too_long", left.walk_too_long),
    ] {
        m.set_item(key, value)?;
    }
    out.set_item("modes_left_out", m)?;
    out.set_item("total_travel_time_s", total_s)?;
    out.set_item("people", people)?;
    out.set_item("completed_people", completed_people)?;
    out.set_item("link_bins", bins.map(PyLinkBins::new))?;
    out.set_item(
        "link_bins_path",
        link_bin_s.map(|_| link_bins_path.to_string_lossy().into_owned()),
    )?;
    let link_count = network.inner.link_count();
    out.set_item("route_choices", PyRouteChoices::new(py, &choices, &sets)?)?;
    out.set_item("route_sets", PyRouteSets::new(Arc::new(sets), link_count))?;
    out.set_item("timings", timings)?;
    for (key, file) in [
        ("trip_results_path", "trip_results.parquet"),
        ("convergence_path", "convergence.parquet"),
        ("chains_path", "chains.parquet"),
        ("timings_path", "timings.csv"),
    ] {
        out.set_item(key, dir.join(file).to_string_lossy().into_owned())?;
    }
    Ok(out.unbind())
}
