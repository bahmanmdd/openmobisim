//! The three tables a caller gives, `nodes`, `links` and `trips`, read into the crates' rows.
//!
//! A table is either in memory or the path of a Parquet file (a `str` or an `os.PathLike`, as
//! `os.fspath` reads it). In memory it is anything that gives a column by name, `table[name]`:
//! a dict of lists, a pandas or polars frame, a pyarrow table. Its column names are read from
//! `column_names` (pyarrow), else `columns` (pandas, polars), else `keys()` (a dict). Each
//! needed column is read once into a plain list: by `to_pylist()` when the column has one
//! (pyarrow), else by iterating it. A file is read whole by `openmobisim_io_parquet::read_table`
//! (uncompressed or snappy), its column names from its schema, and each needed column, in its
//! own kind (text, integer, float), into the same plain list of Python values; from there both
//! are read by the same code, so a file and the same table in memory give the same rows. The
//! column names are fixed, with no aliases: a missing column is refused by name.
//!
//! What cannot be used is set right or left out and noted in the reader's [`Repairs`] by kind
//! (the key, then what it completes "N …" with), never refused:
//!
//! | Key | What | What is done |
//! |---|---|---|
//! | `columns_ignored` | a column the table does not read (`name`, `osm_id`) | ignored |
//! | `nodes_unread`, `links_unread`, `trips_unread` | a row with a cell that cannot be read as its field (the cell table below) | left out (a node with its links) |
//! | `road_classes_unread` | a `road_class` cell that is not text (a number, a boolean) | no class; the link kept |
//! | `shapes_unread` (the graph's key) | a `shape` cell that is not text, or a text that is not points `x y;x y` of finite numbers | drawn straight; the link kept |
//! | `mode_names_unknown` | a row naming a mode other than car, bike, walk, transit (after trimming and lower-casing: `Car`, ` bike` are read) | the name left out of its set; an empty set is the crates' (a link or a trip with no mode, left out there) |
//!
//! How a cell is read as its field ([`Cell`]): text from a string, or from a whole number (an
//! integer, or a float with no fraction) written in decimal (`101`, not `101.0`); an integer
//! (lanes, a second of the clock) from an integer in the field's range, a float with no fraction,
//! or a string of one; a float from an integer, a float, or a string of one. A boolean is no
//! number. Where the field is optional, `None`, a float `NaN` and pandas' `NA` are null. Anything
//! else cannot be read (a fractional or negative second, 300 lanes, a null where none is
//! allowed). A position that is a float `NaN` is a number: the crates leave it out.

use std::path::{Path, PathBuf};

use openmobisim_demand::input::TripRow;
use openmobisim_graph::geometry::{LonLat, Position, Xy};
use openmobisim_graph::network::{LinkDrawing, LinkRow, NetworkBuilder, SHAPES_UNREAD};
use openmobisim_io_parquet::{Cells, InputTable, ReadError, read_table};
use openmobisim_types::mode::{Mode, ModeSet};
use openmobisim_types::repairs::Repairs;
use openmobisim_types::time::Second;
use pyo3::exceptions::{PyOSError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyList, PyString};

/// The optional columns of the links table, for drawing only: `shape` (the points between the
/// two ends, text `x y;x y`, of the nodes' kind) and `road_class` (text). Absent or null: none.
const LINK_DRAWING_COLUMNS: [&str; 2] = ["shape", "road_class"];

/// The columns of the links table, in the order the README lists them.
const LINK_COLUMNS: [&str; 8] = [
    "link_id",
    "upstream",
    "downstream",
    "modes",
    "length_m",
    "lanes",
    "car_speed_km_h",
    "capacity_veh_h",
];

