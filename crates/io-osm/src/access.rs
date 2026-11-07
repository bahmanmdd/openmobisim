//! Who may use a way, and in which direction: car, bike and walk, by the OpenStreetMap wiki.
//!
//! # Access
//!
//! A mode starts from its class's default ([`crate::class`]) and the most specific access key
//! the way carries decides (wiki, `Key:access`): for a car `motorcar`, then `motor_vehicle`,
//! then `vehicle`, then `access`; for a bike `bicycle`, then `vehicle`, then `access`; for a
//! walker `foot`, then `access`. Two tags change a default before the keys are read: a
//! `motorroad=yes` closes the way to bikes and walkers (the wiki's row for a trunk road with
//! it), and a `highway=service` way that is a parking aisle, a driveway, a drive-through or an
//! emergency access (`service=…`) is not a street for cars (version 1's rule; bikes and walkers
//! keep their default; a car key's `yes` reopens it). A class with no car row in
//! [`crate::class`] (a cycleway, a footway, a track, …) is never a car street, whatever its
//! car keys say (Delft's cycleways tagged `motor_vehicle=destination` for mopeds stay bike and
//! walk). An `area=yes` way is a square, not a street, and is not read.
//!
//! The values: `yes`, `designated`, `permissive`, `destination`, `customers`, `delivery`,
//! `official` and `discouraged` let the mode through (the restriction to a destination is on
//! through traffic and is not modelled); `no`, `private`, `permit`, `use_sidepath`,
//! `agricultural`, `forestry` and `dismount` do not (a bike pushed is a walker); `unknown`
//! says nothing, and the next, less specific key decides. Any other value is an error naming
//! the way.
//!
//! # Direction
//!
//! A car follows `oneway` (`yes` or `1` along the way's node order, `-1` against it, `no` or
//! `0` both ways); without it, `junction=roundabout` or `circular` and the motorway classes
//! are one-way along the node order. `reversible` and `alternating` (a direction that changes
//! over the day) are read as both ways. A bike follows `oneway:bicycle` where tagged (same
//! values), else the car's rule. A walker goes both ways (the wiki: `oneway` binds vehicles)
//! unless `oneway:foot` says otherwise. Any other value of these keys is an error naming the
//! way.

use crate::class::Class;
use crate::source::{OsmError, OsmWay};

/// The directions a mode may use a way in, relative to its node order.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Directions {
    /// Along the node order.
    pub forward: bool,
    /// Against the node order.
    pub backward: bool,
}

impl Directions {
    /// Neither direction.
    pub const NONE: Directions = Directions { forward: false, backward: false };
    /// Both directions.
    pub const BOTH: Directions = Directions { forward: true, backward: true };
    /// Along the node order only.
    pub const FORWARD: Directions = Directions { forward: true, backward: false };
    /// Against the node order only.
    pub const BACKWARD: Directions = Directions { forward: false, backward: true };
}

/// Where each mode may go on one way.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Access {
    /// Cars.
    pub car: Directions,
    /// Bikes.
    pub bike: Directions,
    /// Walkers.
    pub walk: Directions,
}

/// The `service=…` values of a `highway=service` way that is not a street for cars.
const NOT_A_CAR_STREET: [&str; 4] =
    ["parking_aisle", "driveway", "drive-through", "emergency_access"];

/// Who may use `way` of `class`, and in which direction.
///
/// # Errors
///
/// [`OsmError::Tag`] naming the way, the key and the value, for an access or one-way value
/// the wiki does not define.
pub fn access(way: &OsmWay, class: &Class) -> Result<Access, OsmError> {
    let motorroad = way.tag("motorroad") == Some("yes");
    let service_only = way.tag("highway") == Some("service")
        && way.tag("service").is_some_and(|s| NOT_A_CAR_STREET.contains(&s));
    // A class with no car row is never a car street, whatever its tags (the reader has no
    // car speed or lanes for it): read the keys for their errors, then refuse.
    let car_keys = ["motorcar", "motor_vehicle", "vehicle", "access"];
    let car_ok = allowed(way, &car_keys, !service_only)? && class.car.is_some();
    let bike_ok = allowed(way, &["bicycle", "vehicle", "access"], class.bike && !motorroad)?;
    let walk_ok = allowed(way, &["foot", "access"], class.walk && !motorroad)?;

    let implied = class.one_way || matches!(way.tag("junction"), Some("roundabout" | "circular"));
    let vehicles = match one_way(way, "oneway")? {
        Some(d) => d,
        None if implied => Directions::FORWARD,
        None => Directions::BOTH,
    };
    let bikes = one_way(way, "oneway:bicycle")?.unwrap_or(vehicles);
    let walkers = one_way(way, "oneway:foot")?.unwrap_or(Directions::BOTH);
    let only = |ok: bool, d: Directions| if ok { d } else { Directions::NONE };
    Ok(Access {
        car: only(car_ok, vehicles),
        bike: only(bike_ok, bikes),
        walk: only(walk_ok, walkers),
    })
}

/// Whether a mode may use the way: the first of `keys` (most specific first) whose value says
/// yes or no decides; with none, `default`.
fn allowed(way: &OsmWay, keys: &[&str], default: bool) -> Result<bool, OsmError> {
    for &key in keys {
        let Some(value) = way.tag(key) else { continue };
        match value {
            "yes" | "designated" | "permissive" | "destination" | "customers" | "delivery"
            | "official" | "discouraged" => return Ok(true),
            "no" | "private" | "permit" | "use_sidepath" | "agricultural" | "forestry"
            | "dismount" => return Ok(false),
            "unknown" => {}
            _ => return Err(bad(way, key, value, "an access value the wiki defines")),
        }
    }
    Ok(default)
}

/// The directions a one-way key gives, if the way carries it.
fn one_way(way: &OsmWay, key: &str) -> Result<Option<Directions>, OsmError> {
    let Some(value) = way.tag(key) else { return Ok(None) };
    Ok(Some(match value {
        "yes" | "1" => Directions::FORWARD,
        "-1" => Directions::BACKWARD,
        "no" | "0" | "reversible" | "alternating" => Directions::BOTH,
        _ => return Err(bad(way, key, value, "yes, no, -1, reversible or alternating")),
    }))
}

/// The error for a value the reader cannot read.
pub(crate) fn bad(way: &OsmWay, key: &str, value: &str, expected: &'static str) -> OsmError {
    OsmError::Tag { way: way.id, key: key.to_owned(), value: value.to_owned(), expected }
}
