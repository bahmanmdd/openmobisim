//! The results tables of a run's docked bike service (S302), from its service day (the last
//! loading's shared-bike trips, played once after the run has stopped, S303):
//!
//! - [`BikeShareEventTable`] (`bike_share_events.parquet`): the day's log, one row per event in
//!   time order, so the sequence of events and each dock's state after each can be followed;
//! - [`BikeShareTripTable`] (`bike_share_trips.parquet`): one row per trip that chose a shared
//!   bike: its planned docks and times, what happened at the docks, the extra walks, and its
//!   outcome ([`trip_outcome`]);
//! - [`BikeShareDockTable`] (`bike_share_docks.parquet`): one row per dock and time bin: its
//!   state, its flows, and the travellers who had trouble there.
//!
//! **Which hub.** The run's hub set ([`RunHubs`]) is built with every service's resources in
//! service and dock order ([`ServiceSetup::resource_rows`]); [`resource_hubs`] reads each dock's
//! hub back from it in that order.
//!
//! **Cost:** about 100 bytes per event, 200 per trip and 200 per dock and bin while held.

use openmobisim_core_demand::{Mode, Travellers, Trips};
use openmobisim_core_types::ids::EntityId;

use super::docks::{DockEvent, DockEventKind, NONE};
use super::hubs::RunHubs;
use super::in_run::ShareDay;
use super::outcome::{FleetOutcome, LegStatus};
use super::{ServiceParts, ServiceSetup};

/// Per service, per dock or charger (in its order), its hub's external id and kind: the
/// services' resources were given to `hubs` in service order, after its parkings
/// ([`RunHubs::build`]'s `rows`).
#[must_use]
pub fn resource_hubs(setups: &[ServiceSetup], hubs: &RunHubs) -> Vec<Vec<(String, &'static str)>> {
    let mut first = 0;
    setups
        .iter()
        .map(|s| {
            let n = s.resource_rows().len();
            let out = (first..first + n)
                .map(|i| {
                    let hub = hubs.set().resource_hub(hubs.row_resource(i));
                    (hubs.hub_external_id(hub).to_string(), hubs.set().kind(hub).as_str())
                })
                .collect();
            first += n;
            out
        })
        .collect()
}

/// Seconds from midnight as a clock time, `08:03:20` (whole seconds, rounded down; past
/// midnight `24:…`).
#[must_use]
pub fn clock(s: f64) -> String {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "a clock")]
    let s = s.max(0.0).floor() as u64;
    format!("{:02}:{:02}:{:02}", s / 3600, s % 3600 / 60, s % 60)
}

fn dock_ids(setup: &ServiceSetup) -> &[String] {
    match setup.parts() {
        ServiceParts::BikeDocked(docks, _) => &docks.external,
        ServiceParts::Taxi(..) => &[],
    }
}

fn finite(v: f64) -> Option<f64> {
    v.is_finite().then_some(v)
}

/// Who each leg of `day` is: its traveller's id and the trip's place in their day.
fn who(day: &ShareDay, trips: &Trips, travellers: &Travellers) -> Vec<(String, u32)> {
    day.legs
        .iter()
        .map(|l| {
            let traveller = trips.traveller(l.trip);
            (
                travellers.external_ids().external(traveller.raw()).to_string(),
                l.trip.raw() - travellers.first_trip(traveller).raw(),
            )
        })
        .collect()
}

