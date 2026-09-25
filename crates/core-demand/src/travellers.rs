//! Travellers as structure-of-arrays with integer weights (S86, S94), built
//! from the raw rows [`crate::trips`] and [`crate::persons`] read.
//!
//! [`build`] is where S127 (ownership) and S128 (trip-chain continuity) are
//! actually enforced — both need every trip of a traveller gathered and
//! ordered, which is exactly what this module does on the way to producing
//! [`Travellers`] and [`Trips`].

use std::collections::HashMap;

use openmobisim_core_graph::geometry::LonLat;
use openmobisim_core_types::diagnostics::{Category, DiagKey, Diagnostics, ElementRef, Severity};
use openmobisim_core_types::ids::{
    EntityId, ExternalIdTable, ExternalIdTableBuilder, TravellerId, TripId, UserClassId,
};
use openmobisim_core_types::time::Second;

use crate::DemandError;
use crate::mode::Mode;
use crate::persons::RawPerson;
use crate::trips::RawTrip;

/// Diagnostic codes this module records.
pub mod codes {
    use openmobisim_core_types::diagnostics::DiagCode;

    /// Trip *n*+1's stated origin disagreed with trip *n*'s destination; the
    /// chain wins (S128). Counted per traveller.
    pub const TRIP_CHAIN_ORIGIN_OVERRIDDEN: DiagCode = DiagCode("trip_chain_origin_overridden");
    /// Two rows for the same traveller stated different `trip_seq` values
    /// resolving to a duplicate after sorting; all but the first (in file
    /// order) were dropped.
    pub const DUPLICATE_TRIP_SEQ: DiagCode = DiagCode("duplicate_trip_seq");
    /// A traveller's trip rows disagreed on `weight`; the first stated value
    /// was kept.
    pub const INCONSISTENT_TRAVELLER_WEIGHT: DiagCode = DiagCode("inconsistent_traveller_weight");
    /// A traveller's trip rows disagreed on `user_class`; the first stated
    /// value was kept (before any `persons.parquet` override).
    pub const INCONSISTENT_USER_CLASS: DiagCode = DiagCode("inconsistent_user_class");
    /// `persons.parquet` overrode at least one ownership field, or the user
    /// class, for this traveller (S127). Counted per traveller — this is the
    /// figure S127 says the manifest should report.
    pub const OWNERSHIP_OVERRIDDEN: DiagCode = DiagCode("ownership_overridden");
    /// A `persons.parquet` row's `traveller_id` matched no traveller in
    /// `trips.parquet`; the row was ignored.
    pub const PERSON_UNKNOWN_TRAVELLER: DiagCode = DiagCode("person_unknown_traveller");
    /// Two `persons.parquet` rows named the same traveller; all but the first
    /// (in file order) were dropped.
    pub const DUPLICATE_PERSON_ROW: DiagCode = DiagCode("duplicate_person_row");
}

/// A traveller's vehicle and pass ownership (S127, §23).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Ownership {
    /// Owns a car — a fleet of size one with a restrictive access policy
    /// (design §23.1).
    pub car: bool,
    /// Owns a bike.
    pub bike: bool,
    /// Holds a transit pass.
    pub transit_pass: bool,
}

impl Ownership {
    /// Owns nothing.
    pub const NONE: Ownership = Ownership { car: false, bike: false, transit_pass: false };

    /// Apply a `persons.parquet` row's overrides, field by field — each of
    /// `owns_car`, `owns_bike`, `has_transit_pass` overrides independently of
    /// the others, matching the interface table's "omit the column to take
    /// the class default for everyone".
    ///
    /// Returns whether anything was actually overridden, which is what
    /// [`codes::OWNERSHIP_OVERRIDDEN`] counts.
    #[must_use]
    fn override_from(self, person: &RawPerson) -> (Ownership, bool) {
        let mut overridden = false;
        let mut out = self;
        if let Some(car) = person.owns_car {
            out.car = car;
            overridden = true;
        }
        if let Some(bike) = person.owns_bike {
            out.bike = bike;
            overridden = true;
        }
        if let Some(pass) = person.has_transit_pass {
            out.transit_pass = pass;
            overridden = true;
        }
        (out, overridden)
    }
}

