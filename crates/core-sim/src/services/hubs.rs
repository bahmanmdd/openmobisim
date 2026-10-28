//! The run's one hub set: every place a traveller can change layer or vehicle, built once.
//!
//! **What joins what.** Built from, in this order:
//!
//! 1. **The stops** of the timetable: one `stop` hub per station (a stop's GTFS
//!    `parent_station`), or per stop without one, holding each of its stops as a transit access
//!    point and the first stop's walk and bike access points; transfers at it take the transit
//!    defaults' `stop_transfer_s`.
//! 2. **The user's hub table**, if any: a hub per row whose id is new (`declared`, at the row's
//!    position), so a resource naming it is placed there.
//! 3. **The parkings** ([`ParkingSetup`]), then **the other resources** (docks, chargers, stands:
//!    [`ResourceRow`]), each joined by **the join rule**: a resource whose row names a `hub_id`
//!    joins that hub (created if new); one without joins the hub of the nearest stop within
//!    `join_m` in a straight line ([`DEFAULT_HUB_JOIN_M`]); else it is a `declared` hub of its
//!    own, named by its own id. A joined resource adds its access points where the hub has none
//!    on that layer.
//!
//! **Identity.** Hubs get dense ids by sorted external id ([`HubSet`]); a hub's resources follow
//! the order above. Each subsystem keeps its own indices (stops by stop index, parkings by parking
//! index) and finds its hub or resource through this set's maps, so a run without services
//! computes exactly what it did when each subsystem built a hub set of its own.
//!
//! **Cost.** Once per run, after the setups: every resource against every stop (Paris: 2 684
//! parkings against 3 287 stops, a few milliseconds); about 20 bytes per hub, 8 per access point
//! and 30 per resource, besides the ids.

use std::collections::HashMap;

use openmobisim_core_graph::geometry::{LonLat, ground_distance_metres};
use openmobisim_core_graph::hubs::{AccessPoint, HubKind, HubSet, HubSpec, Resource, ResourceKind};
use openmobisim_core_graph::layers::Layer;
use openmobisim_core_types::ids::{EntityId, HubId, NodeId, ResourceId};

use crate::parking::ParkingSetup;
use crate::transit::TransitSetup;

/// How far a resource may be from a stop, in metres in a straight line, to join that stop's hub
/// when its row names none: a short walk across a station's forecourt. An assumption,
/// uncalibrated (the run's `hub_join_m`).
pub const DEFAULT_HUB_JOIN_M: f64 = 150.0;

/// A resource other than a parking, as its service's reader gives it: a dock, a charger, a stand.
#[derive(Clone, Debug, PartialEq)]
pub struct ResourceRow {
    /// The resource.
    pub resource: Resource,
    /// The hub its row names, if any.
    pub hub_id: Option<String>,
    /// Where it is.
    pub position: LonLat,
    /// Its access points, snapped by its reader (a dock's bike and walk nodes).
    pub access_points: Vec<AccessPoint>,
}

/// A row of the user's hub table.
#[derive(Clone, Debug, PartialEq)]
pub struct HubRow {
    /// The hub's id, unique in the table.
    pub hub_id: String,
    /// Where it is.
    pub position: LonLat,
}

/// The run's one hub set, with each subsystem's way into it: see the [module docs](self).
#[derive(Clone, Debug, Default)]
pub struct RunHubs {
    set: HubSet,
    /// Per stop index, its hub.
    stop_hub: Vec<HubId>,
    /// Per parking index, its resource.
    parking_resource: Vec<ResourceId>,
    /// Per [`ResourceRow`] given, its resource.
    row_resource: Vec<ResourceId>,
}

/// The hubs being gathered, by external id, in the order first named.
#[derive(Default)]
struct Drafts {
    drafts: Vec<HubSpec>,
    index: HashMap<String, usize>,
}

