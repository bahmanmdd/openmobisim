//! Reading `trips.parquet` — the core demand interface.
//!
//! This module reads the file into [`RawTrip`] rows, in file order,
//! unsorted and unvalidated against the trip-chain rule — that
//! validation needs every trip of a traveller gathered and ordered first, so
//! it lives in [`crate::travellers`], the module that does the gathering.

use std::collections::HashMap;
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
/// `mode`. Columns beyond
/// these — "carried as attributes for choice models" — are not read here;
/// see the crate docs for why.
#[derive(Clone, Debug)]
pub struct RawTrip {
    /// External traveller id, string or integer in the file, always a string
    /// here (external ids live in a side table, never in a
    /// hot array).
    pub traveller_id: String,
    /// Order within the traveller's day. Trip *n*+1 starts where trip *n*
    /// ended, enforced by [`crate::travellers::build`], not here.
    pub trip_seq: u32,
    /// WGS84 origin, as stated in the file — before the correction.
    pub origin: LonLat,
    /// WGS84 destination.
    pub destination: LonLat,
    /// Seconds from `period.start`.
    pub departure_time: Second,
    /// Must match a declared user class; not validated against one here —
    /// the declared class set is scenario schema, not this
    /// crate's concern.
    pub user_class: String,
    /// Optional per-traveller weight override. The interface documents this
    /// as a property of the *traveller*, so a traveller whose trip rows
    /// disagree on it is a data-quality condition
    /// ([`crate::travellers::codes::INCONSISTENT_TRAVELLER_WEIGHT`]), not a
    /// per-trip value — [`crate::travellers::build`] takes the first row's.
    pub weight: Option<u32>,
    /// The trip's mode, if the file states one: a trip without one is a
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

/// Check that no two rows share a `(traveller_id, trip_seq)` (X-68).
///
/// A trip is keyed by the pair, so a second row with the same key would replace or drop the
/// first, and every total of the run (travel time, mode shares, completion) would be of a
/// smaller demand than the user gave, with nothing to say so. **Cost:** one hash set over the
/// rows, once; nothing in a loading.
///
/// # Errors
///
/// [`DemandError::Schema`] naming the first row (from 0, in the order given) that repeats a key,
/// the row it repeats, the key, and how many rows repeat a key in all.
pub fn check_distinct_keys(trips: &[RawTrip]) -> Result<(), DemandError> {
    // Each key's first row, so the message can name the row a repeat repeats.
    let mut first: HashMap<(&str, u32), usize> = HashMap::with_capacity(trips.len());
    let mut named: Option<(usize, usize)> = None;
    let mut repeats = 0_usize;
    for (i, t) in trips.iter().enumerate() {
        if let Some(&earlier) = first.get(&(t.traveller_id.as_str(), t.trip_seq)) {
            repeats += 1;
            named.get_or_insert((i, earlier));
        } else {
            first.insert((t.traveller_id.as_str(), t.trip_seq), i);
        }
    }
    let Some((i, earlier)) = named else { return Ok(()) };
    let t = &trips[i];
    Err(DemandError::Schema(format!(
        "trip row {i} (from 0) repeats row {earlier}'s (traveller_id, trip_seq) = ({:?}, {}); \
         {repeats} row(s) repeat a key already given. A traveller's trips take distinct trip_seq \
         values (0, 1, 2, … in the order of their day), and another traveller takes another \
         traveller_id.",
        t.traveller_id, t.trip_seq
    )))
}
