//! Reading tags: a way's road class and its car defaults, who may use a way and in which
//! direction (a car, a bike, a walker: version 1's rules), a car link's lanes and free-flow
//! speed, and the nodes that split a way. A value is read, defaulted or noted; nothing here
//! refuses. Units: km/h, lanes per direction, vehicles per hour per lane.

use crate::source::tag_of;

/// A way's or a node's tags, key and value.
pub type Tags = [(String, String)];

/// Kilometres in a mile (the international mile, exactly).
pub const KM_PER_MILE: f64 = 1.609_344;

/// The speed `maxspeed=walk` stands for, km/h (version 1's: a brisk walk).
pub const WALKING_SPEED_KM_H: f64 = 7.0;

/// The speed an implicit `maxspeed=XX:<kind>` code stands for, km/h, by `<kind>`, the country
/// `XX` ignored (but `DE:motorway`, which has no limit): the usual limit of the countries that
/// use the codes (OpenStreetMap wiki, "Default speed limits"); version 1's table.
pub const IMPLICIT_SPEED_LIMITS_KM_H: [(&str, f64); 7] = [
    ("urban", 50.0),
    ("rural", 80.0),
    ("motorway", 130.0),
    ("trunk", 100.0),
    ("living_street", 15.0),
    ("zone30", 30.0),
    ("zone20", 20.0),
];

/// Which roads cars drive: every road of a car class (the reader as it has been), or the main
/// roads alone (experimental: a lighter car network).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum CarRoads {
    /// Every class the class table gives cars.
    #[default]
    All,
    /// Motorways down to unclassified roads: no residential street, living street or service
    /// road ([`RoadClass::is_minor`]), but a roundabout tagged as one that main roads meet.
    Main,
}

/// The road classes: the `highway` values the reader knows, and a ferry crossing (`route=ferry`),
/// in the order of the class table (a class before another is the larger road).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
#[allow(missing_docs, reason = "each variant is the `highway` value its name gives")]
pub enum RoadClass {
    Motorway,
    MotorwayLink,
    Trunk,
    TrunkLink,
    Primary,
    PrimaryLink,
    Secondary,
    SecondaryLink,
    Tertiary,
    TertiaryLink,
    Unclassified,
    Residential,
    LivingStreet,
    Service,
    Pedestrian,
    Footway,
    Cycleway,
    Ferry,
    Busway,
}

/// A car class's values where the tags give none: version 1's speeds and lanes, and capacities
/// per lane with the class's junctions counted in.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct CarDefaults {
    /// Free-flow speed, km/h.
    pub speed_km_h: f64,
    /// Lanes in each direction.
    pub lanes: u8,
    /// The flow one lane passes at the link's downstream end, veh/h.
    pub capacity_per_lane: f64,
}

const fn car(speed_km_h: f64, lanes: u8, capacity_per_lane: f64) -> Option<CarDefaults> {
    Some(CarDefaults { speed_km_h, lanes, capacity_per_lane })
}

/// The saturation flow of a signalised approach lane, veh/h (Cerema 2016, "Évaluation de la
/// capacité aux feux"): a lane's capacity at a signal is this × its green share (green time /
/// cycle).
pub const SATURATION_FLOW_PER_LANE: f64 = 1_800.0;

/// The capacity per lane of a class with at-grade junctions: the saturation flow × the class's
/// green share (this project's assumption, by class; the owner's to approve).
const fn at_signals(green_share: f64) -> f64 {
    SATURATION_FLOW_PER_LANE * green_share
}

impl RoadClass {
    /// Every class, in the order of the class table.
    pub const ALL: [RoadClass; 19] = [
        Self::Motorway,
        Self::MotorwayLink,
        Self::Trunk,
        Self::TrunkLink,
        Self::Primary,
        Self::PrimaryLink,
        Self::Secondary,
        Self::SecondaryLink,
        Self::Tertiary,
        Self::TertiaryLink,
        Self::Unclassified,
        Self::Residential,
        Self::LivingStreet,
        Self::Service,
        Self::Pedestrian,
        Self::Footway,
        Self::Cycleway,
        Self::Ferry,
        Self::Busway,
    ];

