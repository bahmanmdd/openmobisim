//! `kpis.parquet` — long format:
//! `run_id, design_id, replication, iteration, mode, metric, value`.
//!
//! A new KPI needs no schema change, only a new row, so the set of metrics grows without
//! touching this file's shape.
//!
//! **`mode`**: `all` for the whole run; the trip metrics of the last
//! loading are also written once per mode that has trips (`car`, `bike`,
//! `walk`, …), so a mode's numbers are a filter away. The metrics of how a run
//! settled (the gaps, the shares, the time change) are the whole run's.
//!
//! **A mode a service of the run serves** (`bike_share` and `bike_share_transit` with a docked
//! bike service, `taxi` with a taxi fleet) has only the rows of its trips' choice: `trips`,
//! `people`, `mode_share`, `gap` and the counts `no_vehicle_available_trips`,
//! `no_feasible_path_trips` and `mode_not_available_trips`. How its trips went (completed or
//! not, and in what time) is what the service day gave them, in the service's rows below; the
//! planned trips' `completed_trips`, `completed_people`, `truncated_trips`, `completion_rate`
//! and `total_travel_time_s` are not written under it. The `all` rows count every trip as
//! planned, a service's trips included.
//!
//! **Transit**, for a run with a timetable, under `mode` `transit`:
//! `boardings` (weighted by traveller weight) and, if its buses rode the roads,
//! `bus_runs_on_roads`, `bus_runs_arrived` and `bus_delay_mean_s` (the mean
//! delay at the last stop, realised minus scheduled; negative is early).
//!
//! **Each service** (S303), for a run with one, under `mode` the service's name (`bike_share`
//! and `taxi` unless the scenario named them otherwise), for the last iteration: what the
//! **service day** actually gave the last loading's trips that chose the service (the day played
//! once after the run had stopped, on its last loading's road times, the demand fixed; the
//! trips' own rows show the planned trip), counted in people (the travellers' weights), in this
//! order. A **taxi fleet** (one request per person): `requests`; `rides` (picked up and dropped
//! off), `cancelled` (no taxi could pick them up within the cancel limit: the trip ends there),
//! `unfinished` (still waiting or riding at the window's end, or requesting after it); over the
//! rides, `waiting_mean_s`, `waiting_median_s`, `waiting_p90_s` (the 90th percentile; the
//! waiting time runs from the request to the pickup, the dispatch delay included);
//! `assumed_wait_s` (the waiting time the travellers assumed when choosing); over the rides,
//! `in_vehicle_mean_s` (pickup to drop-off, boarding included) and `ride_km_mean` (kilometres
//! with the passenger); `empty_km_per_ride` (the fleet's kilometres without a passenger, to
//! pickups, chargers and relocations, per ride), `utilisation` (the share of the fleet's time in
//! the window with a passenger on board, boarding included), `rides_per_taxi`, `kwh_charged`.
//! A **docked bike service**: `requests` (people who planned a shared bike), `rides` (got a bike
//! and returned it), `no_bike` (found none at any dock tried and walked on), `unfinished`; over
//! the rides, `extra_walk_at_pickup_mean_s` and `extra_walk_at_return_mean_s` (the extra seconds
//! at the docks against the planned docks having a bike and a free slot);
//! `assumed_extra_walk_at_pickup_s`, `assumed_extra_walk_at_return_s` (what the travellers
//! assumed; `NaN` when from the docks' start-of-day bikes); `redirected_share` (the share of the
//! rides sent on to another dock at either end). A mean over nobody is `NaN`. A run without a
//! service has none of these rows.

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
/// A `free_flow` run has one iteration (given by the caller, as
/// before): the totals. A run that iterated has a row set per
/// iteration: the total travel time and the completed and truncated trips of that
/// loading, and what the iteration showed of the pattern settling
/// (`reselected_share`, `changed_share`, `time_change`, `gap`, `gap_expected`, `gap_excess`,
/// `incomplete_share`, `gap_network`, `gap_network_excess`, `gap_flow`, `gap_flow_floor`,
/// `gap_flow_excess`, `itinerary_gap_excess`, `gap_generalised_excess`, and the full gaps
/// `gap_route`, `gap_route_expected`, `gap_route_excess`, `gap_route_network`,
/// `gap_route_network_excess`, `gap_mode`, `gap_mode_expected`, `gap_mode_excess`,
/// `excess_min_route`, `excess_min_mode`, `unfinished_cost_share`; a number that was not measured has no row); the
/// counts that cannot change from one loading to the next
/// (`trips`, `no_vehicle_available_trips`, `no_feasible_path_trips`,
/// `mode_not_available_trips`, `completion_rate`) are written once, for the last.
/// All of these are `mode` `all`; the last loading's trip metrics are also written
/// per mode that has trips (for a mode a service serves, only its choice's: see the
/// [module docs](self)).
///
/// `people` and `completed_people` are the trips and the completed trips weighted by
/// their travellers' weights: the people they stand for.
///
/// `total_travel_time_s` is traveller-weight-scaled (`Σ w · travel_time`) —
/// see `core-sim`'s own docs for why, and for the standing note that an
/// unweighted metric row joins this list, rather than replacing this one,
/// once comprehensive KPIs exist.
fn metrics(result: &RunResult, single_iteration: u32) -> Vec<Row> {
    let all = "all";
    // The counts of the choice and, with `planned`, the completion rate of the planned trips.
    let fixed = |iteration: u32, mode: &'static str, c: &TripCompletionStats, planned: bool| {
        let mut rows = vec![
            (iteration, mode, "no_vehicle_available_trips", f64::from(c.no_vehicle_available)),
            (iteration, mode, "no_feasible_path_trips", f64::from(c.no_feasible_path)),
            (iteration, mode, "mode_not_available_trips", f64::from(c.mode_not_available)),
        ];
        if planned {
            rows.push((iteration, mode, "completion_rate", c.completion_rate()));
        }
        rows
    };
    // A mode a service of the run serves: its trips' completions and times are what the
    // service day gave (the rows under the service's name), not the planned trips' (S305).
    let served = |mode: Mode| match mode {
        Mode::BikeShare | Mode::BikeShareTransit => result.services.is_some(),
        Mode::Taxi => result.taxis.is_some(),
        _ => false,
    };
    // The last loading's trip metrics, per mode that has trips (S195); and transit's
    // boardings and how the buses kept time (S199).
    let by_mode = |iteration: u32| {
        let mut rows = Vec::new();
        if let Some(transit) = &result.transit {
            let t = "transit";
            rows.push((iteration, t, "boardings", transit.boardings.iter().sum::<f64>()));
            if let Some(b) = transit.buses {
                rows.push((iteration, t, "bus_runs_on_roads", f64::from(b.runs_on_roads)));
                rows.push((iteration, t, "bus_runs_arrived", f64::from(b.runs_arrived)));
                if b.delay_mean_s.is_finite() {
                    rows.push((iteration, t, "bus_delay_mean_s", b.delay_mean_s));
                }
            }
        }
        // Parking (M4): car parkings under park-and-ride, bike parkings under
        // bike-and-ride; and how the itineraries went.
        if let Some(p) = &result.parking {
            for (mode, totals) in
                [(Mode::CarTransit, &p.by_kind[0]), (Mode::BikeTransit, &p.by_kind[1])]
            {
                if result.by_mode[mode.index()].completion.total_trips == 0 {
                    continue;
                }
                let name = mode.as_str();
                rows.push((iteration, name, "parking_arrivals", totals.arrivals));
                rows.push((iteration, name, "parking_overflow_arrivals", totals.overflow_arrivals));
                if totals.arrivals > 0.0 {
                    rows.push((
                        iteration,
                        name,
                        "parking_full_share",
                        totals.overflow_arrivals / totals.arrivals,
                    ));
                }
                if totals.mismatch_s.is_finite() {
                    rows.push((iteration, name, "hub_mismatch_s", totals.mismatch_s));
                }
                rows.push((iteration, name, "vehicles_left_at_parkings", totals.left_at_end));
            }
        }
        if let Some(it) = &result.itineraries {
            rows.push((iteration, "all", "itinerary_replanned_trips", f64::from(it.replanned)));
            if it.return_mismatch_s.is_finite() {
                rows.push((iteration, "all", "return_mismatch_s", it.return_mismatch_s));
            }
        }
        // The mode shares, by the trips the travellers stand for, when trips chose (M5).
        let choosing = result.itineraries.as_ref().is_some_and(|it| it.choosing.contains(&true));
        // Over all people, those with no mode to take included (S269).
        let weighted: f64 = result.by_mode.iter().map(|m| m.weighted_trips).sum::<f64>()
            + result.no_mode.weighted_trips;
        for mode in Mode::ALL {
            let m = &result.by_mode[mode.index()];
            if m.completion.total_trips == 0 {
                continue;
            }
            let name = mode.as_str();
            if choosing && weighted > 0.0 {
                rows.push((iteration, name, "mode_share", m.weighted_trips / weighted));
            }
            let planned = !served(mode);
            rows.push((iteration, name, "trips", f64::from(m.completion.total_trips)));
            if planned {
                rows.push((iteration, name, "total_travel_time_s", m.total_travel_time.get()));
                rows.push((iteration, name, "completed_trips", f64::from(m.completion.completed)));
                rows.push((iteration, name, "truncated_trips", f64::from(m.completion.truncated)));
            }
            rows.push((iteration, name, "people", m.weighted_trips));
            if planned {
                rows.push((iteration, name, "completed_people", m.weighted_completed));
            }
            rows.extend(fixed(iteration, name, &m.completion, planned));
        }
        rows
    };
    let c = &result.completion;
    // The people the trips stand for (S232): `total_travel_time_s` counts people, the trip
    // counts count simulated trips, so a mean trip is the former over `completed_people`.
    let people: f64 = result.by_mode.iter().map(|m| m.weighted_trips).sum::<f64>()
        + result.no_mode.weighted_trips;
    let completed_people: f64 = result.by_mode.iter().map(|m| m.weighted_completed).sum();
    if result.iterations.len() <= 1 {
        let mut rows = vec![
            (single_iteration, all, "trips", f64::from(c.total_trips)),
            (single_iteration, all, "total_travel_time_s", result.total_travel_time.get()),
            (single_iteration, all, "completed_trips", f64::from(c.completed)),
            (single_iteration, all, "truncated_trips", f64::from(c.truncated)),
            (single_iteration, all, "people", people),
            (single_iteration, all, "completed_people", completed_people),
        ];
        rows.extend(fixed(single_iteration, all, c, true));
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
            ("completed_people", r.completed_people),
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
            // The itineraries' disequilibrium (S227), the third input of the run's verdict, so
            // the verdict can be read back from the file (C2, `Run.open`).
            ("itinerary_gap_excess", r.itinerary_gap_excess),
            // The in-set gap's deterministic part in generalised minutes (S299), which the
            // stopping rule reads where the route cost is not time alone.
            ("gap_generalised_excess", r.gap_generalised_excess),
            // The full gaps (S293), in the currency the report names (not written: it is text).
            ("gap_route", r.gap_route),
            ("gap_route_expected", r.gap_route_expected),
            ("gap_route_excess", r.gap_route_excess),
            ("gap_route_network", r.gap_route_network),
            ("gap_route_network_excess", r.gap_route_network_excess),
            ("gap_mode", r.gap_mode),
            ("gap_mode_expected", r.gap_mode_expected),
            ("gap_mode_excess", r.gap_mode_excess),
            ("excess_min_route", r.excess_min_route),
            ("excess_min_mode", r.excess_min_mode),
            ("unfinished_cost_share", r.unfinished_cost_share),
        ] {
            if value.is_finite() {
                rows.push((i, all, name, value));
            }
        }
        // The itinerary trips' gap, under their modes (M4); the car gap above is the car
        // routes'. And the hub expectation mismatch of the loading.
        for mode in Mode::ALL {
            let g = r.itinerary_gap[mode.index()];
            if g.is_finite() {
                rows.push((i, mode.as_str(), "gap", g));
            }
        }
        if r.hub_mismatch_s.is_finite() {
            rows.push((i, all, "hub_mismatch_s", r.hub_mismatch_s));
        }
        if r.mode_changed_share.is_finite() {
            rows.push((i, all, "mode_changed_share", r.mode_changed_share));
        }
        if i == last {
            rows.push((i, all, "trips", f64::from(c.total_trips)));
            rows.push((i, all, "people", people));
            rows.extend(fixed(i, all, c, true));
            rows.extend(by_mode(i));
        }
    }
    rows
}

