//! The results tables of a run's services (S302): one row per shared-bike leg of the last
//! loading ([`ServiceLegTable`], `service_legs.parquet`) and one row per dock and time bin
//! ([`HubBinTable`], `hub_bins.parquet`), each dock named with its hub in the run's hub set.
//!
//! **Which hub.** The run's hub set ([`RunHubs`]) is built with every service's resources in
//! service and dock order ([`ServiceSetup::resource_rows`]); [`resource_hubs`] reads each dock's
//! hub back from it in that order.
//!
//! **Cost:** about 150 bytes per leg and 120 per dock and bin while the tables are held.

use openmobisim_core_demand::{Mode, Travellers, Trips};
use openmobisim_core_graph::hubs::ResourceKind;
use openmobisim_core_types::ids::EntityId;

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

/// One row per service leg of a fleet day, in the order the day played them: what
/// `service_legs.parquet` holds. Seconds count from midnight; `None` is a time that did not
/// come (written as null).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ServiceLegTable {
    /// The service's name.
    pub service: Vec<String>,
    /// The trip's mode (`bike_share`, or `bike_share_transit` for a shared bike to transit).
    pub mode: Vec<&'static str>,
    /// The traveller's id, as given.
    pub traveller_id: Vec<String>,
    /// The trip's place in its traveller's day, from 0.
    pub trip_seq: Vec<u32>,
    /// The people the leg stands for (its traveller's weight): the bikes it takes.
    pub weight: Vec<u32>,
    /// The second it asked for a bike at its planned pickup dock.
    pub requested_s: Vec<Option<f64>>,
    /// The second a bike was in hand (`None`: none was).
    pub picked_s: Vec<Option<f64>>,
    /// The second it reached the leg's end (the trip's destination, or for a shared bike to
    /// transit the dock at the stop's hub), riding or, after a failed pickup, on foot.
    pub arrived_s: Vec<Option<f64>>,
    /// The hub of the pickup dock its choice planned.
    pub hub_from: Vec<String>,
    /// The hub of the return dock its choice planned.
    pub hub_to: Vec<String>,
    /// The dock the bike was taken from (`None`: none was).
    pub dock_from: Vec<Option<String>>,
    /// The dock the bike was returned to (`None`: not returned).
    pub dock_to: Vec<Option<String>>,
    /// Seconds waited for a vehicle: 0 for a docked bike, whose traveller walks on from an
    /// empty dock instead of waiting.
    pub waited_s: Vec<f64>,
    /// Seconds walked between docks looking for a bike.
    pub extra_walk_s: Vec<f64>,
    /// Docks tried after the planned one, at the pickup and at the return.
    pub redirections: Vec<u32>,
    /// The extra seconds at the docks its choice expected.
    pub expected_dock_s: Vec<f64>,
    /// The extra seconds it met: its arrival less its arrival had the planned pickup dock had a
    /// bike and the planned return dock a free slot (`None`: it did not arrive).
    pub met_dock_s: Vec<Option<f64>>,
    /// What became of it ([`LegStatus::as_str`]).
    pub outcome: Vec<&'static str>,
}

fn finite(v: f64) -> Option<f64> {
    v.is_finite().then_some(v)
}

