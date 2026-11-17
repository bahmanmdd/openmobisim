//! A car link's two fields read from the tags: lanes in its own direction and its free-flow
//! speed. Capacity is the graph's (lanes × `capacity_per_lane`); nothing here sets it.
//!
//! # Lanes (wiki, `Key:lanes`)
//!
//! `lanes` counts the lanes of the whole carriageway, `lanes:forward` and `lanes:backward`
//! those along and against the way's node order, `lanes:both_ways` a centre lane either
//! direction may use (it belongs to neither). On a way cars drive one way, the lanes in that
//! direction are `lanes:forward` (or `:backward`) where tagged, else `lanes`. On a way cars
//! drive both ways, a side tagged is that side's; with the total and one side given, the other
//! side is total − given − both_ways; with the total alone, each side is
//! (total − both_ways) / 2 rounded down, at least 1 (version 1's rule: a three-lane two-way
//! street gives no side its third lane, the smaller error). With no lane tag, the class's
//! default. Repaired, noted (`lanes_repaired`): both sides and a total that is not their sum
//! with `lanes:both_ways` (the sides as tagged, the total ignored); a total and one side that
//! leave the other side no lane, or `lanes:both_ways` above the total (the other side 1 lane); a
//! direction cars drive with 0 lanes (1 lane). A count that is not a whole number from 0 to 255
//! is read as absent, noted (`tags_unread`). On a way cars do not drive no car tag is read.
//!
//! # Free-flow speed (wiki, `Key:maxspeed`)
//!
//! For the direction along the node order `maxspeed:forward` is read first, then `maxspeed`;
//! against it `maxspeed:backward`, then `maxspeed`; with neither, the class's default. A value
//! is a number in km/h, or a number followed by ` mph` (× 1.609 344). Anything else (`none`,
//! `walk`, `signals`, a country code such as `NL:urban`, a list) is read as absent, so the class's
//! default is taken where no other key gives a speed, noted (`speeds_defaulted`): no table of
//! country codes or words.

use crate::access::Directions;
use crate::source::OsmWay;

/// Kilometres in a mile (international mile, exactly).
pub const KM_PER_MILE: f64 = 1.609_344;

/// A car link's lanes and free-flow speed.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct CarFields {
    /// Lanes in the link's own direction, at least 1.
    pub lanes: u8,
    /// Free-flow speed, km/h.
    pub speed_km_h: f64,
}

/// What the reader set right on one way's car tags.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CarNotes {
    /// A lane count that was not a whole number from 0 to 255, read as absent.
    pub unread: bool,
    /// A speed that was not a speed, read as absent.
    pub speed: bool,
    /// Lane tags that contradicted or left a driven direction no lane, repaired.
    pub lanes: bool,
}

/// The car fields along and against the node order of `way`, for the directions cars drive
/// (`None` for a direction they do not), from its tags and its class's default
/// `(speed in km/h, lanes per direction)`; what was set right in `notes`.
pub fn car_fields(
    way: &OsmWay,
    cars: Directions,
    default: (f64, u8),
    notes: &mut CarNotes,
) -> (Option<CarFields>, Option<CarFields>) {
    // A way cars do not drive: its car tags are not read, so not noted either.
    if cars == Directions::NONE {
        return (None, None);
    }
    let (forward_lanes, backward_lanes) = lanes(way, cars, default.1, notes);
    let mut field = |driven: bool, lanes: u8, key: &str| -> Option<CarFields> {
        if !driven {
            return None;
        }
        let speed =
            speed(way, key, notes).or_else(|| speed(way, "maxspeed", notes)).unwrap_or(default.0);
        Some(CarFields { lanes, speed_km_h: speed })
    };
    let forward = field(cars.forward, forward_lanes, "maxspeed:forward");
    (forward, field(cars.backward, backward_lanes, "maxspeed:backward"))
}

/// The lanes along and against the node order (a direction cars do not drive gets 0).
fn lanes(way: &OsmWay, cars: Directions, default: u8, notes: &mut CarNotes) -> (u8, u8) {
    let total = count(way, "lanes", notes);
    let forward = count(way, "lanes:forward", notes);
    let backward = count(way, "lanes:backward", notes);
    let middle = count(way, "lanes:both_ways", notes).unwrap_or(0);
    let mut repaired = |n: u8| {
        notes.lanes = true;
        n
    };
    let (f, b) = match (cars.forward, cars.backward) {
        (false, false) => return (0, 0),
        (true, false) => (forward.or(total).unwrap_or(default), 0),
        (false, true) => (0, backward.or(total).unwrap_or(default)),
        (true, true) => match (total, forward, backward) {
            (Some(t), Some(f), Some(b))
                if u16::from(f) + u16::from(b) + u16::from(middle) != u16::from(t) =>
            {
                (repaired(f), b)
            }
            (_, Some(f), Some(b)) => (f, b),
            (Some(t), Some(f), None) => match rest(t, f, middle) {
                Some(r) => (f, r),
                None => (f, repaired(1)),
            },
            (Some(t), None, Some(b)) => match rest(t, b, middle) {
                Some(r) => (r, b),
                None => (repaired(1), b),
            },
            (Some(t), None, None) => match t.checked_sub(middle) {
                Some(r) => ((r / 2).max(1), (r / 2).max(1)),
                None => (repaired(1), 1),
            },
            (None, f, b) => (f.unwrap_or(default), b.unwrap_or(default)),
        },
    };
    let mut at_least_one = |driven: bool, n: u8| if driven && n == 0 { repaired(1) } else { n };
    (at_least_one(cars.forward, f), at_least_one(cars.backward, b))
}

/// The other side's lanes: total − given − both_ways, if at least one.
fn rest(total: u8, given: u8, middle: u8) -> Option<u8> {
    total.checked_sub(given).and_then(|r| r.checked_sub(middle)).filter(|&r| r > 0)
}

/// A lane count, if tagged as a whole number from 0 to 255 (any other value read as absent,
/// noted).
fn count(way: &OsmWay, key: &str, notes: &mut CarNotes) -> Option<u8> {
    let value = way.tag(key)?;
    let read = value.parse::<u8>().ok();
    notes.unread |= read.is_none();
    read
}

/// A speed, if tagged, in km/h: a number above zero, in km/h or followed by ` mph` (any other
/// value read as absent, noted).
fn speed(way: &OsmWay, key: &str, notes: &mut CarNotes) -> Option<f64> {
    let value = way.tag(key)?;
    let (number, factor) = match value.strip_suffix(" mph") {
        Some(n) => (n, KM_PER_MILE),
        None => (value, 1.0),
    };
    match number.parse::<f64>() {
        Ok(n) if n.is_finite() && n > 0.0 => Some(n * factor),
        _ => {
            notes.speed = true;
            None
        }
    }
}