/// How a cell is read as a row field (the module's cell table).
trait Cell: Sized + Default {
    /// The field `v` is, or `None` when it cannot be read as one.
    fn read(v: &Bound<'_, PyAny>) -> Option<Self>;
}

/// `v` as a whole number: an integer (not a boolean), a float with no fraction, or a string of
/// either.
fn whole(v: &Bound<'_, PyAny>) -> Option<i128> {
    #[allow(
        clippy::cast_possible_truncation,
        reason = "a whole float below 2^127 in magnitude converts exactly"
    )]
    let of = |x: f64| (x.is_finite() && x.fract() == 0.0 && x.abs() < 1.0e38).then_some(x as i128);
    if v.is_instance_of::<PyBool>() {
        return None;
    }
    if let Ok(text) = v.cast::<PyString>() {
        let text = text.to_str().ok()?.trim();
        return text.parse::<i128>().ok().or_else(|| text.parse::<f64>().ok().and_then(of));
    }
    v.extract::<i128>().ok().or_else(|| v.extract::<f64>().ok().and_then(of))
}

impl Cell for String {
    fn read(v: &Bound<'_, PyAny>) -> Option<Self> {
        match v.cast::<PyString>() {
            Ok(text) => text.to_str().ok().map(str::to_owned),
            Err(_) => whole(v).map(|n| n.to_string()),
        }
    }
}

impl Cell for u8 {
    fn read(v: &Bound<'_, PyAny>) -> Option<Self> {
        whole(v).and_then(|n| Self::try_from(n).ok())
    }
}

impl Cell for u32 {
    fn read(v: &Bound<'_, PyAny>) -> Option<Self> {
        whole(v).and_then(|n| Self::try_from(n).ok())
    }
}

impl Cell for f64 {
    fn read(v: &Bound<'_, PyAny>) -> Option<Self> {
        if v.is_instance_of::<PyBool>() {
            return None;
        }
        match v.cast::<PyString>() {
            Ok(text) => text.to_str().ok()?.trim().parse().ok(),
            Err(_) => v.extract().ok(),
        }
    }
}

impl<T: Cell> Cell for Option<T> {
    fn read(v: &Bound<'_, PyAny>) -> Option<Self> {
        let nan = v.extract::<f64>().is_ok_and(f64::is_nan) && !v.is_instance_of::<PyString>();
        let na = v.get_type().name().is_ok_and(|n| n == "NAType");
        if v.is_none() || nan || na { Some(None) } else { T::read(v).map(Some) }
    }
}

/// The path a table argument gives in place of a table: a `str` or an `os.PathLike`, as
/// `os.fspath` reads it; `None` for a table in memory.
pub(crate) fn path(table: &Bound<'_, PyAny>) -> Option<PathBuf> {
    table.extract::<PathBuf>().ok()
}

/// A table's file that cannot be read: an `OSError` when it cannot be opened or read, else a
/// `ValueError`, each naming the table and the file.
fn file_error(what: &str, path: &Path, e: &ReadError) -> PyErr {
    let message = format!("{what} ({}): {e}", path.display());
    match e {
        ReadError::Io(_) => PyOSError::new_err(message),
        _ => PyValueError::new_err(message),
    }
}

/// Where a table's columns come from: an object in memory, or a Parquet file read whole.
enum Source<'py> {
    Memory(Bound<'py, PyAny>),
    File(InputTable),
}

/// One table being read: the interpreter, its name for the messages, its source, its column
/// names, the row count of the first column read, and per row the first column whose cell could
/// not be read.
struct Table<'py> {
    py: Python<'py>,
    what: &'static str,
    source: Source<'py>,
    names: Vec<String>,
    rows: Option<usize>,
    unread: std::collections::BTreeMap<usize, String>,
}

impl<'py> Table<'py> {
    fn new(what: &'static str, table: &Bound<'py, PyAny>) -> PyResult<Self> {
        if let Some(path) = path(table) {
            let read = table.py().detach(|| read_table(&path));
            let file = read.map_err(|e| file_error(what, &path, &e))?;
            let names = file.names();
            let source = Source::File(file);
            return Ok(Self {
                py: table.py(),
                what,
                source,
                names,
                rows: None,
                unread: Default::default(),
            });
        }
        let names = if table.hasattr("column_names")? {
            table.getattr("column_names")?
        } else if table.hasattr("columns")? {
            table.getattr("columns")?
        } else {
            table.call_method0("keys")?
        };
        let names = names.try_iter()?.map(|n| n?.extract::<String>()).collect::<PyResult<_>>()?;
        let source = Source::Memory(table.clone());
        Ok(Self { py: table.py(), what, source, names, rows: None, unread: Default::default() })
    }

