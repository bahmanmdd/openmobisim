//! The parameters of a run the loop reads (`PARAMETERS.md`), and the choice model's name.

use openmobisim_choice::cost::cost_of;

/// The columns of a choice row, in order: the times of each mode's parts in minutes (a car or
/// bike alternative's walks between door and node in `walk_min`), the transfers, euros, the
/// natural logarithm of a car route's path size, and one constant per mode (1 for the
/// alternative's mode, 0 for the others). An alternative fills only its mode's columns.
pub const COLUMNS: [&str; N] = [
    "car_drive_min",
    "bike_ride_min",
    "walk_min",
    "transit_in_vehicle_min",
    "transit_access_min",
    "transit_egress_min",
    "transit_first_wait_min",
    "transit_transfer_wait_min",
    "transit_transfer_walk_min",
    "transit_transfers",
    "money_eur",
    "car_ln_path_size",
    "is_car",
    "is_bike",
    "is_walk",
    "is_transit",
];

/// How many columns a choice row has.
pub const N: usize = 16;

/// The column `car_drive_min`: a car's minutes on the network.
pub const CAR_DRIVE_MIN: usize = 0;
/// The column `bike_ride_min`: a bike's minutes on the network.
pub const BIKE_RIDE_MIN: usize = 1;
/// The column `walk_min`: a walker's minutes, or a car's or a bike's walks between door and node.
pub const WALK_MIN: usize = 2;
/// The column `transit_in_vehicle_min`: a journey's minutes in vehicles.
pub const TRANSIT_IN_VEHICLE_MIN: usize = 3;
/// The column `transit_access_min`: a journey's walk to its first stop, minutes.
pub const TRANSIT_ACCESS_MIN: usize = 4;
/// The column `transit_egress_min`: a journey's walk from its last stop, minutes.
pub const TRANSIT_EGRESS_MIN: usize = 5;
/// The column `transit_first_wait_min`: from the end of the access walk to the first boarding,
/// minutes.
pub const TRANSIT_FIRST_WAIT_MIN: usize = 6;
/// The column `transit_transfer_wait_min`: a journey's waits at transfers, minutes.
pub const TRANSIT_TRANSFER_WAIT_MIN: usize = 7;
/// The column `transit_transfer_walk_min`: a journey's walks between stops, minutes.
pub const TRANSIT_TRANSFER_WALK_MIN: usize = 8;
/// The column `transit_transfers`: a journey's transfers, its rides less one.
pub const TRANSIT_TRANSFERS: usize = 9;
/// The column `money_eur`: euros.
pub const MONEY_EUR: usize = 10;
/// The column `car_ln_path_size`: the natural logarithm of a car route's path size.
pub const LN_PS: usize = 11;
/// The column `is_car`: the car's constant.
pub const IS_CAR: usize = 12;
/// The column `is_bike`: the bike's constant.
pub const IS_BIKE: usize = 13;
/// The column `is_walk`: the walker's constant.
pub const IS_WALK: usize = 14;
/// The column `is_transit`: transit's constant.
pub const IS_TRANSIT: usize = 15;

/// Whether `a` and `b` are the same string; a function the compiler can run, for the check below.
const fn same(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut k = 0;
    while k < a.len() {
        if a[k] != b[k] {
            return false;
        }
        k += 1;
    }
    true
}

/// Every name above with its column's string, one per column of `COLUMNS` in order: the array
/// has `N` entries, so a column added to `COLUMNS` (and `N` raised) stops the build until it is
/// named here.
const NAMED: [(usize, &str); N] = [
    (CAR_DRIVE_MIN, "car_drive_min"),
    (BIKE_RIDE_MIN, "bike_ride_min"),
    (WALK_MIN, "walk_min"),
    (TRANSIT_IN_VEHICLE_MIN, "transit_in_vehicle_min"),
    (TRANSIT_ACCESS_MIN, "transit_access_min"),
    (TRANSIT_EGRESS_MIN, "transit_egress_min"),
    (TRANSIT_FIRST_WAIT_MIN, "transit_first_wait_min"),
    (TRANSIT_TRANSFER_WAIT_MIN, "transit_transfer_wait_min"),
    (TRANSIT_TRANSFER_WALK_MIN, "transit_transfer_walk_min"),
    (TRANSIT_TRANSFERS, "transit_transfers"),
    (MONEY_EUR, "money_eur"),
    (LN_PS, "car_ln_path_size"),
    (IS_CAR, "is_car"),
    (IS_BIKE, "is_bike"),
    (IS_WALK, "is_walk"),
    (IS_TRANSIT, "is_transit"),
];

// Checked when the crate is built, position by position: the k-th name is position k and its
// string is `COLUMNS[k]`. So a column added, removed or moved in `COLUMNS` stops the build until
// the names follow it.
const _: () = {
    let mut k = 0;
    while k < N {
        assert!(NAMED[k].0 == k && same(COLUMNS[k], NAMED[k].1));
        k += 1;
    }
};

/// Which choice model a run uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChoiceModel {
    /// The least cost, ties to the first alternative of the set.
    Deterministic,
    /// The logit family with nests by mode at the nest scale; the plain logit at a scale of 1.
    Logit,
}

