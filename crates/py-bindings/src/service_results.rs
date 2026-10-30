//! A run's docked bike service's results (S302): `bike_share_events.parquet`,
//! `bike_share_trips.parquet` and `bike_share_docks.parquet`, and the same tables for
//! `Run.bike_share_events()`, `Run.bike_share_trips()`, `Run.bike_share_docks()`, with
//! `Run.bike_share_dock_places()` for maps. A run without a service makes none of them. A run's
//! taxi fleet (S302) likewise: `taxi_events.parquet`, `taxi_trips.parquet` and
//! `taxi_vehicles.parquet`, and `Run.taxi_events()`, `Run.taxi_trips()`, `Run.taxi_vehicles()`.

use std::path::Path;
use std::sync::Arc;

use numpy::IntoPyArray;
use pyo3::prelude::*;
use pyo3::types::PyDict;

use openmobisim_core_demand::{Travellers, Trips};
use openmobisim_core_sim::services::engine::VehicleState;
use openmobisim_core_sim::services::hubs::RunHubs;
use openmobisim_core_sim::services::in_run::{ServicesRun, TaxisRun, idle};
use openmobisim_core_sim::services::tables::{
    BikeShareDockTable, BikeShareEventTable, BikeShareTripTable, clock, resource_hubs,
};
use openmobisim_core_sim::services::taxi_tables::{
    TaxiEventTable, TaxiTripTable, TaxiVehicleTable,
};
use openmobisim_core_sim::services::{ServiceParts, ServiceSetup};
use openmobisim_core_sim::{RunDescription, TripTable};

use crate::pipeline::to_value_error;
use openmobisim_io_parquet::taxi::seconds_column;

/// What a run with a service returns of it: each table as columns, and its file's path.
pub(crate) struct ServiceResults {
    pub(crate) events: Py<PyDict>,
    pub(crate) trips: Py<PyDict>,
    pub(crate) docks: Py<PyDict>,
    pub(crate) places: Py<PyDict>,
    pub(crate) events_path: Option<String>,
    pub(crate) trips_path: Option<String>,
    pub(crate) docks_path: Option<String>,
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
    let events = day.map_or_else(BikeShareEventTable::default, |d| {
        BikeShareEventTable::new(d, setups, (trips, travellers, &trip_table.mode), &at)
    });
    let trip_rows = day.map_or_else(BikeShareTripTable::default, |d| {
        BikeShareTripTable::new(d, setups, trips, travellers, &trip_table.mode)
    });
    let docks = match day {
        Some(d) => BikeShareDockTable::new(&d.outcome, setups, &at),
        None => BikeShareDockTable::new(&idle(setups, window), setups, &at),
    };
    let (mut events_path, mut trips_path, mut docks_path) = (None, None, None);
    if !minimal {
        let path = dir.join("bike_share_events.parquet");
        openmobisim_io_parquet::write_bike_share_events(&path, run_id, &events, description)
            .map_err(to_value_error)?;
        events_path = Some(path.to_string_lossy().into_owned());
        let path = dir.join("bike_share_trips.parquet");
        openmobisim_io_parquet::write_bike_share_trips(&path, run_id, &trip_rows, description)
            .map_err(to_value_error)?;
        trips_path = Some(path.to_string_lossy().into_owned());
        let path = dir.join("bike_share_docks.parquet");
        openmobisim_io_parquet::write_bike_share_docks(&path, run_id, &docks, description)
            .map_err(to_value_error)?;
        docks_path = Some(path.to_string_lossy().into_owned());
    }
    Ok(ServiceResults {
        events: events_dict(py, &events)?.unbind(),
        trips: trips_dict(py, &trip_rows)?.unbind(),
        docks: docks_dict(py, &docks)?.unbind(),
        places: places_dict(py, setups, &at)?.unbind(),
        events_path,
        trips_path,
        docks_path,
    })
}

fn nan(v: &[Option<f64>]) -> Vec<f64> {
    v.iter().map(|x| x.unwrap_or(f64::NAN)).collect()
}

