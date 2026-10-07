//! Money in the choice (S248, roadmap I-bb U5): what an alternative costs, in euros, offered to
//! choice models as attributes.
//!
//! | attribute | what it is |
//! |---|---|
//! | `cost_running_eur` | the car or bike leg's kilometres times the vehicle's price per km ([`Prices::car_eur_km`], [`Prices::bike_eur_km`]) |
//! | `cost_toll_eur` | the car leg's tolls: the road link value named `toll_eur` ([`TOLL_COLUMN`], given with the run's link values, [`crate::link_values`]) summed over its links, paid at each passage |
//! | `cost_parking_eur` | the fee of the parking a park-and-ride or bike-and-ride trip leaves its vehicle at: the parking table's `fee_eur`, or the price for its kind ([`Prices::parking_car_eur`], [`Prices::parking_bike_eur`]); counted on the trip that parks, not on the one that fetches |
//! | `cost_fare_eur` | a transit journey's fare: [`Prices::fare_base_eur`] once, [`Prices::fare_km_eur`] per kilometre from each boarding stop to its alighting stop as the crow flies, and [`Prices::fare_transfer_eur`] per transfer |
//! | `cost_eur` | their sum |
//!
//! A car route of a trip given the car pays running costs and tolls; a walk pays nothing. Each
//! part is 0 where an alternative has none, so a model weighs the total, or the parts (a fare
//! felt more than fuel), or both.
//!
//! **Value of time.** The built-in logit and nested logit weigh money by `beta_cost_eur` (or a
//! part's own coefficient), **0 unless given**: the prices are offered, but no run changes until a
//! model weighs them. A value of time of `V` euros per hour is `beta_cost_eur = beta_time_min · 60
//! / V` (−1.2 per euro at −0.2 per minute and 10 €/h); a traveller class's own `beta_cost_eur` is
//! its own value of time.
//!
//! **Cost:** a few multiplications per alternative and attribute read; a toll is one pass over
//! the car leg's links, made only when a `toll_eur` column is given and a model reads money.

use openmobisim_core_graph::hubs::ParkingKind;
use openmobisim_core_types::ids::{EntityId, LinkId};

use crate::link_values::{Aggregate, PreparedLinkValues, ValueLayer};
use crate::parking::ParkingSetup;

/// The road link value that is a toll, in euros per passage.
pub const TOLL_COLUMN: &str = "toll_eur";

/// The money attributes, in the order a batch holds them: the total, then its parts.
pub const COST_ATTRIBUTES: [&str; 5] =
    ["cost_eur", "cost_running_eur", "cost_toll_eur", "cost_parking_eur", "cost_fare_eur"];

/// The prices a run charges: every one named, uncalibrated and overridable by name (S202) in a
/// scenario's `price_options`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Prices {
    /// What driving a car costs per kilometre, in euros: the fuel a driver pays as they go.
    ///
    /// *Uncalibrated: about 6.5 litres per 100 km at about 1.85 € a litre. CITATION OWED (the
    /// fleet's consumption and fuel prices; whether drivers perceive wear and depreciation).*
    pub car_eur_km: f64,
    /// What riding one's own bike costs per kilometre, in euros.
    ///
    /// *Uncalibrated: nothing, as most models assume; a shared bike's price comes with bike
    /// sharing (after the release).*
    pub bike_eur_km: f64,
    /// A transit journey's fare, once per journey, in euros.
    ///
    /// *Uncalibrated: a flat fare of the order of a single urban ticket in 2026 (Lyon's TCL
    /// ticket 2.10 €, Paris's metro-train-RER ticket 2.55 €; Amsterdam charges a base fare of
    /// 1.16 € and 0.217 € per km instead). Set a city's own.*
    pub fare_base_eur: f64,
    /// A transit journey's fare per kilometre ridden, in euros, the kilometres counted from
    /// each boarding stop to its alighting stop as the crow flies (as Dutch distance fares are).
    ///
    /// *Uncalibrated: 0, a flat fare.*
    pub fare_km_eur: f64,
    /// A transit journey's fare per transfer, in euros (a new ticket at each boarding: set it to
    /// the base fare).
    ///
    /// *Uncalibrated: 0, transfers free within a journey.*
    pub fare_transfer_eur: f64,
    /// A car park's fee per stay, in euros, where its table gives none.
    ///
    /// *Uncalibrated: free.*
    pub parking_car_eur: f64,
    /// A bike parking's fee per stay, in euros, where its table gives none.
    ///
    /// *Uncalibrated: free.*
    pub parking_bike_eur: f64,
}

