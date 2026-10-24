//! A network handle's cache file (S294, I-as): the road network with its street geometry, its
//! bike and walk layers and what its import did, written once and read back bit for bit, so that
//! a network is imported only the first time it is opened. Each part's layout is
//! `openmobisim_core_graph::binary`'s; the file is valid for the build that wrote it.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Instant, UNIX_EPOCH};

use pyo3::exceptions::{PyOSError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyDict;

use openmobisim_core_graph::binary::{self, Header, Reader, Writer, invalid};
use openmobisim_core_graph::defaults::RoadClass;
use openmobisim_core_graph::geometry::LonLat;
use openmobisim_core_graph::layers::{StaticLayer, StaticNetwork};
use openmobisim_core_graph::link_geometry::LinkGeometry;
use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_types::GIT_SHA;
use openmobisim_io_osm::{DropReason, DroppedLink, ImportReport, LayerReport};

use super::{ImportInfo, LayerParts, PyNetwork, slot};

/// What tells this build from another: its commit, platform and profile, and the compiled
/// module's size and modification time, which a rebuild changes even without a commit (an
/// import by changed code may build another network). Computed once.
fn build_stamp(py: Python<'_>) -> String {
    static STAMP: OnceLock<String> = OnceLock::new();
    STAMP
        .get_or_init(|| {
            let file = py
                .import("openmobisim._core")
                .and_then(|m| m.getattr("__file__"))
                .and_then(|f| f.extract::<PathBuf>())
                .ok();
            let module = file.and_then(|f| std::fs::metadata(f).ok()).map_or_else(
                || "unknown".to_string(),
                |m| {
                    let modified = m
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                        .map_or(0, |d| d.as_nanos());
                    format!("{}:{modified}", m.len())
                },
            );
            let profile = if cfg!(debug_assertions) { "debug" } else { "release" };
            format!("{GIT_SHA} {} {profile} {module}", env!("OPENMOBISIM_TARGET"))
        })
        .clone()
}

fn format_error(path: &Path, e: &binary::FormatError) -> PyErr {
    PyValueError::new_err(format!("{}: {e}", path.display()))
}

/// Write `network` (the road network's handle: its street geometry, its bike and walk layers
/// and the report of its import go with it) to `path` as a cache file, which
/// `network_read_cache` reads back as the same network, bit for bit, much faster than an import:
/// the same fingerprint, the same results.
///
/// The file is valid for this build of openmobisim only: another version, or another build of
/// this one, refuses it (import the network again and write it anew). It is a cache, not a way
/// to keep or share a network: keep its source files. `source` says what the network was made
/// from (a checksum of its files and reading options, say), stored in the file for whoever
/// reads it to compare (`network_read_cache(path, source=...)`). The file is written beside its
/// final name and then renamed, so a reader never sees half of it.
///
/// # Errors
///
/// `ValueError` for a bike or walk layer's handle (write the road network's); `OSError` if the
/// file cannot be written.
#[pyfunction]
#[pyo3(signature = (network, path, source = ""))]
pub fn network_write_cache(
    py: Python<'_>,
    network: PyRef<'_, PyNetwork>,
    path: PathBuf,
    source: &str,
) -> PyResult<()> {
    if network.static_network.is_some() {
        return Err(PyValueError::new_err(format!(
            "this is the {} layer; write the road network's handle, which holds its layers",
            network.layer
        )));
    }
    let header = Header::current(&build_stamp(py), source, network.read_s.unwrap_or(f64::NAN));
    let mut w = Writer::new();
    header.write(&mut w);
    write_handle(&mut w, &network);
    let bytes = w.finish();
    // A name of its own for each write, so that two writers of one file never share it.
    static WRITES: AtomicU64 = AtomicU64::new(0);
    let partial = path.with_file_name(format!(
        "{}.partial-{}-{}",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("network"),
        std::process::id(),
        WRITES.fetch_add(1, Ordering::Relaxed)
    ));
    let written = py.detach(|| {
        std::fs::write(&partial, &bytes).and_then(|()| std::fs::rename(&partial, &path))
    });
    written.map_err(|e| {
        let _ = std::fs::remove_file(&partial);
        PyOSError::new_err(format!("{}: {e}", path.display()))
    })
}

/// Read a network `network_write_cache` wrote: the same network, bit for bit, with its street
/// geometry, its bike and walk layers and the report of its import. Its `read_s` is the time
/// this read took.
///
/// `source`, if given, must be what the file was written with (else `ValueError`): what the
/// network was made from, so that a file made from other data is never taken for it.
///
/// # Errors
///
/// `ValueError` for a file that is not a network cache, is damaged, was written by another
/// version or build of openmobisim, or from another `source`; `OSError` if it cannot be read.
#[pyfunction]
#[pyo3(signature = (path, source = None))]
pub fn network_read_cache(
    py: Python<'_>,
    path: PathBuf,
    source: Option<&str>,
) -> PyResult<PyNetwork> {
    let started = Instant::now();
    let here = Header::current(&build_stamp(py), "", f64::NAN);
    let bytes = py
        .detach(|| std::fs::read(&path))
        .map_err(|e| PyOSError::new_err(format!("{}: {e}", path.display())))?;
    let (header, network) = py
        .detach(|| {
            let mut r = Reader::new(&bytes)?;
            let header = Header::read(&mut r)?;
            if header.differs_from(&here).is_some() || source.is_some_and(|s| s != header.source) {
                // Refused before the content is read; the message is made below.
                return Ok((header, None));
            }
            let network = read_handle(&mut r)?;
            r.finish()?;
            Ok::<_, binary::FormatError>((header, Some(network)))
        })
        .map_err(|e| format_error(&path, &e))?;
    if let Some(what) = header.differs_from(&here) {
        return Err(PyValueError::new_err(format!(
            "{}: written by another build of openmobisim ({what}); import the network again",
            path.display()
        )));
    }
    let Some(mut network) = network else {
        return Err(PyValueError::new_err(format!(
            "{}: a network made from another source ({:?}, not {:?})",
            path.display(),
            header.source,
            source.unwrap_or_default()
        )));
    };
    network.read_s = Some(started.elapsed().as_secs_f64());
    Ok(network)
}

/// The header of a network cache file, read from its first bytes without reading the rest:
/// `format_version`, `code_version`, `defaults_version`, `version`, `build`, `source` and
/// `made_s` (the seconds its import took), and `current`, whether this build would read it.
/// Not checked (the whole file is, when read).
///
/// # Errors
///
/// `ValueError` for a file that is not a network cache of this format; `OSError` if it cannot be
/// read.
#[pyfunction]
pub fn network_cache_header<'py>(py: Python<'py>, path: PathBuf) -> PyResult<Bound<'py, PyDict>> {
    use std::io::Read;
    let mut start = Vec::new();
    std::fs::File::open(&path)
        .and_then(|f| f.take(1 << 16).read_to_end(&mut start))
        .map_err(|e| PyOSError::new_err(format!("{}: {e}", path.display())))?;
    let header = Header::peek(&start).map_err(|e| format_error(&path, &e))?;
    let here = Header::current(&build_stamp(py), "", f64::NAN);
    let d = PyDict::new(py);
    d.set_item("format_version", header.format_version)?;
    d.set_item("code_version", header.code_version)?;
    d.set_item("defaults_version", header.defaults_version)?;
    d.set_item("version", &header.crate_version)?;
    d.set_item("build", &header.build)?;
    d.set_item("source", &header.source)?;
    d.set_item("made_s", header.made_seconds)?;
    d.set_item("current", header.differs_from(&here).is_none())?;
    Ok(d)
}

