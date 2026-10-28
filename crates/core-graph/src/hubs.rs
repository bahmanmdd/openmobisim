//! Hubs: the only places a traveller changes layer.
//!
//! A hub is a place with **access points** — a layer and a node of that layer's
//! graph — and **transfers** between them. A pavement-to-bus-stop transfer is a
//! hub with a walk and a transit access point; a park-and-ride is one with a car,
//! a walk and a transit access point; its **parking** is a resource.
//! The object is the same, with more fields filled in.
//!
//! **Resources** are typed: a **parking** (by vehicle, car or bike: a station's
//! bike parking and a park-and-ride garage are the same resource), a **dock**
//! (slots holding a sharing service's bikes), a **charger** (plugs of a power)
//! and a **stand** (spaces where a service's cars or taxis wait). Each has a
//! capacity and a count at the start of the day. How full a resource is over
//! the day, and what that costs a traveller, is the run's business
//! (`core-sim::parking`, `core-sim::services`); the hub only says what is there.
//!
//! **Access points:** at most one per layer, except on the transit layer, where
//! a station's hub has one per stop (its platforms).
//!
//! **Transfers.** Every ordered pair of a hub's access points is a transfer,
//! all taking the hub's one transfer time. Explicit per-pair times (GTFS
//! `transfers.txt`) join when there is a user for them.
//!
//! **Identity.** Hubs get dense ids by sorted external id, and
//! access points dense ids in hub order, so a rebuilt set has the same ids.
//!
//! **Cost:** about 20 bytes per hub plus 8 per access point, in flat arrays: a
//! city's thousands of automatic stop hubs are a few hundred kilobytes. A
//! resource adds about 30 bytes and its external id.

use openmobisim_core_types::ids::{
    AccessPointId, EntityId, ExternalIdTable, ExternalIdTableBuilder, HubId, NodeId, ResourceId,
};

use crate::binary::{self, Reader, Writer, invalid};
use crate::geometry::LonLat;
use crate::layers::Layer;

/// What made a hub.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[repr(u8)]
pub enum HubKind {
    /// Declared by the scenario or a fixture: a park-and-ride, a station.
    Declared = 0,
    /// A transit stop, made automatically from GTFS.
    Stop = 1,
    /// A kerb hub, made where a car or bike trip leaves or joins the walk layer.
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

/// Which vehicle a parking holds (typed from the start).
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

/// What a resource is.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ResourceKind {
    /// Parking for one's own vehicle: capacity in vehicles.
    Parking(ParkingKind),
    /// A bike-sharing dock: capacity in slots, the start count in bikes docked.
    Dock,
    /// A charger: capacity in plugs (the power in [`Resource::kw`]).
    Charger,
    /// A stand where a service's vehicles wait: capacity in spaces
    /// ([`UNLIMITED`] for a stand without a limit), the start count in vehicles.
    Stand,
}

impl ResourceKind {
    /// The stable snake_case name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            ResourceKind::Parking(ParkingKind::Car) => "parking_car",
            ResourceKind::Parking(ParkingKind::Bike) => "parking_bike",
            ResourceKind::Dock => "dock",
            ResourceKind::Charger => "charger",
            ResourceKind::Stand => "stand",
        }
    }
}

/// The capacity of a resource without a limit (a stand on a kerb).
pub const UNLIMITED: u32 = u32::MAX;

/// A hub's resource: a parking, a dock, a charger or a stand.
#[derive(Clone, Debug, PartialEq)]
pub struct Resource {
    /// Its external id (a parking table's `parking_id`, a dock's station id).
    pub external_id: String,
    /// What it is.
    pub kind: ResourceKind,
    /// How many it holds: vehicles, slots, plugs or spaces.
    pub capacity: u32,
    /// How many are there at the start of the day: vehicles parked, bikes docked,
    /// vehicles waiting (0 for a charger).
    pub initial: u32,
    /// A charger's power per plug in kilowatts; 0 for the other kinds.
    pub kw: f64,
    /// The service it belongs to (a dock's sharing system, a taxi fleet's stand),
    /// or `None` for one every service and traveller may use.
    pub service: Option<String>,
}

impl Resource {
    /// A parking for one's own vehicle, of no service.
    #[must_use]
    pub fn parking(external_id: &str, kind: ParkingKind, capacity: u32, initial: u32) -> Self {
        Self {
            external_id: external_id.to_string(),
            kind: ResourceKind::Parking(kind),
            capacity,
            initial,
            kw: 0.0,
            service: None,
        }
    }
}

/// One row of a parking table: a parking a user declares, or one the
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
    /// What parking there costs, in euros per stay, if the table says;
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
    /// Its access points: at most one per layer, except several on the transit
    /// layer (a station's stops).
    pub access_points: Vec<AccessPoint>,
    /// The time any transfer between two of its access points takes, in seconds.
    pub transfer_seconds: f64,
    /// Its resources, if any.
    pub resources: Vec<Resource>,
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
    /// `hub_count + 1` offsets into the resource arrays; resource ids follow hub
    /// order, and a hub's resources the order its spec gave them.
    resource_start: Vec<u32>,
    resource_hub: Vec<HubId>,
    resource_external: Vec<String>,
    resource_kind: Vec<ResourceKind>,
    resource_capacity: Vec<u32>,
    resource_initial: Vec<u32>,
    resource_kw: Vec<f64>,
    resource_service: Vec<Option<String>>,
}

