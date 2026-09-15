//! Level 0: free-flow traversal.
//!
//! Fixtures are hand-built with `RoadNetworkBuilder` directly, the same
//! pattern `core-graph`'s own tests use — no path-finder, no `io-osm`, per
//! S133: `core-loading` never routes, so its tests never need to either.

use openmobisim_core_graph::defaults::{GlobalMultipliers, RoadClass, SignalDefaults};
use openmobisim_core_graph::geometry::LonLat;
use openmobisim_core_graph::network::{LinkSpec, RoadNetwork, RoadNetworkBuilder};
use openmobisim_core_loading::{Vehicle, load_level_0, traverse_free_flow};
use openmobisim_core_types::diagnostics::Diagnostics;
use openmobisim_core_types::ids::{EntityId, LinkId, VehicleId};
use openmobisim_core_types::time::Second;
use openmobisim_core_types::units::Pcu;
use proptest::prelude::*;

/// Three nodes in a line, `a -> b -> c`, each leg its own link.
fn line_network() -> RoadNetwork {
    let mut b = RoadNetworkBuilder::new();
    b.add_node("a", LonLat::new(4.800, 45.700));
    b.add_node("b", LonLat::new(4.801, 45.700));
    b.add_node("c", LonLat::new(4.802, 45.700));
    b.add_link("ab", "a", "b", LinkSpec::new(RoadClass::Primary));
    b.add_link("bc", "b", "c", LinkSpec::new(RoadClass::Primary));
    let mut diagnostics = Diagnostics::new();
    b.build(GlobalMultipliers::default(), SignalDefaults::SHIPPED, &mut diagnostics)
        .expect("buildable")
}

fn link(network: &RoadNetwork, external: &str) -> LinkId {
    network.link_external_ids().typed_id_of::<LinkId>(external).expect("known link")
}

#[test]
fn a_single_link_arrives_at_departure_plus_free_flow_time() {
    let network = line_network();
    let ab = link(&network, "ab");
    let vehicle = Vehicle::new(VehicleId::new(0), vec![ab], Pcu(1.0), Second(1000));

    let trajectory = traverse_free_flow(&vehicle, &network);

    assert_eq!(trajectory.links.len(), 1);
    assert_eq!(trajectory.departure(), Second(1000));
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "test assertion")]
    let expected = 1000 + network.free_flow_time(ab).get() as u32;
    assert_eq!(trajectory.arrival(), Second(expected));
}

#[test]
fn a_two_link_route_chains_enter_and_exit_correctly() {
    let network = line_network();
    let (ab, bc) = (link(&network, "ab"), link(&network, "bc"));
    let vehicle = Vehicle::new(VehicleId::new(0), vec![ab, bc], Pcu(1.0), Second(0));

    let trajectory = traverse_free_flow(&vehicle, &network);

    assert_eq!(trajectory.links.len(), 2);
    assert_eq!(trajectory.links[0].enter, Second(0));
    assert_eq!(
        trajectory.links[1].enter, trajectory.links[0].exit,
        "the second link must start exactly where the first one ends"
    );

    // The accumulation must match flooring after each link individually,
    // not flooring the sum once — the same order the implementation uses.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "test assertion")]
    let exit_ab = network.free_flow_time(ab).get() as u32;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "test assertion")]
    let exit_bc = network.free_flow_time(bc).get() as u32;
    assert_eq!(trajectory.links[0].exit, Second(exit_ab));
    assert_eq!(trajectory.arrival(), Second(exit_ab + exit_bc));
    #[allow(
        clippy::float_cmp,
        reason = "both sides are small integer seconds converted to f64 exactly; no \
                  independent derivation, no accumulated rounding to worry about"
    )]
    {
        assert_eq!(
            trajectory.total_travel_time().get(),
            f64::from(exit_ab + exit_bc),
            "total travel time must match arrival minus departure"
        );
    }
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "route is discontinuous")]
fn a_discontinuous_route_is_rejected() {
    // Two links that do not share a node: "ab" ends at b, "ab" again does
    // not start there. A route built this way is a bug in whatever
    // assembled it, not a data condition, so it panics rather than silently
    // producing nonsense.
    let network = line_network();
    let ab = link(&network, "ab");
    let vehicle = Vehicle::new(VehicleId::new(0), vec![ab, ab], Pcu(1.0), Second(0));
    let _ = traverse_free_flow(&vehicle, &network);
}

#[test]
fn vehicles_do_not_interact_at_level_0() {
    // S85/design §10: level 0 has no curves, so two vehicles on the same
    // link at overlapping times must not affect each other's trajectory —
    // each vehicle's result must be identical whether computed alone or
    // alongside another.
    let network = line_network();
    let ab = link(&network, "ab");
    let a = Vehicle::new(VehicleId::new(0), vec![ab], Pcu(1.0), Second(0));
    let b = Vehicle::new(VehicleId::new(1), vec![ab], Pcu(1.0), Second(0));

    let alone_a = traverse_free_flow(&a, &network);
    let alone_b = traverse_free_flow(&b, &network);
    let together = load_level_0([&a, &b], &network);

    assert_eq!(together.len(), 2);
    assert_eq!(together[0].links, alone_a.links);
    assert_eq!(together[1].links, alone_b.links);
}

proptest! {
    /// S85: "FIFO holds on each link by construction." At level 0 that
    /// degenerates to plain monotonicity — a later departure never arrives
    /// before an earlier one on the same route — but it is exactly the
    /// property levels 1-4 must also prove, so it is worth asserting here
    /// even though level 0 gets it for free.
    #[test]
    fn later_departure_never_arrives_before_earlier_departure(
        first in 0u32..100_000,
        gap in 0u32..100_000,
    ) {
        let network = line_network();
        let ab = link(&network, "ab");
        let second = first + gap;

        let early = traverse_free_flow(
            &Vehicle::new(VehicleId::new(0), vec![ab], Pcu(1.0), Second(first)),
            &network,
        );
        let late = traverse_free_flow(
            &Vehicle::new(VehicleId::new(1), vec![ab], Pcu(1.0), Second(second)),
            &network,
        );

        prop_assert!(late.arrival() >= early.arrival());
    }
}
