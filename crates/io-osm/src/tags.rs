//! Reading OSM tags — the part of importing that is actually hard.
//!
//! OpenStreetMap is a folksonomy. `maxspeed` is sometimes `50`, sometimes
//! `50 km/h`, sometimes `30 mph`, sometimes `DE:urban`, sometimes `none`, and
//! occasionally `50;30`. `lanes` may be a total, or split into
//! `lanes:forward`/`lanes:backward`, or absent. A way may be one-way because it
//! says so, because it is a roundabout, or because it is a motorway and the
//! convention is implicit.
//!
//! Every function here follows the same shape: **interpret what can be
//! interpreted, say clearly when it could not, and never fail.** A tag nobody
//! anticipated is a data-quality diagnostic and a fallback, not an error
//! — the alternative is an importer that refuses a city because
//! sixteen ways out of two hundred thousand have a speed limit in a format we
//! had not seen.
//!
//! # What is deliberately not here
//!
//! **Country-specific speed limits.** `maxspeed=DE:urban` means 50 km/h to a
//! German; the real mapping is a table of a few hundred entries that changes
//! with the law. [`maxspeed`] reads a small table of the usual implicit codes
//! with the country prefix ignored ([`IMPLICIT_SPEED_LIMITS_KM_H`], G-21);
//! a per-country data file is a later addition. Codes the table lacks fall to
//! the road class default and the way is recorded.
//!
//! **Turn restrictions.** They are OSM *relations*, not way tags, and they only
//! ever remove turns — so they can arrive later without changing anything built
//! now.

use openmobisim_core_graph::defaults::RoadClass;
use openmobisim_core_graph::layers::BikeInfrastructure;

use crate::source::tag_of;

/// Miles to kilometres.
pub const KM_PER_MILE: f64 = 1.609_344;

/// Which directions a way carries traffic in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Direction {
    /// Traffic runs in the order the way lists its nodes.
    Forward,
    /// Traffic runs against the order the way lists its nodes (`oneway=-1`).
    Backward,
    /// Traffic runs both ways; the importer creates two links per segment.
    Both,
}

impl Direction {
    /// Whether a link should be created in the way's own node order.
    #[inline]
    #[must_use]
    pub const fn has_forward(self) -> bool {
        matches!(self, Direction::Forward | Direction::Both)
    }

    /// Whether a link should be created against the way's node order.
    #[inline]
    #[must_use]
    pub const fn has_backward(self) -> bool {
        matches!(self, Direction::Backward | Direction::Both)
    }
}

/// What the importer concluded about a way's speed limit.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Maxspeed {
    /// A usable value, in km/h.
    Known(f64),
    /// No `maxspeed` tag at all. The road class default applies, silently —
    /// this is the normal case for most of the world's OSM data.
    Absent,
    /// A `maxspeed` tag that could not be interpreted. The road class default
    /// applies and the way is recorded as a data-quality diagnostic.
    Unparsed,
    /// An explicit `maxspeed=none` — a German autobahn without a limit.
    ///
    /// Treated as absent, so the motorway default applies, and recorded. A real
    /// free-flow speed for an unrestricted autobahn is a calibration question,
    /// not a tag-reading one.
    Unlimited,
}

impl Maxspeed {
    /// The value to hand the defaults table, or `None` to use the class row.
    #[inline]
    #[must_use]
    pub const fn value_km_h(self) -> Option<f64> {
        match self {
            Maxspeed::Known(v) => Some(v),
            _ => None,
        }
    }

    /// Whether this should be recorded as a data-quality diagnostic.
    #[inline]
    #[must_use]
    pub const fn is_noteworthy(self) -> bool {
        matches!(self, Maxspeed::Unparsed | Maxspeed::Unlimited)
    }
}

/// Lanes in each direction, as far as the tags say.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Lanes {
    /// Lanes in the way's own direction, if known.
    pub forward: Option<u8>,
    /// Lanes against the way's direction, if known.
    pub backward: Option<u8>,
}

