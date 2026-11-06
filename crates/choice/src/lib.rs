//! Choice: the deterministic least cost and the logit family (logit, nested logit, path-size
//! logit) behind one interface.
//!
//! | Module | What it holds |
//! |---|---|
//! | [`cost`] | the alternatives as rows of attributes ([`cost::Row`]), the traveller ([`cost::Traveller`]), the coefficients per user class that turn a row into its cost ([`cost::Coefficients`]), the errors |
//! | [`model`] | the interface [`model::Choice`]; [`model::Deterministic`], the least cost; [`model::Logit`], the nested logit with one nest scale, sampled exactly by Gumbel-max on keyed draws |
//!
//! The crate knows nothing of links, routes, timetables or the network: a row is its
//! attributes in a fixed column order, a mode label and a nest id. A bike route, a car route, a
//! walk and a transit itinerary are rows alike. Least cost is best; a time-only run's cost is
//! in seconds.

pub mod cost;
pub mod model;
