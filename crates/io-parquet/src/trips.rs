//! `trips.parquet`: one row per trip of the demand, in trip id order (the external ids sorted
//! bytewise), what the run's last iteration made of it.
//!
//! | Column | Arrow type | Null | Meaning |
//! |---|---|---|---|
//! | `trip_id` | Utf8 | no | the trip's external id |
//! | `class_id` | Utf8 | no | its user class's external id |
//! | `departure_s` | UInt32 | no | its departure, a second of the clock |
//! | `mode` | Utf8 | yes | the mode taken: `car`, `bike`, `walk` or `transit`; null without an alternative |
//! | `arrival_s` | Float64 | yes | the arrival at the door, seconds of the clock (`departure_s + travel_time_s` for a finished trip); null unfinished or without an alternative |
//! | `travel_time_s` | Float64 | yes | door to door as realised, seconds; null likewise |
//! | `finished` | Boolean | no | whether it arrived before the window's end |
//! | `cost` | Float64 | yes | the cost of the alternative taken at the last measure, in the choice model's unit (minutes in a time-only run); null without an alternative |
//! | `alternatives` | UInt32 | no | how many alternatives its set holds |

use std::sync::Arc;

use arrow_array::{BooleanArray, Float64Array, RecordBatch, StringArray, UInt32Array};
use arrow_schema::{DataType, Field, Schema};
use openmobisim_demand::demand::Demand;
use openmobisim_loop::run::Trips;
use openmobisim_types::ids::EntityId;
use openmobisim_types::time::Second;

use crate::{WriteError, column, value};

/// The trips table as one batch, one row per trip run (a trip the window could not hold,
/// `kept` false, is left out); `demand` gives the external ids, `start` the window's start on
/// the clock, added to the loop's arrivals (seconds after it).
pub(crate) fn batch(
    trips: &Trips,
    demand: &Demand,
    start: Second,
) -> Result<RecordBatch, WriteError> {
    let rows: Vec<usize> = (0..trips.kept.len()).filter(|&i| trips.kept[i]).collect();
    let floats = |v: &[f64]| column(rows.iter().map(|&i| value(v[i])).collect::<Float64Array>());
    let start = f64::from(start.get());
    let arrivals = rows.iter().map(|&i| value(trips.arrival[i]).map(|x| start + x));
    let ids = demand.trip_ids();
    let columns = vec![
        column(StringArray::from_iter_values(rows.iter().map(|&i| ids.external(as_u32(i))))),
        column(StringArray::from_iter_values(
            rows.iter().map(|&i| demand.class_ids().external(trips.class[i].raw())),
        )),
        column(UInt32Array::from_iter_values(rows.iter().map(|&i| trips.departure[i].get()))),
        column(rows.iter().map(|&i| trips.mode[i].map(|m| m.to_string())).collect::<StringArray>()),
        column(arrivals.collect::<Float64Array>()),
        floats(&trips.travel_time),
        column(BooleanArray::from(rows.iter().map(|&i| trips.finished[i]).collect::<Vec<_>>())),
        floats(&trips.cost),
        column(UInt32Array::from_iter_values(rows.iter().map(|&i| trips.alternatives[i]))),
    ];
    Ok(RecordBatch::try_new(Arc::new(schema()), columns)?)
}

/// A trip index as a raw id.
fn as_u32(i: usize) -> u32 {
    u32::try_from(i).expect("trip ids fit u32")
}

/// The file's columns, in order: the writer writes them and the reader asks for exactly them.
pub(crate) fn schema() -> Schema {
    Schema::new(vec![
        Field::new("trip_id", DataType::Utf8, false),
        Field::new("class_id", DataType::Utf8, false),
        Field::new("departure_s", DataType::UInt32, false),
        Field::new("mode", DataType::Utf8, true),
        Field::new("arrival_s", DataType::Float64, true),
        Field::new("travel_time_s", DataType::Float64, true),
        Field::new("finished", DataType::Boolean, false),
        Field::new("cost", DataType::Float64, true),
        Field::new("alternatives", DataType::UInt32, false),
    ])
}
