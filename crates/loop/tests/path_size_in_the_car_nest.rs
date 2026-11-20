//! The logit reads the coefficient of `car_ln_path_size` times the nest scale, so that −1 is the
//! path-size logit (Ben-Akiva and Bierlaire 1999) inside the car nest at every nest scale.
//!
//! The nested logit divides a row's whole cost by the nest scale μ inside its nest, so a
//! coefficient `β` on `ln PS` puts the path size in a car route's weight at the exponent
//! `−β / μ`. The path-size logit's defining property (n identical copies of a route, each of
//! path size 1/n, count as the one route) needs the exponent 1, so `β = −μ`: the given −1 read
//! times μ.
//!
//! Judged: (a) with the default parameters at μ = 0.5, 0.25 and 1, the coefficients as the
//! choice reads them are −μ on `car_ln_path_size` and every other as given; two classes given
//! −0.9 are read −0.45 at the default μ = 0.5; a nest scale not in (0, 1] (0, 1.5, not a
//! number) leaves every coefficient as given, for the model to refuse the run by naming the
//! nest scale. (b) The loop's reading bound to the choice's formula: a `Logit` built in the test
//! from the parameters as read, over the loop's columns `COLUMNS`, with a nest per mode as the
//! choose stage makes them (this goes through the reading, not through a run; (c) does), over
//! three car routes of 3, 4.5 and 5.5 minutes driving with path sizes 0.5, 0.6 and 2/3, and a
//! bike of 4 minutes riding. At the default 0.1 per minute driving or riding and constants 0
//! for car and bike, their costs without the path-size term are 0.30, 0.45, 0.55 and 0.40. By
//! hand, the closed form (Ben-Akiva and Lerman 1985, chapter 10) with the path size at the
//! exponent 1, computed here from:
//!
//! * `P(j | car) = PS_j exp(−c_j / μ) / Σ_k PS_k exp(−c_k / μ)`;
//! * `I = ln Σ_k PS_k exp(−c_k / μ)`, the car nest's inclusive value;
//! * `P(car) = exp(μ I) / (exp(μ I) + exp(−c_bike))`, `P(bike) = 1 − P(car)`;
//! * `P(j) = P(car) · P(j | car)`;
//!
//! to 1e-12 at μ = 0.5, 0.25 and 1 (to six places, the three routes and the bike: 0.208358,
//! 0.185226, 0.168500, 0.437916 at μ = 0.5; 0.246365, 0.162250, 0.120844, 0.470541 at
//! μ = 0.25; 0.204879, 0.211609, 0.212747, 0.370765 at μ = 1). Read as given, −1 at μ = 0.5
//! would hold the path size squared and give the bike 0.505058.
//!
//! (c) The reading in a run: the cost a logit run reports for each trip, not the shares. The
//! network, demand and window of `path_size_in_a_run.rs`, whose header derives them by hand:
//! three car routes O→D of 800, 1 000 and 1 200 m sharing their first 600 m (`approach`), told
//! apart by their last link `short`, `middle` or `long`; 1 800 car-only travellers one a second
//! from second 0; a window of 4 h in which every trip finishes whatever the choices. The path
//! sizes by hand there: 0.5, 0.6 and 2/3 for short, middle and long with three routes in the
//! set; 0.625, 0.7 and 0.75 with two; 1 alone. Under the logit at μ = 0.5, 0.25 and 1, 1 per
//! minute on each time column, −1 on `car_ln_path_size` and 0 on the rest, 6 iterations, the
//! coefficient is read −μ, so by hand a trip's cost (the cost of the alternative it took, at the
//! last measure) is the minutes of that route, door to door at the last measure, less
//! `μ · ln PS`, PS the hand value of that route in its set: judged to 1e-9 for every trip. Read
//! as given (−1) the term would be off by `(1 − μ) · (−ln PS)`, read twice (−μ²) by
//! `μ (1 − μ) · (−ln PS)`: with `−ln PS ≥ ln(4/3) = 0.288` for a route in a set of two or
//! three, at least 0.05 minutes at μ = 0.5 and 0.25 (at μ = 1 both equal the reading, the plain
//! logit's case). The queues (`short` at 1 200 veh/h, `middle` at 600 veh/h) are only the means
//! by which the search finds the second and the third route; they are not judged. Guards, not
//! expected values: no iteration has an unfinished trip, and at least 100 trips end with two
//! car routes or more at each μ.
//!
//! For bikes: a bike has one route and no path size (its `car_ln_path_size` is 0 and it fills
//! only `bike_ride_min` and `is_bike`); in (b) it is the other nest, its probability against
//! the closed form; the travellers of (c) are car-only.