    /// The column `name` as a plain Python list: from memory by `to_pylist()` when the column
    /// has one, else the column itself, iterated by the caller; from a file, its cells of its own
    /// kind, a null as `None`.
    fn list(&self, name: &str) -> PyResult<Bound<'py, PyAny>> {
        match &self.source {
            Source::Memory(table) => {
                let column = table.get_item(name)?;
                if column.hasattr("to_pylist")? {
                    column.call_method0("to_pylist")
                } else {
                    Ok(column)
                }
            }
            Source::File(file) => {
                let cells = file
                    .column(name)
                    .map_err(|e| PyValueError::new_err(format!("{}: {e}", self.what)))?;
                Ok(match cells {
                    Cells::Text(v) => PyList::new(self.py, v)?.into_any(),
                    Cells::Integer(v) => PyList::new(self.py, v)?.into_any(),
                    Cells::Float(v) => PyList::new(self.py, v)?.into_any(),
                })
            }
        }
    }

    fn has(&self, name: &str) -> bool {
        self.names.iter().any(|n| n == name)
    }

    /// The table holds `columns`, in any order, and of `optional` any: a missing one is
    /// refused by name; any other is ignored, noted in `repairs` as `columns_ignored`.
    fn expect(&self, columns: &[&str], optional: &[&str], repairs: &mut Repairs) -> PyResult<()> {
        if let Some(missing) = columns.iter().find(|c| !self.has(c)) {
            let mut list = columns.join(", ");
            if !optional.is_empty() {
                list = format!("{list}, and optionally {}", optional.join(", "));
            }
            return Err(PyValueError::new_err(format!(
                "{}: missing column {missing}; the columns are {list}",
                self.what
            )));
        }
        let known = |n: &str| columns.contains(&n) || optional.contains(&n);
        for other in self.names.iter().filter(|n| !known(n)) {
            let what = "columns a table does not read: ignored";
            repairs.note("columns_ignored", what, format!("{}: {other}", self.what));
        }
        Ok(())
    }

    /// The column `name` as a list of `T` ([`Cell`]), each row read once; a row whose cell
    /// cannot be read gets `T`'s default and is marked unread (the first such column kept).
    fn column<T: Cell>(&mut self, name: &str) -> PyResult<Vec<T>> {
        let cells = self.cells::<T>(name)?;
        let mut values = Vec::with_capacity(cells.len());
        for (row, cell) in cells.into_iter().enumerate() {
            values.push(cell.unwrap_or_else(|| {
                self.unread.entry(row).or_insert_with(|| name.to_owned());
                T::default()
            }));
        }
        Ok(values)
    }

    /// The column `name` as a list of `T` ([`Cell`]), `None` for a cell that cannot be read (no
    /// row is marked: the caller decides). A column of another length than the first read is
    /// refused by name: which cell is whose row cannot be known.
    fn cells<T: Cell>(&mut self, name: &str) -> PyResult<Vec<Option<T>>> {
        let list = self.list(name)?;
        let mut values = Vec::new();
        for value in list.try_iter()? {
            values.push(T::read(&value?));
        }
        match self.rows {
            Some(rows) if rows != values.len() => Err(PyValueError::new_err(format!(
                "{}: column {name} has {} rows, the columns before it {rows}",
                self.what,
                values.len()
            ))),
            _ => {
                self.rows = Some(values.len());
                Ok(values)
            }
        }
    }

    /// Per row, whether every cell was read; each row that was not is noted in `repairs` under
    /// `key`, by its id in `ids` and the first column not read (`row N` when its id, the column
    /// `id`, is the one not read).
    fn read_rows(
        &self,
        ids: &[String],
        id: &str,
        key: &'static str,
        repairs: &mut Repairs,
    ) -> Vec<bool> {
        let what = "rows with a cell that cannot be read as its field: left out";
        let mut kept = vec![true; ids.len()];
        for (&row, column) in &self.unread {
            kept[row] = false;
            let named = if column == id {
                format!("row {row}")
            } else {
                format!("{} ({column})", ids[row])
            };
            repairs.note(key, what, named);
        }
        kept
    }

    /// Whether the positions are longitude/latitude (`<prefix>lon_deg`, `<prefix>lat_deg`) or
    /// metres (`<prefix>x_m`, `<prefix>y_m`): told by the column names, longitude/latitude when
    /// either of its two names is there. Returns the kind and the two names.
    fn kind(&self, prefix: &str) -> (bool, String, String) {
        let (lon, lat) = (format!("{prefix}lon_deg"), format!("{prefix}lat_deg"));
        if self.has(&lon) || self.has(&lat) {
            (true, lon, lat)
        } else {
            (false, format!("{prefix}x_m"), format!("{prefix}y_m"))
        }
    }

    /// The positions under the two columns of `kind`.
    fn positions(&mut self, (lon_lat, a, b): &(bool, String, String)) -> PyResult<Vec<Position>> {
        let xs: Vec<f64> = self.column(a)?;
        let ys: Vec<f64> = self.column(b)?;
        Ok(xs
            .into_iter()
            .zip(ys)
            .map(|(x, y)| {
                if *lon_lat {
                    Position::LonLat(LonLat::new(x, y))
                } else {
                    Position::Metres(Xy::new(x, y))
                }
            })
            .collect())
    }
}

