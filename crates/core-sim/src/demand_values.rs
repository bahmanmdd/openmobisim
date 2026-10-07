//! Numbers the user gives per traveller and per trip, offered to choice models (S249, roadmap
//! I-bb U6): a traveller's income or age, a trip's purpose or the parking fee at its
//! destination — anything a utility may weigh by who travels and why.
//!
//! A column named `name` becomes an attribute of every alternative of the traveller's (or the
//! trip's) choices: `person_<name>` or `trip_<name>`, the same value for each alternative of one
//! choice. Built in beside them: `trip_departure_h`, the trip's departure in hours after
//! midnight.
//!
//! Being the same for every alternative of a choice, such a value cancels out of a logit on its
//! own: a built-in model weighs it **in a product with another attribute**, a coefficient named
//! `beta_<a>*<b>` (`beta_mode_car*person_age`, a car constant that changes with age;
//! `beta_cost_eur*person_income_inv`, money weighing less as income grows, the user giving
//! `income_inv` as one over the income). A model of the user's own reads them as it likes.
//!
//! **The parking fee at a trip's destination** (S248, U5): a trip column named `parking_eur`
//! ([`DESTINATION_PARKING`]) is what parking a car where the trip ends costs, in euros; it is
//! added to `cost_parking_eur` of every alternative that arrives by car (a car route, or the
//! drive home from a park-and-ride).
//!
//! **Cost:** one value per traveller or trip and column; per alternative and attribute read, one
//! look-up.

use openmobisim_core_demand::Trips;
use openmobisim_core_types::ids::{EntityId, TravellerId, TripId};

/// The trip column that is the parking fee at the trip's destination, in euros.
pub const DESTINATION_PARKING: &str = "parking_eur";

/// The built-in situation attribute: the trip's departure, in hours after midnight.
pub const DEPARTURE_ATTRIBUTE: &str = "trip_departure_h";

/// Where an attribute of a situation comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Source {
    /// The traveller's column `i`.
    Person(usize),
    /// The trip's column `i`.
    Trip(usize),
    /// The trip's departure.
    Departure,
}

/// The user's traveller and trip values: see the [module docs](self).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DemandValues {
    persons: Vec<(String, Vec<f64>)>,
    trips: Vec<(String, Vec<f64>)>,
    /// The trip column that is the destination's parking fee, if given.
    parking: Option<usize>,
}

impl DemandValues {
    /// No values.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn check(name: &str, what: &str, values: &[f64], reserved: &[&str]) -> Result<(), String> {
        let label = format!("{what} value {name:?}");
        if name.is_empty()
            || !name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        {
            return Err(format!("{label}: a name is lower-case letters, digits and _"));
        }
        let attribute = format!("{what}_{name}");
        if reserved.contains(&attribute.as_str()) || attribute == DEPARTURE_ATTRIBUTE {
            return Err(format!("{label}: {attribute} is a built-in attribute; choose another"));
        }
        if let Some(i) = values.iter().position(|v| !v.is_finite()) {
            return Err(format!("{label}: entry {i} has {}, not a finite number", values[i]));
        }
        Ok(())
    }

    /// The same values with a traveller column `name`, one value per traveller, by traveller
    /// index ([`TravellerId`]).
    ///
    /// # Errors
    ///
    /// A message for a name that is empty, not lower-case letters, digits and `_`, given twice,
    /// or whose attribute would take a built-in's name (`reserved`); or for a value that is not
    /// finite.
    pub fn with_person(
        mut self,
        name: &str,
        values: Vec<f64>,
        reserved: &[&str],
    ) -> Result<Self, String> {
        Self::check(name, "person", &values, reserved)?;
        if self.persons.iter().any(|(n, _)| n == name) {
            return Err(format!("person value {name:?} is given twice"));
        }
        self.persons.push((name.to_string(), values));
        Ok(self)
    }

    /// The same values with a trip column `name`, one value per trip, by trip index
    /// ([`TripId`]).
    ///
    /// # Errors
    ///
    /// As [`Self::with_person`].
    pub fn with_trip(
        mut self,
        name: &str,
        values: Vec<f64>,
        reserved: &[&str],
    ) -> Result<Self, String> {
        Self::check(name, "trip", &values, reserved)?;
        if self.trips.iter().any(|(n, _)| n == name) {
            return Err(format!("trip value {name:?} is given twice"));
        }
        if name == DESTINATION_PARKING {
            if let Some(i) = values.iter().position(|v| *v < 0.0) {
                return Err(format!("trip value {name:?}: entry {i} is below 0"));
            }
            self.parking = Some(self.trips.len());
        }
        self.trips.push((name.to_string(), values));
        Ok(self)
    }

    /// Whether there is no column.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.persons.is_empty() && self.trips.is_empty()
    }

    /// The traveller columns as `(name, values)`, in the order given.
    pub fn persons(&self) -> impl Iterator<Item = (&str, &[f64])> {
        self.persons.iter().map(|(n, v)| (n.as_str(), v.as_slice()))
    }

    /// The trip columns as `(name, values)`, in the order given.
    pub fn trips(&self) -> impl Iterator<Item = (&str, &[f64])> {
        self.trips.iter().map(|(n, v)| (n.as_str(), v.as_slice()))
    }

    /// The attributes the columns make: each traveller column's `person_<name>`, then each trip
    /// column's `trip_<name>`, in the order given.
    #[must_use]
    pub fn attribute_names(&self) -> Vec<String> {
        self.persons
            .iter()
            .map(|(n, _)| format!("person_{n}"))
            .chain(self.trips.iter().map(|(n, _)| format!("trip_{n}")))
            .collect()
    }

    /// Check the columns against a demand of `travellers` and `trips`.
    ///
    /// # Errors
    ///
    /// A message for a column whose length is not the count it is per.
    pub(crate) fn check_lengths(&self, travellers: usize, trips: usize) -> Result<(), String> {
        for (name, values) in &self.persons {
            if values.len() != travellers {
                return Err(format!(
                    "person value {name:?} has {} values; the demand has {travellers} travellers",
                    values.len()
                ));
            }
        }
        for (name, values) in &self.trips {
            if values.len() != trips {
                return Err(format!(
                    "trip value {name:?} has {} values; the demand has {trips} trips",
                    values.len()
                ));
            }
        }
        Ok(())
    }

    /// Where attribute `name` comes from, if it is a situation's.
    pub(crate) fn lookup(&self, name: &str) -> Option<Source> {
        if name == DEPARTURE_ATTRIBUTE {
            return Some(Source::Departure);
        }
        if let Some(n) = name.strip_prefix("person_") {
            return self.persons.iter().position(|(p, _)| p == n).map(Source::Person);
        }
        let n = name.strip_prefix("trip_")?;
        self.trips.iter().position(|(t, _)| t == n).map(Source::Trip)
    }

    /// The value from `source` for `traveller`'s `trip`.
    pub(crate) fn value(
        &self,
        source: Source,
        trips: &Trips,
        traveller: TravellerId,
        trip: TripId,
    ) -> f64 {
        match source {
            Source::Person(i) => self.persons[i].1[traveller.index()],
            Source::Trip(i) => self.trips[i].1[trip.index()],
            Source::Departure => f64::from(trips.departure(trip).get()) / 3600.0,
        }
    }

    /// The parking fee at `trip`'s destination ([`DESTINATION_PARKING`]), 0 if not given.
    pub(crate) fn destination_parking(&self, trip: TripId) -> f64 {
        self.parking.map_or(0.0, |i| self.trips[i].1[trip.index()])
    }
}