/// Why a way was not imported as a road.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Rejection {
    /// No `highway` tag: it is a building, a boundary, a river.
    NotAHighway,
    /// A `highway` value the defaults table does not model.
    UnknownHighwayClass,
    /// `area=yes` — a pedestrian square, not a linear way.
    IsAnArea,
    /// `access=no` or `access=private`, or a car-class way closed to cars by
    /// `motorcar`, `motor_vehicle` or `vehicle` set to `no` or `private`.
    AccessDenied,
    /// A `highway=service` way that is a parking aisle, driveway,
    /// drive-through or emergency access (`service=…`, G-18): not part of the
    /// street network cars are routed on (it stays walkable, and cycleable
    /// where tagged).
    ServiceNotAStreet,
    /// Fewer than two nodes, so it has no geometry.
    TooFewNodes,
}

impl Rejection {
    /// The stable snake_case name, for diagnostics.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Rejection::NotAHighway => "not_a_highway",
            Rejection::UnknownHighwayClass => "unknown_highway_class",
            Rejection::IsAnArea => "is_an_area",
            Rejection::AccessDenied => "access_denied",
            Rejection::ServiceNotAStreet => "service_not_a_street",
            Rejection::TooFewNodes => "too_few_nodes",
        }
    }
}

/// The `service=…` values of a `highway=service` way that is not a street cars
/// are routed on: a parking aisle, a driveway, a drive-through lane, an
/// emergency access. `alley` and an untyped `service` stay roads.
const NON_STREET_SERVICES: [&str; 4] =
    ["parking_aisle", "driveway", "drive-through", "emergency_access"];

/// Whether `value` closes a way (`no`, `private`) when read as an access value.
fn closes(value: &str) -> bool {
    matches!(value, "no" | "private")
}

/// Decide whether a way is a road this importer models, and which class.
///
/// A way closed to cars is not a car link: `access=no|private`, or
/// the most specific of `motorcar`, `motor_vehicle`, `vehicle` that the way
/// carries being `no` or `private` (`motor_vehicle=yes` therefore overrides
/// `vehicle=no`). Such a way stays in the bike and walk layers by their own
/// rules, and is a [`RoadClass::Busway`] instead when buses may use it
/// (`bus` or `psv` `yes|designated`). `access=destination|delivery|customers`
/// stay drivable — a level-0 simplification: the restriction is on through
/// traffic and belongs to routing, not to the network. `highway=service` ways
/// that are parking aisles, driveways, drive-throughs or emergency accesses
/// (`service=…`) are refused with [`Rejection::ServiceNotAStreet`].
///
/// # Errors
///
/// Returns the [`Rejection`] reason rather than a class. Every rejection except
/// [`Rejection::NotAHighway`] is worth recording; that one is the overwhelming
/// majority of an extract and recording it would drown the report.
pub fn classify(tags: &[(String, String)], node_count: usize) -> Result<RoadClass, Rejection> {
    let Some(highway) = tag_of(tags, "highway") else {
        return Err(Rejection::NotAHighway);
    };
    if node_count < 2 {
        return Err(Rejection::TooFewNodes);
    }
    if tag_of(tags, "area") == Some("yes") {
        return Err(Rejection::IsAnArea);
    }
    let class = RoadClass::from_osm_highway(highway).ok_or(Rejection::UnknownHighwayClass);
    // A road closed to other motor traffic but open to buses is a busway (S199).
    let for_buses = |key: &str| matches!(tag_of(tags, key), Some("yes" | "designated"));
    let bus_only_road = |class: &Result<RoadClass, Rejection>| {
        (for_buses("bus") || for_buses("psv"))
            && class.as_ref().is_ok_and(|c| c.carries_motor_traffic())
    };
    if tag_of(tags, "access").is_some_and(closes) {
        return if bus_only_road(&class) {
            Ok(RoadClass::Busway)
        } else {
            Err(Rejection::AccessDenied)
        };
    }
    // Closed to cars by the most specific vehicle key (G-17).
    let closed_to_cars = ["motorcar", "motor_vehicle", "vehicle"]
        .into_iter()
        .find_map(|key| tag_of(tags, key))
        .is_some_and(closes);
    if closed_to_cars && class.as_ref().is_ok_and(|c| c.carries_motor_traffic()) {
        return if bus_only_road(&class) {
            Ok(RoadClass::Busway)
        } else {
            Err(Rejection::AccessDenied)
        };
    }
    if highway == "service"
        && tag_of(tags, "service").is_some_and(|v| NON_STREET_SERVICES.contains(&v))
    {
        return Err(Rejection::ServiceNotAStreet);
    }
    class
}

