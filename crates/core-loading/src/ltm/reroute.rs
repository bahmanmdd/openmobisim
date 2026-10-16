//! En-route rerouting: the rule, who decides, what is recorded, and the offers a front is due.

use openmobisim_core_types::ids::{EntityId, LinkId, VehicleId};

use super::{LtmNetwork, NONE, TIME_EPSILON, queue_u32};
use crate::events::Event;

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
/// Only [`Self::Stuck`] exists yet. The others planned join here,
/// so records and rerouters already carry the reason: an **alert** to an informed driver about
/// congestion or a disruption ahead (navigation apps), a **disruption** on the route (a closure
/// or a capacity drop), a **new destination** (a taxi dispatched, an electric vehicle sent to
/// charge).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RerouteReason {
    /// Blocked at the front of its link for the rule's time ([`RerouteRule::after_s`]).
    Stuck,
}

impl RerouteReason {
    /// The reason as written in results: `"stuck"`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Stuck => "stuck",
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
    /// The routes of vehicles that re-routed, whole (the links already taken, then the new rest).
    pub(super) routes: Vec<Box<[LinkId]>>,
    /// Links whose front is due an offer, in the order they became due.
    pub(super) pending: Vec<u32>,
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
        let Some(r) = self.rerouting.as_mut() else { return };
        let slot = front.slot as usize;
        if r.count[slot] >= r.rule.max {
            return;
        }
        let due = front.ready.max(r.last_offer[slot]) + r.rule.after_s;
        if due <= t + TIME_EPSILON {
            r.pending.push(queue_u32(i));
        } else {
            self.events.schedule(Event { time: due, tag: due, queue: queue_u32(4 * n + i) });
        }
    }

    /// Link `i`'s front, held by its next link `j`'s inflow headway, is offered a new route as
    /// one held for room is when that headway alone keeps it waiting the rule's time: a closed
    /// or narrowed link ahead. A short headway, the common case, offers nothing.
    pub(super) fn offer_if_inflow_holds_long(
        &mut self,
        i: usize,
        j: usize,
        pcu: f64,
        ready: f64,
        t: f64,
    ) {
        let Some(r) = self.rerouting.as_ref() else { return };
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
        for i in pending {
            let i = i as usize;
            let Some(front) = self.queues[i].front().copied() else { continue };
            let slot = front.slot as usize;
            let leg = front.leg as usize;
            let planned: Vec<LinkId> = self.route(front.slot)[leg + 1..].to_vec();
            if planned.is_empty() {
                continue;
            }
            let vehicle = self.vehicles[slot].id;
            let reason = RerouteReason::Stuck;
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
                let due = t + r.rule.after_s;
                let n = self.links;
                self.events.schedule(Event { time: due, tag: due, queue: queue_u32(4 * n + i) });
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
        let rate = self.discharge_rate[i];
        let on = (self.curves[i].cumulative_in() - self.curves[i].cumulative_out()).get();
        // What is on the link beyond what moves freely at capacity is queued; it clears at the
        // discharge rate.
        let queued = ((on - rate * self.travel[i]).max(0.0) / rate).max(0.0);
        let blocked = match (self.front_waits_on(i), self.queues[i].front()) {
            (Some(_), Some(front)) => (self.clock - front.ready).max(0.0),
            _ => 0.0,
        };
        free + queued + blocked
    }
}
