//! The loading's event queue: at most one event per queue, in a four-ary heap indexed by
//! queue.
//!
//! Rescheduling a queue moves its event in the heap instead of leaving the old one behind
//! to be skipped, so the heap holds only live events.
//!
//! **The order of events** is a total order that depends neither on the order in which events
//! were scheduled, nor on the hour of the day, nor on the ids of links and nodes:
//!
//! 1. **the time** the event is due, in seconds since the loading's origin (the window's start),
//!    on a grid of [`TICKS_PER_SECOND`] steps per second ([`on_grid_up`]). The loading keeps
//!    every duration it adds to a time on the same grid ([`on_grid_up`]), so every time it computes
//!    is exact, whatever the order of the additions that made it: two events due at the same
//!    instant are due at the same number;
//! 2. at the same instant, **the vehicles at a link's downstream end before every other event**
//!    (a departure waiting at its origin, a vehicle leaving a stop line, room reaching a link's
//!    upstream end, a re-routing timer); among the vehicles at links' downstream ends, **the
//!    service tag** of weighted fair queueing, the loading's merge rule (its module
//!    documentation); among the other events, the instant each became due;
//! 3. **the vehicle that has waited longest**: the earliest instant it reached the place where
//!    it waits (the downstream end of its link, its origin, its stop line);
//! 4. **the vehicle's id**, the lower first: a label of the traveller, which relabelling the
//!    network does not change;
//! 5. the queue's index, which decides only between events that carry no vehicle (room reaching
//!    the upstream end of a link that nobody straddles).
//!
//! **Why four-ary, on integer keys (S294).** A loading serves millions of events from a heap
//! of about a thousand, so the time goes on comparisons whose outcome no branch predictor can
//! guess. The heap holds each event's time and rank as integers that sort as [`f64::total_cmp`]
//! sorts the floats, compares them branching only on a tie of times (rare, so predicted), and
//! has four children per node, half the levels of a binary heap. Any heap serves the same
//! events in the same order: the order is total and no two events share a queue, so the
//! earliest live event is the same whatever the heap's shape.

/// Steps of the loading's time grid per second: 2^20, about a microsecond. A time on the grid is
/// a whole number of steps, so a sum of such times below 2^33 s (272 years) is exact in floating
/// point.
pub(crate) const TICKS_PER_SECOND: f64 = 1_048_576.0;

/// `x` seconds rounded up to the next step of the time grid (itself if on it): how the loading
/// keeps a duration it adds to times (a free-flow time, a delay, a headway: never shorter than
/// stated, so a vehicle is never faster than its link allows and a time that is a whole second
/// in exact arithmetic is not floored to the second before), and when an event is due (never
/// earlier than the time it was computed for).
#[inline]
pub(crate) fn on_grid_up(x: f64) -> f64 {
    (x * TICKS_PER_SECOND).ceil() / TICKS_PER_SECOND
}

/// "The vehicle at the front of queue `queue` is due at `time`."
#[derive(Clone, Copy, Debug)]
pub(crate) struct Event {
    /// When it is due, seconds since the loading's origin; put on the grid when scheduled.
    pub time: f64,
    /// Who goes first at the same time: the service tag at a link's downstream end, the instant
    /// the event became due elsewhere. Never negative.
    pub tag: f64,
    /// When its vehicle reached the place where it waits, seconds since the loading's origin.
    pub waited_from: f64,
    /// Its vehicle's id, [`NO_VEHICLE`] if it carries none.
    pub vehicle: u32,
    pub queue: u32,
}

/// The vehicle of an event that carries none: after every vehicle.
pub(crate) const NO_VEHICLE: u32 = u32::MAX;

/// The sign bit of an `f64`.
const SIGN: u64 = 1 << 63;

/// `x` as an integer that sorts as [`f64::total_cmp`] sorts the floats: a negative number's
/// bits flipped, a positive number's sign bit set. Exact both ways ([`from_sortable`]).
fn sortable(x: f64) -> u64 {
    let bits = x.to_bits();
    if bits & SIGN != 0 { !bits } else { bits | SIGN }
}

/// The `f64` whose [`sortable`] integer is `k`.
fn from_sortable(k: u64) -> f64 {
    f64::from_bits(if k & SIGN != 0 { k & !SIGN } else { !k })
}

