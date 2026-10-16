//! Turn pockets: movements that wait in their own lanes, and the look past a parked front.

use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_types::ids::{EntityId, LinkId};

use super::{Blocked, EXIT, LtmNetwork, NONE, TIME_EPSILON, link_u32};
use crate::curves::ROOM_EPSILON;
use crate::level0::Trajectory;

/// Turn pockets (S217), when asked for: their sizes, and where a queue waits for a movement
/// other than its front's.
#[derive(Debug)]
pub(super) struct Pockets {
    /// Per link: what each of its movements' pockets holds, in PCU; 0 without pockets.
    pub(super) size: Vec<f64>,
    /// Per link, its first movement: one per link leaving its downstream node, then one for any
    /// other; one past the last link's last at the end.
    pub(super) first: Vec<u32>,
    /// Per movement: its link.
    pub(super) owner: Vec<u32>,
    /// Per movement: the link its queue waits on for it, or `NONE`, and the next movement
    /// waiting on the same link.
    pub(super) parked_on: Vec<u32>,
    pub(super) next_parked: Vec<u32>,
    /// Per link: the first movement waiting on it.
    pub(super) first_parked: Vec<u32>,
}

/// The vehicles ahead in a queue with turn pockets, by movement — the next link, or [`EXIT`] —
/// and their PCU (S217). A link has few movements; more than eight bar the way.
#[derive(Default)]
struct Ahead {
    n: usize,
    keys: [u32; 8],
    pcu: [f64; 8],
}

impl Ahead {
    fn has(&self, key: u32) -> bool {
        self.keys[..self.n].contains(&key)
    }

    /// Count `pcu` more bound for `key`; false once they no longer fit in a pocket of `pocket`:
    /// they stand in the way of everything behind them.
    fn add(&mut self, key: u32, pcu: f64, pocket: f64) -> bool {
        let k = match self.keys[..self.n].iter().position(|&x| x == key) {
            Some(k) => k,
            None if self.n == self.keys.len() => return false,
            None => {
                self.keys[self.n] = key;
                self.pcu[self.n] = 0.0;
                self.n += 1;
                self.n - 1
            }
        };
        self.pcu[k] += pcu;
        self.pcu[k] <= pocket + ROOM_EPSILON
    }
}

