//! Reading `trips.parquet` — the core demand interface (S97).
//!
//! This module reads the file into [`RawTrip`] rows, in file order,
//! unsorted and unvalidated against the trip-chain rule (S128) — that
//! validation needs every trip of a traveller gathered and ordered first, so
//! it lives in [`crate::travellers`], the module that does the gathering.

use std::path::Path;

use openmobisim_core_graph::geometry::LonLat;
use openmobisim_core_types::time::Second;

use crate::DemandError;
use crate::columns;

/// One row of `trips.parquet`, read but not yet validated.
///
/// Matches the interface's schema table exactly:
/// `traveller_id, trip_seq, origin_lon, origin_lat, destination_lon,
/// destination_lat, departure_time_s, user_class, weight`, and the optional
/// `mode` (S195). Columns beyond
/// these — "carried as attributes for choice models" — are not read here;
/// see the crate docs for why.
#[derive(Clone, Debug)]
pub struct RawTrip {
    /// External traveller id, string or integer in the file, always a string
    /// here (Foundations §1: external ids live in a side table, never in a
    /// hot array).
    pub traveller_id: String,
    /// Order within the traveller's day. Trip *n*+1 starts where trip *n*
    /// ended, enforced by [`crate::travellers::build`], not here.
    pub trip_seq: u32,
    /// WGS84 origin, as stated in the file — before S128's correction.
    pub origin: LonLat,
    /// WGS84 destination.
    pub destination: LonLat,
    /// Seconds from `period.start`.
    pub departure_time: Second,
    /// Must match a declared user class; not validated against one here —
    /// the declared class set is scenario schema (brief §6a), not this
    /// crate's concern.
    pub user_class: String,
    /// Optional per-traveller weight override. The interface documents this
    /// as a property of the *traveller*, so a traveller whose trip rows
    /// disagree on it is a data-quality condition
    /// ([`crate::travellers::codes::INCONSISTENT_TRAVELLER_WEIGHT`]), not a
    /// per-trip value — [`crate::travellers::build`] takes the first row's.
    pub weight: Option<u32>,
    /// The trip's mode (S195), if the file states one: a trip without one is a
    /// car trip until mode choice exists.
    pub mode: Option<crate::Mode>,
}

/// Read every row of `trips.parquet`, in file order.
///
/// # Errors
///
/// [`DemandError`] if the file cannot be opened or decoded, or a required
/// column is missing or of a type this reader does not handle.
pub fn read_trips_parquet(path: impl AsRef<Path>) -> Result<Vec<RawTrip>, DemandError> {
    let batches = columns::read_batches(path.as_ref())?;
    let mut trips = Vec::new();
    for batch in &batches {
        let traveller_id = columns::required_id(batch, "traveller_id")?;
        let trip_seq = columns::required_u32(batch, "trip_seq")?;
        let origin_lon = columns::required_f64(batch, "origin_lon")?;
        let origin_lat = columns::required_f64(batch, "origin_lat")?;
        let destination_lon = columns::required_f64(batch, "destination_lon")?;
        let destination_lat = columns::required_f64(batch, "destination_lat")?;
        let departure_time_s = columns::required_u32(batch, "departure_time_s")?;
        let user_class = columns::required_id(batch, "user_class")?;
        let weight = columns::optional_u32(batch, "weight")?;
        let modes = columns::optional_string(batch, "mode")?
            .map(|column| {
                column
                    .into_iter()
                    .map(|value| value.map(|v| crate::Mode::from_name(&v)).transpose())
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()?;

        for i in 0..batch.num_rows() {
            trips.push(RawTrip {
                traveller_id: traveller_id[i].clone(),
                trip_seq: trip_seq[i],
                origin: LonLat::new(origin_lon[i], origin_lat[i]),
                destination: LonLat::new(destination_lon[i], destination_lat[i]),
                departure_time: Second(departure_time_s[i]),
                user_class: user_class[i].clone(),
                weight: weight.as_ref().and_then(|w| w[i]),
                mode: modes.as_ref().and_then(|m| m[i]),
            });
        }
    }
    Ok(trips)
}
