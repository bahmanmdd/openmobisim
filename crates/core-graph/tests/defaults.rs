//! The defaults table: the numbers, and the arithmetic that turns them into a
//! fundamental diagram.
//!
//! Two of these tests are about the *values* rather than the code. That is
//! deliberate: the values are a modelling commitment, and a silent edit to one
//! of them changes every result any user ever produces. If one fails, the
//! question is whether the change was intended and whether `DEFAULTS_VERSION`
//! was bumped with it — not how to make the test green.

#![allow(clippy::float_cmp, reason = "the shipped values are exact by definition")]

use openmobisim_core_graph::defaults::{
    DEFAULTS_VERSION, GlobalMultipliers, LinkParameters, ParameterNote, RoadClass, SignalDefaults,
    WAVE_SPEED_BOUNDS_KM_H, default_row,
};
use proptest::prelude::*;

fn derive(class: RoadClass, signalised: bool) -> (LinkParameters, ParameterNote) {
    let row = default_row(class);
    LinkParameters::from_defaults(
        row,
        row.lanes_per_direction,
        signalised,
        SignalDefaults::SHIPPED,
        GlobalMultipliers::default(),
        None,
    )
}

#[test]
fn every_shipped_row_yields_a_consistent_triangular_diagram() {
    // The identity q = v·w·k_j/(v+w) is what makes the diagram a diagram. If it
    // does not hold, the loading's sending and receiving flows disagree about
    // where capacity is, and the model is quietly wrong everywhere.
    for class in RoadClass::ALL {
        let (p, note) = derive(class, false);
        assert_eq!(
            note,
            ParameterNote::Consistent,
            "{} needed an adjustment: {note:?}",
            class.as_str()
        );
        let error = p.consistency_error();
        assert!(error < 1e-12, "{}: q = v·w·k_j/(v+w) is off by {error:e}", class.as_str());
    }
}

#[test]
fn no_shipped_row_clamps_its_wave_speed() {
    // The clamp exists to catch a nonsensical override, not to reshape the
    // shipped defaults. If a row starts clamping, the row is wrong, not the
    // clamp.
    for class in RoadClass::ALL {
        let (p, _) = derive(class, false);
        let w = p.wave_speed.as_km_per_hour();
        let (lo, hi) = WAVE_SPEED_BOUNDS_KM_H;
        assert!(
            w > lo && w < hi,
            "{}: derived wave speed {w:.1} km/h hits a bound",
            class.as_str()
        );
    }
}

#[test]
fn derived_wave_speeds_land_where_the_literature_says_they_should() {
    // 15–20 km/h is the range design §12.1 quotes for urban roads. This checks
    // the *motor traffic* classes; service roads and living streets sit below
    // it, which is expected at 20 km/h free-flow and recorded here rather than
    // discovered later.
    let urban = [
        RoadClass::Motorway,
        RoadClass::Trunk,
        RoadClass::Primary,
        RoadClass::Secondary,
        RoadClass::Tertiary,
        RoadClass::Unclassified,
        RoadClass::Residential,
    ];
    for class in urban {
        let (p, _) = derive(class, false);
        let w = p.wave_speed.as_km_per_hour();
        assert!(
            (14.0..=21.0).contains(&w),
            "{}: wave speed {w:.1} km/h is outside the expected 15–20 band",
            class.as_str()
        );
    }
}

#[test]
fn the_shipped_numbers_are_what_they_say_they_are() {
    // A change here is a modelling change. Bump DEFAULTS_VERSION with it.
    assert_eq!(DEFAULTS_VERSION, 1);

    let motorway = default_row(RoadClass::Motorway);
    assert_eq!(motorway.free_flow_km_h, 110.0);
    assert_eq!(motorway.lanes_per_direction, 2);
    assert_eq!(motorway.saturation_flow_veh_h_lane, 2000.0);

    let residential = default_row(RoadClass::Residential);
    assert_eq!(residential.free_flow_km_h, 30.0);
    assert_eq!(residential.saturation_flow_veh_h_lane, 1400.0);

    assert_eq!(SignalDefaults::SHIPPED.cycle_seconds, 90.0);
    assert_eq!(SignalDefaults::SHIPPED.green_fraction, 0.48);
}

