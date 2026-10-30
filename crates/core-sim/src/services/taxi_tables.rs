//! The results tables of a run's taxi fleet (S302), from its service day (the last loading's
//! taxi trips, played once after the run has stopped, S303):
//!
//! - [`TaxiEventTable`] (`taxi_events.parquet`): the day's log, one row per event in time order,
//!   with the vehicle's state and charge just after, so each traveller's and each vehicle's
//!   sequence can be followed;
//! - [`TaxiTripTable`] (`taxi_trips.parquet`): one row per request (a trip that chose a taxi):
//!   when it was requested, the wait its traveller assumed, and what the dispatcher saw: its
//!   outcome ([`taxi_trip_outcome`]), the taxi, the pickup and drop-off, the waiting and
//!   in-vehicle times and the kilometres with and without the passenger;
//! - [`TaxiVehicleTable`] (`taxi_vehicles.parquet`): one row per vehicle and time bin: the
//!   seconds in each state, the kilometres occupied and empty, the charge and the charging.
//!
//! **One seat per unit request.** A trip standing for `w` people asks for `w` vehicles at once
//! (the engine's unit requests, no pooling); each unit request has its own events, with
//! `travellers` 1. A trip's row describes its last unit request (the one the engine's leg
//! outcome follows), and counts `travellers` people.
//!
//! **Cost:** about 100 bytes per event, 250 per trip and 200 per vehicle and bin while held.

use openmobisim_core_demand::{Travellers, Trips};
use openmobisim_core_types::ids::EntityId;

use super::ServiceSetup;
use super::engine::VehicleState;
use super::in_run::TaxiRunDay;
use super::outcome::LegStatus;
use super::taxis::NO;
use super::{ServiceParts, TaxiDefaults};

fn finite(v: f64) -> Option<f64> {
    v.is_finite().then_some(v)
}

/// Who each leg of `day` is: its traveller's id and the trip's place in their day.
fn who(day: &TaxiRunDay, trips: &Trips, travellers: &Travellers) -> Vec<(String, u32)> {
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

fn parts(setup: &ServiceSetup) -> Option<(&super::Fleet, &TaxiDefaults)> {
    match setup.parts() {
        ServiceParts::Taxi(fleet, d) => Some((fleet, d)),
        ServiceParts::BikeDocked(..) => None,
    }
}

/// The day's log of events, one row per event in time order: `taxi_events.parquet`. `None` is a
/// field that does not apply (written as null).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TaxiEventTable {
    /// The fleet's name.
    pub service: Vec<String>,
    /// When, in seconds from midnight.
    pub time_s: Vec<f64>,
    /// What happened ([`super::taxis::TaxiEventKind::as_str`]).
    pub event: Vec<&'static str>,
    /// The traveller (`None`: a vehicle's own event).
    pub traveller_id: Vec<Option<String>>,
    /// The trip's place in the traveller's day, from 0.
    pub trip_seq: Vec<Option<u32>>,
    /// The people the row stands for: 1 per unit request (`None`: a vehicle's own event).
    pub travellers: Vec<Option<u32>>,
    /// The vehicle.
    pub vehicle: Vec<Option<String>>,
    /// The vehicle's state just after ([`VehicleState::as_str`]).
    pub vehicle_state_after: Vec<Option<&'static str>>,
    /// The vehicle's charge just after, as a share of its battery.
    pub charge_after: Vec<Option<f64>>,
    /// The charger.
    pub charger: Vec<Option<String>>,
    /// The kilometres of the drive that starts with the event (to the pickup, with the
    /// passenger, to a charger).
    pub drive_km: Vec<Option<f64>>,
}

