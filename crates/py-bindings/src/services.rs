//! Shared-mobility services' tables in Python: docks, vehicles and chargers.
//!
//! `openmobisim.dock_read_table`, `dock_read_gbfs`, `vehicle_read_table` and
//! `charger_read_table` (in `services.py`) parse their sources and hand the rows here, which
//! checks and keeps them.

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

use numpy::IntoPyArray;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyDict;

use openmobisim_core_graph::geometry::LonLat;
use openmobisim_core_graph::layers::{BikeCost, StaticLayer};
use openmobisim_core_graph::link_geometry::NetworkFingerprint;
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_routes::SearchContext;
use openmobisim_core_sim::LayerSetup;
use openmobisim_core_sim::services::engine::{FleetDay, FleetEngine, Leg};
use openmobisim_core_sim::services::frozen::{FrozenLegs, TripRow, plan_legs};
use openmobisim_core_sim::services::roads::{FrozenPaths, FrozenRoads, RoadTimes};
use openmobisim_core_sim::services::{
    ChargerRow, DockDefaults, DockRow, ServiceParts, ServiceSetup, TaxiDefaults, VehicleRow,
};
use openmobisim_core_types::hash::fingerprint_hex;

use crate::network::PyNetwork;
use crate::policy::{Ids, PolicyTuple, decision_columns, description_to_py, policy_from_py};

fn place(what: &str, id: &str, lon: f64, lat: f64) -> PyResult<LonLat> {
    if lon.is_finite()
        && lat.is_finite()
        && (-180.0..=180.0).contains(&lon)
        && (-90.0..=90.0).contains(&lat)
    {
        Ok(LonLat::new(lon, lat))
    } else {
        Err(PyValueError::new_err(format!("{what} {id:?}: not a place ({lon}, {lat})")))
    }
}

fn unique<'a>(seen: &mut HashSet<String>, what: &str, id: &'a str) -> PyResult<&'a str> {
    if seen.insert(id.to_string()) {
        Ok(id)
    } else {
        Err(PyValueError::new_err(format!("{what}_id {id:?} appears more than once")))
    }
}

/// A bike-sharing system's docks, each with a place and its slots, and the bikes docked when
/// the day starts if known.
///
/// Read them with ``openmobisim.dock_read_table`` or ``openmobisim.dock_read_gbfs``.
#[pyclass(name = "Docks", module = "openmobisim._core", frozen)]
pub struct PyDocks {
    pub(crate) rows: Vec<DockRow>,
}

#[pymethods]
impl PyDocks {
    /// How many docks.
    #[getter]
    fn count(&self) -> usize {
        self.rows.len()
    }

    /// Every dock: ``{"dock_id", "name", "hub_id", "lon", "lat", "capacity", "bikes"}``, by dock
    /// (``bikes`` ``None`` where the table gave none).
    fn rows<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let d = PyDict::new(py);
        let r = &self.rows;
        d.set_item("dock_id", r.iter().map(|x| x.dock_id.clone()).collect::<Vec<_>>())?;
        d.set_item("name", r.iter().map(|x| x.name.clone()).collect::<Vec<_>>())?;
        d.set_item("hub_id", r.iter().map(|x| x.hub_id.clone()).collect::<Vec<_>>())?;
        d.set_item("lon", r.iter().map(|x| x.position.lon).collect::<Vec<_>>().into_pyarray(py))?;
        d.set_item("lat", r.iter().map(|x| x.position.lat).collect::<Vec<_>>().into_pyarray(py))?;
        d.set_item("capacity", r.iter().map(|x| x.capacity).collect::<Vec<_>>().into_pyarray(py))?;
        d.set_item("bikes", r.iter().map(|x| x.bikes).collect::<Vec<_>>())?;
        Ok(d)
    }

    fn __repr__(&self) -> String {
        let slots: u64 = self.rows.iter().map(|r| u64::from(r.capacity)).sum();
        format!("Docks({} docks, {slots} slots)", self.rows.len())
    }
}

/// One dock row as `services.py` hands it: `(dock_id, lon, lat, capacity, name, hub_id, bikes)`.
type DockTuple = (String, f64, f64, u32, Option<String>, Option<String>, Option<u32>);

