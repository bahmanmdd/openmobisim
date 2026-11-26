//! A case made from an OpenStreetMap extract (experimental): the trips drawn from its streets
//! as `trips.parquet`, a trips table as `Demand.from_table` reads it, and the settings file
//! `run.toml` naming the extract, its region and car roads, the trips, their classes and the
//! GTFS feed, so that the folder runs as any case. Bikes: a trip's `modes` hold `bike` for the
//! classes that own one.

use std::path::Path;
use std::sync::Arc;

use arrow_array::{Float64Array, RecordBatch, StringArray, UInt32Array};
use arrow_schema::{DataType, Field, Schema};
use openmobisim_demand::input::TripRow;
use openmobisim_graph::geometry::Position;
use openmobisim_io_osm::Report;
use openmobisim_types::time::Second;

use crate::{Files, Table, WriteError, column, file, record};

/// Write `trips` as `trips.parquet` and the settings as `run.toml` in `folder`, created if
/// missing: the run's `window`, the extract `osm` read as `report` says (region, car roads),
/// the feed `gtfs` if any, and `classes`. `osm` and `gtfs` are canonical paths, written
/// relative to the folder.
///
/// # Errors
///
/// [`WriteError`]: the folder or a file cannot be written, or a path is not text.
pub fn write_case(
    folder: &Path,
    window: (Second, Second),
    (osm, report): (&Path, &Report),
    gtfs: Option<&Path>,
    classes: Vec<&str>,
    trips: &[TripRow],
) -> Result<Table, WriteError> {
    std::fs::create_dir_all(folder)?;
    let folder = std::fs::canonicalize(folder)?;
    let trips_file = folder.join("trips.parquet");
    let files = Files { osm: Some(osm), trips: Some(&trips_file), gtfs, ..Files::default() };
    let settings = record::case_text(window, report, &files, classes, &folder)?;
    let degrees = matches!(trips.first().map(|t| t.origin), Some(Position::LonLat(_)) | None);
    let names = if degrees { ["lon_deg", "lat_deg"] } else { ["x_m", "y_m"] };
    let pair = |p: Position| match p {
        Position::LonLat(at) => (at.lon, at.lat),
        Position::Metres(at) => (at.x, at.y),
    };
    let text = |of: &dyn Fn(&TripRow) -> String| {
        column(StringArray::from_iter_values(trips.iter().map(of)))
    };
    let float =
        |of: &dyn Fn(&TripRow) -> f64| column(Float64Array::from_iter_values(trips.iter().map(of)));
    let modes = |t: &TripRow| t.modes.iter().map(|m| m.to_string()).collect::<Vec<_>>().join(";");
    let field = |name: String, kind| Field::new(name, kind, false);
    let schema = Schema::new(vec![
        field("trip_id".into(), DataType::Utf8),
        field(format!("origin_{}", names[0]), DataType::Float64),
        field(format!("origin_{}", names[1]), DataType::Float64),
        field(format!("destination_{}", names[0]), DataType::Float64),
        field(format!("destination_{}", names[1]), DataType::Float64),
        field("departure_s".into(), DataType::UInt32),
        field("class_id".into(), DataType::Utf8),
        field("modes".into(), DataType::Utf8),
    ]);
    let columns = vec![
        text(&|t| t.id.clone()),
        float(&|t| pair(t.origin).0),
        float(&|t| pair(t.origin).1),
        float(&|t| pair(t.destination).0),
        float(&|t| pair(t.destination).1),
        column(UInt32Array::from_iter_values(trips.iter().map(|t| t.departure.get()))),
        text(&|t| t.class.clone()),
        text(&modes),
    ];
    let written =
        file(&folder, "trips.parquet", &RecordBatch::try_new(Arc::new(schema), columns)?)?;
    std::fs::write(folder.join("run.toml"), settings)?;
    Ok(written)
}