/// The day's log of events, one row per event in time order: `bike_share_events.parquet`.
/// `None` is a field that does not apply (written as null).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BikeShareEventTable {
    /// The service's name.
    pub service: Vec<String>,
    /// When, in seconds from midnight.
    pub time_s: Vec<f64>,
    /// The traveller (`None`: an operator's van).
    pub traveller_id: Vec<Option<String>>,
    /// The trip's place in the traveller's day, from 0.
    pub trip_seq: Vec<Option<u32>>,
    /// The people the traveller stands for (the bikes they take); a van's event: the bikes
    /// moved.
    pub travellers: Vec<u32>,
    /// What happened ([`super::docks::DockEventKind::as_str`]).
    pub event: Vec<&'static str>,
    /// The dock (`None`: the arrival at the destination).
    pub dock: Vec<Option<String>>,
    /// The bikes docked there just after.
    pub bikes_at_dock_after: Vec<Option<u32>>,
    /// The free slots there just after (0 when bikes stand above capacity).
    pub free_slots_after: Vec<Option<u32>>,
    /// The dock walked or ridden on to.
    pub next_dock: Vec<Option<String>>,
    /// How far that dock is, in metres as the crow flies.
    pub to_next_m: Vec<Option<f64>>,
    /// For `reached_transit_stop`, the stop's hub (the hub of the dock the trip planned to
    /// return its bike at).
    pub transit_stop: Vec<Option<String>>,
}

impl BikeShareEventTable {
    /// The events of `day` on `setups`, whose trips are `trips` of `travellers` with their
    /// `modes` (by trip index), the docks' hubs from `hubs` ([`resource_hubs`]); empty if the
    /// day kept no log. A shared bike to transit ends its leg with `reached_transit_stop` (its
    /// stop's hub in [`Self::transit_stop`], no dock) where a shared bike alone ends with
    /// `reached_destination`.
    ///
    /// # Panics
    ///
    /// If an event's leg or dock is not of `day`, `setups` or `hubs`.
    #[must_use]
    pub fn new(
        day: &ShareDay,
        setups: &[ServiceSetup],
        (trips, travellers, modes): (&Trips, &Travellers, &[Option<Mode>]),
        hubs: &[Vec<(String, &'static str)>],
    ) -> Self {
        let mut t = Self::default();
        let people = who(day, trips, travellers);
        for (s, log) in day.outcome.dock_log.iter().enumerate() {
            let Some(log) = log else { continue };
            let ids = dock_ids(&setups[s]);
            let id = |d: u32| (d != NONE).then(|| ids[d as usize].clone());
            for e in log {
                t.push(setups[s].name(), e, &id, day, &people);
                let leg = (e.leg != NONE).then(|| &day.legs[e.leg as usize]);
                let chain = leg.filter(|l| {
                    e.kind == DockEventKind::ReachedDestination
                        && modes[l.trip.index()] == Some(Mode::BikeShareTransit)
                });
                t.transit_stop.push(chain.map(|l| hubs[s][l.leg.return_dock as usize].0.clone()));
                if chain.is_some() {
                    *t.event.last_mut().expect("just pushed") = "reached_transit_stop";
                }
            }
        }
        t
    }

    fn push(
        &mut self,
        service: &str,
        e: &DockEvent,
        id: &dyn Fn(u32) -> Option<String>,
        day: &ShareDay,
        people: &[(String, u32)],
    ) {
        self.service.push(service.to_string());
        self.time_s.push(e.t);
        let leg = (e.leg != NONE).then_some(e.leg as usize);
        self.traveller_id.push(leg.map(|l| people[l].0.clone()));
        self.trip_seq.push(leg.map(|l| people[l].1));
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "bikes")]
        let n = leg.map_or(e.to_next_m as u32, |l| day.legs[l].leg.weight);
        self.travellers.push(n);
        self.event.push(e.kind.as_str());
        self.dock.push(id(e.dock));
        self.bikes_at_dock_after.push((e.bikes_after != NONE).then_some(e.bikes_after));
        self.free_slots_after.push((e.free_slots_after != NONE).then_some(e.free_slots_after));
        self.next_dock.push(id(e.next_dock));
        self.to_next_m.push(if leg.is_some() { finite(e.to_next_m) } else { None });
    }

    /// How many events.
    #[must_use]
    pub fn len(&self) -> usize {
        self.service.len()
    }

    /// Whether there is none.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.service.is_empty()
    }
}

