//! The path size inside a nest: the coefficient on `ln PS` at −μ gives the path-size logit
//! (Ben-Akiva and Bierlaire 1999) within the nest at every nest scale μ.
//!
//! The nested logit divides a row's whole cost by μ inside its nest, so a coefficient `β` on
//! `ln PS` puts the path size in the row's weight at the exponent `−β / μ`; at `β = −μ` the
//! exponent is 1 and a route's weight in its nest is `PS_j exp(−c_j / μ)`, `c_j` its cost
//! without the path-size term. By hand, the closed form (Ben-Akiva and Lerman 1985, chapter 10)
//! with the path size at the exponent 1, for car routes in one nest and a bike in its own:
//!
//! * `P(j | car) = PS_j exp(−c_j / μ) / Σ_k PS_k exp(−c_k / μ)`;
//! * `I = ln Σ_k PS_k exp(−c_k / μ)`, the car nest's inclusive value;
//! * `P(car) = exp(μ I) / (exp(μ I) + exp(−c_bike))`, `P(bike) = 1 − P(car)`, the bike alone
//!   in its nest (`exp(μ · (−c_bike / μ)) = exp(−c_bike)`);
//! * `P(j) = P(car) · P(j | car)`.
//!
//! Judged, at μ = 0.5 and 0.25: (a) three car routes of cost 0.30, 0.45 and 0.55 with path
//! sizes 0.5, 0.6 and 2/3, and a bike of cost 0.40, every probability against the closed form
//! computed here from the formulas above, to 1e-12 (to six places, the three routes and the
//! bike: 0.208358, 0.185226, 0.168500, 0.437916 at μ = 0.5; 0.246365, 0.162250, 0.120844,
//! 0.470541 at μ = 0.25; with the coefficient at −1 instead, the bike would take 0.505058 and
//! 0.573125); (b) the path-size logit's defining
//! property: the second route replaced by two identical copies, each of half its path size,
//! leaves every other row's probability unchanged and gives each copy half the route's, to
//! 1e-12.
//!
//! For bikes: a bike has one route and no path size (`ln PS` 0 in its row); here it is the
//! other nest, its probability against the closed form and unchanged by the copies.

mod common;

use common::{largest_gap, one_class, traveller};
use openmobisim_choice::cost::Row;
use openmobisim_choice::model::{Choice, Logit};
use openmobisim_types::mode::Mode;
use openmobisim_types::rng::RngKey;

/// The car routes' costs without the path-size term, and their path sizes.
const CAR_COSTS: [f64; 3] = [0.30, 0.45, 0.55];
const PATH_SIZES: [f64; 3] = [0.5, 0.6, 2.0 / 3.0];
/// The bike's cost.
const BIKE_COST: f64 = 0.40;

/// The closed form of the header: the car routes' probabilities, then the bike's.
fn by_hand(mu: f64) -> Vec<f64> {
    let weight: Vec<f64> =
        CAR_COSTS.iter().zip(&PATH_SIZES).map(|(c, ps)| ps * (-c / mu).exp()).collect();
    let total: f64 = weight.iter().sum();
    let inclusive = total.ln();
    let p_car = (mu * inclusive).exp() / ((mu * inclusive).exp() + (-BIKE_COST).exp());
    let mut p: Vec<f64> = weight.iter().map(|w| p_car * w / total).collect();
    p.push(1.0 - p_car);
    p
}

/// The model's probabilities over `routes` (cost, path size) by car in nest 0 and the bike in
/// nest 1, the coefficient on `ln PS` at −μ.
fn model(mu: f64, routes: &[(f64, f64)]) -> Vec<f64> {
    let mut attributes: Vec<Vec<f64>> = routes.iter().map(|&(c, ps)| vec![c, ps.ln()]).collect();
    attributes.push(vec![BIKE_COST, 0.0]);
    let rows: Vec<Row<'_>> = attributes
        .iter()
        .enumerate()
        .map(|(j, a)| {
            let bike = j == routes.len();
            let mode = if bike { Mode::Bike } else { Mode::Car };
            Row { attributes: a, mode, nest: u32::from(bike) }
        })
        .collect();
    let coefficients = one_class(&["cost", "ln_path_size"], &[1.0, -mu]);
    let logit = Logit::new(coefficients, mu, RngKey::new(5)).expect("valid");
    logit.probabilities(&traveller(0, 0), &rows).expect("probabilities")
}

fn three_routes() -> Vec<(f64, f64)> {
    CAR_COSTS.into_iter().zip(PATH_SIZES).collect()
}

#[test]
fn three_car_routes_and_a_bike_take_the_closed_form_at_every_nest_scale() {
    for mu in [0.5, 0.25] {
        let (p, expected) = (model(mu, &three_routes()), by_hand(mu));
        println!("mu {mu}: model {p:?}, by hand {expected:?}");
        assert!(largest_gap(&p, &expected) < 1e-12, "mu {mu}: {p:?} against {expected:?}");
    }
}

#[test]
fn a_route_split_into_two_copies_of_half_its_path_size_counts_as_the_one_route() {
    for mu in [0.5, 0.25] {
        let one = model(mu, &three_routes());
        let (cost, ps) = three_routes()[1];
        let split = [three_routes()[0], (cost, ps / 2.0), (cost, ps / 2.0), three_routes()[2]];
        let two = model(mu, &split);
        println!("mu {mu}: one route {one:?}, two copies {two:?}");
        let others = [one[0], one[2], one[3]];
        assert!(largest_gap(&[two[0], two[3], two[4]], &others) < 1e-12, "mu {mu}: {two:?}");
        let half = one[1] / 2.0;
        assert!(largest_gap(&[two[1], two[2]], &[half, half]) < 1e-12, "mu {mu}: {two:?}");
    }
}
