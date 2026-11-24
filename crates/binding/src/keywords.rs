//! The keywords of the five entry points: each row of `PARAMETERS.md` is read at exactly one
//! entry point, the one whose crate reads it; a keyword that is not a row, or that is given at
//! another entry point, is refused by name.

use std::collections::BTreeMap;

use openmobisim_io_osm::Region;
use pyo3::conversion::FromPyObjectOwned;
use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyDict, PyList, PyTuple};

/// Where a keyword is read.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Entry {
    /// `Network.from_osm` and `Network.from_tables`: the graph's three build values, and the OSM
    /// reader's class table (read by `from_osm` alone).
    Network,
    /// `Transit.from_gtfs`: the GTFS reader.
    Transit,
    /// `Demand.from_table`: the demand crate.
    Demand,
    /// `run`: the loop.
    Run,
}

impl Entry {
    /// The entry point's name as Python spells it.
    fn name(self) -> &'static str {
        match self {
            Self::Network => "Network.from_osm and Network.from_tables",
            Self::Transit => "Transit.from_gtfs",
            Self::Demand => "Demand.from_table",
            Self::Run => "run",
        }
    }
}

/// Every row of `PARAMETERS.md` and the entry point that reads it. The defaults are not here:
/// they are the loop's (`Parameters::defaults`) and, for the five values no crate defaults,
/// the constants of `lib.rs`.
const ROWS: [(&str, Entry); 19] = [
    ("capacity_per_lane", Entry::Network),
    ("bike_speed", Entry::Network),
    ("walk_speed", Entry::Network),
    ("osm_class_defaults", Entry::Network),
    ("transfer_walk_max", Entry::Transit),
    ("stop_walk_max", Entry::Demand),
    ("boarding_slack", Entry::Run),
    ("max_rides", Entry::Run),
    ("nearby_stops", Entry::Run),
    ("max_car_routes", Entry::Run),
    ("max_transit_journeys", Entry::Run),
    ("choice", Entry::Run),
    ("coefficients", Entry::Run),
    ("nest_scale", Entry::Run),
    ("step_exponent", Entry::Run),
    ("iteration_cap", Entry::Run),
    ("iteration_floor", Entry::Run),
    ("link_bin", Entry::Run),
    ("seed", Entry::Run),
];

/// The entry point that reads the row `name` of `PARAMETERS.md`; `None` for a name that is not a
/// row.
pub(crate) fn entry(name: &str) -> Option<Entry> {
    ROWS.iter().find(|(row, _)| *row == name).map(|&(_, at)| at)
}

/// The keywords given at one entry point, each checked to be a row read there.
pub(crate) struct Given<'py>(BTreeMap<String, Bound<'py, PyAny>>);

impl<'py> Given<'py> {
    /// The keywords of `kwargs`, given to the function `here` of the entry point `here_entry`.
    ///
    /// A keyword that is not a row of `PARAMETERS.md`, or that another entry point reads, is a
    /// `TypeError` naming it, as Python refuses an unexpected keyword.
    pub(crate) fn read(
        here_entry: Entry,
        here: &str,
        kwargs: Option<&Bound<'py, PyDict>>,
    ) -> PyResult<Self> {
        let mut given = BTreeMap::new();
        for (key, value) in kwargs.into_iter().flat_map(|d| d.iter()) {
            let name: String = key.extract()?;
            match entry(&name) {
                None => {
                    return Err(PyTypeError::new_err(format!(
                        "{here}: {name} is not a parameter; the parameters are the rows of \
                         PARAMETERS.md"
                    )));
                }
                Some(at) if at != here_entry => {
                    return Err(PyTypeError::new_err(format!(
                        "{here}: {name} is read by {}, not here",
                        at.name()
                    )));
                }
                Some(_) => given.insert(name, value),
            };
        }
        Ok(Self(given))
    }