impl Default for Prices {
    fn default() -> Self {
        Self::SHIPPED
    }
}

impl Prices {
    /// The shipped values.
    pub const SHIPPED: Prices = Prices {
        car_eur_km: 0.12,
        bike_eur_km: 0.0,
        fare_base_eur: 2.0,
        fare_km_eur: 0.0,
        fare_transfer_eur: 0.0,
        parking_car_eur: 0.0,
        parking_bike_eur: 0.0,
    };

    /// The names of the options.
    pub const NAMES: [&'static str; 7] = [
        "car_eur_km",
        "bike_eur_km",
        "fare_base_eur",
        "fare_km_eur",
        "fare_transfer_eur",
        "parking_car_eur",
        "parking_bike_eur",
    ];

    /// Every price, by name, in [`Self::NAMES`]' order (the parameter listing, the manifest).
    #[must_use]
    pub fn values(&self) -> Vec<(&'static str, f64)> {
        vec![
            ("car_eur_km", self.car_eur_km),
            ("bike_eur_km", self.bike_eur_km),
            ("fare_base_eur", self.fare_base_eur),
            ("fare_km_eur", self.fare_km_eur),
            ("fare_transfer_eur", self.fare_transfer_eur),
            ("parking_car_eur", self.parking_car_eur),
            ("parking_bike_eur", self.parking_bike_eur),
        ]
    }

    /// The shipped values with `options` in place of their namesakes.
    ///
    /// # Errors
    ///
    /// The name and the list of known names for an unknown option; the reason for a price that
    /// is not a finite number of at least 0.
    pub fn from_options(options: &std::collections::BTreeMap<String, f64>) -> Result<Self, String> {
        let mut p = Self::SHIPPED;
        for (name, &value) in options {
            let slot = match name.as_str() {
                "car_eur_km" => &mut p.car_eur_km,
                "bike_eur_km" => &mut p.bike_eur_km,
                "fare_base_eur" => &mut p.fare_base_eur,
                "fare_km_eur" => &mut p.fare_km_eur,
                "fare_transfer_eur" => &mut p.fare_transfer_eur,
                "parking_car_eur" => &mut p.parking_car_eur,
                "parking_bike_eur" => &mut p.parking_bike_eur,
                _ => {
                    return Err(format!(
                        "price_options has no {name:?}; the options are: {}",
                        Self::NAMES.join(", ")
                    ));
                }
            };
            if !(value.is_finite() && value >= 0.0) {
                return Err(format!("price_options {name:?} must be 0 or more, got {value}"));
            }
            *slot = value;
        }
        Ok(p)
    }
}

/// What an alternative pays, by part, in euros.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Cost {
    pub running: f64,
    pub toll: f64,
    pub parking: f64,
    pub fare: f64,
}

impl Cost {
    /// The money attribute `name`, if it is one ([`COST_ATTRIBUTES`]).
    pub(crate) fn attribute(&self, name: &str) -> Option<f64> {
        Some(match name {
            "cost_eur" => self.running + self.toll + self.parking + self.fare,
            "cost_running_eur" => self.running,
            "cost_toll_eur" => self.toll,
            "cost_parking_eur" => self.parking,
            "cost_fare_eur" => self.fare,
            _ => return None,
        })
    }
}

/// [`Prices`] ready for a run: each parking's fee resolved, the toll column found.
#[derive(Debug, Default)]
pub(crate) struct PreparedPrices {
    prices: Prices,
    /// Per parking, its fee per stay.
    parking_eur: Vec<f64>,
    /// The road link values' toll column, if given.
    toll: Option<usize>,
}

impl PreparedPrices {
    /// `prices` for a run with `parking` and the link values `values`.
    pub(crate) fn new(
        prices: Prices,
        parking: Option<&ParkingSetup>,
        values: &PreparedLinkValues,
    ) -> Self {
        let parking_eur = parking.map_or_else(Vec::new, |p| {
            (0..u32::try_from(p.count()).expect("parkings fit u32"))
                .map(|i| {
                    p.fee_eur(i).unwrap_or(match p.kind(i) {
                        ParkingKind::Car => prices.parking_car_eur,
                        ParkingKind::Bike => prices.parking_bike_eur,
                    })
                })
                .collect()
        });
        let toll = values.column(ValueLayer::Road, TOLL_COLUMN);
        Self { prices, parking_eur, toll }
    }