    /// The class of a `highway` value, if the reader knows it: `road` is `unclassified`;
    /// `steps`, `path`, `track` and `corridor` are footways; `bus_guideway` is a busway.
    #[must_use]
    pub fn from_highway(value: &str) -> Option<Self> {
        Some(match value {
            "motorway" => Self::Motorway,
            "motorway_link" => Self::MotorwayLink,
            "trunk" => Self::Trunk,
            "trunk_link" => Self::TrunkLink,
            "primary" => Self::Primary,
            "primary_link" => Self::PrimaryLink,
            "secondary" => Self::Secondary,
            "secondary_link" => Self::SecondaryLink,
            "tertiary" => Self::Tertiary,
            "tertiary_link" => Self::TertiaryLink,
            "unclassified" | "road" => Self::Unclassified,
            "residential" => Self::Residential,
            "living_street" => Self::LivingStreet,
            "service" => Self::Service,
            "pedestrian" => Self::Pedestrian,
            "footway" | "steps" | "path" | "track" | "corridor" => Self::Footway,
            "cycleway" => Self::Cycleway,
            "busway" | "bus_guideway" => Self::Busway,
            _ => return None,
        })
    }

    /// The class's name, the road class a link's drawing carries.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Motorway => "motorway",
            Self::MotorwayLink => "motorway_link",
            Self::Trunk => "trunk",
            Self::TrunkLink => "trunk_link",
            Self::Primary => "primary",
            Self::PrimaryLink => "primary_link",
            Self::Secondary => "secondary",
            Self::SecondaryLink => "secondary_link",
            Self::Tertiary => "tertiary",
            Self::TertiaryLink => "tertiary_link",
            Self::Unclassified => "unclassified",
            Self::Residential => "residential",
            Self::LivingStreet => "living_street",
            Self::Service => "service",
            Self::Pedestrian => "pedestrian",
            Self::Footway => "footway",
            Self::Cycleway => "cycleway",
            Self::Ferry => "ferry",
            Self::Busway => "busway",
        }
    }

    /// Whether the class is a minor street, one cars do not drive under [`CarRoads::Main`].
    #[must_use]
    pub fn is_minor(self) -> bool {
        matches!(self, Self::Residential | Self::LivingStreet | Self::Service)
    }

    /// The car's defaults, `None` for a class cars do not use (the README's table). Speeds and
    /// lanes: version 1's, "common OSM speed conventions". Capacity per lane, junctions counted
    /// in (the reader models none): without at-grade junctions, a motorway's 2 000 veh/h
    /// (ICTAVRU), a trunk carriageway's 1 800 and a ramp's 1 550 (Cerema, "Fondamentaux de la
    /// conception routière"); with them, [`SATURATION_FLOW_PER_LANE`] × the class's green share.
    #[must_use]
    pub const fn car(self) -> Option<CarDefaults> {
        match self {
            Self::Motorway => car(110.0, 2, 2_000.0),
            Self::MotorwayLink => car(60.0, 1, 1_550.0),
            Self::Trunk => car(90.0, 2, 1_800.0),
            Self::TrunkLink => car(50.0, 1, 1_550.0),
            Self::Primary => car(60.0, 2, at_signals(0.60)),
            Self::PrimaryLink | Self::SecondaryLink => car(40.0, 1, at_signals(0.50)),
            Self::Secondary => car(50.0, 1, at_signals(0.50)),
            Self::Tertiary => car(50.0, 1, at_signals(0.45)),
            Self::TertiaryLink => car(30.0, 1, at_signals(0.40)),
            Self::Unclassified => car(40.0, 1, at_signals(0.40)),
            Self::Residential => car(30.0, 1, at_signals(1.0 / 3.0)),
            Self::LivingStreet => car(20.0, 1, at_signals(0.25)),
            Self::Service => car(20.0, 1, at_signals(0.25)),
            Self::Pedestrian | Self::Footway | Self::Cycleway | Self::Ferry | Self::Busway => None,
        }
    }
}

/// The directions a way is used in, relative to its node order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Direction {
    /// Along the node order.
    Forward,
    /// Against it.
    Backward,
    /// Both.
    Both,
}

impl Direction {
    /// Used along the node order.
    #[must_use]
    pub const fn has_forward(self) -> bool {
        matches!(self, Self::Forward | Self::Both)
    }

    /// Used against the node order.
    #[must_use]
    pub const fn has_backward(self) -> bool {
        matches!(self, Self::Backward | Self::Both)
    }
}

