//! Two aggregations of a trips table, in space and in time (S294).
//!
//! Each is general — it takes any trips and changes nothing else about them — and together they
//! make a dynamic demand static: every trip end at its zone's point ([`aggregate_space`]) and
//! every departure at one instant ([`aggregate_time`]). The static assignment (flow level 1)
//! applies both before it routes; on a demand already made that way (one point per zone, one
//! departure instant) they change nothing.

use openmobisim_core_graph::geometry::LonLat;
use openmobisim_core_types::time::Second;

use crate::trips::RawTrip;

/// The length of the quarter hour the default start is rounded down to, in seconds.
pub const QUARTER_HOUR_S: u32 = 900;

/// What [`aggregate_space`] did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SpaceAggregation {
    /// Trip ends (origins and destinations) moved to their zone's point.
    pub moved: u64,
    /// Trip ends already at their zone's point, left as they were.
    pub at_zone_point: u64,
}

/// For each of `points`, the index of the nearest of `zones` (the first of equally near ones);
/// `None` for every point if there is no zone.
///
/// Nearest by the equirectangular distance at the point's latitude (east–west degrees scaled by
/// the cosine of the latitude), which orders points as the ground distance does at a city's
/// scale. **Cost:** one comparison per point and zone (a few nanoseconds each): a million trip
/// ends over a thousand zones take about a second.
#[must_use]
pub fn nearest_zone(points: &[LonLat], zones: &[LonLat]) -> Vec<Option<usize>> {
    points
        .iter()
        .map(|p| {
            let scale = p.lat.to_radians().cos();
            let mut best: Option<(usize, f64)> = None;
            for (z, zone) in zones.iter().enumerate() {
                let dx = (p.lon - zone.lon) * scale;
                let dy = p.lat - zone.lat;
                let d = dx * dx + dy * dy;
                if best.is_none_or(|(_, b)| d < b) {
                    best = Some((z, d));
                }
            }
            best.map(|(z, _)| z)
        })
        .collect()
}

/// Move every trip end (origin and destination) to the point of its zone, the zone whose point
/// is nearest ([`nearest_zone`]): the trips of one zone pair then share their two points, and
/// their routes. A trips table carries no zone, so the zone of a trip end is the one whose point
/// is nearest it; a gateway (a zone of one point) keeps its trips there. With no zone, nothing
/// moves. Returns how many ends moved and how many were already at their zone's point.
pub fn aggregate_space(trips: &mut [RawTrip], zones: &[LonLat]) -> SpaceAggregation {
    let mut report = SpaceAggregation::default();
    if zones.is_empty() {
        return report;
    }
    let points: Vec<LonLat> = trips.iter().flat_map(|t| [t.origin, t.destination]).collect();
    let nearest = nearest_zone(&points, zones);
    for (t, ends) in trips.iter_mut().zip(nearest.chunks_exact(2)) {
        for (end, zone) in [&mut t.origin, &mut t.destination].into_iter().zip(ends) {
            // Every point has a nearest zone: there is at least one.
            let Some(&point) = zone.and_then(|z| zones.get(z)) else { continue };
            if end.lon.to_bits() == point.lon.to_bits() && end.lat.to_bits() == point.lat.to_bits()
            {
                report.at_zone_point += 1;
            } else {
                *end = point;
                report.moved += 1;
            }
        }
    }
    report
}

/// The instant [`aggregate_time`] uses unless told otherwise: the earliest departure rounded
/// down to the quarter hour (the default start of a run's window); 0 for no trips.
#[must_use]
pub fn default_start(trips: &[RawTrip]) -> Second {
    let earliest = trips.iter().map(|t| t.departure_time.get()).min().unwrap_or(0);
    Second(earliest / QUARTER_HOUR_S * QUARTER_HOUR_S)
}

/// Set every departure to `start`: the demand in one time bin. Returns how many departures
/// changed.
pub fn aggregate_time(trips: &mut [RawTrip], start: Second) -> u64 {
    let mut moved = 0;
    for t in trips {
        if t.departure_time != start {
            t.departure_time = start;
            moved += 1;
        }
    }
    moved
}

