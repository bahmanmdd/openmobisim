//! What stands still: the lock report, and the loops of fronts that wait on one another.

use openmobisim_core_types::ids::{EntityId, LinkId};

use super::{LtmNetwork, MIN_PART, NONE};
use crate::curves::ROOM_EPSILON;

/// What stands still when a loading ends: where the vehicles that did not finish are,
/// and the closed loops of links whose front vehicles wait on one another.
///
/// **The check on the loading's own guarantee** — traffic stops only at jam density — is
/// [`Self::room_waits_with_room`]: a front vehicle waiting for room on its next link while
/// that link has at least [`MIN_PART`] of room heard would be a stop the model cannot explain.
/// It must be 0. (A front can also wait without the link it waits on being full: while it
/// gives way at a roundabout, or behind a vehicle still straddling the link's end; those
/// waits are not counted.)
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LockReport {
    /// Vehicles counted on a link when the loading ended.
    pub on_network: u32,
    /// Vehicles waiting outside the network: at an origin or at a stop line.
    pub outside: u32,
    /// Links whose front vehicle was waiting when the loading ended.
    pub waiting_links: u32,
    /// Closed loops of links whose front vehicles wait on one another, each in waiting order
    /// ([`LtmNetwork::waiting_cycles`]).
    pub loops: Vec<Vec<LinkId>>,
    /// Of the waiting fronts — and with turn pockets, the movements waiting behind them — how
    /// many wait for room on their next link while it has room: 0 unless something is wrong.
    pub room_waits_with_room: u32,
}

impl LtmNetwork<'_> {
    /// The link `i`'s front currently waits on, if it is blocked: the link its
    /// queue is parked on, or, while the vehicle ahead still straddles the
    /// link's end, the link that vehicle's front is on. `None` means `i`'s
    /// front is not currently waiting for anything.
    pub(super) fn front_waits_on(&self, i: usize) -> Option<usize> {
        if let Some(ahead) = self.straddler_ahead(i) {
            return self.parts[ahead as usize].front().map(|p| p.0 as usize);
        }
        (self.parked_on[i] != NONE).then(|| self.parked_on[i] as usize)
    }

    /// Closed loops of links whose front vehicles wait on one another now,
    /// each as its links in waiting order. By the module docs' argument every
    /// link whose room is waited for in such a loop is full, to within
    /// [`MIN_PART`]: this is where traffic has reached jam density.
    #[must_use]
    pub fn waiting_cycles(&self) -> Vec<Vec<LinkId>> {
        const UNSEEN: u8 = 0;
        const ON_PATH: u8 = 1;
        const DONE: u8 = 2;
        let mut state = vec![UNSEEN; self.links];
        let mut cycles = Vec::new();
        let mut path = Vec::new();
        for first in 0..self.links {
            path.clear();
            let mut at = first;
            let mut closed = false;
            while state[at] == UNSEEN {
                state[at] = ON_PATH;
                path.push(at);
                match self.front_waits_on(at) {
                    None => break,
                    Some(next) => {
                        closed = state[next] == ON_PATH;
                        at = next;
                    }
                }
            }
            if closed {
                if let Some(from) = path.iter().position(|&p| p == at) {
                    cycles.push(path[from..].iter().map(|&p| LinkId::from_index(p)).collect());
                }
            }
            for &p in &path {
                state[p] = DONE;
            }
        }
        cycles
    }

    /// What stands still now: see [`LockReport`]. Walks every link and vehicle once.
    ///
    /// # Panics
    ///
    /// Never in practice: counts of links and vehicles fit `u32`.
    #[must_use]
    pub fn lock_report(&self) -> LockReport {
        let now = self.now;
        let on_network = self.parts.iter().filter(|p| !p.is_empty()).count();
        let outside: usize = (self.links..3 * self.links).map(|q| self.queues[q].len()).sum();
        let mut waiting_links = 0;
        let mut room_waits_with_room = 0;
        for i in 0..self.links {
            let Some(on) = self.front_waits_on(i) else { continue };
            waiting_links += 1;
            let Some(front) = self.queues[i].front() else { continue };
            let next = self.route(front.slot).get(front.leg as usize + 1);
            let waits_for_room =
                self.straddler_ahead(i).is_none() && next.is_some_and(|l| l.index() == on);
            if waits_for_room && self.storage[on].is_finite() {
                let need = MIN_PART.min(self.storage[on]);
                if self.heard_room(on, now).0 >= need + ROOM_EPSILON {
                    room_waits_with_room += 1;
                }
            }
        }
        // With turn pockets, the movements waiting behind a front for room on their own next
        // link (S217) are held to the same guarantee.
        if let Some(p) = &self.pockets {
            let net = self.network;
            for (m, &on) in p.parked_on.iter().enumerate() {
                if on == NONE {
                    continue;
                }
                let (on, i) = (on as usize, p.owner[m] as usize);
                let leaving = net.out_links(net.link_to(LinkId::from_index(i)));
                let waits_for_room = leaving
                    .get(m - p.first[i] as usize)
                    .is_some_and(|l| l.index() == on && self.straddling_in[on] == NONE);
                if waits_for_room && self.storage[on].is_finite() {
                    let need = MIN_PART.min(self.storage[on]);
                    if self.heard_room(on, now).0 >= need + ROOM_EPSILON {
                        room_waits_with_room += 1;
                    }
                }
            }
        }
        let count = |n: usize| u32::try_from(n).expect("counts fit u32");
        LockReport {
            on_network: count(on_network),
            outside: count(outside),
            waiting_links: count(waiting_links),
            loops: self.waiting_cycles(),
            room_waits_with_room: count(room_waits_with_room),
        }
    }
}
