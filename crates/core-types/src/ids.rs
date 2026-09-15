//! Dense `u32` identity spaces, one per entity kind (Foundations §1).
//!
//! # Why `u32`, and why a sentinel
//!
//! Ids sit in every array, every CSR structure, every memory-mapped artifact
//! and every output file. Two consequences follow, and both are load-bearing:
//!
//! * **`u32`, not `u64`.** Doubling the index width would double every CSR
//!   array and every mmap artifact. `u32` covers any city — 10⁵–10⁶ links,
//!   10⁷ trips — with room to spare.
//! * **[`NULL_ID`] (`u32::MAX`) is the null sentinel.** A stored array of
//!   optional indices must use the sentinel, never `Option<Id>`: the latter is
//!   eight bytes where the former is four. [`NullableId`] gives the ergonomics
//!   of `Option` at the cost of the sentinel, and [`EntityId::to_option`]
//!   converts at the boundary where ergonomics matter more than bytes.
//!
//! # Assignment order is part of the contract
//!
//! Ids are assigned by **sorting external ids**, never by hash-map iteration
//! order and never by file order. Two builds of the same input must produce
//! the same id assignment, or every cached artifact and every seeded run
//! silently stops being comparable. [`ExternalIdTableBuilder`] is the only
//! blessed way to do it.
//!
//! An internal id is meaningful **only** together with its network
//! fingerprint, which every output file and cached artifact carries.

use core::fmt;
use core::hash::Hash;

/// The null sentinel shared by every id space.
///
/// Stored arrays use this instead of `Option<Id>`; see the module docs.
pub const NULL_ID: u32 = u32::MAX;

/// The largest assignable raw id. One less than [`NULL_ID`].
pub const MAX_ID: u32 = u32::MAX - 1;

/// The entity kinds openmobisim gives dense id spaces to.
///
/// # The discriminants are part of a total order
///
/// The event queue is keyed on `(second, entity_kind, entity_id)`
/// (Foundations §2), and that tuple is what makes ties resolve identically on
/// every run. The numeric values below are therefore **stable**: reordering
/// them changes the order in which simultaneous events fire, which changes
/// results. Append new kinds at the end; never renumber existing ones.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
#[repr(u8)]
pub enum EntityKind {
    /// A network node.
    Node = 0,
    /// A directed network link.
    Link = 1,
    /// An (incoming link, outgoing link) pair at a node — the node model's unit.
    Turn = 2,
    /// A network layer (road, walk, bike, transit, …).
    Layer = 3,
    /// A point at which a traveller may enter or leave a layer.
    AccessPoint = 4,
    /// A capacitated transfer point — the primitive of the multilayer graph.
    Hub = 5,
    /// A typed, capacitated store held by a hub (dock, bay, platform, …).
    Resource = 6,
    /// A zone: route-set key and reporting unit.
    Zone = 7,
    /// A path in the route store's CSR index.
    Path = 8,
    /// An individual simulated traveller, carrying an integer weight.
    Traveller = 9,
    /// One trip of one traveller.
    Trip = 10,
    /// An individually identified vehicle.
    Vehicle = 11,
    /// One GTFS trip instance on the chosen service date.
    TransitRun = 12,
    /// A user class.
    UserClass = 13,
}

impl EntityKind {
    /// Every kind, in discriminant order.
    pub const ALL: [EntityKind; 14] = [
        EntityKind::Node,
        EntityKind::Link,
        EntityKind::Turn,
        EntityKind::Layer,
        EntityKind::AccessPoint,
        EntityKind::Hub,
        EntityKind::Resource,
        EntityKind::Zone,
        EntityKind::Path,
        EntityKind::Traveller,
        EntityKind::Trip,
        EntityKind::Vehicle,
        EntityKind::TransitRun,
        EntityKind::UserClass,
    ];