/// Per-user-class default ownership (S127) — a build parameter, not
/// something this crate invents. See the crate docs for why: the
/// `[user_classes]` table that would populate this is scenario schema.
///
/// A class with no declared default owns nothing — a safe, fully-overridable
/// placeholder for as long as no scenario layer supplies real defaults.
#[derive(Clone, Debug, Default)]
pub struct ClassDefaults(HashMap<String, Ownership>);

impl ClassDefaults {
    /// No class has a declared default; every traveller owns nothing unless
    /// `persons.parquet` says otherwise.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Declare `class`'s default ownership.
    #[must_use]
    pub fn with_default(mut self, class: impl Into<String>, ownership: Ownership) -> Self {
        self.0.insert(class.into(), ownership);
        self
    }

    /// `class`'s default, or [`Ownership::NONE`] if undeclared.
    #[must_use]
    pub fn default_for(&self, class: &str) -> Ownership {
        self.0.get(class).copied().unwrap_or(Ownership::NONE)
    }
}

/// Trips, as a dense structure-of-arrays keyed by [`TripId`].
///
/// Grouped by traveller and ordered by `trip_seq` within each — the same
/// order [`Travellers::trips_of`] hands back ranges into. Origins here are
/// **after** S128's trip-chain correction; [`RawTrip`] keeps what the file
/// actually said.
#[derive(Clone, Debug, Default)]
pub struct Trips {
    traveller: Vec<TravellerId>,
    departure: Vec<Second>,
    origin: Vec<LonLat>,
    destination: Vec<LonLat>,
    mode: Vec<Mode>,
}

impl Trips {
    /// How many trips, across every traveller.
    #[must_use]
    pub fn len(&self) -> u32 {
        #[allow(clippy::cast_possible_truncation, reason = "bounded by trips.parquet's row count")]
        let n = self.traveller.len() as u32;
        n
    }

    /// Whether there are no trips at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.traveller.is_empty()
    }

    /// The traveller this trip belongs to.
    #[must_use]
    pub fn traveller(&self, trip: TripId) -> TravellerId {
        self.traveller[trip.index()]
    }

    /// The trip's mode (S195): [`Mode::Car`] where the file stated none.
    #[must_use]
    pub fn mode(&self, trip: TripId) -> Mode {
        self.mode[trip.index()]
    }

    /// Departure time.
    #[must_use]
    pub fn departure(&self, trip: TripId) -> Second {
        self.departure[trip.index()]
    }

    /// Origin, after S128's trip-chain correction.
    #[must_use]
    pub fn origin(&self, trip: TripId) -> LonLat {
        self.origin[trip.index()]
    }

    /// Destination.
    #[must_use]
    pub fn destination(&self, trip: TripId) -> LonLat {
        self.destination[trip.index()]
    }
}

/// Travellers as structure-of-arrays (S86), the immutable shared input a
/// [`Trips`] table accompanies.
///
/// Per-run mutable state — vehicle locations, choice state — is deliberately
/// **not** here (Foundations §5); see [`crate::vehicles::VehicleLocations`].
#[derive(Clone, Debug, Default)]
pub struct Travellers {
    ids: ExternalIdTable,
    class_ids: ExternalIdTable,
    weight: Vec<u32>,
    user_class: Vec<UserClassId>,
    ownership: Vec<Ownership>,
    /// `traveller_count() + 1` entries; traveller `t`'s trips are
    /// `trip_offsets[t]..trip_offsets[t + 1]` into the accompanying [`Trips`].
    trip_offsets: Vec<u32>,
}

impl Travellers {
    /// How many travellers.
    #[must_use]
    pub fn len(&self) -> u32 {
        self.ids.count()
    }

    /// Whether there are no travellers at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    /// The external id table, for outputs.
    #[must_use]
    pub fn external_ids(&self) -> &ExternalIdTable {
        &self.ids
    }

    /// The user-class external id table, for outputs.
    #[must_use]
    pub fn class_external_ids(&self) -> &ExternalIdTable {
        &self.class_ids
    }

