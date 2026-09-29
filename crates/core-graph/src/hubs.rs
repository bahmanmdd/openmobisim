//! Hubs: the only places a traveller changes layer (design §3.2; S198).
//!
//! A hub is a place with **access points** — a layer and a node of that layer's
//! graph — and **transfers** between them. A pavement-to-bus-stop transfer is a
//! hub with a walk and a transit access point; a park-and-ride is one with a car,
//! a walk and a transit access point; resources (parking, docks) join in M4. The
//! object is the same, with more fields filled in.
//!
//! **Transfers.** Every ordered pair of a hub's access points is a transfer,
//! all taking the hub's one transfer time. Explicit per-pair times (GTFS
//! `transfers.txt`) join when there is a user for them.
//!
//! **Identity.** Hubs get dense ids by sorted external id (Foundations §1), and
//! access points dense ids in hub order, so a rebuilt set has the same ids.
//!
//! **Cost:** about 20 bytes per hub plus 8 per access point, in flat arrays: a
//! city's thousands of automatic stop hubs (M3) are a few hundred kilobytes.

use openmobisim_core_types::ids::{
    AccessPointId, EntityId, ExternalIdTable, ExternalIdTableBuilder, HubId, NodeId,
};

use crate::geometry::LonLat;
use crate::layers::Layer;

/// What made a hub.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[repr(u8)]
pub enum HubKind {
    /// Declared by the scenario or a fixture: a park-and-ride, a station.
    Declared = 0,
    /// A transit stop, made automatically from GTFS (M3, design §18.4).
    Stop = 1,
    /// A kerb hub, made where a car or bike trip leaves or joins the walk layer
    /// (design §3.2, V5).
    Kerb = 2,
}

impl HubKind {
    /// The stable snake_case name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            HubKind::Declared => "declared",
            HubKind::Stop => "stop",
            HubKind::Kerb => "kerb",
        }
    }
}

/// Where a hub touches a layer: a node of that layer's graph.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct AccessPoint {
    /// The layer.
    pub layer: Layer,
    /// The node of the layer's graph.
    pub node: NodeId,
}

/// One hub, before ids are assigned.
#[derive(Clone, Debug)]
pub struct HubSpec {
    /// The hub's external id (a stop id, a name).
    pub external_id: String,
    /// What made it.
    pub kind: HubKind,
    /// Where it is.
    pub position: LonLat,
    /// Its access points, at most one per layer.
    pub access_points: Vec<AccessPoint>,
    /// The time any transfer between two of its access points takes, in seconds.
    pub transfer_seconds: f64,
}

/// Every hub of a scenario, with ids (see the [module docs](self)).
#[derive(Clone, Debug, Default)]
pub struct HubSet {
    ids: ExternalIdTable,
    kind: Vec<HubKind>,
    position: Vec<LonLat>,
    transfer_seconds: Vec<f64>,
    /// `hub_count + 1` offsets into the access-point arrays.
    access_start: Vec<u32>,
    access_layer: Vec<Layer>,
    access_node: Vec<NodeId>,
}

impl HubSet {
    /// Assign ids and freeze.
    ///
    /// A hub whose external id repeats an earlier one is dropped (the first
    /// wins); a second access point on a layer the hub already has is dropped.
    ///
    /// # Panics
    ///
    /// Panics if there are more than `u32::MAX` hubs or access points.
    #[must_use]
    pub fn build(specs: Vec<HubSpec>) -> Self {
        let mut builder = ExternalIdTableBuilder::with_capacity(specs.len());
        builder.extend(specs.iter().map(|h| h.external_id.clone()));
        let ids = builder.build();
        let n = ids.count() as usize;
        let mut slot: Vec<Option<HubSpec>> = vec![None; n];
        for spec in specs {
            if let Some(id) = ids.id_of(&spec.external_id) {
                let entry = &mut slot[id as usize];
                if entry.is_none() {
                    *entry = Some(spec);
                }
            }
        }
        let mut set = HubSet {
            ids,
            kind: Vec::with_capacity(n),
            position: Vec::with_capacity(n),
            transfer_seconds: Vec::with_capacity(n),
            access_start: vec![0],
            access_layer: Vec::new(),
            access_node: Vec::new(),
        };
        for spec in slot.into_iter().map(|s| s.expect("every id came from a spec")) {
            set.kind.push(spec.kind);
            set.position.push(spec.position);
            set.transfer_seconds.push(spec.transfer_seconds.max(0.0));
            let mut layers_seen: Vec<Layer> = Vec::new();
            let mut points = spec.access_points;
            points.sort_by_key(|p| p.layer);
            for point in points {
                if layers_seen.contains(&point.layer) {
                    continue;
                }
                layers_seen.push(point.layer);
                set.access_layer.push(point.layer);
                set.access_node.push(point.node);
            }
            set.access_start
                .push(u32::try_from(set.access_layer.len()).expect("access points fit u32"));
        }
        set
    }

