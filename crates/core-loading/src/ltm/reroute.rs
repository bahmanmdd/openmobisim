//! En-route rerouting: the rule, who decides, what is recorded, and the offers a front is due.
//!
//! **Two offers.** A vehicle **stuck** at the downstream end of its link for the rule's time is
//! offered a new route ([`RerouteReason::Stuck`]). At the point queue a queue has no length, so
//! nothing upstream of the link that holds it is ever stuck: a closure whose node has no other
//! exit re-routed nobody (X-69). So at the point queue a vehicle is also offered a route at the
//! downstream end of its link when it can **see a queue ahead** ([`RerouteReason::QueueAhead`]):
//! a link on its route, **narrowed or closed by a disruption or feeding a closed one**, holds more
//! waiting vehicles than its storage and the links between would hold, which is where the back of
//! that queue would stand if the point queue had spillback. The offer comes at once (the vehicle
//! is not held), once per link of the route, and the vehicle drives on if it is refused; it
//! reaches every junction upstream of the queue's virtual back, so the last one with a way round
//! takes the traffic. Ordinary congestion offers nothing at the point queue (the equilibration
//! settles it; offered on, Delft's base ran forty times slower) (S294).

use openmobisim_core_types::ids::{EntityId, LinkId, VehicleId};

use super::{FidelityLevel, LtmNetwork, NONE, Queued, TIME_EPSILON, queue_u32};
use crate::curves::ROOM_EPSILON;
use crate::events::Event;

/// How many links ahead a vehicle looks for a queue ([`RerouteReason::QueueAhead`]): the walk
/// stops once the links between would hold any queue in the network, long before this on most.
const QUEUE_LOOKAHEAD: usize = 16;

/// When a vehicle is offered a new route ([`LtmNetwork::with_rerouting`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RerouteRule {
    /// Seconds a vehicle waits at the front of its link, blocked, before it is offered one;
    /// another offer comes after as long again.
    pub after_s: f64,
    /// The most times one vehicle takes one.
    pub max: u8,
}

/// The loading's estimate of how long a link takes now: what a vehicle that re-routes
/// sees of the traffic around it.
pub trait LiveTimes {
    /// Seconds to cross `link` if entered now: its free-flow time, the time its queue takes to
    /// discharge at capacity, and how long its front vehicle has been blocked, if it is.
    fn live_seconds(&self, link: LinkId) -> f64;
}

/// Why a vehicle is offered a new route.
///
/// [`Self::Stuck`] and [`Self::QueueAhead`] exist. The others planned join here,
/// so records and rerouters already carry the reason: an **alert** to an informed driver about
/// congestion or a disruption ahead (navigation apps), a **disruption** on the route (a closure
/// or a capacity drop), a **new destination** (a taxi dispatched, an electric vehicle sent to
/// charge).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RerouteReason {
    /// Blocked at the downstream end of its link for the rule's time ([`RerouteRule::after_s`]).
    Stuck,
    /// At the point queue, at the downstream end of its link with a queue ahead on its route
    /// longer than the links before it would hold: where the back of that queue would stand
    /// if the point queue had spillback (S294).
    QueueAhead,
}

impl RerouteReason {
    /// The reason as written in results: `"stuck"` or `"queue_ahead"`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Stuck => "stuck",
            Self::QueueAhead => "queue_ahead",
        }
    }
}

/// Who decides a new route for a vehicle stuck at the front of its link. The loading
/// never routes; it asks.
pub trait Reroute {
    /// The rest of a new route for `vehicle`, at the end of `current` at second `now` and
    /// planned to go on along `planned` (not empty): links beginning with a legal turn out of
    /// `current` and ending where `planned` ends — or `None` to keep to the plan. `live` is the
    /// loading's own estimate of each link's time now; `reason` says why it is asked, so a
    /// rerouter can treat drivers and triggers differently (an informed share, say).
    fn reroute(
        &mut self,
        vehicle: VehicleId,
        current: LinkId,
        planned: &[LinkId],
        now: f64,
        live: &dyn LiveTimes,
        reason: RerouteReason,
    ) -> Option<Vec<LinkId>>;
}