    /// This traveller's weight — the number of people they represent, and
    /// the PCU their vehicle contributes (S86).
    #[must_use]
    pub fn weight(&self, traveller: TravellerId) -> u32 {
        self.weight[traveller.index()]
    }

    /// This traveller's user class.
    #[must_use]
    pub fn user_class(&self, traveller: TravellerId) -> UserClassId {
        self.user_class[traveller.index()]
    }

    /// This traveller's resolved ownership (S127).
    #[must_use]
    pub fn ownership(&self, traveller: TravellerId) -> Ownership {
        self.ownership[traveller.index()]
    }

    /// The range of [`TripId`]s this traveller's trips occupy in the
    /// accompanying [`Trips`], in `trip_seq` order.
    pub fn trips_of(&self, traveller: TravellerId) -> impl Iterator<Item = TripId> + '_ {
        let start = self.trip_offsets[traveller.index()];
        let end = self.trip_offsets[traveller.index() + 1];
        (start..end).map(TripId::new)
    }

    /// This traveller's first trip — always present, since a traveller
    /// exists only because `trips.parquet` named them at least once.
    #[must_use]
    pub fn first_trip(&self, traveller: TravellerId) -> TripId {
        TripId::new(self.trip_offsets[traveller.index()])
    }
}

/// Build [`Travellers`] and [`Trips`] from the raw rows the two readers
/// produced.
///
/// `class_defaults` supplies S127's per-class ownership default;
/// `default_weight` is the scenario's `traveller_weight` (S89: 1 for
/// `default`, 10 for `fast`) for travellers no row gives an explicit weight.
///
/// # Errors
///
/// [`DemandError::Schema`] if `trips` is empty — an empty demand file is a
/// scenario error, not a data condition to fall back from silently, the same
/// judgement `io-osm` makes for an empty extract.
///
/// # Panics
///
/// Does not panic on any input. The internal `class_ids.typed_id_of` lookup
/// cannot miss: every string it is asked for was inserted into `class_ids`
/// two lines above, from the same `resolved_class` vector.
pub fn build(
    trips: Vec<RawTrip>,
    persons: Vec<RawPerson>,
    class_defaults: &ClassDefaults,
    default_weight: u32,
    diagnostics: &mut Diagnostics,
) -> Result<(Travellers, Trips), DemandError> {
    if trips.is_empty() {
        return Err(DemandError::Schema("trips.parquet has no rows".to_string()));
    }

    let mut by_traveller: HashMap<String, Vec<RawTrip>> = HashMap::new();
    for trip in trips {
        by_traveller.entry(trip.traveller_id.clone()).or_default().push(trip);
    }

    let mut person_of: HashMap<String, RawPerson> = HashMap::new();
    for person in persons {
        if by_traveller.contains_key(&person.traveller_id) {
            if person_of.contains_key(&person.traveller_id) {
                diagnostics.record_run_level(
                    Category::DataQuality,
                    codes::DUPLICATE_PERSON_ROW,
                    Severity::Info,
                );
            } else {
                person_of.insert(person.traveller_id.clone(), person);
            }
        } else {
            diagnostics.record_run_level(
                Category::DataQuality,
                codes::PERSON_UNKNOWN_TRAVELLER,
                Severity::Warning,
            );
        }
    }

    let mut traveller_ids = ExternalIdTableBuilder::with_capacity(by_traveller.len());
    traveller_ids.extend(by_traveller.keys().cloned());
    let ids: ExternalIdTable = traveller_ids.build();
    let traveller_count = ids.count();

    let mut weight = vec![0u32; traveller_count as usize];
    let mut ownership = vec![Ownership::NONE; traveller_count as usize];
    let mut resolved_class = vec![String::new(); traveller_count as usize];
    let mut trip_offsets = vec![0u32; traveller_count as usize + 1];
    let mut ordered_trips: Vec<Vec<RawTrip>> = vec![Vec::new(); traveller_count as usize];

    for (external, mut group) in by_traveller {
        let Some(raw_id) = ids.id_of(&external) else { unreachable!("just inserted") };
        let t = raw_id as usize;
        let traveller = TravellerId::new(raw_id);

        group.sort_by_key(|trip| trip.trip_seq);
        let before = group.len();
        group.dedup_by_key(|trip| trip.trip_seq);
        let dropped = before - group.len();
        if dropped > 0 {
            diagnostics.record_n(
                DiagKey::new(
                    Category::DataQuality,
                    codes::DUPLICATE_TRIP_SEQ,
                    Severity::Warning,
                    ElementRef::of(traveller),
                ),
                dropped as u64,
            );
        }

        let stated_weight = group.iter().find_map(|trip| trip.weight);
        if group.iter().filter_map(|trip| trip.weight).any(|w| Some(w) != stated_weight) {
            diagnostics.record(DiagKey::new(
                Category::DataQuality,
                codes::INCONSISTENT_TRAVELLER_WEIGHT,
                Severity::Warning,
                ElementRef::of(traveller),
            ));
        }
        weight[t] = stated_weight.unwrap_or(default_weight);

        let stated_class = group[0].user_class.clone();
        if group.iter().any(|trip| trip.user_class != stated_class) {
            diagnostics.record(DiagKey::new(
                Category::DataQuality,
                codes::INCONSISTENT_USER_CLASS,
                Severity::Warning,
                ElementRef::of(traveller),
            ));
        }

        let mut class = stated_class;
        let mut ownership_for_traveller = class_defaults.default_for(&class);
        let mut was_overridden = false;
        if let Some(person) = person_of.get(&external) {
            if let Some(class_override) = &person.user_class {
                class = class_override.clone();
                ownership_for_traveller = class_defaults.default_for(&class);
                was_overridden = true;
            }
            let (with_overrides, field_overridden) = ownership_for_traveller.override_from(person);
            ownership_for_traveller = with_overrides;
            was_overridden |= field_overridden;
        }
        if was_overridden {
            diagnostics.record(DiagKey::new(
                Category::DataQuality,
                codes::OWNERSHIP_OVERRIDDEN,
                Severity::Info,
                ElementRef::of(traveller),
            ));
        }
        ownership[t] = ownership_for_traveller;
        resolved_class[t] = class;

        // S128: trip n+1 starts where trip n ended. Any disagreement between
        // that and the file's stated origin is the file's error to report,
        // not the model's to reproduce.
        let mut chain_overridden = 0u64;
        for i in 1..group.len() {
            let previous_destination = group[i - 1].destination;
            if group[i].origin != previous_destination {
                group[i].origin = previous_destination;
                chain_overridden += 1;
            }
        }
        if chain_overridden > 0 {
            diagnostics.record_n(
                DiagKey::new(
                    Category::DataQuality,
                    codes::TRIP_CHAIN_ORIGIN_OVERRIDDEN,
                    Severity::Info,
                    ElementRef::of(traveller),
                ),
                chain_overridden,
            );
        }

        ordered_trips[t] = group;
    }

    let mut class_id_builder = ExternalIdTableBuilder::with_capacity(traveller_count as usize);
    class_id_builder.extend(resolved_class.iter().cloned());
    let class_ids: ExternalIdTable = class_id_builder.build();
    let user_class: Vec<UserClassId> = resolved_class
        .iter()
        .map(|c| class_ids.typed_id_of::<UserClassId>(c).expect("just interned"))
        .collect();

    let mut cursor = 0u32;
    let mut trips = Trips::default();
    for (t, group) in ordered_trips.into_iter().enumerate() {
        trip_offsets[t] = cursor;
        for trip in group {
            trips.traveller.push(TravellerId::from_index(t));
            trips.departure.push(trip.departure_time);
            trips.origin.push(trip.origin);
            trips.destination.push(trip.destination);
            trips.mode.push(trip.mode.unwrap_or_default());
            cursor += 1;
        }
    }
    trip_offsets[traveller_count as usize] = cursor;

    let travellers = Travellers { ids, class_ids, weight, user_class, ownership, trip_offsets };
    Ok((travellers, trips))
}