/// Decide which directions a way carries traffic in.
///
/// Precedence, highest first:
///
/// 1. An explicit `oneway` tag — including `oneway=no`, which overrides the
///    implicit rules below. Someone took the trouble to say so.
/// 2. `junction=roundabout` or `junction=circular`.
/// 3. `highway=motorway` or `motorway_link`, where one-way is the convention.
///
/// `oneway=reversible` and `oneway=alternating` describe a direction that
/// changes over the day. There is no v1 mechanism for that, so they are treated
/// as two-way — the permissive reading, which keeps the network connected.
#[must_use]
pub fn direction(tags: &[(String, String)], class: RoadClass) -> Direction {
    match tag_of(tags, "oneway") {
        Some("yes" | "true" | "1") => return Direction::Forward,
        Some("-1" | "reverse") => return Direction::Backward,
        Some("no" | "false" | "0") => return Direction::Both,
        Some(_) | None => {}
    }
    if matches!(tag_of(tags, "junction"), Some("roundabout" | "circular")) {
        return Direction::Forward;
    }
    if matches!(class, RoadClass::Motorway | RoadClass::MotorwayLink) {
        return Direction::Forward;
    }
    Direction::Both
}

/// Read the lane counts.
///
/// `lanes:forward` and `lanes:backward` win where present. Otherwise a `lanes`
/// total is interpreted according to `direction`: all of it one way on a
/// one-way street, split evenly on a two-way one.
///
/// **Odd totals on a two-way street round down, with a floor of one** — a
/// three-lane two-way street is usually two lanes one way and one the other,
/// and there is nothing in the tags to say which. Giving both directions the
/// floor understates capacity slightly, which is the safer error: overstating
/// capacity produces a network that never congests, and that failure is silent.
#[must_use]
pub fn lanes(tags: &[(String, String)], direction: Direction) -> Lanes {
    let explicit = |key: &str| tag_of(tags, key).and_then(|v| v.trim().parse::<u8>().ok());

    let forward_tag = explicit("lanes:forward");
    let backward_tag = explicit("lanes:backward");
    let total = explicit("lanes");

    let from_total = |want: bool| -> Option<u8> {
        let t = total?;
        if !want {
            return None;
        }
        Some(match direction {
            Direction::Both => (t / 2).max(1),
            _ => t.max(1),
        })
    };

    Lanes {
        forward: forward_tag.or_else(|| from_total(direction.has_forward())),
        backward: backward_tag.or_else(|| from_total(direction.has_backward())),
    }
}