/// The direction vehicles use a way of `class` in: `oneway` (`yes`, `true`, `1`; `-1`,
/// `reverse`; `no`, `false`, `0`), else one-way along the node order on `junction=roundabout`
/// or `circular` and on motorways and their slip roads, else both ways. Any other `oneway`
/// value (`reversible`, `alternating`, …) is read as absent.
#[must_use]
pub fn direction(tags: &Tags, class: RoadClass) -> Direction {
    match tag_of(tags, "oneway") {
        Some("yes" | "true" | "1") => return Direction::Forward,
        Some("-1" | "reverse") => return Direction::Backward,
        Some("no" | "false" | "0") => return Direction::Both,
        _ => {}
    }
    if matches!(tag_of(tags, "junction"), Some("roundabout" | "circular"))
        || matches!(class, RoadClass::Motorway | RoadClass::MotorwayLink)
    {
        return Direction::Forward;
    }
    Direction::Both
}

/// Whether the way's `oneway` value has no known meaning (`reversible` and `alternating`, a
/// direction that changes over the day, are known: read as no `oneway`).
#[must_use]
pub fn oneway_unread(tags: &Tags) -> bool {
    let known = ["yes", "true", "1", "-1", "reverse", "no", "false", "0"];
    tag_of(tags, "oneway")
        .is_some_and(|v| !known.contains(&v) && !matches!(v, "reversible" | "alternating"))
}

/// Whether an access value closes a way: `no` or `private`.
fn closes(value: &str) -> bool {
    matches!(value, "no" | "private")
}

/// Whether an access value lets a mode through: `yes`, `designated` or `permissive`.
fn permits(value: Option<&str>) -> bool {
    matches!(value, Some("yes" | "designated" | "permissive"))
}

/// The `service` values of a `highway=service` way that is no street for cars.
const NOT_A_CAR_STREET: [&str; 4] =
    ["parking_aisle", "driveway", "drive-through", "emergency_access"];

/// The class of a way that is a road (version 1's `classify`): a `highway` value of the class
/// table, at least two nodes, not `area=yes`, not closed by `access=no|private`; of a car class,
/// not closed to cars by the most specific of `motorcar`, `motor_vehicle`, `vehicle` that it
/// carries (`no`, `private`); of `highway=service`, not a parking aisle, driveway,
/// drive-through or emergency access. A road of a car class is a car way; every road counts in
/// the car links' junctions.
#[must_use]
pub fn road_class(tags: &Tags, node_count: usize) -> Option<RoadClass> {
    let highway = tag_of(tags, "highway")?;
    let class = RoadClass::from_highway(highway)?;
    let closed_to_cars = closed_to_cars(tags);
    let not_a_street = highway == "service"
        && tag_of(tags, "service").is_some_and(|v| NOT_A_CAR_STREET.contains(&v));
    let refused = node_count < 2
        || tag_of(tags, "area") == Some("yes")
        || tag_of(tags, "access").is_some_and(closes)
        || (closed_to_cars && class.car().is_some())
        || not_a_street;
    (!refused).then_some(class)
}

/// Whether the most specific of `motorcar`, `motor_vehicle`, `vehicle` the way carries closes it
/// to cars (`no`, `private`).
fn closed_to_cars(tags: &Tags) -> bool {
    ["motorcar", "motor_vehicle", "vehicle"]
        .into_iter()
        .find_map(|key| tag_of(tags, key))
        .is_some_and(closes)
}

/// Whether the way is a roundabout (`junction=roundabout` or `circular`) of a minor street
/// class ([`RoadClass::is_minor`]) that is a road and not closed to cars: under
/// [`CarRoads::Main`], where main roads meet it, cars drive it as the largest of them
/// (`crate::import`), so that a roundabout tagged below its roads does not cut them.
#[must_use]
pub fn minor_roundabout(tags: &Tags, node_count: usize) -> bool {
    matches!(tag_of(tags, "junction"), Some("roundabout" | "circular"))
        && road_class(tags, node_count).is_some_and(RoadClass::is_minor)
        && !closed_to_cars(tags)
}

/// A ferry crossing: `route=ferry` and no `highway`.
#[must_use]
pub fn is_ferry(tags: &Tags) -> bool {
    tag_of(tags, "route") == Some("ferry") && tag_of(tags, "highway").is_none()
}

