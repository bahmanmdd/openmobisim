//! Run time, reported and not asserted: 10 000 random queries on a random timetable of 100
//! stops, 20 patterns and 200 runs, slack 60 s, on one thread and on eight. Run by hand, in a
//! release build:
//!
//! `cargo test --release -p openmobisim-transit --test run_time -- --ignored --nocapture`

mod common;

use std::time::Instant;

use common::{Draws, large_rows, random_queries};
use openmobisim_transit::raptor::earliest_many;
use openmobisim_types::units::Duration;
use rayon::ThreadPoolBuilder;

#[test]
#[ignore = "run time, by hand in a release build (see the module documentation)"]
fn ten_thousand_queries_on_100_stops_20_patterns_200_runs() {
    let mut draws = Draws::new(9);
    let tt = large_rows(&mut draws, 100, 20, 10).build();
    let queries = random_queries(&mut draws, 100, 10_000);
    let mut answers = Vec::new();
    for threads in [1, 8] {
        let pool = ThreadPoolBuilder::new().num_threads(threads).build().expect("a pool");
        let t = Instant::now();
        let journeys = pool.install(|| earliest_many(&tt, Duration(60.0), &queries));
        let s = t.elapsed().as_secs_f64();
        let answered = journeys.iter().flatten().count();
        eprintln!(
            "{threads} thread(s): 10 000 queries in {s:.4} s, {:.2} µs per query; {answered} with a journey",
            s * 1e6 / 1e4
        );
        answers.push(journeys);
    }
    assert_eq!(answers[0], answers[1], "1 and 8 threads give the same answers");
}
