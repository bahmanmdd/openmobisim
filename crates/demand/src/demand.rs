//! The demand of a run: its trips, each end linked to the network per mode, and the transit
//! access and egress lists of each trip.
//!
//! # Linking
//!
//! Each end of a trip is linked, for each mode of its set, to a node by straight line (ties to
//! the lower node id; [`crate::index`]). Where both ends are nearest the same node of the
//! mode's network, both join it: the trip travels no link. Otherwise the origin joins the
//! nearest node with an out-link that allows the mode (a trip can leave it) and the destination
//! the nearest with an in-link (a trip can arrive at it). Transit's ends are linked by walking,
//! to the walk node. The straight line from the end to its node, the gap, is kept in metres.
//! Each stop of the timetable is linked to its nearest node of the walk network, one node for
//! the walk to it and the walk from it: the reach is searched both ways from that node, so a
//! stop on a node walkers can only leave is walked to from that node alone.
//!
//! # Access and egress
//!
//! A walker's seconds over a gap are its metres over the network's walk speed. The access list
//! of a trip with transit in its set holds each stop reached on foot from the origin's walk
//! node, with the walking seconds: the origin's gap, the walk from its node to the stop's node
//! ([`crate::reach`]), and the stop's gap. A stop is kept when that whole walk, in metres at
//! walking speed, is at most `stop_walk_max`, read to a part in a billion (1.5 µm at 1 500 m)
//! so that the rounding of a sum of link times cannot drop a walk of exactly the limit. The
//! egress list is the same from each stop to the destination. An empty list on either end is
//! not an error: the trip has no transit alternative, and the ends are counted.
//!
//! # Identity
//!
//! Trips and classes get dense ids by sorted external id, whatever the order of the rows. Every
//! list is in stop-id order. Nothing depends on the number of threads.

use openmobisim_graph::geometry::Xy;
use openmobisim_graph::network::Network;
use openmobisim_transit::timetable::Timetable;
use openmobisim_types::fingerprint::Fingerprint;
use openmobisim_types::ids::{EntityId, ExternalIdTable, NodeId, StopId, TripId, UserClassId};
use openmobisim_types::mode::{Mode, ModeSet};
use openmobisim_types::time::Second;
use openmobisim_types::units::{Duration, Metres};
use rayon::prelude::*;

use crate::index::{Joins, NodeIndex};
use crate::input::{DemandError, TripRow, Trips, check};
use crate::reach::NearStops;

/// Where a trip end or a stop joins the network: its node, and the straight line to it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct End {
    /// The nearest node allowing the mode (walking, for transit and for a stop).
    pub node: NodeId,
    /// The straight-line distance from the position to the node, in metres.
    pub gap: Metres,
}

/// The column of a mode in the per-mode tables: car, bike, walk, transit.
fn column(mode: Mode) -> usize {
    usize::from(mode.as_u8())
}

/// One side of every trip (origins or destinations): per mode, per trip, its node (null when
/// the mode is not in the trip's set) and gap in metres.
#[derive(Clone, Debug, Default)]
struct Side {
    node: [Vec<NodeId>; 4],
    gap: [Vec<f64>; 4],
}

impl Side {
    fn end(&self, trip: TripId, mode: Mode) -> Option<End> {
        let c = column(mode);
        let node = self.node[c][trip.index()].to_option()?;
        Some(End { node, gap: Metres(self.gap[c][trip.index()]) })
    }
}

/// Per trip, a list of (stop, seconds): `start[t]..start[t + 1]` of `items`.
#[derive(Clone, Debug, Default)]
struct Lists {
    start: Vec<u32>,
    items: Vec<(StopId, Duration)>,
}

impl Lists {
    fn new(lists: Vec<Vec<(StopId, Duration)>>) -> Self {
        let mut start = Vec::with_capacity(lists.len() + 1);
        start.push(0_u32);
        let mut items = Vec::new();
        for list in lists {
            items.extend(list);
            start.push(u32::try_from(items.len()).expect("fewer than 2^32 stops in all lists"));
        }
        Self { start, items }
    }

