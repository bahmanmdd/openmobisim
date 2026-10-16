//! The node rules: when a link can take a vehicle's front, and what a vehicle gives way to.

use openmobisim_core_types::ids::{EntityId, LinkId};
use openmobisim_core_types::units::{Duration, Pcu};

use super::{Blocked, LtmNetwork, MIN_PART, NONE, TIME_EPSILON};
use crate::curves::{ROOM_EPSILON, room_at};

/// How many waiting-front links [`LtmNetwork::closes_a_waiting_loop`] walks
/// before giving up. A genuine local lock (a roundabout, a small junction
/// cluster) closes within a handful of links; a chain this long that has not
/// closed is not the kind of local lock idea 1 targets, and is left
/// alone rather than walked further — a bounded, defendable cost per check,
/// not a claim that no longer chain could ever matter.
const MAX_LOOP_WALK: usize = 64;

/// Under priority ([`LtmNetwork::with_priority`]), how many times another approach's capacity
/// one of the same road class must have to take priority over it. 1.5, uncalibrated: a
/// clearly bigger road, not one a little wider.
pub const PRIORITY_CAPACITY_RATIO: f64 = 1.5;

impl LtmNetwork<'_> {
    /// Give priority by road hierarchy at unsignalised merges: a vehicle entering a
    /// link gives way while the front vehicle of an approach of **higher priority** to the same
    /// node is bound for that link and ready to go — Daganzo's (1995) priority merge in vehicle
    /// form: the minor stream takes what the major one leaves, and a departure (an origin)
    /// enters the stream only when no approach's front wants the link. An approach has priority
    /// over another if its road class is higher, or, in one class, if its capacity is at least
    /// [`PRIORITY_CAPACITY_RATIO`] times the other's; otherwise neither (the capacity-
    /// proportional merge, as without priority). Not at signalised nodes, where the signal
    /// separates the streams, nor into roundabout links, which keep their own rule.
    ///
    /// Why: a merge that shares room in a fixed ratio lets traffic on a closed loop destroy
    /// itself (Daganzo 1996, *The nature of freeway gridlock and how to prevent it*); priority
    /// to the major stream is the remedy that theory names. Cost: one flag per link, and per
    /// entry into a merge a look at the front of each other approach of the node.
    #[must_use]
    pub fn with_priority(mut self) -> Self {
        let net = self.network;
        self.priority_major = (0..self.links)
            .map(|b| {
                let lb = LinkId::from_index(b);
                net.link_class(lb).carries_motor_traffic() && !net.is_link_signalised(lb)
            })
            .collect();
        self
    }

    /// Whether approach `b` has priority over approach `a` (`None`: a departure from an origin).
    fn outranks(&self, b: usize, a: Option<usize>) -> bool {
        let net = self.network;
        let lb = LinkId::from_index(b);
        if !net.link_class(lb).carries_motor_traffic() {
            return false;
        }
        let Some(a) = a else { return true };
        let (cb, ca) = (net.link_class(lb) as u8, net.link_class(LinkId::from_index(a)) as u8);
        cb < ca
            || (cb == ca && self.inflow_rate[b] >= PRIORITY_CAPACITY_RATIO * self.inflow_rate[a])
    }

    /// The approach a vehicle entering `j` from `from` gives way to under priority, if any: one
    /// of higher priority whose front vehicle is bound for `j` and ready by `t`.
    fn priority_give_way(&self, j: usize, from: Option<usize>, t: f64) -> Option<usize> {
        if self.priority_major.is_empty() {
            return None;
        }
        let net = self.network;
        let lj = LinkId::from_index(j);
        if net.is_roundabout(lj) {
            return None;
        }
        let node = net.link_from(lj);
        if net.in_links(node).iter().any(|&l| net.is_link_signalised(l)) {
            return None;
        }
        net.in_links(node).iter().map(|l| l.index()).find(|&b| {
            Some(b) != from
                && self.priority_major[b]
                && self.outranks(b, from)
                && self.head_bound_for(b, j, t)
        })
    }

    /// Whether link `j` can take a vehicle's front at `t`, and the room it has:
    /// nothing straddles its upstream end, no circulating vehicle it must give
    /// way to is waiting for it, and its room is at least a part —
    /// [`MIN_PART`], the vehicle, or the link's whole storage, whichever is
    /// least. `from` is the link the vehicle comes from (`None` at an origin).
    /// If not, what to wait for.
    pub(super) fn admissible(
        &self,
        j: usize,
        pcu: f64,
        t: f64,
        from: Option<usize>,
    ) -> Result<f64, Blocked> {
        if self.straddling_in[j] != NONE {
            return Err(Blocked { link: j, retry_at: None });
        }
        if let Some(r) = self.must_give_way(j, from) {
            return Err(Blocked { link: r, retry_at: None });
        }
        if let Some(b) = self.priority_give_way(j, from, t) {
            return Err(Blocked { link: b, retry_at: None });
        }
        let storage = self.storage[j];
        if storage.is_infinite() {
            return Ok(f64::INFINITY);
        }
        let (room, retry_at) = self.heard_room(j, t);
        if room + ROOM_EPSILON >= pcu.min(storage).min(MIN_PART) {
            // Idea 1 (S157): an outsider may not take the last room on `j` when
            // doing so would close a waiting loop back to `j`. A link is down to
            // its last room once anything smaller would already have failed the
            // check just above, so gating the (bounded) walk on that keeps it
            // off the common, uncongested case entirely.
            if room < MIN_PART + ROOM_EPSILON && self.closes_a_waiting_loop(j, from) {
                return Err(Blocked { link: j, retry_at: None });
            }
            return Ok(room.max(0.0));
        }
        Err(Blocked { link: j, retry_at })
    }

    /// Whether granting `j` the room it has left would close a waiting loop
    /// back to `j` itself, sealed by a vehicle from **outside** that loop —
    /// idea 1, generalising the roundabout give-way to any local
    /// lock, tagged or not (K30). Walks `j`'s current chain of waiting fronts
    /// ([`Self::front_waits_on`]) forward; if it returns to `j`, this
    /// admission would complete the cycle. A vehicle whose own link (`from`)
    /// is already a member of that chain is circulating within the loop, not
    /// sealing it from outside, and is let through — this is what keeps a
    /// loop's own traffic moving while it still can. Idea 1 does not, on its
    /// own, guarantee a loop never locks: it can still fill through its
    /// own circulation; idea 2 (creep) is the guarantee.
    fn closes_a_waiting_loop(&self, j: usize, from: Option<usize>) -> bool {
        let mut at = j;
        for _ in 0..MAX_LOOP_WALK {
            let Some(next) = self.front_waits_on(at) else { return false };
            if Some(next) == from {
                return false;
            }
            if next == j {
                return true;
            }
            at = next;
        }
        false
    }

    /// A vehicle entering roundabout link `j` from `from` (not itself on the
    /// roundabout) gives way while the circulating link before `j` holds any
    /// vehicle bound for `j` — the priority merge: circulating traffic that
    /// wants the room gets it first. Returns that link; its next departure
    /// wakes the entry.
    fn must_give_way(&self, j: usize, from: Option<usize>) -> Option<usize> {
        let ring = |l: usize| self.network.is_roundabout(LinkId::from_index(l));
        if !ring(j) || from.is_some_and(ring) {
            return None;
        }
        let node = self.network.link_from(LinkId::from_index(j));
        self.network.in_links(node).iter().map(|l| l.index()).find(|&r| {
            Some(r) != from
                && ring(r)
                && self.queues[r].iter().any(|q| {
                    self.route(q.slot).get(q.leg as usize + 1).is_some_and(|l| l.index() == j)
                })
        })
    }

    /// Link `j`'s room as heard at its upstream end at `t`, and when room
    /// already released downstream will next be heard there.
    pub(super) fn heard_room(&self, j: usize, t: f64) -> (f64, Option<f64>) {
        // Room freed by an exit reaches the upstream end exactly one wave
        // travel time later; the epsilon keeps that instant inclusive.
        let heard_until = t + TIME_EPSILON;
        let lag = self.wave_lag[j];
        let room =
            room_at(&self.curves[j], Pcu(self.storage[j]), Duration(lag), Duration(heard_until))
                .get();
        let next =
            self.curves[j].first_exit_after(Duration(heard_until - lag)).map(|e| e.get() + lag);
        (room, next)
    }
}
