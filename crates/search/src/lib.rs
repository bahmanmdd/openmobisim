//! Search: the fastest route per mode at the current link times, time-dependent on the
//! first-in-first-out reading.
//!
//! | Module | What it holds |
//! |---|---|
//! | [`search`] | [`search::Search::fastest`]: the route of earliest arrival for one query, by label-setting Dijkstra over nodes with a straight-line bound (A\*); [`search::fastest_many`]: many queries in parallel; [`search::Search::reach`] and [`search::Search::reach_backward`]: every node within a bound of one node, forward or (bike and walk) backward |
//!
//! A bike and a walker cross every link in its length over their mode's speed, whatever the
//! cars do, so their fastest route is their shortest by length. Units are metric: metres and
//! seconds.

pub mod search;
