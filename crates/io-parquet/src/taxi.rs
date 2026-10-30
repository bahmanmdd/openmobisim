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
//! `dropped_off`, `request_cancelled` (no taxi could pick it up within the fleet's cancel limit
//! `request_cancel_s`: the trip's last event), `still_waiting_at_window_end`, `still_riding_at_window_end`, and the
//! vehicles' own `vehicle_went_to_charge`, `queued_for_plug`, `started_charging`, `charged`
//! (`traveller_id`, `trip_seq` and `travellers` null). `vehicle_state_after` is one of `idle`,
//! `to_pickup`, `occupied`, `relocating`, `to_charger`, `charging`, `queued_at_charger`;
//! `charge_after` a share of the battery (a drive's energy is taken as it starts); `drive_km` the
//! kilometres of the drive the event starts. Without the fleet's log (`options={"log_events":
//! 0}`) the file has no rows.
//!
//! **`taxi_trips.parquet`** — one row per request (a trip that chose a taxi), in trip order:
//! `run_id, service, traveller_id, trip_seq, travellers, requested_s` (the departure),
//! `assumed_wait_s` (the waiting time the traveller assumed when choosing), `outcome`
//! (`completed`: picked up and dropped off; `cancelled`: no taxi could pick it up within the
//! fleet's cancel limit `request_cancel_s`, and the trip ends there; `unfinished`: still waiting
//! or riding when the window ended; `not_requested`: a departure after the window), `vehicle`,
//! `pickup_s` (the taxi at the pickup), `dropoff_s`, `waiting_s` (from the request to the
//! pickup, the dispatch delay `dispatch_delay_s` included; null if not picked up),
//! `in_vehicle_s` (from the pickup to the drop-off, boarding included), `ride_km` (with the
//! passenger on board), `empty_km` (the taxi's drive to the pickup).
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
        .number("assumed_wait_s", &t.assumed_wait_s)
        .text("outcome", &t.outcome)
        .maybe_text("vehicle", &t.vehicle)
        .maybe_number("pickup_s", &t.pickup_s)
        .maybe_number("dropoff_s", &t.dropoff_s)
        .maybe_number("waiting_s", &t.waiting_s)
        .maybe_number("in_vehicle_s", &t.in_vehicle_s)
        .maybe_number("ride_km", &t.ride_km)
        .maybe_number("empty_km", &t.empty_km);
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
