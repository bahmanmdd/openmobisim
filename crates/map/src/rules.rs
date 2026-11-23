//! The display rules, fixed: what a place, a pair and a journey are, how many pairs the page
//! holds, how a link is drawn, and the bands the road traffic is coloured in. Every rule of the
//! map is here and nowhere else; the page reads the bands from the file's data.

/// The most pairs the page holds: the busiest, by travellers.
pub const MAX_PAIRS: usize = 2_000;

/// Without road classes on every link, a car link's drawing class from its capacity (veh/h, all
/// lanes): the first row whose capacity it reaches (an infinite capacity, no limit, reaches the
/// first). Five classes, 0 the largest roads: the page draws the classes up to a limit that grows
/// as it zooms in (0 and 1 at an overview, then 2, then every car link, then every link), and the
/// links beyond it faint, so the largest roads stay in view.
pub const CLASS_BY_CAPACITY: [(f64, u8); 3] = [(3_600.0, 0), (2_400.0, 1), (1_600.0, 2)];

/// The drawing class of a car link below the last row of [`CLASS_BY_CAPACITY`].
pub const CLASS_OTHER_CAR: u8 = 3;

/// The drawing class of a link no car may use: a bike or walk link, drawn only zoomed in.
pub const CLASS_NO_CAR: u8 = 4;

/// Version 1's road classes in drawing order, 0 the largest roads: a network whose every link has
/// a road class is drawn by them. A link's drawing class is its class's place here ([`road_rank`]);
/// the page draws the classes up to a limit that grows as it zooms in (5, 9, 13, then 16: every
/// road and path), the roads (0 to 13) as base lines, the rest (paths, cycleways) only zoomed in.
pub const ROAD_CLASSES: [&str; 19] = [
    "motorway",
    "motorway_link",
    "trunk",
    "trunk_link",
    "primary",
    "primary_link",
    "secondary",
    "secondary_link",
    "tertiary",
    "tertiary_link",
    "unclassified",
    "residential",
    "living_street",
    "service",
    "pedestrian",
    "footway",
    "cycleway",
    "ferry",
    "busway",
];

/// Class names drawn as another of [`ROAD_CLASSES`]: version 1's reading of OpenStreetMap's
/// `highway` values (`road` is an unclassified road; steps, paths, tracks and corridors are
/// footways).
pub const ROAD_CLASS_AS: [(&str, &str); 5] = [
    ("road", "unclassified"),
    ("steps", "footway"),
    ("path", "footway"),
    ("track", "footway"),
    ("corridor", "footway"),
];

/// A road class's drawing class: its place in [`ROAD_CLASSES`] (through [`ROAD_CLASS_AS`]); a
/// name of neither is drawn as `unclassified`, as version 1 read an unknown `highway` value.
#[must_use]
pub fn road_rank(name: &str) -> u8 {
    let name = ROAD_CLASS_AS.iter().find(|(from, _)| *from == name).map_or(name, |(_, to)| to);
    ROAD_CLASSES.iter().zip(0_u8..).find(|(c, _)| **c == name).map_or(10, |(_, rank)| rank)
}

/// With road classes, a base line's width by its road's tier, as multiples of the thinnest:
/// tier 0 a link without cars, tiers 1 to 4 a car link by its capacity over the large roads' (the
/// 99th percentile of the car links' capacities; an unlimited capacity is 1), from 0, from each
/// of [`ROAD_TIER_BOUNDS`] on (version 1's `ROAD_TIER_WIDTHS` and `ROAD_TIER_BOUNDS`).
pub const ROAD_TIER_WIDTHS: [f64; 5] = [0.5, 0.8, 1.2, 2.0, 3.0];
/// The capacity ratios at which tiers 2, 3 and 4 start.
pub const ROAD_TIER_BOUNDS: [f64; 3] = [0.25, 0.45, 0.75];

