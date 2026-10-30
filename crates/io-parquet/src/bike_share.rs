//! A run's docked bike service, from its service day (the last loading's shared-bike trips,
//! played once after the run has stopped, S303): three files, written only by a run with a
//! service. Seconds count from midnight; a clock text beside them (`08:03:20`) is the
//! same second. Counts of people are the travellers' weights (one simulated traveller may stand
//! for several people, each taking a bike).
//!
//! **`bike_share_events.parquet`** — the logbook: one row per event in time order (one second's
//! events in the order the simulation handled them), so every traveller's sequence and every
//! dock's state after each event can be followed:
//! `run_id, service, time_s, time, traveller_id, trip_seq, travellers, event, dock,
//! bikes_at_dock_after, free_slots_after, next_dock, to_next_m, transit_stop`.
//! `event` is one of `arrived_for_bike`, `took_bike`, `found_no_bike`, `walking_to_next_dock`,
//! `gave_up_walking_to_destination`, `arrived_to_return`, `returned_bike`, `found_no_free_slot`,
//! `riding_to_next_dock`, `left_bike_over_capacity`, `reached_destination` (a shared bike alone at
//! the trip's destination), `reached_transit_stop` (a shared bike to transit at its stop's hub,
//! named in `transit_stop`; `dock` null), and for an
//! operator's van `van_took_bikes`, `van_brought_bikes` (`traveller_id` null, `travellers` the
//! bikes moved). `dock` is null for `reached_destination` and `reached_transit_stop`; `next_dock` and `to_next_m` (metres
//! as the crow flies) are set for `walking_to_next_dock` and `riding_to_next_dock`. Without the
//! service's log (`options={"log_events": 0}`) the file has no rows.
//!
//! **`bike_share_trips.parquet`** — one row per trip that chose a shared bike, in trip order:
//! who (`run_id, service, traveller_id, trip_seq, travellers, mode`); the plan
//! (`planned_pickup_dock, planned_return_dock, planned_at_pickup_dock_s, planned_leg_end_s,
//! expected_extra_s`); what happened (`pickup_dock, pickup_s, docks_tried_for_bike,
//! return_dock, return_s, docks_tried_for_slot, leg_end_s`); the difference (`extra_s` =
//! `leg_end_s − planned_leg_end_s`, split into `pickup_extra_s` and `return_extra_s`: the
//! return's part is the rides between return docks after a full one plus the walk from the dock
//! the bike was left at less the planned walk from the planned return dock, the pickup's the
//! rest; `extra_walk_m`, `met_minus_expected_s` = `extra_s −
//! expected_extra_s`: met minus expected, positive when worse than expected); `failed_attempts_for_bike`, `failed_attempts_for_slot`; and `result`:
//! `as_planned`, `bike_at_another_dock` (found a dock empty, then a bike at another),
//! `returned_at_another_dock`, `no_bike_walked_to_destination`, `left_over_capacity`,
//! `still_riding_at_window_end`, `still_looking_for_bike_at_window_end`, `not_started`.
//!
//! **`bike_share_docks.parquet`** — one row per dock and time bin (the service's `bin_s`), dock by
//! dock: `run_id, service, dock, hub_id, bin_start, bin_end, bin_start_s, bin_end_s`; its state
//! (`slots, bikes_at_start, bikes_at_end, bikes_min, bikes_mean, seconds_empty, seconds_full`);
//! its flows (`bikes_taken, bikes_returned`); the travellers in trouble there
//! (`people_failed_attempt_no_bike` found it empty and went on, `people_gave_up_no_bike` found it
//! empty and walked to their destination, `people_failed_attempt_no_slot`,
//! `people_left_bike_over_capacity`, `people_sent_here` from another dock); and an operator's
//! (`bikes_brought_by_van, bikes_taken_by_van`, 0 in a run).
//!
//! Each file's key-value metadata carries `openmobisim.run_fingerprint` and
//! `openmobisim.master_seed`; `bike_share_docks.parquet`'s also `openmobisim.fleet_bikes`, the
//! bikes at the start of the day, all docks summed. **Cost.** About 150 bytes per row while writing.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use arrow_array::{ArrayRef, Float64Array, RecordBatch, StringArray, UInt32Array};
use arrow_schema::{DataType, Field, Schema};
use openmobisim_core_sim::RunDescription;
use openmobisim_core_sim::services::tables::{
    BikeShareDockTable, BikeShareEventTable, BikeShareTripTable, clock,
};
use parquet::basic::Compression;

