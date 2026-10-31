//! Parkings for Python: reading them from OpenStreetMap or a table,
//! the toy network's, and what a run's parkings and itineraries did.

use std::time::Instant;

use numpy::IntoPyArray;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyDict;

use openmobisim_core_demand::{Mode, Travellers, Trips};
use openmobisim_core_graph::geometry::LonLat;
use openmobisim_core_graph::hubs::{ParkingKind, ParkingRow};
use openmobisim_core_sim::{
    ItineraryResult, NO_MODE, NO_PARKING, NoAlternative, ParkingResult, ParkingSetup, TripTable,
};
use openmobisim_core_types::ids::{EntityId, TripId};
use openmobisim_io_osm::{ParkingReadOptions, ParkingReadReport, PbfSource, read_parkings};

use crate::network::parse_region;

mod cache;
pub use cache::{parking_cache_header, parking_read_cache, parking_write_cache};

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
    /// How long reading them took, in seconds: from an extract or a cache file (S294, I-as).
    read_s: Option<f64>,
}

#[pymethods]
impl PyParkings {
    /// How many parkings.
    #[getter]
    fn count(&self) -> usize {
        self.rows.len()
    }

    /// How long reading these parkings took, in seconds: from an extract
    /// (``parking_read_osm``) or from a cache file (``parking_read_cache``); ``None`` for
    /// parkings from a table or the toy network's.
    #[getter]
    fn read_s(&self) -> Option<f64> {
        self.read_s
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
    /// "capacity", "initial_occupancy", "fee_eur"}``, lists and arrays by parking (``fee_eur``
    /// ``None`` where the table gave none: the run's price for the kind applies).
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
        d.set_item("fee_eur", self.rows.iter().map(|r| r.fee_eur).collect::<Vec<_>>())?;
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
    let started = Instant::now();
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
    Ok(PyParkings { rows, report: Some(report), read_s: Some(started.elapsed().as_secs_f64()) })
}

/// One row of a parking table: ``(parking_id, lon, lat, vehicle, capacity, name,
/// hub_id, initial_occupancy, fee_eur)``.
type Row = (String, f64, f64, String, u32, Option<String>, Option<String>, u32, Option<f64>);

/// Parkings from table rows (``openmobisim.parking_read_table`` parses the
/// table; this checks and keeps it).
#[pyfunction(name = "_parking_from_rows")]
pub fn parking_from_rows(rows: Vec<Row>) -> PyResult<PyParkings> {
    let mut out = Vec::with_capacity(rows.len());
    let mut seen = std::collections::HashSet::new();
    for (parking_id, lon, lat, vehicle, capacity, name, hub_id, initial_occupancy, fee_eur) in rows
    {
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
        if fee_eur.is_some_and(|f| !(f.is_finite() && f >= 0.0)) {
            return Err(PyValueError::new_err(format!(
                "parking {parking_id:?}: fee_eur must be 0 or more, got {}",
                fee_eur.unwrap_or_default()
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
            fee_eur,
        });
    }
    Ok(PyParkings { rows: out, report: None, read_s: None })
}

/// The toy network's parkings: hub ``H`` at ``D2`` with 6 car and 3 bike
/// spaces, where the tram stop is, and car park ``P2`` at ``R2`` with 20 spaces,
/// 300 m on foot from that stop: the fixture of the hand-derived park-and-ride and
/// bike-and-ride cases.
#[pyfunction]
pub fn toy_network_parkings() -> PyParkings {
    PyParkings {
        rows: openmobisim_core_graph::examples::toy_network_parkings(),
        report: None,
        read_s: None,
    }
}

/// How full each parking was, bin by bin: what `Run.parking_bins` returns.
pub(crate) fn parking_bins<'py>(
    py: Python<'py>,
    setup: &ParkingSetup,
    result: &ParkingResult,
    parking_hubs: &[String],
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
    for (p, parking_hub) in parking_hubs.iter().enumerate().take(setup.count()) {
        let p32 = u32::try_from(p).expect("parkings fit u32");
        for k in 0..b.bins {
            let k32 = u32::try_from(k).expect("bins fit u32");
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "seconds")]
            let start_s = b.start_s as u32 + k32 * b.bin_s;
            bin.push(start_s / b.bin_s); // from midnight, like link_bins (G-31)
            start.push(start_s);
            parking.push(p32);
            id.push(setup.external_id(p32).to_string());
            hub.push(parking_hub.as_str());
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

/// Where each parking is and what it holds, for maps: ``{"parking_id", "hub_id", "vehicle",
/// "capacity", "lon", "lat", "name"}``.
pub(crate) fn parking_places<'py>(
    py: Python<'py>,
    setup: &ParkingSetup,
    parking_hubs: &[String],
) -> PyResult<Bound<'py, PyDict>> {
    let n = u32::try_from(setup.count()).expect("parkings fit u32");
    let d = PyDict::new(py);
    d.set_item("parking_id", (0..n).map(|p| setup.external_id(p).to_string()).collect::<Vec<_>>())?;
    d.set_item("hub_id", parking_hubs.to_vec())?;
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
    let mut class = Vec::new();
    for &t in &result.trip {
        let trip = TripId::new(t);
        let traveller = trips.traveller(trip);
        who.push(travellers.external_ids().external(traveller.raw()).to_string());
        let c = travellers.user_class(traveller).raw();
        class.push(travellers.class_external_ids().external(c).to_string());
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
    d.set_item("user_class", class)?;
    d.set_item("mode", mode)?;
    d.set_item("mode_choice", result.choosing.clone())?;
    d.set_item("logsum", result.logsum.clone().into_pyarray(py))?;
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

/// Each itinerary trip's chosen journey, leg by leg: what `Run.itinerary_journeys` returns (the
/// arrays of the core's `JourneyLegs`, by the names of its fields, and `trip`).
pub(crate) fn itinerary_journeys<'py>(
    py: Python<'py>,
    result: &ItineraryResult,
) -> PyResult<Bound<'py, PyDict>> {
    let j = &result.journeys;
    let d = PyDict::new(py);
    d.set_item("trip", result.trip.clone().into_pyarray(py))?;
    for (name, v) in [
        ("road_origin", &j.road_origin),
        ("road_destination", &j.road_destination),
        ("links", &j.links),
        ("link_offsets", &j.link_offsets),
        ("first_stop", &j.first_stop),
        ("last_stop", &j.last_stop),
        ("ride_line", &j.ride_line),
        ("ride_board", &j.ride_board),
        ("ride_alight", &j.ride_alight),
        ("ride_s", &j.ride_s),
        ("ride_transfer_walk_s", &j.ride_transfer_walk_s),
        ("ride_offsets", &j.ride_offsets),
        ("access_walk_s", &j.access_walk_s),
        ("egress_walk_s", &j.egress_walk_s),
        ("dock_pickup", &j.dock_pickup),
        ("dock_return", &j.dock_return),
    ] {
        d.set_item(name, v.clone().into_pyarray(py))?;
    }
    d.set_item("shape", j.shape.clone().into_pyarray(py))?;
    for (name, v) in [
        ("walk_s", &j.walk_s),
        ("wait_s", &j.wait_s),
        ("ride_total_s", &j.ride_total_s),
        ("vehicle_s", &j.vehicle_s),
        ("parking_s", &j.parking_s),
    ] {
        d.set_item(name, v.clone().into_pyarray(py))?;
    }
    d.set_item("expected_s", result.expected_s.clone().into_pyarray(py))?;
    let mode: Vec<Option<&str>> = result
        .mode
        .iter()
        .map(|&m| (m != NO_MODE).then(|| Mode::ALL[m as usize].as_str()))
        .collect();
    d.set_item("mode", mode)?;
    Ok(d)
}

/// The trips that had no alternative and travelled by car, as columns: who, when,
/// from where to where, and how many people each stands for.
pub(crate) fn fallback_car<'py>(
    py: Python<'py>,
    fallback: &[TripId],
    trips: &Trips,
    travellers: &Travellers,
) -> PyResult<Bound<'py, PyDict>> {
    let n = fallback.len();
    let (mut who, mut seq, mut departure, mut weight) = (
        Vec::with_capacity(n),
        Vec::with_capacity(n),
        Vec::with_capacity(n),
        Vec::with_capacity(n),
    );
    let (mut olon, mut olat, mut dlon, mut dlat) = (
        Vec::with_capacity(n),
        Vec::with_capacity(n),
        Vec::with_capacity(n),
        Vec::with_capacity(n),
    );
    for &trip in fallback {
        let traveller = trips.traveller(trip);
        who.push(travellers.external_ids().external(traveller.raw()).to_string());
        seq.push(trip.raw() - travellers.first_trip(traveller).raw());
        departure.push(trips.departure(trip).get());
        weight.push(travellers.weight(traveller));
        let (o, d) = (trips.origin(trip), trips.destination(trip));
        olon.push(o.lon);
        olat.push(o.lat);
        dlon.push(d.lon);
        dlat.push(d.lat);
    }
    let d = PyDict::new(py);
    d.set_item("traveller_id", who)?;
    d.set_item("trip_seq", seq.into_pyarray(py))?;
    d.set_item("departure_s", departure.into_pyarray(py))?;
    d.set_item("weight", weight.into_pyarray(py))?;
    d.set_item("origin_lon", olon.into_pyarray(py))?;
    d.set_item("origin_lat", olat.into_pyarray(py))?;
    d.set_item("destination_lon", dlon.into_pyarray(py))?;
    d.set_item("destination_lat", dlat.into_pyarray(py))?;
    Ok(d)
}