impl Drafts {
    /// The draft of hub `id`, made with `make` if it is new.
    fn entry(&mut self, id: &str, make: impl FnOnce() -> HubSpec) -> usize {
        if let Some(&i) = self.index.get(id) {
            return i;
        }
        self.drafts.push(make());
        self.index.insert(id.to_string(), self.drafts.len() - 1);
        self.drafts.len() - 1
    }

    /// Add `resource` to draft `d` with its access points; returns where it sits in the hub.
    fn add(&mut self, d: usize, resource: Resource, points: &[AccessPoint]) -> usize {
        let spec = &mut self.drafts[d];
        for p in points {
            if !spec.access_points.iter().any(|q| q.layer == p.layer) {
                spec.access_points.push(*p);
            }
        }
        spec.resources.push(resource);
        spec.resources.len() - 1
    }
}

fn declared(id: &str, position: LonLat) -> HubSpec {
    HubSpec {
        external_id: id.to_string(),
        kind: HubKind::Declared,
        position,
        access_points: Vec::new(),
        transfer_seconds: 0.0,
        resources: Vec::new(),
    }
}

impl RunHubs {
    /// Build the run's hub set from its stops, the user's hub table, its parkings and its other
    /// resources (any may be absent or empty), joining by `join_m` (see the [module docs](self)).
    ///
    /// # Panics
    ///
    /// If there are more than `u32::MAX` stops, parkings or resources.
    #[must_use]
    pub fn build(
        transit: Option<&TransitSetup>,
        hub_table: &[HubRow],
        parking: Option<&ParkingSetup>,
        rows: &[ResourceRow],
        join_m: f64,
    ) -> Self {
        let mut drafts = Drafts::default();
        // 1. The stops: one hub per station, or per stop.
        let mut stop_key: Vec<usize> = Vec::new();
        let mut stop_position: Vec<LonLat> = Vec::new();
        if let Some(t) = transit {
            let timetable = t.timetable();
            let transfer = t.defaults().stop_transfer_s;
            for s in 0..timetable.stop_count() {
                let stop = NodeId::new(s);
                let id =
                    timetable.stop_parent(stop).unwrap_or_else(|| timetable.stop_ids().external(s));
                let at = timetable.stop_position(stop);
                let d = drafts.entry(id, || HubSpec {
                    external_id: id.to_string(),
                    kind: HubKind::Stop,
                    position: at,
                    access_points: Vec::new(),
                    transfer_seconds: transfer,
                    resources: Vec::new(),
                });
                let points = &mut drafts.drafts[d].access_points;
                points.push(AccessPoint { layer: Layer::Transit, node: stop });
                for (layer, node) in
                    [(Layer::Walk, t.stop_walk_node(stop)), (Layer::Bike, t.stop_bike_node(stop))]
                {
                    if let Some(node) = node {
                        if !points.iter().any(|p| p.layer == layer) {
                            points.push(AccessPoint { layer, node });
                        }
                    }
                }
                stop_key.push(d);
                stop_position.push(at);
            }
        }
        // The hub of the nearest stop within `join_m` of `at`, if any (ties to the lower stop).
        let nearest_stop = |at: LonLat| -> Option<usize> {
            stop_position
                .iter()
                .enumerate()
                .map(|(s, &p)| (s, ground_distance_metres(at, p)))
                .filter(|&(_, d)| d <= join_m)
                .min_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)))
                .map(|(s, _)| stop_key[s])
        };
        // 2. The user's hub table.
        for row in hub_table {
            drafts.entry(&row.hub_id, || declared(&row.hub_id, row.position));
        }
        // The hub a resource joins: the one its row names, else the nearest stop's, else its own.
        let join = |drafts: &mut Drafts, named: Option<&str>, own: &str, at: LonLat| -> usize {
            match named {
                Some(hub) => drafts.entry(hub, || declared(hub, at)),
                None => match nearest_stop(at) {
                    Some(d) => d,
                    None => drafts.entry(own, || declared(own, at)),
                },
            }
        };
        // 3. The parkings, then the other resources.
        let mut parking_slot: Vec<(usize, usize)> = Vec::new();
        if let Some(p) = parking {
            let n = u32::try_from(p.count()).expect("parkings fit u32");
            for k in 0..n {
                let own = p.external_id(k);
                let named = (p.hub_external_id(k) != own).then(|| p.hub_external_id(k));
                let d = join(&mut drafts, named, own, p.position(k));
                let kind = p.kind(k);
                let resource = Resource::parking(own, kind, p.capacity(k), p.initial_occupancy(k));
                let points = [
                    AccessPoint { layer: kind.layer(), node: p.node(k) },
                    AccessPoint { layer: Layer::Walk, node: p.walk_node(k) },
                ];
                parking_slot.push((d, drafts.add(d, resource, &points)));
            }
        }
        let mut row_slot: Vec<(usize, usize)> = Vec::with_capacity(rows.len());
        for row in rows {
            let own = row.resource.external_id.as_str();
            let d = join(&mut drafts, row.hub_id.as_deref(), own, row.position);
            row_slot.push((d, drafts.add(d, row.resource.clone(), &row.access_points)));
        }
        let ids: Vec<String> = drafts.drafts.iter().map(|d| d.external_id.clone()).collect();
        let set = HubSet::build(drafts.drafts);
        let hub_of: Vec<HubId> =
            ids.iter().map(|id| set.id_of(id).expect("every draft is a hub")).collect();
        let resource = |(d, k): (usize, usize)| -> ResourceId {
            set.resources(hub_of[d]).nth(k).expect("a resource added to its hub")
        };
        Self {
            stop_hub: stop_key.iter().map(|&d| hub_of[d]).collect(),
            parking_resource: parking_slot.into_iter().map(resource).collect(),
            row_resource: row_slot.into_iter().map(resource).collect(),
            set,
        }
    }

    /// The hub set.
    #[must_use]
    pub fn set(&self) -> &HubSet {
        &self.set
    }

    /// A stop's hub (its station's, or its own).
    #[must_use]
    pub fn stop_hub(&self, stop: NodeId) -> HubId {
        self.stop_hub[stop.index()]
    }

    /// A parking's resource, by parking index.
    #[must_use]
    pub fn parking_resource(&self, parking: u32) -> ResourceId {
        self.parking_resource[parking as usize]
    }

    /// The resource of the `i`-th [`ResourceRow`] given to [`Self::build`].
    #[must_use]
    pub fn row_resource(&self, i: usize) -> ResourceId {
        self.row_resource[i]
    }

    /// A hub's external id.
    #[must_use]
    pub fn hub_external_id(&self, hub: HubId) -> &str {
        self.set.external_ids().external(hub.raw())
    }

    /// Each stop's hub id, by stop index: the `hub_id` the outputs write.
    #[must_use]
    pub fn stop_hub_ids(&self) -> Vec<String> {
        self.stop_hub.iter().map(|&h| self.hub_external_id(h).to_string()).collect()
    }

    /// Each parking's hub id, by parking index: the `hub_id` the outputs write.
    #[must_use]
    pub fn parking_hub_ids(&self) -> Vec<String> {
        self.parking_resource
            .iter()
            .map(|&r| self.hub_external_id(self.set.resource_hub(r)).to_string())
            .collect()
    }

    /// The hub's resources of one kind (its docks, its chargers).
    pub fn resources_of(
        &self,
        hub: HubId,
        kind: ResourceKind,
    ) -> impl Iterator<Item = ResourceId> + '_ {
        self.set.resources_of(hub, kind)
    }

    /// Bytes held.
    #[must_use]
    pub fn bytes(&self) -> usize {
        self.set.bytes()
            + (self.stop_hub.len() + self.parking_resource.len() + self.row_resource.len()) * 4
    }
}
