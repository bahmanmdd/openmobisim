//! What a link costs at a given instant, from the last loading: the first-in-first-out reading.
//!
//! [`LinkTimes`] reads the recorded table ([`LinkBins`]) back as a time-dependent cost:
//! [`LinkTimes::time`] is the time to cross a link when entering it at an instant, and
//! [`LinkTimes::route_time`] walks a route link by link, entering each link when the last one
//! ends (a time-dependent cost that is first-in-first-out can be walked this way).
//!
//! # Bikes and walkers
//!
//! Their time is the link's free-flow time for their mode, length over speed, at every instant:
//! they interact with nothing.
//!
//! # Cars: the exit read between recorded entrants
//!
//! The reading reads the **exit instant** `e(t)` of a car entering at `t`, from the first and
//! the last car to enter each link in each bin, their entry and exit instants being recorded by
//! the loading:
//!
//! - between a bin's first and last entrant, on the straight line through them (a bin with one
//!   entrant has one point);
//! - between the last entrant of bin `k` and the first of bin `k + 1`, across the boundary, on
//!   the straight line through them;
//! - after a bin's last entrant when the next bin has none (an empty bin, or past the last bin
//!   that has any): `max(x_last, t + τ_free)`, behind the last exit, at free flow at least;
//! - before a bin's first entrant when the bin before has none: `max(x_before, t + τ_first)`,
//!   behind the last exit before (if any), at the first entrant's time;
//! - before the link's first recorded bin, and on a link nobody entered by car: free flow.
//!
//! The time is `max(e(t) − t, τ_free)`, computed from differences of recorded instants
//! (`e(t) − t` itself, never `e(t)`). Every piece is nondecreasing in `t` and they meet without
//! a fall, so the reading is first-in-first-out by construction: a car entering later never
//! leaves earlier. It rests on the loading being first-in-first-out, which the point queue is
//! by construction; the exits are checked nondecreasing in debug builds, never repaired. At a
//! recorded entrant the reading is that entrant's own time.
//!
//! **Cost:** 32 bytes per (link, bin) a car entered, 8 per link per mode; a read is one binary
//! search over the link's recorded bins and one interpolation.

use openmobisim_graph::network::Network;
use openmobisim_types::ids::{EntityId, LinkId};
use openmobisim_types::mode::Mode;
use openmobisim_types::time::Second;
use openmobisim_types::units::Duration;

use crate::link_bins::LinkBins;

/// The first and the last car to enter a link in one bin: their entry and exit instants, in
/// seconds since the window's start.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Entrants {
    first_entry: f64,
    first_exit: f64,
    last_entry: f64,
    last_exit: f64,
}

/// Link times by instant of entry, from a loading's table (see the module documentation).
#[derive(Clone, Debug)]
pub struct LinkTimes {
    origin: f64,
    step: f64,
    /// Free-flow time per link, one column per link mode (car, bike, walk); NaN for a car on a
    /// link that does not allow cars.
    free_flow: [Vec<f64>; 3],
    /// Per link, its recorded car bins are `knot_bin[knot_start[l]..knot_start[l + 1]]`, in
    /// increasing order, and their entrants `knots[..]` alike.
    knot_start: Vec<u32>,
    knot_bin: Vec<u32>,
    knots: Vec<Entrants>,
}

/// The column of a link mode in [`LinkTimes::free_flow`].
fn column(mode: Mode) -> usize {
    match mode {
        Mode::Car => 0,
        Mode::Bike => 1,
        Mode::Walk => 2,
        Mode::Transit => panic!("transit is not on links: it runs on its timetable"),
    }
}

/// The time from `t` to the exit instant at `t` on the line through `(a_entry, a_exit)` and
/// `(b_entry, b_exit)`, that is the two entrants' times interpolated linearly in the entry
/// instant; to the later exit where the two entered at the same instant.
///
/// Computed from differences only, so nothing of the order of the clock is rounded twice.
fn between(a_entry: f64, a_exit: f64, b_entry: f64, b_exit: f64, t: f64) -> f64 {
    if b_entry > a_entry {
        let (a, b) = (a_exit - a_entry, b_exit - b_entry);
        a + (b - a) * ((t - a_entry) / (b_entry - a_entry))
    } else {
        a_exit.max(b_exit) - t
    }
}