impl TaxiEventTable {
    /// The events of `day` of the fleet `setup`, whose trips are `trips` of `travellers`; empty
    /// if the day kept no log. A request no taxi could pick up within the cancel limit ends with
    /// `request_cancelled`.
    ///
    /// # Panics
    ///
    /// If an event's unit request, vehicle or charger is not of `day` or `setup`.
    #[must_use]
    pub fn new(
        day: &TaxiRunDay,
        setup: &ServiceSetup,
        trips: &Trips,
        travellers: &Travellers,
    ) -> Self {
        let mut t = Self::default();
        let (Some(log), Some((fleet, _))) = (&day.day.log, parts(setup)) else { return t };
        let people = who(day, trips, travellers);
        let leg_of = |unit: u32| {
            (unit != NO)
                .then(|| day.day.requests[unit as usize].leg as usize)
                .and_then(|l| l.checked_sub(day.offset))
        };
        for e in log {
            let leg = leg_of(e.unit);
            t.service.push(setup.name().to_string());
            t.time_s.push(e.t);
            t.event.push(e.kind.as_str());
            t.traveller_id.push(leg.map(|l| people[l].0.clone()));
            t.trip_seq.push(leg.map(|l| people[l].1));
            t.travellers.push(leg.map(|_| 1));
            t.vehicle
                .push((e.vehicle != NO).then(|| fleet.vehicle_ids[e.vehicle as usize].clone()));
            t.vehicle_state_after.push(e.state_after.map(VehicleState::as_str));
            t.charge_after.push(finite(e.soc_after));
            t.charger
                .push((e.charger != NO).then(|| fleet.charger_ids[e.charger as usize].clone()));
            t.drive_km.push(finite(e.km));
        }
        t
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

/// A request's outcome: `not_requested` (its departure came after the window's end),
/// `completed` (picked up and dropped off at the destination), `cancelled` (no taxi could pick
/// it up within the cancel limit: the trip ends there), `unfinished` (still waiting or riding
/// when the window ended).
#[must_use]
pub fn taxi_trip_outcome(status: LegStatus) -> &'static str {
    match status {
        LegStatus::NotRequested => "not_requested",
        LegStatus::Completed => "completed",
        LegStatus::Cancelled => "cancelled",
        LegStatus::Waiting | LegStatus::Riding | LegStatus::Unfinished => "unfinished",
    }
}

/// One row per request (a trip that chose a taxi), in trip order: `taxi_trips.parquet`.
/// Seconds count from midnight; `None` is what did not happen (written as null).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TaxiTripTable {
    /// The fleet's name.
    pub service: Vec<String>,
    /// The traveller's id, as given.
    pub traveller_id: Vec<String>,
    /// The trip's place in its traveller's day, from 0.
    pub trip_seq: Vec<u32>,
    /// The people the traveller stands for: the seats the trip asks for.
    pub travellers: Vec<u32>,
    /// When the taxi was requested: the trip's departure.
    pub requested_s: Vec<f64>,
    /// The waiting time its traveller assumed when choosing, in seconds.
    pub assumed_wait_s: Vec<f64>,
    /// What became of it ([`taxi_trip_outcome`]).
    pub outcome: Vec<&'static str>,
    /// The taxi matched to it.
    pub vehicle: Vec<Option<String>>,
    /// When the taxi reached the pickup.
    pub pickup_s: Vec<Option<f64>>,
    /// When the passenger was dropped off at the destination.
    pub dropoff_s: Vec<Option<f64>>,
    /// The waiting time, in seconds: from the request to the pickup, the dispatch delay
    /// included (`None`: not picked up).
    pub waiting_s: Vec<Option<f64>>,
    /// The time in the vehicle, in seconds: from the pickup to the drop-off, boarding included.
    pub in_vehicle_s: Vec<Option<f64>>,
    /// The kilometres driven with the passenger on board.
    pub ride_km: Vec<Option<f64>>,
    /// The kilometres the taxi drove empty to the pickup.
    pub empty_km: Vec<Option<f64>>,
}

