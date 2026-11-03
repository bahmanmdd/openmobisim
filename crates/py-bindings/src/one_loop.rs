//! The one-loop assignment (`openmobisim-core-assignment`), exposed to Python.
//!
//! [`run_one_loop`] takes the scenario's parts by name, as `run_pipeline` does, builds the
//! assignment's inputs, runs it and writes its tables to the run folder:
//!
//! | File | What it holds |
//! |---|---|
//! | `trip_results.parquet` | one row per trip of the window: the columns of the other engine's table |
//! | `convergence.parquet` | one row per assignment iteration: the measures of its report |
//! | `chains.parquet` | one row per chain of every trip's choice set: its links, whether it was taken |
//! | `link_bins.parquet` | per link and bin of exit time, the traffic and its time (with `link_bin_s`) |
//! | `timings.csv` | the wall-clock seconds of each stage |
//!
//! The manifest is written by the Python package, from what this function returns.
//!
//! **What it runs.** Car trips only (a trip without a stated mode is a car trip), loaded at
//! flow level 0 (free flow) or 2 (the point queue). The scenario's other parts are checked by
//! the Python package, which refuses by name what this engine does not have before calling it.
//!
//! **The user classes' coefficients.** The scenario states a utility per minute and per unit
//! of each attribute (`beta_time_min` and the like, negative for what travellers dislike); the
//! assignment weighs each attribute by its cost per unit, the opposite sign. Per class, the
//! run's options are overridden by the class's own, by name, and become:
//!
//! - the cost of a minute in the car: `−beta_time_min` (−0.2 for the logit when not given, its
//!   default; 1 for the deterministic choice, whose choice does not depend on the scale);
//! - of a minute walking or waiting: `−(beta_time_min + beta_walk_min)` and
//!   `−(beta_time_min + beta_wait_min)` (walking and waiting count on top of the time);
//! - of a transfer: `−beta_transfers`; of a unit of money: `−beta_cost_eur`, **0 when no money
//!   coefficient is given** (a run without one is a time-based run); of a kilometre:
//!   `−beta_length_km`, 0 when not given.
//!
//! `beta_ln_path_size` (1 by default) weighs the path-size term of the logit. A car route has
//! no walking, waiting or transfer, so only time, money and distance count here.

use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

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
    Coefficients, FreeFlow, HighestUtility, Inputs, IterationReport, Loaded, Loader, Params, Place,
    PointQueue, StageHook, State, assign,
};
use openmobisim_core_choice::{ChoiceModel, Logit, Options};
use openmobisim_core_demand::{
    ClassDefaults, Mode, Ownership, RawTrip, Travellers, Trips, build_travellers,
    check_distinct_keys, read_persons_parquet, read_trips_parquet,
};
use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_loading::{FidelityLevel, LinkBins, LinkTimes, Vehicle, run_ltm_binned};
use openmobisim_core_routes::{Route, RouteKey, RouteSets};
use openmobisim_core_sim::{NO_ROUTE, RouteChoices};
use openmobisim_core_types::diagnostics::Diagnostics;
use openmobisim_core_types::hash::{Fnv1a, fingerprint_hex};
use openmobisim_core_types::ids::{EntityId, LinkId, NodeId, TripId};
use openmobisim_core_types::rng::RngKey;
use openmobisim_core_types::units::Duration;
use openmobisim_core_types::{CODE_VERSION, VERSION};

use crate::choice::PyRouteChoices;
use crate::network::PyNetwork;
use crate::pipeline::{
    PersonRow, PyLinkBins, TripRow, Window, person_from_row, to_value_error, trip_from_row,
    write_timings,
};
use crate::routes::PyRouteSets;

/// The names of the coefficients a class's options may give, besides `beta_ln_path_size`.
const COEFFICIENTS: [&str; 7] = [
    "beta_time_min",
    "beta_walk_min",
    "beta_wait_min",
    "beta_transfers",
    "beta_cost_eur",
    "beta_length_km",
    // A bike leg's weight on mixed traffic: no car route has any.
    "beta_bike_mixed_km",
];

/// What the fingerprint of a run of this engine covers, in words, for the manifest.
const FINGERPRINT_COVERS: [&str; 12] = [
    "the network: every node's position and signal, every link's ends, class, lanes, length, \
     free-flow time, storage, capacity, jam density, wave speed, control delay and closure",
    "the trips run (those departing inside the window): traveller, place in the day, user class, \
     weight, origin, destination, departure second, stated mode",
    "each traveller's car ownership",
    "each user class's coefficients",
    "the choice model and its path-size weight",
    "the loop's parameters: minimum and maximum iterations, the switch and found tolerances, the \
     averaging offset, the cost source, the snapping distance to motorways",
    "the flow level",
    "the loading step",
    "the width of the bins the link travel times are read in",
    "the window",
    "the master seed",
    "the engine and the code version",
];