/// One row per trip that chose a shared bike, in trip order: `bike_share_trips.parquet`. Seconds
/// count from midnight; `None` is what did not happen (written as null).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BikeShareTripTable {
    /// The service's name.
    pub service: Vec<String>,
    /// The traveller's id, as given.
    pub traveller_id: Vec<String>,
    /// The trip's place in its traveller's day, from 0.
    pub trip_seq: Vec<u32>,
    /// The people the traveller stands for: the bikes the trip takes.
    pub travellers: Vec<u32>,
    /// The trip's mode: `bike_share`, or `bike_share_transit` (a shared bike to a stop's hub,
    /// transit on).
    pub mode: Vec<&'static str>,
    /// The pickup dock its choice planned.
    pub planned_pickup_dock: Vec<String>,
    /// The return dock its choice planned.
    pub planned_return_dock: Vec<String>,
    /// When it was to reach the planned pickup dock.
    pub planned_at_pickup_dock_s: Vec<f64>,
    /// When it was to arrive, had the planned docks had a bike and a free slot.
    pub planned_leg_end_s: Vec<f64>,
    /// The dock the bike was taken from.
    pub pickup_dock: Vec<Option<String>>,
    /// When.
    pub pickup_s: Vec<Option<f64>>,
    /// The docks reached to take a bike (the planned one included).
    pub docks_tried_for_bike: Vec<u32>,
    /// The dock the bike was left at (above capacity included).
    pub return_dock: Vec<Option<String>>,
    /// When.
    pub return_s: Vec<Option<f64>>,
    /// The docks reached to return the bike (the planned one included).
    pub docks_tried_for_slot: Vec<u32>,
    /// When it reached the leg's end (the destination, or for `bike_share_transit` the dock at
    /// the stop's hub), riding or on foot.
    pub leg_end_s: Vec<Option<f64>>,
    /// `leg_end_s − planned_leg_end_s`: the extra seconds it had at the docks.
    pub extra_s: Vec<Option<f64>>,
    /// The part of `extra_s` at the pickup: `extra_s − extra_walk_at_return_s` (the walks
    /// between docks looking for a bike, the ride's change from another pickup dock, or the
    /// walk on of a traveller who found no bike).
    pub extra_walk_at_pickup_s: Vec<Option<f64>>,
    /// The part of `extra_s` at the return: the rides between return docks after a full one,
    /// plus the walk from the dock the bike was left at less the planned walk from the planned
    /// return dock (0 when left at the planned return dock).
    pub extra_walk_at_return_s: Vec<Option<f64>>,
    /// Metres walked between docks looking for a bike, as the crow flies.
    pub extra_walk_m: Vec<f64>,
    /// Docks found without a bike (the last one included when it found none at all).
    pub failed_attempts_for_bike: Vec<u32>,
    /// Docks found without a free slot (the last one included when it left the bike above
    /// capacity).
    pub failed_attempts_for_slot: Vec<u32>,
    /// What became of it ([`trip_outcome`]).
    pub outcome: Vec<&'static str>,
}

/// A shared-bike trip's outcome: `not_requested` (it reached the pickup dock after the
/// window's end), `completed` (got a bike and returned it), `cancelled` (found no bike at any
/// dock tried: it walked from the last), `unfinished` (still looking for a bike or riding when
/// the window ended).
#[must_use]
pub fn trip_outcome(status: LegStatus) -> &'static str {
    match status {
        LegStatus::NotRequested => "not_requested",
        LegStatus::Completed => "completed",
        LegStatus::Cancelled => "cancelled",
        LegStatus::Waiting | LegStatus::Riding | LegStatus::Unfinished => "unfinished",
    }
}