    fn of(&self, trip: TripId) -> &[(StopId, Duration)] {
        &self.items[self.start[trip.index()] as usize..self.start[trip.index() + 1] as usize]
    }
}

/// The demand of a run, as a structure of arrays read through accessors (see the module
/// documentation). Built once by [`Demand::build`], then read-only.
#[derive(Clone, Debug)]
pub struct Demand {
    trips: Trips,
    origin: Side,
    destination: Side,
    access: Lists,
    egress: Lists,
    stop_end: Vec<End>,
    origins_without_stop: u32,
    destinations_without_stop: u32,
    without_transit: u32,
}

/// The ends of one trip: per side (origin, destination) and mode, the node and gap; null and
/// NaN for a mode not in its set.
type TripEnds = [[(NodeId, f64); 4]; 2];

/// Link both ends of a trip for every mode of its set (`indices[mode]` holds the mode's nodes
/// by [`Joins::Either`], [`Joins::Out`] and [`Joins::In`]): where both ends are nearest the
/// same node of the mode's network, both join it (no link is travelled); otherwise the origin
/// joins the nearest node it can leave and the destination the nearest it can arrive at. The
/// mode for which no node is kept when there is one.
fn link_trip(
    indices: &[[NodeIndex; 3]; 3],
    modes: ModeSet,
    ends: [Xy; 2],
) -> Result<TripEnds, Mode> {
    let mut out = [[(NodeId::NULL, f64::NAN); 4]; 2];
    for mode in modes.iter() {
        let [either, leave, arrive] =
            &indices[column(if mode == Mode::Transit { Mode::Walk } else { mode })];
        let (o, d) = (either.nearest(ends[0]).ok_or(mode)?, either.nearest(ends[1]).ok_or(mode)?);
        let (o, d) = if o.0 == d.0 {
            (o, d)
        } else {
            (leave.nearest(ends[0]).ok_or(mode)?, arrive.nearest(ends[1]).ok_or(mode)?)
        };
        (out[0][column(mode)], out[1][column(mode)]) = (o, d);
    }
    Ok(out)
}

/// Link every trip, in parallel; the first trip that cannot be linked, in id order, refused.
fn link_trips(network: &Network, trips: &Trips) -> Result<[Side; 2], DemandError> {
    let indices = [Mode::Car, Mode::Bike, Mode::Walk].map(|m| {
        [Joins::Either, Joins::Out, Joins::In].map(|joins| NodeIndex::new(network, m, joins))
    });
    let linked: Vec<Result<TripEnds, Mode>> = (0..trips.modes.len())
        .into_par_iter()
        .map(|t| link_trip(&indices, trips.modes[t], [trips.origin[t], trips.destination[t]]))
        .collect();
    let mut sides = [Side::default(), Side::default()];
    for (t, ends) in linked.into_iter().enumerate() {
        let ends = ends.map_err(|mode| DemandError::NoNode {
            trip: trips.ids.external(TripId::from_index(t).raw()).to_owned(),
            mode,
        })?;
        for (side, per_mode) in sides.iter_mut().zip(ends) {
            for (c, (node, gap)) in per_mode.into_iter().enumerate() {
                side.node[c].push(node);
                side.gap[c].push(gap);
            }
        }
    }
    Ok(sides)
}

/// Link every stop of `timetable` to its nearest walk node.
fn link_stops(network: &Network, timetable: &Timetable) -> Result<Vec<End>, DemandError> {
    let walk = NodeIndex::new(network, Mode::Walk, Joins::Either);
    StopId::iter_space(timetable.stop_count())
        .map(|stop| {
            let (x, y) = timetable.stop_position(stop);
            let (node, gap) =
                walk.nearest(Xy::new(x.get(), y.get())).ok_or_else(|| DemandError::StopNoNode {
                    stop: timetable.stop_ids().external(stop.raw()).to_owned(),
                })?;
            Ok(End { node, gap: Metres(gap) })
        })
        .collect()
}