/// `bike_share_events.parquet`'s columns after `run_id`; a number that does not apply is
/// `NaN` (a count: `None` in a list), a text `None`.
fn events_dict<'py>(py: Python<'py>, t: &BikeShareEventTable) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    d.set_item("service", t.service.clone())?;
    d.set_item("time_s", t.time_s.clone().into_pyarray(py))?;
    d.set_item("time", t.time_s.iter().map(|&s| clock(s)).collect::<Vec<_>>())?;
    d.set_item("traveller_id", t.traveller_id.clone())?;
    d.set_item("trip_seq", t.trip_seq.clone())?;
    d.set_item("travellers", t.travellers.clone().into_pyarray(py))?;
    d.set_item("event", t.event.clone())?;
    d.set_item("dock", t.dock.clone())?;
    d.set_item("bikes_at_dock_after", t.bikes_at_dock_after.clone())?;
    d.set_item("free_slots_after", t.free_slots_after.clone())?;
    d.set_item("next_dock", t.next_dock.clone())?;
    d.set_item("to_next_m", nan(&t.to_next_m).into_pyarray(py))?;
    d.set_item("transit_stop", t.transit_stop.clone())?;
    Ok(d)
}

/// `bike_share_trips.parquet`'s columns after `run_id`; what did not happen is `NaN` (a time)
/// or `None` (a dock).
fn trips_dict<'py>(py: Python<'py>, t: &BikeShareTripTable) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    d.set_item("service", t.service.clone())?;
    d.set_item("traveller_id", t.traveller_id.clone())?;
    d.set_item("trip_seq", t.trip_seq.clone().into_pyarray(py))?;
    d.set_item("travellers", t.travellers.clone().into_pyarray(py))?;
    d.set_item("mode", t.mode.clone())?;
    d.set_item("planned_pickup_dock", t.planned_pickup_dock.clone())?;
    d.set_item("planned_return_dock", t.planned_return_dock.clone())?;
    d.set_item("planned_at_pickup_dock_s", t.planned_at_pickup_dock_s.clone().into_pyarray(py))?;
    d.set_item("planned_leg_end_s", t.planned_leg_end_s.clone().into_pyarray(py))?;
    d.set_item("pickup_dock", t.pickup_dock.clone())?;
    d.set_item("pickup_s", nan(&t.pickup_s).into_pyarray(py))?;
    d.set_item("docks_tried_for_bike", t.docks_tried_for_bike.clone().into_pyarray(py))?;
    d.set_item("return_dock", t.return_dock.clone())?;
    d.set_item("return_s", nan(&t.return_s).into_pyarray(py))?;
    d.set_item("docks_tried_for_slot", t.docks_tried_for_slot.clone().into_pyarray(py))?;
    d.set_item("leg_end_s", nan(&t.leg_end_s).into_pyarray(py))?;
    d.set_item("extra_s", nan(&t.extra_s).into_pyarray(py))?;
    d.set_item("extra_walk_at_pickup_s", nan(&t.extra_walk_at_pickup_s).into_pyarray(py))?;
    d.set_item("extra_walk_at_return_s", nan(&t.extra_walk_at_return_s).into_pyarray(py))?;
    d.set_item("extra_walk_m", t.extra_walk_m.clone().into_pyarray(py))?;
    d.set_item("failed_attempts_for_bike", t.failed_attempts_for_bike.clone().into_pyarray(py))?;
    d.set_item("failed_attempts_for_slot", t.failed_attempts_for_slot.clone().into_pyarray(py))?;
    d.set_item("outcome", t.outcome.clone())?;
    Ok(d)
}

