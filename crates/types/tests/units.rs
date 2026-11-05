//! Units against hand conversions. Every expected value below is exactly representable in
//! binary floating point and is reached by one exact operation, so equality is exact.

#![allow(clippy::float_cmp, reason = "each value is exact by the hand derivation beside it")]

use openmobisim_types::units::{Cost, Duration, Flow, Metres, Speed};

#[test]
fn thirty_six_km_per_hour_is_ten_metres_per_second() {
    // 36 km/h = 36 000 m / 3 600 s = 10 m/s.
    assert_eq!(Speed::from_km_per_hour(36.0), Speed(10.0));
    // A bike at 18 km/h: 18 000 m / 3 600 s = 5 m/s.
    assert_eq!(Speed::from_km_per_hour(18.0), Speed(5.0));
}

#[test]
fn eighteen_hundred_vehicles_per_hour_is_half_a_vehicle_per_second() {
    assert_eq!(Flow::from_veh_per_hour(1_800.0), Flow(0.5));
    assert_eq!(Flow(0.5).as_veh_per_hour(), 1_800.0);
}

#[test]
fn a_bike_rides_one_hundred_metres_at_five_metres_per_second_in_twenty_seconds() {
    // Free-flow time is length over speed: 100 m / 5 m/s = 20 s.
    assert_eq!(Metres(100.0) / Speed(5.0), Duration(20.0));
    // And back: 5 m/s × 20 s = 100 m, 100 m / 20 s = 5 m/s.
    assert_eq!(Speed(5.0) * Duration(20.0), Metres(100.0));
    assert_eq!(Duration(20.0) * Speed(5.0), Metres(100.0));
    assert_eq!(Metres(100.0) / Duration(20.0), Speed(5.0));
}

#[test]
fn a_flow_over_a_duration_counts_vehicles() {
    // 1 800 veh/h for ten minutes: 0.5 veh/s × 600 s = 300 vehicles.
    assert_eq!(Flow::from_veh_per_hour(1_800.0) * Duration(600.0), 300.0);
    assert_eq!(Duration(600.0) * Flow(0.5), 300.0);
}

#[test]
fn like_quantities_add_subtract_scale_and_compare() {
    assert_eq!(Metres(250.0) + Metres(750.0), Metres(1_000.0));
    assert_eq!(Duration(30.0) - Duration(50.0), Duration(-20.0));
    assert_eq!(-Cost(4.0), Cost(-4.0));
    assert_eq!(Metres(100.0) * 2.5, Metres(250.0));
    assert_eq!(2.5 * Metres(100.0), Metres(250.0));
    assert_eq!(Metres(100.0) / 4.0, Metres(25.0));
    assert_eq!(Metres(300.0) / Metres(1_200.0), 0.25);
    assert_eq!(Speed(5.0).max(Speed(13.75)), Speed(13.75));
    assert_eq!(Speed(5.0).min(Speed(13.75)), Speed(5.0));
    let mut d = Duration(10.0);
    d += Duration(5.0);
    d -= Duration(2.0);
    assert_eq!(d, Duration(13.0));
    // 1 + 2 + … + 100 = 5 050.
    let lengths: Vec<Metres> = (1..=100).map(|i| Metres(f64::from(i))).collect();
    assert_eq!(lengths.iter().sum::<Metres>(), Metres(5_050.0));
    assert_eq!(lengths.into_iter().sum::<Metres>(), Metres(5_050.0));
}

#[test]
fn infinity_means_no_limit() {
    // An unlimited capacity passes any finite count; a zero speed gives an infinite time.
    assert!(!Flow::INFINITE.is_finite());
    assert!(Flow::INFINITE * Duration(1.0) > 1e300);
    assert_eq!(Metres(100.0) / Speed::ZERO, Duration::INFINITE);
}

#[test]
fn values_print_with_their_unit() {
    assert_eq!(Speed(10.0).to_string(), "10 m/s");
    assert_eq!(Flow(0.5).to_string(), "0.5 veh/s");
    assert_eq!(format!("{:?}", Metres(20.0)), "Metres(20 m)");
}

#[test]
fn units_cost_nothing_in_memory() {
    assert_eq!(size_of::<Metres>(), size_of::<f64>());
    assert_eq!(size_of::<Flow>(), size_of::<f64>());
}
