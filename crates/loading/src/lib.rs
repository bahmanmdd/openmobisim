//! Loading: cars on the point queue, bikes and walkers at free flow; the first-in-first-out link
//! time reading.
//!
//! | Module | What it holds |
//! |---|---|
//! | [`input`] | the vehicles given to a loading (trip, mode, route, departure) and the errors that refuse them |
//! | [`point_queue`] | [`point_queue::load`]: cars on the point queue (Vickrey 1969), bikes and walkers at free flow, over one window |
//! | [`link_bins`] | the table of what entered each link in each bin of the window, recorded at entry |
//! | [`link_times`] | the first-in-first-out reading of that table: the time to cross a link when entering it at an instant |
//!
//! A bike and a walker travel at their mode's free-flow speed on every link and interact with
//! nothing: their times are the same whether there are cars or not. Transit, which runs on its
//! timetable, is not loaded here yet. Units are metric: metres, seconds and vehicles per second.

pub mod input;
pub mod link_bins;
pub mod link_times;
pub mod point_queue;
