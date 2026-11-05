//! Fixtures shared by the network tests: build values, rows, a two-node network and a grid.

#![allow(dead_code, reason = "each test binary uses a different part of this module")]

use openmobisim_graph::geometry::{Position, Xy};
use openmobisim_graph::network::{BuildValues, LinkRow, Network, NetworkBuilder};
use openmobisim_types::mode::{Mode, ModeSet};

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

/// Whether `a` and `b` agree to a relative 1e-12 (a few units in the last place of a value
/// reached by one or two divisions).
#[must_use]
pub fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-12 * b.abs().max(1.0)
}