/// Read a speed limit.
///
/// Accepted forms: a bare number (km/h); a number with `km/h`, `kph`, `kmh`,
/// `mph` or `knots`; `walk`; `none`; and an implicit country code
/// (`XX:urban`, `XX:rural`, `XX:motorway`, `XX:trunk`, `XX:living_street`,
/// `XX:zone30`, `XX:zone20`, with `XX` two capital letters), read from
/// [`IMPLICIT_SPEED_LIMITS_KM_H`] with the country ignored, except
/// `DE:motorway`, which has no limit ([`Maxspeed::Unlimited`]). A
/// semicolon-separated list takes its first entry. Anything else — other
/// codes, free text — is [`Maxspeed::Unparsed`].
///
/// # Examples
///
/// ```
/// use openmobisim_io_osm::tags::{maxspeed, Maxspeed};
///
/// let tag = |v: &str| vec![("maxspeed".to_owned(), v.to_owned())];
///
/// assert_eq!(maxspeed(&tag("50")), Maxspeed::Known(50.0));
/// assert_eq!(maxspeed(&tag("30 mph")), Maxspeed::Known(30.0 * 1.609_344));
/// assert_eq!(maxspeed(&tag("none")), Maxspeed::Unlimited);
/// assert_eq!(maxspeed(&tag("NL:urban")), Maxspeed::Known(50.0));
/// assert_eq!(maxspeed(&tag("FR:rural")), Maxspeed::Known(80.0));
/// assert_eq!(maxspeed(&tag("DE:motorway")), Maxspeed::Unlimited);
/// assert_eq!(maxspeed(&tag("XX:signals")), Maxspeed::Unparsed);
/// assert_eq!(maxspeed(&[]), Maxspeed::Absent);
/// ```
#[must_use]
pub fn maxspeed(tags: &[(String, String)]) -> Maxspeed {
    let Some(raw) = tag_of(tags, "maxspeed") else {
        return Maxspeed::Absent;
    };
    // `50;30` — a limit that changes along the way. Take the first, which is
    // the one that applies from the way's start.
    let value = raw.split(';').next().unwrap_or(raw).trim();

    match value.to_ascii_lowercase().as_str() {
        "" => return Maxspeed::Absent,
        "none" | "unlimited" => return Maxspeed::Unlimited,
        "walk" | "walking" => return Maxspeed::Known(WALKING_SPEED_KM_H),
        _ => {}
    }
    if let Some(implicit) = implicit_limit(value) {
        return implicit;
    }

    // Split a leading number from a trailing unit.
    let digits_end =
        value.find(|c: char| !c.is_ascii_digit() && c != '.' && c != ',').unwrap_or(value.len());
    let (number, unit) = value.split_at(digits_end);
    let Ok(n) = number.replace(',', ".").parse::<f64>() else {
        return Maxspeed::Unparsed;
    };
    if !n.is_finite() || n <= 0.0 {
        return Maxspeed::Unparsed;
    }

    match unit.trim().to_ascii_lowercase().as_str() {
        "" | "km/h" | "kmh" | "kph" => Maxspeed::Known(n),
        "mph" => Maxspeed::Known(n * KM_PER_MILE),
        "knots" => Maxspeed::Known(n * 1.852),
        _ => Maxspeed::Unparsed,
    }
}

/// The speed limits that an implicit `maxspeed=XX:<kind>` code stands for, in
/// km/h, by `<kind>`. The country prefix `XX` is ignored, except that
/// `DE:motorway` has no limit and is read as [`Maxspeed::Unlimited`].
///
/// *Uncalibrated: the usual limit of the countries that use these codes (NL, FR,
/// BE, DE, …), one value for all of them. Source: the OpenStreetMap wiki,
/// *Default speed limits*, <https://wiki.openstreetmap.org/wiki/Default_speed_limits>.
/// Where a country differs (Germany's rural roads 100 km/h, the Netherlands'
/// motorways 100 km/h by day, Belgium's regional rural limits) the table gives
/// the common value; set the way's `maxspeed` to a number, or calibrate, to
/// change it.*
pub const IMPLICIT_SPEED_LIMITS_KM_H: [(&str, f64); 7] = [
    ("urban", 50.0),
    ("rural", 80.0),
    ("motorway", 130.0),
    ("trunk", 100.0),
    ("living_street", 15.0),
    ("zone30", 30.0),
    ("zone20", 20.0),
];

/// The limit an implicit code `XX:<kind>` stands for, if `value` is one.
fn implicit_limit(value: &str) -> Option<Maxspeed> {
    let (country, kind) = value.split_once(':')?;
    if country.len() != 2 || !country.bytes().all(|b| b.is_ascii_uppercase()) {
        return None;
    }
    let kind = kind.to_ascii_lowercase();
    if country == "DE" && kind == "motorway" {
        return Some(Maxspeed::Unlimited);
    }
    IMPLICIT_SPEED_LIMITS_KM_H.iter().find(|(k, _)| *k == kind).map(|&(_, v)| Maxspeed::Known(v))
}

/// The speed `maxspeed=walk` stands for, in km/h.
///
/// Living streets and shared spaces are tagged this way. Seven km/h is a brisk
/// walk; the value is here rather than inline so it can be cited and changed.
pub const WALKING_SPEED_KM_H: f64 = 7.0;

/// Whether a node interrupts a way, so that the way must be split there even if
/// no other road uses that node.
///
/// Signals and crossings matter because the control delay attaches to the
/// approach; barriers matter because they change what can pass. Splitting
/// at them costs links, which is why the list is short and explicit rather than
/// "any tagged node".
#[must_use]
pub fn node_splits_way(tags: &[(String, String)]) -> bool {
    matches!(tag_of(tags, "highway"), Some("traffic_signals" | "mini_roundabout" | "stop"))
        || tag_of(tags, "barrier").is_some()
}

