//! A network handle's cache file (S294, I-as): the road network with its street geometry, its
//! bike and walk layers and what its import did, written once and read back bit for bit, so that
//! a network is imported only the first time it is opened. Each part's layout is
//! `openmobisim_core_graph::binary`'s; the file is valid for the build that wrote it
//! (`crate::cache_file`).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyDict;

use openmobisim_core_graph::binary::{self, Content, Header, Reader, Writer, invalid};
use openmobisim_core_graph::defaults::RoadClass;
use openmobisim_core_graph::geometry::LonLat;
use openmobisim_core_graph::layers::{StaticLayer, StaticNetwork};
use openmobisim_core_graph::link_geometry::LinkGeometry;
use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_io_osm::{DropReason, DroppedLink, ImportReport, LayerReport};

use super::{ImportInfo, LayerParts, PyNetwork, slot};
use crate::cache_file::{build_stamp, read_header, read_whole, write_whole};

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
    write_whole(py, &path, &w.finish())
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
    let mut network = read_whole(py, &path, Content::Network, source, read_handle)?;
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
    read_header(py, &path, Content::Network)
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
    let ImportInfo {
        report,
        diagnostics,
        dropped,
        connectivity,
        contract_drivable,
        bus_lanes,
        region,
    } = info;
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
        bus_lane_links,
        bus_lane_length_m,
        bus_lane_car_lane_length_removed_m,
        drivable_lane_length_m,
        contraflow_bus_links,
        contraflow_bus_length_m,
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
        bus_lane_links,
        bus_lane_length_m,
        bus_lane_car_lane_length_removed_m,
        drivable_lane_length_m,
        contraflow_bus_links,
        contraflow_bus_length_m,
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
    w.bool(*bus_lanes);
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
        bus_lane_links: r.u64()?,
        bus_lane_length_m: r.u64()?,
        bus_lane_car_lane_length_removed_m: r.u64()?,
        drivable_lane_length_m: r.u64()?,
        contraflow_bus_links: r.u64()?,
        contraflow_bus_length_m: r.u64()?,
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
    let bus_lanes = r.bool()?;
    let region = if r.bool()? { Some(r.str()?.to_string()) } else { None };
    Ok(ImportInfo {
        report,
        diagnostics,
        dropped,
        connectivity,
        contract_drivable,
        bus_lanes,
        region,
    })
}