/// One reroute: who, when, where, and the next link planned and taken instead.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RerouteRecord {
    /// The vehicle.
    pub vehicle: VehicleId,
    /// When, in seconds.
    pub second: f64,
    /// The link at whose end it re-routed.
    pub link: LinkId,
    /// The next link of its route until then.
    pub planned_next: LinkId,
    /// The next link of its new route.
    pub new_next: LinkId,
    /// Why it was offered one.
    pub reason: RerouteReason,
}

/// En-route rerouting's state: per vehicle, nothing until it re-routes.
#[derive(Debug)]
pub(super) struct Rerouting {
    pub(super) rule: RerouteRule,
    /// Per slot: which of `routes` replaces its route (`NONE`: its own).
    pub(super) route_of: Vec<u32>,
    /// Per slot: how many times it re-routed.
    pub(super) count: Vec<u8>,
    /// Per slot: when it was last offered a route (−∞: never).
    pub(super) last_offer: Vec<f64>,
    /// Per slot: the leg of its route at which it was last offered a route for a queue ahead
    /// (`NONE`: never), so that each junction asks once.
    pub(super) offered_leg: Vec<u32>,
    /// The routes of vehicles that re-routed, whole (the links already taken, then the new rest).
    pub(super) routes: Vec<Box<[LinkId]>>,
    /// Links whose front is due an offer, with the reason, in the order they became due.
    pub(super) pending: Vec<(u32, RerouteReason)>,
    pub(super) records: Vec<RerouteRecord>,
}

