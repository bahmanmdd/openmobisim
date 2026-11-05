//! The clock, the step grid and the order of events.

mod common;

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use common::SplitMix64;
use openmobisim_types::ids::EntityKind;
use openmobisim_types::time::{EventKey, Second, StepGrid, StepGridError, StepIndex};

#[test]
fn a_two_hour_window_in_ten_minute_steps_has_twelve_steps() {
    // By hand: 7 200 s / 600 s = 12; 07:30 is second 27 000, 09:30 is second 34 200.
    let grid = StepGrid::new(Second(27_000), 600, 7_200).unwrap();
    assert_eq!(grid.n_steps(), 12);
    assert_eq!(grid.steps().count(), 12);
    assert_eq!(grid.end(), Second(34_200));
    assert_eq!(grid.step_of(Second(27_000)), Some(StepIndex(0)));
    assert_eq!(grid.step_of(Second(27_599)), Some(StepIndex(0)));
    assert_eq!(grid.step_of(Second(27_600)), Some(StepIndex(1)));
    assert_eq!(grid.step_of(Second(34_199)), Some(StepIndex(11)));
    // Half-open: the window's end and anything outside belong to no step.
    assert_eq!(grid.step_of(Second(34_200)), None);
    assert_eq!(grid.step_of(Second(26_999)), None);
    assert_eq!(grid.step_of(Second::MAX), None);
}

#[test]
fn steps_are_half_open_and_contiguous() {
    let grid = StepGrid::new(Second::from_hours(7), 300, 3_600).unwrap();
    let mut previous_end = grid.origin();
    for i in grid.steps() {
        let (start, end) = (grid.step_start(i), grid.step_end(i));
        assert_eq!(start, previous_end, "steps leave no gap and do not overlap");
        assert_eq!(end.get() - start.get(), 300);
        assert_eq!(grid.step_of(start), Some(i), "a step contains its start");
        assert_eq!(grid.step_of(Second(end.get() - 1)), Some(i), "and its last second");
        previous_end = end;
    }
    assert_eq!(previous_end, grid.end());
    assert_eq!(grid.end(), Second::from_hours(8));
}

#[test]
fn every_instant_in_the_window_lies_in_the_step_that_claims_it() {
    // Property over seeded grids and instants: start <= t < end of the step of t, and that step
    // is floor((t - origin) / step) by hand.
    let mut g = SplitMix64(4);
    for _ in 0..10_000 {
        let step = [1u32, 60, 300, 600, 900][g.index(5)];
        let n_steps = 1 + u32::try_from(g.below(500)).unwrap();
        let origin = Second(u32::try_from(g.below(86_400)).unwrap());
        let grid = StepGrid::new(origin, step, step * n_steps).unwrap();
        let offset = u32::try_from(g.below(u64::from(step * n_steps))).unwrap();
        let t = Second(origin.get() + offset);
        let i = grid.step_of(t).expect("an instant inside the window has a step");
        assert_eq!(i.get(), offset / step);
        assert!(grid.step_start(i) <= t && t < grid.step_end(i));
    }
}

#[test]
fn a_partial_last_step_is_refused() {
    let err = StepGrid::new(Second::ZERO, 300, 3_500).unwrap_err();
    assert!(matches!(err, StepGridError::WindowNotWholeSteps { .. }), "{err:?}");
    assert!(err.to_string().contains("whole number"));
}

#[test]
fn zero_steps_zero_windows_and_overflowing_windows_are_refused() {
    assert_eq!(StepGrid::new(Second::ZERO, 0, 3_600), Err(StepGridError::ZeroStep));
    assert_eq!(StepGrid::new(Second::ZERO, 300, 0), Err(StepGridError::ZeroWindow));
    let err = StepGrid::new(Second(u32::MAX - 10), 1, 100).unwrap_err();
    assert!(matches!(err, StepGridError::WindowOverflowsClock { .. }), "{err:?}");
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
fn the_clock_reads_hours_minutes_and_seconds() {
    // By hand: 93 784 s = 26 h (93 600 s) + 3 min (180 s) + 4 s.
    assert_eq!(Second::from_hours(7).to_string(), "07:00:00");
    assert_eq!(Second::from_minutes(450), Second(27_000));
    assert_eq!(Second(93_784).to_string(), "26:03:04");
    assert_eq!(Second(10).saturating_sub(Second(25)), 0);
    assert_eq!(Second::MAX.saturating_add(1), Second::MAX);
}
