//! A run's services' results (S302): `service_legs.parquet` and `hub_bins.parquet`, and the same
//! tables for `Run.service_legs()`, `Run.hub_bins()` and `Run.hub_places()`. A run without a
//! service makes none of them.

use std::path::Path;
use std::sync::Arc;

use numpy::IntoPyArray;
use pyo3::prelude::*;
use pyo3::types::PyDict;

use openmobisim_core_demand::{Travellers, Trips};
use openmobisim_core_sim::services::hubs::RunHubs;
use openmobisim_core_sim::services::in_run::{ServicesRun, idle};
use openmobisim_core_sim::services::tables::{HubBinTable, ServiceLegTable, resource_hubs};
use openmobisim_core_sim::services::{ServiceParts, ServiceSetup};
use openmobisim_core_sim::{RunDescription, TripTable};

use crate::pipeline::to_value_error;

/// What a run with a service returns of it.
pub(crate) struct ServiceResults {
    pub(crate) legs: Py<PyDict>,
    pub(crate) bins: Py<PyDict>,
    pub(crate) places: Py<PyDict>,
    pub(crate) legs_path: Option<String>,
    pub(crate) bins_path: Option<String>,
}

/// What the run that had `setups` (its hub set `hubs`, its `services` result) did with them over
/// `window`: the tables, and the files in `dir` unless `minimal`.
#[allow(clippy::too_many_arguments, reason = "what the tables are made of, named")]
pub(crate) fn service_results(
    py: Python<'_>,
    setups: &Arc<Vec<ServiceSetup>>,
    hubs: &RunHubs,
    services: Option<&ServicesRun>,
    (trips, travellers, trip_table): (&Trips, &Travellers, &TripTable),
    window: (f64, f64),
    (dir, run_id, description, minimal): (&Path, &str, &RunDescription, bool),
) -> PyResult<ServiceResults> {
    let at = resource_hubs(setups, hubs);
    let day = services.and_then(|s| s.last.as_ref());
    let legs = day.map_or_else(ServiceLegTable::default, |d| {
        ServiceLegTable::new(d, setups, &at, trips, travellers, &trip_table.mode)
    });
    let bins = match day {
        Some(d) => HubBinTable::new(&d.outcome, setups, &at),
        None => HubBinTable::new(&idle(setups, window), setups, &at),
    };
    let (mut legs_path, mut bins_path) = (None, None);
    if !minimal {
        let path = dir.join("service_legs.parquet");
        openmobisim_io_parquet::write_service_legs(&path, run_id, &legs, description)
            .map_err(to_value_error)?;
        legs_path = Some(path.to_string_lossy().into_owned());
        let path = dir.join("hub_bins.parquet");
        openmobisim_io_parquet::write_hub_bins(&path, run_id, &bins, description)
            .map_err(to_value_error)?;
        bins_path = Some(path.to_string_lossy().into_owned());
    }
    Ok(ServiceResults {
        legs: legs_dict(py, &legs)?.unbind(),
        bins: bins_dict(py, &bins)?.unbind(),
        places: places_dict(py, setups, &at)?.unbind(),
        legs_path,
        bins_path,
    })
}

fn nan(v: &[Option<f64>]) -> Vec<f64> {
    v.iter().map(|x| x.unwrap_or(f64::NAN)).collect()
}

