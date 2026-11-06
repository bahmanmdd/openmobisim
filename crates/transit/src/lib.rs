//! Transit: the timetable of one service day and the earliest-arrival journey over it.
//!
//! | Module | What it holds |
//! |---|---|
//! | [`timetable`] | [`timetable::Timetable`]: stops by position, runs by their calls, footpaths given in seconds; runs on the same stops grouped into patterns whose runs never overtake (runs that do go to patterns of their own) |
//! | [`raptor`] | [`raptor::Raptor::earliest`]: the journey of earliest arrival, then fewest rides, by RAPTOR (Delling, Pajor and Werneck 2015); [`raptor::earliest_many`]: many queries in parallel |
//!
//! The crate knows stops by position only: which stops a traveller can walk to, and how long
//! the walks take, arrive with the query. Nothing here is a bike: a bike-and-ride journey is
//! an access by bike to the same query. Units are metric: seconds and metres.

pub mod raptor;
pub mod timetable;
