//! Test 6, the run time, reported and not asserted: the whole build of a demand of 13 000
//! trips and 300 stops on a 100 × 200 grid (20 000 nodes, 79 400 links), on one thread and on
//! eight. Run by hand, in a release build:
//!
//! `cargo test --release -p openmobisim-demand --test run_time -- --ignored --nocapture`

mod common;

use std::time::Instant;

use common::city;
use openmobisim_demand::demand::Demand;
use openmobisim_types::ids::{EntityId, TripId};
use openmobisim_types::units::Metres;
use rayon::ThreadPoolBuilder;

#[test]
#[ignore = "run time, by hand in a release build (see the module documentation)"]
fn thirteen_thousand_trips_and_300_stops_on_a_20_000_node_grid() {
    let t0 = Instant::now();
    let (net, tt, rows, classes) = city((100, 200), 300, 13_000, 12);
    eprintln!(
        "{} nodes, {} links, {} stops, {} trips made in {:.3} s",
        net.node_count(),
        net.link_count(),
        tt.stop_count(),
        rows.len(),
        t0.elapsed().as_secs_f64()
    );
    let mut built = Vec::new();
    for threads in [1, 8] {
        let pool = ThreadPoolBuilder::new().num_threads(threads).build().expect("a pool");
        let t = Instant::now();
        let d = pool
            .install(|| Demand::build(&rows, &classes, &net, Some(&tt), Metres(1_500.0)))
            .expect("builds");
        let s = t.elapsed().as_secs_f64();
        let entries: usize =
            TripId::iter_space(d.trip_count()).map(|t| d.access(t).len() + d.egress(t).len()).sum();
        eprintln!(
            "{threads} thread(s): built in {s:.3} s; {entries} list entries; {} of {} transit trips without a transit alternative",
            d.trips_without_transit(),
            TripId::iter_space(d.trip_count())
                .filter(|&t| d.modes(t).contains(openmobisim_types::mode::Mode::Transit))
                .count()
        );
        built.push(entries);
    }
    assert_eq!(built[0], built[1], "1 and 8 threads give the same lists");
}