// --- The handle's parts, in order ---------------------------------------------------------------

fn write_handle(w: &mut Writer, network: &PyNetwork) {
    w.str(&network.source);
    network.inner.write_binary(w);
    write_geometry(w, network.geometry.as_deref());
    for layer in [StaticLayer::Bike, StaticLayer::Walk] {
        let parts = network.layers[slot(layer)].get();
        w.bool(parts.is_some());
        if let Some(LayerParts { network: graph, geometry, report }) = parts {
            graph.write_binary(w);
            write_geometry(w, geometry.as_deref());
            w.bool(report.is_some());
            if let Some(report) = report {
                write_layer_report(w, report);
            }
        }
    }
    w.bool(network.import.is_some());
    if let Some(info) = &network.import {
        write_import(w, info);
    }
}

fn read_handle(r: &mut Reader<'_>) -> binary::Result<PyNetwork> {
    let source = r.str()?.to_string();
    let road = RoadNetwork::read_binary(r)?;
    let geometry = read_geometry(r, &road)?;
    let mut handle = PyNetwork::new(Arc::new(road), geometry.map(Arc::new), &source);
    for layer in [StaticLayer::Bike, StaticLayer::Walk] {
        if !r.bool()? {
            continue;
        }
        let graph = StaticNetwork::read_binary(r)?;
        if graph.layer() != layer {
            let (found, expected) = (graph.layer().as_str(), layer.as_str());
            return invalid(format!("the {found} layer where the {expected} layer was"));
        }
        let geometry = read_geometry(r, graph.network())?.map(Arc::new);
        let report = if r.bool()? { Some(read_layer_report(r)?) } else { None };
        let parts = LayerParts { network: Arc::new(graph), geometry, report };
        let _ = handle.layers[slot(layer)].set(parts);
    }
    if r.bool()? {
        handle.import = Some(Arc::new(read_import(r)?));
    }
    Ok(handle)
}

