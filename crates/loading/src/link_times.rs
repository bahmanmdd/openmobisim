//! What a link costs a traveller at a given instant, from the last loading.
//!
//! [`LinkTimes`] reads the recorded table ([`LinkBins`]) back as a time-dependent cost:
//! [`LinkTimes::exit`] is when a traveller entering a link at an instant leaves it,
//! [`LinkTimes::time`] the time that takes, and [`LinkTimes::route_time`] walks a route link by
//! link, entering each link when the last one ends.
//!
//! # Bikes and walkers
//!
//! Their time is the link's free-flow time for their mode, length over speed, at every instant:
//! they interact with nothing.
//!
//! # Cars: the traveller's place in the queue
//!
//! The reading is the time the traveller would take entering the link at `t`, every other car
//! as the loading recorded it. The point queue lets a car leave at `max(t + τ, x + h)`: its
//! free-flow time `τ` after it entered, and no sooner than one headway `h` (one over the
//! capacity) after the exit `x` of the car ahead of it. The reading is that expression; what
//! it settles is which recorded car is ahead ([`LinkBins`] keeps each car's entry, exit and
//! trip, in the order they entered):
//!
//! 1. the cars that entered before `t` are ahead;
//! 2. of the cars that entered at `t`, those of a lower trip are ahead: the loading's own
//!    order. A [`Reader::stranger`], who has no trip in the loading, is behind them all;
//! 3. the traveller's own recorded car is never ahead of it. Where that car is among the
//!    cars ahead, the cars after it are replayed without it, by the same expression, up to
//!    the first that leaves when it was recorded to;
//! 4. a recorded car whose place the traveller took on the link before (the traveller left
//!    that link at the very instant that car did) is behind it here, whatever the trips.
//!    [`LinkTimes::exit`] returns the reader for the next link, which carries that car.
//!
//! Before the link's first car, and on a link nobody entered by car: free flow, `t + τ`.
//! Nothing is interpolated and no bin is read. A traveller walking its own route from its own
//! departure reads the time it took, to the bit. On a route it did not take, the reading can
//! be one headway off where its own car, removed from a queue, would have been replaced by a
//! car held further upstream: only loading again with the traveller moved settles that.
//!
//! **Cost:** 32 bytes per car crossing here (entry, exit, trip, and the same crossing again
//! by trip) and 20 in the table it is built from, 8 per link per mode and 8 per link for the
//! headway. A read is two binary searches over the link's crossings and one over the
//! traveller's own; rule 3 walks the cars between the traveller's own car and its new place.

use openmobisim_graph::network::Network;
use openmobisim_types::ids::{EntityId, LinkId, TripId};
use openmobisim_types::mode::Mode;
use openmobisim_types::time::Second;
use openmobisim_types::units::Duration;

use crate::link_bins::LinkBins;

/// Who reads a link: the traveller, and the recorded car whose place it took on the link it
/// comes from (rule 4 of the module documentation).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Reader {
    trip: TripId,
    took: TripId,
}

impl Reader {
    /// The traveller of `trip`, setting out or entering a link it reached by no other link.
    #[must_use]
    pub fn of(trip: TripId) -> Self {
        Self { trip, took: TripId::NULL }
    }

    /// A traveller with no trip in the loading: behind every car entering when it does.
    #[must_use]
    pub fn stranger() -> Self {
        Self::of(TripId::NULL)
    }

    /// The same traveller on the next link, having taken the place of `took` on this one
    /// (null: of nobody).
    fn after(self, took: TripId) -> Self {
        Self { trip: self.trip, took }
    }
}

