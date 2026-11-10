//! The three tables a caller gives, `nodes`, `links` and `trips`, read into the crates' rows.
//!
//! A table is anything that gives a column by name, `table[name]`: a dict of lists, a pandas or
//! polars frame, a pyarrow table. Its column names are read from `column_names` (pyarrow),
//! else `columns` (pandas, polars), else `keys()` (a dict). Each needed column is read once
//! into a plain list: by `to_pylist()` when the column has one (pyarrow), else by iterating it.
//! The column names are fixed, with no aliases: a missing column or a column the table should
//! not hold is refused by name. A null is Python's `None`, and only where the crate's field is
//! optional.

use openmobisim_demand::input::TripRow;
use openmobisim_graph::geometry::{LonLat, Position, Xy};
use openmobisim_graph::network::{LinkRow, NetworkBuilder};
use openmobisim_types::mode::{Mode, ModeSet};
use openmobisim_types::time::Second;
use pyo3::conversion::FromPyObjectOwned;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

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

/// One table being read: its name for the messages, the object, its column names, and the row
/// count of the first column read.
struct Table<'py> {
    what: &'static str,
    table: Bound<'py, PyAny>,
    names: Vec<String>,
    rows: Option<usize>,
}

impl<'py> Table<'py> {
    fn new(what: &'static str, table: &Bound<'py, PyAny>) -> PyResult<Self> {
        let names = if table.hasattr("column_names")? {
            table.getattr("column_names")?
        } else if table.hasattr("columns")? {
            table.getattr("columns")?
        } else {
            table.call_method0("keys")?
        };
        let names = names.try_iter()?.map(|n| n?.extract::<String>()).collect::<PyResult<_>>()?;
        Ok(Self { what, table: table.clone(), names, rows: None })
    }

    fn has(&self, name: &str) -> bool {
        self.names.iter().any(|n| n == name)
    }

    /// The table holds exactly `columns`, in any order: a missing one or another one is refused
    /// by name.
    fn expect(&self, columns: &[&str]) -> PyResult<()> {
        let list = columns.join(", ");
        if let Some(missing) = columns.iter().find(|c| !self.has(c)) {
            return Err(PyValueError::new_err(format!(
                "{}: missing column {missing}; the columns are {list}",
                self.what
            )));
        }
        if let Some(other) = self.names.iter().find(|n| !columns.contains(&n.as_str())) {
            return Err(PyValueError::new_err(format!(
                "{}: unknown column {other}; the columns are {list}",
                self.what
            )));
        }
        Ok(())
    }

    /// The column `name` as a list of `T`, each row read once; a row that is not a `T`, or a
    /// column of another length than the first read, is refused by name.
    fn column<T: FromPyObjectOwned<'py>>(&mut self, name: &str) -> PyResult<Vec<T>> {
        let column = self.table.get_item(name)?;
        let list =
            if column.hasattr("to_pylist")? { column.call_method0("to_pylist")? } else { column };
        let mut values = Vec::new();
        for (row, value) in list.try_iter()?.enumerate() {
            values.push(value?.extract::<T>().map_err(|e| {
                let e: PyErr = e.into();
                PyValueError::new_err(format!("{}: column {name}, row {row}: {e}", self.what))
            })?);
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
/// `transit`; the empty string is the empty set (which the crates refuse, naming the row).
fn modes(text: &str) -> Result<ModeSet, String> {
    if text.is_empty() {
        return Ok(ModeSet::EMPTY);
    }
    text.split(';').try_fold(ModeSet::EMPTY, |set, name| {
        let mode = Mode::ALL.into_iter().find(|m| m.to_string() == name);
        mode.map(|m| set.with(m)).ok_or_else(|| {
            format!("{name:?} is not a mode; the modes are car, bike, walk, transit joined by ;")
        })
    })
}

/// The nodes and links tables into a network builder.
pub(crate) fn network(
    nodes: &Bound<'_, PyAny>,
    links: &Bound<'_, PyAny>,
) -> PyResult<NetworkBuilder> {
    let mut builder = NetworkBuilder::new();
    let mut t = Table::new("nodes", nodes)?;
    let kind = t.kind("");
    t.expect(&["node_id", &kind.1, &kind.2])?;
    let ids: Vec<String> = t.column("node_id")?;
    for (id, at) in ids.into_iter().zip(t.positions(&kind)?) {
        builder.add_node(id, at);
    }

    let mut t = Table::new("links", links)?;
    t.expect(&LINK_COLUMNS)?;
    let ids: Vec<String> = t.column("link_id")?;
    let upstream: Vec<String> = t.column("upstream")?;
    let downstream: Vec<String> = t.column("downstream")?;
    let mode_names: Vec<String> = t.column("modes")?;
    let length: Vec<Option<f64>> = t.column("length_m")?;
    let lanes: Vec<Option<u8>> = t.column("lanes")?;
    let speed: Vec<Option<f64>> = t.column("car_speed_km_h")?;
    let capacity: Vec<Option<f64>> = t.column("capacity_veh_h")?;
    for (i, id) in ids.into_iter().enumerate() {
        let row = LinkRow {
            modes: modes(&mode_names[i])
                .map_err(|e| PyValueError::new_err(format!("links: link {id}: {e}")))?,
            lanes: lanes[i],
            car_speed_km_h: speed[i],
            length_m: length[i],
            capacity_veh_h: capacity[i],
            signal_at_downstream_end: false,
        };
        builder.add_link(id, upstream[i].clone(), downstream[i].clone(), row);
    }
    Ok(builder)
}

/// The trips table into the demand crate's rows.
pub(crate) fn trips(trips: &Bound<'_, PyAny>) -> PyResult<Vec<TripRow>> {
    let mut t = Table::new("trips", trips)?;
    let (origin, destination) = (t.kind("origin_"), t.kind("destination_"));
    t.expect(&[
        "trip_id",
        &origin.1,
        &origin.2,
        &destination.1,
        &destination.2,
        "departure_s",
        "class_id",
        "modes",
    ])?;
    let ids: Vec<String> = t.column("trip_id")?;
    let from = t.positions(&origin)?;
    let to = t.positions(&destination)?;
    let departure: Vec<u32> = t.column("departure_s")?;
    let class: Vec<String> = t.column("class_id")?;
    let mode_names: Vec<String> = t.column("modes")?;
    let mut rows = Vec::with_capacity(ids.len());
    for (i, id) in ids.into_iter().enumerate() {
        let modes = modes(&mode_names[i])
            .map_err(|e| PyValueError::new_err(format!("trips: trip {id}: {e}")))?;
        rows.push(TripRow {
            id,
            origin: from[i],
            destination: to[i],
            departure: Second(departure[i]),
            class: class[i].clone(),
            modes,
        });
    }
    Ok(rows)
}
