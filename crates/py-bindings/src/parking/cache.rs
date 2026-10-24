//! A parkings handle's cache file (S294, I-as): the parkings read from an extract, with the
//! report of that read, written once and read back bit for bit, so that a case's extract is read
//! for its parkings only the first time. The layout is `openmobisim_core_graph::binary`'s, under
//! the parkings' magic bytes; the file is valid for the build that wrote it
//! (`crate::cache_file`).

use std::path::PathBuf;
use std::time::Instant;

use pyo3::prelude::*;
use pyo3::types::PyDict;

use openmobisim_core_graph::binary::{self, Content, Header, Reader, Writer};
use openmobisim_core_graph::hubs::ParkingRow;
use openmobisim_io_osm::ParkingReadReport;

use super::PyParkings;
use crate::cache_file::{build_stamp, read_header, read_whole, write_whole};

/// Write `parkings` to `path` as a cache file, which `parking_read_cache` reads back as the
/// same parkings, bit for bit, much faster than reading them from an extract: the same
/// fingerprint, the same results.
///
/// The file is valid for this build of openmobisim only: another version, or another build of
/// this one, refuses it (read the parkings again and write them anew). It is a cache, not a
/// way to keep or share parkings: keep their source (an extract, a table). `source` says what
/// the parkings were made from (a checksum of the extract and the reading options, say), stored
/// in the file for whoever reads it to compare (`parking_read_cache(path, source=...)`). The
/// file is written beside its final name and then renamed, so a reader never sees half of it.
///
/// # Errors
///
/// `OSError` if the file cannot be written.
#[pyfunction]
#[pyo3(signature = (parkings, path, source = ""))]
pub fn parking_write_cache(
    py: Python<'_>,
    parkings: PyRef<'_, PyParkings>,
    path: PathBuf,
    source: &str,
) -> PyResult<()> {
    let header = Header::current(&build_stamp(py), source, parkings.read_s.unwrap_or(f64::NAN));
    let mut w = Writer::of(Content::Parkings);
    header.write(&mut w);
    write_handle(&mut w, &parkings);
    write_whole(py, &path, &w.finish())
}

/// Read parkings `parking_write_cache` wrote: the same parkings, bit for bit, with the report
/// of the read that found them. Their `read_s` is the time this read took.
///
/// `source`, if given, must be what the file was written with (else `ValueError`): what the
/// parkings were made from, so that parkings made from other data are never taken for them.
///
/// # Errors
///
/// `ValueError` for a file that is not a parkings cache, is damaged, was written by another
/// version or build of openmobisim, or from another `source`; `OSError` if it cannot be read.
#[pyfunction]
#[pyo3(signature = (path, source = None))]
pub fn parking_read_cache(
    py: Python<'_>,
    path: PathBuf,
    source: Option<&str>,
) -> PyResult<PyParkings> {
    let started = Instant::now();
    let mut parkings = read_whole(py, &path, Content::Parkings, source, read_handle)?;
    parkings.read_s = Some(started.elapsed().as_secs_f64());
    Ok(parkings)
}

/// The header of a parkings cache file, read from its first bytes without reading the rest:
/// `format_version`, `code_version`, `defaults_version`, `version`, `build`, `source` and
/// `made_s` (the seconds reading the parkings took when they were written), and `current`,
/// whether this build would read it. Not checked (the whole file is, when read).
///
/// # Errors
///
/// `ValueError` for a file that is not a parkings cache of this format; `OSError` if it cannot
/// be read.
#[pyfunction]
pub fn parking_cache_header<'py>(py: Python<'py>, path: PathBuf) -> PyResult<Bound<'py, PyDict>> {
    read_header(py, &path, Content::Parkings)
}

// --- The handle's parts, in order ---------------------------------------------------------------

/// The fewest bytes a row takes (`ParkingRow::write_binary`): an id's length, two flags, a
/// place, a vehicle, a capacity, an occupancy and a flag.
const ROW_BYTES: usize = 8 + 1 + 1 + 16 + 1 + 4 + 4 + 1;

fn write_handle(w: &mut Writer, parkings: &PyParkings) {
    // Destructured without `..`: a field added to the handle does not compile until it is
    // written here (and read below). `read_s` is how long a read took, not what was read: the
    // header keeps it (`made_seconds`), and the handle read back has its own.
    let PyParkings { rows, report, read_s: _ } = parkings;
    w.len(rows.len());
    for row in rows {
        row.write_binary(w);
    }
    w.bool(report.is_some());
    if let Some(report) = report {
        write_report(w, report);
    }
}

fn read_handle(r: &mut Reader<'_>) -> binary::Result<PyParkings> {
    let n = r.len(ROW_BYTES)?;
    let rows = (0..n).map(|_| ParkingRow::read_binary(r)).collect::<binary::Result<Vec<_>>>()?;
    let report = if r.bool()? { Some(read_report(r)?) } else { None };
    Ok(PyParkings { rows, report, read_s: None })
}

fn write_report(w: &mut Writer, report: &ParkingReadReport) {
    let ParkingReadReport {
        car_found,
        bike_found,
        duplicates_dropped,
        private_dropped,
        capacity_tagged,
        capacity_from_area,
        capacity_default,
        sites_car,
        sites_bike,
    } = *report;
    for v in [
        car_found,
        bike_found,
        duplicates_dropped,
        private_dropped,
        capacity_tagged,
        capacity_from_area,
        capacity_default,
        sites_car,
        sites_bike,
    ] {
        w.u32(v);
    }
}

fn read_report(r: &mut Reader<'_>) -> binary::Result<ParkingReadReport> {
    Ok(ParkingReadReport {
        car_found: r.u32()?,
        bike_found: r.u32()?,
        duplicates_dropped: r.u32()?,
        private_dropped: r.u32()?,
        capacity_tagged: r.u32()?,
        capacity_from_area: r.u32()?,
        capacity_default: r.u32()?,
        sites_car: r.u32()?,
        sites_bike: r.u32()?,
    })
}