/// Docks from parsed rows (`openmobisim.dock_read_table` parses the table; this checks it).
#[pyfunction(name = "_docks_from_rows")]
pub fn docks_from_rows(rows: Vec<DockTuple>) -> PyResult<PyDocks> {
    let mut seen = HashSet::new();
    let mut out = Vec::with_capacity(rows.len());
    for (dock_id, lon, lat, capacity, name, hub_id, bikes) in rows {
        unique(&mut seen, "dock", &dock_id)?;
        let position = place("dock", &dock_id, lon, lat)?;
        if bikes.is_some_and(|b| b > capacity) {
            return Err(PyValueError::new_err(format!(
                "dock {dock_id:?}: {} bikes in {capacity} slots",
                bikes.unwrap_or_default()
            )));
        }
        out.push(DockRow { dock_id, name, hub_id, position, capacity, bikes });
    }
    Ok(PyDocks { rows: out })
}

/// A fleet's vehicles, each with its place and, if known, its charge when the day starts.
///
/// Read them with ``openmobisim.vehicle_read_table``.
#[pyclass(name = "Vehicles", module = "openmobisim._core", frozen)]
pub struct PyVehicles {
    pub(crate) rows: Vec<VehicleRow>,
}

#[pymethods]
impl PyVehicles {
    /// How many vehicles.
    #[getter]
    fn count(&self) -> usize {
        self.rows.len()
    }

    /// Every vehicle: ``{"vehicle_id", "lon", "lat", "soc"}`` (``soc`` ``None`` where the table
    /// gave none).
    fn rows<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let d = PyDict::new(py);
        let r = &self.rows;
        d.set_item("vehicle_id", r.iter().map(|x| x.vehicle_id.clone()).collect::<Vec<_>>())?;
        d.set_item("lon", r.iter().map(|x| x.position.lon).collect::<Vec<_>>().into_pyarray(py))?;
        d.set_item("lat", r.iter().map(|x| x.position.lat).collect::<Vec<_>>().into_pyarray(py))?;
        d.set_item("soc", r.iter().map(|x| x.soc).collect::<Vec<_>>())?;
        Ok(d)
    }

    fn __repr__(&self) -> String {
        format!("Vehicles({} vehicles)", self.rows.len())
    }
}

/// Vehicles from parsed rows `(vehicle_id, lon, lat, soc)`.
#[pyfunction(name = "_vehicles_from_rows")]
pub fn vehicles_from_rows(rows: Vec<(String, f64, f64, Option<f64>)>) -> PyResult<PyVehicles> {
    let mut seen = HashSet::new();
    let mut out = Vec::with_capacity(rows.len());
    for (vehicle_id, lon, lat, soc) in rows {
        unique(&mut seen, "vehicle", &vehicle_id)?;
        let position = place("vehicle", &vehicle_id, lon, lat)?;
        if soc.is_some_and(|c| !(0.0..=1.0).contains(&c)) {
            return Err(PyValueError::new_err(format!(
                "vehicle {vehicle_id:?}: soc must be a share from 0 to 1, got {}",
                soc.unwrap_or_default()
            )));
        }
        out.push(VehicleRow { vehicle_id, position, soc });
    }
    Ok(PyVehicles { rows: out })
}

/// Chargers, each with a place, its plugs and, if known, its power per plug.
///
/// Read them with ``openmobisim.charger_read_table``.
#[pyclass(name = "Chargers", module = "openmobisim._core", frozen)]
pub struct PyChargers {
    pub(crate) rows: Vec<ChargerRow>,
}

#[pymethods]
impl PyChargers {
    /// How many chargers.
    #[getter]
    fn count(&self) -> usize {
        self.rows.len()
    }

    /// Every charger: ``{"charger_id", "hub_id", "lon", "lat", "plugs", "kw"}`` (``kw`` ``None``
    /// where the table gave none).
    fn rows<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let d = PyDict::new(py);
        let r = &self.rows;
        d.set_item("charger_id", r.iter().map(|x| x.charger_id.clone()).collect::<Vec<_>>())?;
        d.set_item("hub_id", r.iter().map(|x| x.hub_id.clone()).collect::<Vec<_>>())?;
        d.set_item("lon", r.iter().map(|x| x.position.lon).collect::<Vec<_>>().into_pyarray(py))?;
        d.set_item("lat", r.iter().map(|x| x.position.lat).collect::<Vec<_>>().into_pyarray(py))?;
        d.set_item("plugs", r.iter().map(|x| x.plugs).collect::<Vec<_>>().into_pyarray(py))?;
        d.set_item("kw", r.iter().map(|x| x.kw).collect::<Vec<_>>())?;
        Ok(d)
    }

    fn __repr__(&self) -> String {
        let plugs: u64 = self.rows.iter().map(|r| u64::from(r.plugs)).sum();
        format!("Chargers({} chargers, {plugs} plugs)", self.rows.len())
    }
}

