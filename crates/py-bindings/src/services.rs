//! Shared-mobility services' tables in Python: docks, vehicles and chargers.
//!
//! `openmobisim.dock_read_table`, `dock_read_gbfs`, `vehicle_read_table` and
//! `charger_read_table` (in `services.py`) parse their sources and hand the rows here, which
//! checks and keeps them.

use std::collections::HashSet;

use numpy::IntoPyArray;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyDict;

use openmobisim_core_graph::geometry::LonLat;
use openmobisim_core_sim::services::{ChargerRow, DockRow, VehicleRow};

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