impl LinkTimes {
    /// The reading of a loading's table on its network.
    ///
    /// # Panics
    ///
    /// In debug builds, if the recorded car exits on a link fall from one entrant to the next:
    /// the loading was not first-in-first-out.
    #[must_use]
    pub fn new(network: &Network, bins: &LinkBins) -> Self {
        let links = network.link_count();
        let free = |mode: Mode| -> Vec<f64> {
            LinkId::iter_space(links)
                .map(|l| {
                    if mode == Mode::Car && !network.modes(l).contains(Mode::Car) {
                        f64::NAN
                    } else {
                        network.free_flow_time(l, mode).get()
                    }
                })
                .collect()
        };
        let mut knot_start = vec![0_u32; links as usize + 1];
        let (mut knot_bin, mut knots) = (Vec::new(), Vec::new());
        for r in 0..bins.len() {
            if bins.cars()[r] == 0 {
                continue;
            }
            knot_start[bins.links()[r].index() + 1] += 1;
            knot_bin.push(bins.bins()[r].get());
            knots.push(Entrants {
                first_entry: bins.first_entry()[r].get(),
                first_exit: bins.first_exit()[r].get(),
                last_entry: bins.last_entry()[r].get(),
                last_exit: bins.last_exit()[r].get(),
            });
        }
        for l in 0..links as usize {
            knot_start[l + 1] += knot_start[l];
        }
        let times = Self {
            origin: f64::from(bins.window().origin().get()),
            step: f64::from(bins.window().step_seconds()),
            free_flow: [free(Mode::Car), free(Mode::Bike), free(Mode::Walk)],
            knot_start,
            knot_bin,
            knots,
        };
        debug_assert!(times.exits_never_fall(), "the loading was not first-in-first-out");
        times
    }

    /// Whether, on every link, each recorded exit is at or after the one recorded before it.
    fn exits_never_fall(&self) -> bool {
        self.knot_start.windows(2).all(|w| {
            let mut latest = f64::NEG_INFINITY;
            self.knots[w[0] as usize..w[1] as usize].iter().all(|k| {
                let ok = latest <= k.first_exit && k.first_exit <= k.last_exit;
                latest = k.last_exit;
                ok
            })
        })
    }

    /// The time to cross `link` by `mode` when entering it at `t`, in seconds since the window's
    /// start: free flow for a bike or a walker; for a car, the first-in-first-out reading (see
    /// the module documentation), never below free flow.
    ///
    /// # Panics
    ///
    /// For [`Mode::Transit`], which is not on links, and for a car on a link that does not
    /// allow cars.
    #[must_use]
    pub fn time(&self, link: LinkId, mode: Mode, t: Duration) -> Duration {
        let free = self.free_flow[column(mode)][link.index()];
        if mode != Mode::Car {
            return Duration(free);
        }
        assert!(!free.is_nan(), "link {link} does not allow cars");
        Duration(self.car_time(link.index(), free, t.get()).max(free))
    }

    /// The car's `e(t) − t` on link `l`, `free` its free-flow time, by the pieces of the module
    /// documentation (before the final floor at free flow).
    fn car_time(&self, l: usize, free: f64, t: f64) -> f64 {
        let (ka, kz) = (self.knot_start[l] as usize, self.knot_start[l + 1] as usize);
        if t < 0.0 {
            // Before the window: before any recorded bin.
            return free;
        }
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a non-negative instant over the step, floored; saturates far past the window"
        )]
        let bin = (t / self.step) as u32;
        // The latest recorded bin at or before `bin`.
        let Some(j) = self.knot_bin[ka..kz].partition_point(|&b| b <= bin).checked_sub(1) else {
            return free;
        };
        let (j, here) = (ka + j, self.knots[ka + j]);
        let in_here = self.knot_bin[j] == bin;
        if in_here && t < here.first_entry {
            // Before the bin's first entrant: from the last entrant of the bin before if it has
            // one, else behind the last exit before, at the first entrant's time.
            if j > ka && self.knot_bin[j - 1] + 1 == bin {
                let b = self.knots[j - 1];
                return between(b.last_entry, b.last_exit, here.first_entry, here.first_exit, t);
            }
            let floor = if j > ka { self.knots[j - 1].last_exit } else { f64::NEG_INFINITY };
            return (floor - t).max(here.first_exit - here.first_entry);
        }
        if in_here && t <= here.last_entry {
            return between(here.first_entry, here.first_exit, here.last_entry, here.last_exit, t);
        }
        // After the bin's last entrant: on to the next bin's first if the next bin has one,
        // else behind the last exit, at free flow at least.
        match self.knots[..kz].get(j + 1) {
            Some(next) if in_here && self.knot_bin[j + 1] == bin + 1 => {
                between(here.last_entry, here.last_exit, next.first_entry, next.first_exit, t)
            }
            _ => (here.last_exit - t).max(free),
        }
    }

    /// An instant of the clock as seconds since the window's start: the clock on which
    /// [`Self::time`] reads, and the one every walk of a route starts from.
    #[must_use]
    pub fn since_start(&self, at: Second) -> Duration {
        Duration(f64::from(at.get()) - self.origin)
    }

    /// The time to travel `route` by `mode`, setting out at `departure`: the route walked link
    /// by link, each link entered when the one before it ends. Zero for an empty route.
    ///
    /// # Panics
    ///
    /// As [`Self::time`], for any link of the route.
    #[must_use]
    pub fn route_time(&self, route: &[LinkId], mode: Mode, departure: Second) -> Duration {
        let start = self.since_start(departure).get();
        let mut t = start;
        for &link in route {
            t += self.time(link, mode, Duration(t)).get();
        }
        Duration(t - start)
    }
}