    /// The value given for `name`, if any.
    pub(crate) fn raw(&self, name: &str) -> Option<&Bound<'py, PyAny>> {
        self.0.get(name)
    }

    /// The value given for `name` read as a `T`, else `default`. A value that cannot be read
    /// as a `T`, or is a boolean (in it or inside it), is a `ValueError` naming the keyword.
    pub(crate) fn get<T: FromPyObjectOwned<'py>>(&self, name: &str, default: T) -> PyResult<T> {
        match self.0.get(name) {
            None => Ok(default),
            Some(value) => match value.extract::<T>() {
                Ok(read) => not_boolean(name, value).map(|()| read),
                Err(e) => Err(PyValueError::new_err(format!("{name}: {}", Into::<PyErr>::into(e)))),
            },
        }
    }
}

/// Refuse a boolean given for `name`, or inside it (a dict's value, a list's or tuple's item,
/// named by its key or index): no parameter is a boolean, and Python reads `True` as 1.
pub(crate) fn not_boolean(name: &str, v: &Bound<'_, PyAny>) -> PyResult<()> {
    if let Ok(b) = v.cast::<PyBool>() {
        let instead = u8::from(b.is_true());
        let why = format!("{v} is a boolean; this parameter is a number, give {instead} instead");
        return Err(PyValueError::new_err(format!("{name}: {why}")));
    }
    if let Ok(d) = v.cast::<PyDict>() {
        d.iter().try_for_each(|(k, x)| not_boolean(&format!("{name}[{k:?}]"), &x))
    } else if v.is_instance_of::<PyList>() || v.is_instance_of::<PyTuple>() {
        v.try_iter()?.enumerate().try_for_each(|(i, x)| not_boolean(&format!("{name}[{i}]"), &x?))
    } else {
        Ok(())
    }
}

/// The region `from_osm` cuts the extract to: `None`, a `(west, south, east, north)` box, or a
/// sequence of `(lon, lat)` points of a polygon, in WGS84 degrees, lists and tuples alike (a
/// record's region comes back from TOML as lists), no boolean among them; one that is no area
/// refused with the reader's reason, or, for a polygon of three points or more, because they
/// repeat or lie on one line.
pub(crate) fn region(v: Option<&Bound<'_, PyAny>>) -> PyResult<Option<Region>> {
    let Some(v) = v.filter(|v| !v.is_none()) else { return Ok(None) };
    not_boolean("region", v)?;
    let made = if let Ok([west, south, east, north]) = v.extract::<[f64; 4]>() {
        Region::bbox(west, south, east, north)
    } else {
        let points: Vec<[f64; 2]> = v.extract().map_err(|_| {
            PyValueError::new_err(format!(
                "region: {v} is neither a (west, south, east, north) box nor a sequence of \
                 (lon, lat) points, in degrees"
            ))
        })?;
        let points: Vec<(f64, f64)> = points.into_iter().map(|[lon, lat]| (lon, lat)).collect();
        if points.len() >= 3 && no_area(&points) {
            return Err(PyValueError::new_err(
                "region: a polygon needs at least three vertices that are distinct and not on \
                 one line: these enclose no area",
            ));
        }
        Region::polygon(points)
    };
    made.map(Some).map_err(|e| PyValueError::new_err(format!("region: {e}")))
}

/// Whether a polygon's points enclose no area: all lie on one line (fewer than three distinct
/// points included), each within 1e-9 of the farthest point's distance from the first, of the
/// line through the two (a figure eight is an area, by the even-odd rule the reader cuts by). A
/// point not finite is left to the reader's refusal.
fn no_area(points: &[(f64, f64)]) -> bool {
    let (x0, y0) = points[0];
    let reach2 = |&(x, y): &(f64, f64)| (x - x0).powi(2) + (y - y0).powi(2);
    let far = points.iter().copied().max_by(|a, b| reach2(a).total_cmp(&reach2(b)));
    let Some((dx, dy)) = far.map(|(x, y)| (x - x0, y - y0)) else { return true };
    let far2 = dx * dx + dy * dy;
    points.iter().all(|&(x, y)| ((x - x0) * dy - (y - y0) * dx).abs() <= 1e-9 * far2)
}
