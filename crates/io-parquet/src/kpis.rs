//! `kpis.parquet` — long format (Foundations §6):
//! `run_id, design_id, replication, iteration, mode, metric, value`.
//!
//! A new KPI needs no schema change, only a new row — which is exactly what
//! lets Phase 1 write a real (if short) `kpis.parquet` today and grow the
//! set of metrics later without touching this file's shape.
//!
//! **`mode`** (S195): `all` for the whole run; the trip metrics of the last
//! loading are also written once per mode that has trips (`car`, `bike`,
//! `walk`, …), so a mode's numbers are a filter away. The metrics of how a run
//! settled (the gaps, the shares, the time change) are the whole run's.

use std::path::Path;
use std::sync::Arc;

use arrow_array::{ArrayRef, Float64Array, RecordBatch, StringArray, UInt32Array};
use arrow_schema::{DataType, Field, Schema};
use openmobisim_core_demand::Mode;
use openmobisim_core_sim::{RunResult, TripCompletionStats};

use crate::WriteError;
use crate::io::write_single_batch;

/// One `kpis.parquet` row: `(iteration, mode, metric, value)`.
type Row = (u32, &'static str, &'static str, f64);

/// Every metric of a run, as `(iteration, mode, name, value)`.
///
/// A run without equilibration has one iteration (given by the caller, as
/// before): the totals. A run that iterated (S170) has a row set per
/// iteration: the total travel time and the completed and truncated trips of that
/// loading, and what the iteration showed of the pattern settling
/// (`reselected_share`, `changed_share`, `time_change`, `gap`, `gap_expected`, `gap_excess`,
/// `incomplete_share`, `gap_network`, `gap_network_excess`, `gap_flow`, `gap_flow_floor`,
/// `gap_flow_excess`; a number that was not measured has no row); the counts that cannot change from one loading to the next
/// (`trips`, `no_vehicle_available_trips`, `no_feasible_path_trips`,
/// `mode_not_available_trips`, `completion_rate`) are written once, for the last.
/// All of these are `mode` `all`; the last loading's trip metrics are also written
/// per mode that has trips (S195).
///
/// `total_travel_time_s` is traveller-weight-scaled (`Σ w · travel_time`) —
/// see `core-sim`'s own docs for why, and for the standing note that an
/// unweighted metric row joins this list, rather than replacing this one,
/// once comprehensive KPIs exist (confirmed by the user, 2026-09-14).
fn metrics(result: &RunResult, single_iteration: u32) -> Vec<Row> {
    let all = "all";
    let fixed = |iteration: u32, mode: &'static str, c: &TripCompletionStats| {
        vec![
            (iteration, mode, "no_vehicle_available_trips", f64::from(c.no_vehicle_available)),
            (iteration, mode, "no_feasible_path_trips", f64::from(c.no_feasible_path)),
            (iteration, mode, "mode_not_available_trips", f64::from(c.mode_not_available)),
            (iteration, mode, "completion_rate", c.completion_rate()),
        ]
    };
    // The last loading's trip metrics, per mode that has trips (S195).
    let by_mode = |iteration: u32| {
        let mut rows = Vec::new();
        for mode in Mode::ALL {
            let m = &result.by_mode[mode.index()];
            if m.completion.total_trips == 0 {
                continue;
            }
            let name = mode.as_str();
            rows.push((iteration, name, "trips", f64::from(m.completion.total_trips)));
            rows.push((iteration, name, "total_travel_time_s", m.total_travel_time.get()));
            rows.push((iteration, name, "completed_trips", f64::from(m.completion.completed)));
            rows.push((iteration, name, "truncated_trips", f64::from(m.completion.truncated)));
            rows.extend(fixed(iteration, name, &m.completion));
        }
        rows
    };
    let c = &result.completion;
    if result.iterations.len() <= 1 {
        let mut rows = vec![
            (single_iteration, all, "trips", f64::from(c.total_trips)),
            (single_iteration, all, "total_travel_time_s", result.total_travel_time.get()),
            (single_iteration, all, "completed_trips", f64::from(c.completed)),
            (single_iteration, all, "truncated_trips", f64::from(c.truncated)),
        ];
        rows.extend(fixed(single_iteration, all, c));
        rows.extend(by_mode(single_iteration));
        return rows;
    }
    let mut rows = Vec::new();
    let last = result.iterations.last().map_or(0, |r| r.iteration);
    for r in &result.iterations {
        let i = r.iteration;
        rows.push((i, all, "total_travel_time_s", r.total_travel_time_s));
        rows.push((i, all, "completed_trips", f64::from(r.completed)));
        rows.push((i, all, "truncated_trips", f64::from(r.truncated)));
        for (name, value) in [
            ("reselected_share", r.reselected_share),
            ("changed_share", r.changed_share),
            ("time_change", r.time_change),
            ("gap", r.gap),
            ("gap_expected", r.gap_expected),
            ("gap_excess", r.gap_excess),
            ("incomplete_share", r.incomplete_share),
            ("gap_network", r.gap_network),
            ("gap_network_excess", r.gap_network_excess),
            ("gap_flow", r.gap_flow),
            ("gap_flow_floor", r.gap_flow_floor),
            ("gap_flow_excess", r.gap_flow_excess),
        ] {
            if value.is_finite() {
                rows.push((i, all, name, value));
            }
        }
        if i == last {
            rows.push((i, all, "trips", f64::from(c.total_trips)));
            rows.extend(fixed(i, all, c));
            rows.extend(by_mode(i));
        }
    }
    rows
}

