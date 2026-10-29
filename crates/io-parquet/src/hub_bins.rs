//! `hub_bins.parquet` — every shared-bike dock over a run's last loading, bin by bin, with its hub.
//!
//! One row per dock and time bin, service by service, dock by dock (dock id order), bin by bin:
//! `run_id, bin, start_s, hub_id, hub_kind, resource_kind, resource_id, service, level_mean,
//! level_min, capacity, empty_s, full_s, pickups, returns, failed, redirected_in, redirected_out,
//! moved_in, moved_out`. Written only by a run with a service; the same table `Run.hub_bins()`
//! returns in Python. Parkings keep `parking_bins.parquet`.
//!
//! - `bin` counts from **midnight**, like `parking_bins.parquet`'s: `bin = start_s / bin_s`; the
//!   bins are the service's `bin_s` (900 s by default), recorded as `openmobisim.bin_seconds`.
//! - `hub_id` is the dock's hub in the run's hub set (the hub its row names, else the nearest
//!   stop's within `hub_join_m`, else its own); `hub_kind` is `stop` or `declared` (a hub made
//!   for the dock alone, or named by its row); `resource_kind` is `dock`; `resource_id` the
//!   dock's id; `service` the service's name.
//! - `level_mean` and `level_min` are the bikes docked over the bin (time-weighted mean; fewest at
//!   once), `capacity` the dock's slots; `empty_s` and `full_s` the seconds of the bin it held no
//!   bike, and had every slot taken.
//! - Counts of people (bikes) in the bin: `pickups`, `returns`, `failed` (pickups that failed
//!   here, the last dock tried), `redirected_in` (travellers sent on to this dock from an empty or
//!   full one, counted in the bin they were sent), `redirected_out` (travellers who found it empty
//!   when picking up, or full when returning, and went on), `moved_in` and `moved_out` (bikes an
//!   operator's van brought or took away: 0 in a run, which has no operator yet).
//! - The key-value metadata carries `openmobisim.bin_seconds`, `openmobisim.run_fingerprint` and
//!   `openmobisim.master_seed`.
//!
//! **Cost.** About 120 bytes per row while writing: a city's 1 500 docks over a day of 15-minute
//! bins are about 150 000 rows.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use arrow_array::{ArrayRef, Float64Array, RecordBatch, StringArray, UInt32Array};
use arrow_schema::{DataType, Field, Schema};
use openmobisim_core_sim::RunDescription;
use openmobisim_core_sim::services::tables::HubBinTable;
use parquet::basic::Compression;

use crate::WriteError;
use crate::io::write_single_batch_with;

/// Write `hub_bins.parquet` for one run.
///
/// # Errors
///
/// [`WriteError`] if the file cannot be created or the Parquet writer rejects the data.
pub fn write_hub_bins(
    path: impl AsRef<Path>,
    run_id: &str,
    bins: &HubBinTable,
    description: &RunDescription,
) -> Result<(), WriteError> {
    let n = bins.len();
    let bin_s = bins.bin_s.max(1);
    let text = |v: &[String]| -> ArrayRef {
        Arc::new(StringArray::from(v.iter().map(String::as_str).collect::<Vec<_>>()))
    };
    let float = |v: &[f64]| -> ArrayRef { Arc::new(Float64Array::from(v.to_vec())) };
    let columns: Vec<ArrayRef> = vec![
        Arc::new(StringArray::from(vec![run_id; n])),
        Arc::new(UInt32Array::from(bins.start_s.iter().map(|s| s / bin_s).collect::<Vec<_>>())),
        Arc::new(UInt32Array::from(bins.start_s.clone())),
        text(&bins.hub_id),
        Arc::new(StringArray::from(bins.hub_kind.clone())),
        Arc::new(StringArray::from(bins.resource_kind.clone())),
        text(&bins.resource_id),
        text(&bins.service),
        float(&bins.level_mean),
        float(&bins.level_min),
        Arc::new(UInt32Array::from(bins.capacity.clone())),
        float(&bins.empty_s),
        float(&bins.full_s),
        float(&bins.pickups),
        float(&bins.returns),
        float(&bins.failed),
        float(&bins.redirected_in),
        float(&bins.redirected_out),
        float(&bins.moved_in),
        float(&bins.moved_out),
    ];
    let metadata = vec![
        ("openmobisim.bin_seconds".to_string(), bin_s.to_string()),
        ("openmobisim.run_fingerprint".to_string(), description.fingerprint_hex()),
        ("openmobisim.master_seed".to_string(), description.master_seed.to_string()),
    ];
    let field = |name: &str, t: DataType| Field::new(name, t, false);
    let (s, u, f) = (DataType::Utf8, DataType::UInt32, DataType::Float64);
    let mut fields = vec![
        field("run_id", s.clone()),
        field("bin", u.clone()),
        field("start_s", u.clone()),
        field("hub_id", s.clone()),
        field("hub_kind", s.clone()),
        field("resource_kind", s.clone()),
        field("resource_id", s.clone()),
        field("service", s),
        field("level_mean", f.clone()),
        field("level_min", f.clone()),
        field("capacity", u),
    ];
    for name in [
        "empty_s",
        "full_s",
        "pickups",
        "returns",
        "failed",
        "redirected_in",
        "redirected_out",
        "moved_in",
        "moved_out",
    ] {
        fields.push(field(name, f.clone()));
    }
    let schema = Arc::new(
        Schema::new(fields).with_metadata(metadata.iter().cloned().collect::<HashMap<_, _>>()),
    );
    let batch = RecordBatch::try_new(schema.clone(), columns)?;
    write_single_batch_with(path.as_ref(), schema, &batch, &metadata, Compression::SNAPPY)
}