    /// The stable snake_case name written to Parquet and JSON outputs.
    ///
    /// These strings appear in `diagnostics.parquet` and `events.parquet` and
    /// are read by downstream analysis; treat them as a public schema.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            EntityKind::Node => "node",
            EntityKind::Link => "link",
            EntityKind::Turn => "turn",
            EntityKind::Layer => "layer",
            EntityKind::AccessPoint => "access_point",
            EntityKind::Hub => "hub",
            EntityKind::Resource => "resource",
            EntityKind::Zone => "zone",
            EntityKind::Path => "path",
            EntityKind::Traveller => "traveller",
            EntityKind::Trip => "trip",
            EntityKind::Vehicle => "vehicle",
            EntityKind::TransitRun => "transit_run",
            EntityKind::UserClass => "user_class",
        }
    }

    /// Recover a kind from its stable discriminant, or `None` if unknown.
    ///
    /// Used when reading a stored artifact written by a different build.
    #[must_use]
    pub const fn from_u8(raw: u8) -> Option<Self> {
        match raw {
            0 => Some(EntityKind::Node),
            1 => Some(EntityKind::Link),
            2 => Some(EntityKind::Turn),
            3 => Some(EntityKind::Layer),
            4 => Some(EntityKind::AccessPoint),
            5 => Some(EntityKind::Hub),
            6 => Some(EntityKind::Resource),
            7 => Some(EntityKind::Zone),
            8 => Some(EntityKind::Path),
            9 => Some(EntityKind::Traveller),
            10 => Some(EntityKind::Trip),
            11 => Some(EntityKind::Vehicle),
            12 => Some(EntityKind::TransitRun),
            13 => Some(EntityKind::UserClass),
            _ => None,
        }
    }
}

impl fmt::Display for EntityKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The operations every dense id shares.
///
/// Implemented by the newtypes in this module; a generic function that needs
/// "some id" should take `I: EntityId` rather than a raw `u32`, so that the
/// kind cannot be mixed up at a call site.
pub trait EntityId: Copy + Eq + Ord + Hash + fmt::Debug + fmt::Display {
    /// The entity kind this id space addresses.
    const KIND: EntityKind;

    /// The null sentinel for this id space.
    const NULL: Self;

    /// Wrap a raw value that is known not to be null.
    ///
    /// # Panics
    ///
    /// In debug builds, panics if `raw` is [`NULL_ID`]. Release builds do not
    /// check: this sits on the hot path of every builder. Use
    /// [`from_raw`](EntityId::from_raw) for values that may legitimately be
    /// null.
    fn new(raw: u32) -> Self;

    /// Wrap a raw value that may be [`NULL_ID`].
    ///
    /// This is the round-trip used when reading a stored array.
    fn from_raw(raw: u32) -> Self;

    /// Wrap a `usize` index.
    ///
    /// # Panics
    ///
    /// In debug builds, panics if `index` exceeds [`MAX_ID`].
    fn from_index(index: usize) -> Self;

    /// The raw value, including [`NULL_ID`] if null.
    fn raw(self) -> u32;

    /// The value as a `usize` array index.
    ///
    /// # Panics
    ///
    /// In debug builds, panics if called on the null sentinel — indexing with
    /// a null id is always a bug, and it is the bug this catches.
    fn index(self) -> usize;

    /// Whether this is the null sentinel.
    fn is_null(self) -> bool;

    /// `None` if null, otherwise `Some(self)`.
    ///
    /// Use at API boundaries, never in a stored array.
    fn to_option(self) -> Option<Self> {
        if self.is_null() { None } else { Some(self) }
    }

    /// All ids from `0` to `count` (exclusive), in order.
    ///
    /// The canonical way to iterate an id space whose size is known.
    fn iter_space(count: u32) -> impl Iterator<Item = Self> {
        (0..count).map(Self::new)
    }
}

/// A convenience alias documenting that a field uses the sentinel for "absent".
///
/// `NullableId<LinkId>` and `LinkId` are the same four bytes and the same
/// type; the alias exists so a struct field can say which it means.
pub type NullableId<I> = I;