/// One charger row as `services.py` hands it: `(charger_id, lon, lat, plugs, hub_id, kw)`.
type ChargerTuple = (String, f64, f64, u32, Option<String>, Option<f64>);

/// Chargers from parsed rows.
#[pyfunction(name = "_chargers_from_rows")]
pub fn chargers_from_rows(rows: Vec<ChargerTuple>) -> PyResult<PyChargers> {
    let mut seen = HashSet::new();
    let mut out = Vec::with_capacity(rows.len());
    for (charger_id, lon, lat, plugs, hub_id, kw) in rows {
        unique(&mut seen, "charger", &charger_id)?;
        let position = place("charger", &charger_id, lon, lat)?;
        if kw.is_some_and(|k| !(k.is_finite() && k > 0.0)) {
            return Err(PyValueError::new_err(format!(
                "charger {charger_id:?}: kw must be above 0, got {}",
                kw.unwrap_or_default()
            )));
        }
        out.push(ChargerRow { charger_id, hub_id, position, plugs, kw });
    }
    Ok(PyChargers { rows: out })
}

// --- A fleet day without a run ----------------------------------------------------------------

/// One service as `fleet.py` hands it: `(kind, name, docks, vehicles, chargers, options)`.
type ServiceTuple<'py> = (
    String,
    String,
    Option<Bound<'py, PyDocks>>,
    Option<Bound<'py, PyVehicles>>,
    Option<Bound<'py, PyChargers>>,
    BTreeMap<String, f64>,
);

/// One trip: `(trip_id, service index, weight, departure_s, origin_lon, origin_lat,
/// destination_lon, destination_lat)`.
type TripTuple = (String, u16, u32, f64, f64, f64, f64, f64);

/// Road times: `(bin_s, rows of (link index, bin, seconds), network fingerprint)`.
type RoadTuple = (f64, Vec<(u32, u32, f64)>, Option<String>);