fn write_geometry(w: &mut Writer, geometry: Option<&LinkGeometry>) {
    w.bool(geometry.is_some());
    if let Some(g) = geometry {
        g.write_binary(w);
    }
}

fn read_geometry(
    r: &mut Reader<'_>,
    network: &RoadNetwork,
) -> binary::Result<Option<LinkGeometry>> {
    if r.bool()? { Ok(Some(LinkGeometry::read_binary(r, network)?)) } else { Ok(None) }
}

fn write_layer_report(w: &mut Writer, report: &LayerReport) {
    let LayerReport {
        ways_kept,
        links_before_contraction,
        links_after_contraction,
        components_before,
        links_disconnected,
        length_m,
        dedicated_length_m,
        degenerate_links,
        ferry_ways,
    } = *report;
    for v in [
        ways_kept,
        links_before_contraction,
        links_after_contraction,
        components_before,
        links_disconnected,
        length_m,
        dedicated_length_m,
        degenerate_links,
        ferry_ways,
    ] {
        w.u64(v);
    }
}

fn read_layer_report(r: &mut Reader<'_>) -> binary::Result<LayerReport> {
    Ok(LayerReport {
        ways_kept: r.u64()?,
        links_before_contraction: r.u64()?,
        links_after_contraction: r.u64()?,
        components_before: r.u64()?,
        links_disconnected: r.u64()?,
        length_m: r.u64()?,
        dedicated_length_m: r.u64()?,
        degenerate_links: r.u64()?,
        ferry_ways: r.u64()?,
    })
}

