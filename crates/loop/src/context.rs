//! What every stage reads about the run: the inputs, the window and, per trip, its departure
//! on the window's clock and its class.

use openmobisim_demand::demand::Demand;
use openmobisim_graph::network::Network;
use openmobisim_transit::timetable::Timetable;
use openmobisim_types::ids::{EntityId, TripId, UserClassId};
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
}

impl Context<'_> {
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

    /// Trip `i`'s class id.
    pub fn class_id(&self, i: usize) -> UserClassId {
        UserClassId::from_index(self.class[i])
    }
}
