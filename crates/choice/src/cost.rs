//! What a choice is made over: the alternatives as rows of attributes, the traveller, and the
//! cost that weighs a row for the traveller's user class.
//!
//! # The cost
//!
//! The cost of an alternative `j` for a traveller of user class `c` is
//! `cost_j = Σ_k β[c][k] · x[j][k]`, summed over the columns in their order: `x[j][k]` is the
//! row's attribute in column `k` (in the column's own unit, named by the run: `time_s` in
//! seconds, `money_eur` in euros, `ln_path_size` without unit) and `β[c][k]` the class's
//! coefficient on it, in units of cost per unit of the attribute. Least cost is best. A
//! time-only run has a coefficient of 1 on `time_s` and 0 on the rest, so its cost is in
//! seconds. The coefficients are fixed once, when the [`Coefficients`] are built.
//!
//! The path-size logit is this cost with a column holding `ln PS` (the path-size factor of a
//! route, computed upstream by whoever knows the routes) and a coefficient of −1 on it: a route
//! that shares its links with others is made dearer by `−ln PS ≥ 0`. In the nested logit a
//! row's cost is divided by the nest scale `μ` inside its nest (see [`crate::model`]), so the
//! coefficient that gives the path-size logit within a nest is `−μ`.

use core::fmt;

use openmobisim_types::ids::{EntityId, TripId, UserClassId};
use openmobisim_types::mode::Mode;

/// One alternative offered to a traveller: its attributes, its mode and its nest.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Row<'a> {
    /// The attributes, one per column of the [`Coefficients`], in their order and units.
    pub attributes: &'a [f64],
    /// The alternative's mode. A label only: nothing here reads it.
    pub mode: Mode,
    /// The alternative's nest: rows with the same nest share it. Read by the nested logit only.
    pub nest: u32,
}

/// Who is choosing: what selects the coefficients and keys the random draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Traveller {
    /// The trip, which is also its traveller.
    pub trip: TripId,
    /// The traveller's user class: which coefficients weigh its rows.
    pub class: UserClassId,
}

/// Why a model could not be built, or a choice could not be made. Each names what it refuses.
#[derive(Clone, PartialEq, Debug)]
pub enum ChoiceError {
    /// A class's coefficients are not one per column.
    CoefficientCount {
        /// The class.
        class: UserClassId,
        /// How many columns there are.
        columns: usize,
        /// How many coefficients were given.
        coefficients: usize,
    },
    /// A coefficient is not a finite number.
    NotFinite {
        /// The class.
        class: UserClassId,
        /// The column's name.
        column: String,
        /// The coefficient given.
        value: f64,
    },
    /// A user class of the run was given no coefficients.
    MissingClass(UserClassId),
    /// A class the run does not have (its id at or above the class count, or null), named by a
    /// traveller or given coefficients; or a class given coefficients twice.
    UnknownClass(UserClassId),
    /// The nest scale μ is not in (0, 1].
    NestScale(f64),
    /// No alternative was offered.
    NoRows,
    /// A row's attributes are not one per column.
    RowLength {
        /// The row's index in the alternatives offered.
        row: usize,
        /// How many columns there are.
        columns: usize,
        /// How many attributes the row has.
        attributes: usize,
    },
    /// A row's cost is not a finite number (an attribute is infinite or NaN).
    NotFiniteCost {
        /// The row's index in the alternatives offered.
        row: usize,
    },
}

impl fmt::Display for ChoiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CoefficientCount { class, columns, coefficients } => write!(
                f,
                "user class {class} has {coefficients} coefficients for {columns} columns; give \
                 one per column"
            ),
            Self::NotFinite { class, column, value } => write!(
                f,
                "user class {class} has coefficient {value} on {column}; it must be finite"
            ),
            Self::MissingClass(class) => write!(f, "user class {class} has no coefficients"),
            Self::UnknownClass(class) => write!(
                f,
                "user class {class} is not one of the run's classes, or its coefficients were \
                 given twice"
            ),
            Self::NestScale(mu) => {
                write!(f, "the nest scale is {mu}; it must be above 0 and at most 1")
            }
            Self::NoRows => f.write_str("no alternative was offered"),
            Self::RowLength { row, columns, attributes } => write!(
                f,
                "row {row} has {attributes} attributes for {columns} columns; give one per column"
            ),
            Self::NotFiniteCost { row } => write!(f, "row {row} has a cost that is not finite"),
        }
    }
}

impl core::error::Error for ChoiceError {}