/// Run one fleet day on `network` (`openmobisim.fleet_day` checks and documents the inputs).
#[pyfunction(name = "_fleet_day")]
#[allow(
    clippy::too_many_lines,
    clippy::too_many_arguments,
    reason = "the inputs built, the day run, the tables written, in order; the arguments are \
              fleet_day's, checked in fleet.py"
)]
pub fn fleet_day<'py>(
    py: Python<'py>,
    network: &PyNetwork,
    services: Vec<ServiceTuple<'py>>,
    trips: Vec<TripTuple>,
    road_times: Option<RoadTuple>,
    window: (f64, f64),
    seed: u64,
    policy: Option<PolicyTuple<'py>>,
) -> PyResult<Bound<'py, PyDict>> {
    let started = Instant::now();
    let net = network.inner.clone();
    let layer = |l: StaticLayer| -> PyResult<LayerSetup> {
        Ok(LayerSetup::new(network.static_layer(l)?, BikeCost::Utility, net.defaults().layers))
    };
    let (walk, bike) = (layer(StaticLayer::Walk)?, layer(StaticLayer::Bike)?);
    let bad = |e: String| PyValueError::new_err(e);
    let mut setups = Vec::with_capacity(services.len());
    for (kind, name, docks, vehicles, chargers, options) in &services {
        let setup = match kind.as_str() {
            "bike_docked" => {
                let docks = docks.as_ref().ok_or_else(|| bad(format!("{name}: no docks")))?;
                let d = DockDefaults::from_options(options).map_err(|e| bad(e.to_string()))?;
                ServiceSetup::bike_docked(name, &docks.get().rows, &walk, &bike, Some(&net), d)
            }
            "taxi" => {
                let vehicles =
                    vehicles.as_ref().ok_or_else(|| bad(format!("{name}: no vehicles")))?;
                let d = TaxiDefaults::from_options(options).map_err(|e| bad(e.to_string()))?;
                let chargers = chargers.as_ref().map_or(&[][..], |c| &c.get().rows[..]);
                ServiceSetup::taxi(name, &net, &vehicles.get().rows, chargers, d)
            }
            other => return Err(bad(format!("{name}: unknown service kind {other:?}"))),
        };
        setups.push(setup.map_err(|e| bad(e.to_string()))?);
    }
    let times = match road_times {
        None => RoadTimes::free_flow(&net),
        Some((bin_s, rows, fingerprint)) => {
            let mine = fingerprint_hex(NetworkFingerprint::of(&net).value());
            if let Some(theirs) = fingerprint.filter(|f| *f != mine) {
                return Err(bad(format!(
                    "the road times are of network {theirs}, not this one ({mine})"
                )));
            }
            RoadTimes::from_rows(&net, bin_s, &rows).map_err(bad)?
        }
    };
    let turns = TurnTable::build(&net, net.signals());
    let ctx = SearchContext::new(&net, &turns);
    let roads = FrozenRoads::new(&net, &ctx, times);
    let paths = FrozenPaths::new(bike.network(), bike.seconds(), walk.network(), walk.seconds());
    let rows: Vec<TripRow> = trips
        .iter()
        .enumerate()
        .map(|(i, t)| TripRow {
            trip: u32::try_from(i).expect("trips fit u32"),
            service: t.1,
            weight: t.2,
            departure_s: t.3,
            origin: LonLat::new(t.4, t.5),
            destination: LonLat::new(t.6, t.7),
        })
        .collect();
    if let Some(t) = rows.iter().find(|t| usize::from(t.service) >= setups.len()) {
        return Err(bad(format!("trip {} names service {}", t.trip, t.service)));
    }
    let (planned, report) = plan_legs(&setups, &rows, &net, &walk, &paths);
    let legs: Vec<Leg> = planned.iter().map(|&(_, l)| l).collect();
    let setup_s = started.elapsed().as_secs_f64();
    let clock = Instant::now();
    let day = Arc::new(FleetDay { window, legs: legs.clone(), seed });
    let world = FrozenLegs::new(&setups, &paths, &legs);
    let ids = Arc::new(Ids::new(&setups, Arc::clone(&net)));
    let (policy, raised) = policy_from_py(policy, Arc::clone(&ids))?;
    let engine =
        FleetEngine::new(Arc::new(setups.clone()), day, Arc::new(world), Arc::new(roads), policy);
    let description = engine.policy_description();
    let out = engine.run_day().map_err(|msg| {
        // A Python policy's own exception is raised as it was; any other failure as a
        // ValueError with the engine's message.
        raised
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
            .unwrap_or_else(|| PyValueError::new_err(msg))
    })?;
    let day_s = clock.elapsed().as_secs_f64();

    // Per trip given: its leg's outcome, or "not_planned".
    let n = trips.len();
    let mut leg_of = vec![None; n];
    for (k, &(i, _)) in planned.iter().enumerate() {
        leg_of[i] = Some(k);
    }
    let nan = f64::NAN;
    let mut status = Vec::with_capacity(n);
    let (mut requested, mut picked_s, mut arrived, mut extra, mut waited) = (
        Vec::with_capacity(n),
        Vec::with_capacity(n),
        Vec::with_capacity(n),
        Vec::with_capacity(n),
        Vec::with_capacity(n),
    );
    let (mut from, mut to, mut vehicle) =
        (Vec::with_capacity(n), Vec::with_capacity(n), Vec::with_capacity(n));
    let (mut redirections, mut overflow) = (Vec::with_capacity(n), Vec::with_capacity(n));
    for (i, t) in trips.iter().enumerate() {
        let Some(k) = leg_of[i] else {
            status.push("not_planned");
            requested.push(nan);
            picked_s.push(nan);
            arrived.push(nan);
            extra.push(0.0);
            waited.push(nan);
            from.push(None);
            to.push(None);
            vehicle.push(None);
            redirections.push(0u32);
            overflow.push(false);
            continue;
        };
        let o = &out.legs[k];
        let setup = &setups[usize::from(t.1)];
        let dock_id = |d: Option<u32>| match (setup.parts(), d) {
            (ServiceParts::BikeDocked(docks, _), Some(d)) => {
                Some(docks.external[d as usize].clone())
            }
            _ => None,
        };
        status.push(o.status.as_str());
        requested.push(o.requested_s);
        picked_s.push(o.picked_s);
        arrived.push(o.arrived_s);
        extra.push(o.extra_walk_s);
        waited.push(o.waited_s);
        from.push(dock_id(o.picked));
        to.push(dock_id(o.returned));
        vehicle.push(match (setup.parts(), o.vehicle) {
            (ServiceParts::Taxi(fleet, _), Some(v)) => Some(fleet.vehicle_ids[v as usize].clone()),
            _ => None,
        });
        redirections.push(o.redirections);
        overflow.push(o.overflow);
    }
    let names: Vec<&str> = services.iter().map(|s| s.1.as_str()).collect();
    let cols = PyDict::new(py);
    cols.set_item("trip_id", trips.iter().map(|t| t.0.clone()).collect::<Vec<_>>())?;
    cols.set_item("service", trips.iter().map(|t| names[usize::from(t.1)]).collect::<Vec<_>>())?;
    cols.set_item("status", status)?;
    cols.set_item("requested_s", requested.into_pyarray(py))?;
    cols.set_item("picked_s", picked_s.into_pyarray(py))?;
    cols.set_item("arrived_s", arrived.into_pyarray(py))?;
    cols.set_item("waited_s", waited.into_pyarray(py))?;
    cols.set_item("dock_from", from)?;
    cols.set_item("dock_to", to)?;
    cols.set_item("vehicle_id", vehicle)?;
    cols.set_item("extra_walk_s", extra.into_pyarray(py))?;
    cols.set_item("redirections", redirections.into_pyarray(py))?;
    cols.set_item("overflow", overflow)?;

    let dock_bins = PyDict::new(py);
    for (s, bins) in out.docks.iter().enumerate() {
        let (Some(b), ServiceParts::BikeDocked(docks, _)) = (bins, setups[s].parts()) else {
            continue;
        };
        let d = PyDict::new(py);
        let nd = docks.external.len();
        d.set_item(
            "dock_id",
            (0..nd)
                .flat_map(|k| std::iter::repeat_n(docks.external[k].clone(), b.bins))
                .collect::<Vec<_>>(),
        )?;
        #[allow(clippy::cast_precision_loss, reason = "a bin index")]
        let starts: Vec<f64> = (0..nd)
            .flat_map(|_| (0..b.bins).map(|k| b.start_s + k as f64 * f64::from(b.bin_s)))
            .collect();
        d.set_item("start_s", starts.into_pyarray(py))?;
        for (name, v) in [
            ("bikes_mean", &b.bikes_mean),
            ("bikes_min", &b.bikes_min),
            ("empty_s", &b.empty_s),
            ("full_s", &b.full_s),
            ("pickups", &b.pickups),
            ("returns", &b.returns),
            ("failed", &b.failed),
            ("redirected", &b.redirected),
        ] {
            d.set_item(name, v.clone().into_pyarray(py))?;
        }
        dock_bins.set_item(names[s], d)?;
    }
    let plan = PyDict::new(py);
    plan.set_item("trips", report.trips)?;
    plan.set_item("legs", report.legs)?;
    plan.set_item("no_dock", report.no_dock)?;
    plan.set_item("same_dock", report.same_dock)?;
    plan.set_item("no_ride", report.no_ride)?;
    let counts = PyDict::new(py);
    counts.set_item("events", out.events)?;
    counts.set_item("invariant_violations", out.invariant_violations)?;
    let seconds = PyDict::new(py);
    seconds.set_item("setup_s", setup_s)?;
    seconds.set_item("day_s", day_s)?;
    let result = PyDict::new(py);
    result.set_item("trips", cols)?;
    result.set_item("dock_bins", dock_bins)?;
    result.set_item("plan", plan)?;
    result.set_item("counts", counts)?;
    result.set_item("seconds", seconds)?;
    result.set_item("decisions", decision_columns(py, &out.decisions, &ids)?)?;
    result.set_item("policy", description_to_py(py, description.as_ref())?)?;
    Ok(result)
}
