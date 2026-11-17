//! The rows of a demand, checked before anything is linked: what cannot be used is set right or
//! left out and noted in the demand's [`Repairs`] by kind (the key, then what it completes "N …"
//! with); only what no repair exists for is refused ([`DemandError`]).
//!
//! | Key | Rows | What is done |
//! |---|---|---|
//! | `classes_repeated` | a class given twice | read once |
//! | `trips_repeated` | a trip id given twice | the first row given kept, the others dropped |
//! | `trips_of_unknown_class` | a trip naming a class not given | dropped |
//! | `trips_unplaced` | a trip with a coordinate not finite, or a longitude/latitude not projectable (beyond ±180°, ±84°) | dropped |
//! | `trips_without_car_network`, `…_bike_…`, `…_walk_…`, `…_transit_…` | a trip whose set holds a mode no link of the network allows (for transit, walking, by which its ends are linked) | the mode taken out of its set |
//! | `trips_without_mode` | a trip with no mode, given so or left so by the row above | dropped |
//!
//! A trip dropped is not in the demand: it has no id, and no output names it.

use core::fmt;

use openmobisim_graph::geometry::{Position, Xy};
use openmobisim_graph::network::Network;
use openmobisim_types::ids::{ExternalIdTable, ExternalIdTableBuilder, UserClassId};
use openmobisim_types::mode::{Mode, ModeSet};
use openmobisim_types::repairs::Repairs;
use openmobisim_types::time::Second;

use crate::index::{Joins, NodeIndex};

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

/// Why a demand could not be built: what no repair exists for. Each names the trip at fault by
/// its external id.
#[derive(Clone, Debug, PartialEq)]
pub enum DemandError {
    /// A trip's position is not of the network's kind: longitude/latitude on a network built in
    /// metres, or metres on a network built from longitude/latitude (no projection converts
    /// them).
    PositionNotTaken {
        /// The trip.
        trip: String,
    },
    /// A trip's position is not of the kind of the first trip's origin (in sorted id order).
    MixedKinds {
        /// The trip.
        trip: String,
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
            Self::PositionNotTaken { trip } => write!(
                f,
                "trip {trip}: a position of another kind than the network's (longitude/latitude \
                 on a network in metres, or metres on a network from longitude/latitude)"
            ),
            Self::MixedKinds { trip } => write!(
                f,
                "trip {trip}: a position of another kind than the first trip's (longitude/latitude \
                 and metres mixed)"
            ),
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

/// The key under which a trip losing `mode` (no link allows it) is noted.
fn without_network(mode: Mode) -> &'static str {
    match mode {
        Mode::Car => "trips_without_car_network",
        Mode::Bike => "trips_without_bike_network",
        Mode::Walk => "trips_without_walk_network",
        Mode::Transit => "trips_without_transit_network",
    }
}

/// The modes some link of `network` allows; transit when walking is (its ends are walked to).
fn modes_on(network: &Network) -> ModeSet {
    let mut on = ModeSet::EMPTY;
    for mode in [Mode::Car, Mode::Bike, Mode::Walk] {
        if !NodeIndex::new(network, mode, Joins::Either).is_empty() {
            on = on.with(mode);
        }
    }
    if on.contains(Mode::Walk) { on.with(Mode::Transit) } else { on }
}

/// Check the rows and the classes and place every position in metres, each row used, set right
/// or dropped and noted in `repairs` (the module's table). Rows are sorted by trip id, stably,
/// so that of a repeated id the first given is kept.
///
/// # Errors
///
/// Trip by trip in id order, the first position of another kind than the first trip's origin
/// ([`DemandError::MixedKinds`]), or of another kind than the network's
/// ([`DemandError::PositionNotTaken`]).
pub(crate) fn check(
    rows: &[TripRow],
    classes: &[String],
    network: &Network,
    repairs: &mut Repairs,
) -> Result<Trips, DemandError> {
    let mut class_list: Vec<&str> = classes.iter().map(String::as_str).collect();
    class_list.sort_unstable();
    for w in class_list.windows(2).filter(|w| w[0] == w[1]) {
        repairs.note("classes_repeated", "classes given twice: read once", w[0]);
    }
    class_list.dedup();
    let table = |ids: &[&str]| {
        let mut b = ExternalIdTableBuilder::new();
        b.extend(ids.iter().copied());
        b.build()
    };
    let class_ids = table(&class_list);
    let mut order: Vec<&TripRow> = rows.iter().collect();
    order.sort_by(|a, b| a.id.cmp(&b.id));
    let kind = order.first().map(|r| is_lon_lat(r.origin));
    let on = modes_on(network);
    let mut trips = Trips {
        ids: ExternalIdTable::default(),
        classes: class_ids,
        class: Vec::new(),
        departure: Vec::new(),
        modes: Vec::new(),
        origin: Vec::new(),
        destination: Vec::new(),
    };
    let mut kept: Vec<&str> = Vec::with_capacity(order.len());
    for row in order {
        let id = row.id.as_str();
        if kept.last() == Some(&id) {
            let what = "trip ids given twice: the first row given kept, the others dropped";
            repairs.note("trips_repeated", what, id);
            continue;
        }
        if [row.origin, row.destination].iter().any(|&p| Some(is_lon_lat(p)) != kind) {
            return Err(DemandError::MixedKinds { trip: id.to_owned() });
        }
        if is_lon_lat(row.origin) != network.projection().is_some() {
            return Err(DemandError::PositionNotTaken { trip: id.to_owned() });
        }
        let Some(class) = trips.classes.typed_id_of::<UserClassId>(&row.class) else {
            repairs.note("trips_of_unknown_class", "trips naming a class not given: dropped", id);
            continue;
        };
        let (Some(origin), Some(destination)) =
            (place(network, row.origin), place(network, row.destination))
        else {
            let what = "trips with a position not finite or not projectable: dropped";
            repairs.note("trips_unplaced", what, id);
            continue;
        };
        let mut modes = row.modes;
        for mode in row.modes.iter().filter(|&m| !on.contains(m)) {
            let what = "trips whose set holds a mode no link allows: the mode taken out";
            repairs.note(without_network(mode), what, id);
            modes = modes.iter().filter(|&m| m != mode).fold(ModeSet::EMPTY, ModeSet::with);
        }
        if modes.is_empty() {
            repairs.note("trips_without_mode", "trips with no mode (given, or left): dropped", id);
            continue;
        }
        kept.push(id);
        trips.class.push(class);
        trips.departure.push(row.departure);
        trips.modes.push(modes);
        trips.origin.push(origin);
        trips.destination.push(destination);
    }
    trips.ids = table(&kept);
    Ok(trips)
}

/// Whether a position is longitude/latitude.
fn is_lon_lat(p: Position) -> bool {
    matches!(p, Position::LonLat(_))
}

/// A position in the network's plane, of the network's kind (checked before): metres as given,
/// longitude/latitude projected with the network's projection; `None` when a coordinate is not
/// finite or a longitude/latitude not projectable.
fn place(network: &Network, p: Position) -> Option<Xy> {
    match p {
        Position::Metres(xy) => (xy.x.is_finite() && xy.y.is_finite()).then_some(xy),
        Position::LonLat(ll) => {
            let projection = network.projection()?;
            ll.is_projectable().then(|| projection.project(ll))
        }
    }
}
