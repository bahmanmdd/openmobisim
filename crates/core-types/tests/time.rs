//! The clock, the step grid and the event ordering.

// Exact float comparison is the assertion here, not an oversight: bit-identical
// results are precisely the property under test.
#![allow(clippy::float_cmp, reason = "bit-exact results are the property under test")]

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use openmobisim_core_types::ids::EntityKind;
use openmobisim_core_types::time::{
    EventKey, FastDivU32, Second, StepGrid, StepGridError, StepIndex,
};
use proptest::prelude::*;

#[test]
fn grid_rejects_a_partial_trailing_step() {
    // A silently shortened or lengthened window would change every rate in the
    // run, so this is a scenario error, not something to round.
    let err = StepGrid::new(Second::ZERO, 300, 3_500).unwrap_err();
    assert!(matches!(err, StepGridError::WindowNotWholeSteps { .. }), "{err:?}");
    assert!(err.to_string().contains("whole number"));
}

#[test]
fn grid_rejects_a_zero_step() {
    assert!(matches!(StepGrid::new(Second::ZERO, 0, 3_600).unwrap_err(), StepGridError::ZeroStep));
}

#[test]
fn grid_rejects_a_window_past_the_end_of_the_clock() {
    let err = StepGrid::new(Second(u32::MAX - 10), 1, 100).unwrap_err();
    assert!(matches!(err, StepGridError::WindowOverflowsClock { .. }), "{err:?}");
}

#[test]
fn steps_are_half_open_and_contiguous() {
    let grid = StepGrid::new(Second::from_hours(7), 300, 3_600).unwrap();
    assert_eq!(grid.n_steps(), 12);

    for i in grid.steps() {
        let start = grid.step_start(i);
        let end = grid.step_end(i);
        assert_eq!(grid.step_of(start), i, "a step must contain its own start");
        assert_eq!(end.get() - start.get(), 300);
        if i.get() + 1 < grid.n_steps() {
            assert_eq!(
                grid.step_of(end),
                StepIndex(i.get() + 1),
                "the end belongs to the next step"
            );
        }
    }
    assert_eq!(grid.end(), Second::from_hours(8));
}

#[test]
fn out_of_window_instants_clamp_rather_than_panic() {
    // The simulation never stops: a traveller still moving at the end of the
    // window is counted by the completion statistics, not by a crash here.
    let grid = StepGrid::new(Second::from_hours(7), 300, 3_600).unwrap();
    assert_eq!(grid.step_of(Second::ZERO), StepIndex(0));
    assert_eq!(grid.step_of(Second::MAX), StepIndex(11));
    assert_eq!(grid.try_step_of(Second::ZERO), None);
    assert_eq!(grid.try_step_of(Second::MAX), None);
    assert_eq!(grid.try_step_of(Second::from_hours(7)), Some(StepIndex(0)));
}

#[test]
fn within_step_times_are_floored_to_whole_seconds() {
    let grid = StepGrid::new(Second::ZERO, 300, 3_600).unwrap();
    let i = StepIndex(2); // starts at second 600
    assert_eq!(grid.second_within_step(i, 0.0), Second(600));
    assert_eq!(grid.second_within_step(i, 0.5), Second(750));
    // 0.999 of 300 s is 299.7 s, floored to 299.
    assert_eq!(grid.second_within_step(i, 0.999), Second(899));
    // A fraction of exactly 1.0 must not leak into the next step.
    assert_eq!(grid.second_within_step(i, 1.0), Second(899));
    // Out-of-range input is clamped, not wrapped.
    assert_eq!(grid.second_within_step(i, -3.0), Second(600));
    assert_eq!(grid.second_within_step(i, 7.5), Second(899));
}

#[test]
fn events_order_by_second_then_kind_then_id() {
    let mut q = BinaryHeap::new();
    for key in [
        EventKey::new(Second(10), EntityKind::Vehicle, 1),
        EventKey::new(Second(10), EntityKind::Trip, 9),
        EventKey::new(Second(10), EntityKind::Trip, 2),
        EventKey::new(Second(5), EntityKind::UserClass, 0),
    ] {
        q.push(Reverse(key));
    }

    let popped: Vec<EventKey> = std::iter::from_fn(|| q.pop().map(|r| r.0)).collect();
    assert_eq!(
        popped,
        [
            EventKey::new(Second(5), EntityKind::UserClass, 0),
            EventKey::new(Second(10), EntityKind::Trip, 2),
            EventKey::new(Second(10), EntityKind::Trip, 9),
            EventKey::new(Second(10), EntityKind::Vehicle, 1),
        ]
    );
}

#[test]
fn second_displays_as_a_clock_time_past_midnight() {
    assert_eq!(Second::from_hours(7).to_string(), "07:00:00");
    assert_eq!(Second(93_784).to_string(), "26:03:04");
}

proptest! {
    /// The fast reciprocal agrees with hardware division everywhere.
    ///
    /// This is the only place in the core where a division is replaced by
    /// something cleverer, so it gets the strongest test in the crate.
    #[test]
    fn fast_division_matches_hardware(n in any::<u32>(), d in 1u32..=u32::MAX) {
        let fast = FastDivU32::new(d);
        prop_assert_eq!(fast.div(n), n / d, "div({}, {})", n, d);
        prop_assert_eq!(fast.rem(n), n % d, "rem({}, {})", n, d);
        prop_assert_eq!(fast.div_rem(n), (n / d, n % d));
    }

    /// Including the divisors that actually occur, across the whole clock.
    #[test]
    fn fast_division_matches_for_plausible_steps(
        n in any::<u32>(),
        d in prop::sample::select(vec![1u32, 2, 5, 10, 15, 30, 60, 120, 300, 600, 900, 3_600]),
    ) {
        let fast = FastDivU32::new(d);
        prop_assert_eq!(fast.div(n), n / d);
        prop_assert_eq!(fast.rem(n), n % d);
    }

    /// Any instant inside the window lands in a step that contains it.
    #[test]
    fn step_of_is_consistent_with_step_bounds(
        step in prop::sample::select(vec![1u32, 60, 300, 900]),
        n_steps in 1u32..500,
        offset in any::<u32>(),
    ) {
        let window = step * n_steps;
        let grid = StepGrid::new(Second::ZERO, step, window).unwrap();
        let t = Second(offset % window);
        let i = grid.step_of(t);
        prop_assert!(grid.step_start(i) <= t);
        prop_assert!(t < grid.step_end(i));
        let frac = grid.fraction_into_step(t);
        prop_assert!((0.0..1.0).contains(&frac), "fraction {} out of range", frac);
    }
}