/// A mode set written as mode names joined by `;` (`car;bike`), each of `car`, `bike`, `walk`,
/// `transit`, read trimmed and lower-cased; the empty string is the empty set. A name still
/// unknown is left out, noted in `repairs` under `mode_names_unknown` with `id`.
fn modes(text: &str, id: &str, repairs: &mut Repairs) -> ModeSet {
    let mut set = ModeSet::EMPTY;
    for name in text.split(';').map(|n| n.trim().to_lowercase()).filter(|n| !n.is_empty()) {
        match Mode::ALL.into_iter().find(|m| m.to_string() == name) {
            Some(m) => set = set.with(m),
            None => {
                let what =
                    "rows naming a mode that is not car, bike, walk or transit: the name left out";
                repairs.note("mode_names_unknown", what, format!("{id} ({name})"));
            }
        }
    }
    set
}

/// A shape's text, `x y;x y` (`lon lat;lon lat` when the nodes are in longitude/latitude): the
/// points between a link's two ends, each two finite numbers apart by one space; the empty text
/// is no point (straight). `None` when it is not that: the graph draws the link straight, noted.
fn points(text: &str, lon_lat: bool) -> Option<Vec<Position>> {
    if text.is_empty() {
        return Some(Vec::new());
    }
    text.split(';')
        .map(|point| {
            let xy: Vec<f64> = point.split(' ').map(str::parse).collect::<Result<_, _>>().ok()?;
            match xy[..] {
                [x, y] if !(x.is_finite() && y.is_finite()) => None,
                [x, y] if lon_lat => Some(Position::LonLat(LonLat::new(x, y))),
                [x, y] => Some(Position::Metres(Xy::new(x, y))),
                _ => None,
            }
        })
        .collect()
}