impl HubSet {
    /// Assign ids and freeze.
    ///
    /// A hub whose external id repeats an earlier one is dropped (the first
    /// wins); a second access point on a layer the hub already has is dropped,
    /// except on the transit layer, where only a repeated node is.
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
            resource_start: vec![0],
            resource_hub: Vec::new(),
            resource_external: Vec::new(),
            resource_kind: Vec::new(),
            resource_capacity: Vec::new(),
            resource_initial: Vec::new(),
            resource_kw: Vec::new(),
            resource_service: Vec::new(),
        };
        for (h, spec) in slot.into_iter().map(|s| s.expect("every id came from a spec")).enumerate()
        {
            set.kind.push(spec.kind);
            set.position.push(spec.position);
            set.transfer_seconds.push(spec.transfer_seconds.max(0.0));
            let mut seen: Vec<AccessPoint> = Vec::new();
            let mut points = spec.access_points;
            // Stable: a station's stops keep the order they were given.
            points.sort_by_key(|p| p.layer);
            for point in points {
                let repeated = if point.layer == Layer::Transit {
                    seen.contains(&point)
                } else {
                    seen.iter().any(|p| p.layer == point.layer)
                };
                if repeated {
                    continue;
                }
                seen.push(point);
                set.access_layer.push(point.layer);
                set.access_node.push(point.node);
            }
            set.access_start
                .push(u32::try_from(set.access_layer.len()).expect("access points fit u32"));
            for resource in spec.resources {
                set.resource_hub.push(HubId::from_index(h));
                set.resource_external.push(resource.external_id);
                set.resource_kind.push(resource.kind);
                set.resource_capacity.push(resource.capacity);
                set.resource_initial.push(resource.initial);
                set.resource_kw.push(resource.kw);
                set.resource_service.push(resource.service);
            }
            set.resource_start
                .push(u32::try_from(set.resource_hub.len()).expect("resources fit u32"));
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

    /// The hub's access point on `layer`, if it has one (the first, on the
    /// transit layer).
    #[must_use]
    pub fn access_point_on(&self, hub: HubId, layer: Layer) -> Option<AccessPointId> {
        self.access_points(hub).find(|&p| self.access_layer[p.index()] == layer)
    }

    /// How many resources, over every hub.
    ///
    /// # Panics
    ///
    /// Never: [`Self::build`] already refused more than `u32::MAX`.
    #[must_use]
    pub fn resource_count(&self) -> u32 {
        u32::try_from(self.resource_hub.len()).expect("resources fit u32")
    }

    /// The hub's resources, in the order its spec gave them.
    pub fn resources(&self, hub: HubId) -> impl Iterator<Item = ResourceId> + '_ {
        let (start, end) = (self.resource_start[hub.index()], self.resource_start[hub.index() + 1]);
        (start..end).map(ResourceId::new)
    }

    /// The hub's resources of one kind.
    pub fn resources_of(
        &self,
        hub: HubId,
        kind: ResourceKind,
    ) -> impl Iterator<Item = ResourceId> + '_ {
        self.resources(hub).filter(move |r| self.resource_kind[r.index()] == kind)
    }

    /// The hub a resource belongs to.
    #[must_use]
    pub fn resource_hub(&self, resource: ResourceId) -> HubId {
        self.resource_hub[resource.index()]
    }

    /// A resource's external id.
    #[must_use]
    pub fn resource_external_id(&self, resource: ResourceId) -> &str {
        &self.resource_external[resource.index()]
    }

    /// What a resource is.
    #[must_use]
    pub fn resource_kind(&self, resource: ResourceId) -> ResourceKind {
        self.resource_kind[resource.index()]
    }

    /// How many a resource holds ([`UNLIMITED`] for a stand without a limit).
    #[must_use]
    pub fn resource_capacity(&self, resource: ResourceId) -> u32 {
        self.resource_capacity[resource.index()]
    }

    /// How many are there at the start of the day.
    #[must_use]
    pub fn resource_initial(&self, resource: ResourceId) -> u32 {
        self.resource_initial[resource.index()]
    }

    /// A charger's power per plug in kilowatts; 0 for the other kinds.
    #[must_use]
    pub fn resource_kw(&self, resource: ResourceId) -> f64 {
        self.resource_kw[resource.index()]
    }

    /// The service a resource belongs to, or `None` for a shared one.
    #[must_use]
    pub fn resource_service(&self, resource: ResourceId) -> Option<&str> {
        self.resource_service[resource.index()].as_deref()
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
            + self.resource_start.len() * 4
            + self.resource_hub.len() * (4 + 2 + 4 + 4 + 8)
            + self.resource_external.iter().map(|e| e.len() + 24).sum::<usize>()
            + self
                .resource_service
                .iter()
                .map(|e| e.as_ref().map_or(0, String::len) + 24)
                .sum::<usize>()
    }
}

