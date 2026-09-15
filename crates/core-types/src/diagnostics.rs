//! Run diagnostics: the counters that make "the simulation never stops"
//! honest (Foundations §6, brief §3c).
//!
//! # The bargain
//!
//! openmobisim does not stop for missing data, an unsnappable stop, an infeasible
//! path or an overdrawn dock. Those are normal research conditions, not
//! errors: each is resolved by a documented rule and the run continues. Only a
//! **bug** — a violated invariant such as vehicle non-conservation — raises.
//!
//! What makes that defensible rather than merely convenient is the other half
//! of the bargain: **every run writes a structured diagnostics report, always,
//! and it cannot be switched off.** If the model quietly did something, the
//! report says so, how often, and to what.
//!
//! # Cost
//!
//! Instrumentation pays on the exception path only (brief §3e). A
//! [`Diagnostics`] that records nothing allocates nothing: the maps are empty
//! `HashMap`s, which do not allocate until first insert. Recording is two hash
//! lookups, and it happens only where the model has already decided to do
//! something worth reporting.
//!
//! # Cardinality
//!
//! Per-element detail is capped at [`MAX_ELEMENTS_PER_CODE`] distinct elements
//! per code. Beyond the cap the count still accrues, against
//! [`ElementRef::NONE`] — so totals stay exact while memory stays bounded.
//! A run on a broken network reports "4096 links plus 812 000 more", not an
//! out-of-memory kill.

use std::collections::HashMap;

use crate::ids::{EntityId, EntityKind};

/// How many distinct elements one code keeps individual counts for.
///
/// Past this, counts for that code roll into [`ElementRef::NONE`]. The total
/// per code is always exact; only the per-element breakdown is truncated.
pub const MAX_ELEMENTS_PER_CODE: usize = 4096;

/// Which phase produced a diagnostic.
///
/// There is deliberately **no `error` category**: errors raise.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
#[repr(u8)]
pub enum Category {
    /// A build-time input problem resolved by a documented fallback:
    /// a missing GTFS shape, an untagged OSM link, an unsnappable stop.
    DataQuality = 0,
    /// A run-time modelling condition resolved by the model's own rules:
    /// no feasible path, over capacity, an overdrawn dock.
    Modelling = 1,
    /// A numerical condition: a sweep that hit its iteration cap, a
    /// convergence gap that stalled above its sampling floor.
    Numeric = 2,
}

impl Category {
    /// Every category, in discriminant order.
    pub const ALL: [Category; 3] = [Category::DataQuality, Category::Modelling, Category::Numeric];

    /// The stable snake_case name written to `diagnostics.parquet`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Category::DataQuality => "data_quality",
            Category::Modelling => "modelling",
            Category::Numeric => "numeric",
        }
    }
}

/// How much a diagnostic should worry the reader.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
#[repr(u8)]
pub enum Severity {
    /// Something was resolved by a rule the documentation states. Expected.
    Info = 0,
    /// Something was resolved by a rule the documentation states, but the
    /// result is likely to be worth the analyst's attention.
    Warning = 1,
}

impl Severity {
    /// The stable snake_case name written to `diagnostics.parquet`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Severity::Info => "info",
            Severity::Warning => "warning",
        }
    }
}

/// A stable, machine-readable diagnostic code.
///
/// Codes are `&'static str` rather than an enum so that a plugin can declare
/// its own without modifying the core — "add, don't touch". They are part of
/// the output schema: a downstream script filters on them, so **a code that
/// has shipped is never renamed or reused for a different meaning**.
///
/// Core codes live in [`codes`]; a plugin should prefix its own with its
/// plugin name.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct DiagCode(pub &'static str);

impl DiagCode {
    /// The code as it appears in output.
    #[inline]
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

impl core::fmt::Display for DiagCode {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.0)
    }
}

/// The core diagnostic codes.
///
/// Each is carried by a specific mechanism named in Foundations §6. Phase 1
/// only emits a few of them; the rest are declared here so that the vocabulary
/// is settled in one place and cannot drift as the mechanisms land.
pub mod codes {
    use super::DiagCode;

