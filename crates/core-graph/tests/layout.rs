//! Sizes, for the same reason as in `core-types`: these types sit in arrays
//! with one entry per node, per link or per turn, and a type that quietly
//! doubles doubles a structure a city-scale network holds millions of.

use std::mem::size_of;

use openmobisim_core_graph::defaults::{DefaultRow, LinkParameters, RoadClass, SignalDefaults};
use openmobisim_core_graph::geometry::{Hemisphere, LonLat, Projected, Projection, UtmZone};

#[test]
fn coordinates_are_two_f64s_and_nothing_else() {
    assert_eq!(size_of::<LonLat>(), 16);
    assert_eq!(size_of::<Projected>(), 16);
}

#[test]
fn a_road_class_is_one_byte() {
    assert_eq!(size_of::<RoadClass>(), 1, "one per link");
}

#[test]
fn link_parameters_are_five_f64s() {
    // free-flow speed, capacity, jam density, wave speed, control delay.
    assert_eq!(size_of::<LinkParameters>(), 40);
    assert_eq!(size_of::<DefaultRow>(), 32);
}

#[test]
fn the_projection_is_cheap_to_copy() {
    // Passed by value into per-node loops; it must stay small enough that the
    // compiler keeps it in registers rather than spilling it.
    assert!(size_of::<Projection>() <= 160, "Projection grew to {} bytes", size_of::<Projection>());
    assert_eq!(size_of::<UtmZone>(), 2);
    assert_eq!(size_of::<Hemisphere>(), 1);
    assert_eq!(size_of::<SignalDefaults>(), 24);
}
