//! Hubs: the only places a traveller changes layer (design §3.2; S198).
//!
//! A hub is a place with **access points** — a layer and a node of that layer's
//! graph — and **transfers** between them. A pavement-to-bus-stop transfer is a
//! hub with a walk and a transit access point; a park-and-ride is one with a car,
//! a walk and a transit access point; its **parking** is a resource (M4, S201).
//! The object is the same, with more fields filled in.
//!
//! **Parking** is typed by vehicle, car or bike, from the start (S193): a
//! station's bike parking and a park-and-ride garage are the same resource, with
//! a capacity and an occupancy at the start of the day. How full a parking is
//! over the day, and what that costs a traveller, is the run's business
//! (`core-sim::parking`); the hub only says what is there.
//!
//! **Transfers.** Every ordered pair of a hub's access points is a transfer,
//! all taking the hub's one transfer time. Explicit per-pair times (GTFS
//! `transfers.txt`) join when there is a user for them.
//!
//! **Identity.** Hubs get dense ids by sorted external id (Foundations §1), and
//! access points dense ids in hub order, so a rebuilt set has the same ids.
//!
//! **Cost:** about 20 bytes per hub plus 8 per access point, in flat arrays: a
//! city's thousands of automatic stop hubs (M3) are a few hundred kilobytes. A
//! parking adds about 16 bytes and its external id.

use openmobisim_core_types::ids::{
    AccessPointId, EntityId, ExternalIdTable, ExternalIdTableBuilder, HubId, NodeId, ResourceId,
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

/// Which vehicle a parking holds (S193: typed from the start).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
#[repr(u8)]
pub enum ParkingKind {
    /// Cars: a park-and-ride garage or lot.
    Car = 0,
    /// Bikes: a station's bike parking, a rack by a stop.
    Bike = 1,
}

impl ParkingKind {
    /// Both kinds, in discriminant order.
    pub const ALL: [ParkingKind; 2] = [ParkingKind::Car, ParkingKind::Bike];

    /// The stable snake_case name, as a parking table writes it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            ParkingKind::Car => "car",
            ParkingKind::Bike => "bike",
        }
    }

    /// The kind a name names, if any.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.as_str() == name)
    }

    /// The layer the vehicle arrives on.
    #[must_use]
    pub const fn layer(self) -> Layer {
        match self {
            ParkingKind::Car => Layer::Car,
            ParkingKind::Bike => Layer::Bike,
        }
    }

    /// The position of this kind in [`Self::ALL`], for per-kind tables.
    #[must_use]
    pub const fn index(self) -> usize {
        self as usize
    }
}

/// A parking: a hub's resource (design §3.2; M4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Parking {
    /// Its external id (a parking table's `parking_id`, an OSM element).
    pub external_id: String,
    /// The vehicle it holds.
    pub kind: ParkingKind,
    /// How many vehicles it holds.
    pub capacity: u32,
    /// How many are parked at the start of the day.
    pub initial_occupancy: u32,
}

