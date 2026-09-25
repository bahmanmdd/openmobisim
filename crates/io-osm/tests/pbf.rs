//! The `.osm.pbf` adapter.
//!
//! # What is NOT covered here, and why it matters
//!
//! **The happy path is untested.** Verifying it needs a real `.osm.pbf` file,
//! and there is not one in this repository: a genuine extract is
//! ODbL-licensed data with attribution obligations and a megabyte of binary in
//! the history, and a hand-written one would mean writing a PBF *encoder* for
//! the test suite — more code, and more likely to be wrong, than the forty
//! lines it would be checking.
//!
//! So the risk is stated instead of hidden. The specific failure to watch for
//! is the `DenseNode` arm of the element match: essentially every real extract
//! stores geometry as dense nodes, and dropping that arm would produce an
//! **empty network from a perfectly good file** — silently, because the failure
//! surfaces later and somewhere else.
//!
//! Two things close the gap:
//!
//! * [`imports_a_real_extract_when_one_is_pointed_at`] runs against whatever
//!   `OPENMOBISIM_TEST_PBF` points at, and is skipped when it is unset. Point it at
//!   any city extract to exercise the whole pipeline for real:
//!
//!   ```text
//!   OPENMOBISIM_TEST_PBF=lyon.osm.pbf cargo test -p openmobisim-io-osm --test pbf -- --nocapture
//!   ```
//!
//! * The first real import is a deliberate milestone, not a formality.
//!
//! Everything below this line is what *can* be checked without a fixture.

#![cfg(feature = "pbf")]

use std::io::Write;

use openmobisim_core_types::diagnostics::Diagnostics;
use openmobisim_core_types::ids::{EntityId, LinkId};
use openmobisim_io_osm::import::{
    Connectivity, ImportOptions, LayerOptions, import, import_detailed,
};
use openmobisim_io_osm::pbf::PbfSource;
use openmobisim_io_osm::source::{OsmError, OsmSource};

#[test]
fn a_missing_file_is_an_error_not_an_empty_network() {
    // An empty network from a typo in a path is the worst possible outcome:
    // everything downstream "works" and produces nothing.
    let source = PbfSource::new("/nonexistent/definitely-not-here.osm.pbf");
    let err = source.for_each_way(&mut |_| {}).expect_err("a missing file must be an error");
    assert!(matches!(err, OsmError::Io(_) | OsmError::Format(_)), "{err:?}");
    assert!(err.to_string().contains("OSM input"), "{err}");
}

#[test]
fn a_file_that_is_not_a_pbf_is_an_error() {
    let dir = std::env::temp_dir().join("openmobisim-pbf-test");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("not-really.osm.pbf");
    let mut f = std::fs::File::create(&path).expect("create");
    f.write_all(b"this is not a protobuf blob, it is a sentence").expect("write");
    drop(f);

    let source = PbfSource::new(&path);
    let result = source.for_each_way(&mut |_| {});
    assert!(result.is_err(), "a text file must not read as an OSM extract");

    let _ = std::fs::remove_file(&path);
}

#[test]
fn the_source_remembers_its_path_without_opening_it() {
    // Construction must not touch the filesystem: a scenario is validated
    // before anything is read, and the path is part of the fingerprint.
    let source = PbfSource::new("somewhere/lyon.osm.pbf");
    assert_eq!(source.path().to_string_lossy(), "somewhere/lyon.osm.pbf");
}