impl BikeShareTripTable {
    /// The trips of `day`, a run's last loading on `setups`, whose trips are `trips` of
    /// `travellers`, each with its trip's mode from `modes` (by trip index).
    ///
    /// # Panics
    ///
    /// If a leg's service or dock is not of `setups`.
    #[must_use]
    pub fn new(
        day: &ShareDay,
        setups: &[ServiceSetup],
        trips: &Trips,
        travellers: &Travellers,
        modes: &[Option<Mode>],
    ) -> Self {
        let mut t = Self::default();
        let people = who(day, trips, travellers);
        for ((l, o), (traveller, seq)) in day.legs.iter().zip(&day.outcome.legs).zip(people) {
            let setup = &setups[usize::from(l.leg.service)];
            let ids = dock_ids(setup);
            let id = |d: Option<u32>| d.map(|d| ids[d as usize].clone());
            t.service.push(setup.name().to_string());
            t.traveller_id.push(traveller);
            t.trip_seq.push(seq);
            t.travellers.push(l.leg.weight);
            t.mode.push(modes[l.trip.index()].map_or("", Mode::as_str));
            t.planned_pickup_dock.push(ids[l.leg.pickup_dock as usize].clone());
            t.planned_return_dock.push(ids[l.leg.return_dock as usize].clone());
            t.planned_at_pickup_dock_s.push(l.leg.request_s);
            let planned = l.planned_arrival_s(setup);
            t.planned_leg_end_s.push(planned);
            t.pickup_dock.push(id(o.picked));
            t.pickup_s.push(o.picked.and(finite(o.picked_s)));
            t.docks_tried_for_bike.push(o.failed_for_bike + u32::from(o.picked.is_some()));
            t.return_dock.push(id(o.returned));
            t.return_s.push(o.returned.and(finite(o.returned_s)));
            t.docks_tried_for_slot
                .push(o.failed_for_slot + u32::from(o.returned.is_some() && !o.overflow));
            let arrived = matches!(o.status, LegStatus::Completed | LegStatus::Cancelled)
                && o.arrived_s.is_finite();
            t.leg_end_s.push(arrived.then_some(o.arrived_s));
            let extra = arrived.then_some(o.arrived_s - planned);
            t.extra_s.push(extra);
            let parts = l.dock_extra_s(setup, o);
            t.extra_walk_at_pickup_s.push(parts.map(|(p, _)| p));
            t.extra_walk_at_return_s.push(parts.map(|(_, r)| r));
            t.extra_walk_m.push(o.extra_walk_m);
            t.failed_attempts_for_bike.push(o.failed_for_bike);
            t.failed_attempts_for_slot.push(o.failed_for_slot);
            t.outcome.push(trip_outcome(o.status));
        }
        t
    }

    /// How many trips.
    #[must_use]
    pub fn len(&self) -> usize {
        self.service.len()
    }

    /// Whether there is none.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.service.is_empty()
    }
}

/// One row per dock and time bin, service by service, dock by dock (in dock id order), bin by
/// bin: `bike_share_docks.parquet`. Counts of people are the travellers' weights.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BikeShareDockTable {
    /// The bikes of the services' fleets at the start of the day, all docks summed.
    pub fleet: u64,
    /// The service's name.
    pub service: Vec<String>,
    /// The dock's id.
    pub dock: Vec<String>,
    /// Its hub in the run's hub set.
    pub hub_id: Vec<String>,
    /// The bin's start, in seconds from midnight.
    pub bin_start_s: Vec<f64>,
    /// The bin's end (the window's end for the last bin).
    pub bin_end_s: Vec<f64>,
    /// The dock's slots.
    pub slots: Vec<u32>,
    /// Bikes docked when the bin began.
    pub bikes_at_start: Vec<f64>,
    /// Bikes docked when it ended.
    pub bikes_at_end: Vec<f64>,
    /// The fewest docked at once in the bin.
    pub bikes_min: Vec<f64>,
    /// The time-weighted mean docked over the bin.
    pub bikes_mean: Vec<f64>,
    /// Seconds of the bin it held no bike.
    pub seconds_empty: Vec<f64>,
    /// Seconds of the bin every slot was taken.
    pub seconds_full: Vec<f64>,
    /// Bikes taken.
    pub bikes_taken: Vec<f64>,
    /// Bikes returned (above capacity included).
    pub bikes_returned: Vec<f64>,
    /// Travellers who found no bike here and went on to another dock.
    pub people_failed_attempt_no_bike: Vec<f64>,
    /// Travellers who found no bike here nor at the docks near, and walked to their destination.
    pub people_gave_up_no_bike: Vec<f64>,
    /// Travellers who found no free slot here and rode on to another dock.
    pub people_failed_attempt_no_slot: Vec<f64>,
    /// Travellers who found no free slot here, nor at the docks near, and left the bike here
    /// above capacity.
    pub people_left_bike_over_capacity: Vec<f64>,
    /// Travellers sent here from another dock, empty or full (counted in the bin they were
    /// sent).
    pub people_sent_here: Vec<f64>,
    /// Bikes an operator's van brought (0 without an operator).
    pub bikes_brought_by_van: Vec<f64>,
    /// Bikes an operator's van took away (0 without an operator).
    pub bikes_taken_by_van: Vec<f64>,
}

