//! The delay's floor on the summary, by hand: a link whose cars lose under 2 s each is in the band
//! 0–15 % whatever its delay over free flow, as on the map (`openmobisim_map::rules::delay_band`).
//!
//! The run: nodes a (0, 0), b (100, 0), c (1 100, 0) m; `s` a→b 100 m and `l` b→c 1 000 m, both at
//! 36 km/h (free flow 10 s and 100 s), 1 800 veh/h; three cars a→c, the window 07:00–08:00 in bins
//! of 60 s, one iteration. Its links table is then edited by hand: every row's crossing times set
//! to its cars × 11.9 s on `s` (19 % over free flow, 1.9 s per car: under the floor) and × 130 s on
//! `l` (30 %, 30 s per car). So `s`'s 3 × 0.1 = 0.3 vehicle-km are in the band 0–15 % (9.1 % of
//! 3.3), `l`'s 3.0 in 15–50 % (90.9 %), and on the car map `s` is drawn in the first band's colour
//! (`k-b0`), `l` in the second's (`k-b1`). Without the floor `s` would be in 15–50 % with `l`.
//! For bikes: the floor is the cars'; bikes and walkers are drawn by their counts, never by delay.

mod common;

use std::path::PathBuf;
use std::sync::Arc;

use arrow_array::{ArrayRef, Float64Array, RecordBatch, StringArray, UInt32Array};
use common::value;
use openmobisim_demand::demand::Demand;
use openmobisim_demand::input::TripRow;
use openmobisim_graph::geometry::{Position, Xy};
use openmobisim_graph::network::{BuildValues, LinkRow, Network, NetworkBuilder};
use openmobisim_loop::parameters::{ChoiceModel, Parameters};
use openmobisim_loop::run::run;
use openmobisim_summary::{Summarised, write};
use openmobisim_types::mode::{Mode, ModeSet};
use openmobisim_types::time::Second;
use openmobisim_types::units::Metres;
use parquet::arrow::ArrowWriter;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

const START: u32 = 25_200;

fn network() -> Network {
    let street = |length_m: f64| LinkRow {
        modes: ModeSet::of(&[Mode::Car]),
        lanes: Some(1),
        car_speed_km_h: Some(36.0),
        length_m: Some(length_m),
        capacity_veh_h: Some(1_800.0),
        signal_at_downstream_end: false,
    };
    let mut b = NetworkBuilder::new();
    for (id, x) in [("a", 0.0), ("b", 100.0), ("c", 1_100.0)] {
        b.add_node(id, Position::Metres(Xy::new(x, 0.0)));
    }
    b.add_link("s", "a", "b", street(100.0));
    b.add_link("l", "b", "c", street(1_000.0));
    b.build(BuildValues { capacity_per_lane: 1_800.0, bike_speed: 15.0, walk_speed: 4.8 })
        .expect("a network")
}

/// The links table in `folder` with every row's crossing times set to its cars × `mean(link)` s.
fn set_times(folder: &std::path::Path, mean: fn(&str) -> f64) {
    let path = folder.join("links.parquet");
    let file = std::fs::File::open(&path).expect("open");
    let batches: Vec<RecordBatch> = ParquetRecordBatchReaderBuilder::try_new(file)
        .expect("a Parquet file")
        .build()
        .expect("a reader")
        .collect::<Result<_, _>>()
        .expect("batches");
    let out = std::fs::File::create(&path).expect("a file");
    let mut writer = ArrowWriter::try_new(out, batches[0].schema(), None).expect("a writer");
    for batch in batches {
        let column = |name: &str| batch.column(batch.schema().index_of(name).expect("a column"));
        let ids = column("link_id").as_any().downcast_ref::<StringArray>().expect("text").clone();
        let cars = column("cars").as_any().downcast_ref::<UInt32Array>().expect("counts").clone();
        let times: Float64Array = (0..batch.num_rows())
            .map(|i| (cars.value(i) > 0).then(|| f64::from(cars.value(i)) * mean(ids.value(i))))
            .collect();
        let mut columns: Vec<ArrayRef> = batch.columns().to_vec();
        columns[batch.schema().index_of("car_time_s").expect("a column")] = Arc::new(times);
        writer.write(&RecordBatch::try_new(batch.schema(), columns).expect("a batch")).expect("ok");
    }
    writer.close().expect("closed");
}

#[test]
fn a_link_whose_cars_lose_under_2_s_each_is_in_the_first_band_whatever_its_delay() {
    let net = network();
    let row = |id: &str, departure: u32| TripRow {
        id: id.to_string(),
        origin: Position::Metres(Xy::new(0.0, 0.0)),
        destination: Position::Metres(Xy::new(1_100.0, 0.0)),
        departure: Second(departure),
        class: "all".to_string(),
        modes: ModeSet::of(&[Mode::Car]),
    };
    let rows = [row("c1", START), row("c2", START + 10), row("c3", START + 20)];
    let dem = Demand::build(&rows, &["all".to_string()], &net, None, Metres(400.0)).expect("trips");
    let parameters = Parameters {
        link_bin: 60,
        iteration_cap: 1,
        choice: ChoiceModel::Deterministic,
        ..Parameters::defaults(1)
    };
    let r =
        run(&net, &dem, None, (Second(START), Second(START + 3_600)), &parameters).expect("a run");
    let name = "summary-floor.run";
    common::kept(&r, &net, &dem, None, &parameters, name);
    let folder = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    set_times(&folder, |id| if id == "s" { 11.9 } else { 130.0 });

    let window = r.links.window();
    let t = openmobisim_io_parquet::read(&folder, &dem, &net, None, window, r.dropped())
        .expect("it opens");
    assert!(t.warnings.is_empty(), "{:?}", t.warnings);
    let kept = openmobisim_map::Kept::of(&dem, &t.trips, t.routes, t.links, window);
    let summarised = Summarised {
        kept: &kept,
        network: &net,
        timetable: None,
        lines: None,
        iterations: &r.iterations,
        stopped_by_rule: Some(r.stopped_by_rule),
        record: "",
    };
    let page = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("summary-floor.html");
    write(&summarised, "floor", &page).expect("written");
    let html = std::fs::read_to_string(&page).expect("the summary");
    let shown: Vec<String> =
        ["band.0.km", "band.0.share", "band.1.km", "band.1.share", "band.2.km"]
            .iter()
            .map(|k| value(&html, k))
            .collect();
    assert_eq!(shown, ["0.3", "9.1", "3.0", "90.9", "0.0"]);
    let car = &html[html.find("data-k=\"map.car\"").expect("the car map")..];
    let car = &car[..car.find("</svg>").expect("its end")];
    assert_eq!(
        (car.matches("class=\"k-b0\"").count(), car.matches("class=\"k-b1\"").count()),
        (1, 1),
        "{car}"
    );
}
