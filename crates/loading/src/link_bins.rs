//! Per link and per time bin of entry, what entered the link: recorded as the loading runs.
//!
//! [`LinkBins`] is that table, sparse (only a (link, bin) that someone entered has a row) and
//! held as a structure of arrays, one column per quantity, sorted by link then bin. A bin is a
//! step of the window ([`StepGrid`]); its length is the window's step.
//!
//! # Filed by entry, with the exact exit
//!
//! A crossing is filed under the bin its vehicle **entered** the link in, when it enters: a
//! traveller about to enter a link needs the time taken by those who entered when they would.
//! The point queue knows a car's exit the moment it enters, so the crossing is filed with that
//! exact exit, even when the exit falls after the window's end: a queue that outlasts the
//! window is recorded at its full length, never cut short at the end. Nothing entering at or
//! after the end is recorded.
//!
//! # What a row holds
//!
//! - **Cars:** how many entered, the sum of their crossing times (exit minus entry), and the
//!   first and the last car to enter (each its entry and exit). Of two cars entering at the
//!   same instant, the one with the lower trip id entered first.
//! - **Bikes and walkers:** how many of each entered. Their time is their free-flow time by
//!   definition, so it is not recorded.
//!
//! A row nobody entered by car has 0 cars, a time sum of 0 and NaN for the four entrant
//! instants (there is no such car).
//!
//! # Every car's crossing, for the reading
//!
//! Beside the rows the table keeps, per link, every car that entered it: its entry, its exit
//! and its trip, in the order they entered. The rows are a summary by bin for the reader of a
//! run; the reading ([`crate::link_times::LinkTimes`]) reads the crossings, so what a link
//! costs at an instant does not depend on where the bins fall. 20 bytes per crossing.

use openmobisim_types::ids::{EntityId, LinkId, TripId};
use openmobisim_types::mode::Mode;
use openmobisim_types::time::{StepGrid, StepIndex};
use openmobisim_types::units::Duration;

/// The recorded table: one row per (link, bin of entry) that someone entered, sorted by link
/// then bin. Instants are seconds since the window's start.
/// Not `PartialEq`: a row without cars holds NaN, which equals nothing; compare bit by bit.
#[derive(Clone, Debug)]
pub struct LinkBins {
    window: StepGrid,
    link: Vec<LinkId>,
    bin: Vec<StepIndex>,
    cars: Vec<u32>,
    car_time: Vec<Duration>,
    first_entry: Vec<Duration>,
    first_exit: Vec<Duration>,
    last_entry: Vec<Duration>,
    last_exit: Vec<Duration>,
    bikes: Vec<u32>,
    walkers: Vec<u32>,
    /// Per link, its car crossings are `crossing_entry[crossing_start[l]..crossing_start[l + 1]]`,
    /// `crossing_exit[..]` and `crossing_trip[..]` alike, in the order the cars entered the link.
    crossing_start: Vec<u32>,
    crossing_entry: Vec<f64>,
    crossing_exit: Vec<f64>,
    crossing_trip: Vec<TripId>,
}

impl LinkBins {
    /// Every car crossing: per link the offsets, then the entries and the exits, in seconds
    /// since the window's start, and the trips, in the order the cars entered each link. Link
    /// `l`'s crossings are `entries[offsets[l]..offsets[l + 1]]`, the exits and trips alike;
    /// the offsets are empty in a table no loading filled.
    #[must_use]
    pub fn crossings(&self) -> (&[u32], &[f64], &[f64], &[TripId]) {
        (&self.crossing_start, &self.crossing_entry, &self.crossing_exit, &self.crossing_trip)
    }

    /// The window the bins divide: a bin is one of its steps.
    #[must_use]
    pub fn window(&self) -> StepGrid {
        self.window
    }

    /// How many rows.
    #[must_use]
    pub fn len(&self) -> usize {
        self.link.len()
    }