use crate::WriteError;
use crate::io::write_single_batch_with;

/// Columns being gathered: each with its name, type and whether it may be null.
#[derive(Default)]
pub(crate) struct Columns {
    fields: Vec<Field>,
    arrays: Vec<ArrayRef>,
}

impl Columns {
    pub(crate) fn text(
        &mut self,
        name: &str,
        v: impl IntoIterator<Item = impl AsRef<str>>,
    ) -> &mut Self {
        let v: Vec<String> = v.into_iter().map(|x| x.as_ref().to_string()).collect();
        self.push(name, DataType::Utf8, false, Arc::new(StringArray::from(v)))
    }
    pub(crate) fn maybe_text(&mut self, name: &str, v: &[Option<String>]) -> &mut Self {
        let v: Vec<Option<&str>> = v.iter().map(Option::as_deref).collect();
        self.push(name, DataType::Utf8, true, Arc::new(StringArray::from(v)))
    }
    pub(crate) fn count(&mut self, name: &str, v: &[u32]) -> &mut Self {
        self.push(name, DataType::UInt32, false, Arc::new(UInt32Array::from(v.to_vec())))
    }
    pub(crate) fn maybe_count(&mut self, name: &str, v: &[Option<u32>]) -> &mut Self {
        self.push(name, DataType::UInt32, true, Arc::new(UInt32Array::from(v.to_vec())))
    }
    pub(crate) fn number(&mut self, name: &str, v: &[f64]) -> &mut Self {
        self.push(name, DataType::Float64, false, Arc::new(Float64Array::from(v.to_vec())))
    }
    pub(crate) fn maybe_number(&mut self, name: &str, v: &[Option<f64>]) -> &mut Self {
        self.push(name, DataType::Float64, true, Arc::new(Float64Array::from(v.to_vec())))
    }
    pub(crate) fn push(&mut self, name: &str, t: DataType, null: bool, a: ArrayRef) -> &mut Self {
        self.fields.push(Field::new(name, t, null));
        self.arrays.push(a);
        self
    }

    pub(crate) fn write(
        self,
        path: &Path,
        description: &RunDescription,
        extra: &[(&str, String)],
    ) -> Result<(), WriteError> {
        let mut metadata = vec![
            ("openmobisim.run_fingerprint".to_string(), description.fingerprint_hex()),
            ("openmobisim.master_seed".to_string(), description.master_seed.to_string()),
        ];
        metadata.extend(extra.iter().map(|(k, v)| ((*k).to_string(), v.clone())));
        let schema = Arc::new(
            Schema::new(self.fields)
                .with_metadata(metadata.iter().cloned().collect::<HashMap<_, _>>()),
        );
        let batch = RecordBatch::try_new(schema.clone(), self.arrays)?;
        write_single_batch_with(path, schema, &batch, &metadata, Compression::SNAPPY)
    }
}

