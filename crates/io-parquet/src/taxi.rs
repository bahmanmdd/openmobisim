//! A run's taxi fleet, from its service day (the last loading's taxi trips, played once after
//! the run has stopped, S303): three files, written only by a run
//! with a taxi fleet. Seconds count from midnight; a clock text beside them (`08:03:20`) is the
//! same second. A trip standing for several people asks for that many vehicles (one seat each);
//! each such unit request has its own events.
//!
//! **`taxi_events.parquet`** — the logbook: one row per event in time order (one second's events
//! in the order the simulation handled them), so every traveller's and every vehicle's sequence
//! can be followed: `run_id, service, time_s, time, event, traveller_id, trip_seq, travellers,
//! vehicle, vehicle_state_after, charge_after, charger, drive_km`. `event` is one of
//! `asked_for_taxi`, `waiting_for_taxi` (no idle vehicle could take it at once),
//! `taxi_assigned`, `taxi_arrived_at_pickup`, `picked_up` (boarded, the drive starts),
//! `dropped_off`, `no_taxi_within_wait_limit` (no vehicle sent within the fleet's `max_wait_s`:
//! the trip's last event), `still_waiting_at_window_end`, `still_riding_at_window_end`, and the
//! vehicles' own `vehicle_went_to_charge`, `queued_for_plug`, `started_charging`, `charged`
//! (`traveller_id`, `trip_seq` and `travellers` null). `vehicle_state_after` is one of `idle`,
//! `to_pickup`, `occupied`, `relocating`, `to_charger`, `charging`, `queued_at_charger`;
//! `charge_after` a share of the battery (a drive's energy is taken as it starts); `drive_km` the
//! kilometres of the drive the event starts. Without the fleet's log (`options={"log_events":
//! 0}`) the file has no rows.
//!
//! **`taxi_trips.parquet`** — one row per trip that chose a taxi, in trip order: who (`run_id,
//! service, traveller_id, trip_seq, travellers`); the plan (`requested_s, expected_wait_s,
//! expected_no_taxi_share, expected_door_to_door_s`); what happened (`vehicle, assigned_s,
//! taxi_at_pickup_s, waited_s, km_to_pickup, km_with_passenger, dropped_off_s, arrived_s,
//! door_to_door_s`; a request with no vehicle within the longest wait ends there, so it has no
//! arrival); the difference (`waited_minus_expected_s`,
//! `met_minus_expected_s` = `door_to_door_s − expected_door_to_door_s`, positive when worse than
//! expected); and `result`: `as_planned`, `waited_longer_than_expected` (by more than a
//! minute), `no_taxi`, `still_waiting_at_window_end`, `still_riding_at_window_end`, `not_started`.
//!
//! **`taxi_vehicles.parquet`** — one row per vehicle and 15-minute bin, vehicle by vehicle:
//! `run_id, service, vehicle, bin_start, bin_end, bin_start_s, bin_end_s`; the seconds in each
//! state (`seconds_idle, seconds_to_pickup, seconds_occupied, seconds_relocating,
//! seconds_to_charger, seconds_charging, seconds_queued_at_charger`, adding up to the bin's
//! length); `km_occupied, km_empty` (a drive's kilometres spread over its seconds);
//! `charge_at_start, charge_at_end` (shares of the battery), `kwh_charged`,
//! `passengers_dropped_off`.
//!
//! Each file's key-value metadata carries `openmobisim.run_fingerprint` and
//! `openmobisim.master_seed`; `taxi_vehicles.parquet`'s also `openmobisim.fleet_vehicles`.
//! **Cost.** About 150 bytes per row while writing.

use std::path::Path;

use openmobisim_core_sim::RunDescription;
use openmobisim_core_sim::services::engine::VehicleState;
use openmobisim_core_sim::services::tables::clock;
use openmobisim_core_sim::services::taxi_tables::{
    TaxiEventTable, TaxiTripTable, TaxiVehicleTable,
};

use crate::WriteError;
use crate::bike_share::Columns;

