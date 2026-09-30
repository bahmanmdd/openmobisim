//! Vehicle-location state (S98) — per-run mutable state, seeded from S129's
//! rule: a traveller's vehicles start at the origin of their first trip, and
//! that location is S46's "base".
//!
//! Deliberately separate from [`crate::travellers::Travellers`]: the trips
//! table is immutable shared input, vehicle location is not (Foundations
//! §5). Nothing in this module decides which alternative a traveller takes —
//! see the crate docs for what is and is not built yet.

use openmobisim_core_graph::geometry::LonLat;
use openmobisim_core_types::ids::{EntityId, TravellerId};

use crate::travellers::Travellers;
use crate::travellers::Trips;

/// Which owned vehicle a location is tracked for.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum VehicleKind {
    /// A private car.
    Car,
    /// A private bike.
    Bike,
}

/// Per-traveller, per-vehicle-kind location.
///
/// Structure-of-arrays, one slot per traveller per kind regardless of
/// ownership — cheaper than a sparse map at this size, and
/// [`Travellers::ownership`] is always the authority on whether a slot means
/// anything; a non-owner's slot is never read by [`location`](Self::location)
/// or [`is_at_origin`](Self::is_at_origin).
///
/// **A vehicle can be parked at a hub** (M4, S201): its location is then the
/// parking's position, and [`Self::parking`] says which parking, so the trip
/// back finds it there (design §23.2).
#[derive(Clone, Debug, Default)]
pub struct VehicleLocations {
    car: Vec<LonLat>,
    bike: Vec<LonLat>,
    /// Per traveller, the parking each vehicle is in, or [`NOT_PARKED`].
    car_parking: Vec<u32>,
    bike_parking: Vec<u32>,
}

/// A vehicle not in a parking.
pub const NOT_PARKED: u32 = u32::MAX;

impl VehicleLocations {
    /// Seed every owned vehicle at its traveller's base (S129).
    #[must_use]
    pub fn at_first_trip_origin(travellers: &Travellers, trips: &Trips) -> Self {
        let n = travellers.len() as usize;
        let mut car = vec![LonLat::default(); n];
        let mut bike = vec![LonLat::default(); n];
        for raw in 0..travellers.len() {
            let traveller = TravellerId::new(raw);
            let ownership = travellers.ownership(traveller);
            if !ownership.car && !ownership.bike {
                continue;
            }
            let base = trips.origin(travellers.first_trip(traveller));
            if ownership.car {
                car[traveller.index()] = base;
            }
            if ownership.bike {
                bike[traveller.index()] = base;
            }
        }
        Self { car, bike, car_parking: vec![NOT_PARKED; n], bike_parking: vec![NOT_PARKED; n] }
    }

    /// `traveller`'s vehicle of `kind`, or `None` if they do not own one
    /// ([`Travellers::ownership`] is the authority checked here).
    #[must_use]
    pub fn location(
        &self,
        travellers: &Travellers,
        traveller: TravellerId,
        kind: VehicleKind,
    ) -> Option<LonLat> {
        let ownership = travellers.ownership(traveller);
        match kind {
            VehicleKind::Car if ownership.car => Some(self.car[traveller.index()]),
            VehicleKind::Bike if ownership.bike => Some(self.bike[traveller.index()]),
            VehicleKind::Car | VehicleKind::Bike => None,
        }
    }

    /// Move `traveller`'s vehicle of `kind` to `location`.
    ///
    /// Called by whatever applies a chosen alternative — not by this crate
    /// (see the crate docs). Trusts the caller to move only a vehicle
    /// `travellers.ownership` says exists, the same boundary every other
    /// internal call in the core trusts.
    pub fn relocate(&mut self, traveller: TravellerId, kind: VehicleKind, location: LonLat) {
        match kind {
            VehicleKind::Car => {
                self.car[traveller.index()] = location;
                self.car_parking[traveller.index()] = NOT_PARKED;
            }
            VehicleKind::Bike => {
                self.bike[traveller.index()] = location;
                self.bike_parking[traveller.index()] = NOT_PARKED;
            }
        }
    }

    /// Park `traveller`'s vehicle of `kind` in `parking`, which is at `location`
    /// (M4). The same trust as [`Self::relocate`].
    pub fn park(
        &mut self,
        traveller: TravellerId,
        kind: VehicleKind,
        parking: u32,
        location: LonLat,
    ) {
        self.relocate(traveller, kind, location);
        match kind {
            VehicleKind::Car => self.car_parking[traveller.index()] = parking,
            VehicleKind::Bike => self.bike_parking[traveller.index()] = parking,
        }
    }

    /// The parking `traveller`'s vehicle of `kind` is in, if it is in one (and
    /// they own it).
    #[must_use]
    pub fn parking(
        &self,
        travellers: &Travellers,
        traveller: TravellerId,
        kind: VehicleKind,
    ) -> Option<u32> {
        self.location(travellers, traveller, kind)?;
        let p = match kind {
            VehicleKind::Car => self.car_parking[traveller.index()],
            VehicleKind::Bike => self.bike_parking[traveller.index()],
        };
        (p != NOT_PARKED).then_some(p)
    }

    /// Whether `traveller`'s vehicle of `kind` is exactly at `origin` — the
    /// Phase 1 form of design §23.1's rule: **"a vehicle the traveller may
    /// use is reachable from the origin."**
    ///
    /// Exact coordinate match only. After S128's trip-chain correction this
    /// is exactly right for every trip after the traveller's first — the
    /// vehicle was left at the previous trip's destination, which *is* the
    /// next trip's origin by construction. A tolerance or a walking-distance
    /// radius is a routing question this crate has no network to answer;
    /// S96's access correction is the natural home for it once route sets
    /// exist (Phase 2).
    #[must_use]
    pub fn is_at_origin(
        &self,
        travellers: &Travellers,
        traveller: TravellerId,
        kind: VehicleKind,
        origin: LonLat,
    ) -> bool {
        self.location(travellers, traveller, kind) == Some(origin)
    }
}
