//! Parkings for Python (M4, S201): reading them from OpenStreetMap or a table,
//! the toy network's, and what a run's parkings and itineraries did.

use numpy::IntoPyArray;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyDict;

use openmobisim_core_demand::{Mode, Travellers, Trips};
use openmobisim_core_graph::geometry::LonLat;
use openmobisim_core_graph::hubs::{ParkingKind, ParkingRow};
use openmobisim_core_sim::{ItineraryResult, NO_MODE, NO_PARKING, ParkingResult, ParkingSetup};
use openmobisim_core_types::ids::{EntityId, TripId};
use openmobisim_io_osm::{ParkingReadOptions, ParkingReadReport, PbfSource, read_parkings};

use crate::network::parse_region;

/// A scenario's parkings: park-and-ride car parks and bike parkings, each with
/// a place, a vehicle (``"car"`` or ``"bike"``) and a capacity.
///
/// Read them with ``openmobisim.parking_read_osm`` or
/// ``openmobisim.parking_read_table``; give them to a ``Scenario``
/// (``parkings=``) for its ``"car_transit"`` and ``"bike_transit"`` trips.
#[pyclass(name = "Parkings", module = "openmobisim._core", frozen)]
pub struct PyParkings {
    pub(crate) rows: Vec<ParkingRow>,
    report: Option<ParkingReadReport>,
}

#[pymethods]
impl PyParkings {
    /// How many parkings.
    #[getter]
    fn count(&self) -> usize {
        self.rows.len()
    }

    /// Parkings and spaces by vehicle: ``{"car": (parkings, spaces), "bike": …}``.
    fn by_vehicle<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let d = PyDict::new(py);
        for kind in ParkingKind::ALL {
            let of: Vec<&ParkingRow> = self.rows.iter().filter(|r| r.kind == kind).collect();
            let spaces: u64 = of.iter().map(|r| u64::from(r.capacity)).sum();
            d.set_item(kind.as_str(), (of.len(), spaces))?;
        }
        Ok(d)
    }

    /// Every parking: ``{"parking_id", "name", "hub_id", "lon", "lat", "vehicle",
    /// "capacity", "initial_occupancy"}``, lists and arrays by parking.
    fn rows<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let d = PyDict::new(py);
        d.set_item(
            "parking_id",
            self.rows.iter().map(|r| r.parking_id.clone()).collect::<Vec<_>>(),
        )?;
        d.set_item("name", self.rows.iter().map(|r| r.name.clone()).collect::<Vec<_>>())?;
        d.set_item("hub_id", self.rows.iter().map(|r| r.hub_id.clone()).collect::<Vec<_>>())?;
        d.set_item(
            "lon",
            self.rows.iter().map(|r| r.position.lon).collect::<Vec<_>>().into_pyarray(py),
        )?;
        d.set_item(
            "lat",
            self.rows.iter().map(|r| r.position.lat).collect::<Vec<_>>().into_pyarray(py),
        )?;
        d.set_item("vehicle", self.rows.iter().map(|r| r.kind.as_str()).collect::<Vec<_>>())?;
        d.set_item(
            "capacity",
            self.rows.iter().map(|r| r.capacity).collect::<Vec<_>>().into_pyarray(py),
        )?;
        d.set_item(
            "initial_occupancy",
            self.rows.iter().map(|r| r.initial_occupancy).collect::<Vec<_>>().into_pyarray(py),
        )?;
        Ok(d)
    }

    /// What reading OpenStreetMap found, kept and merged: counts by name
    /// (``None`` for parkings not read from OpenStreetMap).
    fn read_report<'py>(&self, py: Python<'py>) -> PyResult<Option<Bound<'py, PyDict>>> {
        let Some(r) = &self.report else { return Ok(None) };
        let d = PyDict::new(py);
        d.set_item("car_found", r.car_found)?;
        d.set_item("bike_found", r.bike_found)?;
        d.set_item("duplicates_dropped", r.duplicates_dropped)?;
        d.set_item("private_dropped", r.private_dropped)?;
        d.set_item("capacity_tagged", r.capacity_tagged)?;
        d.set_item("capacity_from_area", r.capacity_from_area)?;
        d.set_item("capacity_default", r.capacity_default)?;
        d.set_item("sites_car", r.sites_car)?;
        d.set_item("sites_bike", r.sites_bike)?;
        Ok(Some(d))
    }

    fn __repr__(&self) -> String {
        let car = self.rows.iter().filter(|r| r.kind == ParkingKind::Car).count();
        format!("Parkings({} car, {} bike)", car, self.rows.len() - car)
    }
}

