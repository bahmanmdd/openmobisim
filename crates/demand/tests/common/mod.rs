//! Fixtures shared by the demand tests: build values, link rows, grids, trips, timetables of
//! stops, and random draws.

#![allow(dead_code, reason = "each test binary uses a different part of this module")]

use openmobisim_demand::input::TripRow;
use openmobisim_graph::geometry::{Position, Xy};
use openmobisim_graph::network::{BuildValues, LinkRow, Network, NetworkBuilder};
use openmobisim_transit::timetable::{Timetable, TimetableBuilder};
use openmobisim_types::mode::{Mode, ModeSet};
use openmobisim_types::time::Second;
use openmobisim_types::units::Metres;

/// Test values for the three build values (the tests' hand derivations use them: a walker at
/// 4.8 km/h covers 1 000 m in 750 s).
pub const VALUES: BuildValues =
    BuildValues { capacity_per_lane: 1_800.0, bike_speed: 15.0, walk_speed: 4.8 };

/// A walker's speed at 4.8 km/h, in metres per second.
pub const WALK_M_S: f64 = 4.8 / 3.6;

/// A street: car, bike and walk, one lane at 50 km/h.
pub const STREET: LinkRow = LinkRow {
    modes: ModeSet::of(&[Mode::Car, Mode::Bike, Mode::Walk]),
    lanes: Some(1),
    car_speed_km_h: Some(50.0),
    length_m: None,
    capacity_veh_h: None,
    signal_at_downstream_end: false,
};

/// A road for cars and bikes only, closed to walkers.
pub const NO_WALK: LinkRow = LinkRow { modes: ModeSet::of(&[Mode::Car, Mode::Bike]), ..STREET };

/// A motorway: cars only.
pub const CARS_ONLY: LinkRow = LinkRow { modes: ModeSet::of(&[Mode::Car]), ..STREET };

/// A cycle path: bike and walk, no car field.
pub const CYCLE_PATH: LinkRow = LinkRow {
    modes: ModeSet::of(&[Mode::Bike, Mode::Walk]),
    lanes: None,
    car_speed_km_h: None,
    length_m: None,
    capacity_veh_h: None,
    signal_at_downstream_end: false,
};

/// A footway: walk only, no car field.
pub const FOOTWAY: LinkRow = LinkRow { modes: ModeSet::of(&[Mode::Walk]), ..CYCLE_PATH };

/// A point in metres.
#[must_use]
pub fn at(x: f64, y: f64) -> Position {
    Position::Metres(Xy::new(x, y))
}

/// The external id of grid node `(row, column)`, zero-padded so that sorted ids follow rows
/// then columns.
#[must_use]
pub fn node_name(r: u32, c: u32) -> String {
    format!("n{r:04}_{c:04}")
}

/// A `rows × cols` grid of nodes 100 m apart moved by `jitter(row, column)` metres, with a
/// two-way street along every edge between neighbours, each link's row `row_of(from, to)`.
///
/// # Panics
///
/// If the build fails.
#[must_use]
pub fn grid(
    (rows, cols): (u32, u32),
    jitter: impl Fn(u32, u32) -> (f64, f64),
    row_of: impl Fn((u32, u32), (u32, u32)) -> LinkRow,
) -> Network {
    let mut b = NetworkBuilder::new();
    for r in 0..rows {
        for c in 0..cols {
            let (dx, dy) = jitter(r, c);
            b.add_node(node_name(r, c), at(f64::from(c) * 100.0 + dx, f64::from(r) * 100.0 + dy));
        }
    }
    let mut street = |a: (u32, u32), z: (u32, u32)| {
        let (from, to) = (node_name(a.0, a.1), node_name(z.0, z.1));
        b.add_link(format!("{from}-{to}"), from.clone(), to.clone(), row_of(a, z));
        b.add_link(format!("{to}-{from}"), to, from, row_of(z, a));
    };
    for r in 0..rows {
        for c in 0..cols {
            if c + 1 < cols {
                street((r, c), (r, c + 1));
            }
            if r + 1 < rows {
                street((r, c), (r + 1, c));
            }
        }
    }
    b.build(VALUES).expect("a grid builds")
}

/// A trip of class `c` departing at second 0.
#[must_use]
pub fn trip(id: &str, origin: Position, destination: Position, modes: &[Mode]) -> TripRow {
    TripRow {
        id: id.to_owned(),
        origin,
        destination,
        departure: Second(0),
        class: "c".to_owned(),
        modes: ModeSet::of(modes),
    }
}

/// The one class `c`.
#[must_use]
pub fn one_class() -> Vec<String> {
    vec!["c".to_owned()]
}

/// A timetable of stops only, each `(id, x, y)` in metres: no run is needed to be linked.
///
/// # Panics
///
/// If the build fails.
#[must_use]
pub fn stops(list: &[(&str, f64, f64)]) -> Timetable {
    let mut b = TimetableBuilder::new();
    for &(id, x, y) in list {
        b.add_stop(id, (Metres(x), Metres(y)));
    }
    b.build().expect("stops build")
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

    /// The next draw in `[a, b)`.
    pub fn between(&mut self, a: f64, b: f64) -> f64 {
        a + (b - a) * self.unit()
    }
}

/// A city for the determinism and run-time tests: a `rows × cols` grid of streets 100 m apart,
/// each node moved by up to 20 m, every seventh street closed to walkers one way; `stop_count`
/// stops and `trip_count` trips at random points over it, the trips' mode sets drawn among
/// car, bike, walk and transit (each mode with probability one half, at least one), departing
/// over two hours, in two classes.
#[must_use]
pub fn city(
    (rows, cols): (u32, u32),
    stop_count: u32,
    trip_count: u32,
    seed: u64,
) -> (Network, Timetable, Vec<TripRow>, Vec<String>) {
    let mut draws = Draws::new(seed);
    let jitter: Vec<(f64, f64)> = (0..rows * cols)
        .map(|_| (draws.between(-20.0, 20.0), draws.between(-20.0, 20.0)))
        .collect();
    let net = grid(
        (rows, cols),
        |r, c| jitter[(r * cols + c) as usize],
        |a, z| {
            if (a.0 * 31 + a.1 * 17 + z.0 * 7 + z.1) % 7 == 0 && a < z { NO_WALK } else { STREET }
        },
    );
    let (w, h) = (f64::from(cols) * 100.0, f64::from(rows) * 100.0);
    let mut b = TimetableBuilder::new();
    for s in 0..stop_count {
        b.add_stop(
            format!("s{s:04}"),
            (Metres(draws.between(0.0, w)), Metres(draws.between(0.0, h))),
        );
    }
    let timetable = b.build().expect("stops build");
    let point = |d: &mut Draws| at(d.between(-50.0, w), d.between(-50.0, h));
    let trips = (0..trip_count)
        .map(|i| {
            let mut modes = ModeSet::EMPTY;
            while modes.is_empty() {
                for m in Mode::ALL {
                    if draws.unit() < 0.5 {
                        modes = modes.with(m);
                    }
                }
            }
            TripRow {
                id: format!("trip{i:06}"),
                origin: point(&mut draws),
                destination: point(&mut draws),
                departure: Second(draws.below(7_200)),
                class: if i % 3 == 0 { "b".to_owned() } else { "a".to_owned() },
                modes,
            }
        })
        .collect();
    (net, timetable, trips, vec!["a".to_owned(), "b".to_owned()])
}
