//! Scheduled public transport: the timetable of one service day and RAPTOR
//! (design §18; S199).
//!
//! | Module | What it owns |
//! |---|---|
//! | [`date`] | Service days: calendar dates as day numbers, weekdays |
//! | [`kind`] | The kind of a service from GTFS `route_type`, and whether it rides the roads |
//! | [`timetable`] | Stops, routes, runs and their calls; scheduled and realised call times |
//! | [`raptor`] | Patterns, walking transfers and the RAPTOR earliest-arrival search |
//! | [`defaults`] | Boarding slack, walking limits, bus dwell and PCU |
//! | [`examples`] | The toy network's timetables: the tram `T1` and the bus `B1` |
//!
//! Reading a feed is `io-gtfs`'s job, and linking stops to the walk, bike and
//! road networks is the run's (`core-sim`): this crate knows stops by position
//! only.
//!
//! **Vehicle-based, always** (S20, design §18.1): a run is one vehicle on the
//! service day. Runs that ride the roads (buses) are loaded among the cars and
//! their realised times replace the scheduled ones; the others run by the
//! schedule.

#![cfg_attr(docsrs, feature(doc_cfg))]

pub mod date;
pub mod defaults;
pub mod examples;
pub mod kind;
pub mod raptor;
pub mod timetable;

pub use date::{ServiceDate, Weekday};
pub use defaults::TransitDefaults;
pub use kind::ServiceKind;
pub use raptor::{Footpaths, Journey, JourneyLeg, Raptor, RaptorData};
pub use timetable::{
    ALIGHT, BOARD, CallSpec, CallTimes, RouteSpec, StopSpec, Timetable, TimetableBuilder,
    TimetableReport, Transfer, UNKNOWN_TIME,
};