/// Whether a node is a signalised junction, which gives its approaches a
/// control delay and its turns a green-time fraction.
///
/// Not an emergency signal (`traffic_signals=emergency`, at a fire station's exit: green for
/// traffic but when a vehicle turns out).
#[must_use]
pub fn node_is_signalised(tags: &[(String, String)]) -> bool {
    node_signal(tags).is_some()
}

/// Which approaches a signal node governs.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SignalDirection {
    /// Every approach (no `traffic_signals:direction`, or `both`): a junction's signal.
    Both,
    /// Traffic along the way's node order only: one direction's stop line.
    Forward,
    /// Traffic against it only.
    Backward,
}

/// The signal a node carries, if any, and the approaches it governs.
///
/// Mappers often put a signal on each approach's stop line rather than on the junction, with
/// `traffic_signals:direction=forward|backward` saying which direction it stops: a car passing
/// the junction meets its own stop line and then, on a two-way street, the other direction's.
/// That one stops only the traffic going its way; read without the tag, it gave every car two
/// signals per junction (43 % of Paris's signal nodes carry it, 56 % of Amsterdam's, 61 % of
/// Lyon's).
#[must_use]
pub fn node_signal(tags: &[(String, String)]) -> Option<SignalDirection> {
    if tag_of(tags, "highway") != Some("traffic_signals")
        || tag_of(tags, "traffic_signals") == Some("emergency")
    {
        return None;
    }
    Some(match tag_of(tags, "traffic_signals:direction") {
        Some("forward") => SignalDirection::Forward,
        Some("backward") => SignalDirection::Backward,
        _ => SignalDirection::Both,
    })
}

/// What a way offers a bike, in each direction it may be ridden.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct BikeWay {
    /// The way's road class (`bridleway` is read as a footway).
    pub class: RoadClass,
    /// The infrastructure in the way's own node order, if bikes may go that way.
    pub forward: Option<BikeInfrastructure>,
    /// The infrastructure against the way's node order, if bikes may go that way.
    pub backward: Option<BikeInfrastructure>,
    /// The bike must be walked here (`bicycle=dismount`, steps).
    pub dismount: bool,
}

/// Whether an access value lets the mode through.
fn permits(value: Option<&str>) -> bool {
    matches!(value, Some("yes" | "designated" | "permissive"))
}

/// The class a way has for the bike and walk layers: the road class, and a
/// bridleway read as a footway.
fn layer_class(highway: &str) -> Option<RoadClass> {
    if highway == "bridleway" {
        return Some(RoadClass::Footway);
    }
    RoadClass::from_osm_highway(highway)
}

/// The dedicated infrastructure a `cycleway…` value names, if any.
fn infrastructure_of(value: Option<&str>) -> Option<BikeInfrastructure> {
    match value {
        Some("track" | "opposite_track") => Some(BikeInfrastructure::Separated),
        Some("lane" | "opposite_lane") => Some(BikeInfrastructure::Lane),
        _ => None,
    }
}

/// Whether a `cycleway…` value is an `opposite…` one, which names a facility
/// against the street's direction.
fn is_opposite(value: &str) -> bool {
    value.starts_with("opposite")
}

/// The direction a `cycleway:<side>:oneway` tag says a lane or track runs, in
/// the way's node order (`yes` forward, `-1` backward).
fn lane_runs(tags: &[(String, String)], key: &str) -> Option<Direction> {
    match tag_of(tags, key) {
        Some("yes" | "true" | "1") => Some(Direction::Forward),
        Some("-1" | "reverse") => Some(Direction::Backward),
        _ => None,
    }
}

/// Whether the tags open a one-way street's contraflow to bikes by naming a
/// facility, not by `oneway:bicycle`: `opposite…` in `cycleway` or
/// `cycleway:both|left|right`, or a lane or track on a side whose `:oneway`
/// runs against the street. Always false on a two-way street.
fn contraflow_opened(tags: &[(String, String)], road_direction: Direction) -> bool {
    let against = match road_direction {
        Direction::Forward => Direction::Backward,
        Direction::Backward => Direction::Forward,
        Direction::Both => return false,
    };
    let opposite = ["cycleway", "cycleway:both", "cycleway:left", "cycleway:right"]
        .into_iter()
        .any(|key| tag_of(tags, key).is_some_and(is_opposite));
    let lane_against =
        [("cycleway:left", "cycleway:left:oneway"), ("cycleway:right", "cycleway:right:oneway")]
            .into_iter()
            .any(|(side, oneway)| {
                infrastructure_of(tag_of(tags, side)).is_some()
                    && lane_runs(tags, oneway) == Some(against)
            });
    opposite || lane_against
}