fn write_import(w: &mut Writer, info: &ImportInfo) {
    let ImportInfo { report, diagnostics, dropped, connectivity, contract_drivable, region } = info;
    let ImportReport {
        ways_seen,
        ways_kept,
        nodes_seen,
        nodes_kept,
        junction_nodes,
        links_before_contraction,
        links_after_contraction,
        nodes_contracted,
        closed_loops_dropped,
        components_before,
        links_disconnected,
        nodes_disconnected,
        length_disconnected_m,
        busway_links_disconnected,
        drivable_links,
        drivable_length_m,
        drivable_maxspeed_links,
        drivable_maxspeed_length_m,
        drivable_lanes_links,
        drivable_lanes_length_m,
    } = *report;
    for v in [
        ways_seen,
        ways_kept,
        nodes_seen,
        nodes_kept,
        junction_nodes,
        links_before_contraction,
        links_after_contraction,
        nodes_contracted,
        closed_loops_dropped,
        components_before,
        links_disconnected,
        nodes_disconnected,
        length_disconnected_m,
        busway_links_disconnected,
        drivable_links,
        drivable_length_m,
        drivable_maxspeed_links,
        drivable_maxspeed_length_m,
        drivable_lanes_links,
        drivable_lanes_length_m,
    ] {
        w.u64(v);
    }
    w.len(diagnostics.len());
    for (code, severity, count) in diagnostics {
        w.str(code);
        w.str(severity);
        w.u64(*count);
    }
    w.len(dropped.len());
    for DroppedLink { way_id, reason, class, geometry } in dropped {
        w.i64(*way_id);
        w.u8(match reason {
            DropReason::ClosedLoop => 0,
            DropReason::NotStronglyConnected => 1,
        });
        w.u8(*class as u8);
        w.f64s(geometry.iter().map(|p| p.lon));
        w.f64s(geometry.iter().map(|p| p.lat));
    }
    w.u8(match *connectivity {
        "keep" => 0,
        _ => 1,
    });
    w.bool(*contract_drivable);
    w.bool(region.is_some());
    if let Some(region) = region {
        w.str(region);
    }
}

fn read_import(r: &mut Reader<'_>) -> binary::Result<ImportInfo> {
    let report = ImportReport {
        ways_seen: r.u64()?,
        ways_kept: r.u64()?,
        nodes_seen: r.u64()?,
        nodes_kept: r.u64()?,
        junction_nodes: r.u64()?,
        links_before_contraction: r.u64()?,
        links_after_contraction: r.u64()?,
        nodes_contracted: r.u64()?,
        closed_loops_dropped: r.u64()?,
        components_before: r.u64()?,
        links_disconnected: r.u64()?,
        nodes_disconnected: r.u64()?,
        length_disconnected_m: r.u64()?,
        busway_links_disconnected: r.u64()?,
        drivable_links: r.u64()?,
        drivable_length_m: r.u64()?,
        drivable_maxspeed_links: r.u64()?,
        drivable_maxspeed_length_m: r.u64()?,
        drivable_lanes_links: r.u64()?,
        drivable_lanes_length_m: r.u64()?,
    };
    let n = r.len(24)?;
    let mut diagnostics = Vec::with_capacity(n);
    for _ in 0..n {
        diagnostics.push((r.str()?.to_string(), r.str()?.to_string(), r.u64()?));
    }
    let n = r.len(26)?;
    let mut dropped = Vec::with_capacity(n);
    for _ in 0..n {
        let way_id = r.i64()?;
        let reason = match r.u8()? {
            0 => DropReason::ClosedLoop,
            1 => DropReason::NotStronglyConnected,
            other => return invalid(format!("drop reason {other}")),
        };
        let class = r.u8()?;
        let Some(&class) = RoadClass::ALL.get(usize::from(class)) else {
            return invalid(format!("road class {class}"));
        };
        let lon = r.f64s()?;
        let lat = r.f64s()?;
        if lon.len() != lat.len() {
            return invalid("a dropped road's longitudes and latitudes differ in number");
        }
        let geometry = lon.into_iter().zip(lat).map(|(lon, lat)| LonLat::new(lon, lat)).collect();
        dropped.push(DroppedLink { way_id, reason, class, geometry });
    }
    let connectivity = match r.u8()? {
        0 => "keep",
        1 => "strong",
        other => return invalid(format!("connectivity {other}")),
    };
    let contract_drivable = r.bool()?;
    let region = if r.bool()? { Some(r.str()?.to_string()) } else { None };
    Ok(ImportInfo { report, diagnostics, dropped, connectivity, contract_drivable, region })
}
