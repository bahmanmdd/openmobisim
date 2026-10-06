//! Scenario edits of a timetable (S238): a line cancelled, a line at a new headway.

use openmobisim_core_graph::geometry::LonLat;
use openmobisim_core_transit::{
    ALIGHT, BOARD, CallSpec, Headway, RouteSpec, ServiceChanges, ServiceDate, StopSpec, Timetable,
    TimetableBuilder,
};
use openmobisim_core_types::ids::{EntityId, TransitRunId};

/// Line `L` every 10 min from 07:00 to 08:50, A → B in 6 min; line `M` once, B → A at 09:00.
fn day() -> Timetable {
    let mut b = TimetableBuilder::new(ServiceDate::parse("20261009").unwrap());
    let stop = |b: &mut TimetableBuilder, id: &str, lon: f64| {
        b.add_stop(StopSpec {
            external_id: id.into(),
            name: id.into(),
            position: LonLat::new(lon, 52.0),
            parent: None,
        })
    };
    let (a, z) = (stop(&mut b, "A", 4.90), stop(&mut b, "B", 4.92));
    let l =
        b.add_route(RouteSpec { external_id: "L".into(), short_name: "L".into(), route_type: 3 });
    let m =
        b.add_route(RouteSpec { external_id: "M".into(), short_name: "M".into(), route_type: 0 });
    let call = |stop, t| CallSpec { stop, arrival: t, departure: t, flags: BOARD | ALIGHT };
    for k in 0..12 {
        let t = 7 * 3600 + 600 * k;
        b.add_run(format!("L{k:02}"), l, &[call(a, t), call(z, t + 360)]);
    }
    b.add_run("M1", m, &[call(z, 9 * 3600), call(a, 9 * 3600 + 300)]);
    b.add_transfer(a, z, 120);
    b.build().0
}

fn departures(t: &Timetable, route: &str) -> Vec<u32> {
    let r = t.route_ids().id_of(route).unwrap();
    let mut out: Vec<u32> = (0..t.run_count())
        .map(TransitRunId::new)
        .filter(|&run| t.run_route(run) == r)
        .map(|run| t.scheduled().departure[t.run_calls(run).start])
        .collect();
    out.sort_unstable();
    out
}

#[test]
fn a_line_at_a_new_headway_in_a_window_and_a_line_cancelled() {
    let t = day();
    let (l, m) = (t.route_ids().id_of("L").unwrap(), t.route_ids().id_of("M").unwrap());
    let changes = ServiceChanges {
        cancelled: vec![m],
        headways: vec![Headway {
            route: l,
            headway_s: 300,
            from_s: 7 * 3600 + 1800,
            to_s: 8 * 3600 + 1800,
        }],
    };
    let (edited, report) = t.with_changes(&changes).unwrap();
    // 07:30 to 08:20 (six runs) replaced by one every 5 min from 07:30 while before 08:30.
    assert_eq!((report.runs_cancelled, report.runs_replaced, report.runs_added), (1, 6, 12));
    let expected: Vec<u32> = (0..6)
        .map(|k| 7 * 3600 + 600 * k)
        .filter(|&s| s < 7 * 3600 + 1800)
        .chain((0..12).map(|k| 7 * 3600 + 1800 + 300 * k))
        .chain((0..12).map(|k| 7 * 3600 + 600 * k).filter(|&s| s >= 8 * 3600 + 1800))
        .collect();
    assert_eq!(departures(&edited, "L"), expected);
    assert!(departures(&edited, "M").is_empty());
    // Each new run copies the template's 6-minute ride; ids of stops and lines are kept.
    let run = TransitRunId::new(edited.run_ids().id_of("L06@27900").expect("the 07:45 run"));
    let calls = edited.run_calls(run);
    assert_eq!(
        edited.scheduled().arrival[calls.end - 1] - edited.scheduled().departure[calls.start],
        360
    );
    assert_eq!(edited.stop_ids().id_of("B"), t.stop_ids().id_of("B"));
    assert_eq!(edited.route_ids().id_of("M"), Some(m));
    assert_eq!(edited.transfers(), t.transfers());
    assert_eq!(edited.call_flags(calls.start), BOARD | ALIGHT);
    let _ = ALIGHT;
}

#[test]
fn nothing_changed_is_the_same_day_and_wrong_changes_are_refused() {
    let t = day();
    let (same, report) = t.with_changes(&ServiceChanges::default()).unwrap();
    assert_eq!(report, Default::default());
    assert_eq!(same.run_count(), t.run_count());
    assert_eq!(same.scheduled(), t.scheduled());
    let bad = |c: ServiceChanges| t.with_changes(&c).unwrap_err();
    assert!(bad(ServiceChanges { cancelled: vec![9], ..Default::default() }).contains("no line 9"));
    let h = |headway_s, from_s, to_s| ServiceChanges {
        headways: vec![Headway { route: 0, headway_s, from_s, to_s }],
        ..Default::default()
    };
    assert!(bad(h(0, 0, 10)).contains("headway of 0"));
    assert!(bad(h(60, 10, 10)).contains("ends before"));
}