mod common;

use common::{TIME_ONLY, car_link, demand, network, stream, time_only};
use openmobisim_choice::cost::{Coefficients, Row, Traveller};
use openmobisim_choice::model::{Choice, Logit};
use openmobisim_loop::parameters::{COLUMNS, ChoiceModel, LN_PS, N, Parameters};
use openmobisim_loop::run::run;
use openmobisim_types::ids::{EntityId, TripId, UserClassId};
use openmobisim_types::mode::Mode;
use openmobisim_types::rng::RngKey;
use openmobisim_types::time::Second;

/// The car routes' minutes of driving and path sizes, and the bike's minutes of riding.
const CAR_MINUTES: [f64; 3] = [3.0, 4.5, 5.5];
const PATH_SIZES: [f64; 3] = [0.5, 0.6, 2.0 / 3.0];
const BIKE_MINUTES: f64 = 4.0;

/// The column named `name`.
fn column(name: &str) -> usize {
    COLUMNS.iter().position(|&c| c == name).expect("a column")
}

#[test]
fn the_logit_reads_the_path_size_coefficient_times_the_nest_scale_and_the_rest_as_given() {
    for mu in [0.5, 0.25, 1.0] {
        let given = Parameters { nest_scale: mu, ..Parameters::defaults(1) };
        let mut row = Parameters::DEFAULT_COEFFICIENTS;
        row[LN_PS] = -mu;
        let by_hand = Parameters { coefficients: vec![row], ..given.clone() };
        assert_eq!(given.as_the_choice_reads(), by_hand, "mu {mu}");
    }

    let mut row = Parameters::DEFAULT_COEFFICIENTS;
    row[LN_PS] = -0.9;
    let given = Parameters { coefficients: vec![row, row], ..Parameters::defaults(2) };
    assert_eq!(given.nest_scale, 0.5, "the default nest scale");
    let mut read = row;
    read[LN_PS] = -0.45;
    let by_hand = Parameters { coefficients: vec![read, read], ..given.clone() };
    assert_eq!(given.as_the_choice_reads(), by_hand, "two classes given -0.9");

    for mu in [0.0, 1.5, f64::NAN] {
        let given = Parameters { nest_scale: mu, ..Parameters::defaults(1) };
        let read = given.as_the_choice_reads();
        assert_eq!(read.coefficients, given.coefficients, "mu {mu}: the coefficients as given");
    }
}

