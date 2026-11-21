//! The cost of an alternative is formed in one place: the cost the loop reports (the trips
//! table, the gap, the rule of who chooses again) is, to the bit, the cost the choice reads; and
//! the cost without the path-size term, which decides whether a route the search found is
//! cheaper than the set's cheapest, leaves that term out instead of adding and subtracting it.
//!
//! Judged: (a) a property, over 200 alternatives of each mode built with awkward numbers (times
//! in thousandths of a second, walks in tenths, path sizes from 0.3 to 1, journeys of one to
//! three rides with footpaths of awkward length) and two classes (the default coefficients as
//! the logit reads them at a nest scale of 0.37, and a row of thirds, sevenths and elevenths):
//! `Parameters::cost` equals the cost `Coefficients::costs` gives the same row, bit for bit.
//! Both now call the one function that forms the sum, so (a) is a guard against the two ever
//! being formed apart again, not evidence about the change that made them one; (b) is that
//! evidence.
//! (b) By hand: two car routes of exactly the same door-to-door time, 189.5 s with a walk of
//! 22.5 s at each end (144.5 s on the network), one alone in its links (path size 1) and one
//! of path size 0.5, at the default coefficients as the logit reads them at its default nest
//! scale 0.5 (−0.5 on `car_ln_path_size`). Without the path-size term both cost
//! 0.1 · 144.5 / 60 + 0.165 · 45 / 60 = 0.2408333 + 0.12375 = 0.3645833, and the two are the
//! same number to the bit. The whole cost less the term would give the second 0.3645833333333333
//! against 0.36458333333333337 for the first: a route counted as cheaper at an exact tie.
//!
//! For bikes: a bike's row (its riding minutes, its walks, its constant) is one of the four
//! modes of (a); its path size is 0.

use openmobisim_choice::cost::{Coefficients, Row, Traveller};
use openmobisim_loop::alternative::{Alternative, Leg};
use openmobisim_loop::parameters::{COLUMNS, LN_PS, N, Parameters};
use openmobisim_transit::raptor::{Footpath, Journey, Ride};
use openmobisim_types::ids::{EntityId, StopId, TransitRunId, TripId, UserClassId};
use openmobisim_types::mode::Mode;
use openmobisim_types::time::Second;
use openmobisim_types::units::Duration;

/// A walk, a route of `mode` on no particular link, a walk.
fn on_the_network(mode: Mode, time: f64, walks: (f64, f64), ln_path_size: f64) -> Alternative {
    let legs = vec![
        Leg::Walk(Duration(walks.0)),
        Leg::Route { mode, links: Vec::new() },
        Leg::Walk(Duration(walks.1)),
    ];
    Alternative { mode, legs, time, ln_path_size }
}

/// A journey of `rides` rides of awkward lengths, the traveller leaving its door at second 0.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "seconds of a journey, above zero and far below u32::MAX"
)]
fn journey(k: u32, rides: u32) -> Alternative {
    let access = 13.7 + 0.3 * f64::from(k);
    let at_stop = Second(access.ceil() as u32);
    let mut t = at_stop.get() + 61 + 7 * k;
    let (mut all, mut footpaths) = (Vec::new(), Vec::new());
    for r in 0..rides {
        if r > 0 {
            let seconds = Duration(33.3 + 0.1 * f64::from(k));
            footpaths.push(Some(Footpath { from: StopId::new(r), to: StopId::new(r), seconds }));
            t += 120 + k;
        }
        let board = (StopId::new(r), Second(t));
        t += 311 + 13 * k;
        all.push(Ride { run: TransitRunId::new(r), board, alight: (StopId::new(r), Second(t)) });
    }
    let egress = 17.9 + 0.7 * f64::from(k);
    let last = Second(t);
    let journey = Journey {
        rides: all,
        footpaths,
        access: (StopId::new(0), at_stop),
        egress: (StopId::new(rides - 1), Duration(egress)),
        arrival: Second((f64::from(last.get()) + egress).ceil() as u32),
    };
    let legs =
        vec![Leg::Walk(Duration(access)), Leg::Journey(journey), Leg::Walk(Duration(egress))];
    Alternative {
        mode: Mode::Transit,
        legs,
        time: f64::from(last.get()) + egress,
        ln_path_size: 0.0,
    }
}