/// The facility a one-way street offers bikes with the flow and against it,
/// as `(forward, backward)` in the way's node order.
///
/// Right-hand traffic: the right side serves the forward direction and the left
/// the backward one. With the contraflow closed, a lane or track on either side
/// serves the one direction (right first). With it open, the contraflow side's
/// lane or track (`opposite…` values, a lane running against the street, or
/// the left side of a `oneway=yes` street, the right of a `oneway=-1` one)
/// serves the contraflow and the other side the with-flow direction.
fn one_way_infrastructure(
    tags: &[(String, String)],
    road_direction: Direction,
    bike_direction: Direction,
) -> (Option<BikeInfrastructure>, Option<BikeInfrastructure>) {
    let forward_road = road_direction == Direction::Forward;
    let against = if forward_road { Direction::Backward } else { Direction::Forward };
    let open =
        if forward_road { bike_direction.has_backward() } else { bike_direction.has_forward() };

    let plain = |key: &str| infrastructure_of(tag_of(tags, key).filter(|v| !is_opposite(v)));
    let both = plain("cycleway:both").or_else(|| plain("cycleway"));
    let right = plain("cycleway:right").or(both);
    let left = plain("cycleway:left").or(both);
    let (with, contra) = if open {
        // The contraflow side.
        let (with_side, contra_side) = if forward_road { (right, left) } else { (left, right) };
        // A facility named `opposite…`, or a lane that runs against the street.
        let named = ["cycleway", "cycleway:both", "cycleway:left", "cycleway:right"]
            .into_iter()
            .find_map(|key| infrastructure_of(tag_of(tags, key).filter(|v| is_opposite(v))));
        let running_against = [
            ("cycleway:left", "cycleway:left:oneway"),
            ("cycleway:right", "cycleway:right:oneway"),
        ]
        .into_iter()
        .find_map(|(side, oneway)| {
            (lane_runs(tags, oneway) == Some(against))
                .then(|| infrastructure_of(tag_of(tags, side)))
                .flatten()
        });
        (with_side, named.or(running_against).or(contra_side))
    } else {
        (right.or(left), None)
    };
    if forward_road { (with, contra) } else { (contra, with) }
}

