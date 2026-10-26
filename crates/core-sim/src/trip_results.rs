//! Every trip's result in the last loading: what became of it, when it arrived, and where a
//! trip still travelling at the window's end was. [`TripOutcomes`] is what the run keeps
//! (three values per trip); [`TripTable`] adds who, from where to where, when and by which mode,
//! as `trip_results.parquet` and `Run.trip_results()` give it.
//!
//! Cost: one pass over the last loading's events and one row per trip, once per run; the
//! loading itself reads where each unfinished vehicle is once, at its end.

use openmobisim_core_demand::{Mode, Travellers, Trips};
use openmobisim_core_graph::geometry::LonLat;
use openmobisim_core_types::ids::{EntityId, LinkId, TripId};
use openmobisim_core_types::time::Second;

use crate::events::{EventRow, EventType};
use crate::itinerary_choice::{NO_MODE, NoAlternative};
use crate::run::RunResult;

/// What became of one trip: the outcomes of [`crate::TripCompletionStats`], and a trip that
/// chose its mode and had none to choose.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum TripOutcome {
    /// Arrived within the window.
    Completed,
    /// Still travelling when the window ended.
    Truncated,
    /// Its mode needed a vehicle its traveller did not have at the origin.
    NoVehicleAvailable,
    /// No path reached its destination.
    NoFeasiblePath,
    /// The run lacks what its mode needs (a layer, a timetable, parkings).
    ModeNotAvailable,
    /// It had no alternative: no mode it may use reached its destination within the limits
    /// ([`RunResult::no_alternative`], not counting a trip that then travelled by car). A trip
    /// that chose its mode has none; one given transit, park-and-ride or bike-and-ride keeps it.
    /// [`crate::TripCompletionStats`] counts it under the event it had (most often
    /// `no_feasible_path`).
    #[default]
    NoAlternative,
}

impl TripOutcome {
    /// The stable snake_case name written to `trip_results.parquet`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            TripOutcome::Completed => "completed",
            TripOutcome::Truncated => "truncated",
            TripOutcome::NoVehicleAvailable => "no_vehicle_available",
            TripOutcome::NoFeasiblePath => "no_feasible_path",
            TripOutcome::ModeNotAvailable => "mode_not_available",
            TripOutcome::NoAlternative => "no_alternative",
        }
    }
}

/// Every trip's outcome in the last loading, by trip index.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TripOutcomes {
    /// What became of each trip.
    pub outcome: Vec<TripOutcome>,
    /// When a completed trip arrived; `None` for the others.
    pub arrival: Vec<Option<Second>>,
    /// For a trip still travelling when the window ended whose car was on the roads: the road
    /// link it was on (driving, or at the link's stop line) or waited at its origin to enter.
    /// `None` for every other trip, and for one travelling by another means at the end (on
    /// foot, by bike, in a transit vehicle or waiting at a stop).
    pub link_at_end: Vec<Option<LinkId>>,
    /// For a completed trip, its seconds in a vehicle of its own: the car's drive or the bike's
    /// ride, in a direct trip or a trip through a parking; 0 for a walk or a transit trip
    /// without one. `None` for a trip that did not complete.
    pub vehicle_s: Vec<Option<u32>>,
    /// For a completed trip, its fixed seconds around that vehicle: reaching it, leaving it and
    /// its parking's time (`mode_options` `car_access_s`, `car_egress_s`, `bike_access_s`,
    /// `bike_egress_s`; `parking_options` the slopes); 0 without a vehicle of its own. `None` for a
    /// trip that did not complete.
    pub access_egress_s: Vec<Option<u32>>,
}

