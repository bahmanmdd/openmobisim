//! The parameters of a run the loop reads (`PARAMETERS.md`), and the choice model's name.

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
/// The column of the path size.
pub const LN_PS: usize = 11;

/// Which choice model a run uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChoiceModel {
    /// The least cost, ties to the first alternative of the set.
    Deterministic,
    /// The logit family with nests by mode at the nest scale; the plain logit at a scale of 1.
    Logit,
}

/// The loop's parameters. The defaults are the rulings's; see `PARAMETERS.md` for the sources.
#[derive(Clone, Debug, PartialEq)]
pub struct Parameters {
    /// The length of a bin of the links table and of the link-time reading, seconds.
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
    /// The choice model.
    pub choice: ChoiceModel,
    /// The nest scale of the logit, in (0, 1]; read by [`ChoiceModel::Logit`] only.
    pub nest_scale: f64,
    /// Per user class, in class id order, one coefficient per column of [`COLUMNS`]: cost per
    /// minute of each time, per transfer, per euro, per unit of `ln PS`, and each mode's
    /// constant.
    pub coefficients: Vec<[f64; N]>,
}

impl Parameters {
    /// The default coefficients of a class, in the order of [`COLUMNS`]: 0.1 per minute driving,
    /// riding a bike or in a vehicle; 0.165 per minute walking (to and from a stop, between
    /// stops, or the whole way); 0.145 per minute waiting; 0.5 per transfer; 0.6 per euro (a
    /// value of time of 10 euros per hour); −1 on `ln PS` (the path-size logit's definition);
    /// constants 0 for car and bike, 0.5 for walk, 0.25 for transit. Starting values to be
    /// calibrated per study; the time, walking and waiting weights are in the proportions of
    /// release 0.1's built-in model.
    pub const DEFAULT_COEFFICIENTS: [f64; N] = [
        0.1, 0.1, 0.165, 0.1, 0.165, 0.165, 0.145, 0.145, 0.165, 0.5, 0.6, -1.0, 0.0, 0.0, 0.5,
        0.25,
    ];

    /// The defaults of the rulings for a run with `classes` user classes.
    #[must_use]
    pub fn defaults(classes: u32) -> Self {
        Self {
            link_bin: 300,
            iteration_cap: 50,
            iteration_floor: 0,
            seed: 0,
            boarding_slack: 60,
            max_rides: 4,
            choice: ChoiceModel::Logit,
            nest_scale: 0.5,
            coefficients: vec![Self::DEFAULT_COEFFICIENTS; classes as usize],
        }
    }

    /// The cost of `attributes` (one per column) for class `class`: the systematic utility.
    #[must_use]
    pub fn cost(&self, class: usize, attributes: &[f64; N]) -> f64 {
        let b = &self.coefficients[class];
        b.iter().zip(attributes).map(|(b, x)| b * x).sum()
    }

    /// The cost of `attributes` without the path-size term: what decides whether a route the
    /// search found is cheaper than the set's cheapest.
    #[must_use]
    pub fn time_money_cost(&self, class: usize, attributes: &[f64; N]) -> f64 {
        self.cost(class, attributes) - self.coefficients[class][LN_PS] * attributes[LN_PS]
    }
}
