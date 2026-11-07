//! A car link's two fields read from the tags: lanes in its own direction and its free-flow
//! speed. Capacity is the graph's (lanes × `capacity_per_lane`); nothing here sets it.
//!
//! # Lanes (wiki, `Key:lanes`)
//!
//! `lanes` counts the lanes of the whole carriageway, `lanes:forward` and `lanes:backward`
//! those along and against the way's node order, `lanes:both_ways` a centre lane either
//! direction may use (it belongs to neither). On a way cars drive one way, the lanes in that
//! direction are `lanes:forward` (or `:backward`) where tagged, else `lanes`. On a way cars
//! drive both ways, a side tagged is that side's (with both sides and the total given, the
//! total must be their sum with `lanes:both_ways`); with the total and one side given, the
//! other side is total − given − both_ways; with the total alone, each side is
//! (total − both_ways) / 2 rounded down, at least 1 (version 1's rule: a three-lane two-way
//! street gives no side its third lane, the smaller error). With no lane tag, the class's
//! default. A count that cannot be read, or that leaves a direction cars drive with no lane,
//! is an error naming the way.
//!
//! # Free-flow speed (wiki, `Key:maxspeed`)
//!
//! For the direction along the node order `maxspeed:forward` is read first, then `maxspeed`;
//! against it `maxspeed:backward`, then `maxspeed`; with neither, the class's default. A value
//! is a number in km/h, or a number followed by ` mph` (× 1.609 344). Anything else (`none`,
//! `walk`, `signals`, a country code such as `NL:urban`, a list) is an error naming the way:
//! nothing is silently replaced by a default.

use crate::access::{Directions, bad};
use crate::source::{OsmError, OsmWay};

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

/// The car fields along and against the node order of `way`, for the directions cars drive
/// (`None` for a direction they do not), from its tags and its class's default
/// `(speed in km/h, lanes per direction)`.
///
/// # Errors
///
/// [`OsmError::Tag`] for a lane count or speed that cannot be read, [`OsmError::Lanes`] for
/// lane counts that leave a driven direction with no lane.
pub fn car_fields(
    way: &OsmWay,
    cars: Directions,
    default: (f64, u8),
) -> Result<(Option<CarFields>, Option<CarFields>), OsmError> {
    let (forward_lanes, backward_lanes) = lanes(way, cars, default.1)?;
    let field = |driven: bool, lanes: u8, key: &str| -> Result<Option<CarFields>, OsmError> {
        if !driven {
            return Ok(None);
        }
        let speed = match speed(way, key)? {
            Some(s) => s,
            None => speed(way, "maxspeed")?.unwrap_or(default.0),
        };
        Ok(Some(CarFields { lanes, speed_km_h: speed }))
    };
    Ok((
        field(cars.forward, forward_lanes, "maxspeed:forward")?,
        field(cars.backward, backward_lanes, "maxspeed:backward")?,
    ))
}

/// The lanes along and against the node order (a direction cars do not drive gets 0).
fn lanes(way: &OsmWay, cars: Directions, default: u8) -> Result<(u8, u8), OsmError> {
    let total = count(way, "lanes")?;
    let forward = count(way, "lanes:forward")?;
    let backward = count(way, "lanes:backward")?;
    let middle = count(way, "lanes:both_ways")?.unwrap_or(0);
    let (f, b) = match (cars.forward, cars.backward) {
        (false, false) => return Ok((0, 0)),
        (true, false) => (forward.or(total).unwrap_or(default), 0),
        (false, true) => (0, backward.or(total).unwrap_or(default)),
        (true, true) => match (total, forward, backward) {
            (Some(t), Some(f), Some(b))
                if u16::from(f) + u16::from(b) + u16::from(middle) != u16::from(t) =>
            {
                return Err(OsmError::Lanes {
                    way: way.id,
                    why: format!(
                        "lanes={t} is not lanes:forward={f} + lanes:backward={b} + lanes:both_ways={middle}"
                    ),
                });
            }
            (_, Some(f), Some(b)) => (f, b),
            (Some(t), Some(f), None) => (f, rest(way, t, f, middle)?),
            (Some(t), None, Some(b)) => (rest(way, t, b, middle)?, b),
            (Some(t), None, None) => {
                let each = t.checked_sub(middle).ok_or_else(|| over(way, t, middle))? / 2;
                (each.max(1), each.max(1))
            }
            (None, f, b) => (f.unwrap_or(default), b.unwrap_or(default)),
        },
    };
    if (cars.forward && f == 0) || (cars.backward && b == 0) {
        return Err(OsmError::Lanes {
            way: way.id,
            why: format!("a direction cars drive has no lane ({f} forward, {b} backward)"),
        });
    }
    Ok((f, b))
}

/// The other side's lanes: total − given − both_ways, at least one.
fn rest(way: &OsmWay, total: u8, given: u8, middle: u8) -> Result<u8, OsmError> {
    match total.checked_sub(given).and_then(|r| r.checked_sub(middle)) {
        Some(r) if r > 0 => Ok(r),
        _ => Err(OsmError::Lanes {
            way: way.id,
            why: format!(
                "lanes={total} with {given} on one side and lanes:both_ways={middle} leaves the \
                 other side no lane"
            ),
        }),
    }
}

fn over(way: &OsmWay, total: u8, middle: u8) -> OsmError {
    OsmError::Lanes { way: way.id, why: format!("lanes:both_ways={middle} exceeds lanes={total}") }
}

/// A lane count, if tagged: a whole number from 0 to 255.
fn count(way: &OsmWay, key: &str) -> Result<Option<u8>, OsmError> {
    let Some(value) = way.tag(key) else { return Ok(None) };
    value.parse::<u8>().map(Some).map_err(|_| bad(way, key, value, "a whole number of lanes"))
}

/// A speed, if tagged, in km/h: a number above zero, in km/h or followed by ` mph`.
fn speed(way: &OsmWay, key: &str) -> Result<Option<f64>, OsmError> {
    let Some(value) = way.tag(key) else { return Ok(None) };
    let (number, factor) = match value.strip_suffix(" mph") {
        Some(n) => (n, KM_PER_MILE),
        None => (value, 1.0),
    };
    match number.parse::<f64>() {
        Ok(n) if n.is_finite() && n > 0.0 => Ok(Some(n * factor)),
        _ => Err(bad(way, key, value, "a speed above zero in km/h, or followed by \" mph\"")),
    }
}
