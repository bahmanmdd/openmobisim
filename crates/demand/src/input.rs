//! The rows of a demand and what is refused in them: trip and class ids, classes, mode sets and
//! positions, checked before anything is linked.

use core::fmt;

use openmobisim_graph::geometry::{Position, Xy};
use openmobisim_graph::network::Network;
use openmobisim_types::ids::{ExternalIdTable, ExternalIdTableBuilder, UserClassId};
use openmobisim_types::mode::{Mode, ModeSet};
use openmobisim_types::time::Second;

/// One trip as given: one row of a trip table.
#[derive(Clone, Debug, PartialEq)]
pub struct TripRow {
    /// The trip's external id, which is also its traveller's.
    pub id: String,
    /// Where it starts: longitude/latitude, projected with the network's projection, on a
    /// network built from longitude/latitude; metres, used as given, on a network built in
    /// metres. Every position of a demand is of one kind, the network's.
    pub origin: Position,
    /// Where it ends, of the same kind.
    pub destination: Position,
    /// When it sets out, a second of the clock.
    pub departure: Second,
    /// The external id of its user class, one of the classes given.
    pub class: String,
    /// The modes open to it, at least one: a given mode is a set of one.
    pub modes: ModeSet,
}

/// Why a demand could not be built. Each names the trip, class or stop at fault by its
/// external id.
#[derive(Clone, Debug, PartialEq)]
pub enum DemandError {
    /// Two trips share an external id.
    DuplicateTrip {
        /// The external id.
        trip: String,
    },
    /// A class is given twice.
    DuplicateClass {
        /// The external id.
        class: String,
    },
    /// A trip names a class that is not given.
    UnknownClass {
        /// The trip.
        trip: String,
        /// The class it names.
        class: String,
    },
    /// A trip's mode set is empty.
    NoMode {
        /// The trip.
        trip: String,
    },
    /// A trip's position cannot be placed on the network: not of the network's kind
    /// (longitude/latitude on a network built in metres, or metres on a network built from
    /// longitude/latitude), longitude/latitude outside the projectable range, or a coordinate
    /// that is not a finite number.
    PositionNotTaken {
        /// The trip.
        trip: String,
    },
    /// A trip's position is not of the kind of the first trip's origin (in sorted id order).
    MixedKinds {
        /// The trip.
        trip: String,
    },
    /// A trip's mode set holds a mode no link of the network allows (for transit, walking,
    /// by which its ends are linked).
    NoNode {
        /// The trip.
        trip: String,
        /// The mode.
        mode: Mode,
    },
    /// No link of the network allows walking, so a stop cannot be linked.
    StopNoNode {
        /// The stop.
        stop: String,
    },
    /// The walk to a stop at most, `stop_walk_max`, is not a finite number of metres at or
    /// above zero.
    StopWalkMax {
        /// The value given.
        metres: f64,
    },
}

impl fmt::Display for DemandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateTrip { trip } => write!(f, "trip {trip} is given twice"),
            Self::DuplicateClass { class } => write!(f, "class {class} is given twice"),
            Self::UnknownClass { trip, class } => {
                write!(f, "trip {trip} names class {class}, which is not given")
            }
            Self::NoMode { trip } => write!(f, "trip {trip} has no mode"),
            Self::PositionNotTaken { trip } => write!(
                f,
                "trip {trip}: a position the network cannot take (not of the network's kind, \
                 longitude/latitude or metres; out of range; or not finite)"
            ),
            Self::MixedKinds { trip } => write!(
                f,
                "trip {trip}: a position of another kind than the first trip's (longitude/latitude \
                 and metres mixed)"
            ),
            Self::NoNode { trip, mode: Mode::Transit } => {
                write!(f, "trip {trip}: no link allows walking, by which transit's ends are linked")
            }
            Self::NoNode { trip, mode } => write!(f, "trip {trip}: no link allows {mode}"),
            Self::StopNoNode { stop } => {
                write!(f, "stop {stop}: no link allows walking, so it cannot be linked")
            }
            Self::StopWalkMax { metres } => {
                write!(f, "stop_walk_max is {metres} m: not a finite number at or above zero")
            }
        }
    }
}