/// Link times by instant of entry, from a loading's table (see the module documentation).
#[derive(Clone, Debug)]
pub struct LinkTimes {
    origin: f64,
    /// Free-flow time per link, one column per link mode (car, bike, walk); NaN for a car on a
    /// link that does not allow cars.
    free_flow: [Vec<f64>; 3],
    /// Per link, the headway of its capacity, seconds (0 for an unlimited capacity).
    headway: Vec<f64>,
    /// Per link, its car crossings are `entry[start[l]..start[l + 1]]`, `exit[..]` and
    /// `trip[..]` alike, in the order the cars entered.
    start: Vec<u32>,
    entry: Vec<f64>,
    exit: Vec<f64>,
    trip: Vec<TripId>,
    /// Per trip, its own crossings are `own_link[own_start[trip]..own_start[trip + 1]]`, sorted
    /// by link, and `own_place[..]` each one's place among its link's crossings.
    own_start: Vec<u32>,
    own_link: Vec<u32>,
    own_place: Vec<u32>,
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
        let headway = LinkId::iter_space(links).map(|l| 1.0 / network.capacity(l).get()).collect();
        let (start, entry, exit, trip) = bins.crossings();
        let start = if start.is_empty() { vec![0; links as usize + 1] } else { start.to_vec() };
        let mut times = Self {
            origin: f64::from(bins.window().origin().get()),
            free_flow: [free(Mode::Car), free(Mode::Bike), free(Mode::Walk)],
            headway,
            start,
            entry: entry.to_vec(),
            exit: exit.to_vec(),
            trip: trip.to_vec(),
            own_start: Vec::new(),
            own_link: Vec::new(),
            own_place: Vec::new(),
        };
        debug_assert!(times.exits_never_fall(), "the loading was not first-in-first-out");
        times.index_by_trip();
        times
    }

    /// Whether, on every link, the cars entered in time order and each exit is at or after the
    /// one before it.
    fn exits_never_fall(&self) -> bool {
        self.start.windows(2).all(|w| {
            let (a, z) = (w[0] as usize, w[1] as usize);
            self.entry[a..z].windows(2).all(|e| e[0] <= e[1])
                && self.exit[a..z].windows(2).all(|x| x[0] <= x[1])
        })
    }

    /// Fill, per trip, where its own crossings are: one counting pass over the links in order,
    /// so each trip's crossings come sorted by link.
    fn index_by_trip(&mut self) {
        let trips = self.trip.iter().map(|t| t.index() + 1).max().unwrap_or(0);
        let mut from = vec![0_u32; trips + 1];
        for t in &self.trip {
            from[t.index() + 1] += 1;
        }
        for t in 0..trips {
            from[t + 1] += from[t];
        }
        let mut next = from.clone();
        self.own_link = vec![0; self.trip.len()];
        self.own_place = vec![0; self.trip.len()];
        for l in 0..self.start.len() - 1 {
            for k in self.start[l]..self.start[l + 1] {
                let at = &mut next[self.trip[k as usize].index()];
                self.own_link[*at as usize] = u32::try_from(l).expect("fewer than 2^32 links");
                self.own_place[*at as usize] = k - self.start[l];
                *at += 1;
            }
        }
        self.own_start = from;
    }

    /// The place of `trip`'s own crossing among those of link `l`, if its car crossed it (the
    /// first such crossing, for a route that crosses a link twice).
    fn own(&self, l: usize, trip: TripId) -> Option<usize> {
        let t = trip.to_option()?.index();
        let (a, z) = (*self.own_start.get(t)? as usize, *self.own_start.get(t + 1)? as usize);
        let links = &self.own_link[a..z];
        let l = u32::try_from(l).ok()?;
        let first = links.partition_point(|&x| x < l);
        (links.get(first) == Some(&l)).then(|| self.own_place[a + first] as usize)
    }

    /// When the traveller `reader`, entering `link` by `mode` at `t`, leaves it, and the reader
    /// for the link after it; instants in seconds since the window's start. Free flow for a
    /// bike or a walker; for a car, its place in the queue (see the module documentation),
    /// never before `t` plus the free-flow time.
    ///
    /// # Panics
    ///
    /// For [`Mode::Transit`], which is not on links, and for a car on a link that does not
    /// allow cars.
    #[must_use]
    pub fn exit(
        &self,
        link: LinkId,
        mode: Mode,
        t: Duration,
        reader: Reader,
    ) -> (Duration, Reader) {
        let free = self.free_flow[column(mode)][link.index()];
        if mode != Mode::Car {
            return (Duration(t.get() + free), reader.after(TripId::NULL));
        }
        assert!(!free.is_nan(), "link {link} does not allow cars");
        let l = link.index();
        let (ka, kz) = (self.start[l] as usize, self.start[l + 1] as usize);
        let (entry, exits, trips) = (&self.entry[ka..kz], &self.exit[ka..kz], &self.trip[ka..kz]);
        let (t, h) = (t.get(), self.headway[l]);
        let (ahead, behind) = places(entry, trips, t, reader);
        let own = self.own(l, reader.trip).filter(|&o| o < ahead);
        let last = match own {
            Some(o) => Some(without(entry, exits, o, ahead, free, h)),
            None => ahead.checked_sub(1).map(|a| exits[a]),
        };
        let exit = last.map_or(t + free, |x| (t + free).max(x + h));
        // The car right behind, when it left this link at the very instant the traveller now
        // does: the traveller has its place, and that car is behind it on the next link.
        #[allow(clippy::float_cmp, reason = "the same instant, to the bit: the same place")]
        let took = h > 0.0 && exits.get(behind) == Some(&exit);
        (Duration(exit), reader.after(if took { trips[behind] } else { TripId::NULL }))
    }

    /// The time `reader` takes to cross `link` by `mode` entering it at `t` (seconds since the
    /// window's start): [`Self::exit`] less `t`.
    ///
    /// # Panics
    ///
    /// As [`Self::exit`].
    #[must_use]
    pub fn time(&self, link: LinkId, mode: Mode, t: Duration, reader: Reader) -> Duration {
        self.exit(link, mode, t, reader).0 - t
    }

    /// An instant of the clock as seconds since the window's start: the clock on which
    /// [`Self::exit`] reads, and the one every walk of a route starts from.
    #[must_use]
    pub fn since_start(&self, at: Second) -> Duration {
        Duration(f64::from(at.get()) - self.origin)
    }

    /// The time `reader` takes to travel `route` by `mode`, setting out at `departure` (seconds
    /// since the window's start): the route walked link by link, each link entered when the
    /// one before it ends. Zero for an empty route.
    ///
    /// # Panics
    ///
    /// As [`Self::exit`], for any link of the route.
    #[must_use]
    pub fn route_time(
        &self,
        route: &[LinkId],
        mode: Mode,
        departure: Duration,
        reader: Reader,
    ) -> Duration {
        let (mut t, mut reader) = (departure, reader);
        for &link in route {
            (t, reader) = self.exit(link, mode, t, reader);
        }
        t - departure
    }
}