    /// How many hubs.
    #[must_use]
    pub fn len(&self) -> u32 {
        self.ids.count()
    }

    /// Whether there are none.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The hub with this external id.
    #[must_use]
    pub fn id_of(&self, external: &str) -> Option<HubId> {
        self.ids.typed_id_of(external)
    }

    /// The external ids.
    #[must_use]
    pub fn external_ids(&self) -> &ExternalIdTable {
        &self.ids
    }

    /// What made the hub.
    #[must_use]
    pub fn kind(&self, hub: HubId) -> HubKind {
        self.kind[hub.index()]
    }

    /// Where the hub is.
    #[must_use]
    pub fn position(&self, hub: HubId) -> LonLat {
        self.position[hub.index()]
    }

    /// The time a transfer at the hub takes, in seconds.
    #[must_use]
    pub fn transfer_seconds(&self, hub: HubId) -> f64 {
        self.transfer_seconds[hub.index()]
    }

    /// The hub's access points, in layer order.
    pub fn access_points(&self, hub: HubId) -> impl Iterator<Item = AccessPointId> + '_ {
        let (start, end) = (self.access_start[hub.index()], self.access_start[hub.index() + 1]);
        (start..end).map(AccessPointId::new)
    }

    /// The access point's layer and node.
    #[must_use]
    pub fn access_point(&self, point: AccessPointId) -> AccessPoint {
        AccessPoint {
            layer: self.access_layer[point.index()],
            node: self.access_node[point.index()],
        }
    }

    /// The hub's access point on `layer`, if it has one.
    #[must_use]
    pub fn access_point_on(&self, hub: HubId, layer: Layer) -> Option<AccessPointId> {
        self.access_points(hub).find(|&p| self.access_layer[p.index()] == layer)
    }

    /// Bytes held.
    #[must_use]
    pub fn bytes(&self) -> usize {
        self.ids.arena_bytes()
            + self.kind.len()
            + self.position.len() * core::mem::size_of::<LonLat>()
            + self.transfer_seconds.len() * 8
            + self.access_start.len() * 4
            + self.access_layer.len()
            + self.access_node.len() * 4
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(id: &str, points: &[(Layer, u32)], transfer: f64) -> HubSpec {
        HubSpec {
            external_id: id.to_string(),
            kind: HubKind::Declared,
            position: LonLat::new(4.9, 52.37),
            access_points: points
                .iter()
                .map(|&(layer, node)| AccessPoint { layer, node: NodeId::new(node) })
                .collect(),
            transfer_seconds: transfer,
        }
    }

    #[test]
    fn ids_follow_sorted_external_ids_and_access_points_follow_their_hub() {
        let set = HubSet::build(vec![
            spec("b", &[(Layer::Walk, 7), (Layer::Car, 3)], 60.0),
            spec("a", &[(Layer::Bike, 1)], 0.0),
        ]);
        assert_eq!(set.len(), 2);
        let (a, b) = (set.id_of("a").unwrap(), set.id_of("b").unwrap());
        assert_eq!((a.raw(), b.raw()), (0, 1));
        let points: Vec<AccessPoint> = set.access_points(b).map(|p| set.access_point(p)).collect();
        assert_eq!(
            points,
            [
                AccessPoint { layer: Layer::Car, node: NodeId::new(3) },
                AccessPoint { layer: Layer::Walk, node: NodeId::new(7) }
            ],
            "in layer order"
        );
        assert_eq!(
            set.access_point_on(b, Layer::Walk).map(|p| set.access_point(p).node),
            Some(NodeId::new(7))
        );
        assert_eq!(set.access_point_on(b, Layer::Bike), None);
        assert!((set.transfer_seconds(b) - 60.0).abs() < 1e-12);
    }

    #[test]
    fn a_repeated_hub_or_layer_keeps_the_first() {
        let set = HubSet::build(vec![
            spec("h", &[(Layer::Walk, 1), (Layer::Walk, 2)], 30.0),
            spec("h", &[(Layer::Car, 9)], 99.0),
        ]);
        assert_eq!(set.len(), 1);
        let h = set.id_of("h").unwrap();
        assert_eq!(set.access_points(h).count(), 1);
        assert!((set.transfer_seconds(h) - 30.0).abs() < 1e-12);
    }
}