// --- The binary form (S294, I-as) -------------------------------------------------------------

impl ParkingRow {
    /// Write every field of the row into `w` as it is held ([`crate::binary`]):
    /// [`Self::read_binary`] gives back a row equal to this one, bit for bit. A row takes at
    /// least 36 bytes.
    pub fn write_binary(&self, w: &mut Writer) {
        // Destructured without `..`: a field added to the row does not compile until it is
        // written here (and read below).
        let ParkingRow {
            parking_id,
            name,
            hub_id,
            position,
            kind,
            capacity,
            initial_occupancy,
            fee_eur,
        } = self;
        let LonLat { lon, lat } = *position;
        w.str(parking_id);
        w.opt_str(name.as_deref());
        w.opt_str(hub_id.as_deref());
        w.f64(lon);
        w.f64(lat);
        w.u8(*kind as u8);
        w.u32(*capacity);
        w.u32(*initial_occupancy);
        w.opt_f64(*fee_eur);
    }

    /// Read a row [`Self::write_binary`] wrote.
    ///
    /// # Errors
    ///
    /// [`binary::FormatError`] for bytes that are not such a row: past the end, a string that
    /// is not UTF-8, a vehicle that is not one.
    pub fn read_binary(r: &mut Reader<'_>) -> binary::Result<Self> {
        let parking_id = r.str()?.to_string();
        let name = r.opt_str()?.map(str::to_string);
        let hub_id = r.opt_str()?.map(str::to_string);
        let lon = r.f64()?;
        let lat = r.f64()?;
        let code = r.u8()?;
        // `ALL` is in discriminant order, the order written.
        let Some(&kind) = ParkingKind::ALL.get(usize::from(code)) else {
            return invalid(format!("parking vehicle {code}"));
        };
        let capacity = r.u32()?;
        let initial_occupancy = r.u32()?;
        let fee_eur = r.opt_f64()?;
        Ok(Self {
            parking_id,
            name,
            hub_id,
            position: LonLat::new(lon, lat),
            kind,
            capacity,
            initial_occupancy,
            fee_eur,
        })
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
            resources: Vec::new(),
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
    fn resources_follow_their_hub_and_keep_their_kind_and_capacity() {
        let mut a = spec("a", &[(Layer::Car, 1), (Layer::Walk, 2)], 60.0);
        a.resources = vec![
            Resource::parking("a-car", ParkingKind::Car, 6, 1),
            Resource::parking("a-bike", ParkingKind::Bike, 3, 0),
            Resource {
                external_id: "a-dock".into(),
                kind: ResourceKind::Dock,
                capacity: 20,
                initial: 10,
                kw: 0.0,
                service: Some("bike_share".into()),
            },
        ];
        let set = HubSet::build(vec![spec("b", &[(Layer::Walk, 7)], 0.0), a]);
        assert_eq!(set.resource_count(), 3);
        let (a, b) = (set.id_of("a").unwrap(), set.id_of("b").unwrap());
        assert_eq!(set.resources(b).count(), 0);
        let rs: Vec<ResourceId> = set.resources(a).collect();
        assert_eq!(rs.len(), 3);
        assert_eq!(set.resource_hub(rs[0]), a);
        assert_eq!(set.resource_external_id(rs[0]), "a-car");
        assert_eq!(set.resource_kind(rs[1]), ResourceKind::Parking(ParkingKind::Bike));
        assert_eq!((set.resource_capacity(rs[0]), set.resource_initial(rs[0])), (6, 1));
        assert_eq!(set.resource_service(rs[0]), None);
        assert_eq!(set.resource_service(rs[2]), Some("bike_share"));
        let docks: Vec<ResourceId> = set.resources_of(a, ResourceKind::Dock).collect();
        assert_eq!(docks, [rs[2]]);
        assert_eq!(ParkingKind::from_name("bike"), Some(ParkingKind::Bike));
        assert_eq!(ParkingKind::from_name("Bike"), None);
        assert_eq!(ParkingKind::Car.layer(), Layer::Car);
        assert_eq!(ResourceKind::Parking(ParkingKind::Car).as_str(), "parking_car");
    }

    #[test]
    fn a_station_keeps_one_transit_access_point_per_stop() {
        let set = HubSet::build(vec![spec(
            "station",
            &[(Layer::Transit, 4), (Layer::Walk, 1), (Layer::Transit, 2), (Layer::Transit, 4)],
            60.0,
        )]);
        let h = set.id_of("station").unwrap();
        let points: Vec<AccessPoint> = set.access_points(h).map(|p| set.access_point(p)).collect();
        assert_eq!(
            points,
            [
                AccessPoint { layer: Layer::Walk, node: NodeId::new(1) },
                AccessPoint { layer: Layer::Transit, node: NodeId::new(4) },
                AccessPoint { layer: Layer::Transit, node: NodeId::new(2) },
            ],
            "the repeated stop dropped; the others in the order given"
        );
        assert_eq!(
            set.access_point_on(h, Layer::Transit).map(|p| set.access_point(p).node),
            Some(NodeId::new(4))
        );
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
