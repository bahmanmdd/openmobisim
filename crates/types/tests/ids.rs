//! Identifiers: the sentinel, the round trips, and assignment by sorted external id.

mod common;

use common::SplitMix64;
use openmobisim_types::ids::{
    EntityId, EntityKind, ExternalIdTableBuilder, LinkId, MAX_ID, NULL_ID, NodeId, StopId, TripId,
};

#[test]
fn null_is_the_sentinel_and_nothing_else() {
    assert!(LinkId::NULL.is_null());
    assert_eq!(LinkId::NULL.raw(), u32::MAX);
    assert!(!LinkId::new(0).is_null());
    assert!(!LinkId::new(MAX_ID).is_null());
    assert_eq!(LinkId::NULL.to_option(), None);
    assert_eq!(LinkId::new(7).to_option(), Some(LinkId::new(7)));
    assert_eq!(LinkId::NULL.to_string(), "null");
}

#[test]
fn an_identifier_is_four_bytes() {
    // The reason for the sentinel: a u32, where Option<u32> would take eight.
    assert_eq!(size_of::<LinkId>(), 4);
    assert_eq!(size_of::<Option<u32>>(), 8);
}

#[test]
fn raw_and_index_round_trip() {
    for raw in [0u32, 1, 12_345, MAX_ID, NULL_ID] {
        assert_eq!(LinkId::from_raw(raw).raw(), raw);
    }
    let mut g = SplitMix64(1);
    for raw in (0..10_000).map(|_| u32::try_from(g.below(u64::from(MAX_ID) + 1)).unwrap()) {
        let id = TripId::new(raw);
        assert_eq!(id.index(), raw as usize);
        assert_eq!(TripId::from_index(id.index()), id);
    }
}

#[test]
#[should_panic(expected = "null sentinel")]
#[cfg(debug_assertions)]
fn constructing_from_the_sentinel_is_a_bug() {
    let _ = LinkId::new(NULL_ID);
}

#[test]
#[should_panic(expected = "null")]
#[cfg(debug_assertions)]
fn indexing_with_null_is_a_bug() {
    let _ = LinkId::NULL.index();
}

#[test]
fn kinds_are_nine_in_their_documented_order() {
    let expected = [
        (EntityKind::Node, "node"),
        (EntityKind::Link, "link"),
        (EntityKind::Zone, "zone"),
        (EntityKind::Path, "path"),
        (EntityKind::Trip, "trip"),
        (EntityKind::Vehicle, "vehicle"),
        (EntityKind::TransitRun, "transit_run"),
        (EntityKind::UserClass, "user_class"),
        (EntityKind::Stop, "stop"),
    ];
    assert_eq!(EntityKind::ALL.len(), expected.len());
    for (i, (kind, name)) in expected.into_iter().enumerate() {
        assert_eq!(EntityKind::ALL[i], kind);
        assert_eq!(kind as usize, i);
        assert_eq!(kind.to_string(), name);
    }
    assert_eq!(NodeId::KIND, EntityKind::Node);
    assert_eq!(TripId::KIND, EntityKind::Trip);
}

#[test]
fn a_stop_has_its_own_identifier_space_after_every_other_kind() {
    // Last in the event order, so adding it moved no simultaneous event of another kind.
    assert_eq!(StopId::KIND, EntityKind::Stop);
    assert_eq!(EntityKind::Stop as usize, EntityKind::ALL.len() - 1);
    assert!(EntityKind::ALL.iter().all(|&k| k <= EntityKind::Stop));
    assert_eq!(StopId::new(4).to_string(), "4");
    assert_eq!(format!("{:?}", StopId::NULL), "StopId(null)");
    assert_eq!(size_of::<StopId>(), 4);
}

#[test]
fn iter_space_is_dense_and_ordered() {
    let ids: Vec<u32> = LinkId::iter_space(5).map(EntityId::raw).collect();
    assert_eq!(ids, [0, 1, 2, 3, 4]);
}

#[test]
fn external_ids_are_assigned_by_bytewise_sorted_order() {
    let mut b = ExternalIdTableBuilder::new();
    b.extend(["way/9", "way/1", "way/33", "way/1"]);
    let table = b.build();
    // By hand: bytewise, "way/1" < "way/33" < "way/9"; the duplicate collapses.
    assert_eq!(table.iter().collect::<Vec<_>>(), ["way/1", "way/33", "way/9"]);
    assert_eq!(table.id_of("way/33"), Some(1));
    assert_eq!(table.id_of("way/404"), None);
    assert_eq!(table.typed_id_of::<NodeId>("way/9"), Some(NodeId::new(2)));
    assert_eq!(table.external_of(NodeId::NULL), None);
    assert_eq!(table.external_of(NodeId::new(3)), None);
}

fn random_name(g: &mut SplitMix64) -> String {
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789/_:.-";
    let len = 1 + g.index(12);
    (0..len).map(|_| char::from(ALPHABET[g.index(ALPHABET.len())])).collect()
}

#[test]
fn the_table_matches_the_sorted_deduplicated_input_both_ways() {
    // The external answer is the standard library's sort and dedup.
    let mut g = SplitMix64(2);
    for _ in 0..50 {
        let n = g.index(300);
        let mut names: Vec<String> = (0..n).map(|_| random_name(&mut g)).collect();
        let mut b = ExternalIdTableBuilder::new();
        b.extend(names.clone());
        let table = b.build();
        names.sort_unstable();
        names.dedup();
        assert_eq!(table.len(), names.len());
        for (expected, name) in names.iter().enumerate() {
            let id = table.id_of(name).expect("an inserted name is found");
            assert_eq!(id as usize, expected);
            assert_eq!(table.external(id), name);
        }
    }
}

#[test]
fn assignment_does_not_depend_on_insertion_order() {
    let mut g = SplitMix64(3);
    let names: Vec<String> = (0..500).map(|_| random_name(&mut g)).collect();
    let mut b = ExternalIdTableBuilder::new();
    b.extend(names.clone());
    let reference: Vec<String> = b.build().iter().map(str::to_owned).collect();
    for _ in 0..20 {
        // A Fisher–Yates shuffle of the input.
        let mut shuffled = names.clone();
        for i in (1..shuffled.len()).rev() {
            shuffled.swap(i, g.index(i + 1));
        }
        let mut b = ExternalIdTableBuilder::new();
        b.extend(shuffled);
        assert!(b.build().iter().eq(reference.iter().map(String::as_str)));
    }
}
