//! Fixtures shared by the loading tests: build values, rows, a two-node network, a line of links
//! and a grid (copied from the network tests), and the vehicles, windows and random draws the
//! loading tests use.

#![allow(dead_code, reason = "each test binary uses a different part of this module")]

use openmobisim_graph::geometry::{Position, Xy};
use openmobisim_graph::network::{BuildValues, LinkRow, Network, NetworkBuilder};
use openmobisim_loading::input::Vehicle;
use openmobisim_types::ids::{EntityId, LinkId, TripId};
use openmobisim_types::mode::{Mode, ModeSet};
use openmobisim_types::time::{Second, StepGrid};
use openmobisim_types::units::Duration;

/// Test values for the three build values (not defaults: the tests' hand derivations use them).
pub const VALUES: BuildValues =
    BuildValues { capacity_per_lane: 1_800.0, bike_speed: 15.0, walk_speed: 4.8 };

/// Every link mode.
pub const ALL_LINK_MODES: ModeSet = ModeSet::of(&[Mode::Car, Mode::Bike, Mode::Walk]);

/// A row for every link mode, `lanes` lanes, a car speed of `car_km_h`, no length, no capacity,
/// no signal.
#[must_use]
pub fn row(lanes: u8, car_km_h: f64) -> LinkRow {
    LinkRow {
        modes: ALL_LINK_MODES,
        lanes: Some(lanes),
        car_speed_km_h: Some(car_km_h),
        length_m: None,
        capacity_veh_h: None,
        signal_at_downstream_end: false,
    }
}

/// A row for a cycle path (bike and walk, no cars): no car field, no length, no signal.
#[must_use]
pub fn cycle_path() -> LinkRow {
    LinkRow {
        modes: ModeSet::of(&[Mode::Bike, Mode::Walk]),
        lanes: None,
        car_speed_km_h: None,
        length_m: None,
        capacity_veh_h: None,
        signal_at_downstream_end: false,
    }
}

/// A point in metres.
#[must_use]
pub fn at(x: f64, y: f64) -> Position {
    Position::Metres(Xy::new(x, y))
}

/// Nodes `a` at the origin and `b` 1 000 m east, and the link `ab` between them with `row`.
#[must_use]
pub fn two_nodes(row: LinkRow) -> NetworkBuilder {
    let mut b = NetworkBuilder::new();
    b.add_node("a", at(0.0, 0.0));
    b.add_node("b", at(1_000.0, 0.0));
    b.add_link("ab", "a", "b", row);
    b
}

/// The external id of grid node `(row, column)`. The row and the column are kept apart by `_`
/// (the network tests' `n{r}{c}` names node (1, 23) and node (12, 3) alike from 11 × 11 on).
#[must_use]
pub fn node_name(r: u32, c: u32) -> String {
    format!("n{r}_{c}")
}

/// An `n × n` grid of nodes 100 m apart in metres, with a two-way street (two links) along every
/// edge between neighbours, each with one lane at 50 km/h.
///
/// # Panics
///
/// If the build fails.
#[must_use]
pub fn grid(n: u32) -> Network {
    let mut b = NetworkBuilder::new();
    for r in 0..n {
        for c in 0..n {
            b.add_node(node_name(r, c), at(f64::from(c) * 100.0, f64::from(r) * 100.0));
        }
    }
    let mut street = |from: String, to: String| {
        b.add_link(format!("{from}-{to}"), from.clone(), to.clone(), row(1, 50.0));
        b.add_link(format!("{to}-{from}"), to, from, row(1, 50.0));
    };
    for r in 0..n {
        for c in 0..n {
            if c + 1 < n {
                street(node_name(r, c), node_name(r, c + 1));
            }
            if r + 1 < n {
                street(node_name(r, c), node_name(r + 1, c));
            }
        }
    }
    b.build(VALUES).expect("a grid builds")
}

/// Nodes `n0`, `n1`, ... on a line 1 m apart in the plane, and the links `l0` (from `n0` to
/// `n1`), `l1`, ... between them, one per row, in order. The link ids are their places in the
/// line while there are at most ten links (`l0` < `l1` < ... in sorted order).
///
/// # Panics
///
/// If the build fails or there are more than ten rows.
#[must_use]
pub fn line(rows: &[LinkRow]) -> Network {
    assert!(rows.len() <= 10, "ids follow the line's order only up to ten links");
    let mut b = NetworkBuilder::new();
    for i in 0..=rows.len() {
        b.add_node(format!("n{i}"), at(f64::from(u32::try_from(i).unwrap()), 0.0));
    }
    for (i, &r) in rows.iter().enumerate() {
        b.add_link(format!("l{i}"), format!("n{i}"), format!("n{}", i + 1), r);
    }
    b.build(VALUES).expect("a line builds")
}

/// A car row of one lane at 50 km/h, `length_m` long, with capacity `veh_h` vehicles per hour.
#[must_use]
pub fn car_link(length_m: f64, veh_h: f64) -> LinkRow {
    LinkRow { length_m: Some(length_m), capacity_veh_h: Some(veh_h), ..row(1, 50.0) }
}

/// Link `i` of a network.
#[must_use]
pub fn link(i: u32) -> LinkId {
    LinkId::new(i)
}

/// Every link of a line of `n` links, in order.
#[must_use]
pub fn whole_line(n: u32) -> Vec<LinkId> {
    (0..n).map(link).collect()
}

/// A vehicle of trip `trip`, departing `departure` whole seconds after the window's start.
#[must_use]
pub fn vehicle(trip: u32, mode: Mode, route: &[LinkId], departure: u32) -> Vehicle {
    let departure = Duration(f64::from(departure));
    Vehicle { trip: TripId::new(trip), mode, route: route.to_vec(), departure }
}

/// A window from second 0 of `length` seconds in steps of `step` seconds.
///
/// # Panics
///
/// If the window is not a whole number of steps.
#[must_use]
pub fn window(step: u32, length: u32) -> StepGrid {
    StepGrid::new(Second::ZERO, step, length).expect("a whole number of steps")
}

/// The route through grid nodes `nodes` (each a `(row, column)`), the link between each node
/// and the next.
///
/// # Panics
///
/// If two consecutive nodes are not neighbours.
#[must_use]
pub fn grid_route(net: &Network, nodes: &[(u32, u32)]) -> Vec<LinkId> {
    nodes
        .windows(2)
        .map(|w| {
            let id = format!("{}-{}", node_name(w[0].0, w[0].1), node_name(w[1].0, w[1].1));
            net.link_ids().typed_id_of(&id).expect("neighbours have a link")
        })
        .collect()
}

/// A stream of pseudo-random numbers in `[0, 1)`: splitmix64 (Steele, Lea and Flood 2014), for
/// test inputs only.
#[derive(Clone, Debug)]
pub struct Draws(u64);

impl Draws {
    /// The stream from `seed`.
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    /// The next draw in `[0, 1)`.
    pub fn unit(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^= z >> 31;
        #[allow(clippy::cast_precision_loss, reason = "the top 53 bits are exact in an f64")]
        let u = (z >> 11) as f64 / (1_u64 << 53) as f64;
        u
    }

    /// The next draw in `0..n`.
    pub fn below(&mut self, n: u32) -> u32 {
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a draw in [0, 1) times n, floored, is below n"
        )]
        let k = (self.unit() * f64::from(n)) as u32;
        k
    }
}

/// Whether `a` and `b` agree to a relative 1e-12 (a few units in the last place of a value
/// reached by one or two divisions).
#[must_use]
pub fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-12 * b.abs().max(1.0)
}
