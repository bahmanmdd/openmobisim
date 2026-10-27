//! En-route rerouting: the rule, who decides, what is recorded, and the offers a front is due.
//!
//! **Two offers.** A vehicle **stuck** at the downstream end of its link for the rule's time is
//! offered a new route ([`RerouteReason::Stuck`]): at levels 3 and 4, where spillback can hold a
//! vehicle anywhere, this lets it out of a lock. A vehicle whose next link is **closed** by a
//! disruption is offered one as it reaches the downstream end of its link, or at its origin if
//! the closed link is its first ([`RerouteReason::ClosedAhead`]): a driver who meets a closed
//! road sees it and may take another way from that junction. A refused vehicle waits there and
//! is offered again after the rule's time. With [`RerouteRule::closures_only`] (the point queue
//! under disruptions the travellers did not know of, S300) the second is the only offer: nobody
//! reads the road further ahead, and ordinary congestion is the equilibrium's to settle.

use openmobisim_core_types::ids::{EntityId, LinkId, NodeId, VehicleId};

use super::{LtmNetwork, NONE, Place, TIME_EPSILON, queue_u32};
use crate::events::Event;

/// When a vehicle is offered a new route ([`LtmNetwork::with_rerouting`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RerouteRule {
    /// Seconds a vehicle waits at the front of its link, blocked, before it is offered one;
    /// another offer comes after as long again.
    pub after_s: f64,
    /// The most times one vehicle takes one.
    pub max: u8,
    /// Offers only to a vehicle whose next link is closed ([`RerouteReason::ClosedAhead`]), none
    /// for being blocked elsewhere.
    pub closures_only: bool,
}

/// The loading's estimate of how long a link takes now: what a vehicle that re-routes
/// sees of the traffic around it.
pub trait LiveTimes {
    /// Seconds to cross `link` if entered now: its free-flow time, the time its queue takes to
    /// discharge at capacity, and how long its front vehicle has been blocked, if it is.
    fn live_seconds(&self, link: LinkId) -> f64;
}

/// Why a vehicle is offered a new route; records and rerouters carry the reason.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RerouteReason {
    /// Blocked at the downstream end of its link for the rule's time ([`RerouteRule::after_s`]).
    Stuck,
    /// Its next link is closed by a disruption: offered as it reaches the downstream end of its
    /// link, or at its origin if the closed link is its first (S289, S300).
    ClosedAhead,
}

impl RerouteReason {
    /// The reason as written in results: `"stuck"` or `"closed_ahead"`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Stuck => "stuck",
            Self::ClosedAhead => "closed_ahead",
        }
    }
}

/// Where a vehicle offered a new route is: at the downstream end of a link, or at its origin
/// before its first link.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RerouteFrom {
    /// At the downstream end of this link: the new route begins with one of its legal turns.
    EndOf(LinkId),
    /// At this origin node, not yet on any link: the new route begins with any link out of it.
    Origin(NodeId),
}

/// Who decides a new route for a vehicle offered one. The loading never routes; it asks.
pub trait Reroute {
    /// The rest of a new route for `vehicle`, from `from` at second `now` and planned to go on
    /// along `planned` (not empty): links beginning where `from` is (a legal turn out of its link,
    /// or a link out of its origin) and ending where `planned` ends — or `None` to keep to the
    /// plan. `live` is the loading's own estimate of each link's time now; `reason` says why it
    /// is asked, so a rerouter can treat drivers and triggers differently (an informed share,
    /// say).
    fn reroute(
        &mut self,
        vehicle: VehicleId,
        from: RerouteFrom,
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
    /// The link at whose end it re-routed; for a vehicle re-routed at its origin, the closed
    /// first link it did not take.
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
    /// Queues (a link's, or an origin's) whose front is due an offer, with the reason, in the
    /// order they became due.
    pub(super) pending: Vec<(u32, RerouteReason)>,
    pub(super) records: Vec<RerouteRecord>,
}

impl LtmNetwork<'_> {
    /// Offer a vehicle a new route when it has waited `rule.after_s` at the front of its link,
    /// blocked — for room on its next link, or giving way — or at once when its next link is
    /// closed ([`RerouteReason::ClosedAhead`]). The new route comes from the [`Reroute`] given to
    /// [`Self::step_rerouting`]; a vehicle takes at most `rule.max`, and is offered again only
    /// after waiting as long again. Must be set before vehicles are added.
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