#[test]
fn websters_delay_lands_in_the_documented_band() {
    // Design §12.2 promises "≈ 12 s at low flow and ≈ 23 s near saturation"
    // with a 90-second cycle at g/C = 0.48.
    let at_zero = SignalDefaults { nominal_degree_of_saturation: 0.0, ..SignalDefaults::SHIPPED };
    let at_saturation =
        SignalDefaults { nominal_degree_of_saturation: 0.95, ..SignalDefaults::SHIPPED };

    let low = at_zero.uniform_delay().get();
    let high = at_saturation.uniform_delay().get();
    assert!((11.5..12.5).contains(&low), "delay at zero flow was {low:.1} s");
    assert!((22.0..24.0).contains(&high), "delay near saturation was {high:.1} s");

    let shipped = SignalDefaults::SHIPPED.uniform_delay().get();
    assert!((19.0..22.0).contains(&shipped), "shipped delay was {shipped:.1} s");
    assert!(low < shipped && shipped < high, "delay must rise with saturation");
}

#[test]
fn control_delay_applies_only_at_signals() {
    let (plain, _) = derive(RoadClass::Primary, false);
    let (signalised, _) = derive(RoadClass::Primary, true);
    assert_eq!(plain.control_delay.get(), 0.0, "an unsignalised approach has no control delay");
    assert!(signalised.control_delay.get() > 10.0);

    // And it lands in the traversal time, which is the whole point: without it
    // an undersaturated signalised approach is free, and urban travel times
    // come out low network-wide.
    let length = openmobisim_core_types::units::Metres(300.0);
    let extra = signalised.free_flow_time(length).get() - plain.free_flow_time(length).get();
    assert!((extra - signalised.control_delay.get()).abs() < 1e-9);
}

#[test]
fn units_are_converted_exactly_once() {
    let (p, _) = derive(RoadClass::Secondary, false);
    // Internal units are SI and per-second.
    assert!((p.capacity.as_veh_per_hour() - 1800.0).abs() < 1e-9);
    assert!((p.capacity.get() - 0.5).abs() < 1e-9, "1800 veh/h is exactly 0.5 veh/s");
    assert!((p.free_flow_speed.as_km_per_hour() - 50.0).abs() < 1e-9);
    assert!((p.jam_density.as_veh_per_km() - 130.0).abs() < 1e-9);
}

#[test]
fn multipliers_move_what_they_say_they_move() {
    let row = default_row(RoadClass::Primary);
    let base = LinkParameters::from_defaults(
        row,
        2,
        true,
        SignalDefaults::SHIPPED,
        GlobalMultipliers::default(),
        None,
    )
    .0;
    let scaled = LinkParameters::from_defaults(
        row,
        2,
        true,
        SignalDefaults::SHIPPED,
        GlobalMultipliers { capacity: 0.9, ..GlobalMultipliers::default() },
        None,
    )
    .0;
    assert!((scaled.capacity.get() / base.capacity.get() - 0.9).abs() < 1e-12);
    // Free-flow speed and jam density are untouched by a capacity multiplier.
    assert_eq!(scaled.free_flow_speed, base.free_flow_speed);
    assert_eq!(scaled.jam_density, base.jam_density);
    // But the derived wave speed moves, because it is derived.
    assert!(scaled.wave_speed != base.wave_speed);
}

#[test]
fn an_osm_maxspeed_overrides_the_row() {
    let row = default_row(RoadClass::Residential);
    let (p, _) = LinkParameters::from_defaults(
        row,
        1,
        false,
        SignalDefaults::SHIPPED,
        GlobalMultipliers::default(),
        Some(50.0),
    );
    assert!((p.free_flow_speed.as_km_per_hour() - 50.0).abs() < 1e-9);
}