    /// Whether nobody entered any link.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.link.is_empty()
    }

    /// Each row's link.
    #[must_use]
    pub fn links(&self) -> &[LinkId] {
        &self.link
    }

    /// Each row's bin of entry: step `i` of the window.
    #[must_use]
    pub fn bins(&self) -> &[StepIndex] {
        &self.bin
    }

    /// Each row's count of cars that entered.
    #[must_use]
    pub fn cars(&self) -> &[u32] {
        &self.cars
    }

    /// Each row's sum of car crossing times (exit minus entry), in seconds. Divided by
    /// [`Self::cars`], the mean time a car entering in that bin took.
    #[must_use]
    pub fn car_time(&self) -> &[Duration] {
        &self.car_time
    }

    /// Each row's first car to enter: when it entered, in seconds since the window's start.
    /// NaN in a row without cars.
    #[must_use]
    pub fn first_entry(&self) -> &[Duration] {
        &self.first_entry
    }

    /// Each row's first car to enter: when it left. NaN in a row without cars.
    #[must_use]
    pub fn first_exit(&self) -> &[Duration] {
        &self.first_exit
    }

    /// Each row's last car to enter: when it entered. NaN in a row without cars.
    #[must_use]
    pub fn last_entry(&self) -> &[Duration] {
        &self.last_entry
    }

    /// Each row's last car to enter: when it left. NaN in a row without cars.
    #[must_use]
    pub fn last_exit(&self) -> &[Duration] {
        &self.last_exit
    }

    /// Each row's count of bikes that entered.
    #[must_use]
    pub fn bikes(&self) -> &[u32] {
        &self.bikes
    }

    /// Each row's count of walkers that entered.
    #[must_use]
    pub fn walkers(&self) -> &[u32] {
        &self.walkers
    }

    /// An empty table over `window`.
    fn empty(window: StepGrid) -> Self {
        Self {
            window,
            link: Vec::new(),
            bin: Vec::new(),
            cars: Vec::new(),
            car_time: Vec::new(),
            first_entry: Vec::new(),
            first_exit: Vec::new(),
            last_entry: Vec::new(),
            last_exit: Vec::new(),
            bikes: Vec::new(),
            walkers: Vec::new(),
            crossing_start: Vec::new(),
            crossing_entry: Vec::new(),
            crossing_exit: Vec::new(),
            crossing_trip: Vec::new(),
        }
    }

    /// Append a row (rows come in link, then bin, order).
    fn push(&mut self, link: u32, bin: u32, car: Option<&CarRow>, bikes: u32, walkers: u32) {
        let nan = Duration(f64::NAN);
        self.link.push(LinkId::new(link));
        self.bin.push(StepIndex(bin));
        self.cars.push(car.map_or(0, |c| c.count));
        self.car_time.push(Duration(car.map_or(0.0, |c| c.time)));
        self.first_entry.push(car.map_or(nan, |c| Duration(c.first_entry)));
        self.first_exit.push(car.map_or(nan, |c| Duration(c.first_exit)));
        self.last_entry.push(car.map_or(nan, |c| Duration(c.last_entry)));
        self.last_exit.push(car.map_or(nan, |c| Duration(c.last_exit)));
        self.bikes.push(bikes);
        self.walkers.push(walkers);
    }
}

/// The cars of one (link, bin) so far.
#[derive(Clone, Copy, Debug)]
struct CarRow {
    link: u32,
    bin: u32,
    count: u32,
    time: f64,
    first_entry: f64,
    first_exit: f64,
    last_entry: f64,
    last_exit: f64,
}

/// No row yet: a link's note in [`Recorder::current`].
const NO_ROW: u32 = u32::MAX;

/// Builds a [`LinkBins`] as the loading runs. Cars are fed in the order they enter each link
/// (the point queue's order); bikes and walkers in any order.
#[derive(Debug)]
pub(crate) struct Recorder {
    window: StepGrid,
    step: f64,
    /// Per link, the place of its latest car row in `car_rows` ([`NO_ROW`]: none yet).
    current: Vec<u32>,
    car_rows: Vec<CarRow>,
    /// One `(link, entry, exit, trip)` per car crossing, in the order fed.
    crossings: Vec<(u32, f64, f64, TripId)>,
    /// One `(link, bin, walker?)` per bike or walker crossing, counted when finished.
    active: Vec<(u32, u32, bool)>,
}

impl Recorder {
    /// A recorder over `link_count` links and the bins of `window`.
    pub(crate) fn new(link_count: u32, window: StepGrid) -> Self {
        Self {
            window,
            step: f64::from(window.step_seconds()),
            current: vec![NO_ROW; link_count as usize],
            car_rows: Vec::new(),
            crossings: Vec::new(),
            active: Vec::new(),
        }
    }