/// Write `taxi_events.parquet` (see the [module docs](self)).
///
/// # Errors
///
/// [`WriteError`] if the file cannot be created or the Parquet writer rejects the data.
pub fn write_taxi_events(
    path: impl AsRef<Path>,
    run_id: &str,
    t: &TaxiEventTable,
    description: &RunDescription,
) -> Result<(), WriteError> {
    let states: Vec<Option<String>> =
        t.vehicle_state_after.iter().map(|s| s.map(str::to_string)).collect();
    let mut c = Columns::default();
    c.text("run_id", vec![run_id; t.len()])
        .text("service", &t.service)
        .number("time_s", &t.time_s)
        .text("time", t.time_s.iter().map(|&s| clock(s)))
        .text("event", &t.event)
        .maybe_text("traveller_id", &t.traveller_id)
        .maybe_count("trip_seq", &t.trip_seq)
        .maybe_count("travellers", &t.travellers)
        .maybe_text("vehicle", &t.vehicle)
        .maybe_text("vehicle_state_after", &states)
        .maybe_number("charge_after", &t.charge_after)
        .maybe_text("charger", &t.charger)
        .maybe_number("drive_km", &t.drive_km);
    c.write(path.as_ref(), description, &[])
}

/// Write `taxi_trips.parquet` (see the [module docs](self)).
///
/// # Errors
///
/// [`WriteError`] if the file cannot be created or the Parquet writer rejects the data.
pub fn write_taxi_trips(
    path: impl AsRef<Path>,
    run_id: &str,
    t: &TaxiTripTable,
    description: &RunDescription,
) -> Result<(), WriteError> {
    let mut c = Columns::default();
    c.text("run_id", vec![run_id; t.len()])
        .text("service", &t.service)
        .text("traveller_id", &t.traveller_id)
        .count("trip_seq", &t.trip_seq)
        .count("travellers", &t.travellers)
        .number("requested_s", &t.requested_s)
        .number("expected_wait_s", &t.expected_wait_s)
        .number("expected_no_taxi_share", &t.expected_no_taxi_share)
        .number("expected_door_to_door_s", &t.expected_door_to_door_s)
        .maybe_text("vehicle", &t.vehicle)
        .maybe_number("assigned_s", &t.assigned_s)
        .maybe_number("taxi_at_pickup_s", &t.taxi_at_pickup_s)
        .number("waited_s", &t.waited_s)
        .maybe_number("waited_minus_expected_s", &t.waited_minus_expected_s)
        .maybe_number("km_to_pickup", &t.km_to_pickup)
        .maybe_number("km_with_passenger", &t.km_with_passenger)
        .maybe_number("dropped_off_s", &t.dropped_off_s)
        .maybe_number("arrived_s", &t.arrived_s)
        .maybe_number("door_to_door_s", &t.door_to_door_s)
        .maybe_number("met_minus_expected_s", &t.met_minus_expected_s)
        .text("result", &t.result);
    c.write(path.as_ref(), description, &[])
}

/// The column of the seconds a vehicle spent in `state` in a bin.
#[must_use]
pub fn seconds_column(state: VehicleState) -> String {
    format!("seconds_{}", state.as_str())
}

/// Write `taxi_vehicles.parquet` (see the [module docs](self)).
///
/// # Errors
///
/// [`WriteError`] if the file cannot be created or the Parquet writer rejects the data.
pub fn write_taxi_vehicles(
    path: impl AsRef<Path>,
    run_id: &str,
    t: &TaxiVehicleTable,
    description: &RunDescription,
) -> Result<(), WriteError> {
    let mut c = Columns::default();
    c.text("run_id", vec![run_id; t.len()])
        .text("service", &t.service)
        .text("vehicle", &t.vehicle)
        .text("bin_start", t.bin_start_s.iter().map(|&s| clock(s)))
        .text("bin_end", t.bin_end_s.iter().map(|&s| clock(s)))
        .number("bin_start_s", &t.bin_start_s)
        .number("bin_end_s", &t.bin_end_s);
    for (state, v) in VehicleState::ALL.iter().zip(&t.seconds) {
        c.number(&seconds_column(*state), v);
    }
    for (name, v) in [
        ("km_occupied", &t.km_occupied),
        ("km_empty", &t.km_empty),
        ("charge_at_start", &t.charge_at_start),
        ("charge_at_end", &t.charge_at_end),
        ("kwh_charged", &t.kwh_charged),
        ("passengers_dropped_off", &t.passengers_dropped_off),
    ] {
        c.number(name, v);
    }
    c.write(path.as_ref(), description, &[("openmobisim.fleet_vehicles", t.fleet.to_string())])
}
