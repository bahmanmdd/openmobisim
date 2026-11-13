//! The network of Spiess and Florian (1989) as timetables (test 8 of the specification, its
//! second half): stops A, X, Y, B; line 1 A to B in 25 min every 12 min; line 2 A, X, Y in
//! 7 + 6 min every 12; line 3 X, Y, B in 4 + 4 min every 30; line 4 Y to B in 10 min every 6;
//! each line's first departure drawn from its headway by a fixed seed. A traveller every 10 s for two hours from A to B, slack 0, up to four rides.
//! RAPTOR must equal the brute force on every traveller. The mean door time is reported next to
//! the paper's 27.75 min, and not asserted: the paper's travellers follow the optimal strategy
//! knowing only the frequencies, these know the timetable, so they are expected to do better.

mod common;

use common::{Draws, Rows, brute_force, check_journey, stop_to_stop};
use openmobisim_transit::raptor::Raptor;
use openmobisim_types::ids::EntityId;
use openmobisim_types::units::Duration;

const A: usize = 0;
const X: usize = 1;
const Y: usize = 2;
const B: usize = 3;

#[test]
fn every_traveller_matches_the_brute_force_and_the_mean_is_reported() {
    let start = std::time::Instant::now();
    let mut draws = Draws::new(1989);
    let until = 4 * 3_600;
    let mut rows = Rows { stops: 4, ..Rows::default() };
    rows.line("1", &[A, B], &[1_500], draws.below(720), 720, until);
    rows.line("2", &[A, X, Y], &[420, 360], draws.below(720), 720, until);
    rows.line("3", &[X, Y, B], &[240, 240], draws.below(1_800), 1_800, until);
    rows.line("4", &[Y, B], &[600], draws.below(360), 360, until);
    let tt = rows.build();
    let mut raptor = Raptor::new(&tt, Duration::ZERO);
    let (mut door, mut n) = (0.0, 0.0);
    let mut by_lines = std::collections::BTreeMap::<String, u32>::new();
    for t in (3_600..3_600 + 7_200).step_by(10) {
        let q = stop_to_stop(A, t, B, 4);
        let j = raptor.earliest(&q).expect("line 1 always comes");
        let got = (j.arrival.get(), j.rides.len(), j.rides.last().unwrap().run.raw());
        assert_eq!(Some(got), brute_force(&rows, &tt, 0.0, &q), "traveller at {t} s");
        check_journey(&rows, &tt, 0.0, &q, &j);
        door += f64::from(j.arrival.get() - t);
        n += 1.0;
        let lines: Vec<&str> = j
            .rides
            .iter()
            .map(|r| tt.run_ids().external(r.run.raw()).split('-').next().unwrap())
            .collect();
        *by_lines.entry(lines.join(" then ")).or_default() += 1;
    }
    let minutes = door / n / 60.0;
    eprintln!(
        "{n} travellers: mean door time {minutes:.2} min, {:+.2} min from the paper's 27.75 (frequency-based); in {:.3} s",
        minutes - 27.75,
        start.elapsed().as_secs_f64()
    );
    eprintln!("lines taken: {by_lines:?}");
}