impl core::error::Error for DemandError {}

/// The trips checked, in id order: their ids, the classes, and per trip its class, departure,
/// modes and two positions in metres.
#[derive(Clone, Debug)]
pub(crate) struct Trips {
    pub ids: ExternalIdTable,
    pub classes: ExternalIdTable,
    pub class: Vec<UserClassId>,
    pub departure: Vec<Second>,
    pub modes: Vec<ModeSet>,
    pub origin: Vec<Xy>,
    pub destination: Vec<Xy>,
}

/// The first id given twice, in sorted order.
fn twice<'a>(ids: impl Iterator<Item = &'a str>) -> Option<String> {
    let mut sorted: Vec<&str> = ids.collect();
    sorted.sort_unstable();
    sorted.windows(2).find(|w| w[0] == w[1]).map(|w| w[0].to_owned())
}

/// Check the rows and the classes and place every position in metres. The first fault, in
/// this order, is returned: a class given twice, a trip given twice; then trip by trip in id
/// order, an unknown class, an empty mode set, a position of another kind than the first
/// trip's origin, a position the network cannot take.
pub(crate) fn check(
    rows: &[TripRow],
    classes: &[String],
    network: &Network,
) -> Result<Trips, DemandError> {
    if let Some(class) = twice(classes.iter().map(String::as_str)) {
        return Err(DemandError::DuplicateClass { class });
    }
    if let Some(trip) = twice(rows.iter().map(|r| r.id.as_str())) {
        return Err(DemandError::DuplicateTrip { trip });
    }
    let table = |ids: Vec<&str>| {
        let mut b = ExternalIdTableBuilder::new();
        b.extend(ids);
        b.build()
    };
    let class_ids = table(classes.iter().map(String::as_str).collect());
    let mut order: Vec<&TripRow> = rows.iter().collect();
    order.sort_unstable_by(|a, b| a.id.cmp(&b.id));
    let kind = order.first().map(|r| is_lon_lat(r.origin));
    let n = order.len();
    let mut trips = Trips {
        ids: table(order.iter().map(|r| r.id.as_str()).collect()),
        classes: class_ids,
        class: Vec::with_capacity(n),
        departure: Vec::with_capacity(n),
        modes: Vec::with_capacity(n),
        origin: Vec::with_capacity(n),
        destination: Vec::with_capacity(n),
    };
    for row in order {
        let trip = || row.id.clone();
        let Some(class) = trips.classes.typed_id_of::<UserClassId>(&row.class) else {
            return Err(DemandError::UnknownClass { trip: trip(), class: row.class.clone() });
        };
        if row.modes.is_empty() {
            return Err(DemandError::NoMode { trip: trip() });
        }
        if [row.origin, row.destination].iter().any(|&p| Some(is_lon_lat(p)) != kind) {
            return Err(DemandError::MixedKinds { trip: trip() });
        }
        let (Some(origin), Some(destination)) =
            (place(network, row.origin), place(network, row.destination))
        else {
            return Err(DemandError::PositionNotTaken { trip: trip() });
        };
        trips.class.push(class);
        trips.departure.push(row.departure);
        trips.modes.push(row.modes);
        trips.origin.push(origin);
        trips.destination.push(destination);
    }
    Ok(trips)
}

/// Whether a position is longitude/latitude.
fn is_lon_lat(p: Position) -> bool {
    matches!(p, Position::LonLat(_))
}

/// A position in the network's plane: metres as given on a network built in metres,
/// longitude/latitude projected with the projection of a network built from them; `None` when
/// the network cannot take it (the other kind, out of range, not finite).
fn place(network: &Network, p: Position) -> Option<Xy> {
    match p {
        Position::Metres(xy) => {
            let taken = network.projection().is_none() && xy.x.is_finite() && xy.y.is_finite();
            taken.then_some(xy)
        }
        Position::LonLat(ll) => {
            let projection = network.projection()?;
            ll.is_projectable().then(|| projection.project(ll))
        }
    }
}
