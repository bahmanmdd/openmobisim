//! The keywords of the five entry points: each row of `PARAMETERS.md` is read at exactly one
//! entry point, the one whose crate reads it; a keyword that is not a row, or that is given at
//! another entry point, is refused by name.

use std::collections::BTreeMap;

use pyo3::conversion::FromPyObjectOwned;
use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyDict;

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
const ROWS: [(&str, Entry); 15] = [
    ("capacity_per_lane", Entry::Network),
    ("bike_speed", Entry::Network),
    ("walk_speed", Entry::Network),
    ("osm_class_defaults", Entry::Network),
    ("transfer_walk_max", Entry::Transit),
    ("stop_walk_max", Entry::Demand),
    ("boarding_slack", Entry::Run),
    ("max_rides", Entry::Run),
    ("choice", Entry::Run),
    ("coefficients", Entry::Run),
    ("nest_scale", Entry::Run),
    ("iteration_cap", Entry::Run),
    ("iteration_floor", Entry::Run),
    ("link_bin", Entry::Run),
    ("seed", Entry::Run),
];

/// The keywords given at one entry point, each checked to be a row read there.
pub(crate) struct Given<'py>(BTreeMap<String, Bound<'py, PyAny>>);

impl<'py> Given<'py> {
    /// The keywords of `kwargs`, given to the function `here` of the entry point `entry`.
    ///
    /// A keyword that is not a row of `PARAMETERS.md`, or that another entry point reads, is a
    /// `TypeError` naming it, as Python refuses an unexpected keyword.
    pub(crate) fn read(
        entry: Entry,
        here: &str,
        kwargs: Option<&Bound<'py, PyDict>>,
    ) -> PyResult<Self> {
        let mut given = BTreeMap::new();
        for (key, value) in kwargs.into_iter().flat_map(|d| d.iter()) {
            let name: String = key.extract()?;
            match ROWS.iter().find(|(row, _)| *row == name) {
                None => {
                    return Err(PyTypeError::new_err(format!(
                        "{here}: {name} is not a parameter; the parameters are the rows of \
                         PARAMETERS.md"
                    )));
                }
                Some((_, at)) if *at != entry => {
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
    /// as a `T` is a `ValueError` naming the keyword.
    pub(crate) fn get<T: FromPyObjectOwned<'py>>(&self, name: &str, default: T) -> PyResult<T> {
        match self.0.get(name) {
            None => Ok(default),
            Some(value) => value
                .extract::<T>()
                .map_err(|e| PyValueError::new_err(format!("{name}: {}", Into::<PyErr>::into(e)))),
        }
    }
}
