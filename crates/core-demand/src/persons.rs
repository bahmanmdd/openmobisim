//! Reading the optional `persons.parquet` — per-traveller ownership and
//! class overrides (S127).

use std::path::Path;

use crate::DemandError;
use crate::columns;

/// One row of `persons.parquet`.
///
/// Every field but `traveller_id` is optional at two levels: the *column*
/// may be absent from the file entirely, or *one row's* value may be null —
/// either way it means "no override for this traveller", and
/// [`crate::travellers::build`] falls back to the class default (S127).
#[derive(Clone, Debug, Default)]
pub struct RawPerson {
    /// Must match a `traveller_id` in `trips.parquet`.
    pub traveller_id: String,
    /// Overrides the user class's default car ownership.
    pub owns_car: Option<bool>,
    /// Overrides the user class's default bike ownership.
    pub owns_bike: Option<bool>,
    /// Overrides the user class's default transit-pass ownership.
    pub has_transit_pass: Option<bool>,
    /// Overrides the user class stated in `trips.parquet`.
    pub user_class: Option<String>,
}

/// Read every row of `persons.parquet`, in file order.
///
/// # Errors
///
/// [`DemandError`] if the file cannot be opened or decoded, `traveller_id`
/// is missing, or a present optional column is of a type this reader does
/// not handle.
pub fn read_persons_parquet(path: impl AsRef<Path>) -> Result<Vec<RawPerson>, DemandError> {
    let batches = columns::read_batches(path.as_ref())?;
    let mut persons = Vec::new();
    for batch in &batches {
        let traveller_id = columns::required_id(batch, "traveller_id")?;
        let owns_car = columns::optional_bool(batch, "owns_car")?;
        let owns_bike = columns::optional_bool(batch, "owns_bike")?;
        let has_transit_pass = columns::optional_bool(batch, "has_transit_pass")?;
        let user_class = columns::optional_string(batch, "user_class")?;

        for i in 0..batch.num_rows() {
            persons.push(RawPerson {
                traveller_id: traveller_id[i].clone(),
                owns_car: owns_car.as_ref().and_then(|c| c[i]),
                owns_bike: owns_bike.as_ref().and_then(|c| c[i]),
                has_transit_pass: has_transit_pass.as_ref().and_then(|c| c[i]),
                user_class: user_class.as_ref().and_then(|c| c[i].clone()),
            });
        }
    }
    Ok(persons)
}