#[test]
fn impossible_parameters_are_resolved_and_recorded_never_raised() {
    // A user who sets capacity absurdly high relative to jam density asks for a
    // diagram that does not exist. The run must still complete.
    let row = default_row(RoadClass::Residential);
    let (p, note) = LinkParameters::from_defaults(
        row,
        1,
        false,
        SignalDefaults::SHIPPED,
        GlobalMultipliers { capacity: 20.0, ..GlobalMultipliers::default() },
        None,
    );
    assert_eq!(note, ParameterNote::CapacityReducedToFitJamDensity);
    assert!(p.capacity.get() > 0.0 && p.capacity.get().is_finite());
    assert!(p.wave_speed.get() > 0.0 && p.wave_speed.get().is_finite());
    assert!(p.jam_density.get() > 0.0 && p.jam_density.get().is_finite());
}

#[test]
fn osm_tags_classify_or_say_they_do_not() {
    assert_eq!(RoadClass::from_osm_highway("residential"), Some(RoadClass::Residential));
    assert_eq!(RoadClass::from_osm_highway("motorway_link"), Some(RoadClass::MotorwayLink));
    assert_eq!(RoadClass::from_osm_highway("steps"), Some(RoadClass::Footway));
    // An unmodelled tag returns None so that the fallback, and its diagnostic,
    // live in one place rather than being guessed at here.
    assert_eq!(RoadClass::from_osm_highway("proposed"), None);
    assert_eq!(RoadClass::from_osm_highway(""), None);

    // Round trip for everything that has a canonical tag.
    for class in RoadClass::ALL {
        if matches!(class, RoadClass::Footway) {
            continue; // several tags collapse into it
        }
        assert_eq!(
            RoadClass::from_osm_highway(class.as_str()),
            Some(class),
            "{} did not round-trip",
            class.as_str()
        );
    }
}

#[test]
fn access_defaults_are_the_ones_a_modeller_expects() {
    assert!(!RoadClass::Motorway.carries_pedestrians());
    assert!(!RoadClass::Motorway.carries_cyclists());
    assert!(RoadClass::Residential.carries_pedestrians());
    assert!(RoadClass::Residential.carries_cyclists());
    assert!(!RoadClass::Footway.carries_motor_traffic());
    assert!(RoadClass::Footway.carries_pedestrians());
    assert!(!RoadClass::Cycleway.carries_pedestrians());
    assert!(RoadClass::Cycleway.carries_cyclists());
}

proptest! {
    /// Whatever a user does with the multipliers, the result is a usable
    /// diagram: finite, positive, and either consistent or recorded as adjusted.
    ///
    /// This is the "simulation never stops" obligation, tested rather than
    /// asserted — there is no combination of the exposed knobs that produces a
    /// NaN, a negative capacity or a diagram the loading cannot evaluate.
    #[test]
    fn any_multiplier_combination_gives_a_usable_diagram(
        class_index in 0usize..RoadClass::ALL.len(),
        lanes in 1u8..=8,
        speed_mult in 0.2f64..3.0,
        capacity_mult in 0.1f64..10.0,
        jam_mult in 0.3f64..3.0,
        signalised in any::<bool>(),
        maxspeed in prop::option::of(5.0f64..140.0),
    ) {
        let class = RoadClass::ALL[class_index];
        let (p, note) = LinkParameters::from_defaults(
            default_row(class),
            lanes,
            signalised,
            SignalDefaults::SHIPPED,
            GlobalMultipliers {
                free_flow_speed: speed_mult,
                capacity: capacity_mult,
                jam_density: jam_mult,
                control_delay: 1.0,
                green_fraction: 1.0,
            },
            maxspeed,
        );

        prop_assert!(p.free_flow_speed.get() > 0.0 && p.free_flow_speed.is_finite());
        prop_assert!(p.capacity.get() > 0.0 && p.capacity.is_finite());
        prop_assert!(p.jam_density.get() > 0.0 && p.jam_density.is_finite());
        prop_assert!(p.wave_speed.get() > 0.0 && p.wave_speed.is_finite());
        prop_assert!(p.control_delay.get() >= 0.0 && p.control_delay.is_finite());

        // The critical density must sit strictly inside the diagram.
        prop_assert!(
            p.critical_density().get() < p.jam_density.get(),
            "critical density {:?} is not below jam density {:?}",
            p.critical_density(), p.jam_density
        );

        // And whatever adjustment was made, the diagram closes.
        prop_assert!(
            p.consistency_error() < 1e-9,
            "note {:?} left the diagram inconsistent by {:e}", note, p.consistency_error()
        );
    }
}
