//! The footer's lines from a run's record, by hand: each part of the provenance line from its key,
//! a part whose key is missing left out (nothing put in its place), and the source line.

use openmobisim_map::record::{dropped, provenance, source};

/// A whole record of the io-parquet layout, by hand.
const WHOLE: [&str; 9] = [
    "openmobisim_version = \"0.2.0-dev\"",
    "window_start_s = 27000",
    "window_end_s = 34200",
    "[parameters]\nlink_bin = 300  # s",
    "[parameters]\nchoice = \"logit\"  # name",
    "[parameters]\nseed = 7  # none",
    "[network]\nsource = \"osm\"\nfingerprint = \"b61e1b28aa00ff11\"",
    "[result]\niterations = 10",
    "[result]\nstopped_by_rule = true",
];

fn record(lines: &[&str]) -> String {
    lines.iter().map(|l| format!("{l}\n")).collect()
}

#[test]
fn every_part_from_its_key_and_a_missing_key_leaves_its_part_out() {
    let full = record(&WHOLE);
    assert_eq!(
        provenance(&full),
        "0.2.0-dev · window 07:30–09:30 · bin 300 s · choice logit · 10 it, ended by the stopping \
         rule · seed 7 · fingerprint b61e1b28"
    );
    assert_eq!(source(&full), "© OpenStreetMap contributors (ODbL)");
    let parts = [
        "0.2.0-dev",
        "window 07:30–09:30",
        "window 07:30–09:30",
        "bin 300 s",
        "choice logit",
        "seed 7",
        "fingerprint b61e1b28",
        "10 it, ended by the stopping rule",
        "10 it, ended by the stopping rule",
    ];
    for (k, gone) in parts.iter().enumerate() {
        let lines: Vec<&str> =
            WHOLE.iter().enumerate().filter(|(i, _)| *i != k).map(|(_, l)| *l).collect();
        let line = provenance(&record(&lines));
        assert!(!line.contains(gone), "without line {k}: {line}");
        assert!(!line.contains('?') && !line.contains("00:00") && !line.contains("  "), "{line}");
    }
    let without_iterations: Vec<&str> =
        WHOLE.iter().copied().filter(|l| !l.contains("iterations")).collect();
    assert!(provenance(&record(&without_iterations)).contains(" · ended by the stopping rule · "));
    let without_end: Vec<&str> =
        WHOLE.iter().copied().filter(|l| !l.contains("stopped_by_rule")).collect();
    assert!(provenance(&record(&without_end)).contains(" · 10 it, its end not recorded · "));
    let tables: Vec<&str> = WHOLE.iter().copied().filter(|l| !l.contains("source")).collect();
    assert_eq!(source(&record(&tables)), "");
}

#[test]
fn an_empty_record_gives_empty_lines() {
    assert_eq!(provenance(""), "");
    assert_eq!(source(""), "");
}

/// The trips given and dropped, by reason, from `[demand] trips_given`, `[demand.repairs]` and
/// `[result.repairs]`; a repair that drops no trip (`classes_repeated`) is not a reason.
#[test]
fn the_trips_given_and_dropped_by_reason() {
    let r = "[demand]\ntrips = 6\ntrips_given = 10\n\n[demand.repairs]\nclasses_repeated = 1\n\
             trips_repeated = 2\ntrips_unplaced = 1\n\n[result]\niterations = 3\n\n\
             [result.repairs]\nwindow_end_moved = 1\ntrips_outside_window = 1\n";
    assert_eq!(
        dropped(r),
        (
            Some(10),
            vec![
                ("an id given before", 2),
                ("a position not placed", 1),
                ("departing outside the window", 1)
            ]
        )
    );
    assert_eq!(dropped(""), (None, vec![]));
    // Without trips_given (the demand dropped none): the trips, all given.
    let r = "[demand]\ntrips = 7\n\n[result.repairs]\ntrips_outside_window = 1\n";
    assert_eq!(dropped(r), (Some(7), vec![("departing outside the window", 1)]));
}