/// Why a class's options cannot be read, naming the class.
fn class_error(class: &str, message: &str) -> PyErr {
    PyValueError::new_err(format!("user class {class:?}: {message}"))
}

/// A class's coefficients from the run's options overridden by the class's, as the module docs
/// say; `model` is `"deterministic"` or `"logit"`.
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
        if k == "beta_ln_path_size" {
            continue;
        }
        if !COEFFICIENTS.contains(&k.as_str()) {
            return Err(class_error(
                name,
                &format!(
                    "the one-loop engine has no coefficient {k:?}; it reads {} and \
                     beta_ln_path_size",
                    COEFFICIENTS.join(", ")
                ),
            ));
        }
        if !v.is_finite() {
            return Err(class_error(name, &format!("{k} must be a finite number, got {v}")));
        }
    }
    let logit = model == "logit";
    let given = |k: &str, default: f64| merged.get(k).copied().unwrap_or(default);
    let time = given("beta_time_min", if logit { -0.2 } else { -1.0 });
    let walk = given("beta_walk_min", if logit { -0.13 } else { 0.0 });
    let wait = given("beta_wait_min", if logit { -0.09 } else { 0.0 });
    Ok(Coefficients {
        in_vehicle_per_minute: -time,
        walk_per_minute: -(time + walk),
        wait_per_minute: -(time + wait),
        per_transfer: -given("beta_transfers", if logit { -1.0 } else { 0.0 }),
        per_money: -given("beta_cost_eur", 0.0),
        per_kilometre: -given("beta_length_km", 0.0),
    })
}