    /// A map-matching fallback rung was used for a route.
    pub const MATCHER_FALLBACK: DiagCode = DiagCode("matcher_fallback");
    /// A walk or bike table was truncated at its cutoff.
    pub const ACCESS_CUTOFF_TRUNCATED: DiagCode = DiagCode("access_cutoff_truncated");
    /// A vehicle was displaced rather than returned to its home overnight.
    pub const VEHICLE_DISPLACED: DiagCode = DiagCode("vehicle_displaced");
    /// A scheduled disruption was applied.
    pub const DISRUPTION_APPLIED: DiagCode = DiagCode("disruption_applied");
    /// A trip did not complete within the simulation window.
    pub const TRIP_TRUNCATED: DiagCode = DiagCode("trip_truncated");
    /// A node model sweep blocked an approach.
    pub const APPROACH_BLOCKED: DiagCode = DiagCode("approach_blocked");
    /// Anchor candidates were truncated at their cap.
    pub const ANCHOR_CANDIDATES_TRUNCATED: DiagCode = DiagCode("anchor_candidates_truncated");
    /// A store was drawn below zero and resolved by its soft-capacity rule.
    pub const STORE_OVERDRAFT: DiagCode = DiagCode("store_overdraft");
    /// Within-step vehicle hand-over did not converge within its sweep cap.
    pub const HANDOVER_NOT_CONVERGED: DiagCode = DiagCode("handover_not_converged");
    /// The route store exceeded its configured budget.
    pub const ROUTE_STORE_BUDGET_EXCEEDED: DiagCode = DiagCode("route_store_budget_exceeded");
    /// No feasible path existed for a trip; the documented fallback was used.
    pub const NO_FEASIBLE_PATH: DiagCode = DiagCode("no_feasible_path");
    /// Per-element diagnostic detail was capped; see [`super::MAX_ELEMENTS_PER_CODE`].
    pub const DIAGNOSTIC_DETAIL_TRUNCATED: DiagCode = DiagCode("diagnostic_detail_truncated");
}

/// Which element a diagnostic is about, or none.
///
/// Four bytes of id plus one of kind, with `0xFF` meaning "no element" — the
/// same reasoning as the id sentinel: an `Option<(EntityKind, u32)>` would be
/// eight bytes, and these sit in a map with one entry per reported element.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct ElementRef {
    kind: u8,
    id: u32,
}

impl ElementRef {
    const NO_KIND: u8 = 0xFF;

    /// A diagnostic about the run as a whole rather than about one element.
    pub const NONE: ElementRef = ElementRef { kind: Self::NO_KIND, id: u32::MAX };

    /// A diagnostic about one entity, given its kind and raw id.
    #[inline]
    #[must_use]
    pub const fn new(kind: EntityKind, id: u32) -> Self {
        Self { kind: kind as u8, id }
    }

    /// A diagnostic about one entity, taking the kind from the id's type.
    #[inline]
    #[must_use]
    pub fn of<I: EntityId>(id: I) -> Self {
        Self { kind: I::KIND as u8, id: id.raw() }
    }

    /// The entity kind, or `None` for [`ElementRef::NONE`].
    #[inline]
    #[must_use]
    pub const fn kind(self) -> Option<EntityKind> {
        EntityKind::from_u8(self.kind)
    }

    /// The raw id, or `None` for [`ElementRef::NONE`].
    #[inline]
    #[must_use]
    pub const fn id(self) -> Option<u32> {
        if self.kind == Self::NO_KIND { None } else { Some(self.id) }
    }
}

/// What identifies one line of the diagnostics report.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct DiagKey {
    /// Which phase produced it.
    pub category: Category,
    /// The stable code.
    pub code: DiagCode,
    /// How much it should worry the reader.
    pub severity: Severity,
    /// Which element it is about.
    pub element: ElementRef,
}

impl DiagKey {
    /// A diagnostic about one element.
    #[inline]
    #[must_use]
    pub const fn new(
        category: Category,
        code: DiagCode,
        severity: Severity,
        element: ElementRef,
    ) -> Self {
        Self { category, code, severity, element }
    }

    /// A diagnostic about the run as a whole.
    #[inline]
    #[must_use]
    pub const fn run_level(category: Category, code: DiagCode, severity: Severity) -> Self {
        Self { category, code, severity, element: ElementRef::NONE }
    }
}

/// One row of `diagnostics.parquet`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct DiagRow {
    /// What the row is about.
    pub key: DiagKey,
    /// How many times it happened.
    pub count: u64,
}