    /// The front of `queue` — a link's, or an origin's ([`Place::Origin`]) — blocked (for room,
    /// or giving way; parked or waiting for room already released to be heard; at an origin,
    /// refused by a closed first link): if it may still re-route and its next link is closed for
    /// at least the rule's time, it is due an offer at once (a closure is seen, not waited out),
    /// so a queue behind a closure drains by rerouting vehicle after vehicle; a refused vehicle
    /// waits the rule's time for the next. Otherwise, unless the rule offers at closures only, it
    /// is due one once it has waited the rule's time since it reached the end of its link, and
    /// its timer is set for then. A timer that fires finds the front again: the same vehicle
    /// still waiting, or another to time afresh.
    pub(super) fn offer_reroute(&mut self, queue: usize, t: f64) {
        let Some(front) = self.queues[queue].front().copied() else { return };
        let n = self.links;
        let at_origin = matches!(self.place(queue), Place::Origin);
        // Its next link: the one after its link, or its first from an origin.
        let next_leg = if at_origin { front.leg as usize } else { front.leg as usize + 1 };
        let closed_ahead = self
            .route(front.slot)
            .get(next_leg)
            .and_then(|next| self.reopens_at(next.index()))
            .is_some_and(|opens| {
                opens - t >= self.rerouting.as_ref().map_or(0.0, |r| r.rule.after_s)
            });
        let Some(r) = self.rerouting.as_mut() else { return };
        let slot = front.slot as usize;
        if r.count[slot] >= r.rule.max || (r.rule.closures_only && !closed_ahead) {
            return;
        }
        let since =
            if closed_ahead { r.last_offer[slot] } else { front.ready.max(r.last_offer[slot]) };
        let due = since + r.rule.after_s;
        let reason = if closed_ahead { RerouteReason::ClosedAhead } else { RerouteReason::Stuck };
        if due <= t + TIME_EPSILON {
            r.pending.push((queue_u32(queue), reason));
        } else if at_origin {
            // An origin has no timer of its own: its front is looked at again then.
            self.schedule_front(queue, t, due);
        } else {
            self.events.schedule(Event { time: due, tag: due, queue: queue_u32(4 * n + queue) });
        }
    }

    /// Link `i`'s front, held by its next link `j`'s inflow headway, is offered a new route as
    /// one held for room is when that headway alone keeps it waiting the rule's time **and the
    /// link ahead is closed or narrowed by a disruption** (a capacity change below its own).
    /// A headway at the link's own capacity, however long the entry queue, offers nothing: it
    /// is the ordinary queue of a point queue, whose vehicles keep their routes (S289).
    /// At the classic point queue ([`Self::with_node_inflow`]) only a closed link holds a
    /// vehicle so: a narrowed one takes it, and its queue waits at the narrowed link's own
    /// downstream end, where nothing offers a new route (S293).
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

    /// Offer every front due one a new route, in the order they became due.
    pub(super) fn serve_reroutes(&mut self, t0: f64, rr: &mut dyn Reroute) {
        let Some(pending) = self.rerouting.as_mut().map(|r| std::mem::take(&mut r.pending)) else {
            return;
        };
        let t = self.clock;
        let n = self.links;
        let net = self.network;
        for (queue, reason) in pending {
            let queue = queue as usize;
            let Some(front) = self.queues[queue].front().copied() else { continue };
            let slot = front.slot as usize;
            let leg = front.leg as usize;
            let at_origin = matches!(self.place(queue), Place::Origin);
            // What it has not driven yet: the links after its link, or its whole route.
            let taken = if at_origin { 0 } else { leg + 1 };
            let planned: Vec<LinkId> = self.route(front.slot)[taken..].to_vec();
            if planned.is_empty() {
                continue;
            }
            let (from, here) = if at_origin {
                let origin = net.link_from(planned[0]);
                (RerouteFrom::Origin(origin), origin)
            } else {
                let link = LinkId::from_index(queue);
                (RerouteFrom::EndOf(link), net.link_to(link))
            };
            let vehicle = self.vehicles[slot].id;
            let answer = rr.reroute(vehicle, from, &planned, t, &*self, reason);
            let end = net.link_to(*planned.last().expect("not empty"));
            // Taken only if it leaves where the vehicle is, each link follows the one before,
            // and it ends where the plan ends (X-37): a rerouter's slip never moves a vehicle.
            let accepted = answer.filter(|rest| {
                rest.first().is_some_and(|&f| f != planned[0] && net.link_from(f) == here)
                    && rest.windows(2).all(|w| net.link_to(w[0]) == net.link_from(w[1]))
                    && rest.last().is_some_and(|&l| net.link_to(l) == end)
            });
            let r = self.rerouting.as_mut().expect("rerouting is on");
            r.last_offer[slot] = t;
            let Some(rest) = accepted else {
                // Still blocked: the next offer after the rule's time.
                let due = t + r.rule.after_s;
                if at_origin {
                    self.schedule_front(queue, t0, due);
                } else {
                    self.events.schedule(Event {
                        time: due,
                        tag: due,
                        queue: queue_u32(4 * n + queue),
                    });
                }
                continue;
            };
            let mut whole: Vec<LinkId> = Vec::with_capacity(taken + rest.len());
            whole.extend_from_slice(&self.route(front.slot)[..taken]);
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
                link: if at_origin { planned[0] } else { LinkId::from_index(queue) },
                planned_next: planned[0],
                new_next: rest[0],
                reason,
            });
            if at_origin {
                // Departures wait by their first link: it moves to the queue of its new one.
                self.pop_front(queue, t0);
                self.join(n + rest[0].index(), front, t0);
            } else {
                if self.parked_on[queue] != NONE {
                    self.unpark(queue);
                }
                self.schedule_front(queue, t0, t);
            }
        }
    }
}

impl LiveTimes for LtmNetwork<'_> {
    fn live_seconds(&self, link: LinkId) -> f64 {
        let i = link.index();
        let free = self.travel[i] + self.stop_delay[i];
        let on = (self.curves[i].cumulative_in() - self.curves[i].cumulative_out()).get();
        // A link closed by a disruption costs the wait until it opens, and its queue then
        // clears at its own rate (S289), not read as free when it is empty.
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