#[test]
fn three_car_routes_and_a_bike_take_the_path_size_logit_in_the_car_nest_by_hand() {
    let (drive, ride, ln_ps) = (column("car_drive_min"), column("bike_ride_min"), LN_PS);
    let (is_car, is_bike) = (column("is_car"), column("is_bike"));
    let mut attributes: Vec<([f64; N], Mode)> = CAR_MINUTES
        .iter()
        .zip(&PATH_SIZES)
        .map(|(&minutes, &ps)| {
            let mut x = [0.0; N];
            (x[drive], x[ln_ps], x[is_car]) = (minutes, ps.ln(), 1.0);
            (x, Mode::Car)
        })
        .collect();
    let mut bike = [0.0; N];
    (bike[ride], bike[is_bike]) = (BIKE_MINUTES, 1.0);
    attributes.push((bike, Mode::Bike));
    let rows: Vec<Row<'_>> = attributes
        .iter()
        .map(|(x, mode)| Row { attributes: x, mode: *mode, nest: u32::from(mode.as_u8()) })
        .collect();
    let traveller = Traveller { trip: TripId::new(0), class: UserClassId::new(0) };

    for mu in [0.5, 0.25, 1.0] {
        let read = Parameters { nest_scale: mu, ..Parameters::defaults(1) }.as_the_choice_reads();
        let by_class: Vec<(UserClassId, Vec<f64>)> =
            vec![(UserClassId::from_index(0), read.coefficients[0].to_vec())];
        let coefficients = Coefficients::new(&COLUMNS, 1, &by_class).expect("valid");
        let logit = Logit::new(coefficients, read.nest_scale, RngKey::new(3)).expect("valid");
        let p = logit.probabilities(&traveller, &rows).expect("probabilities");

        // By hand: 0.1 per minute driving or riding, constants 0, the path size at exponent 1.
        let weight: Vec<f64> =
            CAR_MINUTES.iter().zip(&PATH_SIZES).map(|(m, ps)| ps * (-0.1 * m / mu).exp()).collect();
        let total: f64 = weight.iter().sum();
        let inclusive = total.ln();
        let bike_term = (-0.1 * BIKE_MINUTES).exp();
        let p_car = (mu * inclusive).exp() / ((mu * inclusive).exp() + bike_term);
        let mut expected: Vec<f64> = weight.iter().map(|w| p_car * w / total).collect();
        expected.push(1.0 - p_car);

        println!("mu {mu}: model {p:?}, by hand {expected:?}");
        let gap = p.iter().zip(&expected).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
        assert!(gap < 1e-12, "mu {mu}: {p:?} against {expected:?}");
    }
}

/// The hand path size of the route ending on `last` in a set of `routes` car routes (the header
/// of `path_size_in_a_run.rs`).
fn path_size_by_hand(last: &str, routes: usize) -> f64 {
    match (last, routes) {
        (_, 1) => 1.0,
        ("short", 2) => 0.625,
        ("middle", 2) => 0.7,
        ("long", 2) => 0.75,
        ("short", 3) => 0.5,
        ("middle", 3) => 0.6,
        ("long", 3) => 2.0 / 3.0,
        other => panic!("no hand value for {other:?}"),
    }
}

#[test]
fn in_a_logit_run_each_trips_cost_holds_the_path_size_term_times_the_nest_scale() {
    let unlimited = 1.0e9;
    let net = network(
        &[("O", 0.0, 0.0), ("J", 600.0, 0.0), ("D", 800.0, 0.0)],
        &[
            ("approach", "O", "J", car_link(600.0, 36.0, unlimited)),
            ("short", "J", "D", car_link(200.0, 36.0, 1_200.0)),
            ("middle", "J", "D", car_link(400.0, 36.0, 600.0)),
            ("long", "J", "D", car_link(600.0, 36.0, unlimited)),
        ],
    );
    let rows = stream(&net, "c", "O", "D", 3_600.0, 0.5, Second(0));
    assert_eq!(rows.len(), 1_800);
    let dem = demand(&net, &rows);
    let mut row = TIME_ONLY;
    row[LN_PS] = -1.0;
    for mu in [0.5, 0.25, 1.0] {
        let parameters = Parameters {
            choice: ChoiceModel::Logit,
            nest_scale: mu,
            coefficients: vec![row],
            ..time_only(6)
        };
        let result =
            run(&net, &dem, None, (Second(0), Second(4 * 3_600)), &parameters).expect("a run");
        for it in &result.iterations {
            assert_eq!(it.unfinished, 0, "mu {mu}, iteration {}: unfinished trips", it.iteration);
        }
        let mut overlapping = 0;
        for i in 0..rows.len() {
            let trip = TripId::from_index(i);
            let cars = result.set(trip).iter().filter(|a| a.mode == Mode::Car).count();
            let taken = result.taken(trip).expect("every trip has a car route");
            let route = taken.route().expect("a car route");
            let last = net.link_ids().external(route.last().expect("a link").raw());
            let want = taken.time / 60.0 - mu * path_size_by_hand(last, cars).ln();
            let cost = result.trips.cost[i];
            assert!((cost - want).abs() < 1e-9, "mu {mu}, trip {i}: cost {cost}, by hand {want}");
            overlapping += usize::from(cars > 1);
        }
        println!("mu {mu}: {overlapping} trips end with two car routes or more");
        assert!(overlapping >= 100, "mu {mu}: only {overlapping} trips with two routes or more");
    }
}
