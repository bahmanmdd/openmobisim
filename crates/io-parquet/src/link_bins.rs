//! `link_bins.parquet` — the per-link, per-time-bin results (S168).
//!
//! One row per (bin, link) that saw traffic, sorted by bin then link:
//! `run_id, bin, start_s, link, link_index, crossings, pcu, pcu_seconds`. The
//! same table `RunResult::link_bins` holds in memory and the figures draw, in
//! the form an analyst reads with pandas, polars or R.
//!
//! - `link` is the link's **external id**, the one that survives a rebuilt
//!   network; `link_index` is the internal id, the row of the network's arrays,
//!   good only next to this file's `network_fingerprint` (Foundations §1: an
//!   internal id is meaningful only with its network fingerprint).
//! - `crossings` counts traversals that *finished* in the bin, unweighted;
//!   `pcu` is the traffic that left the link, traveller weight and vehicle size
//!   included; `pcu_seconds` is the PCU-weighted time those traversals took, so
//!   `pcu_seconds / pcu` is the mean traversal time and `pcu × 3600 /
//!   bin_seconds` the flow in PCU per hour. Rates and times are derived by the
//!   reader, not stored twice.
//! - The file's key-value metadata carries what makes the rows readable on
//!   their own: `openmobisim.bin_seconds`, `openmobisim.run_fingerprint`,
//!   `openmobisim.network_fingerprint` and `openmobisim.master_seed`.
//!
//! **Cost.** 28 bytes per row in memory; in the file **21 bytes per row**
//! (Snappy: the codec every Parquet reader has, and fast to write) — Luxembourg's
//! 468 000 rows are 10 MB and write in 0.07 s. The link ids and the constant run id
//! are dictionary-encoded, so the two id columns cost little; the time column is
//! the bulk, since real numbers do not compress. Brotli would make it 6.8 MB at 0.5 s.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use arrow_array::{ArrayRef, Float64Array, RecordBatch, StringArray, UInt32Array};
use arrow_schema::{DataType, Field, Schema};
use openmobisim_core_graph::layers::StaticLayer;
use openmobisim_core_graph::link_geometry::NetworkFingerprint;
use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_loading::LinkBins;
use openmobisim_core_sim::RunDescription;
use parquet::basic::Compression;

use crate::WriteError;
use crate::io::write_single_batch_with;

/// Write `link_bins.parquet` for one run.
///
/// # Errors
///
/// [`WriteError`] if the file cannot be created or the Parquet writer rejects
/// the data.
pub fn write_link_bins(
    path: impl AsRef<Path>,
    run_id: &str,
    bins: &LinkBins,
    network: &RoadNetwork,
    description: &RunDescription,
) -> Result<(), WriteError> {
    write(path.as_ref(), run_id, bins, network, description, None)
}

/// Write a bike or walk layer's per-link results (S195): `link_bins_bike.parquet`
/// or `link_bins_walk.parquet`, with the road file's columns except that the
/// weighted count is `travellers` and its time `traveller_seconds` — a bike
/// is no passenger-car unit. The metadata's network fingerprint is the
/// layer's, and `openmobisim.layer` names it.
///
/// # Errors
///
/// [`WriteError`] if the file cannot be created or the Parquet writer rejects
/// the data.
pub fn write_layer_link_bins(
    path: impl AsRef<Path>,
    run_id: &str,
    layer: StaticLayer,
    bins: &LinkBins,
    graph: &RoadNetwork,
    description: &RunDescription,
) -> Result<(), WriteError> {
    write(path.as_ref(), run_id, bins, graph, description, Some(layer))
}

fn write(
    path: &Path,
    run_id: &str,
    bins: &LinkBins,
    network: &RoadNetwork,
    description: &RunDescription,
    layer: Option<StaticLayer>,
) -> Result<(), WriteError> {
    let n = bins.len();
    let ids = network.link_external_ids();
    let bin_seconds = bins.bin_seconds();

    let run_id_col: ArrayRef = Arc::new(StringArray::from(vec![run_id; n]));
    let bin_col: ArrayRef = Arc::new(UInt32Array::from(bins.bins().to_vec()));
    let start_col: ArrayRef = Arc::new(UInt32Array::from(
        bins.bins().iter().map(|b| b * bin_seconds).collect::<Vec<_>>(),
    ));
    let link_col: ArrayRef = Arc::new(StringArray::from(
        bins.links().iter().map(|&l| ids.external(l)).collect::<Vec<_>>(),
    ));
    let index_col: ArrayRef = Arc::new(UInt32Array::from(bins.links().to_vec()));
    let crossings_col: ArrayRef = Arc::new(UInt32Array::from(bins.crossings().to_vec()));
    let pcu_col: ArrayRef = Arc::new(Float64Array::from(bins.pcu().to_vec()));
    let pcu_seconds_col: ArrayRef = Arc::new(Float64Array::from(bins.pcu_seconds().to_vec()));

    let (network_fingerprint, count, seconds) = match layer {
        None => (description.network_fingerprint_hex(), "pcu", "pcu_seconds"),
        Some(_) => (
            openmobisim_core_types::hash::fingerprint_hex(NetworkFingerprint::of(network).value()),
            "travellers",
            "traveller_seconds",
        ),
    };
    let mut metadata = vec![
        ("openmobisim.bin_seconds".to_string(), bin_seconds.to_string()),
        ("openmobisim.run_fingerprint".to_string(), description.fingerprint_hex()),
        ("openmobisim.network_fingerprint".to_string(), network_fingerprint),
        ("openmobisim.master_seed".to_string(), description.master_seed.to_string()),
    ];
    if let Some(layer) = layer {
        metadata.push(("openmobisim.layer".to_string(), layer.as_str().to_string()));
    }
    let schema = Arc::new(
        Schema::new(vec![
            Field::new("run_id", DataType::Utf8, false),
            Field::new("bin", DataType::UInt32, false),
            Field::new("start_s", DataType::UInt32, false),
            Field::new("link", DataType::Utf8, false),
            Field::new("link_index", DataType::UInt32, false),
            Field::new("crossings", DataType::UInt32, false),
            Field::new(count, DataType::Float64, false),
            Field::new(seconds, DataType::Float64, false),
        ])
        .with_metadata(metadata.iter().cloned().collect::<HashMap<_, _>>()),
    );
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            run_id_col,
            bin_col,
            start_col,
            link_col,
            index_col,
            crossings_col,
            pcu_col,
            pcu_seconds_col,
        ],
    )?;

    write_single_batch_with(path, schema, &batch, &metadata, Compression::SNAPPY)
}