/// Cyclists and walkers per link over the window at which a line is drawn at its full width, the
/// same for both: a line is `1 + 7 √(min(n, FLOW_FULL) / FLOW_FULL)` px wide on the map (and
/// `1.5 + 8 √(…)` units on the summary's maps), so the two layers and two runs read alike. 3 000:
/// Delft's busiest cyclists' link carries 3 006 in the morning peak, so Delft's lines are as wide as
/// when each layer was scaled to its own busiest link.
pub const FLOW_FULL: u32 = 3_000;

/// The share of the full width's square a link of `n` cyclists or walkers is drawn at:
/// `√(min(n, FLOW_FULL) / FLOW_FULL)`, 0 to 1.
#[must_use]
pub fn flow_share(n: u32) -> f64 {
    (f64::from(n.min(FLOW_FULL)) / f64::from(FLOW_FULL)).sqrt()
}

/// The delay over free flow (mean car time over free-flow time, less one): the upper edges of
/// its first three bands, and the four bands' labels.
pub const DELAY_BANDS: [f64; 3] = [0.15, 0.5, 1.0];
/// The labels of the four delay bands.
pub const DELAY_BAND_LABELS: [&str; 4] = ["0–15%", "15–50%", "50–100%", "100%+"];

/// Volume over capacity (the busiest period's volume per hour over the capacity): the
/// upper edges of its first three bands; the busiest period as [`PEAK_SECONDS`] says.
pub const VC_BANDS: [f64; 3] = [0.6, 0.8, 1.0];
/// The labels of the four volume-over-capacity bands.
pub const VC_BAND_LABELS: [&str; 4] = ["0–0.6", "0.6–0.8", "0.8–1.0", "1.0+"];

/// The delay over free flow of cars crossing a link: their mean crossing time over the link's car
/// free-flow time, less one, never below 0 (a car crosses at free flow at best); 0 for a free-flow
/// time of 0. The one definition: the map's page mirrors it, the summary reads it.
#[must_use]
pub fn delay(mean_time: f64, free_flow: f64) -> f64 {
    if free_flow > 0.0 { (mean_time / free_flow - 1.0).max(0.0) } else { 0.0 }
}

/// The least delay per car, seconds (the cars' mean crossing time less the free-flow time), at
/// which a link takes a delay band above the first: below it a link is drawn as free flow,
/// whatever its delay over free flow (a link crossed in 0.433 s against 0.045 s free flow is 862 %
/// over free flow and costs each car 0.39 s). Bikes and walkers are drawn by their counts, never
/// by delay.
pub const DELAY_FLOOR_S: f64 = 2.0;

/// The delay band of cars crossing a link, 0 to 3 (an index of [`DELAY_BAND_LABELS`]): the bands
/// [`DELAY_BANDS`] of [`delay`], and 0 when their delay per car is under [`DELAY_FLOOR_S`]. The one
/// rule of the delay's colour: the map's page mirrors it, the summary reads it.
#[must_use]
pub fn delay_band(mean_time: f64, free_flow: f64) -> usize {
    if mean_time - free_flow < DELAY_FLOOR_S {
        return 0;
    }
    let d = delay(mean_time, free_flow);
    DELAY_BANDS.iter().filter(|&&e| d >= e).count()
}

/// A link's drawing class: by its capacity when cars may use it, else [`CLASS_NO_CAR`].
#[must_use]
pub fn class(car: bool, capacity_veh_h: f64) -> u8 {
    if !car {
        return CLASS_NO_CAR;
    }
    CLASS_BY_CAPACITY
        .iter()
        .find(|&&(at_least, _)| capacity_veh_h >= at_least)
        .map_or(CLASS_OTHER_CAR, |&(_, class)| class)
}

/// The seconds the busiest period of volume over capacity aims at: the page takes the busiest run
/// of `k = max(1, min(bins shown, round(900 / bin)))` consecutive bins, and names its length, `k`
/// times the bin (15 min for bins of 300 s, 20 min for 600 s, 1 h for 3 600 s).
pub const PEAK_SECONDS: u32 = 900;
