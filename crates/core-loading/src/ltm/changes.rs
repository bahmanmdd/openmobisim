//! Timed capacity changes: a link narrowed or closed from a second on.

use openmobisim_core_types::ids::{EntityId, LinkId};
use openmobisim_core_types::units::Duration;

use super::LtmNetwork;

/// A link's capacity from a second on, as a share of its own (disruptions): `factor` 0
/// closes it (nothing passes until a later change opens it), 1 restores it. Made by the loading
/// when its clock reaches `time`; zero cost when there is none.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CapacityChange {
    /// The second it takes effect.
    pub time: f64,
    /// The link.
    pub link: LinkId,
    /// The share of the link's capacity, discharge and inflow alike, from then on.
    pub factor: f64,
}

/// What a closed link's rates are multiplied by: next to nothing, but finite, so no headway
/// becomes infinite and no arithmetic turns into `NaN`.
const CLOSED_FACTOR: f64 = 1e-9;

impl LtmNetwork<'_> {
    /// The same network with timed capacity changes (disruptions): a road narrowed or
    /// closed for a while by an accident or works. Each is made when the loading's clock
    /// reaches it — before any movement at that second — and the queues it touches (the link's
    /// own, those feeding it, the departures onto it) are looked at again, so a reopened link
    /// moves at once. A vehicle on a closed link waits there; behind it, the queue grows as it
    /// would behind any bottleneck.
    ///
    /// Costs nothing per movement when there is no change; one comparison per movement when
    /// there are.
    #[must_use]
    pub fn with_capacity_changes(mut self, mut changes: Vec<CapacityChange>) -> Self {
        changes.sort_by(|a, b| a.time.total_cmp(&b.time).then(a.link.cmp(&b.link)));
        if !changes.is_empty() {
            self.base_rates =
                self.discharge_rate.iter().copied().zip(self.inflow_rate.iter().copied()).collect();
        }
        self.changes = changes;
        self.next_change = 0;
        self
    }

    /// The time of the next capacity change, if one is left.
    pub(super) fn next_change_time(&self) -> Option<f64> {
        self.changes.get(self.next_change).map(|c| c.time)
    }

    /// Make the next capacity change, and look again at the queues it touches.
    pub(super) fn make_change(&mut self, t0: f64) {
        let change = self.changes[self.next_change];
        self.next_change += 1;
        let i = change.link.index();
        let (discharge, inflow) = self.base_rates[i];
        let factor = if change.factor > 0.0 { change.factor } else { CLOSED_FACTOR };
        self.discharge_rate[i] = discharge * factor;
        self.inflow_rate[i] = inflow * factor;
        let at = change.time;
        if change.factor <= 0.0 {
            // Closed from `at`: its headway clocks restart there. A headway counts from the last
            // vehicle in or out, and with none yet (the clocks at -inf) even a closed link's
            // tiny rate would let the first one pass at once.
            let curves = &mut self.curves[i];
            curves.set_last_entry(Duration(curves.last_entry().get().max(at)));
            curves.set_last_exit(Duration(curves.last_exit().get().max(at)));
        }
        // The link's own queue (its discharge), its departures (their inflow) and the queues
        // bound onto it from upstream (their inflow): each front's time, again.
        let n = self.links;
        let mut touched = vec![i, n + i];
        let network = self.network;
        for &up in network.in_links(network.link_from(change.link)) {
            touched.push(up.index());
            touched.push(2 * n + up.index());
        }
        for queue in touched {
            if !self.queues[queue].is_empty() {
                self.schedule_front(queue, t0, at);
            }
        }
    }
}
