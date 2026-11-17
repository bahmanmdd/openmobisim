//! A trip's way: the legs it took (`routes.parquet`) read as the page draws it, its identity (the mode and
//! the exact links, or the rides as pattern, boarding stop and alighting stop) and its legs, each
//! with its seconds.

use openmobisim_io_parquet::RouteLeg;
use openmobisim_transit::raptor::{Footpath, Ride};
use openmobisim_transit::timetable::Timetable;
use openmobisim_types::ids::{EntityId, LinkId, StopId, TransitRunId};
use openmobisim_types::mode::Mode;

use crate::MapError;
use crate::kept::Trip;

/// A leg driven, on the network.
pub(crate) const DRIVE: u8 = 0;
/// A leg ridden by bike, on the network.
pub(crate) const BIKE: u8 = 1;
/// A leg walked on the network (a walk alternative's route).
pub(crate) const WALK: u8 = 2;
/// A ride on a transit run, from stop to stop.
pub(crate) const RIDE: u8 = 3;
/// A walk off the network: between a door and the network or a stop, or between two stops.
pub(crate) const CONNECT: u8 = 4;
/// A wait at a stop.
pub(crate) const WAIT: u8 = 5;

/// Where a leg is drawn.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Geometry<'a> {
    /// Not drawn: a walk between a door and the network or a stop, a wait.
    None,
    /// Along links, in order.
    Links(&'a [LinkId]),
    /// Through stops, in order: a ride.
    Stops(&'a [StopId]),
    /// Straight from one stop to another: a walk between two rides.
    Between(StopId, StopId),
}

/// One leg: its kind ([`DRIVE`] to [`WAIT`]), where it is drawn and its seconds.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Leg<'a> {
    pub kind: u8,
    pub geometry: Geometry<'a>,
    pub seconds: f64,
    /// The run ridden, for a ride.
    pub run: Option<TransitRunId>,
}

/// A trip's way: its mode, its identity and its legs.
#[derive(Clone, Debug)]
pub(crate) struct Way<'a> {
    pub mode: Mode,
    /// The identity: a route's links, or per ride its pattern, boarding stop and alighting stop.
    pub key: Vec<u32>,
    pub legs: Vec<Leg<'a>>,
}

/// The way of `trip`, none without an alternative or when it is left out (no legs read).
///
/// # Errors
///
/// [`MapError::NoTimetable`] for a journey without a timetable; [`MapError::NotTheRunsTransit`]
/// for a ride the timetable does not have.
pub(crate) fn of<'a>(
    trip: &'a Trip,
    timetable: Option<&'a Timetable>,
) -> Result<Option<Way<'a>>, MapError> {
    let (Some(mode), Some(legs)) = (trip.mode, &trip.legs) else { return Ok(None) };
    if mode == Mode::Transit {
        return transit(trip, legs, timetable.ok_or(MapError::NoTimetable)?).map(Some);
    }
    let walked: f64 =
        legs.iter().map(|l| if let RouteLeg::WalkGap(d) = l { d.get() } else { 0.0 }).sum();
    let mut way = Way { mode, key: Vec::new(), legs: Vec::new() };
    for leg in legs {
        match leg {
            RouteLeg::WalkGap(d) => way.legs.push(connect(Geometry::None, d.get())),
            RouteLeg::Route { links, .. } => {
                way.key = links.iter().map(|l| l.raw()).collect();
                let kind = match mode {
                    Mode::Car => DRIVE,
                    Mode::Bike => BIKE,
                    _ => WALK,
                };
                let seconds = trip.travel_time - walked;
                way.legs.push(Leg { kind, geometry: Geometry::Links(links), seconds, run: None });
            }
            RouteLeg::Ride(_) | RouteLeg::TransferWalk(_) => {
                return Err(MapError::NotTheRunsTransit);
            }
        }
    }
    Ok(Some(way))
}

/// A walk off the network.
fn connect(geometry: Geometry<'_>, seconds: f64) -> Leg<'_> {
    Leg { kind: CONNECT, geometry, seconds, run: None }
}

/// A journey's way: the walk to its first stop, then per ride the walk from the last ride's stop
/// (none before the first: a footpath where it changes stop, else 0 s), the wait and the ride,
/// then the walk to the door. The first wait runs from the end of the walk to the stop to the
/// first departure, each later one from the end of the walk between rides to the next departure,
/// so the legs add up to the journey's time.
fn transit<'a>(
    trip: &'a Trip,
    legs: &'a [RouteLeg],
    timetable: &'a Timetable,
) -> Result<Way<'a>, MapError> {
    let (Some(RouteLeg::WalkGap(access)), Some(RouteLeg::WalkGap(egress))) =
        (legs.first(), legs.last())
    else {
        return Err(MapError::NotTheRunsTransit);
    };
    let mut way = Way { mode: Mode::Transit, key: Vec::new(), legs: Vec::new() };
    way.legs.push(connect(Geometry::None, access.get()));
    let mut at = f64::from(trip.departure.get()) + access.get();
    let (mut last, mut footpath): (Option<&Ride>, Option<Footpath>) = (None, None);
    for leg in &legs[1..legs.len() - 1] {
        let ride = match leg {
            RouteLeg::TransferWalk(f) => {
                footpath = Some(*f);
                continue;
            }
            RouteLeg::Ride(ride) => ride,
            _ => return Err(MapError::NotTheRunsTransit),
        };
        if let Some(before) = last {
            let walk = match footpath.take() {
                Some(f) => connect(Geometry::Between(before.alight.0, f.to), f.seconds.get()),
                None => connect(Geometry::None, 0.0),
            };
            at = f64::from(before.alight.1.get()) + walk.seconds;
            way.legs.push(walk);
        }
        let (board, alight) = (f64::from(ride.board.1.get()), f64::from(ride.alight.1.get()));
        way.legs.push(Leg { kind: WAIT, geometry: Geometry::None, seconds: board - at, run: None });
        let stops = ride_stops(timetable, ride)?;
        let geometry = Geometry::Stops(stops);
        way.legs.push(Leg { kind: RIDE, geometry, seconds: alight - board, run: Some(ride.run) });
        way.key.extend([timetable.run_pattern(ride.run), ride.board.0.raw(), ride.alight.0.raw()]);
        last = Some(ride);
    }
    if last.is_none() {
        return Err(MapError::NotTheRunsTransit);
    }
    way.legs.push(connect(Geometry::None, egress.get()));
    Ok(way)
}

/// The stops a ride passes, from the one boarded at to the one left at: the run's calls from
/// the one at the boarding stop and second to the next at the alighting stop and second.
///
/// # Errors
///
/// [`MapError::NotTheRunsTransit`] for a run the timetable does not have, or a ride whose calls
/// are not the run's.
pub(crate) fn ride_stops<'a>(
    timetable: &'a Timetable,
    ride: &Ride,
) -> Result<&'a [StopId], MapError> {
    if ride.run.raw() >= timetable.run_count() {
        return Err(MapError::NotTheRunsTransit);
    }
    let calls: Vec<_> = timetable.calls(ride.run).collect();
    let board = calls.iter().position(|&(s, _, d)| (s, d) == ride.board);
    let alight = board.and_then(|i| {
        calls[i + 1..].iter().position(|&(s, a, _)| (s, a) == ride.alight).map(|k| i + 1 + k)
    });
    match (board, alight) {
        (Some(i), Some(j)) => Ok(&timetable.pattern_stops(timetable.run_pattern(ride.run))[i..=j]),
        _ => Err(MapError::NotTheRunsTransit),
    }
}
