//! Fixtures shared by the reader's tests: a street to walk on, feeds written as zips, and the
//! runs and footpaths of a timetable read back by external id.

#![allow(dead_code, reason = "each test binary uses a different part of this module")]

use std::io::Write;
use std::path::PathBuf;

use openmobisim_graph::geometry::{LonLat, Position};
use openmobisim_graph::network::{BuildValues, LinkRow, Network, NetworkBuilder};
use openmobisim_io_gtfs::{GtfsError, Report};
use openmobisim_transit::timetable::Timetable;
use openmobisim_types::ids::{EntityId, StopId, TransitRunId};
use openmobisim_types::mode::{Mode, ModeSet};
use openmobisim_types::units::Metres;

/// Test values for the three build values (not defaults): capacity_per_lane 1 800 veh/h,
/// bike_speed 15 km/h, walk_speed 4.8 km/h.
pub const VALUES: BuildValues =
    BuildValues { capacity_per_lane: 1_800.0, bike_speed: 15.0, walk_speed: 4.8 };

/// The point `metres` east of 4.36°E on the parallel 52.0°N, at about 68.7 km per degree.
pub fn east(metres: f64) -> LonLat {
    LonLat::new(4.36 + metres / 68_700.0, 52.0)
}

/// A walking street through nodes at `marks` metres east of 4.36°E on 52.0°N, both ways, each
/// link's length the difference of its marks (given, not measured).
pub fn street(marks: &[f64]) -> Network {
    let mut b = NetworkBuilder::new();
    for (i, &m) in marks.iter().enumerate() {
        b.add_node(format!("n{i}"), Position::LonLat(east(m)));
    }
    for (i, w) in marks.windows(2).enumerate() {
        let row = LinkRow {
            modes: ModeSet::of(&[Mode::Walk]),
            lanes: None,
            car_speed_km_h: None,
            length_m: Some(w[1] - w[0]),
            capacity_veh_h: None,
            signal_at_downstream_end: false,
        };
        b.add_link(format!("f{i}"), format!("n{i}"), format!("n{}", i + 1), row);
        b.add_link(format!("b{i}"), format!("n{}", i + 1), format!("n{i}"), row);
    }
    b.build(VALUES).expect("a street builds")
}

/// A zip at a fresh path holding `files` (name, text), each under `prefix` (`""` for the root).
pub fn zip(name: &str, prefix: &str, files: &[(&str, &str)]) -> PathBuf {
    let path =
        std::env::temp_dir().join(format!("openmobisim-gtfs-{name}-{}.zip", std::process::id()));
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).expect("a temporary file"));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for (file, text) in files {
        zip.start_file(format!("{prefix}{file}"), options).expect("a zip entry");
        zip.write_all(text.as_bytes()).expect("written");
    }
    zip.finish().expect("a zip");
    path
}

/// Read `files` as a feed on a 1 000 m street, `transfer_walk_max` 400 m.
pub fn read(name: &str, files: &[(&str, &str)]) -> Result<(Timetable, Report), GtfsError> {
    let path = zip(name, "", files);
    let out = openmobisim_io_gtfs::read(&path, &street(&[0.0, 1_000.0]), Metres(400.0));
    std::fs::remove_file(path).ok();
    out
}

/// The run of external id `id`, if the timetable has it.
pub fn run(t: &Timetable, id: &str) -> Option<TransitRunId> {
    t.run_ids().id_of(id).map(TransitRunId::new)
}

/// A run's calls: stop external id, arrival and departure seconds.
pub fn calls(t: &Timetable, id: &str) -> Vec<(String, u32, u32)> {
    let r = run(t, id).unwrap_or_else(|| panic!("run {id} is in the timetable"));
    t.calls(r)
        .map(|(s, a, d)| (t.stop_ids().external(s.raw()).to_owned(), a.get(), d.get()))
        .collect()
}

/// The footpaths from stop `id`: to stop external id and seconds.
pub fn footpaths(t: &Timetable, id: &str) -> Vec<(String, f64)> {
    let s: StopId = t.stop_ids().typed_id_of(id).expect("the stop");
    t.footpaths(s).map(|(to, d)| (t.stop_ids().external(to.raw()).to_owned(), d.get())).collect()
}

/// Seconds after midnight of `h:m:s`.
pub const fn hms(h: u32, m: u32, s: u32) -> u32 {
    h * 3600 + m * 60 + s
}

/// The header of `calendar.txt`.
pub const CALENDAR: &str =
    "service_id,monday,tuesday,wednesday,thursday,friday,saturday,sunday,start_date,end_date\n";
/// The header of `stop_times.txt` without shape distances.
pub const STOP_TIMES: &str = "trip_id,arrival_time,departure_time,stop_id,stop_sequence\n";
/// Four stops on 52.0°N, 0, 100, 400 and 1 000 m east of 4.36°E (on the street of [`read`]).
pub const STOPS: &str = "stop_id,stop_name,stop_lat,stop_lon\n\
    A,Alpha,52.0,4.36\n\
    B,Bravo,52.0,4.361455604075692\n\
    C,Charlie,52.0,4.365822416302766\n\
    D,Delta,52.0,4.374556040756914\n";
