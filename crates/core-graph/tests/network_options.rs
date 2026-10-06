//! Every network parameter by name (S225, roadmap I-ae): what the names are, that each one acts,
//! that bad ones are refused, and that the network keeps what it was built with.

use std::collections::BTreeMap;

use openmobisim_core_graph::defaults::{RoadClass, SignalDefaults, default_row};
use openmobisim_core_graph::geometry::LonLat;
use openmobisim_core_graph::layers::StaticLayerDefaults;
use openmobisim_core_graph::network::{LinkSpec, RoadNetwork, RoadNetworkBuilder};
use openmobisim_core_graph::network_options::NetworkDefaults;
use openmobisim_core_graph::turns::TurnTable;
use openmobisim_core_types::diagnostics::Diagnostics;
use openmobisim_core_types::ids::{EntityId, LinkId};

fn options(pairs: &[(&str, f64)]) -> BTreeMap<String, f64> {
    pairs.iter().map(|&(k, v)| (k.to_string(), v)).collect()
}

/// Three nodes in a row, the middle one signalised, residential links with nothing stated.
fn line(defaults: &NetworkDefaults) -> RoadNetwork {
    let mut b = RoadNetworkBuilder::new();
    for (i, name) in ["a", "b", "c"].into_iter().enumerate() {
        #[allow(clippy::cast_precision_loss, reason = "a small index")]
        b.add_node(name, LonLat { lon: 4.9 + 0.005 * i as f64, lat: 52.37 });
    }
    b.mark_signalised("b");
    for (id, from, to) in [("ab", "a", "b"), ("bc", "b", "c"), ("ba", "b", "a"), ("cb", "c", "b")] {
        b.add_link(id, from, to, LinkSpec::new(RoadClass::Residential));
    }
    b.build_with(defaults, &mut Diagnostics::new()).expect("a valid network")
}

#[test]
fn every_parameter_has_a_name_and_its_shipped_value() {
    let names = NetworkDefaults::names();
    assert_eq!(names.len(), 15 + 4 * RoadClass::ALL.len());
    assert!(names.windows(2).all(|w| w[0] < w[1]), "sorted, no repeats");
    let values: BTreeMap<String, f64> = NetworkDefaults::shipped().values().into_iter().collect();
    assert_eq!(values.len(), names.len());
    let row = default_row(RoadClass::Primary);
    let same = |a: f64, b: f64| a.to_bits() == b.to_bits();
    assert!(same(values["primary.free_flow_km_h"], row.free_flow_km_h));
    assert!(same(values["primary.lanes"], f64::from(row.lanes_per_direction)));
    assert!(same(values["signal_cycle_s"], SignalDefaults::SHIPPED.cycle_seconds));
    assert!(same(values["walk_km_h"], StaticLayerDefaults::SHIPPED.walk_km_h));
    assert!(same(values["capacity_factor"], 1.0));
    assert_eq!(NetworkDefaults::shipped().descriptor(), "", "nothing differs from itself");
}

#[test]
fn each_kind_of_parameter_acts_on_the_network() {
    let shipped = line(&NetworkDefaults::shipped());
    let link = LinkId::new(0);
    let speed = |n: &RoadNetwork| n.link_parameters(link).free_flow_speed.get() * 3.6;
    let capacity = |n: &RoadNetwork| n.link_parameters(link).capacity.get() * 3600.0;
    assert!((speed(&shipped) - default_row(RoadClass::Residential).free_flow_km_h).abs() < 1e-9);

    let slower = NetworkDefaults::from_options(&options(&[("residential.free_flow_km_h", 20.0)]))
        .expect("a known name");
    assert!((speed(&line(&slower)) - 20.0).abs() < 1e-9, "the class row");
    assert_eq!(slower.descriptor(), "residential.free_flow_km_h=20");

    let half = NetworkDefaults::from_options(&options(&[("capacity_factor", 0.5)])).expect("known");
    assert!((capacity(&line(&half)) - 0.5 * capacity(&shipped)).abs() < 1e-6, "a multiplier");

    let walking = NetworkDefaults::from_options(&options(&[("walk_km_h", 4.0)])).expect("known");
    assert!(
        (line(&walking).defaults().layers.walk_km_h - 4.0).abs() < 1e-12,
        "kept by the network"
    );
}

#[test]
fn the_green_fraction_factor_reaches_turn_capacity_at_signals() {
    // S225: the factor existed but nothing read it.
    let shipped = line(&NetworkDefaults::shipped());
    let less_green = line(
        &NetworkDefaults::from_options(&options(&[("green_fraction_factor", 0.5)])).expect("known"),
    );
    assert!(
        (less_green.signals().green_fraction - 0.5 * shipped.signals().green_fraction).abs()
            < 1e-12
    );
    let a = TurnTable::build(&shipped, shipped.signals());
    let b = TurnTable::build(&less_green, less_green.signals());
    assert_eq!(a.capacity_fractions().len(), b.capacity_fractions().len());
    let signalled: Vec<(f32, f32)> = a
        .capacity_fractions()
        .iter()
        .zip(b.capacity_fractions())
        .filter(|(x, _)| **x < 1.0)
        .map(|(x, y)| (*x, *y))
        .collect();
    assert!(!signalled.is_empty(), "turns at the signal");
    for (x, y) in signalled {
        assert!((y - 0.5 * x).abs() < 1e-6, "{x} {y}");
    }
    // And the signal's delay on the approach grows with less green.
    let delay = |n: &RoadNetwork| n.link_parameters(LinkId::new(0)).control_delay.get();
    assert!(delay(&less_green) > delay(&shipped));
}

#[test]
fn unknown_names_and_bad_values_are_refused_with_the_reason() {
    let err = |pairs: &[(&str, f64)]| NetworkDefaults::from_options(&options(pairs)).unwrap_err();
    let unknown = err(&[("residential.speed", 30.0)]);
    assert!(
        unknown.contains("no network option called")
            && unknown.contains("residential.free_flow_km_h")
    );
    assert!(err(&[("nowhere.lanes", 2.0)]).contains("no network option called"));
    assert!(err(&[("capacity_factor", 0.0)]).contains("positive"));
    assert!(err(&[("walk_km_h", f64::NAN)]).contains("positive"));
    assert!(err(&[("primary.lanes", 2.5)]).contains("whole number from 1 to 20"));
    assert!(err(&[("primary.lanes", 21.0)]).contains("whole number from 1 to 20"));
    assert!(err(&[("signal_green_fraction", 1.2)]).contains("from 0 to 1"));
}

#[test]
fn the_plain_build_is_the_shipped_defaults() {
    let mut b = RoadNetworkBuilder::new();
    b.add_node("a", LonLat { lon: 4.9, lat: 52.37 });
    b.add_node("b", LonLat { lon: 4.905, lat: 52.37 });
    b.add_link("ab", "a", "b", LinkSpec::new(RoadClass::Primary));
    let plain = b
        .build(Default::default(), SignalDefaults::SHIPPED, &mut Diagnostics::new())
        .expect("valid");
    assert_eq!(*plain.defaults(), NetworkDefaults::shipped());
}
