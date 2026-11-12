//! The loading: cars on the point queue, bikes and walkers at free flow, over one window.
//!
//! # Cars: the point queue (Vickrey 1969)
//!
//! A car entering link `a` at `t` leaves it at `x = max(t + τ_a, x_prev + h_a)`: `τ_a` is the
//! link's car free-flow time (length over speed), `x_prev` the exit of the car that entered `a`
//! before it, and `h_a = 1 / capacity_a` the headway its downstream end passes cars at (0 for an
//! infinite capacity). The car enters the next link of its route at `x`. Storage is infinite:
//! nothing downstream holds a car, and there is no node rule, merge rule or signal. Exits are
//! fractional, never floored, and the loading is first-in-first-out on every link by
//! construction (checked in debug builds).
//!
//! # The order of events
//!
//! An event is a car reaching a link's downstream end (its departure counts as reaching its
//! origin). Events run in the order of (instant, trip id), the earliest first, from a binary
//! heap compared with `f64::total_cmp`; a car's next event is due at its exit, computed when it
//! enters. Two cars entering one link at the same instant enter it in trip order.
//!
//! # The window
//!
//! Events run while their instant is before the window's end. A car entering a link before the
//! end has its exit computed then, and that crossing is recorded with its exact exit even when
//! the exit is past the end. A trip is **finished** when it arrives (leaves its last link)
//! before the end, else **unfinished**, reported with the link it is on and when it entered it.
//! No crossing starts at or after the end.
//!
//! # Bikes and walkers
//!
//! At free flow, their mode's speed on every link, interacting with nothing: a bike enters each
//! link of its route when it leaves the one before, after the link's length over the bike
//! speed. Their results are the same, to the bit, whether there are cars or not.
//!
//! # The clock
//!
//! An instant is `f64` seconds since the window's start, a [`Duration`]: a departure, an entry
//! and an arrival alike, none of them rounded to a whole second.

use core::cmp::{Ordering, Reverse};
use std::collections::BinaryHeap;
use std::collections::binary_heap::PeekMut;

use openmobisim_graph::network::Network;
use openmobisim_types::ids::{EntityId, LinkId, TripId};
use openmobisim_types::mode::Mode;
use openmobisim_types::time::StepGrid;
use openmobisim_types::units::Duration;

use crate::input::{LoadError, Vehicle, check};
use crate::link_bins::{LinkBins, Recorder};

/// What a loading produced: the finished trips, the unfinished ones, and the table of what
/// entered each link in each bin.
#[derive(Clone, Debug)]
pub struct Loading {
    /// The trips that arrived before the window's end.
    pub finished: Finished,
    /// The trips still travelling at the window's end.
    pub unfinished: Unfinished,
    /// Per link and bin of entry, what entered.
    pub links: LinkBins,
}

/// The trips that arrived before the window's end, sorted by trip.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Finished {
    trip: Vec<TripId>,
    mode: Vec<Mode>,
    departure: Vec<Duration>,
    arrival: Vec<Duration>,
}

impl Finished {
    /// How many trips.
    #[must_use]
    pub fn len(&self) -> usize {
        self.trip.len()
    }

    /// Whether no trip finished.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.trip.is_empty()
    }

    /// Each trip.
    #[must_use]
    pub fn trips(&self) -> &[TripId] {
        &self.trip
    }

    /// Each trip's mode.
    #[must_use]
    pub fn modes(&self) -> &[Mode] {
        &self.mode
    }

    /// Each trip's departure, in seconds since the window's start.
    #[must_use]
    pub fn departures(&self) -> &[Duration] {
        &self.departure
    }

    /// Each trip's arrival (when it left its last link), in seconds since the window's start.
    #[must_use]
    pub fn arrivals(&self) -> &[Duration] {
        &self.arrival
    }

    /// Row `i`'s travel time, door to door: its arrival less its departure.
    ///
    /// # Panics
    ///
    /// If `i` is out of range.
    #[must_use]
    pub fn travel_time(&self, i: usize) -> Duration {
        self.arrival[i] - self.departure[i]
    }
}

