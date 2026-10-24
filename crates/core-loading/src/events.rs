//! The loading's event queue: at most one event per queue, in a four-ary heap indexed by
//! queue.
//!
//! Rescheduling a queue moves its event in the heap instead of leaving the old one behind
//! to be skipped, so the heap holds only live events. The order is the total order of
//! (time, service tag, queue), the same order the loading processed live events in when
//! superseded ones stayed in the heap: results are unchanged.
//!
//! **Why four-ary, on integer keys (S294).** A loading serves millions of events from a heap
//! of about a thousand (Delft: 2.2 million per loading, 1 100 live on average), so the time
//! goes on comparisons whose outcome no branch predictor can guess. The heap holds each
//! event's time and tag as integers that sort as [`f64::total_cmp`] sorts the floats, compares
//! them branching only on a tie of times (rare, so predicted), and has four children per node,
//! half the levels of a binary heap: replaying a Delft loading's events, 100 ms where the binary
//! heap on floats took 174 ms. The
//! events served are the same, in the same order: the order is total and no two events share
//! a queue, so the earliest live event is the same whatever the heap's shape.

/// "The vehicle at the front of queue `queue` is due at `time`."
#[derive(Clone, Copy, Debug)]
pub(crate) struct Event {
    pub time: f64,
    pub tag: f64,
    pub queue: u32,
}

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

/// An event as the heap holds it: its time and tag as [`sortable`] integers.
#[derive(Clone, Copy, Debug)]
struct Key {
    time: u64,
    tag: u64,
    queue: u32,
}

impl Key {
    fn of(event: Event) -> Self {
        Self { time: sortable(event.time), tag: sortable(event.tag), queue: event.queue }
    }

    fn event(self) -> Event {
        Event { time: from_sortable(self.time), tag: from_sortable(self.tag), queue: self.queue }
    }

    /// Whether `self` comes before `other`: by time, then tag, then queue, each as
    /// [`f64::total_cmp`] orders the floats. It branches only on a tie of times, which is rare
    /// and so predicted; the outcome on the times is taken as a value.
    #[inline]
    fn before(&self, other: &Self) -> bool {
        if self.time == other.time {
            (self.tag < other.tag) | ((self.tag == other.tag) & (self.queue < other.queue))
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
/// Memory: 24 bytes per live event and 4 per queue.
pub(crate) struct EventQueue {
    heap: Vec<Key>,
    /// Per queue, the place of its event in `heap`, or [`ABSENT`].
    at: Vec<u32>,
}

impl EventQueue {
    /// An empty queue for `queues` queues.
    pub(crate) fn new(queues: usize) -> Self {
        Self { heap: Vec::new(), at: vec![ABSENT; queues] }
    }

    /// The earliest event.
    pub(crate) fn peek(&self) -> Option<Event> {
        self.heap.first().map(|k| k.event())
    }

    /// Set `event.queue`'s event to `event`, replacing the one it had.
    pub(crate) fn schedule(&mut self, event: Event) {
        let key = Key::of(event);
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
        if i == ABSENT || Key::of(event).before(&self.heap[i as usize]) {
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
    use super::{Event, EventQueue};

    /// A small deterministic generator (no dependency): 64-bit LCG.
    fn next(state: &mut u64) -> u64 {
        *state =
            state.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        *state >> 33
    }

    /// Against the reference it replaces: a heap of every event ever scheduled, superseded ones
    /// skipped when popped. Both must give the same live events in the same order.
    #[test]
    fn pops_live_events_in_the_order_the_skipping_heap_did() {
        use std::cmp::Reverse;
        use std::collections::BinaryHeap;
        let queues = 50;
        let mut state = 7_u64;
        let mut indexed = EventQueue::new(queues);
        // Times and tags are non-negative, so their bits sort as their values.
        let mut skipping: BinaryHeap<Reverse<(u64, u64, u32, u32)>> = BinaryHeap::new();
        let mut generation = vec![0_u32; queues];
        let (mut a, mut b) = (Vec::new(), Vec::new());
        for _ in 0..20_000 {
            if next(&mut state) % 3 == 0 {
                if let Some(e) = indexed.pop() {
                    a.push((e.time.to_bits(), e.tag.to_bits(), e.queue));
                }
                while let Some(Reverse((t, tag, q, g))) = skipping.pop() {
                    if g == generation[q as usize] {
                        b.push((t, tag, q));
                        break;
                    }
                }
            } else {
                let q = u32::try_from(next(&mut state) % queues as u64).unwrap();
                // Few distinct times, so ties on time and tag happen.
                #[allow(clippy::cast_precision_loss, reason = "small numbers")]
                let (t, tag) = ((next(&mut state) % 40) as f64, (next(&mut state) % 3) as f64);
                indexed.schedule(Event { time: t, tag, queue: q });
                generation[q as usize] += 1;
                skipping.push(Reverse((t.to_bits(), tag.to_bits(), q, generation[q as usize])));
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
        let queues = 64;
        let mut state = 11_u64;
        let mut indexed = EventQueue::new(queues);
        let mut skipping: BinaryHeap<Reverse<(u64, u64, u32, u32)>> = BinaryHeap::new();
        let mut generation = vec![0_u32; queues];
        // The reference's live event per queue.
        let mut live: Vec<Option<(u64, u64)>> = vec![None; queues];
        let (mut a, mut b) = (Vec::new(), Vec::new());
        for _ in 0..40_000 {
            if next(&mut state) % 3 == 0 {
                if let Some(e) = indexed.pop() {
                    a.push((e.time.to_bits(), e.tag.to_bits(), e.queue));
                }
                while let Some(Reverse((t, tag, q, g))) = skipping.pop() {
                    if g == generation[q as usize] {
                        b.push((t, tag, q));
                        live[q as usize] = None;
                        break;
                    }
                }
            } else {
                let q = u32::try_from(next(&mut state) % queues as u64).unwrap();
                #[allow(clippy::cast_precision_loss, reason = "small numbers")]
                let (t, tag) = ((next(&mut state) % 40) as f64, (next(&mut state) % 3) as f64);
                let event = Event { time: t, tag, queue: q };
                // Non-negative times and tags: their bits sort as their values.
                let key = (t.to_bits(), tag.to_bits());
                let earlier_only = next(&mut state) % 2 == 0;
                let goes = !earlier_only || live[q as usize].is_none_or(|old| key < old);
                if earlier_only {
                    indexed.schedule_earlier(event);
                } else {
                    indexed.schedule(event);
                }
                if goes {
                    generation[q as usize] += 1;
                    live[q as usize] = Some(key);
                    skipping.push(Reverse((key.0, key.1, q, generation[q as usize])));
                }
            }
        }
        assert!(a.len() > 5000, "{} events", a.len());
        assert_eq!(a, b);
    }
}