/// `bike_share_docks.parquet`'s columns after `run_id`.
fn docks_dict<'py>(py: Python<'py>, t: &BikeShareDockTable) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    d.set_item("service", t.service.clone())?;
    d.set_item("dock", t.dock.clone())?;
    d.set_item("hub_id", t.hub_id.clone())?;
    d.set_item("bin_start", t.bin_start_s.iter().map(|&s| clock(s)).collect::<Vec<_>>())?;
    d.set_item("bin_end", t.bin_end_s.iter().map(|&s| clock(s)).collect::<Vec<_>>())?;
    d.set_item("bin_start_s", t.bin_start_s.clone().into_pyarray(py))?;
    d.set_item("bin_end_s", t.bin_end_s.clone().into_pyarray(py))?;
    d.set_item("slots", t.slots.clone().into_pyarray(py))?;
    for (name, v) in [
        ("bikes_at_start", &t.bikes_at_start),
        ("bikes_at_end", &t.bikes_at_end),
        ("bikes_min", &t.bikes_min),
        ("bikes_mean", &t.bikes_mean),
        ("seconds_empty", &t.seconds_empty),
        ("seconds_full", &t.seconds_full),
        ("bikes_taken", &t.bikes_taken),
        ("bikes_returned", &t.bikes_returned),
        ("people_failed_attempt_no_bike", &t.people_failed_attempt_no_bike),
        ("people_gave_up_no_bike", &t.people_gave_up_no_bike),
        ("people_failed_attempt_no_slot", &t.people_failed_attempt_no_slot),
        ("people_left_bike_over_capacity", &t.people_left_bike_over_capacity),
        ("people_sent_here", &t.people_sent_here),
        ("bikes_brought_by_van", &t.bikes_brought_by_van),
        ("bikes_taken_by_van", &t.bikes_taken_by_van),
    ] {
        d.set_item(name, v.clone().into_pyarray(py))?;
    }
    Ok(d)
}

/// Each dock of the run's services, for maps: `dock, hub_id, hub_kind, service, slots,
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
    d.set_item("dock", id)?;
    d.set_item("hub_id", hub)?;
    d.set_item("hub_kind", kind)?;
    d.set_item("service", service)?;
    d.set_item("slots", capacity.into_pyarray(py))?;
    d.set_item("bikes_start", start.into_pyarray(py))?;
    d.set_item("lon", lon.into_pyarray(py))?;
    d.set_item("lat", lat.into_pyarray(py))?;
    Ok(d)
}

/// What a run with a taxi fleet returns of it (S302): each table as columns, and its file's path.
pub(crate) struct TaxiResults {
    pub(crate) events: Py<PyDict>,
    pub(crate) trips: Py<PyDict>,
    pub(crate) vehicles: Py<PyDict>,
    pub(crate) events_path: Option<String>,
    pub(crate) trips_path: Option<String>,
    pub(crate) vehicles_path: Option<String>,
}

/// What the run's taxi fleet `setup` did (its `taxis` result) over `window`: the tables, and the
/// files in `dir` unless `minimal`.
pub(crate) fn taxi_results(
    py: Python<'_>,
    setup: &ServiceSetup,
    taxis: &TaxisRun,
    (trips, travellers): (&Trips, &Travellers),
    window: (f64, f64),
    (dir, run_id, description, minimal): (&Path, &str, &RunDescription, bool),
) -> PyResult<TaxiResults> {
    let day = taxis.last.as_ref();
    let events = day
        .map_or_else(TaxiEventTable::default, |d| TaxiEventTable::new(d, setup, trips, travellers));
    let trip_rows = day
        .map_or_else(TaxiTripTable::default, |d| TaxiTripTable::new(d, setup, trips, travellers));
    let vehicles = TaxiVehicleTable::new(day, setup, window);
    let (mut events_path, mut trips_path, mut vehicles_path) = (None, None, None);
    if !minimal {
        let path = dir.join("taxi_events.parquet");
        openmobisim_io_parquet::write_taxi_events(&path, run_id, &events, description)
            .map_err(to_value_error)?;
        events_path = Some(path.to_string_lossy().into_owned());
        let path = dir.join("taxi_trips.parquet");
        openmobisim_io_parquet::write_taxi_trips(&path, run_id, &trip_rows, description)
            .map_err(to_value_error)?;
        trips_path = Some(path.to_string_lossy().into_owned());
        let path = dir.join("taxi_vehicles.parquet");
        openmobisim_io_parquet::write_taxi_vehicles(&path, run_id, &vehicles, description)
            .map_err(to_value_error)?;
        vehicles_path = Some(path.to_string_lossy().into_owned());
    }
    Ok(TaxiResults {
        events: taxi_events_dict(py, &events)?.unbind(),
        trips: taxi_trips_dict(py, &trip_rows)?.unbind(),
        vehicles: taxi_vehicles_dict(py, &vehicles)?.unbind(),
        events_path,
        trips_path,
        vehicles_path,
    })
}