/// The class a way has for bikes and walkers: its road class, a bridleway a footway.
fn layer_class(highway: &str) -> Option<RoadClass> {
    if highway == "bridleway" { Some(RoadClass::Footway) } else { RoadClass::from_highway(highway) }
}

/// The class and direction of a way bikes may ride (version 1's `bike_way`), else `None`.
///
/// - Never: motorways and their slip roads, `bicycle=no|use_sidepath`, `motorroad=yes`, an
///   area, a way of fewer than two nodes, `access=no|private` without
///   `bicycle=yes|designated|permissive`.
/// - Trunk roads, their slip roads and busways only with `bicycle=yes|designated|permissive`;
///   footways (paths, tracks, steps, bridleways among them) and pedestrian streets only with
///   `bicycle=yes|designated|permissive|dismount`; a ferry unless `bicycle=no`; every other
///   class (cycleways, car streets).
/// - Direction: `oneway:bicycle` (`no`: both ways; `yes`: the street's one way, or along the
///   node order; `-1`: against it), else both ways on a one-way street whose contraflow a
///   cycleway tag opens (an `opposite…` value of `cycleway`, `cycleway:both|left|right`, or a
///   lane, track or shared bus lane on a side whose `cycleway:<side>:oneway` runs against the
///   street), else the street's ([`direction`]).
#[must_use]
pub fn bike_way(tags: &Tags, node_count: usize) -> Option<(RoadClass, Direction)> {
    if is_ferry(tags) {
        let ok = node_count >= 2 && tag_of(tags, "bicycle") != Some("no");
        return ok.then(|| (RoadClass::Ferry, direction(tags, RoadClass::Ferry)));
    }
    let highway = tag_of(tags, "highway")?;
    let bicycle = tag_of(tags, "bicycle");
    if node_count < 2
        || tag_of(tags, "area") == Some("yes")
        || matches!(bicycle, Some("no" | "use_sidepath"))
        || tag_of(tags, "motorroad") == Some("yes")
        || (tag_of(tags, "access").is_some_and(closes) && !permits(bicycle))
    {
        return None;
    }
    let class = layer_class(highway)?;
    let admitted = match class {
        RoadClass::Motorway | RoadClass::MotorwayLink => false,
        RoadClass::Trunk | RoadClass::TrunkLink | RoadClass::Busway => permits(bicycle),
        RoadClass::Footway | RoadClass::Pedestrian => {
            matches!(bicycle, Some("yes" | "designated" | "permissive" | "dismount"))
        }
        _ => true,
    };
    if !admitted {
        return None;
    }
    let road = direction(tags, class);
    let bike = match tag_of(tags, "oneway:bicycle") {
        Some("no" | "false" | "0") => Direction::Both,
        Some("yes" | "true" | "1") if road == Direction::Backward => Direction::Backward,
        Some("yes" | "true" | "1") => Direction::Forward,
        Some("-1" | "reverse") => Direction::Backward,
        _ if contraflow_opened(tags, road) => Direction::Both,
        _ => road,
    };
    Some((class, bike))
}

/// Whether a cycleway tag opens a one-way street to bikes against its traffic (right-hand
/// traffic: the right side serves the way's own direction).
fn contraflow_opened(tags: &Tags, road: Direction) -> bool {
    let against = match road {
        Direction::Forward => Direction::Backward,
        Direction::Backward => Direction::Forward,
        Direction::Both => return false,
    };
    let opposite = ["cycleway", "cycleway:both", "cycleway:left", "cycleway:right"]
        .into_iter()
        .any(|key| tag_of(tags, key).is_some_and(|v| v.starts_with("opposite")));
    let facility = |v: Option<&str>| {
        let names = ["track", "opposite_track", "lane", "opposite_lane", "share_busway"];
        v.is_some_and(|v| names.contains(&v) || v == "opposite_share_busway")
    };
    let runs = |key: &str| match tag_of(tags, key) {
        Some("yes" | "true" | "1") => Some(Direction::Forward),
        Some("-1" | "reverse") => Some(Direction::Backward),
        _ => None,
    };
    let lane_against = ["left", "right"].into_iter().any(|side| {
        facility(tag_of(tags, &format!("cycleway:{side}")))
            && runs(&format!("cycleway:{side}:oneway")) == Some(against)
    });
    opposite || lane_against
}