/// The run's choice model: the deterministic choice, or the logit with its path-size weight.
fn model_of(name: &str, options: &BTreeMap<String, f64>) -> PyResult<Box<dyn ChoiceModel>> {
    match name {
        "deterministic" => Ok(Box::new(HighestUtility)),
        "logit" => {
            let mut o: Options = [("beta_utility".to_string(), 1.0)].into_iter().collect();
            if let Some(&v) = options.get("beta_ln_path_size") {
                o.insert("beta_ln_path_size".to_string(), v);
            }
            Ok(Box::new(Logit::from_options(&o).map_err(to_value_error)?))
        }
        other => Err(PyValueError::new_err(format!(
            "the one-loop engine's choice models are \"deterministic\" and \"logit\", not {other:?}"
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
    /// Each vehicle's arrival, `None` if unfinished.
    arrival: Vec<Option<f64>>,
}

impl StageHook for LastLoading {
    fn before_load(&mut self, _iteration: u32, state: &State, vehicles: &[Vehicle]) {
        // The loop makes one vehicle per car leg that uses a link, trip after trip.
        self.trip_of.clear();
        for (t, entry) in state.trips().iter().enumerate() {
            if let Some(chain) = entry.current_chain() {
                let legs = chain.legs().iter().filter(|l| !l.links.is_empty()).count();
                self.trip_of.extend(std::iter::repeat_n(TripId::from_index(t), legs));
            }
        }
        self.vehicles = vehicles.to_vec();
    }

    fn after_load(&mut self, _iteration: u32, loaded: &Loaded) {
        self.arrival.clone_from(&loaded.arrival);
    }
}

/// A hash of what decides the run's results ([`FINGERPRINT_COVERS`]).
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
) -> u64 {
    let mut h = Fnv1a::new();
    h.write_str("one-loop");
    h.write_str(VERSION);
    h.write_u32(CODE_VERSION);
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
    h.write_u32(params.step_offset);
    h.write_str(&format!("{:?}", params.cost_source));
    h.write_f64(params.motorway_snap_metres);
    h.write_u32(loading.0);
    h.write_u32(loading.1);
    h.write_u32(loading.2);
    h.write_u32(window.0);
    h.write_u32(window.1);
    h.write_u64(seed);
    h.finish()
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
    mode: Vec<&'static str>,
    outcome: Vec<&'static str>,
    arrival_s: Vec<Option<f64>>,
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

/// Each trip's outcome and arrival from the last loading and the final state.
fn trip_rows(
    travellers: &Travellers,
    trips: &Trips,
    state: &State,
    last: &LastLoading,
) -> PyResult<TripRows> {
    let n = trips.len() as usize;
    // Per trip: `None` if none of its legs was loaded; `Some(None)` if a vehicle had not
    // arrived; `Some(Some(a))` with its last vehicle's arrival.
    let mut loaded: Vec<Option<Option<f64>>> = vec![None; n];
    if last.trip_of.len() != last.arrival.len() {
        return Err(PyValueError::new_err(format!(
            "the last loading returned {} arrivals for {} vehicles",
            last.arrival.len(),
            last.trip_of.len()
        )));
    }
    for (trip, &at) in last.trip_of.iter().zip(&last.arrival) {
        let slot = &mut loaded[trip.index()];
        *slot = Some(match (*slot, at) {
            (Some(None), _) | (_, None) => None,
            (Some(Some(a)), Some(b)) => Some(a.max(b)),
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
        outcome: Vec::with_capacity(n),
        arrival_s: Vec::with_capacity(n),
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
        rows.mode.push(Mode::Car.as_str());
        let entry = state.trip(trip);
        let (outcome, arrival) = match (entry.current, slot) {
            (None, _) if !state.has_car(traveller) => ("no_vehicle_available", None),
            (None, _) => ("no_feasible_path", None),
            // A chain whose ends snapped to one node uses no link: it arrives as it sets out.
            (Some(_), None) => ("completed", Some(f64::from(departure))),
            (Some(_), Some(Some(a))) => ("completed", Some(*a)),
            (Some(_), Some(None)) => ("truncated", None),
        };
        rows.outcome.push(outcome);
        rows.arrival_s.push(arrival);
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

/// The route sets and route choices of the final state: per pair of snapped trip ends, every
/// chain any of its trips has in its choice set, in the order first found (trips in order);
/// per trip, the chain it takes.
fn routes(
    network: &RoadNetwork,
    travellers: &Travellers,
    trips: &Trips,
    state: &State,
) -> (RouteSets, RouteChoices) {
    let mut by_key: BTreeMap<RouteKey, Vec<Vec<u32>>> = BTreeMap::new();
    let key_of = |t: usize| -> Option<RouteKey> {
        let chain = state.trips()[t].set.first()?;
        let (first, last) = (chain.legs().first()?, chain.legs().last()?);
        let (Place::Road(o), Place::Road(d)) = (first.from, last.to);
        Some(RouteKey::new(o, d))
    };
    for (t, entry) in state.trips().iter().enumerate() {
        let Some(key) = key_of(t) else { continue };
        let routes = by_key.entry(key).or_default();
        for chain in &entry.set {
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
        choices.alternatives[t] = u32::try_from(entry.set.len()).expect("a set fits u32");
        let (Some(current), Some(key)) = (entry.current, key_of(t)) else { continue };
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
        Arc::new(BooleanArray::from(vec![false; n])),
        Arc::new(StringArray::from(rows.outcome.clone())),
        Arc::new(Float64Array::from(rows.arrival_s.clone())),
        Arc::new(Float64Array::from(travel.clone())),
        Arc::new(UInt32Array::from(nulls)),
        Arc::new(Float64Array::from(travel)),
        Arc::new(Float64Array::from(
            rows.arrival_s.iter().map(|a| a.map(|_| 0.0)).collect::<Vec<_>>(),
        )),
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

/// The convergence table's columns, as real numbers but the first two.
fn convergence_columns(reports: &[IterationReport]) -> Vec<(&'static str, Vec<f64>)> {
    let col = |f: &dyn Fn(&IterationReport) -> f64| reports.iter().map(f).collect::<Vec<f64>>();
    vec![
        ("found_share", col(&|r| r.found_share)),
        ("switch_share", col(&|r| r.switch_share)),
        ("switch_floor", col(&|r| r.switch_floor)),
        ("switch_excess", col(&|r| r.switch_excess)),
        ("departed", col(&|r| r.departed)),
        ("arrived", col(&|r| r.arrived)),
        ("unfinished", col(&|r| r.unfinished)),
        ("unfinished_share", col(&|r| r.unfinished_share)),
        ("without_chain", col(&|r| r.without_chain)),
        ("gap", col(&|r| r.duality_gap)),
        ("gap_deterministic", col(&|r| r.gap_deterministic)),
        ("spread", col(&|r| r.on_cheapest_share)),
        ("mean_trip_s", col(&|r| r.mean_trip_seconds)),
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
    ];
    let mut columns: Vec<ArrayRef> = vec![
        Arc::new(StringArray::from(vec![run_id; n])),
        whole(reports.iter().map(|r| r.iteration).collect()),
        whole(reports.iter().map(|r| r.found).collect()),
    ];
    for (name, values) in convergence_columns(reports) {
        fields.push(Field::new(name, DataType::Float64, false));
        columns.push(floats(values));
    }
    write_batch(path, fields, columns, metadata)
}

/// One row per chain of every trip's set: `trip`, `chain` (its place in the set), `taken`,
/// `probability` (the model's at the trip's last choice), `cost` (utility units) and
/// `travel_time_s` at the final link travel times, and its `links`.
fn write_chains(
    path: &Path,
    state: &State,
    travellers: &Travellers,
    trips: &Trips,
    classes: &[Coefficients],
    metadata: &[(String, String)],
) -> PyResult<()> {
    let (mut trip, mut chain, mut taken, mut probability, mut cost, mut seconds, mut links) =
        (vec![], vec![], vec![], vec![], vec![], vec![], vec![]);
    for (t, entry) in state.trips().iter().enumerate() {
        let id = TripId::from_index(t);
        let k = classes[travellers.user_class(trips.traveller(id)).index()];
        for (c, ch) in entry.set.iter().enumerate() {
            trip.push(id.raw());
            chain.push(u32::try_from(c).expect("a set fits u32"));
            taken.push(entry.current == Some(c));
            probability.push(entry.probabilities.get(c).copied().unwrap_or(f64::NAN));
            cost.push(k.cost(ch.total()));
            seconds.push(ch.total().total_seconds());
            links.push(Some(ch.road_links().into_iter().map(Some).collect::<Vec<_>>()));
        }
    }
    let list = ListArray::from_iter_primitive::<UInt32Type, _, _>(links);
    let fields = vec![
        Field::new("trip", DataType::UInt32, false),
        Field::new("chain", DataType::UInt32, false),
        Field::new("taken", DataType::Boolean, false),
        Field::new("probability", DataType::Float64, false),
        Field::new("cost", DataType::Float64, false),
        Field::new("travel_time_s", DataType::Float64, false),
        Field::new("links", list.data_type().clone(), true),
    ];
    let columns: Vec<ArrayRef> = vec![
        whole(trip),
        whole(chain),
        Arc::new(BooleanArray::from(taken)),
        floats(probability),
        floats(cost),
        floats(seconds),
        Arc::new(list),
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

// --- the function -----------------------------------------------------------------------------

/// Run the one-loop assignment on car trips and write its tables to `output_dir`; returns the
/// run's results as a dict (see the module docs for what is read and written).
///
/// # Errors
///
/// `ValueError` for inputs that cannot be read or do not fit together (a mode other than the
/// car, a flow level other than 0 or 2, a coefficient that cannot price a route), and if a
/// file cannot be written.
#[pyfunction]
#[pyo3(signature = (
    network, run_id, output_dir, *, trips=None, trips_path=None, persons=None,
    persons_path=None, class_defaults=None, class_options=None, default_weight=1,
    window_start_s=None, window_end_s=None, flow_level=2, flow_step_s=300, cost_bin_s=300,
    link_bin_s=None, master_seed=0, choice_model="logit", choice_options=None,
    min_iterations=None, max_iterations=None, step_offset=None, switch_tolerance=None,
    found_tolerance=None, motorway_snap_m=None
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
    motorway_snap_m: Option<f64>,
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
        .filter_map(|t| t.mode.filter(|m| *m != Mode::Car))
        .fold(BTreeMap::new(), |mut acc, m| {
            *acc.entry(m.as_str()).or_insert(0) += 1;
            acc
        });
    if !stated.is_empty() {
        let list: Vec<String> = stated.iter().map(|(m, n)| format!("{n} {m}")).collect();
        return Err(PyValueError::new_err(format!(
            "the one-loop engine runs car trips only; the demand states other modes: {} trips",
            list.join(", ")
        )));
    }
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
    if let Some(v) = motorway_snap_m {
        params.motorway_snap_metres = v;
    }
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
    );
    let inputs = Inputs {
        network: &network.inner,
        turns: &turns,
        travellers: &travellers,
        trips: &trips,
        classes: &classes,
        money_per_link: &money,
        model: model.as_ref(),
        loader,
        seed: RngKey::from_seed(master_seed),
        initial: None,
    };
    let clock = Instant::now();
    let mut last = LastLoading::default();
    let (state, reports) = assign(&inputs, &params, &mut last).map_err(to_value_error)?;
    let assign_s = clock.elapsed().as_secs_f64();

    // The results.
    let clock = Instant::now();
    let rows = trip_rows(&travellers, &trips, &state, &last)?;
    let bins = match link_bin_s {
        Some(b) => Some(link_bins(&network.inner, &turns, &last, flow_level, end_s, step_s, b)?),
        None => None,
    };
    let (sets, choices) = routes(&network.inner, &travellers, &trips, &state);
    let results_s = clock.elapsed().as_secs_f64();

    let clock = Instant::now();
    let dir = Path::new(output_dir);
    std::fs::create_dir_all(dir).map_err(to_value_error)?;
    let print_hex = fingerprint_hex(print);
    let metadata = vec![
        ("openmobisim.engine".to_string(), "one-loop".to_string()),
        ("openmobisim.run_fingerprint".to_string(), print_hex.clone()),
        ("openmobisim.master_seed".to_string(), master_seed.to_string()),
    ];
    write_trip_results(&dir.join("trip_results.parquet"), run_id, &rows, &metadata)?;
    write_convergence(&dir.join("convergence.parquet"), run_id, &reports, &metadata)?;
    write_chains(&dir.join("chains.parquet"), &state, &travellers, &trips, &classes, &metadata)?;
    let link_bins_path = dir.join("link_bins.parquet");
    if let Some(b) = &bins {
        write_link_bins(&link_bins_path, run_id, b, &network.inner, &metadata)?;
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
    timings.push(("results_assembled".to_string(), None, results_s));
    timings.push(("results_written".to_string(), None, written_s));
    timings.push(("total".to_string(), None, started.elapsed().as_secs_f64()));
    write_timings(&dir.join("timings.csv"), &timings).map_err(to_value_error)?;

    // What Python reads.
    let out = PyDict::new(py);
    out.set_item("engine", "one-loop")?;
    out.set_item("fingerprint", &print_hex)?;
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
    p.set_item("step_offset", params.step_offset)?;
    p.set_item("cost_source", format!("{:?}", params.cost_source).to_lowercase())?;
    p.set_item("motorway_snap_m", params.motorway_snap_metres)?;
    p.set_item("flow_level", flow_level)?;
    p.set_item("flow_step_s", flow_step_s)?;
    p.set_item("cost_bin_s", cost_bin_s)?;
    p.set_item("link_bin_s", link_bin_s)?;
    p.set_item("path_size_weight", path_size)?;
    out.set_item("parameters", p)?;
    let k = PyDict::new(py);
    for c in 0..ids.count() {
        let v = &classes[c as usize];
        let d = PyDict::new(py);
        d.set_item("in_vehicle_per_minute", v.in_vehicle_per_minute)?;
        d.set_item("walk_per_minute", v.walk_per_minute)?;
        d.set_item("wait_per_minute", v.wait_per_minute)?;
        d.set_item("per_transfer", v.per_transfer)?;
        d.set_item("per_money", v.per_money)?;
        d.set_item("per_kilometre", v.per_kilometre)?;
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
    t.set_item("mode_choice", vec![false; n as usize])?;
    t.set_item("outcome", rows.outcome.clone())?;
    t.set_item("arrival_s", maybe(&rows.arrival_s).into_pyarray(py))?;
    t.set_item("travel_time_s", maybe(&travel).into_pyarray(py))?;
    t.set_item("link_at_end", vec![f64::NAN; n as usize].into_pyarray(py))?;
    t.set_item("vehicle_s", maybe(&travel).into_pyarray(py))?;
    t.set_item(
        "access_egress_s",
        rows.arrival_s
            .iter()
            .map(|a| a.map_or(f64::NAN, |_| 0.0))
            .collect::<Vec<_>>()
            .into_pyarray(py),
    )?;
    out.set_item("trip_results", t)?;

    // The counts of the last loading, by outcome, and its travel time by traveller weight.
    let count = |o: &str| rows.outcome.iter().filter(|x| **x == o).count();
    let (mut total_s, mut people, mut completed_people) = (0.0, 0.0, 0.0);
    for (i, w) in rows.weight.iter().enumerate() {
        let w = f64::from(*w);
        people += w;
        if let Some(s) = travel[i] {
            total_s += w * s;
            completed_people += w;
        }
    }
    out.set_item("total_trips", n)?;
    out.set_item("completed", count("completed"))?;
    out.set_item("truncated", count("truncated"))?;
    out.set_item("no_vehicle_available", count("no_vehicle_available"))?;
    out.set_item("no_feasible_path", count("no_feasible_path"))?;
    out.set_item("mode_not_available", 0)?;
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