/// One service row: `(iteration, service name, metric, value)`.
type ServiceRow = (u32, String, &'static str, f64);

/// The iteration the service rows are written under: the run's last.
fn last_iteration(result: &RunResult, single_iteration: u32) -> u32 {
    if result.iterations.len() <= 1 {
        single_iteration
    } else {
        result.iterations.last().map_or(single_iteration, |r| r.iteration)
    }
}

/// A run's docked service rows (see the [module docs](self)); none without one.
fn service_metrics(result: &RunResult, single_iteration: u32) -> Vec<ServiceRow> {
    use openmobisim_core_sim::services::in_run::DockSummary;
    let Some(s) = &result.services else { return Vec::new() };
    let last = last_iteration(result, single_iteration);
    let summary = s.last.as_ref().map_or_else(DockSummary::idle, |d| d.summary);
    summary
        .rows(s.assumed_extra_walk_s)
        .into_iter()
        .map(|(metric, v)| (last, s.service.clone(), metric, v))
        .collect()
}

/// A run's taxi rows (see the [module docs](self)); none without a taxi fleet.
fn taxi_metrics(result: &RunResult, single_iteration: u32) -> Vec<ServiceRow> {
    use openmobisim_core_sim::services::in_run::TaxiSummary;
    let Some(s) = &result.taxis else { return Vec::new() };
    let last = last_iteration(result, single_iteration);
    let summary = s.last.as_ref().map_or_else(TaxiSummary::idle, |d| d.summary);
    summary
        .rows(s.assumed.wait_s)
        .into_iter()
        .map(|(metric, v)| (last, s.service.clone(), metric, v))
        .collect()
}

/// Every row of `kpis.parquet`: the run's, then its services'.
fn all_rows(result: &RunResult, single_iteration: u32) -> Vec<(u32, String, &'static str, f64)> {
    metrics(result, single_iteration)
        .into_iter()
        .map(|(i, mode, name, v)| (i, mode.to_string(), name, v))
        .chain(service_metrics(result, single_iteration))
        .chain(taxi_metrics(result, single_iteration))
        .collect()
}

/// Write `kpis.parquet` for one run.
///
/// A run that iterated writes a row set for every iteration it made and
/// takes each row's `iteration` from its report: the total travel time and the
/// completed and truncated trips of that loading, and what the iteration showed of
/// the pattern settling (`reselected_share`, `changed_share`, `time_change`,
/// `gap`, `gap_expected`, `gap_excess` (the disequilibrium), `incomplete_share`,
/// `gap_network`, `gap_network_excess`, `gap_flow`, `gap_flow_floor`, `gap_flow_excess`,
/// `itinerary_gap_excess`, `gap_generalised_excess`, and the full gaps `gap_route`, `gap_route_expected`, `gap_route_excess`, `gap_route_network`,
/// `gap_route_network_excess`, `gap_mode`, `gap_mode_expected`, `gap_mode_excess`,
/// `excess_min_route`, `excess_min_mode`, `unfinished_cost_share`; a number that was not
/// measured has no row). The counts that cannot change from one loading to the
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
    let rows = all_rows(result, iteration);
    let n = rows.len();

    let run_id_col: ArrayRef = Arc::new(StringArray::from(vec![run_id; n]));
    let design_id_col: ArrayRef = Arc::new(StringArray::from(vec![design_id; n]));
    let replication_col: ArrayRef = Arc::new(UInt32Array::from(vec![replication; n]));
    let iteration_col: ArrayRef =
        Arc::new(UInt32Array::from(rows.iter().map(|(i, _, _, _)| *i).collect::<Vec<_>>()));
    let mode_col: ArrayRef = Arc::new(StringArray::from(
        rows.iter().map(|(_, mode, _, _)| mode.as_str()).collect::<Vec<_>>(),
    ));
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

/// The same rows as [`write_kpis`], as a CSV file (`kpis.csv`): the run's only results file when
/// it writes the minimal outputs (C2, `outputs="minimal"`, for mass runs). The header is the seven
/// columns of `kpis.parquet`; values are written in full (the shortest form that reads back to the
/// same number); a text field holding a comma, a quote or a line break is quoted.
///
/// # Errors
///
/// [`WriteError`] if the file cannot be created or written.
pub fn write_kpis_csv(
    path: impl AsRef<Path>,
    run_id: &str,
    design_id: &str,
    replication: u32,
    iteration: u32,
    result: &RunResult,
) -> Result<(), WriteError> {
    use std::fmt::Write as _;
    let quoted = |field: &str| -> String {
        if field.contains([',', '"', '\n', '\r']) {
            format!("\"{}\"", field.replace('"', "\"\""))
        } else {
            field.to_string()
        }
    };
    let (run_id, design_id) = (quoted(run_id), quoted(design_id));
    let mut text = String::from("run_id,design_id,replication,iteration,mode,metric,value\n");
    for (i, mode, name, value) in all_rows(result, iteration) {
        let mode = quoted(&mode);
        let _ = writeln!(text, "{run_id},{design_id},{replication},{i},{mode},{name},{value}");
    }
    std::fs::write(path, text)?;
    Ok(())
}
