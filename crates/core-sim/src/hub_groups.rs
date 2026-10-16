//! Hub groups (C4, D-6 b): one label for a station's stops and the parkings beside them, written in
//! the outputs (`hub_group` in `transit_calls.parquet` and `parking_bins.parquet`).
//!
//! **A label only.** The model's own hubs (a parking with the stops its travellers reach on foot)
//! are unchanged, and no result depends on the groups, so they are in no fingerprint. A stop's group
//! is its GTFS `parent_station`, or the stop itself without one. A parking's group is the group of
//! the nearest stop within [`DEFAULT_HUB_GROUP_M`] (the run's `hub_group_m`), in a straight line,
//! unless its table named a hub (`hub_id`), which it keeps; a parking with neither is its own group,
//! by its hub's id. One set of hubs with docks and the operator's interface comes later (D-6 a).
//!
//! **Cost.** Once per run, after setup: every parking against every stop (Paris: 2 684 × 3 287,
//! a few milliseconds), and one string per stop and per parking.

use openmobisim_core_graph::geometry::ground_distance_metres;
use openmobisim_core_transit::Timetable;
use openmobisim_core_types::ids::{EntityId, NodeId};

use crate::parking::ParkingSetup;

/// How far a parking may be from a stop, in a straight line, to join that stop's group, in metres
/// (C4): a short walk across a station's forecourt. An assumption, uncalibrated; it labels the
/// outputs only.
pub const DEFAULT_HUB_GROUP_M: f64 = 150.0;

/// Each stop's and each parking's hub group.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HubGroups {
    /// Each stop's group, by stop index.
    pub stops: Vec<String>,
    /// Each parking's group, by parking index.
    pub parkings: Vec<String>,
}

/// The groups of the run's stops and parkings (either may be absent).
///
/// # Panics
///
/// If there are more than `u32::MAX` parkings.
#[must_use]
pub fn hub_groups(
    timetable: Option<&Timetable>,
    parking: Option<&ParkingSetup>,
    radius_m: f64,
) -> HubGroups {
    let stops: Vec<String> = timetable.map_or_else(Vec::new, |t| {
        (0..t.stop_count())
            .map(|s| t.stop_parent(NodeId::new(s)).unwrap_or_else(|| t.stop_ids().external(s)))
            .map(str::to_string)
            .collect()
    });
    let parkings = parking.map_or_else(Vec::new, |p| {
        let n = u32::try_from(p.count()).expect("parkings fit u32");
        (0..n)
            .map(|k| {
                let hub = p.hub_external_id(k);
                if hub != p.external_id(k) {
                    return hub.to_string(); // a hub its table named
                }
                let at = p.position(k);
                let nearest = timetable.and_then(|t| {
                    (0..t.stop_count())
                        .map(|s| (s, ground_distance_metres(at, t.stop_position(NodeId::new(s)))))
                        .filter(|&(_, d)| d <= radius_m)
                        .min_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)))
                });
                nearest.map_or_else(|| hub.to_string(), |(s, _)| stops[s as usize].clone())
            })
            .collect()
    });
    HubGroups { stops, parkings }
}