/// The trips still travelling at the window's end, sorted by trip.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Unfinished {
    trip: Vec<TripId>,
    mode: Vec<Mode>,
    departure: Vec<Duration>,
    link: Vec<LinkId>,
    entered: Vec<Duration>,
}

impl Unfinished {
    /// How many trips.
    #[must_use]
    pub fn len(&self) -> usize {
        self.trip.len()
    }

    /// Whether every trip finished.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.trip.is_empty()
    }

    /// Each trip.
    #[must_use]
    pub fn trips(&self) -> &[TripId] {
        &self.trip
    }

    /// Each trip's mode.
    #[must_use]
    pub fn modes(&self) -> &[Mode] {
        &self.mode
    }

    /// Each trip's departure, in seconds since the window's start.
    #[must_use]
    pub fn departures(&self) -> &[Duration] {
        &self.departure
    }

    /// The link each trip is on at the window's end.
    #[must_use]
    pub fn links(&self) -> &[LinkId] {
        &self.link
    }

    /// When each trip entered that link, in seconds since the window's start.
    #[must_use]
    pub fn entered(&self) -> &[Duration] {
        &self.entered
    }
}

/// Where a trip ended up.
#[derive(Clone, Copy, Debug)]
enum Outcome {
    /// It arrived at this instant, before the end.
    Arrived(f64),
    /// It was on this link at the end, entered at this instant.
    OnLink(LinkId, f64),
}

/// A car reaching the downstream end of a link (or its origin) at `time`; `rank` is the car's
/// place in trip order, so ordering by rank is ordering by trip.
#[derive(Clone, Copy, Debug)]
struct Event {
    time: f64,
    rank: u32,
}

impl PartialEq for Event {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Event {}

impl PartialOrd for Event {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Event {
    fn cmp(&self, other: &Self) -> Ordering {
        self.time.total_cmp(&other.time).then(self.rank.cmp(&other.rank))
    }
}

/// Load `vehicles` on `network` over `window`: cars on the point queue, bikes and walkers at
/// free flow. The window's step is the length of a bin of the table.
///
/// # Errors
///
/// A [`LoadError`] naming the first refused trip, in trip order: a trip given twice, transit,
/// an empty route, a departure outside the window, a link the network does not have, a link
/// that forbids the mode, or two links that do not follow one another.
pub fn load(
    network: &Network,
    vehicles: &[Vehicle],
    window: StepGrid,
) -> Result<Loading, LoadError> {
    let order = check(network, vehicles, window)?;
    let end = f64::from(window.end().get() - window.origin().get());
    let mut recorder = Recorder::new(network.link_count(), window);
    let mut outcome = vec![Outcome::Arrived(f64::NAN); order.len()];
    run_cars(network, vehicles, &order, end, &mut recorder, &mut outcome);
    for (rank, &i) in order.iter().enumerate() {
        let v = &vehicles[i as usize];
        if v.mode != Mode::Car {
            outcome[rank] = free_flow(network, v, v.departure.get(), end, &mut recorder);
        }
    }

    let mut finished = Finished::default();
    let mut unfinished = Unfinished::default();
    for (rank, &i) in order.iter().enumerate() {
        let v = &vehicles[i as usize];
        match outcome[rank] {
            Outcome::Arrived(at) => {
                finished.trip.push(v.trip);
                finished.mode.push(v.mode);
                finished.departure.push(v.departure);
                finished.arrival.push(Duration(at));
            }
            Outcome::OnLink(link, entered) => {
                unfinished.trip.push(v.trip);
                unfinished.mode.push(v.mode);
                unfinished.departure.push(v.departure);
                unfinished.link.push(link);
                unfinished.entered.push(Duration(entered));
            }
        }
    }
    Ok(Loading { finished, unfinished, links: recorder.finish() })
}

/// The point queue of every link: its car free-flow time, its headway, and the exit of the
/// last car to enter it.
struct Queues {
    free_flow: Vec<f64>,
    headway: Vec<f64>,
    last_exit: Vec<f64>,
}

impl Queues {
    fn new(network: &Network) -> Self {
        let links = LinkId::iter_space(network.link_count());
        let (mut free_flow, mut headway) = (Vec::new(), Vec::new());
        for l in links {
            let car = network.modes(l).contains(Mode::Car);
            free_flow.push(if car { network.free_flow_time(l, Mode::Car).get() } else { f64::NAN });
            // An infinite capacity passes cars with no headway: 1 / ∞ = 0.
            headway.push(1.0 / network.capacity(l).get());
        }
        let last_exit = vec![f64::NEG_INFINITY; free_flow.len()];
        Self { free_flow, headway, last_exit }
    }