/// Park-and-ride car parks and bike parkings from an OpenStreetMap extract.
///
/// A car park is kept if it is a park-and-ride (``park_ride`` other than ``no``,
/// or a name that says so); every bike parking is kept; private ones are not.
/// A capacity comes from the ``capacity`` tag, else from the area (``area_per_*``),
/// else the default for a point (``capacity_*``). Parkings of one vehicle
/// closer than ``merge_m`` are merged into one site. **These values are
/// defaults, not a calibration.**
///
/// Args:
///     path: The ``.osm.pbf`` extract.
///     region: A study area, as for ``network_read_osm``: ``(west, south, east,
///         north)`` or a list of ``(lon, lat)`` vertices.
///
/// Raises:
///     ValueError: If the file cannot be read or the region is not valid.
#[pyfunction]
#[pyo3(signature = (
    path, region=None, merge_m=100.0, capacity_car=100, capacity_bike=10,
    area_per_car_m2=25.0, area_per_bike_m2=1.5,
))]
pub fn parking_read_osm(
    path: &str,
    region: Option<&Bound<'_, PyAny>>,
    merge_m: f64,
    capacity_car: u32,
    capacity_bike: u32,
    area_per_car_m2: f64,
    area_per_bike_m2: f64,
) -> PyResult<PyParkings> {
    for (name, v) in [
        ("merge_m", merge_m),
        ("area_per_car_m2", area_per_car_m2),
        ("area_per_bike_m2", area_per_bike_m2),
    ] {
        if !(v.is_finite() && v >= 0.0) {
            return Err(PyValueError::new_err(format!(
                "{name} must be a finite number of at least 0, got {v}"
            )));
        }
    }
    let region = region.map(parse_region).transpose()?;
    let options = ParkingReadOptions {
        merge_m,
        capacity_car,
        capacity_bike,
        area_per_car_m2,
        area_per_bike_m2,
    };
    let source = PbfSource::new(path);
    let (rows, report) = read_parkings(&source, region.as_ref(), options)
        .map_err(|e| PyValueError::new_err(e.to_string()))?;
    Ok(PyParkings { rows, report: Some(report) })
}

/// One row of a parking table: ``(parking_id, lon, lat, vehicle, capacity, name,
/// hub_id, initial_occupancy)``.
type Row = (String, f64, f64, String, u32, Option<String>, Option<String>, u32);

/// Parkings from table rows (``openmobisim.parking_read_table`` parses the
/// table; this checks and keeps it).
#[pyfunction(name = "_parking_from_rows")]
pub fn parking_from_rows(rows: Vec<Row>) -> PyResult<PyParkings> {
    let mut out = Vec::with_capacity(rows.len());
    let mut seen = std::collections::HashSet::new();
    for (parking_id, lon, lat, vehicle, capacity, name, hub_id, initial_occupancy) in rows {
        let kind = ParkingKind::from_name(&vehicle).ok_or_else(|| {
            PyValueError::new_err(format!(
                "parking {parking_id:?}: vehicle must be \"car\" or \"bike\", got {vehicle:?}"
            ))
        })?;
        if !(lon.is_finite()
            && lat.is_finite()
            && (-180.0..=180.0).contains(&lon)
            && (-90.0..=90.0).contains(&lat))
        {
            return Err(PyValueError::new_err(format!(
                "parking {parking_id:?}: not a place ({lon}, {lat})"
            )));
        }
        if !seen.insert(parking_id.clone()) {
            return Err(PyValueError::new_err(format!(
                "parking_id {parking_id:?} appears more than once"
            )));
        }
        out.push(ParkingRow {
            parking_id,
            name,
            hub_id,
            position: LonLat::new(lon, lat),
            kind,
            capacity,
            initial_occupancy,
        });
    }
    Ok(PyParkings { rows: out, report: None })
}

/// The toy network's parkings: hub ``H`` at ``D2`` with 6 car and 3 bike
/// spaces, where the tram stop is, and car park ``P2`` at ``R2`` with 20 spaces,
/// 300 m on foot from that stop: the fixture of the hand-derived park-and-ride and
/// bike-and-ride cases.
#[pyfunction]
pub fn toy_network_parkings() -> PyParkings {
    PyParkings { rows: openmobisim_core_graph::examples::toy_network_parkings(), report: None }
}