/// An event's rank at its instant: the vehicles at links' downstream ends (queues below
/// `link_queues`) first, by their tag, then every other event by its tag. A tag is never
/// negative, so its bits sort as its value and leave the sign bit free to mark the second group.
fn rank(tag: f64, queue: u32, link_queues: u32) -> u64 {
    let bits = if tag > 0.0 { tag.to_bits() } else { 0 };
    if queue < link_queues { bits } else { bits | SIGN }
}

/// An event as the heap holds it: its time, rank and waiting time as integers.
#[derive(Clone, Copy, Debug)]
struct Key {
    time: u64,
    rank: u64,
    waited_from: u64,
    vehicle: u32,
    queue: u32,
}

impl Key {
    fn of(event: Event, link_queues: u32) -> Self {
        Self {
            time: sortable(on_grid_up(event.time)),
            rank: rank(event.tag, event.queue, link_queues),
            waited_from: sortable(event.waited_from),
            vehicle: event.vehicle,
            queue: event.queue,
        }
    }

    fn event(self) -> Event {
        Event {
            time: from_sortable(self.time),
            tag: f64::from_bits(self.rank & !SIGN),
            waited_from: from_sortable(self.waited_from),
            vehicle: self.vehicle,
            queue: self.queue,
        }
    }

    /// Whether `self` comes before `other`: by time, then rank, then the instant its vehicle
    /// has waited from, its id and the queue (the [module docs](self)). It branches only on a
    /// tie of times, which is rare and so predicted; the outcome on the times is taken as a value.
    #[inline]
    fn before(&self, other: &Self) -> bool {
        if self.time == other.time {
            (self.rank, self.waited_from, self.vehicle, self.queue)
                < (other.rank, other.waited_from, other.vehicle, other.queue)
        } else {
            self.time < other.time
        }
    }
}

/// No event for this queue.
const ABSENT: u32 = u32::MAX;

/// Children per node of the heap.
const ARITY: usize = 4;

/// A min-heap of events with each queue's place in it.
///
/// Memory: 32 bytes per live event and 4 per queue.
pub(crate) struct EventQueue {
    heap: Vec<Key>,
    /// Per queue, the place of its event in `heap`, or [`ABSENT`].
    at: Vec<u32>,
    /// The queues below this index hold vehicles at links' downstream ends.
    link_queues: u32,
}

impl EventQueue {
    /// An empty queue for `queues` queues, the first `link_queues` of them links'.
    pub(crate) fn new(queues: usize, link_queues: usize) -> Self {
        let link_queues = u32::try_from(link_queues).expect("fewer links than u32::MAX");
        Self { heap: Vec::new(), at: vec![ABSENT; queues], link_queues }
    }

    /// The earliest event.
    pub(crate) fn peek(&self) -> Option<Event> {
        self.heap.first().map(|k| k.event())
    }

    /// Set `event.queue`'s event to `event`, replacing the one it had.
    pub(crate) fn schedule(&mut self, event: Event) {
        let key = Key::of(event, self.link_queues);
        let i = self.at[event.queue as usize];
        if i == ABSENT {
            self.heap.push(key);
            self.up(self.heap.len() - 1);
        } else {
            let i = i as usize;
            let earlier = key.before(&self.heap[i]);
            self.heap[i] = key;
            if earlier {
                self.up(i);
            } else {
                self.down(i);
            }
        }
    }

    /// [`Self::schedule`], unless `event.queue` already has an event that comes no later.
    pub(crate) fn schedule_earlier(&mut self, event: Event) {
        let i = self.at[event.queue as usize];
        if i == ABSENT || Key::of(event, self.link_queues).before(&self.heap[i as usize]) {
            self.schedule(event);
        }
    }

    /// Remove and return the earliest event.
    pub(crate) fn pop(&mut self) -> Option<Event> {
        let last = self.heap.pop()?;
        let top = if self.heap.is_empty() {
            last
        } else {
            let top = std::mem::replace(&mut self.heap[0], last);
            self.down(0);
            top
        };
        self.at[top.queue as usize] = ABSENT;
        Some(top.event())
    }

    fn place(&mut self, i: usize, key: Key) {
        self.heap[i] = key;
        self.at[key.queue as usize] = u32::try_from(i).expect("fewer events than queues");
    }

