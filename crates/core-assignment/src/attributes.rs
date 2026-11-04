//! What an alternative is made of, and what it is worth to a user class.
//!
//! **Attributes** are the measurable components of a leg or of a whole chain: the seconds spent
//! in a vehicle, walking, waiting and parking, the number of transfers, the money paid, the
//! length in metres and the metres ridden by bike in mixed traffic. A chain's attributes are the
//! sums over its legs.
//!
//! **Coefficients** are a user class's weights on those components: what one minute in a
//! vehicle, one minute walking, one minute waiting, one minute parking, one transfer, one unit
//! of money, one kilometre and one kilometre ridden by bike in mixed traffic cost the class. They
//! define the **systematic utility** of an alternative,
//!
//! ```text
//! V = −( β_ivt · in-vehicle minutes + β_walk · walking minutes + β_wait · waiting minutes
//!        + β_park · parking minutes + β_transfer · transfers + β_money · money
//!        + β_km · kilometres + β_mixed · kilometres by bike in mixed traffic )
//! ```
//!
//! A bike's ride is in-vehicle time (the bike is the traveller's vehicle), so a minute riding
//! costs `β_ivt`, as a minute driving; the kilometres ridden in mixed traffic (on a link without
//! a cycle lane or track of its own) cost `β_mixed` each **on top of** that time.
//!
//! a negative number in **utility units**; the **cost** of the alternative is `−V`. Each
//! coefficient is therefore the cost, in utility units, of one unit of its attribute: positive
//! for a component the class dislikes. The cost is the one quantity the whole assignment
//! compares: the route search minimises it link by link, the choice model reads it, and the
//! convergence measures are expressed in it.
//!
//! **A time-based run** is a run whose classes all have [`Coefficients::time_based`]: every
//! minute of door-to-door time (in a vehicle, walking, waiting or parking) costs 1, everything
//! else 0,
//! so a cost reads in minutes. It is not a separate code path; it is these coefficients.
//!
//! **Money is per link, always.** A toll or a distance charge is a value per link of the
//! network (zero where none is given) that the car planner adds to each link it prices; a run
//! without tolls has a vector of zeros, not a missing option.

/// The components of a leg or a chain, in seconds, metres, transfers and units of money.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Attributes {
    /// Seconds spent travelling in a vehicle (driving, for a car leg; riding, for a bike leg),
    /// from the moment the vehicle sets off to the moment it arrives, including any wait to get
    /// onto the first link.
    pub in_vehicle_seconds: f64,
    /// Seconds spent walking (access, egress or transfer on foot).
    pub walk_seconds: f64,
    /// Seconds spent waiting for a vehicle or a service.
    pub wait_seconds: f64,
    /// Seconds spent parking a vehicle and leaving it (a car leg's egress time, after the
    /// vehicle arrives), or fetching a parked one.
    pub parking_seconds: f64,
    /// How many times the traveller changes vehicle.
    pub transfers: u32,
    /// Money paid, in the run's unit of currency (tolls, fares, fees).
    pub money: f64,
    /// Distance travelled, in metres.
    pub length_metres: f64,
    /// Metres ridden by bike in mixed traffic: on bike-layer links without a cycle lane or a
    /// track of their own (a ferry crossing counts as none); 0 for every other leg.
    pub bike_mixed_metres: f64,
}

impl Attributes {
    /// The door-to-door time: in-vehicle, walking, waiting and parking seconds together.
    #[must_use]
    pub fn total_seconds(&self) -> f64 {
        self.in_vehicle_seconds + self.walk_seconds + self.wait_seconds + self.parking_seconds
    }

    /// The component-wise sum of `self` and `other`: the attributes of two legs one after the
    /// other.
    #[must_use]
    pub fn plus(&self, other: &Self) -> Self {
        Self {
            in_vehicle_seconds: self.in_vehicle_seconds + other.in_vehicle_seconds,
            walk_seconds: self.walk_seconds + other.walk_seconds,
            wait_seconds: self.wait_seconds + other.wait_seconds,
            parking_seconds: self.parking_seconds + other.parking_seconds,
            transfers: self.transfers + other.transfers,
            money: self.money + other.money,
            length_metres: self.length_metres + other.length_metres,
            bike_mixed_metres: self.bike_mixed_metres + other.bike_mixed_metres,
        }
    }
}

/// A user class's weights on each attribute: the cost in utility units of one unit of it.
///
/// See the [module docs](self) for the utility they define.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Coefficients {
    /// Cost of one minute in a vehicle.
    pub in_vehicle_per_minute: f64,
    /// Cost of one minute walking.
    pub walk_per_minute: f64,
    /// Cost of one minute waiting.
    pub wait_per_minute: f64,
    /// Cost of one minute parking (and leaving the vehicle, or fetching it).
    pub parking_per_minute: f64,
    /// Cost of one transfer.
    pub per_transfer: f64,
    /// Cost of one unit of money.
    pub per_money: f64,
    /// Cost of one kilometre travelled.
    pub per_kilometre: f64,
    /// Cost of one kilometre ridden by bike in mixed traffic, on top of the ride's time and
    /// kilometres.
    pub bike_mixed_per_kilometre: f64,
}

impl Coefficients {
    /// The coefficients of a time-based run: one minute of door-to-door time (in a vehicle,
    /// walking, waiting or parking) costs 1; transfers, money, distance and the kilometres
    /// ridden in mixed traffic cost 0. A cost then reads in minutes of travel time.
    #[must_use]
    pub const fn time_based() -> Self {
        Self {
            in_vehicle_per_minute: 1.0,
            walk_per_minute: 1.0,
            wait_per_minute: 1.0,
            parking_per_minute: 1.0,
            per_transfer: 0.0,
            per_money: 0.0,
            per_kilometre: 0.0,
            bike_mixed_per_kilometre: 0.0,
        }
    }

    /// The systematic utility of an alternative with `attributes`, in utility units (zero or
    /// negative for coefficients that are zero or positive).
    #[must_use]
    pub fn utility(&self, attributes: &Attributes) -> f64 {
        -self.cost(attributes)
    }

    /// The cost of an alternative with `attributes`: minus its [utility](Self::utility).
    #[must_use]
    pub fn cost(&self, attributes: &Attributes) -> f64 {
        self.in_vehicle_per_minute * attributes.in_vehicle_seconds / 60.0
            + self.walk_per_minute * attributes.walk_seconds / 60.0
            + self.wait_per_minute * attributes.wait_seconds / 60.0
            + self.parking_per_minute * attributes.parking_seconds / 60.0
            + self.per_transfer * f64::from(attributes.transfers)
            + self.per_money * attributes.money
            + self.per_kilometre * attributes.length_metres / 1000.0
            // Last, so a chain without a bike leg (0 here) costs what it cost without the term,
            // to the bit.
            + self.bike_mixed_per_kilometre * attributes.bike_mixed_metres / 1000.0
    }
}

impl Default for Coefficients {
    /// [`Coefficients::time_based`].
    fn default() -> Self {
        Self::time_based()
    }
}
