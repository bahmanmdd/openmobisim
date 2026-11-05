//! Fixtures shared by the search tests, copied from the loading tests: build values, rows, a
//! grid, windows, vehicles, the free-flow and loaded link times, and random draws.

#![allow(dead_code, reason = "each test binary uses a different part of this module")]

use openmobisim_graph::geometry::{Position, Xy};
use openmobisim_graph::network::{BuildValues, LinkRow, Network, NetworkBuilder};
use openmobisim_loading::input::Vehicle;
use openmobisim_loading::link_times::LinkTimes;
use openmobisim_loading::point_queue::load;
use openmobisim_types::ids::{EntityId, LinkId, NodeId, TripId};
use openmobisim_types::mode::{Mode, ModeSet};
use openmobisim_types::time::{Second, StepGrid};

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

/// A car row of one lane at 50 km/h, `length_m` long, with capacity `veh_h` vehicles per hour.
#[must_use]
pub fn car_link(length_m: f64, veh_h: f64) -> LinkRow {
    LinkRow { length_m: Some(length_m), capacity_veh_h: Some(veh_h), ..row(1, 50.0) }
}

/// A point in metres.
#[must_use]
pub fn at(x: f64, y: f64) -> Position {
    Position::Metres(Xy::new(x, y))
}

/// The external id of grid node `(row, column)`.
#[must_use]
pub fn node_name(r: u32, c: u32) -> String {
    format!("n{r}_{c}")
}

/// An `n × n` grid of nodes 100 m apart in metres (row `r` at y = 100 r, column `c` at
/// x = 100 c), with a two-way street (two links, `from-to` by node names) along every edge
/// between neighbours; each link's row is `row_of(from, to)`, both `(row, column)`.
///
/// # Panics
///
/// If the build fails.
#[must_use]
pub fn grid_with(n: u32, row_of: impl Fn((u32, u32), (u32, u32)) -> LinkRow) -> Network {
    let mut b = NetworkBuilder::new();
    for r in 0..n {
        for c in 0..n {
            b.add_node(node_name(r, c), at(f64::from(c) * 100.0, f64::from(r) * 100.0));
        }
    }
    let mut street = |a: (u32, u32), z: (u32, u32)| {
        let (from, to) = (node_name(a.0, a.1), node_name(z.0, z.1));
        b.add_link(format!("{from}-{to}"), from.clone(), to.clone(), row_of(a, z));
        b.add_link(format!("{to}-{from}"), to, from, row_of(z, a));
    };
    for r in 0..n {
        for c in 0..n {
            if c + 1 < n {
                street((r, c), (r, c + 1));
            }
            if r + 1 < n {
                street((r, c), (r + 1, c));
            }
        }
    }
    b.build(VALUES).expect("a grid builds")
}

/// The loading tests' grid: every link one lane at 50 km/h, 1 800 veh/h.
#[must_use]
pub fn grid(n: u32) -> Network {
    grid_with(n, |_, _| row(1, 50.0))
}

/// Grid node `(row, column)`.
///
/// # Panics
///
/// If the network has no such node.
#[must_use]
pub fn grid_node(net: &Network, r: u32, c: u32) -> NodeId {
    net.node_ids().typed_id_of(&node_name(r, c)).expect("a grid node")
}

/// The link from grid node `a` to grid node `z`, both `(row, column)`.
///
/// # Panics
///
/// If the two are not neighbours.
#[must_use]
pub fn grid_link(net: &Network, a: (u32, u32), z: (u32, u32)) -> LinkId {
    let id = format!("{}-{}", node_name(a.0, a.1), node_name(z.0, z.1));
    net.link_ids().typed_id_of(&id).expect("neighbours have a link")
}

/// The route through grid nodes `nodes`, the link between each node and the next.
#[must_use]
pub fn grid_route(net: &Network, nodes: &[(u32, u32)]) -> Vec<LinkId> {
    nodes.windows(2).map(|w| grid_link(net, w[0], w[1])).collect()
}

/// The L-shaped route on a grid from `a` to `z`: along `a`'s row to `z`'s column, then along
/// that column; empty when `a` is `z`.
#[must_use]
pub fn l_route(net: &Network, a: (u32, u32), z: (u32, u32)) -> Vec<LinkId> {
    let step = |from: u32, to: u32| -> Vec<u32> {
        if from <= to { (from..=to).collect() } else { (to..=from).rev().collect() }
    };
    let mut nodes: Vec<(u32, u32)> = step(a.1, z.1).into_iter().map(|c| (a.0, c)).collect();
    nodes.extend(step(a.0, z.0).into_iter().skip(1).map(|r| (r, z.1)));
    grid_route(net, &nodes)
}

/// A vehicle of trip `trip`.
#[must_use]
pub fn vehicle(trip: u32, mode: Mode, route: &[LinkId], departure: u32) -> Vehicle {
    Vehicle { trip: TripId::new(trip), mode, route: route.to_vec(), departure: Second(departure) }
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

/// The link times of a loading of `vehicles` on `net` over `window`.
///
/// # Panics
///
/// If the loading refuses a vehicle.
#[must_use]
pub fn loaded(net: &Network, vehicles: &[Vehicle], window: StepGrid) -> LinkTimes {
    let run = load(net, vehicles, window).expect("the vehicles load");
    LinkTimes::new(net, &run.links)
}

/// The link times of an empty loading: free flow everywhere.
#[must_use]
pub fn free_flow(net: &Network) -> LinkTimes {
    loaded(net, &[], window(300, 3_600))
}

/// `cars` cars on random L-shaped routes across an `n × n` grid, departing at random seconds
/// in `0..seconds`, drawn from `draws`.
pub fn l_route_cars(
    net: &Network,
    n: u32,
    cars: u32,
    seconds: u32,
    draws: &mut Draws,
) -> Vec<Vehicle> {
    let mut out = Vec::new();
    while out.len() < cars as usize {
        let a = (draws.below(n), draws.below(n));
        let z = (draws.below(n), draws.below(n));
        let route = l_route(net, a, z);
        if !route.is_empty() {
            let trip = u32::try_from(out.len()).expect("few cars");
            out.push(vehicle(trip, Mode::Car, &route, draws.below(seconds)));
        }
    }
    out
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
