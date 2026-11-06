//! What the timetable refuses, each refusal naming the run or the stop at fault; and what it
//! does not refuse: runs that overtake on the same stops.

use openmobisim_transit::raptor::{Query, Raptor};
use openmobisim_transit::timetable::{Call, TimetableBuilder, TimetableError};
use openmobisim_types::ids::{EntityId, StopId, TransitRunId};
use openmobisim_types::time::Second;
use openmobisim_types::units::{Duration, Metres};

fn call(stop: &str, arrival: u32, departure: u32) -> Call {
    Call { stop: stop.into(), arrival: Second(arrival), departure: Second(departure) }
}

/// Stops a, b, c and run `ok` a to b to c.
fn builder() -> TimetableBuilder {
    let mut b = TimetableBuilder::new();
    for (i, s) in ["a", "b", "c"].into_iter().enumerate() {
        b.add_stop(s, (Metres(100.0 * f64::from(u32::try_from(i).unwrap())), Metres::ZERO));
    }
    b.add_run("ok", vec![call("a", 0, 0), call("b", 60, 90), call("c", 200, 200)]);
    b
}

fn refused(b: TimetableBuilder) -> TimetableError {
    let e = b.build().expect_err("refused");
    eprintln!("{e}");
    e
}

#[test]
fn a_call_out_of_order_is_refused_naming_the_run() {
    let mut b = builder();
    b.add_run("leaves-early", vec![call("a", 0, 0), call("b", 90, 60), call("c", 200, 200)]);
    let e = refused(b);
    assert_eq!(e, TimetableError::CallOutOfOrder { run: "leaves-early".into(), call: 1 });
    assert!(e.to_string().contains("leaves-early"));
    let mut b = builder();
    b.add_run("goes-back", vec![call("a", 0, 100), call("b", 90, 90), call("c", 200, 200)]);
    assert_eq!(refused(b), TimetableError::CallOutOfOrder { run: "goes-back".into(), call: 1 });
}

#[test]
fn an_unknown_stop_is_refused_naming_the_run_and_the_stop() {
    let mut b = builder();
    b.add_run("lost", vec![call("a", 0, 0), call("z", 60, 60)]);
    let e = refused(b);
    assert_eq!(e, TimetableError::UnknownStop { run: "lost".into(), stop: "z".into() });
    assert!(e.to_string().contains("lost") && e.to_string().contains('z'));
    let mut b = builder();
    b.add_footpath("a", "z", Duration(60.0));
    assert_eq!(refused(b), TimetableError::FootpathUnknownStop { stop: "z".into() });
}

#[test]
fn overtaking_runs_on_the_same_stops_build_into_two_patterns() {
    // On the stops of `ok` (a at 0, b at 60 to 90, c at 200): `express` leaves a later (10) and
    // is ahead at b (50) and c (150); `late` follows `ok` all the way (a at 300). `express`
    // opens a pattern of its own, `late` joins the pattern of `ok`. Patterns are numbered by
    // their lowest run id: "express" < "late" < "ok".
    let mut b = builder();
    b.add_run("express", vec![call("a", 10, 10), call("b", 50, 50), call("c", 150, 150)]);
    b.add_run("late", vec![call("a", 300, 300), call("b", 360, 390), call("c", 500, 500)]);
    let tt = b.build().expect("overtaking runs build");
    assert_eq!((tt.pattern_count(), tt.run_count()), (2, 3));
    let run = |name: &str| tt.run_ids().typed_id_of::<TransitRunId>(name).expect("a run");
    assert_eq!(tt.run_pattern(run("express")), 0);
    assert_eq!((tt.run_pattern(run("ok")), tt.run_pattern(run("late"))), (1, 1));
    assert_eq!(tt.pattern_runs(1), [run("ok"), run("late")]);
    // A traveller catches whichever arrives first: at a at 0, `express` (at c at 150, not
    // `ok` at 200); at b at 55, after `express` has left, `ok` (at c at 200).
    let stop = |name: &str| tt.stop_ids().typed_id_of::<StopId>(name).expect("a stop");
    let ask = |from: &str, t: u32| {
        let q = Query {
            access: vec![(stop(from), Second(t))],
            egress: vec![(stop("c"), Duration::ZERO)],
            max_rides: 1,
        };
        let j = Raptor::new(&tt, Duration::ZERO).earliest(&q).expect("a journey");
        (tt.run_ids().external(j.rides[0].run.raw()).to_owned(), j.arrival)
    };
    assert_eq!(ask("a", 0), ("express".to_owned(), Second(150)));
    assert_eq!(ask("b", 55), ("ok".to_owned(), Second(200)));
}

#[test]
fn repeated_ids_short_runs_and_bad_footpaths_are_refused() {
    let mut b = builder();
    b.add_stop("b", (Metres::ZERO, Metres::ZERO));
    assert_eq!(refused(b), TimetableError::DuplicateStop { stop: "b".into() });
    let mut b = builder();
    b.add_run("ok", vec![call("a", 0, 0), call("c", 60, 60)]);
    assert_eq!(refused(b), TimetableError::DuplicateRun { run: "ok".into() });
    let mut b = builder();
    b.add_run("stub", vec![call("a", 0, 0)]);
    assert_eq!(refused(b), TimetableError::TooFewCalls { run: "stub".into() });
    let mut b = builder();
    b.add_footpath("a", "b", Duration(-1.0));
    assert_eq!(refused(b), TimetableError::FootpathSeconds { from: "a".into(), to: "b".into() });
}

#[test]
fn ids_follow_the_external_ids_not_the_order_given() {
    let mut b = TimetableBuilder::new();
    for s in ["c", "a", "b"] {
        b.add_stop(s, (Metres::ZERO, Metres::ZERO));
    }
    b.add_run("r2", vec![call("a", 0, 0), call("b", 60, 60)]);
    b.add_run("r1", vec![call("a", 30, 30), call("b", 90, 90)]);
    let tt = b.build().expect("builds");
    assert_eq!(tt.stop_ids().iter().collect::<Vec<_>>(), ["a", "b", "c"]);
    assert_eq!(tt.run_ids().iter().collect::<Vec<_>>(), ["r1", "r2"]);
    // One pattern, its runs by departure: r2 (at 0) before r1 (at 30).
    let names: Vec<&str> =
        tt.pattern_runs(0).iter().map(|&r| tt.run_ids().external(r.raw())).collect();
    assert_eq!(names, ["r2", "r1"]);
}