    fn up(&mut self, mut i: usize) {
        let key = self.heap[i];
        while i > 0 {
            let parent = (i - 1) / ARITY;
            if !key.before(&self.heap[parent]) {
                break;
            }
            self.place(i, self.heap[parent]);
            i = parent;
        }
        self.place(i, key);
    }

    fn down(&mut self, mut i: usize) {
        let key = self.heap[i];
        let n = self.heap.len();
        loop {
            let first = ARITY * i + 1;
            if first >= n {
                break;
            }
            let h = &self.heap;
            let child = if first + 3 < n {
                // The earliest of four, as the earlier of two pairs' earlier.
                let a = first + usize::from(h[first + 1].before(&h[first]));
                let b = first + 2 + usize::from(h[first + 3].before(&h[first + 2]));
                if h[b].before(&h[a]) { b } else { a }
            } else {
                (first + 1..n).fold(first, |m, j| if h[j].before(&h[m]) { j } else { m })
            };
            if !self.heap[child].before(&key) {
                break;
            }
            self.place(i, self.heap[child]);
            i = child;
        }
        self.place(i, key);
    }
}

#[cfg(test)]
mod tests {
    use super::{Event, EventQueue, rank};

    /// A small deterministic generator (no dependency): 64-bit LCG.
    fn next(state: &mut u64) -> u64 {
        *state =
            state.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        *state >> 33
    }

    /// The queues of the tests, the first half of them links'.
    const QUEUES: usize = 50;
    const LINK_QUEUES: usize = 25;

    /// The order the heap must follow, as a tuple: time, rank, waiting time, vehicle, queue.
    /// Times, tags and waiting times are non-negative, so their bits sort as their values.
    type Order = (u64, u64, u64, u32, u32);

    fn order(e: &Event) -> Order {
        #[allow(clippy::cast_possible_truncation, reason = "a small queue count")]
        let r = rank(e.tag, e.queue, LINK_QUEUES as u32);
        (e.time.to_bits(), r, e.waited_from.to_bits(), e.vehicle, e.queue)
    }

    /// A random event: few distinct times, tags, waiting times and vehicles, so ties happen at
    /// every level of the order.
    fn draw(state: &mut u64) -> Event {
        let queue = u32::try_from(next(state) % QUEUES as u64).unwrap();
        #[allow(clippy::cast_precision_loss, reason = "small numbers")]
        let mut small = |k: u64| (next(state) % k) as f64;
        let (time, tag, waited_from) = (small(40), small(3), small(3));
        let vehicle = u32::try_from(next(state) % 3).unwrap();
        Event { time, tag, waited_from, vehicle, queue }
    }

    /// Against the reference it replaces: a heap of every event ever scheduled, superseded ones
    /// skipped when popped. Both must give the same live events in the same order.
    #[test]
    fn pops_live_events_in_the_order_the_skipping_heap_did() {
        use std::cmp::Reverse;
        use std::collections::BinaryHeap;
        let mut state = 7_u64;
        let mut indexed = EventQueue::new(QUEUES, LINK_QUEUES);
        let mut skipping: BinaryHeap<Reverse<(Order, u32)>> = BinaryHeap::new();
        let mut generation = [0_u32; QUEUES];
        let (mut a, mut b) = (Vec::new(), Vec::new());
        for _ in 0..20_000 {
            if next(&mut state) % 3 == 0 {
                if let Some(e) = indexed.pop() {
                    a.push(order(&e));
                }
                while let Some(Reverse((o, g))) = skipping.pop() {
                    if g == generation[o.4 as usize] {
                        b.push(o);
                        break;
                    }
                }
            } else {
                let e = draw(&mut state);
                indexed.schedule(e);
                generation[e.queue as usize] += 1;
                skipping.push(Reverse((order(&e), generation[e.queue as usize])));
            }
        }
        assert!(a.len() > 1000);
        assert_eq!(a, b);
    }