/// `service_legs.parquet`'s columns after `run_id`; a time that did not come is `NaN`, a dock
/// not used `None`.
fn legs_dict<'py>(py: Python<'py>, t: &ServiceLegTable) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    d.set_item("service", t.service.clone())?;
    d.set_item("mode", t.mode.clone())?;
    d.set_item("traveller_id", t.traveller_id.clone())?;
    d.set_item("trip_seq", t.trip_seq.clone().into_pyarray(py))?;
    d.set_item("weight", t.weight.clone().into_pyarray(py))?;
    d.set_item("requested_s", nan(&t.requested_s).into_pyarray(py))?;
    d.set_item("picked_s", nan(&t.picked_s).into_pyarray(py))?;
    d.set_item("arrived_s", nan(&t.arrived_s).into_pyarray(py))?;
    d.set_item("hub_from", t.hub_from.clone())?;
    d.set_item("hub_to", t.hub_to.clone())?;
    d.set_item("dock_from", t.dock_from.clone())?;
    d.set_item("dock_to", t.dock_to.clone())?;
    d.set_item("waited_s", t.waited_s.clone().into_pyarray(py))?;
    d.set_item("extra_walk_s", t.extra_walk_s.clone().into_pyarray(py))?;
    d.set_item("redirections", t.redirections.clone().into_pyarray(py))?;
    d.set_item("expected_dock_s", t.expected_dock_s.clone().into_pyarray(py))?;
    d.set_item("met_dock_s", nan(&t.met_dock_s).into_pyarray(py))?;
    d.set_item("outcome", t.outcome.clone())?;
    Ok(d)
}

/// `hub_bins.parquet`'s columns after `run_id`.
fn bins_dict<'py>(py: Python<'py>, t: &HubBinTable) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    let bin_s = t.bin_s.max(1);
    d.set_item("bin", t.start_s.iter().map(|s| s / bin_s).collect::<Vec<_>>().into_pyarray(py))?;
    d.set_item("start_s", t.start_s.clone().into_pyarray(py))?;
    d.set_item("hub_id", t.hub_id.clone())?;
    d.set_item("hub_kind", t.hub_kind.clone())?;
    d.set_item("resource_kind", t.resource_kind.clone())?;
    d.set_item("resource_id", t.resource_id.clone())?;
    d.set_item("service", t.service.clone())?;
    for (name, v) in [("level_mean", &t.level_mean), ("level_min", &t.level_min)] {
        d.set_item(name, v.clone().into_pyarray(py))?;
    }
    d.set_item("capacity", t.capacity.clone().into_pyarray(py))?;
    for (name, v) in [
        ("empty_s", &t.empty_s),
        ("full_s", &t.full_s),
        ("pickups", &t.pickups),
        ("returns", &t.returns),
        ("failed", &t.failed),
        ("redirected_in", &t.redirected_in),
        ("redirected_out", &t.redirected_out),
        ("moved_in", &t.moved_in),
        ("moved_out", &t.moved_out),
    ] {
        d.set_item(name, v.clone().into_pyarray(py))?;
    }
    Ok(d)
}

/// Each dock of the run's services, for maps: `resource_id, hub_id, hub_kind, service, capacity,
/// bikes_start, lon, lat`.
fn places_dict<'py>(
    py: Python<'py>,
    setups: &[ServiceSetup],
    at: &[Vec<(String, &'static str)>],
) -> PyResult<Bound<'py, PyDict>> {
    let (mut id, mut hub, mut kind, mut service) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let (mut capacity, mut start, mut lon, mut lat) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for (s, setup) in setups.iter().enumerate() {
        let ServiceParts::BikeDocked(docks, _) = setup.parts() else { continue };
        for (k, (h, hk)) in at[s].iter().enumerate().take(docks.len()) {
            id.push(docks.external[k].clone());
            hub.push(h.clone());
            kind.push(*hk);
            service.push(setup.name().to_string());
            capacity.push(docks.capacity[k]);
            start.push(docks.start[k]);
            lon.push(docks.position[k].lon);
            lat.push(docks.position[k].lat);
        }
    }
    let d = PyDict::new(py);
    d.set_item("resource_id", id)?;
    d.set_item("hub_id", hub)?;
    d.set_item("hub_kind", kind)?;
    d.set_item("service", service)?;
    d.set_item("capacity", capacity.into_pyarray(py))?;
    d.set_item("bikes_start", start.into_pyarray(py))?;
    d.set_item("lon", lon.into_pyarray(py))?;
    d.set_item("lat", lat.into_pyarray(py))?;
    Ok(d)
}