/// `taxi_events.parquet`'s columns after `run_id`; a number that does not apply is `NaN` (a
/// count: `None` in a list), a text `None`.
fn taxi_events_dict<'py>(py: Python<'py>, t: &TaxiEventTable) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    d.set_item("service", t.service.clone())?;
    d.set_item("time_s", t.time_s.clone().into_pyarray(py))?;
    d.set_item("time", t.time_s.iter().map(|&s| clock(s)).collect::<Vec<_>>())?;
    d.set_item("event", t.event.clone())?;
    d.set_item("traveller_id", t.traveller_id.clone())?;
    d.set_item("trip_seq", t.trip_seq.clone())?;
    d.set_item("travellers", t.travellers.clone())?;
    d.set_item("vehicle", t.vehicle.clone())?;
    d.set_item("vehicle_state_after", t.vehicle_state_after.clone())?;
    d.set_item("charge_after", nan(&t.charge_after).into_pyarray(py))?;
    d.set_item("charger", t.charger.clone())?;
    d.set_item("drive_km", nan(&t.drive_km).into_pyarray(py))?;
    Ok(d)
}

/// `taxi_trips.parquet`'s columns after `run_id`; what did not happen is `NaN` (a number) or
/// `None` (a vehicle).
fn taxi_trips_dict<'py>(py: Python<'py>, t: &TaxiTripTable) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    d.set_item("service", t.service.clone())?;
    d.set_item("traveller_id", t.traveller_id.clone())?;
    d.set_item("trip_seq", t.trip_seq.clone().into_pyarray(py))?;
    d.set_item("travellers", t.travellers.clone().into_pyarray(py))?;
    d.set_item("requested_s", t.requested_s.clone().into_pyarray(py))?;
    d.set_item("assumed_wait_s", t.assumed_wait_s.clone().into_pyarray(py))?;
    d.set_item("outcome", t.outcome.clone())?;
    d.set_item("vehicle", t.vehicle.clone())?;
    for (name, v) in [
        ("pickup_s", &t.pickup_s),
        ("dropoff_s", &t.dropoff_s),
        ("waiting_s", &t.waiting_s),
        ("in_vehicle_s", &t.in_vehicle_s),
        ("ride_km", &t.ride_km),
        ("empty_km", &t.empty_km),
    ] {
        d.set_item(name, nan(v).into_pyarray(py))?;
    }
    Ok(d)
}

/// `taxi_vehicles.parquet`'s columns after `run_id`.
fn taxi_vehicles_dict<'py>(py: Python<'py>, t: &TaxiVehicleTable) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    d.set_item("service", t.service.clone())?;
    d.set_item("vehicle", t.vehicle.clone())?;
    d.set_item("bin_start", t.bin_start_s.iter().map(|&s| clock(s)).collect::<Vec<_>>())?;
    d.set_item("bin_end", t.bin_end_s.iter().map(|&s| clock(s)).collect::<Vec<_>>())?;
    d.set_item("bin_start_s", t.bin_start_s.clone().into_pyarray(py))?;
    d.set_item("bin_end_s", t.bin_end_s.clone().into_pyarray(py))?;
    for (state, v) in VehicleState::ALL.iter().zip(&t.seconds) {
        d.set_item(seconds_column(*state), v.clone().into_pyarray(py))?;
    }
    for (name, v) in [
        ("km_occupied", &t.km_occupied),
        ("km_empty", &t.km_empty),
        ("charge_at_start", &t.charge_at_start),
        ("charge_at_end", &t.charge_at_end),
        ("kwh_charged", &t.kwh_charged),
        ("passengers_dropped_off", &t.passengers_dropped_off),
    ] {
        d.set_item(name, v.clone().into_pyarray(py))?;
    }
    Ok(d)
}