/// The class and direction of a way walkers may walk (version 1's `walk_way`), else `None`:
/// never motorways and their slip roads, `foot=no|use_sidepath`, `motorroad=yes`, an area, a way
/// of fewer than two nodes, `access=no|private` without `foot=yes|designated|permissive`; a
/// busway only with `foot=yes|designated|permissive`; a ferry unless `foot=no`, in its own
/// direction; every other class (cycleways and steps among them) both ways.
#[must_use]
pub fn walk_way(tags: &Tags, node_count: usize) -> Option<(RoadClass, Direction)> {
    if is_ferry(tags) {
        let ok = node_count >= 2 && tag_of(tags, "foot") != Some("no");
        return ok.then(|| (RoadClass::Ferry, direction(tags, RoadClass::Ferry)));
    }
    let highway = tag_of(tags, "highway")?;
    let foot = tag_of(tags, "foot");
    if node_count < 2
        || tag_of(tags, "area") == Some("yes")
        || matches!(foot, Some("no" | "use_sidepath"))
        || tag_of(tags, "motorroad") == Some("yes")
        || (tag_of(tags, "access").is_some_and(closes) && !permits(foot))
    {
        return None;
    }
    match layer_class(highway)? {
        RoadClass::Motorway | RoadClass::MotorwayLink => None,
        RoadClass::Busway if !permits(foot) => None,
        class => Some((class, Direction::Both)),
    }
}

/// A car link's lanes in its own direction, free-flow speed and capacity.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Car {
    /// Lanes, at least 1.
    pub lanes: u8,
    /// Free-flow speed, km/h.
    pub speed_km_h: f64,
    /// Capacity of the downstream end, veh/h: lanes × the class's capacity per lane.
    pub capacity_veh_h: f64,
}

/// What the reader set right on one way's car tags.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CarNotes {
    /// A lane count that is not a whole number from 0 to 255, or a `oneway` of no known
    /// meaning: read as absent.
    pub unread: bool,
    /// A speed tag that is not a speed: read as absent.
    pub speed: bool,
    /// Lane tags that contradict, or leave a driven direction no lane: repaired.
    pub lanes: bool,
}

/// The car fields along and against the node order of a way cars drive `road`, from its tags
/// and its class's `defaults` (`None` for a direction cars do not drive); what was set right
/// in `notes`.
///
/// Lanes (wiki, `Key:lanes`): one-way, `lanes:forward` (or `:backward`) else `lanes`; two-way, a
/// tagged side is its own; the total and one side: the other is total − given −
/// `lanes:both_ways`; the total alone: (total − `lanes:both_ways`) / 2 rounded down, at least 1;
/// no tag: the class's lanes. Repaired (`lanes`): a total not the sum of the sides tagged (the
/// sides kept), a total and one side leaving the other none (1), `lanes:both_ways` at or above
/// the total, `lanes=0` included (1 each way), a driven direction given 0 lanes (1).
///
/// Speed (wiki, `Key:maxspeed`): `maxspeed:forward` (or `:backward`), then `maxspeed`, then the
/// class's speed; a value [`maxspeed`] cannot read is noted and the next key taken.
#[must_use]
pub fn car_fields(
    tags: &Tags,
    road: Direction,
    defaults: CarDefaults,
    notes: &mut CarNotes,
) -> (Option<Car>, Option<Car>) {
    notes.unread |= oneway_unread(tags);
    let (forward_lanes, backward_lanes) = lanes(tags, road, defaults.lanes, notes);
    let mut field = |driven: bool, lanes: u8, key: &str| {
        driven.then(|| {
            let speed_km_h = [key, "maxspeed"]
                .into_iter()
                .filter_map(|k| tag_of(tags, k))
                .find_map(|value| {
                    let read = maxspeed(value);
                    notes.speed |= read.is_none();
                    read
                })
                .unwrap_or(defaults.speed_km_h);
            let capacity_veh_h = f64::from(lanes) * defaults.capacity_per_lane;
            Car { lanes, speed_km_h, capacity_veh_h }
        })
    };
    let forward = field(road.has_forward(), forward_lanes, "maxspeed:forward");
    (forward, field(road.has_backward(), backward_lanes, "maxspeed:backward"))
}