    /// The heap's integer keys sort as `f64::total_cmp` sorts the floats, and give the floats
    /// back exactly: signed zeros, infinities, subnormals and NaNs included (S294).
    #[test]
    fn sortable_keys_order_as_total_cmp_and_round_trip() {
        use super::{from_sortable, sortable};
        let mut state = 3_u64;
        let mut xs = vec![
            0.0,
            -0.0,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NAN,
            -f64::NAN,
            f64::MIN_POSITIVE,
            -f64::MIN_POSITIVE,
            f64::MAX,
            f64::MIN,
            5e-324,
            -5e-324,
            1.0,
            -1.0,
            27_000.5,
        ];
        for _ in 0..2_000 {
            xs.push(f64::from_bits(next(&mut state) << 31 ^ next(&mut state)));
        }
        for &x in &xs {
            assert_eq!(from_sortable(sortable(x)).to_bits(), x.to_bits());
            for &y in &xs {
                assert_eq!(sortable(x).cmp(&sortable(y)), x.total_cmp(&y), "{x} against {y}");
            }
        }
    }

    /// `schedule_earlier` among `schedule`s and pops, against the skipping heap: an event
    /// replaces a queue's live one only if it comes earlier (S294).
    #[test]
    fn scheduling_only_earlier_pops_what_the_skipping_heap_did() {
        use std::cmp::Reverse;
        use std::collections::BinaryHeap;
        let mut state = 11_u64;
        let mut indexed = EventQueue::new(QUEUES, LINK_QUEUES);
        let mut skipping: BinaryHeap<Reverse<(Order, u32)>> = BinaryHeap::new();
        let mut generation = [0_u32; QUEUES];
        // The skipping heap's live event per queue.
        let mut live: [Option<Order>; QUEUES] = [None; QUEUES];
        let (mut a, mut b) = (Vec::new(), Vec::new());
        for _ in 0..40_000 {
            if next(&mut state) % 3 == 0 {
                if let Some(e) = indexed.pop() {
                    a.push(order(&e));
                }
                while let Some(Reverse((o, g))) = skipping.pop() {
                    if g == generation[o.4 as usize] {
                        b.push(o);
                        live[o.4 as usize] = None;
                        break;
                    }
                }
            } else {
                let e = draw(&mut state);
                let q = e.queue as usize;
                let key = order(&e);
                let earlier_only = next(&mut state) % 2 == 0;
                let goes = !earlier_only || live[q].is_none_or(|old| key < old);
                if earlier_only {
                    indexed.schedule_earlier(e);
                } else {
                    indexed.schedule(e);
                }
                if goes {
                    generation[q] += 1;
                    live[q] = Some(key);
                    skipping.push(Reverse((key, generation[q])));
                }
            }
        }
        assert!(a.len() > 5000, "{} events", a.len());
        assert_eq!(a, b);
    }

    /// The order at one instant: a vehicle at a link's downstream end before a departure
    /// whatever their tags; then by tag, by the instant the vehicle waited from and by its id.
    #[test]
    fn at_one_instant_links_first_then_tag_waiting_and_vehicle() {
        let mut q = EventQueue::new(8, 4);
        let at = |queue: u32, tag: f64, waited_from: f64, vehicle: u32| Event {
            time: 100.0,
            tag,
            waited_from,
            vehicle,
            queue,
        };
        // A departure (queue 5) with the smallest tag; links 0–3 in an order their indices do not
        // give.
        q.schedule(at(5, 0.0, 0.0, 0));
        q.schedule(at(0, 2.0, 50.0, 1));
        q.schedule(at(1, 1.0, 90.0, 9));
        q.schedule(at(2, 1.0, 80.0, 9));
        q.schedule(at(3, 1.0, 80.0, 4));
        let popped: Vec<u32> = std::iter::from_fn(|| q.pop()).map(|e| e.queue).collect();
        assert_eq!(popped, vec![3, 2, 1, 0, 5]);
    }

    /// Times are put on the grid, upwards: one a fraction of a step later than another is due at
    /// the next step, and the sums of times on the grid are exact.
    #[test]
    #[allow(clippy::float_cmp, reason = "times on the grid are exact")]
    fn times_are_put_on_the_grid_upwards() {
        use super::{TICKS_PER_SECOND, on_grid_up};
        let step = 1.0 / TICKS_PER_SECOND;
        assert_eq!(on_grid_up(32_400.0), 32_400.0);
        assert_eq!(on_grid_up(32_400.0 + step / 4.0), 32_400.0 + step);
        let (t1, t2) = (on_grid_up(25.874_640), on_grid_up(44.750_160));
        for d in [3_600.0, 32_400.0, 36_000.0, 86_400.0] {
            assert_eq!((d + t1) + t2, (d + t2) + t1, "at {d} s");
        }
    }
}
