//! What the run's pages say of the run, read from its record, `run.toml` (the io-parquet crate
//! writes it; its layout in that crate's README): the provenance line, the source line, and
//! the trips given and dropped.

use std::collections::BTreeMap;

/// The record's values by `table.key` (`""` for the top), as written: strings without their
/// quotes, every other value as its text.
fn values(record: &str) -> BTreeMap<String, String> {
    let mut table = String::new();
    let mut out = BTreeMap::new();
    for line in record.lines().map(str::trim) {
        if line.starts_with('[') {
            table = line.trim_matches(['[', ']']).to_owned();
        } else if let Some((key, value)) = line.split_once('=') {
            let value = value.split_once(" #").map_or(value, |(v, _)| v).trim();
            let value = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')).unwrap_or(value);
            out.insert(format!("{table}.{}", key.trim()), value.to_owned());
        }
    }
    out
}

/// The provenance line: the version, the window, the bin, the choice model, the iterations and
/// what ended them, the seed, the network's fingerprint (its first eight digits); a part whose key
/// the record lacks (or cannot be read) is left out, nothing put in its place.
#[must_use]
pub fn provenance(record: &str) -> String {
    let v = values(record);
    let get = |k: &str| v.get(k).map(String::as_str);
    let mut parts: Vec<String> = Vec::new();
    if let Some(version) = get(".openmobisim_version") {
        parts.push(version.to_owned());
    }
    let second = |k: &str| get(k).and_then(|t| t.parse::<u32>().ok());
    if let (Some(a), Some(b)) = (second(".window_start_s"), second(".window_end_s")) {
        parts.push(format!("window {}–{}", crate::clock(a), crate::clock(b)));
    }
    if let Some(bin) = get("parameters.link_bin") {
        parts.push(format!("bin {bin} s"));
    }
    if let Some(choice) = get("parameters.choice") {
        parts.push(format!("choice {choice}"));
    }
    let ended = match get("result.stopped_by_rule") {
        Some("true") => Some("ended by the stopping rule"),
        Some("false") => Some("ended by the cap"),
        // A record that holds anything says when it does not hold the run's end.
        _ if v.is_empty() => None,
        _ => Some("its end not recorded"),
    };
    match (get("result.iterations"), ended) {
        (Some(n), Some(e)) => parts.push(format!("{n} it, {e}")),
        (Some(n), None) => parts.push(format!("{n} it")),
        (None, Some(e)) => parts.push(e.to_owned()),
        (None, None) => {}
    }
    if let Some(seed) = get("parameters.seed") {
        parts.push(format!("seed {seed}"));
    }
    if let Some(f) = get("network.fingerprint") {
        parts.push(format!("fingerprint {}", f.chars().take(8).collect::<String>()));
    }
    parts.join(" · ")
}

/// The source line: the OpenStreetMap credit when the network was read from OpenStreetMap.
#[must_use]
pub fn source(record: &str) -> &'static str {
    if values(record).get("network.source").is_some_and(|s| s == "osm") {
        "© OpenStreetMap contributors (ODbL)"
    } else {
        ""
    }
}

/// The kinds of repair that drop a trip, each with what the summary calls it: the demand's
/// readers' (`[demand.repairs]`), then the run's (`[result.repairs]`).
const DROPS: [(&str, &str); 7] = [
    ("demand.repairs.trips_unread", "a cell that cannot be read"),
    ("demand.repairs.trips_repeated", "an id given before"),
    ("demand.repairs.trips_of_unknown_class", "a class not given"),
    ("demand.repairs.trips_unplaced", "a position not placed"),
    ("demand.repairs.trips_without_mode", "no mode"),
    ("result.repairs.trips_outside_window", "departing outside the window"),
    ("result.repairs.trips_after_window", "reaching the network after the window"),
];

/// The trips the record says were given (`[demand] trips_given`, written when the demand
/// dropped some, else `[demand] trips`; `None` when it says neither) and, of them, those
/// dropped, by reason (each count above zero, in the order of [`DROPS`]).
#[must_use]
pub fn dropped(record: &str) -> (Option<u64>, Vec<(&'static str, u64)>) {
    let v = values(record);
    let count = |k: &str| v.get(k).and_then(|t| t.parse::<u64>().ok());
    let reasons = DROPS
        .iter()
        .filter_map(|&(k, why)| count(k).filter(|&n| n > 0).map(|n| (why, n)))
        .collect();
    (count("demand.trips_given").or_else(|| count("demand.trips")), reasons)
}