/// How full each parking was, bin by bin: what `Run.parking_bins` returns.
pub(crate) fn parking_bins<'py>(
    py: Python<'py>,
    setup: &ParkingSetup,
    result: &ParkingResult,
) -> PyResult<Bound<'py, PyDict>> {
    let b = &result.bins;
    let n = setup.count() * b.bins;
    let (mut bin, mut start, mut parking, mut id, mut hub, mut vehicle, mut capacity) = (
        Vec::with_capacity(n),
        Vec::with_capacity(n),
        Vec::with_capacity(n),
        Vec::with_capacity(n),
        Vec::with_capacity(n),
        Vec::with_capacity(n),
        Vec::with_capacity(n),
    );
    for p in 0..setup.count() {
        let p32 = u32::try_from(p).expect("parkings fit u32");
        for k in 0..b.bins {
            let k32 = u32::try_from(k).expect("bins fit u32");
            bin.push(k32);
            start.push(k32 * b.bin_s);
            parking.push(p32);
            id.push(setup.external_id(p32).to_string());
            hub.push(setup.hub_external_id(p32).to_string());
            vehicle.push(setup.kind(p32).as_str());
            capacity.push(setup.capacity(p32));
        }
    }
    let d = PyDict::new(py);
    d.set_item("bin", bin.into_pyarray(py))?;
    d.set_item("start_s", start.into_pyarray(py))?;
    d.set_item("parking", parking.into_pyarray(py))?;
    d.set_item("parking_id", id)?;
    d.set_item("hub_id", hub)?;
    d.set_item("vehicle", vehicle)?;
    d.set_item("capacity", capacity.into_pyarray(py))?;
    d.set_item("arrivals", b.arrivals.clone().into_pyarray(py))?;
    d.set_item("departures", b.departures.clone().into_pyarray(py))?;
    d.set_item("occupancy_mean", b.occupancy_mean.clone().into_pyarray(py))?;
    d.set_item("occupancy_max", b.occupancy_max.clone().into_pyarray(py))?;
    d.set_item("full_s", b.full_s.clone().into_pyarray(py))?;
    d.set_item(
        "overflow_max",
        b.overflow_max.iter().map(|&v| v.max(0.0)).collect::<Vec<_>>().into_pyarray(py),
    )?;
    Ok(d)
}

/// Where each parking is and what it holds, for maps: ``{"parking_id", "hub_id",
/// "vehicle", "capacity", "lon", "lat", "name"}``.
pub(crate) fn parking_places<'py>(
    py: Python<'py>,
    setup: &ParkingSetup,
) -> PyResult<Bound<'py, PyDict>> {
    let n = u32::try_from(setup.count()).expect("parkings fit u32");
    let d = PyDict::new(py);
    d.set_item("parking_id", (0..n).map(|p| setup.external_id(p).to_string()).collect::<Vec<_>>())?;
    d.set_item("hub_id", (0..n).map(|p| setup.hub_external_id(p).to_string()).collect::<Vec<_>>())?;
    d.set_item("vehicle", (0..n).map(|p| setup.kind(p).as_str()).collect::<Vec<_>>())?;
    d.set_item("capacity", (0..n).map(|p| setup.capacity(p)).collect::<Vec<_>>().into_pyarray(py))?;
    d.set_item("lon", (0..n).map(|p| setup.position(p).lon).collect::<Vec<_>>().into_pyarray(py))?;
    d.set_item("lat", (0..n).map(|p| setup.position(p).lat).collect::<Vec<_>>().into_pyarray(py))?;
    d.set_item("name", (0..n).map(|p| setup.name(p).map(str::to_string)).collect::<Vec<_>>())?;
    Ok(d)
}

/// What the parkings did in a run, as numbers by name: `Run.parking_summary`.
pub(crate) fn parking_summary<'py>(
    py: Python<'py>,
    setup: &ParkingSetup,
    result: &ParkingResult,
) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    let r = setup.report();
    d.set_item("parkings_given", r.rows)?;
    d.set_item("parkings_car", r.kept_car)?;
    d.set_item("parkings_bike", r.kept_bike)?;
    d.set_item("parkings_off_layer", r.off_layer)?;
    d.set_item("parkings_no_stop", r.no_stop)?;
    for kind in ParkingKind::ALL {
        let t = &result.by_kind[kind.index()];
        let k = kind.as_str();
        d.set_item(format!("{k}_arrivals"), t.arrivals)?;
        d.set_item(format!("{k}_overflow_arrivals"), t.overflow_arrivals)?;
        d.set_item(format!("{k}_mismatch_s"), t.mismatch_s)?;
        d.set_item(format!("{k}_left_at_end"), t.left_at_end)?;
    }
    Ok(d)
}

