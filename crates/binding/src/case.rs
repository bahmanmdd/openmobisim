//! `make_case` (experimental): one call from an OpenStreetMap extract and a population to a
//! folder `run_file` runs. The network is `Network.from_osm`'s, the trips are drawn from its
//! streets (`openmobisim_io_osm::trips`: where, who, through traffic, when), the settings file
//! names the extract and the feed where they lie. Nothing is tuned: the case is a start for
//! trying ideas, and how congested it is follows from `population * trips_per_person`.

use std::path::PathBuf;

use openmobisim_io_osm::trips::{CLASSES, THROUGH, draw};
use openmobisim_io_parquet::write_case;
use openmobisim_types::time::Second;
use pyo3::exceptions::{PyOSError, PyValueError};
use pyo3::prelude::*;

use crate::{Network, canonical};

/// The window of a made case: 07:30 to 09:30, the trips setting out in its first hour.
const WINDOW: (Second, Second) = (Second(27_000), Second(34_200));

/// Make the case folder `folder` from the extract `osm` (cut to `region`, cars on `car_roads`,
/// as `Network.from_osm` reads them) for `population` people, `trips_per_person` of them
/// travelling in the hour, with the GTFS feed `gtfs` if given: `trips.parquet` and `run.toml`
/// (the extract and the feed are named, not copied). The same arguments give the same case.
/// Returns the folder.
#[pyfunction]
#[pyo3(signature = (folder, osm, population, region=None, gtfs=None, car_roads="all", trips_per_person=0.1, seed=0))]
#[allow(clippy::too_many_arguments, reason = "the case's few numbers, each a keyword")]
#[allow(clippy::needless_pass_by_value, reason = "pyo3 hands the arguments over by value")]
pub(crate) fn make_case(
    py: Python<'_>,
    folder: PathBuf,
    osm: PathBuf,
    population: f64,
    region: Option<&Bound<'_, PyAny>>,
    gtfs: Option<PathBuf>,
    car_roads: &str,
    trips_per_person: f64,
    seed: u64,
) -> PyResult<PathBuf> {
    let trips = (population * trips_per_person).round();
    if !(1.0..=f64::from(u32::MAX)).contains(&trips) {
        return Err(PyValueError::new_err(format!(
            "make_case: population x trips_per_person is {trips} trips; at least one is needed"
        )));
    }
    let gtfs = gtfs.map(|g| canonical(&g)).transpose()?;
    let network = Network::from_osm(py, osm, region, car_roads, None)?;
    let (Some(report), Some(osm)) = (&network.report, &network.osm_file) else {
        unreachable!("from_osm keeps its report and its file")
    };
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "checked above")]
    let rows = py.detach(|| draw(&network.network, trips as u32, WINDOW.0, gtfs.is_some(), seed));
    if rows.is_empty() {
        return Err(PyValueError::new_err(
            "make_case: the network has no residential or other street to walk on to draw trips from",
        ));
    }
    let mut classes: Vec<&str> = CLASSES.iter().map(|c| c.0).collect();
    if rows.iter().any(|t| t.class == THROUGH) {
        classes.push(THROUGH);
    }
    write_case(&folder, WINDOW, (osm, report), gtfs.as_deref(), classes, &rows)
        .map_err(|e| PyOSError::new_err(e.to_string()))?;
    Ok(folder)
}
