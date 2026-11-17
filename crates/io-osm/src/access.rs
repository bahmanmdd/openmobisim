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
//! The values (the full table, with where each rule comes from, is in the crate's README): a
//! value that admits lets the mode through; a value that refuses, or names a restricted group of
//! people (`private`, `permit`, `residents`, `employees`, …), does not (a restriction to a
//! destination is on through traffic and is not modelled; a bike pushed is a walker). A value
//! that names a mode or a group of vehicles (`foot`, `bicycle`, `motor_vehicle`, `bus`, …) means
//! "only these": it admits the modes the group holds and closes every other. `separate` on
//! `bicycle` or `foot` is read as `use_sidepath`. A `;` list is read part by part: the groups
//! named in it are one group (`foot;bicycle`: walkers and bikes); then the most restrictive known
//! part decides (any part that refuses refuses; else any part that admits admits). A value with
//! no known meaning, `unknown` included, says nothing: the next, less specific key decides, else
//! the class's default; it is noted (`tags_unread`) unless it is `unknown`, which says it knows
//! nothing. Car keys are not read on a class with no car row.
//!
//! # Direction
//!
//! A car follows `oneway` (`yes` or `1` along the way's node order, `-1` against it, `no` or
//! `0` both ways); without it, `junction=roundabout` or `circular` and the motorway classes
//! are one-way along the node order. `reversible` and `alternating` (a direction that changes
//! over the day) are read as both ways. A bike follows `oneway:bicycle` where tagged (same
//! values), else the car's rule. A walker goes both ways (the wiki: `oneway` binds vehicles)
//! unless `oneway:foot` says otherwise. Any other value of these keys is read as absent, noted
//! (`tags_unread`).

use openmobisim_types::mode::Mode;

use crate::class::Class;
use crate::source::OsmWay;

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

/// The values that let a mode through.
pub const ADMIT: [&str; 10] = [
    "yes",
    "designated",
    "permissive",
    "destination",
    "customers",
    "visitors",
    "delivery",
    "official",
    "discouraged",
    "optional_sidepath",
];

/// The values that close a way to a mode: a refusal, or a restricted group of people (read as
/// `private`).
pub const REFUSE: [&str; 18] = [
    "no",
    "private",
    "permit",
    "use_sidepath",
    "dismount",
    "emergency",
    "restricted",
    "residents",
    "employees",
    "staff",
    "members",
    "military",
    "police",
    "disabled",
    "school",
    "students",
    "agricultural",
    "forestry",
];

/// The values that name a mode or a group of vehicles, "only these", with the modes of this
/// reader each admits (a group none of whose vehicles is a car, a bike or a walker admits none).
pub const ONLY: [(&str, &[Mode]); 18] = [
    ("foot", &[Mode::Walk]),
    ("pedestrian", &[Mode::Walk]),
    ("bicycle", &[Mode::Bike]),
    ("motorcar", &[Mode::Car]),
    ("motor_vehicle", &[Mode::Car]),
    ("vehicle", &[Mode::Car, Mode::Bike]),
    ("motorcycle", &[]),
    ("moped", &[]),
    ("mofa", &[]),
    ("horse", &[]),
    ("bus", &[]),
    ("psv", &[]),
    ("taxi", &[]),
    ("hgv", &[]),
    ("coach", &[]),
    ("goods", &[]),
    ("hov", &[]),
    ("carpool", &[]),
];

/// What a way's tags said that the reader did not read: a value of no known meaning.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Unread(pub bool);

/// Who may use `way` of `class`, and in which direction; `unread` is set when a value of an
/// access or one-way key had no known meaning (read as absent).
pub fn access(way: &OsmWay, class: &Class, unread: &mut Unread) -> Access {
    let motorroad = way.tag("motorroad") == Some("yes");
    let service_only = way.tag("highway") == Some("service")
        && way.tag("service").is_some_and(|s| NOT_A_CAR_STREET.contains(&s));
    // A class with no car row is never a car street, whatever its tags (the reader has no
    // car speed or lanes for it): its car keys are not read.
    let car_keys = ["motorcar", "motor_vehicle", "vehicle", "access"];
    let car_ok = class.car.is_some() && allowed(way, Mode::Car, &car_keys, !service_only, unread);
    let bike_keys = ["bicycle", "vehicle", "access"];
    let bike_ok = allowed(way, Mode::Bike, &bike_keys, class.bike && !motorroad, unread);
    let walk_ok = allowed(way, Mode::Walk, &["foot", "access"], class.walk && !motorroad, unread);

    let implied = class.one_way || matches!(way.tag("junction"), Some("roundabout" | "circular"));
    let vehicles = match one_way(way, "oneway", unread) {
        Some(d) => d,
        None if implied => Directions::FORWARD,
        None => Directions::BOTH,
    };
    let bikes = one_way(way, "oneway:bicycle", unread).unwrap_or(vehicles);
    let walkers = one_way(way, "oneway:foot", unread).unwrap_or(Directions::BOTH);
    let only = |ok: bool, d: Directions| if ok { d } else { Directions::NONE };
    Access { car: only(car_ok, vehicles), bike: only(bike_ok, bikes), walk: only(walk_ok, walkers) }
}

/// What the value of `key` says for `mode`: `Some(true)` admits, `Some(false)` refuses, `None`
/// says nothing (the module's documentation; the README's table).
#[must_use]
pub fn says(key: &str, value: &str, mode: Mode) -> Option<bool> {
    let sidepath = matches!(key, "bicycle" | "foot");
    let parts: Vec<&str> = value
        .split(';')
        .map(str::trim)
        .map(|p| if p == "separate" && sidepath { "use_sidepath" } else { p })
        .collect();
    let groups: Vec<&[Mode]> =
        parts.iter().filter_map(|p| ONLY.iter().find(|g| g.0 == *p).map(|g| g.1)).collect();
    let only = (!groups.is_empty()).then(|| groups.iter().any(|g| g.contains(&mode)));
    if only == Some(false) || parts.iter().any(|p| REFUSE.contains(p)) {
        Some(false)
    } else if only == Some(true) || parts.iter().any(|p| ADMIT.contains(p)) {
        Some(true)
    } else {
        None
    }
}

/// Whether `mode` may use the way: the first of `keys` (most specific first) whose value says
/// yes or no decides; with none, `default`.
fn allowed(way: &OsmWay, mode: Mode, keys: &[&str], default: bool, unread: &mut Unread) -> bool {
    for &key in keys {
        let Some(value) = way.tag(key) else { continue };
        match says(key, value, mode) {
            Some(ok) => return ok,
            None if value == "unknown" => {}
            None => unread.0 = true,
        }
    }
    default
}

/// The directions a one-way key gives, if the way carries it with a value the reader reads.
fn one_way(way: &OsmWay, key: &str, unread: &mut Unread) -> Option<Directions> {
    match way.tag(key)? {
        "yes" | "1" => Some(Directions::FORWARD),
        "-1" => Some(Directions::BACKWARD),
        "no" | "0" | "reversible" | "alternating" => Some(Directions::BOTH),
        _ => {
            unread.0 = true;
            None
        }
    }
}
