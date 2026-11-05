//! What an alternative is made of, and what it is worth to a user class.
//!
//! **Attributes** are the measurable components of a leg or of a whole chain: the seconds spent
//! in a vehicle, walking, waiting and parking, the number of transfers, the money paid, the
//! length in metres and the metres ridden by bike on each kind of facility (in mixed traffic, on
//! a painted lane, on a track or path of its own). A chain's attributes are the sums over its
//! legs.
//!
//! **Coefficients** are a user class's weights on those components: what one minute in a
//! vehicle, one minute walking, one minute waiting, one minute parking, one transfer, one unit
//! of money, one kilometre and one kilometre ridden by bike on each kind of facility cost the
//! class, and what choosing each mode costs it on top (the mode's constant). They define the
//! **systematic utility** of an alternative,
//!
//! ```text
//! V = −( β_ivt · in-vehicle minutes + β_walk · walking minutes + β_wait · waiting minutes
//!        + β_park · parking minutes + β_transfer · transfers + β_money · money
//!        + β_km · kilometres + β_mixed · kilometres by bike in mixed traffic
//!        + β_lane · kilometres by bike on a painted lane
//!        + β_separated · kilometres by bike on a track or path of its own
//!        + κ_mode )
//! ```
//!
//! A bike's ride is in-vehicle time (the bike is the traveller's vehicle), so a minute riding
//! costs `β_ivt`, as a minute driving; the kilometres ridden on each kind of facility (in mixed
//! traffic: a link without a cycle lane or track of its own; on a painted lane; on a track or
//! path of its own) cost `β_mixed`, `β_lane` and `β_separated` each **on top of** that time.
//! `κ_mode` is the **mode's constant**: what the class's travellers dislike in the chain's mode
//! beyond its measured attributes (`κ_car`, `κ_bike`, `κ_walk`, `κ_transit`; 0 by default: the
//! attributes decide). It is the same for every chain of one mode, so it never changes which route of a mode
//! is cheapest, only how the modes compare ([`Coefficients::chain_cost`]).
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

use openmobisim_core_demand::Mode;

use crate::chain::Chain;

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
    /// Metres ridden by bike on a painted lane (or a street where bikes have priority); 0 for
    /// every other leg.
    pub bike_lane_metres: f64,
    /// Metres ridden by bike on a track or path of its own; 0 for every other leg.
    pub bike_separated_metres: f64,
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
            bike_lane_metres: self.bike_lane_metres + other.bike_lane_metres,
            bike_separated_metres: self.bike_separated_metres + other.bike_separated_metres,
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
    /// Cost of one kilometre ridden by bike on a painted lane, on top of the ride's time and
    /// kilometres.
    pub bike_lane_per_kilometre: f64,
    /// Cost of one kilometre ridden by bike on a track or path of its own, on top of the ride's
    /// time and kilometres.
    pub bike_separated_per_kilometre: f64,
    /// The car's constant: what a car chain costs on top of its attributes.
    pub car_constant: f64,
    /// The bike's constant: what a bike chain costs on top of its attributes.
    pub bike_constant: f64,
    /// Walking's constant: what a walking chain costs on top of its attributes.
    pub walk_constant: f64,
    /// Transit's constant: what a transit chain costs on top of its attributes.
    pub transit_constant: f64,
}

impl Coefficients {
    /// The coefficients of a time-based run: one minute of door-to-door time (in a vehicle,
    /// walking, waiting or parking) costs 1; transfers, money, distance and the kilometres
    /// ridden on any bike facility and the modes' constants cost 0. A cost then reads in minutes
    /// of travel time.
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
            bike_lane_per_kilometre: 0.0,
            bike_separated_per_kilometre: 0.0,
            car_constant: 0.0,
            bike_constant: 0.0,
            walk_constant: 0.0,
            transit_constant: 0.0,
        }
    }

    /// The systematic utility of an alternative with `attributes`, in utility units (zero or
    /// negative for coefficients that are zero or positive), its mode's constant not counted
    /// (a chain's: [`Self::chain_utility`]).
    #[must_use]
    pub fn utility(&self, attributes: &Attributes) -> f64 {
        -self.cost(attributes)
    }

    /// The constant of `mode` (0 for a mode without one).
    #[must_use]
    pub fn constant(&self, mode: Mode) -> f64 {
        match mode {
            Mode::Car => self.car_constant,
            Mode::Bike => self.bike_constant,
            Mode::Walk => self.walk_constant,
            Mode::Transit => self.transit_constant,
            _ => 0.0,
        }
    }

    /// The cost of `chain`: the cost of its attributes plus its mode's constant (the mode of its
    /// legs; every chain the engine builds is of one mode). What the whole assignment compares:
    /// the choice, the found share and the duality gap read it. A constant of 0 adds nothing, to
    /// the bit.
    #[must_use]
    pub fn chain_cost(&self, chain: &Chain) -> f64 {
        let cost = self.cost(chain.total());
        let constant = self.constant(chain.mode());
        #[allow(clippy::float_cmp, reason = "a constant that is not given at all adds nothing")]
        if constant == 0.0 { cost } else { cost + constant }
    }

    /// The systematic utility of `chain`: minus its [cost](Self::chain_cost).
    #[must_use]
    pub fn chain_utility(&self, chain: &Chain) -> f64 {
        -self.chain_cost(chain)
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
            + self.bike_lane_per_kilometre * attributes.bike_lane_metres / 1000.0
            + self.bike_separated_per_kilometre * attributes.bike_separated_metres / 1000.0
    }
}

impl Default for Coefficients {
    /// [`Coefficients::time_based`].
    fn default() -> Self {
        Self::time_based()
    }
}
