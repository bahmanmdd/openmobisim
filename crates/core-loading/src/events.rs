//! The loading's event queue: at most one event per queue, in a binary heap indexed by
//! queue (S210).
//!
//! Rescheduling a queue moves its event in the heap instead of leaving the old one behind
//! to be skipped, so the heap holds only live events. The order is the total order of
//! (time, service tag, queue), the same order the loading processed live events in when
//! superseded ones stayed in the heap: results are unchanged.

use std::cmp::Ordering;

/// "The vehicle at the front of queue `queue` is due at `time`."
#[derive(Clone, Copy, Debug)]
pub(crate) struct Event {
    pub time: f64,
    pub tag: f64,
    pub queue: u32,
}

impl Event {
    /// Whether `self` comes before `other`: by time, then tag, then queue.
    fn before(&self, other: &Self) -> bool {
        self.time
            .total_cmp(&other.time)
            .then(self.tag.total_cmp(&other.tag))
            .then(self.queue.cmp(&other.queue))
            == Ordering::Less
    }
}

/// No event for this queue.
const ABSENT: u32 = u32::MAX;

/// A min-heap of events with each queue's place in it.
///
/// Memory: 24 bytes per live event and 4 per queue.
pub(crate) struct EventQueue {
    heap: Vec<Event>,
    /// Per queue, the place of its event in `heap`, or [`ABSENT`].
    at: Vec<u32>,
}

impl EventQueue {
    /// An empty queue for `queues` queues.
    pub(crate) fn new(queues: usize) -> Self {
        Self { heap: Vec::new(), at: vec![ABSENT; queues] }
    }

    /// The earliest event.
    pub(crate) fn peek(&self) -> Option<&Event> {
        self.heap.first()
    }

    /// Set `event.queue`'s event to `event`, replacing the one it had.
    pub(crate) fn schedule(&mut self, event: Event) {
        let i = self.at[event.queue as usize];
        if i == ABSENT {
            self.heap.push(event);
            self.up(self.heap.len() - 1);
        } else {
            let i = i as usize;
            let earlier = event.before(&self.heap[i]);
            self.heap[i] = event;
            if earlier {
                self.up(i);
            } else {
                self.down(i);
            }
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
        Some(top)
    }

    fn place(&mut self, i: usize, event: Event) {
        self.heap[i] = event;
        self.at[event.queue as usize] = u32::try_from(i).expect("fewer events than queues");
    }

    fn up(&mut self, mut i: usize) {
        let event = self.heap[i];
        while i > 0 {
            let parent = (i - 1) / 2;
            if !event.before(&self.heap[parent]) {
                break;
            }
            self.place(i, self.heap[parent]);
            i = parent;
        }
        self.place(i, event);
    }

    fn down(&mut self, mut i: usize) {
        let event = self.heap[i];
        let n = self.heap.len();
        loop {
            let left = 2 * i + 1;
            if left >= n {
                break;
            }
            let right = left + 1;
            let child =
                if right < n && self.heap[right].before(&self.heap[left]) { right } else { left };
            if !self.heap[child].before(&event) {
                break;
            }
            self.place(i, self.heap[child]);
            i = child;
        }
        self.place(i, event);
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
}
