//! Identity: the sentinel, the round trip, and deterministic assignment.

use openmobisim_core_types::ids::{
    EntityId, EntityKind, ExternalIdTableBuilder, LinkId, MAX_ID, NULL_ID, NodeId,
};
use proptest::prelude::*;

#[test]
fn null_is_the_sentinel_and_nothing_else() {
    assert!(LinkId::NULL.is_null());
    assert_eq!(LinkId::NULL.raw(), NULL_ID);
    assert!(!LinkId::new(0).is_null());
    assert!(!LinkId::new(MAX_ID).is_null());
    assert_eq!(LinkId::NULL.to_option(), None);
    assert_eq!(LinkId::new(7).to_option(), Some(LinkId::new(7)));
}

#[test]
fn from_raw_round_trips_including_null() {
    for raw in [0u32, 1, 12_345, MAX_ID, NULL_ID] {
        assert_eq!(LinkId::from_raw(raw).raw(), raw);
    }
}

#[test]
fn kinds_carry_their_type() {
    assert_eq!(NodeId::KIND, EntityKind::Node);
    assert_eq!(LinkId::KIND, EntityKind::Link);
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
fn entity_kind_discriminants_are_stable() {
    // The event queue's tie-break depends on these numbers. Changing one
    // changes which of two simultaneous events fires first, and therefore
    // changes results. If this test fails, that is what happened.
    let expected = [
        (EntityKind::Node, 0u8, "node"),
        (EntityKind::Link, 1, "link"),
        (EntityKind::Turn, 2, "turn"),
        (EntityKind::Layer, 3, "layer"),
        (EntityKind::AccessPoint, 4, "access_point"),
        (EntityKind::Hub, 5, "hub"),
        (EntityKind::Resource, 6, "resource"),
        (EntityKind::Zone, 7, "zone"),
        (EntityKind::Path, 8, "path"),
        (EntityKind::Traveller, 9, "traveller"),
        (EntityKind::Trip, 10, "trip"),
        (EntityKind::Vehicle, 11, "vehicle"),
        (EntityKind::TransitRun, 12, "transit_run"),
        (EntityKind::UserClass, 13, "user_class"),
    ];
    assert_eq!(expected.len(), EntityKind::ALL.len());
    for (kind, discriminant, name) in expected {
        assert_eq!(kind as u8, discriminant, "discriminant of {name} changed");
        assert_eq!(kind.as_str(), name, "output name of {name} changed");
        assert_eq!(EntityKind::from_u8(discriminant), Some(kind));
    }
    assert_eq!(EntityKind::from_u8(200), None);
}

#[test]
fn iter_space_is_dense_and_ordered() {
    let ids: Vec<u32> = LinkId::iter_space(5).map(EntityId::raw).collect();
    assert_eq!(ids, [0, 1, 2, 3, 4]);
}

#[test]
fn external_ids_are_assigned_by_sorted_order() {
    let mut b = ExternalIdTableBuilder::new();
    for name in ["way/9", "way/1", "way/33", "way/1"] {
        b.insert(name);
    }
    let table = b.build();

    assert_eq!(table.len(), 3);
    // Bytewise sort, so "way/1" < "way/33" < "way/9".
    assert_eq!(table.iter().collect::<Vec<_>>(), ["way/1", "way/33", "way/9"]);
    assert_eq!(table.id_of("way/33"), Some(1));
    assert_eq!(table.id_of("way/404"), None);
    assert_eq!(table.typed_id_of::<NodeId>("way/9"), Some(NodeId::new(2)));
    assert_eq!(table.external_of(NodeId::NULL), None);
}

#[test]
fn assignment_does_not_depend_on_insertion_order() {
    // The property that makes cached artifacts and seeded runs comparable
    // across builds: the same set of external ids must always produce the same
    // internal ids, whatever order the input arrived in.
    let names = ["c", "a", "d", "b"];

    let mut forwards = ExternalIdTableBuilder::new();
    forwards.extend(names);
    let a = forwards.build();

    let mut backwards = ExternalIdTableBuilder::new();
    backwards.extend(names.iter().rev().copied());
    let b = backwards.build();

    assert_eq!(a.iter().collect::<Vec<_>>(), b.iter().collect::<Vec<_>>());
}

proptest! {
    /// Every id round-trips through the external table, both ways.
    #[test]
    fn external_table_round_trips(mut names in prop::collection::vec("[a-z0-9/_:.-]{1,12}", 0..200)) {
        let mut builder = ExternalIdTableBuilder::new();
        builder.extend(names.clone());
        let table = builder.build();

        names.sort_unstable();
        names.dedup();

        prop_assert_eq!(table.len(), names.len());
        for (expected_id, name) in names.iter().enumerate() {
            let id = table.id_of(name).expect("inserted name must be found");
            prop_assert_eq!(id as usize, expected_id);
            prop_assert_eq!(table.external(id), name.as_str());
        }
    }

    /// `index()` and `from_index()` are inverses over the whole id space.
    #[test]
    fn index_round_trips(raw in 0u32..=MAX_ID) {
        let id = LinkId::new(raw);
        prop_assert_eq!(LinkId::from_index(id.index()), id);
        prop_assert_eq!(id.index(), raw as usize);
    }
}