/// The cost of one row: `Σ_k β[k] · x[k]`, the products of each coefficient and its attribute
/// added in column order (see the module documentation). The one place the sum is formed: the
/// [`Coefficients`] weigh each row with it, and a caller that reports a cost forms it here too,
/// so that what it reports is, to the bit, what the choice reads. Columns past the shorter of the
/// two are not read.
#[must_use]
pub fn cost_of(coefficients: &[f64], attributes: &[f64]) -> f64 {
    coefficients.iter().zip(attributes).map(|(b, x)| b * x).sum()
}

/// The columns of the rows, by name, and each user class's coefficient on each: what turns a
/// row into its cost (see the module documentation). Fixed once built.
#[derive(Clone, Debug, PartialEq)]
pub struct Coefficients {
    columns: Vec<String>,
    /// Per class, by its index: one coefficient per column.
    by_class: Vec<Vec<f64>>,
}

impl Coefficients {
    /// The coefficients of a run with `classes` user classes (ids `0..classes`) over `columns`,
    /// from one `(class, coefficients)` pair per class, the coefficients in column order, each in
    /// units of cost per unit of its column's attribute.
    ///
    /// # Errors
    ///
    /// [`ChoiceError::CoefficientCount`] for a class whose coefficients are not one per column;
    /// [`ChoiceError::NotFinite`] for a coefficient that is not finite;
    /// [`ChoiceError::UnknownClass`] for a class at or above `classes`, or given twice;
    /// [`ChoiceError::MissingClass`] for a class below `classes` that was not given.
    pub fn new(
        columns: &[&str],
        classes: u32,
        by_class: &[(UserClassId, Vec<f64>)],
    ) -> Result<Self, ChoiceError> {
        let mut table: Vec<Option<Vec<f64>>> = vec![None; classes as usize];
        for (class, betas) in by_class {
            if betas.len() != columns.len() {
                let (columns, coefficients) = (columns.len(), betas.len());
                return Err(ChoiceError::CoefficientCount { class: *class, columns, coefficients });
            }
            if let Some((k, &value)) = betas.iter().enumerate().find(|(_, b)| !b.is_finite()) {
                let column = columns[k].to_string();
                return Err(ChoiceError::NotFinite { class: *class, column, value });
            }
            match table.get_mut(class.raw() as usize) {
                Some(slot @ None) => *slot = Some(betas.clone()),
                _ => return Err(ChoiceError::UnknownClass(*class)),
            }
        }
        let by_class = table
            .into_iter()
            .enumerate()
            .map(|(c, betas)| betas.ok_or(ChoiceError::MissingClass(UserClassId::from_index(c))))
            .collect::<Result<_, _>>()?;
        Ok(Self { columns: columns.iter().map(|c| (*c).to_string()).collect(), by_class })
    }

    /// The columns' names, in order.
    #[must_use]
    pub fn columns(&self) -> &[String] {
        &self.columns
    }

    /// How many user classes there are.
    #[must_use]
    pub fn classes(&self) -> usize {
        self.by_class.len()
    }

    /// The cost of every row for `traveller`, in row order.
    ///
    /// # Errors
    ///
    /// [`ChoiceError::NoRows`] for no rows; [`ChoiceError::UnknownClass`] for a class the
    /// coefficients do not have; [`ChoiceError::RowLength`] for a row whose attributes are not
    /// one per column; [`ChoiceError::NotFiniteCost`] for a cost that is not finite.
    pub fn costs(&self, traveller: &Traveller, rows: &[Row<'_>]) -> Result<Vec<f64>, ChoiceError> {
        if rows.is_empty() {
            return Err(ChoiceError::NoRows);
        }
        let betas = self.betas(traveller.class)?;
        let mut costs = Vec::with_capacity(rows.len());
        for (row, r) in rows.iter().enumerate() {
            if r.attributes.len() != betas.len() {
                let (columns, attributes) = (betas.len(), r.attributes.len());
                return Err(ChoiceError::RowLength { row, columns, attributes });
            }
            let cost = cost_of(betas, r.attributes);
            if !cost.is_finite() {
                return Err(ChoiceError::NotFiniteCost { row });
            }
            costs.push(cost);
        }
        Ok(costs)
    }

    /// The coefficients of `class`.
    fn betas(&self, class: UserClassId) -> Result<&[f64], ChoiceError> {
        let betas = if class.is_null() { None } else { self.by_class.get(class.index()) };
        betas.map(Vec::as_slice).ok_or(ChoiceError::UnknownClass(class))
    }
}
