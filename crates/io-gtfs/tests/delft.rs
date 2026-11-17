//! The Delft feed (`fixtures/delft/gtfs.zip`, 614 735 bytes, a cut for Delft of OVapi's Dutch
//! national feed) read on the Delft network
//! (`crates/io-osm/tests/fixtures/delft/network.osm.pbf`, © OpenStreetMap contributors, ODbL)
//! at `transfer_walk_max` 400 m. The busiest weekday, its trips, runs, calls and night runs
//! against `fixtures/delft/expected.csv`, written by `delft_gtfs.py` with gtfs-kit, which
//! reads the feed with pandas and decides which trips run on a date by the reference's rule.
//!
//! The read time (feed and footpaths, the network read before) is printed. In a release build
//! it must be under 3 s: `cargo test --release -p openmobisim-io-gtfs --test delft --
//! --ignored --nocapture` (release only: ignored in the debug suite).

mod common;

use std::path::Path;
use std::time::Instant;

use common::VALUES;
use openmobisim_types::units::Metres;

#[test]
#[ignore = "release only: about 7 s in debug; CI runs it in release with --ignored"]
fn delft_reads_and_its_counts_match_gtfs_kit() {
    let started = Instant::now();
    let (net, _) = openmobisim_io_osm::read(
        Path::new("../io-osm/tests/fixtures/delft/network.osm.pbf"),
        VALUES,
        None,
    )
    .expect("the Delft network reads");
    let network_seconds = started.elapsed().as_secs_f64();
    let started = Instant::now();
    let (t, report) =
        openmobisim_io_gtfs::read(Path::new("tests/fixtures/delft/gtfs.zip"), &net, Metres(400.0))
            .expect("the Delft feed reads");
    let seconds = started.elapsed().as_secs_f64();
    println!("Delft feed: {report:?}");
    println!("network read in {network_seconds:.2} s; feed and footpaths read in {seconds:.2} s");
    if !cfg!(debug_assertions) {
        assert!(seconds < 3.0, "the Delft feed read in {seconds:.2} s, the budget is 3 s");
    }

    let csv = std::fs::read_to_string("tests/fixtures/delft/expected.csv").expect("the fixture");
    let mut want = std::collections::HashMap::new();
    for line in csv.lines().skip(1) {
        let (k, v) = line.split_once(',').expect("key,value");
        want.insert(k.to_owned(), v.parse::<u64>().expect("a count"));
    }
    let got = [
        ("date", u64::from(report.date)),
        ("trips", u64::from(report.trips)),
        ("runs", u64::from(report.runs)),
        ("calls", report.calls),
        ("night_runs", u64::from(report.night_runs)),
        ("stops", u64::from(report.stops)),
    ];
    for (k, v) in got {
        assert_eq!(Some(&v), want.get(k), "{k}: read {v}, gtfs-kit {:?}", want.get(k));
    }
    assert_eq!(u64::from(t.run_count()), want["runs"]);
    assert_eq!(u64::from(t.stop_count()), want["stops"]);
}
