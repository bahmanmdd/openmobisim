//! Determinism, the one self-comparison: 1 000 queries on a random timetable of 100 stops, 20
//! patterns and 200 runs give the same journeys under one thread and under eight, and over two
//! runs built from scratch.

mod common;

use common::{Draws, large_rows, random_queries};
use openmobisim_transit::raptor::{Journey, earliest_many};
use openmobisim_types::units::Duration;
use rayon::ThreadPoolBuilder;

/// Build the timetable and the queries from the seed, and answer them on `threads` threads.
fn run(threads: usize) -> Vec<Option<Journey>> {
    let mut draws = Draws::new(7);
    let tt = large_rows(&mut draws, 100, 20, 10).build();
    let queries = random_queries(&mut draws, 100, 1_000);
    let pool = ThreadPoolBuilder::new().num_threads(threads).build().expect("a pool");
    pool.install(|| earliest_many(&tt, Duration(60.0), &queries))
}

#[test]
fn one_and_eight_threads_and_two_runs_give_the_same_journeys() {
    let one = run(1);
    let eight = run(8);
    let again = run(8);
    assert_eq!(one.len(), 1_000);
    let answered = one.iter().flatten().count();
    let transfers = one.iter().flatten().filter(|j| j.rides.len() >= 2).count();
    // Many answers, many of them transfers, or this compares little.
    assert!(answered >= 300 && transfers >= 100, "{answered} answered, {transfers} transfer");
    let first_difference =
        |a: &[Option<Journey>], b: &[Option<Journey>]| a.iter().zip(b).position(|(x, y)| x != y);
    assert_eq!(first_difference(&one, &eight), None, "1 and 8 threads differ at this query");
    assert_eq!(first_difference(&eight, &again), None, "two runs differ at this query");
    eprintln!("1 000 queries: {answered} with a journey, {transfers} of two rides or more");
}
