//! Diagnostics: exact totals, bounded memory, order-independent merging.

use openmobisim_core_types::diagnostics::{
    Category, DiagKey, Diagnostics, ElementRef, MAX_ELEMENTS_PER_CODE, Severity, codes,
};
use openmobisim_core_types::ids::{EntityId, EntityKind, LinkId};

fn link_key(id: u32) -> DiagKey {
    DiagKey::new(
        Category::Modelling,
        codes::NO_FEASIBLE_PATH,
        Severity::Warning,
        ElementRef::of(LinkId::new(id)),
    )
}

#[test]
fn an_empty_report_costs_nothing() {
    let diag = Diagnostics::new();
    assert!(diag.is_empty());
    assert_eq!(diag.len(), 0);
    assert_eq!(diag.total(), 0);
    assert!(diag.rows().is_empty());
}

#[test]
fn recording_zero_is_a_no_op() {
    // So a caller may pass a computed count without guarding it.
    let mut diag = Diagnostics::new();
    diag.record_n(link_key(1), 0);
    assert!(diag.is_empty());
}

#[test]
fn counts_accumulate_per_element() {
    let mut diag = Diagnostics::new();
    diag.record(link_key(1));
    diag.record(link_key(1));
    diag.record(link_key(2));

    assert_eq!(diag.len(), 2, "two distinct elements");
    assert_eq!(diag.total(), 3);
    assert_eq!(diag.count_of(codes::NO_FEASIBLE_PATH), 3);
    assert_eq!(diag.count_of_key(link_key(1)), 2);
    assert_eq!(diag.count_of_key(link_key(3)), 0);
}

#[test]
fn element_refs_round_trip() {
    let e = ElementRef::of(LinkId::new(77));
    assert_eq!(e.kind(), Some(EntityKind::Link));
    assert_eq!(e.id(), Some(77));

    assert_eq!(ElementRef::NONE.kind(), None);
    assert_eq!(ElementRef::NONE.id(), None);

    assert_eq!(ElementRef::new(EntityKind::Hub, 4).kind(), Some(EntityKind::Hub));
}

#[test]
fn per_element_detail_is_capped_but_totals_stay_exact() {
    // A run on a broken network must report "4096 links plus many more",
    // not exhaust memory recording every one of them.
    let mut diag = Diagnostics::new();
    let over = u32::try_from(MAX_ELEMENTS_PER_CODE).unwrap() + 5_000;
    for id in 0..over {
        diag.record(link_key(id));
    }

    assert_eq!(diag.total(), u64::from(over), "the total must remain exact");
    assert_eq!(
        diag.len(),
        MAX_ELEMENTS_PER_CODE + 1,
        "capped elements plus the one run-level overflow row"
    );

    let overflow = DiagKey::new(
        Category::Modelling,
        codes::NO_FEASIBLE_PATH,
        Severity::Warning,
        ElementRef::NONE,
    );
    assert_eq!(diag.count_of_key(overflow), 5_000);
}

#[test]
fn merging_is_independent_of_how_work_was_split() {
    // Per-chunk diagnostics merged in any order must give the same report,
    // for the same reason the reductions must: results cannot depend on the
    // thread count.
    let mut whole = Diagnostics::new();
    for id in 0..300 {
        whole.record(link_key(id % 40));
    }

    let mut parts: Vec<Diagnostics> = (0..4).map(|_| Diagnostics::new()).collect();
    for id in 0..300u32 {
        parts[(id % 4) as usize].record(link_key(id % 40));
    }

    let mut merged_forwards = Diagnostics::new();
    for p in &parts {
        merged_forwards.merge(p);
    }
    let mut merged_backwards = Diagnostics::new();
    for p in parts.iter().rev() {
        merged_backwards.merge(p);
    }

    assert_eq!(whole.rows(), merged_forwards.rows());
    assert_eq!(whole.rows(), merged_backwards.rows());
}

#[test]
fn rows_are_sorted_deterministically() {
    let mut diag = Diagnostics::new();
    diag.record(DiagKey::run_level(Category::Numeric, codes::STORE_OVERDRAFT, Severity::Info));
    diag.record(link_key(9));
    diag.record(link_key(1));
    diag.record_run_level(Category::DataQuality, codes::MATCHER_FALLBACK, Severity::Info);

    let rows = diag.rows();
    let mut sorted = rows.clone();
    sorted.sort_unstable_by_key(|r| r.key);
    assert_eq!(rows, sorted);

    // data_quality < modelling < numeric, by category discriminant.
    assert_eq!(rows[0].key.category, Category::DataQuality);
    assert_eq!(rows.last().unwrap().key.category, Category::Numeric);
}

#[test]
fn category_and_severity_names_are_stable() {
    // These strings are the output schema; downstream scripts filter on them.
    assert_eq!(Category::DataQuality.as_str(), "data_quality");
    assert_eq!(Category::Modelling.as_str(), "modelling");
    assert_eq!(Category::Numeric.as_str(), "numeric");
    assert_eq!(Severity::Info.as_str(), "info");
    assert_eq!(Severity::Warning.as_str(), "warning");
}