/// The loop's parameters. The defaults and their sources are listed in `PARAMETERS.md`.
#[derive(Clone, Debug, PartialEq)]
pub struct Parameters {
    /// The length of a bin of the links table, seconds. The link-time reading does not depend
    /// on it.
    pub link_bin: u32,
    /// The most iterations a run does, counting iteration 0.
    pub iteration_cap: u32,
    /// The iterations a run does before the stopping rule is read: 0 reads it from the first;
    /// at or above the cap the run goes to the cap.
    pub iteration_floor: u32,
    /// The run's seed: every random draw is a function of it, a stream and an identity.
    pub seed: u64,
    /// The seconds a traveller must be at the stop before a run leaves, every boarding.
    pub boarding_slack: u32,
    /// The most rides a transit journey may take.
    pub max_rides: u8,
    /// How many transit searches a trip end gets, at least 1: the first is between any stops
    /// in reach of both ends; each further one tries the next nearest stop of the end alone.
    pub nearby_stops: u8,
    /// The choice model.
    pub choice: ChoiceModel,
    /// The nest scale of the logit, in (0, 1]; read by [`ChoiceModel::Logit`] only.
    pub nest_scale: f64,
    /// Per user class, in class id order, one coefficient per column of [`COLUMNS`]: cost per
    /// minute of each time, per transfer, per euro; on `ln PS`, minus the exponent of the path
    /// size in a car route's weight inside its nest (read times the nest scale under the logit,
    /// as 0 under the least cost, [`Parameters::as_the_choice_reads`]); and each mode's
    /// constant.
    pub coefficients: Vec<[f64; N]>,
}

impl Parameters {
    /// The default coefficients of a class, in the order of [`COLUMNS`]: 0.1 per minute driving,
    /// riding a bike or in a vehicle; 0.165 per minute walking (to and from a stop, between
    /// stops, or the whole way); 0.145 per minute waiting; 0.5 per transfer; 0.6 per euro (a
    /// value of time of 10 euros per hour); −1 on `ln PS`, the path-size logit (Ben-Akiva and
    /// Bierlaire 1999) at every nest scale (see [`Parameters::as_the_choice_reads`]);
    /// constants 0 for car and bike, 0.5 for walk, 0.25 for transit. Starting values to be
    /// calibrated per study; the time, walking and waiting weights are in the proportions of
    /// release 0.1's built-in model.
    pub const DEFAULT_COEFFICIENTS: [f64; N] = [
        0.1, 0.1, 0.165, 0.1, 0.165, 0.165, 0.145, 0.145, 0.165, 0.5, 0.6, -1.0, 0.0, 0.0, 0.5,
        0.25,
    ];

    /// The defaults (`PARAMETERS.md`) for a run with `classes` user classes.
    #[must_use]
    pub fn defaults(classes: u32) -> Self {
        Self {
            link_bin: 300,
            iteration_cap: 50,
            iteration_floor: 0,
            seed: 0,
            boarding_slack: 60,
            max_rides: 4,
            nearby_stops: 3,
            choice: ChoiceModel::Logit,
            nest_scale: 0.5,
            coefficients: vec![Self::DEFAULT_COEFFICIENTS; classes as usize],
        }
    }

    /// These parameters as the run's choice reads them, the copy every stage of a run holds.
    ///
    /// The coefficient of `car_ln_path_size`, its sign turned, is the exponent of a car route's
    /// path size in the route's weight inside its nest: −1 is the path-size logit (Ben-Akiva and
    /// Bierlaire 1999) at every nest scale, in which n identical copies of a route, each of path
    /// size 1/n, count as the one route. The nested logit divides a row's whole cost by the nest
    /// scale μ inside its nest, so a route's weight there is `exp(−cost / μ)`, which holds the
    /// path size at the exponent `−β / μ` for a coefficient `β` on `ln PS`. Under the logit the
    /// coefficient is therefore read times μ, for every class: the default −1 is read as −0.5 at
    /// μ = 0.5 and as −1 at μ = 1.
    ///
    /// The path size corrects the logit for routes that overlap; the least-cost choice has no
    /// such error to correct, so under it the coefficient is read as 0 for every class, whatever
    /// number was given: its cost is that of the times, the transfers, the money and the
    /// constants.
    ///
    /// A coefficient that is not a finite number is left as given, and so is every coefficient
    /// under the logit when μ is not a finite number in (0, 1], for the model to refuse the run
    /// by naming what is wrong. The coefficients of the other columns are read as given.
    ///
    /// To be called once, on the parameters as given, as [`crate::run::run`] does: under the
    /// logit a second reading would multiply the coefficient by the nest scale again.
    #[must_use]
    pub fn as_the_choice_reads(&self) -> Self {
        let mut read = self.clone();
        let mu = read.nest_scale;
        let finite = read.coefficients.iter_mut().filter(|row| row[LN_PS].is_finite());
        match read.choice {
            ChoiceModel::Deterministic => finite.for_each(|row| row[LN_PS] = 0.0),
            ChoiceModel::Logit if mu > 0.0 && mu <= 1.0 => finite.for_each(|row| row[LN_PS] *= mu),
            ChoiceModel::Logit => {}
        }
        read
    }

    /// The cost of `attributes` (one per column) for class `class`: the systematic utility,
    /// formed by the choice crate's [`cost_of`], so that it is to the bit the cost the choice
    /// reads.
    #[must_use]
    pub fn cost(&self, class: usize, attributes: &[f64; N]) -> f64 {
        cost_of(&self.coefficients[class], attributes)
    }

    /// The cost of `attributes` without the path-size term: what decides whether a route the
    /// search found is cheaper than the set's cheapest. The column `car_ln_path_size` is left
    /// out of the sum (read as 0, its term adds nothing), so two alternatives that differ only
    /// in their path size have this cost to the bit.
    #[must_use]
    pub fn time_money_cost(&self, class: usize, attributes: &[f64; N]) -> f64 {
        let mut without = *attributes;
        without[LN_PS] = 0.0;
        self.cost(class, &without)
    }
}