/// The nodes and links tables into a network builder, and what their reading repaired.
pub(crate) fn network(
    nodes: &Bound<'_, PyAny>,
    links: &Bound<'_, PyAny>,
) -> PyResult<(NetworkBuilder, Repairs)> {
    let (mut builder, mut repairs) = (NetworkBuilder::new(), Repairs::new());
    let mut t = Table::new("nodes", nodes)?;
    let kind = t.kind("");
    t.expect(&["node_id", &kind.1, &kind.2], &[], &mut repairs)?;
    let ids: Vec<String> = t.column("node_id")?;
    let at = t.positions(&kind)?;
    let kept = t.read_rows(&ids, "node_id", "nodes_unread", &mut repairs);
    for ((id, at), _) in ids.into_iter().zip(at).zip(kept).filter(|(_, k)| *k) {
        builder.add_node(id, at);
    }

    let mut t = Table::new("links", links)?;
    t.expect(&LINK_COLUMNS, &LINK_DRAWING_COLUMNS, &mut repairs)?;
    let ids: Vec<String> = t.column("link_id")?;
    let upstream: Vec<String> = t.column("upstream")?;
    let downstream: Vec<String> = t.column("downstream")?;
    let mode_names: Vec<String> = t.column("modes")?;
    let length: Vec<Option<f64>> = t.column("length_m")?;
    let lanes: Vec<Option<u8>> = t.column("lanes")?;
    let speed: Vec<Option<f64>> = t.column("car_speed_km_h")?;
    let capacity: Vec<Option<f64>> = t.column("capacity_veh_h")?;
    // The drawing columns: a cell that cannot be read is read as null and noted below, the link
    // kept (`None`: not read; `Some(None)`: null).
    let none = vec![Some(None); ids.len()];
    let shape: Vec<Option<Option<String>>> =
        if t.has("shape") { t.cells("shape")? } else { none.clone() };
    let class: Vec<Option<Option<String>>> =
        if t.has("road_class") { t.cells("road_class")? } else { none };
    let kept = t.read_rows(&ids, "link_id", "links_unread", &mut repairs);
    for (i, id) in ids.into_iter().enumerate().filter(|&(i, _)| kept[i]) {
        let row = LinkRow {
            modes: modes(&mode_names[i], &id, &mut repairs),
            lanes: lanes[i],
            car_speed_km_h: speed[i],
            length_m: length[i],
            capacity_veh_h: capacity[i],
            signal_at_downstream_end: false,
        };
        builder.add_link(id.clone(), upstream[i].clone(), downstream[i].clone(), row);
        let road_class = class[i].clone().unwrap_or_else(|| {
            let what = "road classes that are not text: no class";
            repairs.note("road_classes_unread", what, id.clone());
            None
        });
        // An empty class's text is no class.
        let road_class = road_class.filter(|c| !c.is_empty());
        // A shape that is not points ("x y;x y", each finite) is drawn straight, noted under the
        // graph's key, which also counts the points it cannot place.
        let points = match &shape[i] {
            Some(None) => Some(Vec::new()),
            Some(Some(text)) => points(text, kind.0),
            None => None,
        };
        let shape = points.unwrap_or_else(|| {
            repairs.note(SHAPES_UNREAD.0, SHAPES_UNREAD.1, id.clone());
            Vec::new()
        });
        if !shape.is_empty() || road_class.is_some() {
            builder.add_drawing(id, LinkDrawing { shape, road_class });
        }
    }
    Ok((builder, repairs))
}

/// The trips table into the demand crate's rows, what its reading repaired, and how many rows it
/// held.
pub(crate) fn trips(trips: &Bound<'_, PyAny>) -> PyResult<(Vec<TripRow>, Repairs, u64)> {
    let mut repairs = Repairs::new();
    let mut t = Table::new("trips", trips)?;
    let (origin, destination) = (t.kind("origin_"), t.kind("destination_"));
    t.expect(
        &[
            "trip_id",
            &origin.1,
            &origin.2,
            &destination.1,
            &destination.2,
            "departure_s",
            "class_id",
            "modes",
        ],
        &[],
        &mut repairs,
    )?;
    let ids: Vec<String> = t.column("trip_id")?;
    let from = t.positions(&origin)?;
    let to = t.positions(&destination)?;
    let departure: Vec<u32> = t.column("departure_s")?;
    let class: Vec<String> = t.column("class_id")?;
    let mode_names: Vec<String> = t.column("modes")?;
    let kept = t.read_rows(&ids, "trip_id", "trips_unread", &mut repairs);
    let given = ids.len() as u64;
    let mut rows = Vec::with_capacity(ids.len());
    for (i, id) in ids.into_iter().enumerate().filter(|&(i, _)| kept[i]) {
        let modes = modes(&mode_names[i], &id, &mut repairs);
        rows.push(TripRow {
            id,
            origin: from[i],
            destination: to[i],
            departure: Second(departure[i]),
            class: class[i].clone(),
            modes,
        });
    }
    Ok((rows, repairs, given))
}