/// The per-run diagnostic counters.
///
/// Owned by exactly one run — there is no global and no thread-local. Work
/// split across threads collects into per-chunk instances and
/// [`merge`](Diagnostics::merge)s them, which keeps the result independent of
/// how the work was split.
///
/// # Examples
///
/// ```
/// use openmobisim_core_types::diagnostics::{
///     codes, Category, DiagKey, Diagnostics, ElementRef, Severity,
/// };
/// use openmobisim_core_types::ids::{EntityId, LinkId};
///
/// let mut diag = Diagnostics::new();
/// assert!(diag.is_empty()); // and has allocated nothing
///
/// diag.record(DiagKey::new(
///     Category::Modelling,
///     codes::NO_FEASIBLE_PATH,
///     Severity::Warning,
///     ElementRef::of(LinkId::new(42)),
/// ));
///
/// assert_eq!(diag.total(), 1);
/// assert_eq!(diag.count_of(codes::NO_FEASIBLE_PATH), 1);
/// ```
#[derive(Clone, Debug, Default)]
pub struct Diagnostics {
    counts: HashMap<DiagKey, u64>,
    /// How many distinct elements each (category, code, severity) has seen,
    /// so that the cap can be applied without scanning `counts`.
    distinct: HashMap<(Category, DiagCode, Severity), usize>,
}

impl Diagnostics {
    /// An empty report. Allocates nothing until something is recorded.
    #[must_use]
    pub fn new() -> Self {
        Self { counts: HashMap::new(), distinct: HashMap::new() }
    }

    /// Record one occurrence.
    #[inline]
    pub fn record(&mut self, key: DiagKey) {
        self.record_n(key, 1);
    }

    /// Record `n` occurrences.
    ///
    /// Recording zero is a no-op, so a caller may pass a computed count
    /// without guarding it.
    pub fn record_n(&mut self, key: DiagKey, n: u64) {
        if n == 0 {
            return;
        }
        let key = self.apply_cap(key);
        *self.counts.entry(key).or_insert(0) += n;
    }

    /// Record one occurrence about a whole run rather than an element.
    #[inline]
    pub fn record_run_level(&mut self, category: Category, code: DiagCode, severity: Severity) {
        self.record(DiagKey::run_level(category, code, severity));
    }

    /// Fold `other` into `self`.
    ///
    /// Addition is associative and exact on `u64`, so the merged result does
    /// not depend on how work was chunked — the same property
    /// [`crate::reduce`] buys for floating-point sums.
    pub fn merge(&mut self, other: &Diagnostics) {
        for (&key, &count) in &other.counts {
            self.record_n(key, count);
        }
    }

    /// Whether anything at all was recorded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.counts.is_empty()
    }

    /// How many distinct rows the report holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.counts.len()
    }

    /// The total number of occurrences across every row.
    #[must_use]
    pub fn total(&self) -> u64 {
        self.counts.values().sum()
    }

    /// The total recorded against one code, across every element and severity.
    #[must_use]
    pub fn count_of(&self, code: DiagCode) -> u64 {
        self.counts.iter().filter(|(k, _)| k.code == code).map(|(_, &c)| c).sum()
    }

    /// The count for one exact key.
    #[must_use]
    pub fn count_of_key(&self, key: DiagKey) -> u64 {
        self.counts.get(&key).copied().unwrap_or(0)
    }

    /// Every row, in a deterministic order.
    ///
    /// Sorted by `(category, code, severity, element)`, so two runs that
    /// recorded the same things write byte-identical reports — which is what
    /// lets the run-twice CI gate compare them directly.
    #[must_use]
    pub fn rows(&self) -> Vec<DiagRow> {
        let mut rows: Vec<DiagRow> =
            self.counts.iter().map(|(&key, &count)| DiagRow { key, count }).collect();
        rows.sort_unstable_by_key(|r| r.key);
        rows
    }

    /// Redirect a key to [`ElementRef::NONE`] once its code has reported on
    /// [`MAX_ELEMENTS_PER_CODE`] distinct elements.
    fn apply_cap(&mut self, key: DiagKey) -> DiagKey {
        if key.element == ElementRef::NONE || self.counts.contains_key(&key) {
            return key;
        }
        let group = (key.category, key.code, key.severity);
        let seen = self.distinct.entry(group).or_insert(0);
        if *seen >= MAX_ELEMENTS_PER_CODE {
            DiagKey { element: ElementRef::NONE, ..key }
        } else {
            *seen += 1;
            key
        }
    }
}