impl BikeShareDockTable {
    /// The docks of `setups` over `outcome`'s day, with their hubs from `hubs`
    /// ([`resource_hubs`]). A service without docks adds no row.
    ///
    /// # Panics
    ///
    /// If `outcome` is not of `setups`.
    #[must_use]
    pub fn new(
        outcome: &FleetOutcome,
        setups: &[ServiceSetup],
        hubs: &[Vec<(String, &'static str)>],
    ) -> Self {
        let mut t = Self::default();
        for (s, setup) in setups.iter().enumerate() {
            let (ServiceParts::BikeDocked(docks, _), Some(Some(b))) =
                (setup.parts(), outcome.docks.get(s))
            else {
                continue;
            };
            t.fleet += docks.fleet();
            for (k, (hub, _)) in hubs[s].iter().enumerate().take(docks.len()) {
                for bin in 0..b.bins {
                    let cell = k * b.bins + bin;
                    #[allow(clippy::cast_precision_loss, reason = "a count of bins")]
                    let start = b.start_s + f64::from(b.bin_s) * bin as f64;
                    t.service.push(setup.name().to_string());
                    t.dock.push(docks.external[k].clone());
                    t.hub_id.push(hub.clone());
                    t.bin_start_s.push(start);
                    t.bin_end_s.push(start + b.bin_len(bin));
                    t.slots.push(docks.capacity[k]);
                    t.bikes_at_start.push(if bin == 0 {
                        f64::from(docks.start[k])
                    } else {
                        b.bikes_end[cell - 1]
                    });
                    t.bikes_at_end.push(b.bikes_end[cell]);
                    t.bikes_min.push(b.bikes_min[cell]);
                    t.bikes_mean.push(b.bikes_mean[cell]);
                    t.seconds_empty.push(b.empty_s[cell]);
                    t.seconds_full.push(b.full_s[cell]);
                    t.bikes_taken.push(b.pickups[cell]);
                    t.bikes_returned.push(b.returns[cell]);
                    t.people_failed_attempt_no_bike
                        .push(b.redirected[cell] - b.redirected_no_slot[cell]);
                    t.people_gave_up_no_bike.push(b.no_bike[cell]);
                    t.people_failed_attempt_no_slot.push(b.redirected_no_slot[cell]);
                    t.people_left_bike_over_capacity.push(b.over_capacity[cell]);
                    t.people_sent_here.push(b.redirected_in[cell]);
                    t.bikes_brought_by_van.push(b.moved_in[cell]);
                    t.bikes_taken_by_van.push(b.moved_out[cell]);
                }
            }
        }
        t
    }

    /// How many rows.
    #[must_use]
    pub fn len(&self) -> usize {
        self.service.len()
    }

    /// Whether there is none.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.service.is_empty()
    }
}