    /// The bin of an entry at `entry` seconds since the window's start, inside the window.
    fn bin_of(&self, entry: f64) -> u32 {
        debug_assert!(entry >= 0.0, "an entry inside the window");
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "an instant inside the window over the step: below the u32 step count"
        )]
        let bin = (entry / self.step) as u32;
        debug_assert!(bin < self.window.n_steps(), "an entry inside the window");
        bin
    }

    /// The car of `trip` entered `link` at `entry` and leaves it at `exit` (seconds since the
    /// window's start). Calls for one link come in the order the cars enter it.
    pub(crate) fn car(&mut self, link: LinkId, entry: f64, exit: f64, trip: TripId) {
        let bin = self.bin_of(entry);
        let l = link.index();
        self.crossings.push((link.raw(), entry, exit, trip));
        let at = self.current[l];
        if at != NO_ROW && self.car_rows[at as usize].bin == bin {
            let row = &mut self.car_rows[at as usize];
            debug_assert!(entry >= row.last_entry, "cars enter a link in time order");
            row.count += 1;
            row.time += exit - entry;
            row.last_entry = entry;
            row.last_exit = exit;
        } else {
            debug_assert!(at == NO_ROW || self.car_rows[at as usize].bin < bin);
            self.current[l] = u32::try_from(self.car_rows.len()).expect("fewer than 2^32 car rows");
            self.car_rows.push(CarRow {
                link: link.raw(),
                bin,
                count: 1,
                time: exit - entry,
                first_entry: entry,
                first_exit: exit,
                last_entry: entry,
                last_exit: exit,
            });
        }
    }

    /// A bike or a walker entered `link` at `entry` seconds since the window's start.
    pub(crate) fn active(&mut self, link: LinkId, entry: f64, mode: Mode) {
        debug_assert!(matches!(mode, Mode::Bike | Mode::Walk));
        let bin = self.bin_of(entry);
        self.active.push((link.raw(), bin, mode == Mode::Walk));
    }

    /// The finished table, sorted by link then bin.
    pub(crate) fn finish(mut self) -> LinkBins {
        self.car_rows.sort_unstable_by_key(|r| (r.link, r.bin));
        // The crossings sorted by link in one counting pass (their links are dense), then each
        // link's few by bin: the same order as one comparison sort of them all.
        let mut from = vec![0_usize; self.current.len() + 1];
        for &(l, _, _) in &self.active {
            from[l as usize + 1] += 1;
        }
        for l in 0..self.current.len() {
            from[l + 1] += from[l];
        }
        let mut sorted = vec![(0, 0, false); self.active.len()];
        let mut next = from.clone();
        for &crossing in &self.active {
            sorted[next[crossing.0 as usize]] = crossing;
            next[crossing.0 as usize] += 1;
        }
        for l in 0..self.current.len() {
            sorted[from[l]..from[l + 1]].sort_unstable();
        }
        self.active = sorted;
        let mut out = LinkBins::empty(self.window);
        let (mut c, mut a) = (0, 0);
        while c < self.car_rows.len() || a < self.active.len() {
            let car_key = self.car_rows.get(c).map(|r| (r.link, r.bin));
            let active_key = self.active.get(a).map(|&(l, b, _)| (l, b));
            let key = match (car_key, active_key) {
                (Some(x), Some(y)) => x.min(y),
                (Some(x), None) => x,
                (None, Some(y)) => y,
                (None, None) => unreachable!("the loop runs while either has rows"),
            };
            let car = self.car_rows.get(c).filter(|r| (r.link, r.bin) == key);
            if car.is_some() {
                c += 1;
            }
            let (mut bikes, mut walkers) = (0, 0);
            while let Some(&(l, b, walker)) = self.active.get(a) {
                if (l, b) != key {
                    break;
                }
                if walker {
                    walkers += 1;
                } else {
                    bikes += 1;
                }
                a += 1;
            }
            out.push(key.0, key.1, car, bikes, walkers);
        }
        // The crossings by link in one counting pass, each link's in the order they were fed.
        let mut start = vec![0_u32; self.current.len() + 1];
        for &(l, ..) in &self.crossings {
            start[l as usize + 1] += 1;
        }
        for l in 0..self.current.len() {
            start[l + 1] = start[l].checked_add(start[l + 1]).expect("fewer than 2^32 crossings");
        }
        let mut next = start.clone();
        out.crossing_entry = vec![0.0; self.crossings.len()];
        out.crossing_exit = vec![0.0; self.crossings.len()];
        out.crossing_trip = vec![TripId::NULL; self.crossings.len()];
        for &(l, entry, exit, trip) in &self.crossings {
            let at = next[l as usize] as usize;
            next[l as usize] += 1;
            (out.crossing_entry[at], out.crossing_exit[at]) = (entry, exit);
            out.crossing_trip[at] = trip;
        }
        out.crossing_start = start;
        out
    }
}