    /// The running cost of `metres` by `vehicle` (none: on foot or on board).
    pub(crate) fn running(&self, vehicle: Option<ParkingKind>, metres: f64) -> f64 {
        let per_km = match vehicle {
            Some(ParkingKind::Car) => self.prices.car_eur_km,
            Some(ParkingKind::Bike) => self.prices.bike_eur_km,
            None => 0.0,
        };
        per_km * metres / 1000.0
    }

    /// The tolls on the road links `links`.
    pub(crate) fn toll(&self, values: &PreparedLinkValues, links: &[LinkId]) -> f64 {
        self.toll.map_or(0.0, |c| values.total(c, Aggregate::Sum, links.iter().map(|l| l.index())))
    }

    /// The tolls on the road links `links`, given as raw link ids.
    pub(crate) fn toll_raw(&self, values: &PreparedLinkValues, links: &[u32]) -> f64 {
        self.toll
            .map_or(0.0, |c| values.total(c, Aggregate::Sum, links.iter().map(|&l| l as usize)))
    }

    /// The fee of parking `parking`.
    pub(crate) fn parking(&self, parking: u32) -> f64 {
        self.parking_eur.get(parking as usize).copied().unwrap_or(0.0)
    }

    /// The fare of a journey of `rides` rides covering `ride_m` metres as the crow flies; 0
    /// without a ride.
    pub(crate) fn fare(&self, rides: usize, ride_m: f64) -> f64 {
        if rides == 0 {
            return 0.0;
        }
        #[allow(clippy::cast_precision_loss, reason = "a handful of rides")]
        let transfers = (rides - 1) as f64;
        self.prices.fare_base_eur
            + self.prices.fare_km_eur * ride_m / 1000.0
            + self.prices.fare_transfer_eur * transfers
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_are_checked_by_name_and_value() {
        let set = |pairs: &[(&str, f64)]| {
            Prices::from_options(&pairs.iter().map(|(k, v)| ((*k).to_string(), *v)).collect())
        };
        assert_eq!(set(&[]).unwrap(), Prices::SHIPPED);
        let p = set(&[("fare_km_eur", 0.217), ("fare_base_eur", 1.16)]).unwrap();
        assert!((p.fare_km_eur - 0.217).abs() < 1e-12 && (p.fare_base_eur - 1.16).abs() < 1e-12);
        assert!(set(&[("fare", 1.0)]).unwrap_err().contains("fare_base_eur"));
        assert!(set(&[("car_eur_km", -0.1)]).unwrap_err().contains("0 or more"));
        assert!(set(&[("car_eur_km", f64::NAN)]).is_err());
        assert_eq!(Prices::SHIPPED.values().len(), Prices::NAMES.len());
        for ((name, _), known) in Prices::SHIPPED.values().iter().zip(Prices::NAMES) {
            assert_eq!(*name, known);
        }
    }

    #[test]
    #[allow(clippy::float_cmp, reason = "zeros and sums of exact binary fractions")]
    fn fares_running_costs_and_parts() {
        let prices = PreparedPrices {
            prices: Prices {
                fare_base_eur: 1.16,
                fare_km_eur: 0.217,
                fare_transfer_eur: 0.5,
                ..Prices::SHIPPED
            },
            parking_eur: vec![3.0],
            toll: None,
        };
        assert_eq!(prices.fare(0, 5000.0), 0.0);
        // Two rides, 10 km: base, ten kilometres, one transfer.
        assert!((prices.fare(2, 10_000.0) - (1.16 + 2.17 + 0.5)).abs() < 1e-12);
        assert!((prices.running(Some(ParkingKind::Car), 25_000.0) - 3.0).abs() < 1e-12);
        assert_eq!(prices.running(Some(ParkingKind::Bike), 25_000.0), 0.0);
        assert_eq!(prices.running(None, 25_000.0), 0.0);
        assert_eq!(prices.parking(0), 3.0);
        assert_eq!(prices.toll(&PreparedLinkValues::default(), &[LinkId::from_index(0)]), 0.0);
        let cost = Cost { running: 1.0, toll: 2.0, parking: 3.0, fare: 4.0 };
        assert_eq!(cost.attribute("cost_eur"), Some(10.0));
        assert_eq!(cost.attribute("cost_fare_eur"), Some(4.0));
        assert_eq!(cost.attribute("time_min"), None);
    }
}
