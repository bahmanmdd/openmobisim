//! The link-geometry artifact and the road network's own stored length: what
//! is supposed to agree, and the one documented case where it does not.

use std::collections::HashMap;

use openmobisim_core_graph::defaults::{GlobalMultipliers, RoadClass, SignalDefaults};
use openmobisim_core_graph::geometry::LonLat;
use openmobisim_core_graph::link_geometry::LinkGeometry;
use openmobisim_core_graph::network::{LinkSpec, RoadNetworkBuilder};
use openmobisim_core_types::diagnostics::Diagnostics;

#[test]
fn a_degenerate_links_geometry_does_not_have_to_remeasure_to_its_floored_length() {
    // Two nodes at the same point (a duplicate-node mapping error, common in
    // OSM): the network floors the link's length to `MIN_LINK_LENGTH_M`
    // (`network.rs`'s own test, `a_zero_length_link_is_given_a_minimum_and_recorded`),
    // but its real geometry is the two identical, zero-distance points —
    // which cannot remeasure to that floor. `LinkGeometry::build` must not
    // treat that as a bug: it panicked on a real Andorra import at exactly
    // this combination (checkpoint 8b) before this was a documented
    // exception in the code, not only in a comment.
    let mut b = RoadNetworkBuilder::new();
    b.add_node("a", LonLat::new(4.80, 45.70));
    b.add_node("b", LonLat::new(4.80, 45.70));
    b.add_link("degenerate", "a", "b", LinkSpec::new(RoadClass::Residential));

    let mut diagnostics = Diagnostics::new();
    let network = b
        .build(GlobalMultipliers::default(), SignalDefaults::SHIPPED, &mut diagnostics)
        .expect("two coincident nodes still project");

    let points_of: HashMap<String, Vec<LonLat>> = HashMap::from([(
        "degenerate".to_string(),
        vec![LonLat::new(4.80, 45.70), LonLat::new(4.80, 45.70)],
    )]);

    // Must not panic, in debug or release: this is the assertion under test.
    let geometry = LinkGeometry::build(&network, &points_of);
    assert_eq!(geometry.street_count(), 1);
}