    /// A car enters `link` at `t`: its exit, `max(t + τ, x_prev + h)`.
    #[inline]
    fn enter(&mut self, link: LinkId, t: f64) -> f64 {
        let l = link.index();
        let exit = (t + self.free_flow[l]).max(self.last_exit[l] + self.headway[l]);
        debug_assert!(exit >= self.last_exit[l], "the point queue is first-in-first-out");
        self.last_exit[l] = exit;
        exit
    }
}

/// Where a car is: the place in its route of the link it is on ([`NOT_DEPARTED`] before its
/// departure) and when it entered it.
#[derive(Clone, Copy)]
struct Car {
    at: u32,
    entered: f64,
}

/// A car's place in its route before it departs.
const NOT_DEPARTED: u32 = u32::MAX;

/// Run the cars' events while they are due before `end`; fill each car's outcome.
fn run_cars(
    network: &Network,
    vehicles: &[Vehicle],
    order: &[u32],
    end: f64,
    recorder: &mut Recorder,
    outcome: &mut [Outcome],
) {
    let mut queues = Queues::new(network);
    let mut cars = vec![Car { at: NOT_DEPARTED, entered: f64::NAN }; order.len()];
    let mut heap = BinaryHeap::new();
    for (rank, &i) in order.iter().enumerate() {
        let v = &vehicles[i as usize];
        if v.mode == Mode::Car {
            let rank = u32::try_from(rank).expect("at most 2^32 vehicles");
            heap.push(Reverse(Event { time: v.departure.get(), rank }));
        }
    }
    // The earliest event is handled in place: replaced by the car's next one (one sift down),
    // or removed when the car arrives.
    while let Some(mut top) = heap.peek_mut() {
        let Reverse(Event { time, rank }) = *top;
        if time >= end {
            break;
        }
        let r = rank as usize;
        let vehicle = &vehicles[order[r] as usize];
        let route = &vehicle.route;
        let car = &mut cars[r];
        let next = if car.at == NOT_DEPARTED { 0 } else { car.at as usize + 1 };
        let Some(&link) = route.get(next) else {
            outcome[r] = Outcome::Arrived(time);
            PeekMut::pop(top);
            continue;
        };
        let exit = queues.enter(link, time);
        recorder.car(link, time, exit, vehicle.trip);
        *car = Car {
            at: u32::try_from(next).expect("routes of fewer than 2^32 links"),
            entered: time,
        };
        *top = Reverse(Event { time: exit, rank });
    }
    for Reverse(Event { rank, .. }) in heap {
        let r = rank as usize;
        let car = cars[r];
        let route = &vehicles[order[r] as usize].route;
        outcome[r] = Outcome::OnLink(route[car.at as usize], car.entered);
    }
}

/// A bike's or a walker's trip at free flow, setting out at `t` (seconds since the window's
/// start): each link entered when the one before it ends, recorded at entry.
fn free_flow(network: &Network, v: &Vehicle, mut t: f64, end: f64, rec: &mut Recorder) -> Outcome {
    for &link in &v.route {
        rec.active(link, t, v.mode);
        let exit = t + network.free_flow_time(link, v.mode).get();
        if exit >= end {
            return Outcome::OnLink(link, t);
        }
        t = exit;
    }
    Outcome::Arrived(t)
}