impl TripOutcomes {
    /// From the last loading's events (one per trip), the mode each trip took (`None`: it
    /// chose and had none), the trips the last choice left with no alternative, the road
    /// link of each trip still travelling at the end, and each completed trip's seconds in its
    /// own vehicle and around it (`parts`, for the trips with a vehicle of their own).
    #[must_use]
    pub fn of(
        events: &[EventRow],
        taken: &[Option<Mode>],
        no_alternative: &[NoAlternative],
        at_end: &[(TripId, LinkId)],
        parts: &[(TripId, u32, u32)],
    ) -> Self {
        let n = taken.len();
        let mut out = TripOutcomes {
            outcome: vec![TripOutcome::NoAlternative; n],
            arrival: vec![None; n],
            link_at_end: vec![None; n],
            vehicle_s: vec![None; n],
            access_egress_s: vec![None; n],
        };
        for e in events {
            let t = e.entity_id as usize;
            out.outcome[t] = match e.event_type {
                EventType::TripCompleted => {
                    out.arrival[t] = Some(e.second);
                    out.vehicle_s[t] = Some(0);
                    out.access_egress_s[t] = Some(0);
                    TripOutcome::Completed
                }
                EventType::TripTruncated => TripOutcome::Truncated,
                EventType::NoVehicleAvailable => TripOutcome::NoVehicleAvailable,
                EventType::NoFeasiblePath => TripOutcome::NoFeasiblePath,
                EventType::ModeNotAvailable => TripOutcome::ModeNotAvailable,
            };
        }
        let without = no_alternative.iter().filter(|n| !n.fallback_car).map(|n| n.trip.index());
        let none = taken.iter().enumerate().filter(|(_, m)| m.is_none()).map(|(t, _)| t);
        for t in without.chain(none) {
            out.outcome[t] = TripOutcome::NoAlternative;
            out.arrival[t] = None;
        }
        for &(trip, link) in at_end {
            if out.outcome[trip.index()] == TripOutcome::Truncated {
                out.link_at_end[trip.index()] = Some(link);
            }
        }
        for &(trip, vehicle_s, fixed_s) in parts {
            if out.outcome[trip.index()] == TripOutcome::Completed {
                out.vehicle_s[trip.index()] = Some(vehicle_s);
                out.access_egress_s[trip.index()] = Some(fixed_s);
            }
        }
        out
    }
}

/// One row per trip of a run, in trip order: what `trip_results.parquet` holds and
/// `Run.trip_results()` returns.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TripTable {
    /// Its traveller's id, as given.
    pub traveller_id: Vec<String>,
    /// Its place in its traveller's day, from 0.
    pub trip_seq: Vec<u32>,
    /// Its traveller's class, as given.
    pub user_class: Vec<String>,
    /// The people its traveller stands for.
    pub weight: Vec<u32>,
    /// Where it starts, as given.
    pub origin: Vec<LonLat>,
    /// Where it ends, as given.
    pub destination: Vec<LonLat>,
    /// When it departs, in seconds after midnight.
    pub departure_s: Vec<u32>,
    /// The mode it took: its stated mode, a trip without one counting as the run's one mode
    /// or else a car; with mode choice, the mode chosen; `None` for a trip that chose and had
    /// none to choose.
    pub mode: Vec<Option<Mode>>,
    /// Whether it chose its mode.
    pub mode_choice: Vec<bool>,
    /// What became of it.
    pub outcome: Vec<TripOutcome>,
    /// When it arrived, if it completed, in seconds after midnight.
    pub arrival_s: Vec<Option<u32>>,
    /// The road link where it was at the window's end ([`TripOutcomes::link_at_end`]), as an
    /// index into the network's links.
    pub link_at_end: Vec<Option<u32>>,
    /// Its seconds in a vehicle of its own, if it completed ([`TripOutcomes::vehicle_s`]).
    pub vehicle_s: Vec<Option<u32>>,
    /// Its fixed seconds around that vehicle, if it completed
    /// ([`TripOutcomes::access_egress_s`]).
    pub access_egress_s: Vec<Option<u32>>,
}