/// The trips the last choice left with no alternative, as columns: who,
/// their class, when, from where to where, how many people each stands for, the mode stated
/// (`None`: the trip chose its mode), whether it then went by car, and per mode
/// (`reason_car` … `reason_bike_transit`) why that mode gave it nothing (`""`: not offered).
pub(crate) fn no_alternative<'py>(
    py: Python<'py>,
    none: &[NoAlternative],
    trips: &Trips,
    travellers: &Travellers,
    modes: &[Mode],
) -> PyResult<Bound<'py, PyDict>> {
    let n = none.len();
    let (mut who, mut class, mut seq, mut departure, mut weight) = (
        Vec::with_capacity(n),
        Vec::with_capacity(n),
        Vec::with_capacity(n),
        Vec::with_capacity(n),
        Vec::with_capacity(n),
    );
    let (mut olon, mut olat, mut dlon, mut dlat) = (
        Vec::with_capacity(n),
        Vec::with_capacity(n),
        Vec::with_capacity(n),
        Vec::with_capacity(n),
    );
    let (mut stated, mut fallback) = (Vec::with_capacity(n), Vec::with_capacity(n));
    let mut reasons: Vec<Vec<&str>> = (0..Mode::COUNT).map(|_| Vec::with_capacity(n)).collect();
    for row in none {
        let trip = row.trip;
        let traveller = trips.traveller(trip);
        who.push(travellers.external_ids().external(traveller.raw()).to_string());
        let c = travellers.user_class(traveller).raw();
        class.push(travellers.class_external_ids().external(c).to_string());
        seq.push(trip.raw() - travellers.first_trip(traveller).raw());
        departure.push(trips.departure(trip).get());
        weight.push(travellers.weight(traveller));
        let (o, d) = (trips.origin(trip), trips.destination(trip));
        olon.push(o.lon);
        olat.push(o.lat);
        dlon.push(d.lon);
        dlat.push(d.lat);
        stated.push((!row.choosing).then(|| trips.mode(trip).as_str()));
        fallback.push(row.fallback_car);
        for (column, reason) in reasons.iter_mut().zip(row.reasons) {
            column.push(reason.as_str());
        }
    }
    let d = PyDict::new(py);
    d.set_item("traveller_id", who)?;
    d.set_item("trip_seq", seq.into_pyarray(py))?;
    d.set_item("user_class", class)?;
    d.set_item("departure_s", departure.into_pyarray(py))?;
    d.set_item("weight", weight.into_pyarray(py))?;
    d.set_item("origin_lon", olon.into_pyarray(py))?;
    d.set_item("origin_lat", olat.into_pyarray(py))?;
    d.set_item("destination_lon", dlon.into_pyarray(py))?;
    d.set_item("destination_lat", dlat.into_pyarray(py))?;
    d.set_item("mode_stated", stated)?;
    d.set_item("fallback_car", fallback)?;
    // The modes the run lists alone (S302): a service's mode only in a run with a service.
    for (mode, column) in Mode::ALL.iter().zip(reasons) {
        if modes.contains(mode) {
            d.set_item(format!("reason_{}", mode.as_str()), column)?;
        }
    }
    Ok(d)
}