macro_rules! define_id {
    ($(#[$doc:meta])* $name:ident, $kind:ident) => {
        $(#[$doc])*
        ///
        /// A dense `u32` index. [`NULL_ID`] is the null sentinel; see the
        /// [module docs](self) for why stored arrays use it rather than
        /// `Option`.
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        #[repr(transparent)]
        pub struct $name(u32);

        impl EntityId for $name {
            const KIND: EntityKind = EntityKind::$kind;
            const NULL: Self = Self(NULL_ID);

            #[inline]
            fn new(raw: u32) -> Self {
                debug_assert!(
                    raw != NULL_ID,
                    concat!(stringify!($name), "::new called with the null sentinel")
                );
                Self(raw)
            }

            #[inline]
            fn from_raw(raw: u32) -> Self {
                Self(raw)
            }

            #[inline]
            fn from_index(index: usize) -> Self {
                debug_assert!(
                    index <= MAX_ID as usize,
                    concat!(stringify!($name), "::from_index overflowed the u32 id space")
                );
                #[allow(
                    clippy::cast_possible_truncation,
                    reason = "guarded by the debug assertion above; the id space is u32 by design"
                )]
                let raw = index as u32;
                Self(raw)
            }

            #[inline]
            fn raw(self) -> u32 {
                self.0
            }

            #[inline]
            fn index(self) -> usize {
                debug_assert!(
                    self.0 != NULL_ID,
                    concat!("indexed an array with a null ", stringify!($name))
                );
                self.0 as usize
            }

            #[inline]
            fn is_null(self) -> bool {
                self.0 == NULL_ID
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                if self.0 == NULL_ID {
                    write!(f, concat!(stringify!($name), "(null)"))
                } else {
                    write!(f, concat!(stringify!($name), "({})"), self.0)
                }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                if self.0 == NULL_ID {
                    f.write_str("null")
                } else {
                    write!(f, "{}", self.0)
                }
            }
        }
    };
}

define_id!(/// Identifies a network node.
    NodeId, Node);
define_id!(/// Identifies a directed network link.
    LinkId, Link);
define_id!(/// Identifies a turn: an (incoming link, outgoing link) pair at a node.
    TurnId, Turn);
define_id!(/// Identifies a network layer.
    LayerId, Layer);
define_id!(/// Identifies an access point between a layer and a hub.
    AccessPointId, AccessPoint);
define_id!(/// Identifies a hub — the capacitated transfer point.
    HubId, Hub);
define_id!(/// Identifies a typed store held by a hub.
    ResourceId, Resource);
define_id!(/// Identifies a zone.
    ZoneId, Zone);
define_id!(/// Identifies a path in the route store.
    PathId, Path);
define_id!(/// Identifies an individual traveller.
    TravellerId, Traveller);
define_id!(/// Identifies one trip of one traveller.
    TripId, Trip);
define_id!(/// Identifies an individual vehicle.
    VehicleId, Vehicle);
define_id!(/// Identifies one transit trip instance on the service date.
    TransitRunId, TransitRun);
define_id!(/// Identifies a user class.
    UserClassId, UserClass);

/// Builds an [`ExternalIdTable`], assigning internal ids deterministically.
///
/// External ids — an OSM way id, a GTFS `stop_id`, a person id from a MATSim
/// plan — are collected here, then **sorted and deduplicated**; the internal
/// id of an external id is its rank in that sorted order. This is what makes
/// two builds of the same input produce the same id assignment, whatever order
/// the input happened to arrive in.
///
/// # Examples
///
/// ```
/// use openmobisim_core_types::ids::{ExternalIdTableBuilder, NodeId, EntityId};
///
/// let mut b = ExternalIdTableBuilder::new();
/// b.insert("way/77");
/// b.insert("way/12");
/// b.insert("way/77"); // duplicates collapse
/// let table = b.build();
///
/// assert_eq!(table.len(), 2);
/// // Sorted order decides the ids: "way/12" < "way/77" bytewise.
/// assert_eq!(table.external(0), "way/12");
/// assert_eq!(table.id_of("way/77"), Some(1));
/// assert_eq!(table.typed_id_of::<NodeId>("way/77"), Some(NodeId::new(1)));
/// assert_eq!(table.id_of("way/99"), None);
/// ```
#[derive(Clone, Debug, Default)]
pub struct ExternalIdTableBuilder {
    items: Vec<String>,
}

impl ExternalIdTableBuilder {
    /// An empty builder.
    #[must_use]
    pub const fn new() -> Self {
        Self { items: Vec::new() }
    }