/// Decide whether a way is a bike link, and what it offers in each direction.
///
/// - **Never:** motorways, `motorroad=yes`, `bicycle=no`, `bicycle=use_sidepath`
///   (a parallel cycleway is mapped and used instead), areas, and ways with
///   `access=no|private` unless `bicycle=yes|designated|permissive` lets bikes
///   through (a bike-only road).
/// - **Trunk roads and their links** only with `bicycle=yes|designated|permissive`
///   (G-19; [`RoadClass::carries_cyclists`] says the class does not carry bikes
///   by default, the tag overrides it).
/// - **Every other car road**, in mixed traffic, unless a
///   `cycleway`, `cycleway:both|right|left` tag names a lane or a track, or the
///   street is a cycle street (`cyclestreet=yes`, `bicycle_road=yes`).
/// - **`highway=cycleway`**, and a footway, path or bridleway with
///   `bicycle=designated`: separated.
/// - **Footways, paths, tracks, steps and pedestrian streets** only with
///   `bicycle=yes|permissive|designated|dismount`; shared with pedestrians
///   (mixed) unless designated.
/// - **Walking the bike:** `bicycle=dismount` and steps.
/// - **Ferries** (`route=ferry`): bikes ride on unless `bicycle=no`.
///
/// **Direction:** the car rules ([`direction`]), except that `oneway:bicycle`
/// overrides them and the contraflow is opened by `oneway:bicycle=no`, by
/// `opposite*` in `cycleway` or `cycleway:both|left|right`, or by a lane or
/// track whose `cycleway:left:oneway` / `cycleway:right:oneway` runs against
/// the street. **Sides:** the right side serves the way's own direction
/// and the left the reverse — right-hand traffic is assumed, a known
/// simplification for left-hand-traffic countries. On a one-way street whose
/// contraflow is closed, a lane or track on either side serves the one
/// direction; where the contraflow is open, the left lane or track (the right
/// one on a `oneway=-1` street) serves the contraflow, the other side the
/// with-flow direction, and the contraflow is mixed where no lane or track
/// is tagged (`opposite_lane|opposite_track` name theirs).
#[must_use]
pub fn bike_way(tags: &[(String, String)], node_count: usize) -> Option<BikeWay> {
    if is_ferry(tags) {
        // A ferry crossing (S197): bikes ride on unless the tags say not.
        if node_count < 2 || tag_of(tags, "bicycle") == Some("no") {
            return None;
        }
        let direction = direction(tags, RoadClass::Ferry);
        let level = Some(BikeInfrastructure::Mixed);
        return Some(BikeWay {
            class: RoadClass::Ferry,
            forward: level.filter(|_| direction.has_forward()),
            backward: level.filter(|_| direction.has_backward()),
            dismount: false,
        });
    }
    let highway = tag_of(tags, "highway")?;
    if node_count < 2 || tag_of(tags, "area") == Some("yes") {
        return None;
    }
    let bicycle = tag_of(tags, "bicycle");
    if matches!(bicycle, Some("no" | "use_sidepath")) || tag_of(tags, "motorroad") == Some("yes") {
        return None;
    }
    if matches!(tag_of(tags, "access"), Some("no" | "private")) && !permits(bicycle) {
        return None;
    }
    let class = layer_class(highway)?;
    // A busway (S199) carries bikes only where its tags say so; so does a
    // trunk road (G-19): the class says no, the tag overrides it.
    if (class == RoadClass::Busway
        || (matches!(class, RoadClass::Trunk | RoadClass::TrunkLink) && !class.carries_cyclists()))
        && !permits(bicycle)
    {
        return None;
    }
    let dismount = bicycle == Some("dismount") || highway == "steps";

    let road_direction = direction(tags, class);
    let cycleway = tag_of(tags, "cycleway");
    let contraflow = contraflow_opened(tags, road_direction);
    let bike_direction = match tag_of(tags, "oneway:bicycle") {
        Some("no" | "false" | "0") => Direction::Both,
        Some("yes" | "true" | "1") if road_direction == Direction::Backward => Direction::Backward,
        Some("yes" | "true" | "1") => Direction::Forward,
        Some("-1" | "reverse") => Direction::Backward,
        _ if contraflow => Direction::Both,
        _ => road_direction,
    };

    let (forward, backward) = match class {
        RoadClass::Motorway | RoadClass::MotorwayLink => return None,
        RoadClass::Cycleway => (BikeInfrastructure::Separated, BikeInfrastructure::Separated),
        RoadClass::Footway | RoadClass::Pedestrian => {
            let level = match bicycle {
                Some("designated") => BikeInfrastructure::Separated,
                Some("yes" | "permissive" | "dismount") => BikeInfrastructure::Mixed,
                _ => return None,
            };
            (level, level)
        }
        _ => {
            let base = if tag_of(tags, "cyclestreet") == Some("yes")
                || tag_of(tags, "bicycle_road") == Some("yes")
            {
                BikeInfrastructure::Lane
            } else {
                BikeInfrastructure::Mixed
            };
            let (f, b) = if road_direction == Direction::Both {
                let both = infrastructure_of(tag_of(tags, "cycleway:both"))
                    .or_else(|| infrastructure_of(cycleway.filter(|v| !v.starts_with("opposite"))));
                let right = infrastructure_of(tag_of(tags, "cycleway:right")).or(both);
                let left = infrastructure_of(tag_of(tags, "cycleway:left")).or(both);
                (right, left)
            } else {
                one_way_infrastructure(tags, road_direction, bike_direction)
            };
            (base.max(f.unwrap_or(base)), base.max(b.unwrap_or(base)))
        }
    };
    Some(BikeWay {
        class,
        forward: bike_direction.has_forward().then_some(forward),
        backward: bike_direction.has_backward().then_some(backward),
        dismount,
    })
}