impl LtmNetwork<'_> {
    /// Offer a vehicle a new route when it has waited `rule.after_s` at the front of its link,
    /// blocked — for room on its next link, or giving way. The new route comes from the
    /// [`Reroute`] given to [`Self::step_rerouting`]; a vehicle takes at most `rule.max`, and is
    /// offered again only after waiting as long again. Must be set before vehicles are added.
    ///
    /// Cost: per vehicle, 13 bytes (which route, how many, when last offered); per reroute, the
    /// new route and a 32-byte record; one timer per waiting link front.
    ///
    /// # Panics
    ///
    /// Panics if vehicles were already added.
    #[must_use]
    pub fn with_rerouting(mut self, rule: RerouteRule) -> Self {
        assert!(self.vehicles.is_empty(), "rerouting is set before vehicles are added");
        self.rerouting = Some(Rerouting {
            rule,
            route_of: Vec::new(),
            count: Vec::new(),
            last_offer: Vec::new(),
            offered_leg: Vec::new(),
            routes: Vec::new(),
            pending: Vec::new(),
            records: Vec::new(),
        });
        self
    }

    /// Every reroute so far, in the order they happened.
    #[must_use]
    pub fn reroutes(&self) -> &[RerouteRecord] {
        self.rerouting.as_ref().map_or(&[], |r| r.records.as_slice())
    }

    /// The route a vehicle follows: its own, or the one it re-routed to.
    #[inline]
    pub(super) fn route(&self, slot: u32) -> &[LinkId] {
        if let Some(r) = &self.rerouting {
            let i = r.route_of[slot as usize];
            if i != NONE {
                return &r.routes[i as usize];
            }
        }
        &self.vehicles[slot as usize].route
    }

    /// Link `i`'s front vehicle, blocked (for room, or giving way; parked or waiting for room
    /// already released to be heard): if it has waited long enough since it reached the end of
    /// the link, and may still re-route, it is due an offer, served right after the event being
    /// processed; if not yet, its timer is set for when it will have. A timer that fires finds
    /// the front again: the same vehicle still waiting, or another to time afresh.
    pub(super) fn offer_reroute(&mut self, i: usize, t: f64) {
        let Some(front) = self.queues[i].front().copied() else { return };
        let n = self.links;
        // A link ahead closed by a disruption for at least the rule's time: the front is offered
        // a route as soon as it gets there (a closure is seen, not waited out), so a queue behind
        // a closure drains by rerouting vehicle after vehicle instead of one every `after_s`;
        // a refused vehicle still waits the rule's time for the next offer (S289).
        let closed_ahead = self
            .route(front.slot)
            .get(front.leg as usize + 1)
            .and_then(|next| self.reopens_at(next.index()))
            .is_some_and(|opens| {
                opens - t >= self.rerouting.as_ref().map_or(0.0, |r| r.rule.after_s)
            });
        let Some(r) = self.rerouting.as_mut() else { return };
        let slot = front.slot as usize;
        if r.count[slot] >= r.rule.max {
            return;
        }
        let since =
            if closed_ahead { r.last_offer[slot] } else { front.ready.max(r.last_offer[slot]) };
        let due = since + r.rule.after_s;
        if due <= t + TIME_EPSILON {
            r.pending.push((queue_u32(i), RerouteReason::Stuck));
        } else {
            self.events.schedule(Event { time: due, tag: due, queue: queue_u32(4 * n + i) });
        }
    }

    /// At the point queue, whether link `i`'s `front`, about to leave, can see a queue
    /// ahead on its route behind a disruption: a link ahead, narrowed or closed by a capacity
    /// change or feeding a closed one, holds more waiting vehicles (what is on it beyond what
    /// crosses it freely at its discharge rate) than its own storage and the storage of the
    /// links between would hold, which is where the back of that queue would stand with
    /// spillback. If so, and the vehicle may still re-route and was not asked at this leg, it is
    /// due an offer ([`RerouteReason::QueueAhead`]), served right after the event being processed,
    /// and `true` is returned: the caller lets the offer be served before the vehicle moves.
    /// Costs a few lookups per link ahead, at most [`QUEUE_LOOKAHEAD`], for each vehicle leaving
    /// a link; nothing at the other levels, where the queue itself reaches upstream.
    pub(super) fn offer_if_queue_ahead(&mut self, i: usize, front: Queued) -> bool {
        if !matches!(self.level, FidelityLevel::PointQueue) {
            return false;
        }
        let Some(r) = self.rerouting.as_ref() else { return false };
        let slot = front.slot as usize;
        if r.count[slot] >= r.rule.max || r.offered_leg[slot] == front.leg {
            return false;
        }
        // Only a queue behind a disruption counts (S289, S293: offers at every congested entry
        // made a level-2 run ten times slower, and ordinary congestion is the equilibrium's to
        // settle, not the day's): a link narrowed or closed by a capacity change queues at its
        // own downstream end, and the link before a closed one holds the traffic it refuses.
        if self.changes.is_empty() {
            return false;
        }
        let route = self.route(front.slot);
        let leg = front.leg as usize;
        let disrupted = |m: usize| self.discharge_rate[m] < self.base_rates[m].0;
        let mut behind = 0.0;
        let mut seen = false;
        for (p, &m) in route.iter().enumerate().skip(leg + 1).take(QUEUE_LOOKAHEAD) {
            let m = m.index();
            let storage = self.network.storage(LinkId::from_index(m)).get();
            let behind_a_disruption =
                disrupted(m) || route.get(p + 1).is_some_and(|&n| self.is_closed(n.index()));
            if behind_a_disruption {
                let on = (self.curves[m].cumulative_in() - self.curves[m].cumulative_out()).get();
                let waiting = (on - self.discharge_rate[m] * self.travel[m]).max(0.0);
                if waiting - storage > behind + ROOM_EPSILON {
                    seen = true;
                    break;
                }
            }
            behind += storage;
        }
        if !seen {
            return false;
        }
        let r = self.rerouting.as_mut().expect("checked above");
        r.offered_leg[slot] = front.leg;
        r.pending.push((queue_u32(i), RerouteReason::QueueAhead));
        true
    }

    /// Link `i`'s front, held by its next link `j`'s inflow headway, is offered a new route as
    /// one held for room is when that headway alone keeps it waiting the rule's time **and the
    /// link ahead is closed or narrowed by a disruption** (a capacity change below its own).
    /// A headway at the link's own capacity, however long the entry queue, offers nothing: it
    /// is the ordinary queue of a point queue, whose vehicles keep their routes (S289: offers
    /// at every congested entry made a level-2 run with known disruptions ten times slower).
    /// At the classic point queue ([`Self::with_node_inflow`]) only a closed link holds a
    /// vehicle so: a narrowed one takes it, and its queue waits at the narrowed link's own exit,
    /// where nothing offers a new route (S293).
    pub(super) fn offer_if_inflow_holds_long(
        &mut self,
        i: usize,
        j: usize,
        pcu: f64,
        ready: f64,
        t: f64,
    ) {
        let Some(r) = self.rerouting.as_ref() else { return };
        if self.changes.is_empty() || self.inflow_rate[j] >= self.base_rates[j].1 {
            return;
        }
        let clear = self.curves[j].last_entry().get() + pcu / self.inflow_rate[j];
        if clear - ready >= r.rule.after_s {
            self.offer_reroute(i, t);
        }
    }

    /// Offer every link front due one a new route, in the order they became due.
    pub(super) fn serve_reroutes(&mut self, t0: f64, rr: &mut dyn Reroute) {
        let Some(pending) = self.rerouting.as_mut().map(|r| std::mem::take(&mut r.pending)) else {
            return;
        };
        let t = self.clock;
        for (i, reason) in pending {
            let i = i as usize;
            let Some(front) = self.queues[i].front().copied() else { continue };
            let slot = front.slot as usize;
            let leg = front.leg as usize;
            let planned: Vec<LinkId> = self.route(front.slot)[leg + 1..].to_vec();
            if planned.is_empty() {
                continue;
            }
            let vehicle = self.vehicles[slot].id;
            let answer = rr.reroute(vehicle, LinkId::from_index(i), &planned, t, &*self, reason);
            let net = self.network;
            let end = net.link_to(*planned.last().expect("not empty"));
            // Taken only if it leaves where the vehicle is, each link follows the one before,
            // and it ends where the plan ends (X-37): a rerouter's slip never moves a vehicle.
            let here = net.link_to(LinkId::from_index(i));
            let accepted = answer.filter(|rest| {
                rest.first().is_some_and(|&f| f != planned[0] && net.link_from(f) == here)
                    && rest.windows(2).all(|w| net.link_to(w[0]) == net.link_from(w[1]))
                    && rest.last().is_some_and(|&l| net.link_to(l) == end)
            });
            let r = self.rerouting.as_mut().expect("rerouting is on");
            r.last_offer[slot] = t;
            let Some(rest) = accepted else {
                match reason {
                    // Still blocked: the next offer after the rule's time.
                    RerouteReason::Stuck => {
                        let due = t + r.rule.after_s;
                        let n = self.links;
                        self.events.schedule(Event {
                            time: due,
                            tag: due,
                            queue: queue_u32(4 * n + i),
                        });
                    }
                    // Free to go: it drives on towards the queue.
                    RerouteReason::QueueAhead => self.schedule_front(i, t0, t),
                }
                continue;
            };
            let mut whole: Vec<LinkId> = Vec::with_capacity(leg + 1 + rest.len());
            whole.extend_from_slice(&self.route(front.slot)[..=leg]);
            whole.extend_from_slice(&rest);
            let r = self.rerouting.as_mut().expect("rerouting is on");
            match r.route_of[slot] {
                NONE => {
                    r.route_of[slot] =
                        u32::try_from(r.routes.len()).expect("fewer reroutes than u32");
                    r.routes.push(whole.into_boxed_slice());
                }
                k => r.routes[k as usize] = whole.into_boxed_slice(),
            }
            r.count[slot] += 1;
            r.records.push(RerouteRecord {
                vehicle,
                second: t,
                link: LinkId::from_index(i),
                planned_next: planned[0],
                new_next: rest[0],
                reason,
            });
            if self.parked_on[i] != NONE {
                self.unpark(i);
            }
            self.schedule_front(i, t0, t);
        }
    }
}

impl LiveTimes for LtmNetwork<'_> {
    fn live_seconds(&self, link: LinkId) -> f64 {
        let i = link.index();
        let free = self.travel[i] + self.stop_delay[i];
        let on = (self.curves[i].cumulative_in() - self.curves[i].cumulative_out()).get();
        // A link closed by a disruption costs the wait until it opens, and its queue then
        // clears at its own rate (S289); an empty closed link read as free before.
        let (rate, closed) = match self.reopens_at(i) {
            Some(opens) => (self.base_rates[i].0, (opens - self.clock).max(0.0)),
            None => (self.discharge_rate[i], 0.0),
        };
        // What is on the link beyond what moves freely at capacity is queued; it clears at the
        // discharge rate.
        let queued = ((on - rate * self.travel[i]).max(0.0) / rate).max(0.0);
        let blocked = match (self.front_waits_on(i), self.queues[i].front()) {
            (Some(_), Some(front)) => (self.clock - front.ready).max(0.0),
            _ => 0.0,
        };
        free + closed + queued + blocked
    }
}
