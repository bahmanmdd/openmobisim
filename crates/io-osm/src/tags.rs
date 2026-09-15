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
//! (brief §3c) — the alternative is an importer that refuses a city because
//! sixteen ways out of two hundred thousand have a speed limit in a format we
//! had not seen.
//!
//! # What is deliberately not here
//!
//! **Implicit country speed limits.** `maxspeed=DE:urban` means 50 km/h to a
//! German, and the mapping is a table of a few hundred entries that changes
//! with the law. It is a v2 addition with its own data file; until then the
//! road class default applies and the way is recorded.
//!
//! **Turn restrictions.** They are OSM *relations*, not way tags, and they only
//! ever remove turns — so they can arrive later without changing anything built
//! now.

use openmobisim_core_graph::defaults::RoadClass;

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
    /// `access=no` or `access=private`.
    AccessDenied,
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
            Rejection::TooFewNodes => "too_few_nodes",
        }
    }
}

/// Decide whether a way is a road this importer models, and which class.
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
    if matches!(tag_of(tags, "access"), Some("no" | "private")) {
        return Err(Rejection::AccessDenied);
    }
    RoadClass::from_osm_highway(highway).ok_or(Rejection::UnknownHighwayClass)
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
/// `mph` or `knots`; `walk`; `none`. A semicolon-separated list takes its first
/// entry. Anything else — country codes such as `DE:urban`, free text — is
/// [`Maxspeed::Unparsed`].
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
/// assert_eq!(maxspeed(&tag("DE:urban")), Maxspeed::Unparsed);
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

/// The speed `maxspeed=walk` stands for, in km/h.
///
/// Living streets and shared spaces are tagged this way. Seven km/h is a brisk
/// walk; the value is here rather than inline so it can be cited and changed.
pub const WALKING_SPEED_KM_H: f64 = 7.0;

/// Whether a node interrupts a way, so that the way must be split there even if
/// no other road uses that node.
///
/// Signals and crossings matter because the control delay attaches to the
/// approach (S90); barriers matter because they change what can pass. Splitting
/// at them costs links, which is why the list is short and explicit rather than
/// "any tagged node".
#[must_use]
pub fn node_splits_way(tags: &[(String, String)]) -> bool {
    matches!(tag_of(tags, "highway"), Some("traffic_signals" | "mini_roundabout" | "stop"))
        || tag_of(tags, "barrier").is_some()
}

/// Whether a node is a signalised junction, which gives its approaches a
/// control delay and its turns a green-time fraction (S90).
#[must_use]
pub fn node_is_signalised(tags: &[(String, String)]) -> bool {
    tag_of(tags, "highway") == Some("traffic_signals")
}