/// One row of a parking table (M4, D11): a parking a user declares, or one the
/// OSM reader found. A run snaps it to its vehicle's layer and the walk layer
/// and makes it a hub (rows sharing a `hub_id` join one hub).
#[derive(Clone, Debug, PartialEq)]
pub struct ParkingRow {
    /// Its id, unique in the table.
    pub parking_id: String,
    /// Its name, if it has one.
    pub name: Option<String>,
    /// The hub it belongs to, if it shares one with other rows (a station's car
    /// and bike parking); else the parking is its own hub.
    pub hub_id: Option<String>,
    /// Where it is.
    pub position: LonLat,
    /// The vehicle it holds.
    pub kind: ParkingKind,
    /// How many vehicles it holds.
    pub capacity: u32,
    /// How many are parked at the start of the day.
    pub initial_occupancy: u32,
    /// What parking there costs, in euros per stay (S248, roadmap I-bb U5), if the table says;
    /// else the run's price for its kind (`core-sim`'s `Prices`).
    pub fee_eur: Option<f64>,
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
    /// Its parkings, if any.
    pub parkings: Vec<Parking>,
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
    /// `hub_count + 1` offsets into the parking arrays; parking ids follow hub order.
    parking_start: Vec<u32>,
    parking_hub: Vec<HubId>,
    parking_external: Vec<String>,
    parking_kind: Vec<ParkingKind>,
    parking_capacity: Vec<u32>,
    parking_initial: Vec<u32>,
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
            parking_start: vec![0],
            parking_hub: Vec::new(),
            parking_external: Vec::new(),
            parking_kind: Vec::new(),
            parking_capacity: Vec::new(),
            parking_initial: Vec::new(),
        };
        for (h, spec) in slot.into_iter().map(|s| s.expect("every id came from a spec")).enumerate()
        {
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
            for parking in spec.parkings {
                set.parking_hub.push(HubId::from_index(h));
                set.parking_external.push(parking.external_id);
                set.parking_kind.push(parking.kind);
                set.parking_capacity.push(parking.capacity);
                set.parking_initial.push(parking.initial_occupancy);
            }
            set.parking_start.push(u32::try_from(set.parking_hub.len()).expect("parkings fit u32"));
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

    /// How many parkings, over every hub.
    ///
    /// # Panics
    ///
    /// Never: [`Self::build`] already refused more than `u32::MAX`.
    #[must_use]
    pub fn parking_count(&self) -> u32 {
        u32::try_from(self.parking_hub.len()).expect("parkings fit u32")
    }

    /// The hub's parkings.
    pub fn parkings(&self, hub: HubId) -> impl Iterator<Item = ResourceId> + '_ {
        let (start, end) = (self.parking_start[hub.index()], self.parking_start[hub.index() + 1]);
        (start..end).map(ResourceId::new)
    }

    /// The hub a parking belongs to.
    #[must_use]
    pub fn parking_hub(&self, parking: ResourceId) -> HubId {
        self.parking_hub[parking.index()]
    }

    /// A parking's external id.
    #[must_use]
    pub fn parking_external_id(&self, parking: ResourceId) -> &str {
        &self.parking_external[parking.index()]
    }

    /// The vehicle a parking holds.
    #[must_use]
    pub fn parking_kind(&self, parking: ResourceId) -> ParkingKind {
        self.parking_kind[parking.index()]
    }

    /// How many vehicles a parking holds.
    #[must_use]
    pub fn parking_capacity(&self, parking: ResourceId) -> u32 {
        self.parking_capacity[parking.index()]
    }

    /// How many are parked there at the start of the day.
    #[must_use]
    pub fn parking_initial_occupancy(&self, parking: ResourceId) -> u32 {
        self.parking_initial[parking.index()]
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
            + self.parking_start.len() * 4
            + self.parking_hub.len() * 13
            + self.parking_external.iter().map(|e| e.len() + 24).sum::<usize>()
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
            parkings: Vec::new(),
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
    fn parkings_follow_their_hub_and_keep_their_kind_and_capacity() {
        let mut a = spec("a", &[(Layer::Car, 1), (Layer::Walk, 2)], 60.0);
        a.parkings = vec![
            Parking {
                external_id: "a-car".into(),
                kind: ParkingKind::Car,
                capacity: 6,
                initial_occupancy: 1,
            },
            Parking {
                external_id: "a-bike".into(),
                kind: ParkingKind::Bike,
                capacity: 3,
                initial_occupancy: 0,
            },
        ];
        let set = HubSet::build(vec![spec("b", &[(Layer::Walk, 7)], 0.0), a]);
        assert_eq!(set.parking_count(), 2);
        let (a, b) = (set.id_of("a").unwrap(), set.id_of("b").unwrap());
        assert_eq!(set.parkings(b).count(), 0);
        let ps: Vec<ResourceId> = set.parkings(a).collect();
        assert_eq!(ps.len(), 2);
        assert_eq!(set.parking_hub(ps[0]), a);
        assert_eq!(set.parking_external_id(ps[0]), "a-car");
        assert_eq!(set.parking_kind(ps[1]), ParkingKind::Bike);
        assert_eq!((set.parking_capacity(ps[0]), set.parking_initial_occupancy(ps[0])), (6, 1));
        assert_eq!(ParkingKind::from_name("bike"), Some(ParkingKind::Bike));
        assert_eq!(ParkingKind::from_name("Bike"), None);
        assert_eq!(ParkingKind::Car.layer(), Layer::Car);
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
