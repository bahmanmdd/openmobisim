//! Dense `u32` identifiers, one kind per entity, with a null sentinel.
//!
//! # Why `u32`, and why a sentinel
//!
//! Identifiers sit in every array of the simulator. A `u32` covers any city (10⁵ to 10⁶ links,
//! 10⁷ trips) at half the memory of a `u64`. [`NULL_ID`] (`u32::MAX`) marks "absent": a stored
//! array of optional identifiers uses the sentinel, never `Option<Id>`, which takes eight bytes
//! where the sentinel takes four. [`EntityId::to_option`] converts at the boundary.
//!
//! # Assignment by sorted external id
//!
//! Internal identifiers are assigned by sorting the external ones (an OpenStreetMap node id, a
//! GTFS `stop_id`, a trip id from a demand table), never by file order or hash-map order. The
//! same set of external ids therefore gives the same internal ids whatever order it arrived in.
//! [`ExternalIdTableBuilder`] is the one way to do it.

use core::fmt;
use core::hash::Hash;

/// The null sentinel shared by every identifier kind.
pub const NULL_ID: u32 = u32::MAX;

/// The largest assignable raw identifier, one less than [`NULL_ID`].
pub const MAX_ID: u32 = u32::MAX - 1;

/// The entity kinds that have their own identifier space.
///
/// The discriminants order simultaneous events: the event key is `(second, kind, id)`, so at
/// equal seconds a lower kind fires first. Reordering them changes results.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
#[repr(u8)]
pub enum EntityKind {
    /// A network node.
    Node = 0,
    /// A link, with an upstream end and a downstream end.
    Link = 1,
    /// A zone: an origin or destination area and a reporting unit.
    Zone = 2,
    /// A path: a sequence of links an alternative follows.
    Path = 3,
    /// A trip: one traveller going from an origin to a destination at a departure second.
    Trip = 4,
    /// An individually identified vehicle.
    Vehicle = 5,
    /// One run of a transit trip on the service date.
    TransitRun = 6,
    /// A user class.
    UserClass = 7,
    /// A transit stop: where runs call. Not a node: a stop knows its position only, and linking
    /// it to the walk network is the caller's.
    Stop = 8,
}

impl EntityKind {
    /// Every kind, in discriminant order.
    pub const ALL: [EntityKind; 9] = [
        EntityKind::Node,
        EntityKind::Link,
        EntityKind::Zone,
        EntityKind::Path,
        EntityKind::Trip,
        EntityKind::Vehicle,
        EntityKind::TransitRun,
        EntityKind::UserClass,
        EntityKind::Stop,
    ];

    /// The snake_case name written to outputs.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            EntityKind::Node => "node",
            EntityKind::Link => "link",
            EntityKind::Zone => "zone",
            EntityKind::Path => "path",
            EntityKind::Trip => "trip",
            EntityKind::Vehicle => "vehicle",
            EntityKind::TransitRun => "transit_run",
            EntityKind::UserClass => "user_class",
            EntityKind::Stop => "stop",
        }
    }
}

impl fmt::Display for EntityKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The operations every identifier kind shares.
///
/// A function that needs "some identifier" takes `I: EntityId` rather than a raw `u32`, so the
/// kind cannot be mixed up at a call site.
pub trait EntityId: Copy + Eq + Ord + Hash + fmt::Debug + fmt::Display {
    /// The entity kind this identifier addresses.
    const KIND: EntityKind;

    /// The null sentinel of this kind.
    const NULL: Self;

    /// Wrap a raw value that is known not to be null.
    ///
    /// # Panics
    ///
    /// In debug builds, if `raw` is [`NULL_ID`]. Use [`from_raw`](EntityId::from_raw) for a value
    /// that may be null.
    fn new(raw: u32) -> Self;

    /// Wrap a raw value that may be [`NULL_ID`].
    fn from_raw(raw: u32) -> Self;

    /// Wrap a `usize` array index.
    ///
    /// # Panics
    ///
    /// In debug builds, if `index` exceeds [`MAX_ID`].
    fn from_index(index: usize) -> Self;

    /// The raw value, [`NULL_ID`] if null.
    fn raw(self) -> u32;

    /// The value as a `usize` array index.
    ///
    /// # Panics
    ///
    /// In debug builds, if called on the null sentinel: indexing with a null identifier is a bug.
    fn index(self) -> usize;

    /// Whether this is the null sentinel.
    fn is_null(self) -> bool;

    /// `None` if null, otherwise `Some(self)`. For interfaces, never for stored arrays.
    fn to_option(self) -> Option<Self> {
        if self.is_null() { None } else { Some(self) }
    }

    /// The identifiers `0..count`, in order.
    fn iter_space(count: u32) -> impl Iterator<Item = Self> {
        (0..count).map(Self::new)
    }
}

