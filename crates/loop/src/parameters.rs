//! The parameters of a run the loop reads (`PARAMETERS.md`), and the choice model's name.

/// The columns of a choice row, in order: seconds, euros, the natural logarithm of the route's
/// path size.
pub const COLUMNS: [&str; 3] = ["time_s", "money_eur", "ln_path_size"];

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
    /// second, cost per euro, cost per unit of `ln PS`.
    pub coefficients: Vec<[f64; 3]>,
}

impl Parameters {
    /// the rulings's coefficients of a class: a time scale of 1/600 per second (0.1 per
    /// minute), a value of time of 10 euros per hour (0.6 per euro), and −1 on `ln PS` (the
    /// path-size logit's definition).
    pub const DEFAULT_COEFFICIENTS: [f64; 3] = [1.0 / 600.0, 0.6, -1.0];

    /// The defaults of the rulings for a run with `classes` user classes.
    #[must_use]
    pub fn defaults(classes: u32) -> Self {
        Self {
            link_bin: 300,
            iteration_cap: 50,
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
    pub fn cost(&self, class: usize, attributes: &[f64; 3]) -> f64 {
        let b = &self.coefficients[class];
        b[0] * attributes[0] + b[1] * attributes[1] + b[2] * attributes[2]
    }

    /// The cost of `attributes` on time and money alone: what decides whether a route the search
    /// found is cheaper than the set's cheapest.
    #[must_use]
    pub fn time_money_cost(&self, class: usize, attributes: &[f64; 3]) -> f64 {
        let b = &self.coefficients[class];
        b[0] * attributes[0] + b[1] * attributes[1]
    }
}
