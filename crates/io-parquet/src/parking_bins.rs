//! `parking_bins.parquet` — how full every parking was over the day, bin by bin.
//!
//! One row per parking and time bin, parking by parking:
//! `run_id, bin, start_s, parking, parking_id, hub_id, vehicle, capacity,
//! arrivals, departures, occupancy_mean, occupancy_max, full_s, overflow_max, hub_group`.
//! The same table `Run.parking_bins()` returns in Python.
//!
//! - `bin` counts from **midnight**, like `link_bins.parquet`'s: `bin = start_s /
//!   bin_s`, so the first row of a window starting at 07:30 with 15-minute bins has `bin` 30 and
//!   `start_s` 27 000. The file records `bin_s` as `openmobisim.bin_seconds`.
//! - `parking` is the parking's index in the run, `parking_id` its id in the
//!   parking table, `hub_id` its hub's (the model's); `vehicle` is `car` or `bike`;
//!   `hub_group` the group it shares with the stops beside it (C4: a label,
//!   `openmobisim_core_sim::hub_groups`).
//! - `arrivals` and `departures` count vehicles parked and fetched in the bin,
//!   weighted by traveller weight; `occupancy_mean` and `occupancy_max` the
//!   vehicles parked; `full_s` the seconds of the bin the parking was full;
//!   `overflow_max` the most vehicles above capacity at once (soft capacity: a
//!   full parking refuses nobody).
//! - The key-value metadata carries `openmobisim.bin_seconds`, `openmobisim.run_fingerprint` and
//!   `openmobisim.master_seed`.
//!
//! **Cost.** About 80 bytes per row while writing: a city's thousand bike
//! parkings over a day of 15-minute bins are about 100 000 rows.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use arrow_array::{ArrayRef, Float64Array, RecordBatch, StringArray, UInt32Array};
use arrow_schema::{DataType, Field, Schema};
use openmobisim_core_sim::{ParkingResult, ParkingSetup, RunDescription};
use parquet::basic::Compression;

use crate::WriteError;
use crate::io::write_single_batch_with;

/// Write `parking_bins.parquet` for one run.
///
/// # Errors
///
/// [`WriteError`] if the file cannot be created or the Parquet writer rejects
/// the data.
///
/// # Panics
///
/// Panics if `result` was not made with `parking` (its arrays have another length).
pub fn write_parking_bins(
    path: impl AsRef<Path>,
    run_id: &str,
    parking: &ParkingSetup,
    result: &ParkingResult,
    description: &RunDescription,
    parking_groups: &[String],
) -> Result<(), WriteError> {
    let b = &result.bins;
    let parkings = parking.count();
    let n = parkings * b.bins;
    assert_eq!(b.arrivals.len(), n, "the result is of these parkings");
    let mut bin = Vec::with_capacity(n);
    let mut start = Vec::with_capacity(n);
    let mut index = Vec::with_capacity(n);
    let mut id = Vec::with_capacity(n);
    let mut hub = Vec::with_capacity(n);
    let mut vehicle = Vec::with_capacity(n);
    let mut capacity = Vec::with_capacity(n);
    let mut group = Vec::with_capacity(n);
    for (p, parking_group) in parking_groups.iter().enumerate().take(parkings) {
        let p32 = u32::try_from(p).expect("parkings fit u32");
        for k in 0..b.bins {
            let k32 = u32::try_from(k).expect("bins fit u32");
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "seconds")]
            let start_s = b.start_s as u32 + k32 * b.bin_s;
            bin.push(start_s / b.bin_s); // from midnight, like link_bins (G-31)
            start.push(start_s);
            index.push(p32);
            id.push(parking.external_id(p32));
            hub.push(parking.hub_external_id(p32));
            vehicle.push(parking.kind(p32).as_str());
            capacity.push(parking.capacity(p32));
            group.push(parking_group.as_str());
        }
    }
    let columns: Vec<ArrayRef> = vec![
        Arc::new(StringArray::from(vec![run_id; n])),
        Arc::new(UInt32Array::from(bin)),
        Arc::new(UInt32Array::from(start)),
        Arc::new(UInt32Array::from(index)),
        Arc::new(StringArray::from(id)),
        Arc::new(StringArray::from(hub)),
        Arc::new(StringArray::from(vehicle)),
        Arc::new(UInt32Array::from(capacity)),
        Arc::new(Float64Array::from(b.arrivals.clone())),
        Arc::new(Float64Array::from(b.departures.clone())),
        Arc::new(Float64Array::from(b.occupancy_mean.clone())),
        Arc::new(Float64Array::from(b.occupancy_max.clone())),
        Arc::new(Float64Array::from(b.full_s.clone())),
        Arc::new(Float64Array::from(
            b.overflow_max.iter().map(|&v| v.max(0.0)).collect::<Vec<_>>(),
        )),
        Arc::new(StringArray::from(group)),
    ];
    let metadata = vec![
        ("openmobisim.bin_seconds".to_string(), b.bin_s.to_string()),
        ("openmobisim.run_fingerprint".to_string(), description.fingerprint_hex()),
        ("openmobisim.master_seed".to_string(), description.master_seed.to_string()),
    ];
    let field = |name: &str, t: DataType| Field::new(name, t, false);
    let schema = Arc::new(
        Schema::new(vec![
            field("run_id", DataType::Utf8),
            field("bin", DataType::UInt32),
            field("start_s", DataType::UInt32),
            field("parking", DataType::UInt32),
            field("parking_id", DataType::Utf8),
            field("hub_id", DataType::Utf8),
            field("vehicle", DataType::Utf8),
            field("capacity", DataType::UInt32),
            field("arrivals", DataType::Float64),
            field("departures", DataType::Float64),
            field("occupancy_mean", DataType::Float64),
            field("occupancy_max", DataType::Float64),
            field("full_s", DataType::Float64),
            field("overflow_max", DataType::Float64),
            field("hub_group", DataType::Utf8),
        ])
        .with_metadata(metadata.iter().cloned().collect::<HashMap<_, _>>()),
    );
    let batch = RecordBatch::try_new(schema.clone(), columns)?;
    write_single_batch_with(path.as_ref(), schema, &batch, &metadata, Compression::SNAPPY)
}