/// Every trip's departure and the mode it took: what `Run.trip_modes` returns (columns of
/// `table`, which `trip_results` gives whole).
pub(crate) fn trip_modes<'py>(py: Python<'py>, table: &TripTable) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    d.set_item("traveller_id", table.traveller_id.clone())?;
    d.set_item("trip_seq", table.trip_seq.clone().into_pyarray(py))?;
    d.set_item("departure_s", table.departure_s.clone().into_pyarray(py))?;
    d.set_item("mode", table.mode.iter().map(|m| m.map(Mode::as_str)).collect::<Vec<_>>())?;
    d.set_item("mode_choice", table.mode_choice.clone())?;
    d.set_item("weight", table.weight.clone().into_pyarray(py))?;
    Ok(d)
}

/// Every trip and what became of it: what `Run.trip_results` returns, `trip_results.parquet`'s
/// columns without `run_id`. A missing value (an arrival, a travel time or a link where there is
/// none) is `NaN`; a trip with no mode has `None`.
pub(crate) fn trip_results<'py>(
    py: Python<'py>,
    table: &TripTable,
) -> PyResult<Bound<'py, PyDict>> {
    let n = u32::try_from(table.len()).expect("the trips of a run fit u32");
    let maybe = |v: &[Option<u32>]| -> Vec<f64> {
        v.iter().map(|x| x.map_or(f64::NAN, f64::from)).collect()
    };
    let d = PyDict::new(py);
    d.set_item("trip", (0..n).collect::<Vec<u32>>().into_pyarray(py))?;
    d.set_item("traveller_id", table.traveller_id.clone())?;
    d.set_item("trip_seq", table.trip_seq.clone().into_pyarray(py))?;
    d.set_item("user_class", table.user_class.clone())?;
    d.set_item("weight", table.weight.clone().into_pyarray(py))?;
    let lon_lat = |points: &[LonLat]| -> (Vec<f64>, Vec<f64>) {
        (points.iter().map(|p| p.lon).collect(), points.iter().map(|p| p.lat).collect())
    };
    let (lon, lat) = lon_lat(&table.origin);
    d.set_item("origin_lon", lon.into_pyarray(py))?;
    d.set_item("origin_lat", lat.into_pyarray(py))?;
    let (lon, lat) = lon_lat(&table.destination);
    d.set_item("destination_lon", lon.into_pyarray(py))?;
    d.set_item("destination_lat", lat.into_pyarray(py))?;
    d.set_item("departure_s", table.departure_s.clone().into_pyarray(py))?;
    d.set_item("mode", table.mode.iter().map(|m| m.map(Mode::as_str)).collect::<Vec<_>>())?;
    d.set_item("mode_choice", table.mode_choice.clone())?;
    d.set_item("outcome", table.outcome.iter().map(|o| o.as_str()).collect::<Vec<_>>())?;
    d.set_item("arrival_s", maybe(&table.arrival_s).into_pyarray(py))?;
    d.set_item("travel_time_s", maybe(&table.travel_time_s()).into_pyarray(py))?;
    d.set_item("link_at_end", maybe(&table.link_at_end).into_pyarray(py))?;
    d.set_item("vehicle_s", maybe(&table.vehicle_s).into_pyarray(py))?;
    d.set_item("access_egress_s", maybe(&table.access_egress_s).into_pyarray(py))?;
    // A run with a service (S303): what its service day actually gave each trip.
    if let Some((outcome, at)) = &table.service_outcome {
        d.set_item("service_outcome", outcome.clone())?;
        d.set_item("service_arrival_s", at.clone().into_pyarray(py))?;
    }
    Ok(d)
}
