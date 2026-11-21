//! `iterations.parquet`: one row per iteration of the run, from 0, in order.
//!
//! | Column | Arrow type | Null | Meaning |
//! |---|---|---|---|
//! | `iteration` | UInt32 | no | the iteration, from 0 |
//! | `mean_trip_time_s` | Float64 | yes | the mean door-to-door time of the finished trips, seconds; null when none finished |
//! | `finished` | UInt64 | no | trips that arrived before the window's end |
//! | `unfinished` | UInt64 | no | trips with an alternative still travelling at the window's end |
//! | `without_alternative` | UInt64 | no | trips with no alternative in any mode open to them |
//! | `without_transit` | UInt64 | no | trips with transit open to them and no journey offered |
//! | `without_stop` | UInt64 | no | of those, trips with no stop within `stop_walk_max` of their origin or of their destination (a larger `stop_walk_max` reaches more of them) |
//! | `offered_share` | Float64 | yes | the share of trips with an alternative that were offered a new choice in this iteration's choice (1 at iteration 0, where everyone chooses); null when no trip has one |
//! | `switch_share` | Float64 | yes | the share of trips with an alternative that changed it in this iteration's choice; null when no trip has one |
//! | `found_share` | Float64 | yes | the share of trips with an alternative whose search found one cheaper than every alternative of its mode in their set; null likewise |
//! | `would_change_share` | Float64 | yes | under the logit, the share of trips with an alternative that the logit, choosing again over their own set at the costs this iteration measured, the set as it stands after this iteration (the next iteration's choice reads the same unless its search adds an alternative first), would put on another alternative than the one they hold; 0 when the run is at the fixed point of its iterations; null under the least-cost choice, and when no trip has an alternative |
//! | `gap` | Float64 | yes | under the least-cost choice, the relative duality gap, Σ (chosen − least) / Σ least; null under the logit, when no trip has an alternative, or when the least costs sum to 0 |
//! | `search_s`, `choose_s`, `load_s`, `measure_s`, `assess_s` | Float64 | no | the seconds each stage took, wall clock |
//!
//! Each model reports the one measure that can reach 0. Under the least-cost choice the routes
//! in use cost the same at the equilibrium, so the share of travellers not on their lowest-cost
//! alternative does not settle, and the gap, which weighs each traveller by what it would save,
//! is the measure. Under the logit the model puts a share of travellers on an alternative that
//! is not their lowest-cost one, so the gap does not reach zero, and the share who would change
//! is the measure.

use std::sync::Arc;

use arrow_array::{Float64Array, RecordBatch, UInt32Array, UInt64Array};
use arrow_schema::{DataType, Field, Schema};
use openmobisim_loop::assess::Iteration;

use crate::{WriteError, column, value};

/// The five stages, in the order of `Iteration::stage_seconds`.
pub(crate) const STAGES: [&str; 5] = ["search_s", "choose_s", "load_s", "measure_s", "assess_s"];

/// The iterations table as one batch.
pub(crate) fn batch(rows: &[Iteration]) -> Result<RecordBatch, WriteError> {
    let nullable = |f: fn(&Iteration) -> f64| {
        column(rows.iter().map(|r| value(f(r))).collect::<Float64Array>())
    };
    let count =
        |f: fn(&Iteration) -> u64| column(UInt64Array::from_iter_values(rows.iter().map(f)));
    let mut columns = vec![
        column(UInt32Array::from_iter_values(rows.iter().map(|r| r.iteration))),
        nullable(|r| r.mean_trip_time),
        count(|r| r.finished),
        count(|r| r.unfinished),
        count(|r| r.without_alternative),
        count(|r| r.without_transit),
        count(|r| r.without_stop),
        nullable(|r| r.offered_share),
        nullable(|r| r.switch_share),
        nullable(|r| r.found_share),
        nullable(|r| r.would_change_share),
        nullable(|r| r.gap),
    ];
    for k in 0..STAGES.len() {
        columns
            .push(column(Float64Array::from_iter_values(rows.iter().map(|r| r.stage_seconds[k]))));
    }
    Ok(RecordBatch::try_new(Arc::new(schema()), columns)?)
}

/// The file's columns, in order: the writer writes them and the reader asks for exactly them.
pub(crate) fn schema() -> Schema {
    let mut fields = vec![
        Field::new("iteration", DataType::UInt32, false),
        Field::new("mean_trip_time_s", DataType::Float64, true),
        Field::new("finished", DataType::UInt64, false),
        Field::new("unfinished", DataType::UInt64, false),
        Field::new("without_alternative", DataType::UInt64, false),
        Field::new("without_transit", DataType::UInt64, false),
        Field::new("without_stop", DataType::UInt64, false),
        Field::new("offered_share", DataType::Float64, true),
        Field::new("switch_share", DataType::Float64, true),
        Field::new("found_share", DataType::Float64, true),
        Field::new("would_change_share", DataType::Float64, true),
        Field::new("gap", DataType::Float64, true),
    ];
    for name in STAGES {
        fields.push(Field::new(name, DataType::Float64, false));
    }
    Schema::new(fields)
}