/// Import whatever `OPENMOBISIM_TEST_PBF` points at, and report what came out.
///
/// Skipped when the variable is unset. This is the test that would catch a
/// missing `DenseNode` arm, and it is the first thing to run against a real
/// extract.
#[test]
fn imports_a_real_extract_when_one_is_pointed_at() {
    let Ok(path) = std::env::var("OPENMOBISIM_TEST_PBF") else {
        eprintln!("skipped: set OPENMOBISIM_TEST_PBF to a .osm.pbf file to run this");
        return;
    };

    let source = PbfSource::new(&path);
    let mut diagnostics = Diagnostics::new();
    let (network, report, geometry) =
        import(&source, ImportOptions::default(), &mut diagnostics).expect("the extract imports");

    println!("{path}");
    println!("  ways   {:>9} seen, {:>9} kept", report.ways_seen, report.ways_kept);
    println!(
        "  nodes  {:>9} seen, {:>9} kept, {:>9} junctions, {:>9} contracted",
        report.nodes_seen, report.nodes_kept, report.junction_nodes, report.nodes_contracted
    );
    println!(
        "  links  {:>9} before, {:>9} after contraction ({:.0}% removed)",
        report.links_before_contraction,
        report.links_after_contraction,
        report.contraction_ratio() * 100.0
    );
    println!("  projection {}", network.projection().zone());
    for row in diagnostics.rows() {
        println!("  diagnostic {:<40} {:>9}", row.key.code.as_str(), row.count);
    }

    // The S125 geometry artifact: the measurement its estimate needs (S130).
    // "Points per link" here counts each direction of a two-way street
    // separately, matching the S125 estimate's own convention (it costed a
    // resident field on `RoadNetwork`, which is per directed link); the
    // artifact itself stores each street's points once, which `street_count`
    // and `bytes` below report directly.
    let mut links_with_geometry = 0u64;
    let mut points_over_links = 0u64;
    for link in LinkId::iter_space(network.link_count()) {
        if let Some(points) = geometry.points(link) {
            links_with_geometry += 1;
            points_over_links += points.count() as u64;
        }
    }
    #[allow(clippy::cast_precision_loss, reason = "reporting only, counts are far below 2^53")]
    let mean_points_per_link = points_over_links as f64 / links_with_geometry.max(1) as f64;
    println!(
        "  geometry   {:>9} streets, {:>9}/{:>9} links with geometry, \
         {mean_points_per_link:.2} points/link mean ({:.2} interior)",
        geometry.street_count(),
        links_with_geometry,
        network.link_count(),
        mean_points_per_link - 2.0,
    );
    #[allow(clippy::cast_precision_loss, reason = "reporting only, counts are far below 2^53")]
    let bytes_per_link = geometry.bytes() as f64 / f64::from(network.link_count().max(1));
    println!(
        "  geometry   {:>9} bytes total ({bytes_per_link:.1} B per directed link)",
        geometry.bytes(),
    );

    // The assertions that matter are structural, not numeric — a real extract's
    // counts depend on the city.
    assert!(report.ways_kept > 0, "no ways were kept: the tag reading is broken");
    assert!(
        report.nodes_kept > 0,
        "no nodes were kept: the DenseNode arm is almost certainly missing"
    );
    assert!(network.link_count() > 0, "the network is empty");
    assert!(
        report.links_after_contraction <= report.links_before_contraction,
        "contraction added links"
    );
    // Real OSM splits streets constantly, so contraction should always find work.
    assert!(
        report.contraction_ratio() > 0.05,
        "contraction removed only {:.1}% of links, which is implausibly little for real data",
        report.contraction_ratio() * 100.0
    );
    assert!(
        links_with_geometry == u64::from(network.link_count()),
        "every link should have stored geometry: {links_with_geometry} of {}",
        network.link_count()
    );
    assert!(geometry.street_count() > 0, "no streets were stored");
}

/// On a real extract, the bike and walk layers (S195) leave the road network
/// exactly as it is without them, and each layer is a usable graph. Prints the
/// layers' sizes and what building them cost.
///
/// Skipped when `OPENMOBISIM_TEST_PBF` is unset.
#[test]
fn a_real_extract_s_layers_leave_its_road_network_unchanged() {
    let Ok(path) = std::env::var("OPENMOBISIM_TEST_PBF") else {
        eprintln!("skipped: set OPENMOBISIM_TEST_PBF to a .osm.pbf file to run this");
        return;
    };
    let source = PbfSource::new(&path);
    // The options the Python reader uses (S191), with and without layers.
    let base = ImportOptions {
        connectivity: Connectivity::Strong,
        contract_drivable: true,
        ..ImportOptions::default()
    };
    let started = std::time::Instant::now();
    let plain = import_detailed(&source, base, &mut Diagnostics::new()).expect("imports");
    let plain_s = started.elapsed().as_secs_f64();
    let started = std::time::Instant::now();
    let layered = import_detailed(
        &source,
        ImportOptions { layers: LayerOptions::BOTH, ..base },
        &mut Diagnostics::new(),
    )
    .expect("imports");
    let layered_s = started.elapsed().as_secs_f64();

    let (a, b) = (&plain.network, &layered.network);
    assert_eq!(a.link_count(), b.link_count());
    assert_eq!(a.node_count(), b.node_count());
    for link in LinkId::iter_space(a.link_count()) {
        assert_eq!(a.link_from(link), b.link_from(link));
        assert_eq!(a.link_to(link), b.link_to(link));
        assert_eq!(a.link_length(link), b.link_length(link));
    }
    assert_eq!(plain.report, layered.report);

    println!("{path}: import {plain_s:.2} s without layers, {layered_s:.2} s with");
    println!("  road  {:>9} links {:>12} bytes", a.link_count(), a.bytes());
    for layer in [layered.bike.as_ref().unwrap(), layered.walk.as_ref().unwrap()] {
        let g = layer.network.network();
        let r = layer.report;
        println!(
            "  {:<5} {:>9} links {:>12} bytes; {} ways, {} of {} m dedicated, \
             {} links trimmed of {} components, {} degenerate",
            layer.network.layer().as_str(),
            g.link_count(),
            layer.network.bytes(),
            r.ways_kept,
            r.dedicated_length_m,
            r.length_m,
            r.links_disconnected,
            r.components_before,
            r.degenerate_links,
        );
        assert!(g.link_count() > 0, "an empty {} layer", layer.network.layer().as_str());
        assert!(layer.geometry.matches(g));
        assert_eq!(g.projection(), a.projection());
    }
}