/// Write `kpis.parquet` for one run.
///
/// A run that iterated (S170) writes a row set for every iteration it made and
/// takes each row's `iteration` from its report: the total travel time and the
/// completed and truncated trips of that loading, and what the iteration showed of
/// the pattern settling (`reselected_share`, `changed_share`, `time_change`,
/// `gap`, `gap_expected`, `gap_excess` (the disequilibrium, S178), `incomplete_share`,
/// `gap_network`, `gap_network_excess`, `gap_flow`, `gap_flow_floor`, `gap_flow_excess`; a
/// number that was not measured has no row). The counts that cannot change from one loading to the
/// next (`no_vehicle_available_trips`, `no_feasible_path_trips`,
/// `completion_rate`) are written once, for the last iteration. The `iteration`
/// argument is used only for a run that did not iterate.
///
/// `design_id` and `replication` are accepted as given rather than computed
/// here: there is no `DesignSpace`/`run_batch` (P7/P12) yet, so there is exactly
/// one design and one replication — the caller states that rather than this
/// module inventing an identity for concepts that do not exist yet.
///
/// # Errors
///
/// [`WriteError`] if the file cannot be created or the Parquet writer
/// rejects the data.
pub fn write_kpis(
    path: impl AsRef<Path>,
    run_id: &str,
    design_id: &str,
    replication: u32,
    iteration: u32,
    result: &RunResult,
) -> Result<(), WriteError> {
    let rows = metrics(result, iteration);
    let n = rows.len();

    let run_id_col: ArrayRef = Arc::new(StringArray::from(vec![run_id; n]));
    let design_id_col: ArrayRef = Arc::new(StringArray::from(vec![design_id; n]));
    let replication_col: ArrayRef = Arc::new(UInt32Array::from(vec![replication; n]));
    let iteration_col: ArrayRef =
        Arc::new(UInt32Array::from(rows.iter().map(|(i, _, _, _)| *i).collect::<Vec<_>>()));
    let mode_col: ArrayRef =
        Arc::new(StringArray::from(rows.iter().map(|(_, mode, _, _)| *mode).collect::<Vec<_>>()));
    let metric_col: ArrayRef =
        Arc::new(StringArray::from(rows.iter().map(|(_, _, name, _)| *name).collect::<Vec<_>>()));
    let value_col: ArrayRef =
        Arc::new(Float64Array::from(rows.iter().map(|(_, _, _, v)| *v).collect::<Vec<_>>()));

    let schema = Arc::new(Schema::new(vec![
        Field::new("run_id", DataType::Utf8, false),
        Field::new("design_id", DataType::Utf8, false),
        Field::new("replication", DataType::UInt32, false),
        Field::new("iteration", DataType::UInt32, false),
        Field::new("mode", DataType::Utf8, false),
        Field::new("metric", DataType::Utf8, false),
        Field::new("value", DataType::Float64, false),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            run_id_col,
            design_id_col,
            replication_col,
            iteration_col,
            mode_col,
            metric_col,
            value_col,
        ],
    )?;

    write_single_batch(path.as_ref(), schema, &batch)
}
