//! Test 5, determinism, the one self-comparison: a demand of 3 000 trips and 60 stops on a
//! 40 × 40 grid is the same, bit for bit, under one thread and under eight, and over two runs
//! built from scratch.

mod common;

use common::city;
use openmobisim_demand::demand::Demand;
use openmobisim_types::ids::{EntityId, StopId, TripId};
use openmobisim_types::mode::Mode;
use openmobisim_types::units::Metres;
use rayon::ThreadPoolBuilder;

/// Everything the demand holds, as bits: per trip its class, departure, modes, ends per mode
/// and both lists; per stop its node and gap; the three counts.
fn bits(d: &Demand) -> Vec<u64> {
    let mut out = Vec::new();
    for t in TripId::iter_space(d.trip_count()) {
        out.push(u64::from(d.class(t).raw()));
        out.push(u64::from(d.departure(t).get()));
        out.extend(Mode::ALL.map(|m| u64::from(d.modes(t).contains(m))));
        for m in Mode::ALL {
            for end in [d.origin_end(t, m), d.destination_end(t, m)] {
                out.push(end.map_or(u64::MAX, |e| u64::from(e.node.raw())));
                out.push(end.map_or(u64::MAX, |e| e.gap.get().to_bits()));
            }
        }
        for list in [d.access(t), d.egress(t)] {
            out.push(list.len() as u64);
            for &(s, seconds) in list {
                out.extend([u64::from(s.raw()), seconds.get().to_bits()]);
            }
        }
    }
    for s in StopId::iter_space(d.stop_count()) {
        out.extend([u64::from(d.stop_end(s).node.raw()), d.stop_end(s).gap.get().to_bits()]);
    }
    out.extend(
        [d.origins_without_stop(), d.destinations_without_stop(), d.trips_without_transit()]
            .map(u64::from),
    );
    out
}

/// Build the city's demand on a pool of `threads` threads; its bits and its list entries.
fn run(threads: usize) -> (Vec<u64>, usize) {
    let (net, tt, rows, classes) = city((40, 40), 60, 3_000, 11);
    let pool = ThreadPoolBuilder::new().num_threads(threads).build().expect("a pool");
    let d = pool
        .install(|| Demand::build(&rows, &classes, &net, Some(&tt), Metres(1_500.0)))
        .expect("builds");
    let entries =
        TripId::iter_space(d.trip_count()).map(|t| d.access(t).len() + d.egress(t).len()).sum();
    (bits(&d), entries)
}

#[test]
fn one_and_eight_threads_and_two_runs_give_the_same_demand_to_the_bit() {
    let (one, entries) = run(1);
    let (eight, _) = run(8);
    let (again, _) = run(8);
    // The lists must be full, or this compares empty lists only.
    assert!(entries >= 3_000, "only {entries} list entries");
    let first_difference = |a: &[u64], b: &[u64]| a.iter().zip(b).position(|(x, y)| x != y);
    assert_eq!(one.len(), eight.len());
    assert_eq!(first_difference(&one, &eight), None, "1 and 8 threads differ at this value");
    assert_eq!(first_difference(&eight, &again), None, "two runs differ at this value");
    eprintln!(
        "{} values, {entries} list entries, the same at 1 and 8 threads and twice",
        one.len()
    );
}