/// Write `bike_share_events.parquet` (see the [module docs](self)).
///
/// # Errors
///
/// [`WriteError`] if the file cannot be created or the Parquet writer rejects the data.
pub fn write_bike_share_events(
    path: impl AsRef<Path>,
    run_id: &str,
    t: &BikeShareEventTable,
    description: &RunDescription,
) -> Result<(), WriteError> {
    let mut c = Columns::default();
    c.text("run_id", vec![run_id; t.len()])
        .text("service", &t.service)
        .number("time_s", &t.time_s)
        .text("time", t.time_s.iter().map(|&s| clock(s)))
        .maybe_text("traveller_id", &t.traveller_id)
        .maybe_count("trip_seq", &t.trip_seq)
        .count("travellers", &t.travellers)
        .text("event", &t.event)
        .maybe_text("dock", &t.dock)
        .maybe_count("bikes_at_dock_after", &t.bikes_at_dock_after)
        .maybe_count("free_slots_after", &t.free_slots_after)
        .maybe_text("next_dock", &t.next_dock)
        .maybe_number("to_next_m", &t.to_next_m)
        .maybe_text("transit_stop", &t.transit_stop);
    c.write(path.as_ref(), description, &[])
}

/// Write `bike_share_trips.parquet` (see the [module docs](self)).
///
/// # Errors
///
/// [`WriteError`] if the file cannot be created or the Parquet writer rejects the data.
pub fn write_bike_share_trips(
    path: impl AsRef<Path>,
    run_id: &str,
    t: &BikeShareTripTable,
    description: &RunDescription,
) -> Result<(), WriteError> {
    let mut c = Columns::default();
    c.text("run_id", vec![run_id; t.len()])
        .text("service", &t.service)
        .text("traveller_id", &t.traveller_id)
        .count("trip_seq", &t.trip_seq)
        .count("travellers", &t.travellers)
        .text("mode", &t.mode)
        .text("planned_pickup_dock", &t.planned_pickup_dock)
        .text("planned_return_dock", &t.planned_return_dock)
        .number("planned_at_pickup_dock_s", &t.planned_at_pickup_dock_s)
        .number("planned_leg_end_s", &t.planned_leg_end_s)
        .number("expected_extra_s", &t.expected_extra_s)
        .maybe_text("pickup_dock", &t.pickup_dock)
        .maybe_number("pickup_s", &t.pickup_s)
        .count("docks_tried_for_bike", &t.docks_tried_for_bike)
        .maybe_text("return_dock", &t.return_dock)
        .maybe_number("return_s", &t.return_s)
        .count("docks_tried_for_slot", &t.docks_tried_for_slot)
        .maybe_number("leg_end_s", &t.leg_end_s)
        .maybe_number("extra_s", &t.extra_s)
        .maybe_number("pickup_extra_s", &t.pickup_extra_s)
        .maybe_number("return_extra_s", &t.return_extra_s)
        .number("extra_walk_m", &t.extra_walk_m)
        .maybe_number("met_minus_expected_s", &t.met_minus_expected_s)
        .count("failed_attempts_for_bike", &t.failed_attempts_for_bike)
        .count("failed_attempts_for_slot", &t.failed_attempts_for_slot)
        .text("result", &t.result);
    c.write(path.as_ref(), description, &[])
}

/// Write `bike_share_docks.parquet` (see the [module docs](self)).
///
/// # Errors
///
/// [`WriteError`] if the file cannot be created or the Parquet writer rejects the data.
pub fn write_bike_share_docks(
    path: impl AsRef<Path>,
    run_id: &str,
    t: &BikeShareDockTable,
    description: &RunDescription,
) -> Result<(), WriteError> {
    let mut c = Columns::default();
    c.text("run_id", vec![run_id; t.len()])
        .text("service", &t.service)
        .text("dock", &t.dock)
        .text("hub_id", &t.hub_id)
        .text("bin_start", t.bin_start_s.iter().map(|&s| clock(s)))
        .text("bin_end", t.bin_end_s.iter().map(|&s| clock(s)))
        .number("bin_start_s", &t.bin_start_s)
        .number("bin_end_s", &t.bin_end_s)
        .count("slots", &t.slots);
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
        c.number(name, v);
    }
    c.write(path.as_ref(), description, &[("openmobisim.fleet_bikes", t.fleet.to_string())])
}
