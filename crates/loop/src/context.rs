//! What every stage reads about the run: the inputs, the window and, per trip, its departure
//! on the window's clock, its class and the modes open to it in this run.

use std::collections::HashMap;

use openmobisim_demand::demand::{Demand, End};
use openmobisim_graph::network::Network;
use openmobisim_transit::timetable::Timetable;
use openmobisim_types::ids::{EntityId, NodeId, TripId, UserClassId};
use openmobisim_types::mode::{Mode, ModeSet};
use openmobisim_types::time::{Second, StepGrid};
use openmobisim_types::units::Duration;

use crate::parameters::Parameters;

/// The run's inputs and window, read by every stage.
pub(crate) struct Context<'a> {
    /// The network.
    pub network: &'a Network,
    /// The demand.
    pub demand: &'a Demand,
    /// The timetable, or none.
    pub timetable: Option<&'a Timetable>,
    /// The parameters.
    pub parameters: &'a Parameters,
    /// The window, its step the link bin.
    pub window: StepGrid,
    /// Per trip, its departure in seconds after the window's start.
    pub departure: Vec<f64>,
    /// Per trip, its class as an index.
    pub class: Vec<usize>,
    /// Per trip, the modes of its set the window can hold: none for a trip the run dropped
    /// (`crate::run`, "Repairs").
    pub modes: Vec<ModeSet>,
    /// Per trip with cars open to it, its origin-destination pair for cars (the trips with the
    /// same car origin node and destination node), by number; `None` for the others.
    pub car_pair: Vec<Option<usize>>,
    /// Per pair, its trips in trip order.
    pub car_pair_trips: Vec<Vec<usize>>,
}

impl Context<'_> {
    /// Number the trips' car origin-destination pairs, in the order of their first trip, into
    /// `car_pair` and `car_pair_trips`: once per run, since a trip's ends do not change.
    pub fn number_car_pairs(&mut self) {
        let mut number: HashMap<(NodeId, NodeId), usize> = HashMap::new();
        let (mut pair, mut trips) = (vec![None; self.trip_count()], Vec::<Vec<usize>>::new());
        for (i, of) in pair.iter_mut().enumerate() {
            let Some((o, d)) = self.ends(i, Mode::Car) else { continue };
            let p = *number.entry((o.node, d.node)).or_insert(trips.len());
            if p == trips.len() {
                trips.push(Vec::new());
            }
            trips[p].push(i);
            *of = Some(p);
        }
        (self.car_pair, self.car_pair_trips) = (pair, trips);
    }

    /// How many trips.
    pub fn trip_count(&self) -> usize {
        self.departure.len()
    }

    /// The window's start.
    pub fn start(&self) -> Second {
        self.window.origin()
    }

    /// The window's length in seconds.
    pub fn length(&self) -> f64 {
        f64::from(self.window.end().get() - self.window.origin().get())
    }

    /// The seconds to walk `gap` metres at the network's walking speed.
    pub fn gap_walk(&self, gap: openmobisim_types::units::Metres) -> Duration {
        gap / self.network.walk_speed()
    }

    /// Trip `i`'s id.
    pub fn trip(&self, i: usize) -> TripId {
        TripId::from_index(i)
    }

    /// Whether trip `i` is run: it has a mode the window can hold.
    pub fn runs(&self, i: usize) -> bool {
        !self.modes[i].is_empty()
    }

    /// Trip `i`'s two ends by `mode`, if the mode is open to it in this run.
    pub fn ends(&self, i: usize, mode: Mode) -> Option<(End, End)> {
        if !self.modes[i].contains(mode) {
            return None;
        }
        let trip = self.trip(i);
        Some((self.demand.origin_end(trip, mode)?, self.demand.destination_end(trip, mode)?))
    }

    /// Trip `i`'s class id.
    pub fn class_id(&self, i: usize) -> UserClassId {
        UserClassId::from_index(self.class[i])
    }
}