impl ServiceLegTable {
    /// The legs of `day`, a run's last loading on `setups`, whose trips are `trips` of
    /// `travellers`, each with its trip's `modes` (by trip index) and its planned docks' hubs
    /// from `hubs` ([`resource_hubs`]).
    ///
    /// # Panics
    ///
    /// If a leg's service or dock is not of `setups` or `hubs`.
    #[must_use]
    pub fn new(
        day: &ShareDay,
        setups: &[ServiceSetup],
        hubs: &[Vec<(String, &'static str)>],
        trips: &Trips,
        travellers: &Travellers,
        modes: &[Option<Mode>],
    ) -> Self {
        let mut t = Self::default();
        for (l, o) in day.legs.iter().zip(&day.outcome.legs) {
            let s = usize::from(l.leg.service);
            let setup = &setups[s];
            let dock = |d: Option<u32>| match (setup.parts(), d) {
                (ServiceParts::BikeDocked(docks, _), Some(d)) => {
                    Some(docks.external[d as usize].clone())
                }
                _ => None,
            };
            let traveller = trips.traveller(l.trip);
            t.service.push(setup.name().to_string());
            t.mode.push(modes[l.trip.index()].map_or("", Mode::as_str));
            t.traveller_id.push(travellers.external_ids().external(traveller.raw()).to_string());
            t.trip_seq.push(l.trip.raw() - travellers.first_trip(traveller).raw());
            t.weight.push(l.leg.weight);
            t.requested_s.push(finite(o.requested_s));
            t.picked_s.push(o.picked.and(finite(o.picked_s)));
            t.arrived_s.push(finite(o.arrived_s));
            t.hub_from.push(hubs[s][l.leg.pickup_dock as usize].0.clone());
            t.hub_to.push(hubs[s][l.leg.return_dock as usize].0.clone());
            t.dock_from.push(dock(o.picked));
            t.dock_to.push(dock(o.returned));
            t.waited_s.push(o.waited_s);
            t.extra_walk_s.push(o.extra_walk_s);
            t.redirections.push(o.redirections);
            t.expected_dock_s.push(l.expected_dock_s);
            let arrived = matches!(o.status, LegStatus::Served | LegStatus::Failed)
                && o.arrived_s.is_finite();
            t.met_dock_s.push(arrived.then(|| o.arrived_s - l.planned_arrival_s(setup)));
            t.outcome.push(o.status.as_str());
        }
        t
    }

    /// How many legs.
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

/// One row per dock and time bin of a fleet day, service by service, dock by dock (in dock id
/// order), bin by bin: what `hub_bins.parquet` holds. Counts are of people (bikes).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HubBinTable {
    /// The bin's length, in seconds.
    pub bin_s: u32,
    /// The bin's start, in seconds from midnight.
    pub start_s: Vec<u32>,
    /// The dock's hub in the run's hub set.
    pub hub_id: Vec<String>,
    /// The hub's kind (`stop`: a stop's hub the dock joined; `declared`: a hub of its own).
    pub hub_kind: Vec<&'static str>,
    /// The resource's kind (`dock`).
    pub resource_kind: Vec<&'static str>,
    /// The dock's id.
    pub resource_id: Vec<String>,
    /// The service's name.
    pub service: Vec<String>,
    /// The mean bikes docked over the bin.
    pub level_mean: Vec<f64>,
    /// The fewest bikes docked at once in the bin.
    pub level_min: Vec<f64>,
    /// The dock's slots.
    pub capacity: Vec<u32>,
    /// Seconds of the bin it held no bike.
    pub empty_s: Vec<f64>,
    /// Seconds of the bin its slots were all taken.
    pub full_s: Vec<f64>,
    /// Bikes taken.
    pub pickups: Vec<f64>,
    /// Bikes returned.
    pub returns: Vec<f64>,
    /// Pickups that failed here, the last dock tried.
    pub failed: Vec<f64>,
    /// Travellers sent on to this dock from another empty or full one.
    pub redirected_in: Vec<f64>,
    /// Travellers who found it empty (picking up) or full (returning) and went on.
    pub redirected_out: Vec<f64>,
    /// Bikes an operator brought (0 without an operator).
    pub moved_in: Vec<f64>,
    /// Bikes an operator took away (0 without an operator).
    pub moved_out: Vec<f64>,
}

impl HubBinTable {
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
            t.bin_s = b.bin_s;
            for (k, (hub, kind)) in hubs[s].iter().enumerate().take(docks.len()) {
                for bin in 0..b.bins {
                    let cell = k * b.bins + bin;
                    let bin32 = u32::try_from(bin).expect("bins fit u32");
                    #[allow(
                        clippy::cast_possible_truncation,
                        clippy::cast_sign_loss,
                        reason = "s"
                    )]
                    t.start_s.push(b.start_s as u32 + bin32 * b.bin_s);
                    t.hub_id.push(hub.clone());
                    t.hub_kind.push(kind);
                    t.resource_kind.push(ResourceKind::Dock.as_str());
                    t.resource_id.push(docks.external[k].clone());
                    t.service.push(setup.name().to_string());
                    t.level_mean.push(b.bikes_mean[cell]);
                    t.level_min.push(b.bikes_min[cell]);
                    t.capacity.push(docks.capacity[k]);
                    t.empty_s.push(b.empty_s[cell]);
                    t.full_s.push(b.full_s[cell]);
                    t.pickups.push(b.pickups[cell]);
                    t.returns.push(b.returns[cell]);
                    t.failed.push(b.failed[cell]);
                    t.redirected_in.push(b.redirected_in[cell]);
                    t.redirected_out.push(b.redirected[cell]);
                    t.moved_in.push(b.moved_in[cell]);
                    t.moved_out.push(b.moved_out[cell]);
                }
            }
        }
        t
    }

    /// How many rows.
    #[must_use]
    pub fn len(&self) -> usize {
        self.start_s.len()
    }

    /// Whether there is none.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.start_s.is_empty()
    }
}