/// The period a demand departs over, in seconds: from its earliest departure rounded down to the
/// quarter hour to the end of the quarter hour of its latest, so it holds every quarter hour in
/// which a trip departs (at least one); 0 for no trips. The static assignment's hourly flows are
/// the trips over this period.
#[must_use]
pub fn departure_period_s(trips: &[RawTrip]) -> u32 {
    let mut departures = trips.iter().map(|t| t.departure_time.get());
    let Some(first) = departures.next() else { return 0 };
    let (lo, hi) = departures.fold((first, first), |(lo, hi), d| (lo.min(d), hi.max(d)));
    (hi / QUARTER_HOUR_S + 1) * QUARTER_HOUR_S - lo / QUARTER_HOUR_S * QUARTER_HOUR_S
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trip(o: (f64, f64), d: (f64, f64), at: u32) -> RawTrip {
        RawTrip {
            traveller_id: "t".to_string(),
            trip_seq: 0,
            origin: LonLat::new(o.0, o.1),
            destination: LonLat::new(d.0, d.1),
            departure_time: Second(at),
            user_class: "c".to_string(),
            weight: None,
            mode: None,
        }
    }

    /// **Property:** every end goes to the nearest zone point, an end already at one stays and
    /// is counted as such, and without zones nothing moves.
    #[test]
    fn trip_ends_move_to_the_nearest_zone_point() {
        let zones = [LonLat::new(4.30, 52.00), LonLat::new(4.40, 52.00)];
        let mut trips = vec![
            trip((4.31, 52.01), (4.40, 52.00), 25_200),
            trip((4.36, 51.99), (4.34, 52.00), 25_300),
        ];
        let report = aggregate_space(&mut trips, &zones);
        assert_eq!(report, SpaceAggregation { moved: 3, at_zone_point: 1 });
        assert_eq!(trips[0].origin, zones[0]);
        assert_eq!(trips[0].destination, zones[1]);
        assert_eq!(trips[1].origin, zones[1], "4.36 is nearer 4.40 than 4.30");
        assert_eq!(trips[1].destination, zones[0]);
        let again = aggregate_space(&mut trips, &zones);
        assert_eq!(again, SpaceAggregation { moved: 0, at_zone_point: 4 }, "idempotent");
        let mut untouched = vec![trip((4.31, 52.01), (4.40, 52.00), 0)];
        assert_eq!(aggregate_space(&mut untouched, &[]), SpaceAggregation::default());
        assert_eq!(untouched[0].origin, LonLat::new(4.31, 52.01));
        // Equally near: the first zone.
        assert_eq!(nearest_zone(&[LonLat::new(4.35, 52.00)], &zones), vec![Some(0)]);
    }

    /// **Property:** every departure goes to the start (the earliest rounded down to the quarter
    /// hour unless given), the count says how many moved, and the period holds every quarter
    /// hour a trip departs in.
    #[test]
    fn departures_move_to_one_instant_and_the_period_spans_their_quarter_hours() {
        let mut trips = vec![
            trip((0.0, 0.0), (0.1, 0.0), 25_260),
            trip((0.0, 0.0), (0.1, 0.0), 28_799),
            trip((0.0, 0.0), (0.1, 0.0), 26_000),
        ];
        assert_eq!(departure_period_s(&trips), 3_600, "07:00 to 08:00");
        assert_eq!(default_start(&trips), Second(25_200));
        let start = default_start(&trips);
        assert_eq!(aggregate_time(&mut trips, start), 3);
        assert!(trips.iter().all(|t| t.departure_time == Second(25_200)));
        assert_eq!(aggregate_time(&mut trips, Second(25_200)), 0, "idempotent");
        assert_eq!(departure_period_s(&trips), 900, "one instant: one quarter hour");
        let edge = vec![trip((0.0, 0.0), (0.1, 0.0), 25_200), trip((0.0, 0.0), (0.1, 0.0), 28_800)];
        assert_eq!(departure_period_s(&edge), 4_500, "a departure at 08:00 is in 08:00–08:15");
        assert_eq!(departure_period_s(&[]), 0);
    }
}
