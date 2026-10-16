//! Chained vehicles: the release rule of a vehicle that departs when another arrives.

use super::LtmNetwork;

/// Per slot: the slot released when it arrives, the release rule of a chained
/// slot, and every slot's departure (a chained one's once released).
#[derive(Debug, Default)]
pub(super) struct Chains {
    pub(super) next: Vec<u32>,
    pub(super) wait: Vec<f64>,
    pub(super) not_before: Vec<f64>,
    pub(super) start: Vec<f64>,
}

impl LtmNetwork<'_> {
    /// When the vehicle in `slot` departs: its own departure, or a chained
    /// vehicle's release.
    #[inline]
    pub(super) fn departure_of(&self, slot: u32) -> f64 {
        match &self.chains {
            Some(c) => c.start[slot as usize],
            None => f64::from(self.vehicles[slot as usize].departure.get()),
        }
    }
}
