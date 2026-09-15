//! A vehicle and the route it carries (S85).
//!
//! "The flow motor never sees routes" (design §10.4) — it receives turning
//! demand at each node and returns flows. A [`Vehicle`] is what carries a
//! route *into* the flow motor: an ordered sequence of directed links, read
//! link by link as the vehicle advances. At level 0 nothing reads
//! [`Vehicle::pcu`] — there are no curves to occupy yet — but the field is
//! part of what a vehicle *is* (S85: "occupying an interval of width = its
//! PCU × traveller weight", S94's individually-identified vehicles), not
//! something level 1's flow motor would need to add later.

use openmobisim_core_types::ids::LinkId;
use openmobisim_core_types::ids::VehicleId;
use openmobisim_core_types::time::Second;
use openmobisim_core_types::units::Pcu;

/// A single vehicle, carrying its own route.
///
/// Built by whatever assigned the route — a hand-authored test fixture for
/// now, a real path-finder from `core-sim` onward (S133). Nothing here
/// computes a route; a [`Vehicle`] already has one.
#[derive(Clone, Debug)]
pub struct Vehicle {
    /// This vehicle's identity.
    pub id: VehicleId,
    /// The links this vehicle traverses, in order. Never empty — a vehicle
    /// with nowhere to go is not a vehicle in the loading at all.
    pub route: Vec<LinkId>,
    /// This vehicle's weight on the curves: its own PCU (1 for a car, 2 by
    /// default for a bus, design §10.4) times the traveller weight it
    /// carries, already multiplied in by the caller. Unused at level 0.
    pub pcu: Pcu,
    /// When this vehicle enters the first link of its route.
    pub departure: Second,
}

impl Vehicle {
    /// A vehicle with a route, PCU and departure time.
    ///
    /// # Panics
    ///
    /// In debug builds, panics if `route` is empty — see the field's own
    /// documentation for why an empty route is not a valid vehicle.
    #[must_use]
    pub fn new(id: VehicleId, route: Vec<LinkId>, pcu: Pcu, departure: Second) -> Self {
        debug_assert!(!route.is_empty(), "a vehicle's route must not be empty");
        Self { id, route, pcu, departure }
    }
}