/// The access and egress lists of every trip, in parallel: for a trip with transit in its set,
/// each stop within the limit (`limit` walking seconds, `speed` m/s), by stop; empty otherwise.
fn lists(
    trips: &Trips,
    [origin, destination]: &[Side; 2],
    stops: &[End],
    near: &NearStops,
    (speed, limit): (f64, f64),
) -> [Lists; 2] {
    let walk = |gap: f64, list: &[(StopId, f64)]| -> Vec<(StopId, Duration)> {
        list.iter()
            .filter_map(|&(stop, seconds)| {
                let total = gap / speed + seconds + stops[stop.index()].gap.get() / speed;
                (total <= limit).then_some((stop, Duration(total)))
            })
            .collect()
    };
    let per_trip: Vec<_> = (0..trips.ids.count())
        .into_par_iter()
        .map(|t| {
            let t = TripId::new(t);
            let (Some(o), Some(d)) =
                (origin.end(t, Mode::Transit), destination.end(t, Mode::Transit))
            else {
                return (Vec::new(), Vec::new());
            };
            (
                walk(o.gap.get(), near.stops_reached_from(o.node)),
                walk(d.gap.get(), near.stops_reaching(d.node)),
            )
        })
        .collect();
    let (access, egress): (Vec<_>, Vec<_>) = per_trip.into_iter().unzip();
    [Lists::new(access), Lists::new(egress)]
}

impl Demand {
    /// The demand of `rows` over `classes` (external ids) on `network`, with the transit access
    /// and egress lists of `timetable` (none: every transit trip is without a stop in reach),
    /// a walk to a stop being at most `stop_walk_max` metres.
    ///
    /// # Errors
    ///
    /// A [`DemandError`] naming what it refuses: `stop_walk_max` not a finite number at or above
    /// zero; a class or a trip given twice; then trip by trip in id order, an unknown class, an
    /// empty mode set, mixed position kinds, a position the network cannot take, a mode no link
    /// allows; then a stop when no link allows walking.
    pub fn build(
        rows: &[TripRow],
        classes: &[String],
        network: &Network,
        timetable: Option<&Timetable>,
        stop_walk_max: Metres,
    ) -> Result<Self, DemandError> {
        let max = stop_walk_max.get();
        if !max.is_finite() || max < 0.0 {
            return Err(DemandError::StopWalkMax { metres: max });
        }
        let trips = check(rows, classes, network)?;
        let sides = link_trips(network, &trips)?;
        let stop_end = match timetable {
            Some(timetable) => link_stops(network, timetable)?,
            None => Vec::new(),
        };
        let [access, egress] = if stop_end.is_empty() {
            let none = || Lists::new(vec![Vec::new(); trips.modes.len()]);
            [none(), none()]
        } else {
            let speed = network.walk_speed().get();
            let limit = max / speed * (1.0 + 1e-9);
            let stop_nodes: Vec<NodeId> = stop_end.iter().map(|e| e.node).collect();
            let near = NearStops::new(network, &stop_nodes, Duration(limit));
            lists(&trips, &sides, &stop_end, &near, (speed, limit))
        };
        let [origin, destination] = sides;
        let mut demand = Self {
            trips,
            origin,
            destination,
            access,
            egress,
            stop_end,
            origins_without_stop: 0,
            destinations_without_stop: 0,
            without_transit: 0,
        };
        demand.count_ends_without_stop();
        Ok(demand)
    }

    /// Count the transit trips' ends with an empty list, and the trips with either.
    fn count_ends_without_stop(&mut self) {
        for t in TripId::iter_space(self.trip_count()) {
            if !self.modes(t).contains(Mode::Transit) {
                continue;
            }
            let (a, e) = (self.access(t).is_empty(), self.egress(t).is_empty());
            self.origins_without_stop += u32::from(a);
            self.destinations_without_stop += u32::from(e);
            self.without_transit += u32::from(a || e);
        }
    }

    /// How many trips.
    #[must_use]
    pub fn trip_count(&self) -> u32 {
        self.trips.ids.count()
    }

    /// The trips' external ids, in trip-id order.
    #[must_use]
    pub fn trip_ids(&self) -> &ExternalIdTable {
        &self.trips.ids
    }

    /// How many user classes.
    #[must_use]
    pub fn class_count(&self) -> u32 {
        self.trips.classes.count()
    }

    /// The classes' external ids, in class-id order.
    #[must_use]
    pub fn class_ids(&self) -> &ExternalIdTable {
        &self.trips.classes
    }