/// The lanes along and against the node order (a direction cars do not drive gets 0).
fn lanes(tags: &Tags, road: Direction, default: u8, notes: &mut CarNotes) -> (u8, u8) {
    let mut count = |key: &str| {
        let value = tag_of(tags, key)?;
        let read = value.trim().parse::<u8>().ok();
        notes.unread |= read.is_none();
        read
    };
    let (total, forward, backward) =
        (count("lanes"), count("lanes:forward"), count("lanes:backward"));
    let middle = count("lanes:both_ways").unwrap_or(0);
    let mut repaired = |n: u8| {
        notes.lanes = true;
        n
    };
    let (f, b) = match road {
        Direction::Forward => (forward.or(total).unwrap_or(default), 0),
        Direction::Backward => (0, backward.or(total).unwrap_or(default)),
        Direction::Both => match (total, forward, backward) {
            (Some(t), Some(f), Some(b))
                if u16::from(f) + u16::from(b) + u16::from(middle) != u16::from(t) =>
            {
                (repaired(f), b)
            }
            (_, Some(f), Some(b)) => (f, b),
            (Some(t), Some(f), None) => (f, rest(t, f, middle).unwrap_or_else(|| repaired(1))),
            (Some(t), None, Some(b)) => (rest(t, b, middle).unwrap_or_else(|| repaired(1)), b),
            (Some(t), None, None) => match t.checked_sub(middle) {
                Some(0) | None => (repaired(1), 1),
                Some(r) => ((r / 2).max(1), (r / 2).max(1)),
            },
            (None, f, b) => (f.unwrap_or(default), b.unwrap_or(default)),
        },
    };
    let mut at_least_one = |driven: bool, n: u8| if driven && n == 0 { repaired(1) } else { n };
    (at_least_one(road.has_forward(), f), at_least_one(road.has_backward(), b))
}

/// The other side's lanes: total − given − both_ways, if at least one.
fn rest(total: u8, given: u8, middle: u8) -> Option<u8> {
    total.checked_sub(given).and_then(|r| r.checked_sub(middle)).filter(|&r| r > 0)
}

/// A speed limit in km/h, if `value` is one (version 1's reading): a number, alone or with
/// `km/h`, `kmh`, `kph`, `mph` or `knots` (a comma read as a decimal point); `walk`
/// ([`WALKING_SPEED_KM_H`]); an implicit code `XX:<kind>` ([`IMPLICIT_SPEED_LIMITS_KM_H`]); of a
/// `;` list, the first. `None`, a no-limit `DE:motorway`, and anything else: not a speed.
#[must_use]
pub fn maxspeed(value: &str) -> Option<f64> {
    let value = value.split(';').next().unwrap_or(value).trim();
    if matches!(value.to_ascii_lowercase().as_str(), "walk" | "walking") {
        return Some(WALKING_SPEED_KM_H);
    }
    if let Some((country, kind)) = value.split_once(':') {
        let kind = kind.to_ascii_lowercase();
        let is_code = country.len() == 2 && country.bytes().all(|b| b.is_ascii_uppercase());
        if !is_code || (country == "DE" && kind == "motorway") {
            return None;
        }
        return IMPLICIT_SPEED_LIMITS_KM_H.iter().find(|(k, _)| *k == kind).map(|&(_, v)| v);
    }
    let digits =
        value.find(|c: char| !c.is_ascii_digit() && c != '.' && c != ',').unwrap_or(value.len());
    let (number, unit) = value.split_at(digits);
    let n = number.replace(',', ".").parse::<f64>().ok().filter(|n| n.is_finite() && *n > 0.0)?;
    match unit.trim().to_ascii_lowercase().as_str() {
        "" | "km/h" | "kmh" | "kph" => Some(n),
        "mph" => Some(n * KM_PER_MILE),
        "knots" => Some(n * 1.852),
        _ => None,
    }
}

/// Whether a node splits every way through it, as a junction does: a signal, a mini
/// roundabout, a stop sign (`highway=traffic_signals|mini_roundabout|stop`) or a barrier.
#[must_use]
pub fn splits(tags: &Tags) -> bool {
    matches!(tag_of(tags, "highway"), Some("traffic_signals" | "mini_roundabout" | "stop"))
        || tag_of(tags, "barrier").is_some()
}

/// Whether a node is signalised: `highway=traffic_signals`, but for an emergency signal
/// (`traffic_signals=emergency`, green but when a fire engine turns out).
#[must_use]
pub fn signalised(tags: &Tags) -> bool {
    tag_of(tags, "highway") == Some("traffic_signals")
        && tag_of(tags, "traffic_signals") != Some("emergency")
}