/// Each itinerary trip's choice: what `Run.itinerary_choices` returns.
pub(crate) fn itinerary_choices<'py>(
    py: Python<'py>,
    setup: Option<&ParkingSetup>,
    result: &ItineraryResult,
    trips: &Trips,
    travellers: &Travellers,
) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    d.set_item("trip", result.trip.clone().into_pyarray(py))?;
    let (mut who, mut seq, mut mode) = (Vec::new(), Vec::new(), Vec::new());
    for &t in &result.trip {
        let trip = TripId::new(t);
        let traveller = trips.traveller(trip);
        who.push(travellers.external_ids().external(traveller.raw()).to_string());
        let first = travellers.trips_of(traveller).next().map_or(t, |f| f.raw());
        seq.push(t - first);
    }
    // The mode taken: the chosen alternative's (M5), else the stated one; none for a trip
    // that chose and had nothing to choose from.
    for (i, &t) in result.trip.iter().enumerate() {
        mode.push(match (result.mode[i], result.choosing[i]) {
            (NO_MODE, true) => None,
            (NO_MODE, false) => Some(trips.mode(TripId::new(t)).as_str()),
            (m, _) => Some(Mode::ALL[m as usize].as_str()),
        });
    }
    d.set_item("traveller_id", who)?;
    d.set_item("trip_seq", seq.into_pyarray(py))?;
    d.set_item("mode", mode)?;
    d.set_item("mode_choice", result.choosing.clone())?;
    let parking_id: Vec<Option<String>> = result
        .parking
        .iter()
        .map(|&p| {
            (p != NO_PARKING)
                .then(|| setup.map_or_else(String::new, |s| s.external_id(p).to_string()))
        })
        .collect();
    d.set_item("parking_id", parking_id)?;
    let direction: Vec<&str> = result
        .direction
        .iter()
        .map(|&x| match x {
            1 => "out",
            2 => "back",
            _ => "",
        })
        .collect();
    d.set_item("direction", direction)?;
    d.set_item("alternatives", result.alternatives.clone().into_pyarray(py))?;
    d.set_item("probability", result.probability.clone().into_pyarray(py))?;
    d.set_item("expected_s", result.expected_s.clone().into_pyarray(py))?;
    d.set_item("rides", result.rides.clone().into_pyarray(py))?;
    Ok(d)
}

/// Every trip's departure and the mode it took: what `Run.trip_modes` returns (D17).
///
/// `unstated` is the one mode a run with a single mode gives the trips without one (A24).
pub(crate) fn trip_modes<'py>(
    py: Python<'py>,
    result: Option<&ItineraryResult>,
    trips: &Trips,
    travellers: &Travellers,
    unstated: Option<Mode>,
) -> PyResult<Bound<'py, PyDict>> {
    let n = trips.len();
    // The stated mode (a car without one, or the run's one mode), then the itinerary
    // trips' mode taken, as `itinerary_choices` gives it.
    let stated = |trip: TripId| match unstated {
        Some(one) if !trips.mode_given(trip) => one,
        _ => trips.mode(trip),
    };
    let mut mode: Vec<Option<&str>> =
        (0..n).map(|t| Some(stated(TripId::new(t)).as_str())).collect();
    let mut choosing = vec![false; mode.len()];
    if let Some(result) = result {
        for (i, &t) in result.trip.iter().enumerate() {
            let t = t as usize;
            choosing[t] = result.choosing[i];
            match (result.mode[i], result.choosing[i]) {
                (NO_MODE, true) => mode[t] = None,
                (NO_MODE, false) => {}
                (m, _) => mode[t] = Some(Mode::ALL[m as usize].as_str()),
            }
        }
    }
    let (mut who, mut seq, mut departure, mut weight) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for t in 0..n {
        let trip = TripId::new(t);
        let traveller = trips.traveller(trip);
        who.push(travellers.external_ids().external(traveller.raw()).to_string());
        seq.push(t - travellers.first_trip(traveller).raw());
        departure.push(trips.departure(trip).get());
        weight.push(travellers.weight(traveller));
    }
    let d = PyDict::new(py);
    d.set_item("traveller_id", who)?;
    d.set_item("trip_seq", seq.into_pyarray(py))?;
    d.set_item("departure_s", departure.into_pyarray(py))?;
    d.set_item("mode", mode)?;
    d.set_item("mode_choice", choosing)?;
    d.set_item("weight", weight.into_pyarray(py))?;
    Ok(d)
}