/// Decide whether a way is a walk link, and its class. Every walk link
/// is walked in both directions.
///
/// - **Never:** motorways, `motorroad=yes`, `foot=no`, `foot=use_sidepath`,
///   areas, and ways with `access=no|private` unless
///   `foot=yes|designated|permissive`.
/// - **Cycleways** too, unless `foot=no`: where no footway runs beside it, a
///   pedestrian may use the cycle path (the Netherlands' RVV 1990, art. 4), and the
///   mappers of such countries rarely tag `foot`; read as closed to walkers, Amsterdam's
///   walk network fell apart into islands (Schiphol's station among them, 11 % of its
///   walk nodes outside the largest part).
/// - **Busways** only with `foot=yes|designated|permissive`.
/// - **Ferries** (`route=ferry`) unless `foot=no`.
/// - **Everything else**, steps included.
#[must_use]
pub fn walk_way(tags: &[(String, String)], node_count: usize) -> Option<RoadClass> {
    if is_ferry(tags) {
        return (node_count >= 2 && tag_of(tags, "foot") != Some("no")).then_some(RoadClass::Ferry);
    }
    let highway = tag_of(tags, "highway")?;
    if node_count < 2 || tag_of(tags, "area") == Some("yes") {
        return None;
    }
    let foot = tag_of(tags, "foot");
    if matches!(foot, Some("no" | "use_sidepath")) || tag_of(tags, "motorroad") == Some("yes") {
        return None;
    }
    if matches!(tag_of(tags, "access"), Some("no" | "private")) && !permits(foot) {
        return None;
    }
    let class = layer_class(highway)?;
    match class {
        RoadClass::Motorway | RoadClass::MotorwayLink => None,
        RoadClass::Busway => permits(foot).then_some(class),
        _ => Some(class),
    }
}

/// Whether a way is a ferry crossing (`route=ferry` and no `highway`), which
/// the bike and walk layers carry and the road network does not.
#[must_use]
pub fn is_ferry(tags: &[(String, String)]) -> bool {
    tag_of(tags, "route") == Some("ferry") && tag_of(tags, "highway").is_none()
}

/// A ferry's crossing time from its `duration` tag, in seconds.
///
/// Accepted forms: `HH:MM`, `HH:MM:SS`, a bare number of minutes, and ISO 8601
/// `PT…H…M…S`. Anything else is `None`, and the default ferry speed applies.
///
/// # Examples
///
/// ```
/// use openmobisim_io_osm::tags::duration_seconds;
///
/// let tag = |v: &str| vec![("duration".to_owned(), v.to_owned())];
///
/// assert_eq!(duration_seconds(&tag("00:05")), Some(300.0));
/// assert_eq!(duration_seconds(&tag("1:02:30")), Some(3750.0));
/// assert_eq!(duration_seconds(&tag("12")), Some(720.0));
/// assert_eq!(duration_seconds(&tag("PT1H30M")), Some(5400.0));
/// assert_eq!(duration_seconds(&tag("soon")), None);
/// ```
#[must_use]
pub fn duration_seconds(tags: &[(String, String)]) -> Option<f64> {
    let raw = tag_of(tags, "duration")?.trim();
    let positive = |x: f64| (x.is_finite() && x > 0.0).then_some(x);
    if let Some(iso) = raw.strip_prefix("PT") {
        let (mut total, mut number) = (0.0, String::new());
        for c in iso.chars() {
            match c {
                '0'..='9' | '.' => number.push(c),
                'H' | 'M' | 'S' => {
                    let n: f64 = number.parse().ok()?;
                    total += n * match c {
                        'H' => 3600.0,
                        'M' => 60.0,
                        _ => 1.0,
                    };
                    number.clear();
                }
                _ => return None,
            }
        }
        return if number.is_empty() { positive(total) } else { None };
    }
    let parts: Vec<&str> = raw.split(':').collect();
    let numbers: Option<Vec<f64>> = parts.iter().map(|p| p.trim().parse::<f64>().ok()).collect();
    match numbers?.as_slice() {
        [minutes] => positive(minutes * 60.0),
        [hours, minutes] => positive(hours * 3600.0 + minutes * 60.0),
        [hours, minutes, seconds] => positive(hours * 3600.0 + minutes * 60.0 + seconds),
        _ => None,
    }
}