    /// An empty builder with room for `capacity` external ids.
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self { items: Vec::with_capacity(capacity) }
    }

    /// Offer an external id. Duplicates are collapsed by [`build`](Self::build).
    pub fn insert(&mut self, external: impl Into<String>) {
        self.items.push(external.into());
    }

    /// Offer many external ids.
    pub fn extend<S: Into<String>>(&mut self, iter: impl IntoIterator<Item = S>) {
        self.items.extend(iter.into_iter().map(Into::into));
    }

    /// How many external ids have been offered, before deduplication.
    #[must_use]
    pub fn offered(&self) -> usize {
        self.items.len()
    }

    /// Sort, deduplicate and freeze into an [`ExternalIdTable`].
    ///
    /// # Panics
    ///
    /// Panics if the ids do not fit the `u32` space ([`MAX_ID`]) or if their
    /// concatenated length exceeds `u32::MAX` bytes. Both are "your city is
    /// not a city" conditions, and both are bugs rather than data problems.
    #[must_use]
    pub fn build(mut self) -> ExternalIdTable {
        self.items.sort_unstable();
        self.items.dedup();

        assert!(
            self.items.len() <= MAX_ID as usize,
            "external id space exceeds the u32 id space ({} ids)",
            self.items.len()
        );
        let total: usize = self.items.iter().map(String::len).sum();
        assert!(
            total <= u32::MAX as usize,
            "concatenated external ids exceed 4 GiB ({total} bytes)"
        );

        let mut buf = String::with_capacity(total);
        let mut offsets = Vec::with_capacity(self.items.len() + 1);
        offsets.push(0u32);
        for item in &self.items {
            buf.push_str(item);
            #[allow(
                clippy::cast_possible_truncation,
                reason = "the total length was asserted to fit u32 above"
            )]
            let end = buf.len() as u32;
            offsets.push(end);
        }

        ExternalIdTable { buf, offsets }
    }
}

/// A frozen, two-way map between internal ids and external ids.
///
/// Internal id → external id is a slice of a single arena, so the table costs
/// one allocation rather than one per id. External id → internal id is a
/// binary search, which is exact precisely *because* ids were assigned in
/// sorted order: the invariant and the lookup are the same fact.
///
/// External ids appear in outputs. They never appear in hot arrays.
#[derive(Clone, Debug, Default)]
pub struct ExternalIdTable {
    buf: String,
    /// `len() + 1` entries; entry `i..i+1` brackets external id `i`.
    offsets: Vec<u32>,
}

impl ExternalIdTable {
    /// How many external ids the table holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.offsets.len().saturating_sub(1)
    }

    /// How many external ids the table holds, as a `u32`.
    ///
    /// Always exact: [`ExternalIdTableBuilder::build`] refuses to produce a
    /// table larger than [`MAX_ID`].
    #[inline]
    #[must_use]
    pub fn count(&self) -> u32 {
        #[allow(
            clippy::cast_possible_truncation,
            reason = "the builder asserts the table fits the u32 id space"
        )]
        let n = self.len() as u32;
        n
    }

    /// Whether the table is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The external id for an internal raw id.
    ///
    /// # Panics
    ///
    /// Panics if `id` is out of range.
    #[must_use]
    pub fn external(&self, id: u32) -> &str {
        let i = id as usize;
        assert!(i < self.len(), "external id {id} out of range (len {})", self.len());
        let start = self.offsets[i] as usize;
        let end = self.offsets[i + 1] as usize;
        &self.buf[start..end]
    }

    /// The external id for a typed id, or `None` if it is null or out of range.
    #[must_use]
    pub fn external_of<I: EntityId>(&self, id: I) -> Option<&str> {
        if id.is_null() || (id.raw() as usize) >= self.len() {
            None
        } else {
            Some(self.external(id.raw()))
        }
    }

    /// The internal raw id for an external id, or `None` if absent.
    #[must_use]
    pub fn id_of(&self, external: &str) -> Option<u32> {
        let mut lo = 0u32;
        let mut hi = self.count();
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            match self.external(mid).cmp(external) {
                core::cmp::Ordering::Less => lo = mid + 1,
                core::cmp::Ordering::Greater => hi = mid,
                core::cmp::Ordering::Equal => return Some(mid),
            }
        }
        None
    }

    /// The typed internal id for an external id, or `None` if absent.
    #[must_use]
    pub fn typed_id_of<I: EntityId>(&self, external: &str) -> Option<I> {
        self.id_of(external).map(I::new)
    }

    /// Every external id, in internal-id order.
    pub fn iter(&self) -> impl Iterator<Item = &str> + '_ {
        (0..self.count()).map(|i| self.external(i))
    }

    /// Bytes held by the arena, for the memory figures in the manifest.
    #[must_use]
    pub fn arena_bytes(&self) -> usize {
        self.buf.len() + self.offsets.len() * size_of::<u32>()
    }
}
