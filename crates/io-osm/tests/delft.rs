//! The Delft extract (`fixtures/delft/network.osm.pbf`, 17 976 211 bytes, © OpenStreetMap
//! contributors, ODbL) read in full, and 20 of its ways against `fixtures/delft/ways.csv`,
//! written by `delft_ways.py` from libosmium (pyosmium 4.3.2: tags and node positions) and
//! pyproj 3.8.0 (each way's length, Karney's geodesic). Per way and direction: the modes, the
//! car's lanes and free-flow speed, and the length summed over the way's links. The cases
//! include both fixed bugs (`lanes=3` with one side given; `maxspeed:forward`/`:backward`)
//! and, for bikes, a one-way street open to bikes both ways, `bicycle=no`, `use_sidepath`, a
//! cycleway, a one-way cycleway, a footway with and without `bicycle=yes`, a path.
//!
//! The import time is printed. In a release build it must be under 3 s (version 1: 2.0 s):
//! `cargo test --release -p openmobisim-io-osm --test delft -- --nocapture`.

mod common;

use std::path::Path;
use std::time::Instant;

use common::{VALUES, close, direction};

#[test]
fn delft_reads_in_full_and_twenty_ways_match_libosmium_and_pyproj() {
    let started = Instant::now();
    let (net, report) =
        openmobisim_io_osm::read(Path::new("tests/fixtures/delft/network.osm.pbf"), VALUES)
            .expect("Delft reads");
    let seconds = started.elapsed().as_secs_f64();
    println!("Delft: {report:?}, read in {seconds:.2} s");
    assert!(report.nodes > 0 && report.links > 0);
    if !cfg!(debug_assertions) {
        assert!(seconds < 3.0, "Delft read in {seconds:.2} s, the budget is 3 s");
    }

    let csv = std::fs::read_to_string("tests/fixtures/delft/ways.csv").expect("the fixture");
    let (mut rows, mut worst) = (0, 0.0f64);
    for line in csv.lines().skip(1) {
        let cell: Vec<&str> = line.split(',').collect();
        let way: i64 = cell[0].parse().unwrap();
        let length: f64 = cell[2].parse().unwrap();
        for (tag, at) in [("f", 3), ("b", 6)] {
            let got = direction(&net, way, tag);
            let case = format!("way {way} ({}) {tag}", cell[1]);
            if cell[at] == "-" {
                assert!(got.is_none(), "{case}: no link expected, got {got:?}");
                continue;
            }
            let got = got.unwrap_or_else(|| panic!("{case}: no link in the network"));
            assert_eq!(got.modes, cell[at], "{case}: modes");
            assert_eq!(got.lanes, cell[at + 1].parse().ok(), "{case}: lanes");
            let speed: Option<f64> = cell[at + 2].parse().ok();
            match (got.speed_km_h, speed) {
                (Some(a), Some(b)) => assert!(close(a, b, 1e-9), "{case}: {a} km/h, expected {b}"),
                (a, b) => assert_eq!(a, b, "{case}: speed"),
            }
            // Vincenty against Karney, summed over at most a few dozen street segments.
            worst = worst.max((got.length_m - length).abs());
            assert!(
                close(got.length_m, length, 1e-6),
                "{case}: {} m, pyproj {length} m",
                got.length_m
            );
        }
        rows += 1;
    }
    assert_eq!(rows, 20);
    println!("20 ways: largest length difference from pyproj {worst:.2e} m");
}