    /// A trip's user class.
    #[must_use]
    pub fn class(&self, trip: TripId) -> UserClassId {
        self.trips.class[trip.index()]
    }

    /// A trip's departure, a second of the clock.
    #[must_use]
    pub fn departure(&self, trip: TripId) -> Second {
        self.trips.departure[trip.index()]
    }

    /// The modes open to a trip.
    #[must_use]
    pub fn modes(&self, trip: TripId) -> ModeSet {
        self.trips.modes[trip.index()]
    }

    /// Where a trip starts, in metres in the network's plane.
    #[must_use]
    pub fn origin(&self, trip: TripId) -> Xy {
        self.trips.origin[trip.index()]
    }

    /// Where a trip ends, in metres in the network's plane.
    #[must_use]
    pub fn destination(&self, trip: TripId) -> Xy {
        self.trips.destination[trip.index()]
    }

    /// Where a trip's origin joins the network for `mode` (transit: its walk node); `None`
    /// when the mode is not in the trip's set.
    #[must_use]
    pub fn origin_end(&self, trip: TripId, mode: Mode) -> Option<End> {
        self.origin.end(trip, mode)
    }

    /// Where a trip's destination joins the network for `mode`, as [`Self::origin_end`].
    #[must_use]
    pub fn destination_end(&self, trip: TripId, mode: Mode) -> Option<End> {
        self.destination.end(trip, mode)
    }

    /// A trip's access list: each stop within walking reach of its origin, by stop, with the
    /// walking seconds from the origin to the stop; empty without transit in its set.
    #[must_use]
    pub fn access(&self, trip: TripId) -> &[(StopId, Duration)] {
        self.access.of(trip)
    }

    /// A trip's egress list: each stop from which its destination is within walking reach, by
    /// stop, with the walking seconds from the stop to the destination.
    #[must_use]
    pub fn egress(&self, trip: TripId) -> &[(StopId, Duration)] {
        self.egress.of(trip)
    }

    /// How many stops (the timetable's; 0 without one).
    #[must_use]
    pub fn stop_count(&self) -> u32 {
        #[allow(clippy::cast_possible_truncation, reason = "the timetable's stop count, a u32")]
        let n = self.stop_end.len() as u32;
        n
    }

    /// Where a stop joins the walk network.
    #[must_use]
    pub fn stop_end(&self, stop: StopId) -> End {
        self.stop_end[stop.index()]
    }

    /// How many trips with transit in their set have no stop in reach of their origin.
    #[must_use]
    pub fn origins_without_stop(&self) -> u32 {
        self.origins_without_stop
    }

    /// How many trips with transit in their set have no stop in reach of their destination.
    #[must_use]
    pub fn destinations_without_stop(&self) -> u32 {
        self.destinations_without_stop
    }

    /// How many trips with transit in their set have no transit alternative: an empty access
    /// or egress list.
    #[must_use]
    pub fn trips_without_transit(&self) -> u32 {
        self.without_transit
    }

    /// The fingerprint of what the demand was built from (`openmobisim_types::fingerprint`),
    /// fed in this order: the class count, then each class's external id in id order; the trip
    /// count, then per trip in id order its external id, class id, departure (a second of the
    /// clock), modes, and origin and destination x and y (metres in the network's plane). Two
    /// builds of the same rows, in any order, have the same fingerprint. The joins to nodes and
    /// stops and the access and egress lists are not fed: they follow from the network, the
    /// timetable and `stop_walk_max`.
    #[must_use]
    pub fn fingerprint(&self) -> u64 {
        let mut h = Fingerprint::new();
        let t = &self.trips;
        h.u32(t.classes.count());
        for id in t.classes.iter() {
            h.str(id);
        }
        h.u32(t.ids.count());
        for (i, id) in t.ids.iter().enumerate() {
            h.str(id);
            h.u32(t.class[i].raw());
            h.u32(t.departure[i].get());
            h.modes(t.modes[i]);
            for p in [t.origin[i], t.destination[i]] {
                h.f64(p.x);
                h.f64(p.y);
            }
        }
        h.finish()
    }
}