impl TripTable {
    /// The table of `result`'s trips. `unstated` is the one mode a run with a single mode gives
    /// the trips without one (`None`: a car).
    ///
    /// # Panics
    ///
    /// Panics if `result` was not made on `trips` (its outcomes have another length).
    #[must_use]
    pub fn new(
        result: &RunResult,
        trips: &Trips,
        travellers: &Travellers,
        unstated: Option<Mode>,
    ) -> Self {
        let n = trips.len() as usize;
        let outcomes = &result.trip_outcomes;
        assert_eq!(outcomes.outcome.len(), n, "the result is of these trips");
        let stated = |trip: TripId| match unstated {
            Some(one) if !trips.mode_given(trip) => one,
            _ => trips.mode(trip),
        };
        let mut mode: Vec<Option<Mode>> =
            (0..trips.len()).map(|t| Some(stated(TripId::new(t)))).collect();
        let mut mode_choice = vec![false; n];
        if let Some(itin) = &result.itineraries {
            for (i, &t) in itin.trip.iter().enumerate() {
                let t = t as usize;
                mode_choice[t] = itin.choosing[i];
                match (itin.mode[i], itin.choosing[i]) {
                    (NO_MODE, true) => mode[t] = None,
                    (NO_MODE, false) => {}
                    (m, _) => mode[t] = Some(Mode::ALL[m as usize]),
                }
            }
        }
        let mut table = TripTable {
            traveller_id: Vec::with_capacity(n),
            trip_seq: Vec::with_capacity(n),
            user_class: Vec::with_capacity(n),
            weight: Vec::with_capacity(n),
            origin: Vec::with_capacity(n),
            destination: Vec::with_capacity(n),
            departure_s: Vec::with_capacity(n),
            mode,
            mode_choice,
            outcome: outcomes.outcome.clone(),
            arrival_s: outcomes.arrival.iter().map(|a| a.map(Second::get)).collect(),
            link_at_end: outcomes.link_at_end.iter().map(|l| l.map(LinkId::raw)).collect(),
            vehicle_s: outcomes.vehicle_s.clone(),
            access_egress_s: outcomes.access_egress_s.clone(),
        };
        for t in 0..trips.len() {
            let trip = TripId::new(t);
            let traveller = trips.traveller(trip);
            table
                .traveller_id
                .push(travellers.external_ids().external(traveller.raw()).to_string());
            table.trip_seq.push(t - travellers.first_trip(traveller).raw());
            let class = travellers.user_class(traveller).raw();
            table.user_class.push(travellers.class_external_ids().external(class).to_string());
            table.weight.push(travellers.weight(traveller));
            table.origin.push(trips.origin(trip));
            table.destination.push(trips.destination(trip));
            table.departure_s.push(trips.departure(trip).get());
        }
        table
    }

    /// How many trips.
    #[must_use]
    pub fn len(&self) -> usize {
        self.outcome.len()
    }

    /// Whether there are none.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.outcome.is_empty()
    }

    /// Each trip's travel time in seconds, if it completed: its arrival less its departure, door
    /// to door in a run that compares modes or has a timetable (the fixed times of its vehicle,
    /// its walks, waits and rides included); the drive alone in a run of stated car trips.
    #[must_use]
    pub fn travel_time_s(&self) -> Vec<Option<u32>> {
        self.arrival_s
            .iter()
            .zip(&self.departure_s)
            .map(|(a, &d)| a.map(|a| a.saturating_sub(d)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::itinerary_choice::NoAlternativeReason;

    /// Seven trips: arrived at 100 s, still travelling (on link 7), no vehicle, no path, one that
    /// chose and had nothing to choose (its event says no path; its mode is none), one given
    /// transit with no journey (listed with no alternative; it keeps its mode), and one listed
    /// that then went by car and arrived (its car's outcome stands).
    #[test]
    fn each_trip_gets_its_event_s_outcome_and_a_link_only_if_it_was_still_travelling() {
        let trip = TripId::new;
        let events = [
            EventRow::trip(Second(100), EventType::TripCompleted, trip(0)),
            EventRow::trip(Second(900), EventType::TripTruncated, trip(1)),
            EventRow::trip(Second(5), EventType::NoVehicleAvailable, trip(2)),
            EventRow::trip(Second(5), EventType::NoFeasiblePath, trip(3)),
            EventRow::trip(Second(5), EventType::NoFeasiblePath, trip(4)),
            EventRow::trip(Second(5), EventType::NoFeasiblePath, trip(5)),
            EventRow::trip(Second(300), EventType::TripCompleted, trip(6)),
        ];
        let car = Some(Mode::Car);
        let taken = [car, car, car, Some(Mode::Transit), None, Some(Mode::Transit), car];
        let listed = |t: u32, choosing: bool, fallback_car: bool| NoAlternative {
            trip: trip(t),
            choosing,
            reasons: [NoAlternativeReason::NotOffered; Mode::COUNT],
            fallback_car,
        };
        let none = [listed(4, true, false), listed(5, false, false), listed(6, true, true)];
        // A link listed for a trip that arrived (its vehicle's leg) is not its end.
        let at_end = [(trip(1), LinkId::new(7)), (trip(0), LinkId::new(3))];
        let out = TripOutcomes::of(&events, &taken, &none, &at_end, &[]);
        let names: Vec<&str> = out.outcome.iter().map(|o| o.as_str()).collect();
        assert_eq!(
            names,
            [
                "completed",
                "truncated",
                "no_vehicle_available",
                "no_feasible_path",
                "no_alternative",
                "no_alternative",
                "completed"
            ]
        );
        let at = |x| Some(Second(x));
        assert_eq!(out.arrival, vec![at(100), None, None, None, None, None, at(300)]);
        assert_eq!(out.link_at_end, vec![None, Some(LinkId::new(7)), None, None, None, None, None]);
    }
}
