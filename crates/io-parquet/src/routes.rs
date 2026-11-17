//! `routes.parquet`: what each trip took at the run's last iteration, one row per leg, in trip id
//! order (the external ids sorted bytewise) then leg order; a trip without an alternative has no
//! row. Routes nobody took are not written.
//!
//! | Column | Arrow type | Null | Meaning |
//! |---|---|---|---|
//! | `trip_id` | Utf8 | no | the trip's external id |
//! | `leg` | UInt32 | no | the leg's place in the trip, from 0 |
//! | `kind` | Utf8 | no | `walk_gap`, `car`, `bike`, `walk`, `ride` or `transfer_walk` |
//! | `seconds` | Float64 | yes | a `walk_gap`'s or a `transfer_walk`'s seconds, as priced (constant over the run); null for the others |
//! | `links` | List<Utf8> | yes | a `car`, `bike` or `walk` leg's link ids in the order travelled (empty when the trip's two nodes are one); null for the others |
//! | `run_id` | Utf8 | yes | a `ride`'s transit run |
//! | `board_stop`, `alight_stop` | Utf8 | yes | a `ride`'s stops: boarded at, left at |
//! | `board_s`, `alight_s` | UInt32 | yes | a `ride`'s departure from its board stop and arrival at its alight stop, seconds of the clock |
//! | `from_stop`, `to_stop` | Utf8 | yes | a `transfer_walk`'s stops: the one left, the one reached |
//!
//! The kinds: a `walk_gap` is a walk between a door and the network or a stop, on no link,
//! priced by its seconds; `car`, `bike` and `walk` a route on the network's links; a `ride` one
//! transit run from stop to stop; a `transfer_walk` a footpath of the timetable between two rides.
//! A car, bike or walk trip is a `walk_gap`, its route and a `walk_gap`; a transit trip a
//! `walk_gap`, its rides, with a `transfer_walk` between two rides where it changes stop (none
//! where the next ride leaves from the stop the last was left at), and a `walk_gap`. A route's
//! seconds are not written: the loop keeps a trip's realised door-to-door time
//! (`trips.parquet`), not each route's; a ride's are its two clock seconds.

use std::fs::File;
use std::path::Path;
use std::sync::Arc;

use arrow_array::RecordBatch;
use arrow_array::builder::{Float64Builder, ListBuilder, StringBuilder, UInt32Builder};
use arrow_schema::{DataType, Field, Schema};
use openmobisim_demand::demand::Demand;
use openmobisim_graph::network::Network;
use openmobisim_loop::alternative::Leg;
use openmobisim_loop::run::Run;
use openmobisim_transit::raptor::{Footpath, Ride};
use openmobisim_transit::timetable::Timetable;
use openmobisim_types::ids::{EntityId, LinkId, StopId, TripId};
use openmobisim_types::mode::Mode;
use openmobisim_types::units::Duration;
use parquet::arrow::ArrowWriter;

use crate::{Table, WriteError, column};

/// One leg of what a trip took, as `routes.parquet` holds it and [`crate::read_routes`] reads
/// it back.
#[derive(Clone, Debug, PartialEq)]
pub enum RouteLeg {
    /// A walk between a door and the network or a stop, on no link: its seconds.
    WalkGap(Duration),
    /// A route on the network by car, bike or on foot: the links in order.
    Route {
        /// Car, bike or walk.
        mode: Mode,
        /// The links in the order travelled.
        links: Vec<LinkId>,
    },
    /// One transit run from stop to stop.
    Ride(Ride),
    /// A footpath between two rides.
    TransferWalk(Footpath),
}

/// The file's columns, in order: the writer writes them and the reader asks for exactly them.
pub(crate) fn schema() -> Schema {
    let text = |name: &str, null: bool| Field::new(name, DataType::Utf8, null);
    let list = DataType::List(Arc::new(Field::new_list_field(DataType::Utf8, true)));
    Schema::new(vec![
        text("trip_id", false),
        Field::new("leg", DataType::UInt32, false),
        text("kind", false),
        Field::new("seconds", DataType::Float64, true),
        Field::new("links", list, true),
        text("run_id", true),
        text("board_stop", true),
        text("alight_stop", true),
        Field::new("board_s", DataType::UInt32, true),
        Field::new("alight_s", DataType::UInt32, true),
        text("from_stop", true),
        text("to_stop", true),
    ])
}

/// The legs of what `trip` took in `run`, as the file holds them: a journey's rides and
/// footpaths unfolded.
pub(crate) fn legs(run: &Run, trip: TripId) -> Vec<RouteLeg> {
    let Some(taken) = run.taken(trip) else { return Vec::new() };
    let mut out = Vec::new();
    for leg in &taken.legs {
        match leg {
            Leg::Walk(d) => out.push(RouteLeg::WalkGap(*d)),
            Leg::Route { mode, links } => {
                out.push(RouteLeg::Route { mode: *mode, links: links.clone() });
            }
            Leg::Journey(j) => {
                for (k, ride) in j.rides.iter().enumerate() {
                    if let Some(Some(f)) = k.checked_sub(1).map(|i| j.footpaths[i]) {
                        out.push(RouteLeg::TransferWalk(f));
                    }
                    out.push(RouteLeg::Ride(*ride));
                }
            }
        }
    }
    out
}

/// The file's columns as they are filled, row by row, straight into Arrow's builders: an id is
/// copied from its table into the column's buffer, never held as a string of its own.
#[derive(Default)]
struct Columns {
    trip: StringBuilder,
    leg: UInt32Builder,
    kind: StringBuilder,
    seconds: Float64Builder,
    links: ListBuilder<StringBuilder>,
    run: StringBuilder,
    board: StringBuilder,
    alight: StringBuilder,
    board_s: UInt32Builder,
    alight_s: UInt32Builder,
    from: StringBuilder,
    to: StringBuilder,
}

