//! Reading a written summary back: each value a test reads, by its `data-k` name.

#![allow(dead_code, reason = "each test binary uses a different part of this module")]

/// The text of the element named `key` (`data-k="key"`), up to its first tag.
///
/// # Panics
///
/// When the page has no such element.
pub fn value(html: &str, key: &str) -> String {
    let open = format!("data-k=\"{key}\"");
    let at = html.find(&open).unwrap_or_else(|| panic!("no {key}"));
    let start = html[at..].find('>').expect("the tag's end") + at + 1;
    let end = html[start..].find('<').expect("the next tag") + start;
    html[start..end].to_owned()
}

/// Whether the page has an element named `key`.
pub fn has(html: &str, key: &str) -> bool {
    html.contains(&format!("data-k=\"{key}\""))
}

/// What the pages keep of run `r` of `demand` on `network` and `timetable` with `parameters`:
/// its four tables written by `openmobisim_io_parquet::write` into the folder `name` of the
/// tests' scratch folder, then read back, as the binding reads them. The tests' networks are
/// built at 1 800 veh/h per lane, 15 km/h by bike and 4.8 km/h on foot.
///
/// # Panics
///
/// When the run cannot be written or read back.
pub fn kept(
    r: &openmobisim_loop::run::Run,
    network: &openmobisim_graph::network::Network,
    demand: &openmobisim_demand::demand::Demand,
    timetable: Option<&openmobisim_transit::timetable::Timetable>,
    parameters: &openmobisim_loop::parameters::Parameters,
    name: &str,
) -> openmobisim_map::Kept {
    use openmobisim_io_parquet::{Files, Given, Source, Transit};
    let report = openmobisim_io_gtfs::Report::default();
    let given = Given {
        source: Source::Tables,
        files: Files::default(),
        build_values: openmobisim_graph::network::BuildValues {
            capacity_per_lane: 1_800.0,
            bike_speed: 15.0,
            walk_speed: 4.8,
        },
        stop_walk_max: 1_500.0,
        transit: timetable.map(|tt| Transit {
            timetable: tt,
            report: &report,
            transfer_walk_max: 400.0,
        }),
        parameters,
        network_repairs: network.repairs(),
        trips_given: u64::from(demand.trip_count()),
        demand_repairs: demand.repairs(),
    };
    let folder = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    openmobisim_io_parquet::write(r, network, demand, &given, &folder).expect("written");
    let window = r.links.window();
    let t = openmobisim_io_parquet::read(&folder, demand, network, timetable, window, r.dropped())
        .expect("read back");
    assert!(t.warnings.is_empty(), "{:?}", t.warnings);
    openmobisim_map::Kept::of(demand, &t.trips, t.routes, t.links, window)
}