impl TaxiTripTable {
    /// The trips of `day` of the fleet `setup`, whose trips are `trips` of `travellers`.
    ///
    /// # Panics
    ///
    /// If a leg's vehicle is not of `setup`.
    #[must_use]
    pub fn new(
        day: &TaxiRunDay,
        setup: &ServiceSetup,
        trips: &Trips,
        travellers: &Travellers,
    ) -> Self {
        let mut t = Self::default();
        let Some((fleet, _)) = parts(setup) else { return t };
        // Each leg's last unit request: the one its outcome follows.
        let mut last_unit = vec![None; day.legs.len()];
        for r in &day.day.requests {
            if let Some(i) = (r.leg as usize).checked_sub(day.offset) {
                if let Some(slot) = last_unit.get_mut(i) {
                    *slot = Some(*r);
                }
            }
        }
        let people = who(day, trips, travellers);
        for (i, ((l, o), (traveller, seq))) in
            day.legs.iter().zip(&day.outcome).zip(people).enumerate()
        {
            let r = last_unit[i];
            t.service.push(setup.name().to_string());
            t.traveller_id.push(traveller);
            t.trip_seq.push(seq);
            t.travellers.push(l.leg.weight);
            t.requested_s.push(l.leg.request_s);
            t.assumed_wait_s.push(l.assumed_wait_s);
            t.outcome.push(taxi_trip_outcome(o.status));
            t.vehicle.push(o.vehicle.map(|v| fleet.vehicle_ids[v as usize].clone()));
            let pickup = finite(o.picked_s);
            t.pickup_s.push(pickup);
            let dropoff =
                (o.status == LegStatus::Completed).then_some(o.arrived_s).and_then(finite);
            t.dropoff_s.push(dropoff);
            t.waiting_s.push(pickup.map(|_| o.waited_s));
            t.in_vehicle_s.push(pickup.zip(dropoff).map(|(p, d)| d - p));
            t.ride_km.push(r.filter(|r| r.picked_s.is_finite()).map(|r| r.trip_km));
            t.empty_km.push(r.filter(|r| r.vehicle.is_some()).map(|r| r.empty_km));
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

/// One row per vehicle and time bin, vehicle by vehicle (in the fleet's order), bin by bin:
/// `taxi_vehicles.parquet`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TaxiVehicleTable {
    /// The vehicles of the fleet.
    pub fleet: u64,
    /// The fleet's name.
    pub service: Vec<String>,
    /// The vehicle's id.
    pub vehicle: Vec<String>,
    /// The bin's start, in seconds from midnight.
    pub bin_start_s: Vec<f64>,
    /// The bin's end (the window's end for the last bin).
    pub bin_end_s: Vec<f64>,
    /// Seconds of the bin in each state, in [`VehicleState::ALL`]'s order: idle, driving to a
    /// pickup, occupied (boarding and carrying), relocating, driving to a charger, charging,
    /// queued for a plug.
    pub seconds: [Vec<f64>; 7],
    /// Kilometres with a passenger (a drive's spread over its seconds).
    pub km_occupied: Vec<f64>,
    /// Kilometres empty: to pickups, to chargers, relocations.
    pub km_empty: Vec<f64>,
    /// The charge at the bin's start, as a share of the battery.
    pub charge_at_start: Vec<f64>,
    /// The charge at its end.
    pub charge_at_end: Vec<f64>,
    /// Energy charged, in kWh (a charge's spread over its seconds).
    pub kwh_charged: Vec<f64>,
    /// Passengers dropped off.
    pub passengers_dropped_off: Vec<f64>,
}

impl TaxiVehicleTable {
    /// The vehicles of the fleet `setup` over `day` (`None`: a day without taxi trips, every
    /// vehicle idle where it stood, at its start-of-day charge, over `window`).
    #[must_use]
    pub fn new(day: Option<&TaxiRunDay>, setup: &ServiceSetup, window: (f64, f64)) -> Self {
        let mut t = Self::default();
        let Some((fleet, _)) = parts(setup) else { return t };
        t.fleet = fleet.vehicle_ids.len() as u64;
        let len = f64::from(super::taxis::FLEET_BIN_S);
        let window_s = (window.1 - window.0).max(0.0);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "a bin count")]
        let idle_bins = ((window_s / len).ceil() as usize).max(1);
        for (v, id) in fleet.vehicle_ids.iter().enumerate() {
            let bins = day.map_or(idle_bins, |d| d.day.vehicle_bins.bins);
            for b in 0..bins {
                #[allow(clippy::cast_precision_loss, reason = "a bin index")]
                let start = window.0 + len * b as f64;
                let end = if b + 1 == bins { window.1.max(start) } else { start + len };
                t.service.push(setup.name().to_string());
                t.vehicle.push(id.clone());
                t.bin_start_s.push(start);
                t.bin_end_s.push(end);
                match day {
                    Some(d) => {
                        let vb = &d.day.vehicle_bins;
                        let c = v * vb.bins + b;
                        for (k, col) in t.seconds.iter_mut().enumerate() {
                            col.push(vb.state_s[c][k]);
                        }
                        t.km_occupied.push(vb.km_occupied[c]);
                        t.km_empty.push(vb.km_empty[c]);
                        t.charge_at_start.push(vb.soc_start[c]);
                        t.charge_at_end.push(vb.soc_end[c]);
                        t.kwh_charged.push(vb.kwh_charged[c]);
                        t.passengers_dropped_off.push(vb.dropped_off[c]);
                    }
                    None => {
                        for (k, col) in t.seconds.iter_mut().enumerate() {
                            col.push(if k == VehicleState::Idle as usize {
                                end - start
                            } else {
                                0.0
                            });
                        }
                        t.km_occupied.push(0.0);
                        t.km_empty.push(0.0);
                        t.charge_at_start.push(fleet.vehicle_soc[v]);
                        t.charge_at_end.push(fleet.vehicle_soc[v]);
                        t.kwh_charged.push(0.0);
                        t.passengers_dropped_off.push(0.0);
                    }
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