macro_rules! define_id {
    ($(#[$doc:meta])* $name:ident, $kind:ident) => {
        $(#[$doc])*
        ///
        /// A dense `u32` index; [`NULL_ID`] is the null sentinel.
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
                    concat!(stringify!($name), "::from_index overflowed the u32 identifier space")
                );
                #[allow(
                    clippy::cast_possible_truncation,
                    reason = "guarded by the debug assertion above; identifiers are u32 by design"
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
                if self.0 == NULL_ID { f.write_str("null") } else { write!(f, "{}", self.0) }
            }
        }
    };
}

define_id!(/// Identifies a network node.
    NodeId, Node);
define_id!(/// Identifies a link.
    LinkId, Link);
define_id!(/// Identifies a zone.
    ZoneId, Zone);
define_id!(/// Identifies a path.
    PathId, Path);
define_id!(/// Identifies a trip, which is also its traveller.
    TripId, Trip);
define_id!(/// Identifies an individual vehicle.
    VehicleId, Vehicle);
define_id!(/// Identifies one run of a transit trip on the service date.
    TransitRunId, TransitRun);
define_id!(/// Identifies a user class.
    UserClassId, UserClass);
define_id!(/// Identifies a transit stop (not a node: see [`EntityKind::Stop`]).
    StopId, Stop);

/// Builds an [`ExternalIdTable`], assigning internal identifiers by sorted external id.
///
/// External ids are collected, then sorted bytewise and deduplicated; the internal identifier
/// of an external id is its rank in that order.
///
/// # Examples
///
/// ```
/// use openmobisim_types::ids::{EntityId, ExternalIdTableBuilder, NodeId};
///
/// let mut b = ExternalIdTableBuilder::new();
/// b.insert("way/77");
/// b.insert("way/12");
/// b.insert("way/77"); // duplicates collapse
/// let table = b.build();
///
/// assert_eq!(table.len(), 2);
/// assert_eq!(table.external(0), "way/12"); // "way/12" < "way/77" bytewise
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

    /// Offer an external id. Duplicates are collapsed by [`build`](Self::build).
    pub fn insert(&mut self, external: impl Into<String>) {
        self.items.push(external.into());
    }

    /// Offer many external ids.
    pub fn extend<S: Into<String>>(&mut self, iter: impl IntoIterator<Item = S>) {
        self.items.extend(iter.into_iter().map(Into::into));
    }

    /// Sort, deduplicate and freeze into an [`ExternalIdTable`].
    ///
    /// # Panics
    ///
    /// If the ids do not fit the `u32` space ([`MAX_ID`]) or their concatenated length exceeds
    /// `u32::MAX` bytes.
    #[must_use]
    pub fn build(mut self) -> ExternalIdTable {
        self.items.sort_unstable();
        self.items.dedup();

        assert!(
            self.items.len() <= MAX_ID as usize,
            "external ids exceed the u32 identifier space ({} ids)",
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

/// A frozen two-way map between internal identifiers and external ids.
///
/// Internal to external is a slice of one string arena; external to internal is a binary
/// search, exact because the identifiers were assigned in sorted order.
#[derive(Clone, Debug, Default)]
pub struct ExternalIdTable {
    buf: String,
    /// `len() + 1` entries; entries `i` and `i + 1` bracket external id `i`.
    offsets: Vec<u32>,
}

impl ExternalIdTable {
    /// How many external ids the table holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.offsets.len().saturating_sub(1)
    }

    /// How many external ids the table holds, as a `u32` (exact: the builder caps it).
    #[must_use]
    pub fn count(&self) -> u32 {
        #[allow(
            clippy::cast_possible_truncation,
            reason = "the builder asserts the table fits the u32 identifier space"
        )]
        let n = self.len() as u32;
        n
    }

    /// Whether the table is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The external id of an internal raw identifier.
    ///
    /// # Panics
    ///
    /// If `id` is out of range.
    #[must_use]
    pub fn external(&self, id: u32) -> &str {
        let i = id as usize;
        assert!(i < self.len(), "external id {id} out of range (len {})", self.len());
        &self.buf[self.offsets[i] as usize..self.offsets[i + 1] as usize]
    }

    /// The external id of a typed identifier, or `None` if it is null or out of range.
    #[must_use]
    pub fn external_of<I: EntityId>(&self, id: I) -> Option<&str> {
        if id.is_null() || (id.raw() as usize) >= self.len() {
            None
        } else {
            Some(self.external(id.raw()))
        }
    }

    /// The internal raw identifier of an external id, or `None` if absent.
    #[must_use]
    pub fn id_of(&self, external: &str) -> Option<u32> {
        let (mut lo, mut hi) = (0u32, self.count());
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

    /// The typed internal identifier of an external id, or `None` if absent.
    #[must_use]
    pub fn typed_id_of<I: EntityId>(&self, external: &str) -> Option<I> {
        self.id_of(external).map(I::new)
    }

    /// Every external id, in internal-identifier order.
    pub fn iter(&self) -> impl Iterator<Item = &str> + '_ {
        (0..self.count()).map(|i| self.external(i))
    }
}