impl LtmNetwork<'_> {
    /// Give approaches of two lanes or more **turn pockets** (S217): a vehicle at the end of such a
    /// link may pass vehicles ahead of it that wait for another movement, as long as those fit in
    /// their movements' pockets — they wait in their own lanes — and none ahead of it is bound
    /// the same way. Each movement's pocket is `length_m` metres of its share of the lanes (the
    /// lanes split evenly among the link's movements, at least one each), never longer than the
    /// link. A link of one lane, or with one movement, keeps one first-in-first-out queue. The
    /// link's discharge capacity stays shared by its movements.
    ///
    /// Why: with one queue per link, a vehicle that cannot enter its next link holds up every
    /// vehicle behind it, including those turning into free streets — full first-in-first-out
    /// at diverges is a known source of unrealistic gridlock (Wright et al. 2017, *On node
    /// models for high-dimensional road networks*), and it made priority at merges hold up more
    /// than it let through (S214). Cost: 16 bytes per link and 12 per movement; and per look at
    /// a link whose front waits, a look at the vehicles behind it as far as the pockets reach.
    #[must_use]
    pub fn with_pockets(mut self, turns: &TurnTable, length_m: f64) -> Self {
        let net = self.network;
        let n = self.links;
        let mut size = Vec::with_capacity(n);
        let mut first = Vec::with_capacity(n + 1);
        let mut owner = Vec::new();
        for i in 0..n {
            let link = LinkId::from_index(i);
            let lanes = f64::from(net.link_lanes(link));
            #[allow(clippy::cast_precision_loss, reason = "a handful of turns")]
            let movements = turns.turns_from(link).len() as f64;
            let length = net.link_length(link).get();
            let pocket = if lanes >= 2.0 && movements >= 2.0 && length > 0.0 && length_m > 0.0 {
                let share = (lanes / movements).max(1.0);
                net.storage(link).get() * (length_m / length).min(1.0) * share / lanes
            } else {
                0.0
            };
            size.push(pocket);
            first.push(link_u32(owner.len()));
            let leaving = net.out_links(net.link_to(link)).len() + 1;
            owner.extend(std::iter::repeat_n(link_u32(i), leaving));
        }
        first.push(link_u32(owner.len()));
        let m = owner.len();
        self.pockets = Some(Pockets {
            size,
            first,
            owner,
            parked_on: vec![NONE; m],
            next_parked: vec![NONE; m],
            first_parked: vec![NONE; n],
        });
        self
    }

    /// What each movement's pocket on link `i` holds, in PCU: 0 without pockets.
    #[inline]
    pub(super) fn pocket(&self, i: usize) -> f64 {
        self.pockets.as_ref().map_or(0.0, |p| p.size[i])
    }

    /// The movement from link `i` onto link `j`, among `i`'s.
    fn movement(&self, p: &Pockets, i: usize, j: usize) -> usize {
        let net = self.network;
        let leaving = net.out_links(net.link_to(LinkId::from_index(i)));
        let k = leaving.iter().position(|l| l.index() == j).unwrap_or(leaving.len());
        p.first[i] as usize + k
    }

    /// Link `i`'s queue waits on link `on` for its movement onto `j` (S217), besides whatever
    /// its front waits on: room heard on `on`, or its upstream end clearing, wakes the queue.
    fn park_movement(&mut self, i: usize, j: usize, on: usize) {
        let Some(mut p) = self.pockets.take() else { return };
        let m = self.movement(&p, i, j);
        let on32 = link_u32(on);
        if p.parked_on[m] != on32 {
            if p.parked_on[m] != NONE {
                let l = p.parked_on[m] as usize;
                let m32 = link_u32(m);
                if p.first_parked[l] == m32 {
                    p.first_parked[l] = p.next_parked[m];
                } else {
                    let mut at = p.first_parked[l];
                    while at != NONE {
                        let next = p.next_parked[at as usize];
                        if next == m32 {
                            p.next_parked[at as usize] = p.next_parked[m];
                            break;
                        }
                        at = next;
                    }
                }
            }
            p.next_parked[m] = p.first_parked[on];
            p.first_parked[on] = link_u32(m);
            p.parked_on[m] = on32;
        }
        self.pockets = Some(p);
    }

    /// The part on link `i` of the vehicle straddling link `j`'s upstream end, if it came from
    /// `i`: it straddles `i`'s downstream end.
    fn straddle_part(&self, j: usize, i: usize) -> Option<f64> {
        let s = self.straddling_in[j];
        if s == NONE {
            return None;
        }
        let parts = &self.parts[s as usize];
        let k = parts.iter().position(|p| p.0 as usize == j)?;
        parts.get(k + 1).filter(|p| p.0 as usize == i).map(|p| p.1)
    }

    /// The vehicle straddling link `i`'s downstream end that holds up its front, if any: the
    /// only one without turn pockets; with them, the one in the front's movement, or one that
    /// does not fit in its pocket (S217). With no vehicle on the link, any.
    pub(super) fn straddler_ahead(&self, i: usize) -> Option<u32> {
        if self.out_straddlers[i] == 0 {
            return None;
        }
        let net = self.network;
        let pocket = self.pocket(i);
        let front = self.queues[i].front();
        let front_next =
            front.and_then(|q| self.route(q.slot).get(q.leg as usize + 1)).map(|l| l.index());
        for &j in net.out_links(net.link_to(LinkId::from_index(i))) {
            let j = j.index();
            let Some(part) = self.straddle_part(j, i) else { continue };
            if pocket <= 0.0
                || front.is_none()
                || Some(j) == front_next
                || part > pocket + ROOM_EPSILON
            {
                return Some(self.straddling_in[j]);
            }
        }
        None
    }

    /// Whether a vehicle at the end of link `b` with its way clear is bound for `j` and ready by
    /// `t`: the front, or with turn pockets one behind it that no vehicle ahead bars (S217).
    pub(super) fn head_bound_for(&self, b: usize, j: usize, t: f64) -> bool {
        let pocket = self.pocket(b);
        let j = link_u32(j);
        let mut ahead = Ahead::default();
        for q in &self.queues[b] {
            if q.ready > t + TIME_EPSILON {
                return false;
            }
            let key =
                self.route(q.slot).get(q.leg as usize + 1).map_or(EXIT, |l| link_u32(l.index()));
            if key == j && !ahead.has(key) {
                return true;
            }
            if !ahead.add(key, self.vehicles[q.slot as usize].pcu.get(), pocket) {
                return false;
            }
        }
        false
    }

    /// Link `i`'s queue at `t`, with turn pockets (S217): the first vehicle at the end of the link
    /// that can move does — the front, or one behind it whose way is clear: none ahead of it bound
    /// for its movement, and those ahead bound for each other movement within its pocket. If none
    /// can, the queue waits on what holds up each movement within reach, and looks again at the
    /// earliest time one may go.
    pub(super) fn process_pockets(
        &mut self,
        i: usize,
        t: f64,
        front_tag: f64,
        t0: f64,
        completed: &mut Vec<Trajectory>,
    ) {
        let pocket = self.pocket(i);
        let mut ahead = Ahead::default();
        // Vehicles straddling the link's end are ahead, each in its movement's lane.
        if self.out_straddlers[i] > 0 {
            let net = self.network;
            for &j in net.out_links(net.link_to(LinkId::from_index(i))) {
                let j = j.index();
                let Some(part) = self.straddle_part(j, i) else { continue };
                if !ahead.add(link_u32(j), part, pocket) {
                    // It does not fit in its pocket: everything waits for its rear to clear the
                    // link's end, which looks at this queue again.
                    if self.rerouting.is_some() {
                        self.offer_reroute(i, t);
                    }
                    return;
                }
            }
        }
        let mut look_again = f64::INFINITY;
        let mut front_held = None;
        // At most eight held movements are remembered. A link with more than eight movements
        // degrades to FIFO safely: `Ahead::add` returns false for a ninth distinct movement, which
        // ends the scan there, so nothing past it is considered or lost (L-16).
        let mut held = [(0, Blocked { link: 0, retry_at: None }); 8];
        let mut n_held = 0;
        let last_exit = self.curves[i].last_exit().get();
        for k in 0..self.queues[i].len() {
            let q = self.queues[i][k];
            let pcu = self.vehicles[q.slot as usize].pcu.get();
            let next = self.route(q.slot).get(q.leg as usize + 1).map(|l| l.index());
            let key = next.map_or(EXIT, link_u32);
            if ahead.has(key) {
                // Behind a vehicle bound the same way, in its lane.
                if k == 0 && self.rerouting.is_some() {
                    self.offer_reroute(i, t);
                }
                if !ahead.add(key, pcu, pocket) {
                    break;
                }
                continue;
            }
            let headway = pcu / self.discharge_rate[i];
            let ready = q.ready.max(last_exit + headway);
            if ready > t + TIME_EPSILON {
                look_again = look_again.min(ready);
                break;
            }
            let room = match next {
                None => Some(f64::INFINITY),
                Some(j) => {
                    // On a signalised approach the inflow headway applies at the stop line.
                    let clear = self.curves[j].last_entry().get() + pcu / self.inflow_rate[j];
                    if self.stop_delay[i] <= 0.0 && clear > t + TIME_EPSILON {
                        look_again = look_again.min(clear);
                        if k == 0 {
                            self.offer_if_inflow_holds_long(i, j, pcu, q.ready, t);
                        }
                        None
                    } else {
                        match self.admissible(j, pcu, t, Some(i)) {
                            Ok(room) => Some(room),
                            Err(blocked) => {
                                if k == 0 {
                                    if self.rerouting.is_some() {
                                        self.offer_reroute(i, t);
                                    }
                                    front_held = Some(blocked);
                                } else if n_held < held.len() {
                                    held[n_held] = (j, blocked);
                                    n_held += 1;
                                }
                                None
                            }
                        }
                    }
                }
            };
            if let Some(room) = room {
                let tag = if k == 0 {
                    front_tag
                } else {
                    let virtual_now = next.map_or(0.0, |j| self.virtual_time[j]);
                    virtual_now.max(self.service_tag[i] + headway)
                };
                return self.advance(i, k, tag, t, room, t0, completed);
            }
            if !ahead.add(key, pcu, pocket) {
                break;
            }
        }
        // Nothing can move now.
        if let Some(blocked) = front_held {
            self.hold(i, blocked, t, t0);
        }
        for &(j, blocked) in &held[..n_held] {
            match blocked.retry_at {
                Some(at) if at > t + TIME_EPSILON => look_again = look_again.min(at),
                _ => self.park_movement(i, j, blocked.link),
            }
        }
        if look_again.is_finite() {
            self.schedule_front(i, t0, look_again);
        }
    }
}