/// The external ids a row is written with: the links', the timetable's.
struct Names<'a> {
    network: &'a Network,
    timetable: Option<&'a Timetable>,
}

impl<'a> Names<'a> {
    /// The timetable of a ride or a transfer walk: [`check`] made sure, before anything was
    /// written, that every one has its timetable.
    fn timetable(&self) -> &'a Timetable {
        self.timetable.expect("checked before writing: every ride is on the timetable given")
    }

    fn stop(&self, s: StopId) -> &'a str {
        self.timetable().stop_ids().external(s.raw())
    }
}

/// Refuse, before anything is written, a ride or a transfer walk that the timetable given does
/// not have (no timetable, or a run or stop beyond its counts).
///
/// # Errors
///
/// [`WriteError::NotTheRunsJourney`], naming the first such trip.
pub(crate) fn check(
    run: &Run,
    demand: &Demand,
    timetable: Option<&Timetable>,
) -> Result<(), WriteError> {
    let (runs, stops) = timetable.map_or((0, 0), |tt| (tt.run_count(), tt.stop_count()));
    for t in 0..demand.trip_count() {
        let held = legs(run, TripId::new(t)).iter().all(|leg| match leg {
            RouteLeg::Ride(r) => {
                r.run.raw() < runs && r.board.0.raw() < stops && r.alight.0.raw() < stops
            }
            RouteLeg::TransferWalk(f) => f.from.raw() < stops && f.to.raw() < stops,
            _ => true,
        });
        if !held {
            return Err(WriteError::NotTheRunsJourney(demand.trip_ids().external(t).to_owned()));
        }
    }
    Ok(())
}

impl Columns {
    /// Push leg `k` of trip `trip`, every column a value or a null.
    fn push(&mut self, trip: &str, k: usize, leg: &RouteLeg, n: &Names<'_>) {
        self.trip.append_value(trip);
        self.leg.append_value(u32::try_from(k).expect("fewer than 2^32 legs"));
        let (mut seconds, mut ride, mut walk) = (None, None, None);
        self.kind.append_value(match leg {
            RouteLeg::WalkGap(d) => {
                seconds = Some(d.get());
                "walk_gap"
            }
            RouteLeg::Route { mode, links } => {
                for l in links {
                    self.links.values().append_value(n.network.link_ids().external(l.raw()));
                }
                match mode {
                    Mode::Car => "car",
                    Mode::Bike => "bike",
                    _ => "walk",
                }
            }
            RouteLeg::Ride(r) => {
                ride = Some(*r);
                "ride"
            }
            RouteLeg::TransferWalk(f) => {
                seconds = Some(f.seconds.get());
                walk = Some(*f);
                "transfer_walk"
            }
        });
        self.links.append(matches!(leg, RouteLeg::Route { .. }));
        self.seconds.append_option(seconds);
        self.run.append_option(ride.map(|r| n.timetable().run_ids().external(r.run.raw())));
        self.board.append_option(ride.map(|r| n.stop(r.board.0)));
        self.alight.append_option(ride.map(|r| n.stop(r.alight.0)));
        self.board_s.append_option(ride.map(|r| r.board.1.get()));
        self.alight_s.append_option(ride.map(|r| r.alight.1.get()));
        self.from.append_option(walk.map(|f| n.stop(f.from)));
        self.to.append_option(walk.map(|f| n.stop(f.to)));
    }
}

/// Trips per batch the file is written in: the batch's arrays are a few megabytes at most, and
/// freed once written (the writer keeps the encoded row group, the file's size, until it closes).
const TRIPS_PER_BATCH: usize = 8_192;

/// Write the routes table to `path`, in batches of [`TRIPS_PER_BATCH`] trips; `demand`, `network`
/// and `timetable` give the external ids.
///
/// Every ride and transfer walk is on `timetable` ([`check`], before anything is written).
///
/// # Errors
///
/// The file that cannot be written, or the writer's refusal.
pub(crate) fn file(
    path: &Path,
    run: &Run,
    demand: &Demand,
    network: &Network,
    timetable: Option<&Timetable>,
) -> Result<Table, WriteError> {
    let schema = Arc::new(schema());
    let mut writer = ArrowWriter::try_new(File::create(path)?, Arc::clone(&schema), None)?;
    let names = Names { network, timetable };
    let ids: Vec<&str> = demand.trip_ids().iter().collect();
    let mut rows = 0;
    for (chunk, trips) in ids.chunks(TRIPS_PER_BATCH).enumerate() {
        let mut c = Columns::default();
        for (i, id) in trips.iter().enumerate() {
            let trip = TripId::from_index(chunk * TRIPS_PER_BATCH + i);
            for (k, leg) in legs(run, trip).iter().enumerate() {
                c.push(id, k, leg, &names);
            }
        }
        let columns = vec![
            column(c.trip.finish()),
            column(c.leg.finish()),
            column(c.kind.finish()),
            column(c.seconds.finish()),
            column(c.links.finish()),
            column(c.run.finish()),
            column(c.board.finish()),
            column(c.alight.finish()),
            column(c.board_s.finish()),
            column(c.alight_s.finish()),
            column(c.from.finish()),
            column(c.to.finish()),
        ];
        let batch = RecordBatch::try_new(Arc::clone(&schema), columns)?;
        rows += batch.num_rows();
        writer.write(&batch)?;
    }
    writer.close()?;
    Ok(Table { path: path.to_path_buf(), rows })
}