/// Among a link's crossings (`entry` and `trips`, in the order the cars entered), how many
/// cars are ahead of `reader` entering at `t`, its own crossing never among them, and the
/// place of the first recorded car behind it (rules 1, 2 and 4 of the module documentation).
fn places(entry: &[f64], trips: &[TripId], t: f64, reader: Reader) -> (usize, usize) {
    let before = entry.partition_point(|&e| e < t);
    let tied = &trips[before..before + entry[before..].partition_point(|&e| e <= t)];
    // Of the cars that entered at `t`: those recorded before the traveller's own crossing when
    // it is one of them; else those of a lower trip, less the car whose place it took.
    let mut ahead = before
        + match tied.iter().position(|&id| id == reader.trip) {
            Some(own) => own,
            None => {
                let lower = tied.partition_point(|&id| id < reader.trip);
                let took = !reader.took.is_null() && tied[..lower].contains(&reader.took);
                lower - usize::from(took)
            }
        };
    let mut behind = ahead;
    if trips.get(behind) == Some(&reader.trip) {
        behind += 1;
    }
    if ahead > 0 && trips[ahead - 1] == reader.trip {
        ahead -= 1;
    }
    (ahead, behind)
}

/// The exit of the last of the `ahead` cars ahead of the traveller, its own car at place `own`
/// among them taken out (rule 3 of the module documentation): the cars after it replayed by
/// the loading's expression, up to the first that leaves when it was recorded to, from which
/// on the record stands.
fn without(entry: &[f64], exits: &[f64], own: usize, ahead: usize, free: f64, h: f64) -> f64 {
    let mut x = if own > 0 { exits[own - 1] } else { f64::NEG_INFINITY };
    for k in own + 1..ahead {
        x = (entry[k] + free).max(x + h);
        #[allow(clippy::float_cmp, reason = "the recorded instant, to the bit")]
        if x == exits[k] {
            return exits[ahead - 1];
        }
    }
    x
}