/// The alternatives of (a): per `k`, one of each mode.
fn awkward() -> Vec<Alternative> {
    let mut all = Vec::new();
    for k in 0..200_u32 {
        let x = f64::from(k);
        let time = 100.0 + 37.123_456_789 * x + 1.0 / 3.0;
        let walks = (0.1 * x + 22.5, 45.1 - 0.2 * x);
        let ln_path_size = (0.3 + 0.0035 * x).ln();
        all.push(on_the_network(Mode::Car, time, walks, ln_path_size));
        all.push(on_the_network(Mode::Bike, time * 2.7, walks, 0.0));
        all.push(on_the_network(Mode::Walk, time * 9.1, (0.0, 0.0), 0.0));
        all.push(journey(k, 1 + k % 3));
    }
    all
}

#[test]
fn the_cost_the_loop_reports_is_the_cost_the_choice_reads_to_the_bit() {
    let thirds: [f64; N] = [
        1.0 / 3.0,
        2.0 / 7.0,
        0.165,
        1.0 / 11.0,
        0.7 / 3.0,
        0.4 / 7.0,
        0.1 + 0.2,
        2.0 / 3.0,
        0.9 / 11.0,
        0.55,
        0.6,
        -1.0 / 3.0,
        0.1,
        -0.2 / 7.0,
        0.5,
        0.25,
    ];
    let mut parameters = Parameters { nest_scale: 0.37, ..Parameters::defaults(2) };
    parameters.coefficients[1] = thirds;
    let read = parameters.as_the_choice_reads();
    let by_class: Vec<(UserClassId, Vec<f64>)> =
        (0..2).map(|c| (UserClassId::from_index(c), read.coefficients[c].to_vec())).collect();
    let coefficients = Coefficients::new(&COLUMNS, 2, &by_class).expect("valid");
    let set = awkward();
    let attributes: Vec<[f64; N]> = set.iter().map(Alternative::attributes).collect();
    let rows: Vec<Row<'_>> = set
        .iter()
        .zip(&attributes)
        .map(|(a, x)| Row { attributes: x, mode: a.mode, nest: u32::from(a.mode.as_u8()) })
        .collect();
    for class in 0..2 {
        let traveller = Traveller { trip: TripId::new(0), class: UserClassId::from_index(class) };
        let read_by_the_choice = coefficients.costs(&traveller, &rows).expect("costs");
        for ((a, x), choice) in set.iter().zip(&attributes).zip(&read_by_the_choice) {
            let reported = read.cost(class, x);
            assert_eq!(
                reported.to_bits(),
                choice.to_bits(),
                "class {class}, {} of {} s: the loop reports {reported}, the choice reads {choice}",
                a.mode,
                a.time
            );
        }
    }
}

#[test]
fn two_routes_of_the_same_time_cost_the_same_without_the_path_size_term_to_the_bit() {
    let read = Parameters::defaults(1).as_the_choice_reads();
    assert!(read.coefficients[0][LN_PS].to_bits() == (-0.5_f64).to_bits(), "−1 read times 0.5");
    let alone = on_the_network(Mode::Car, 189.5, (22.5, 22.5), 0.0);
    let shared = on_the_network(Mode::Car, 189.5, (22.5, 22.5), 0.5_f64.ln());
    let (a, b) = (
        read.time_money_cost(0, &alone.attributes()),
        read.time_money_cost(0, &shared.attributes()),
    );
    assert_eq!(a.to_bits(), b.to_bits(), "path size 1: {a}; path size 0.5: {b}");
    let by_hand = 0.1 * 144.5 / 60.0 + 0.165 * 45.0 / 60.0;
    assert!((a - by_hand).abs() < 1e-12, "{a} against {by_hand}");
    // The path-size term is kept in the whole cost: −0.5 · ln 0.5 = 0.3466 more for the route
    // that shares its links.
    let more = read.cost(0, &shared.attributes()) - read.cost(0, &alone.attributes());
    assert!((more - 0.5 * 2.0_f64.ln()).abs() < 1e-12, "{more}");
}
