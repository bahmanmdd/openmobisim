//! What the run's pages need from a finished run, as plain data read from its files and its
//! demand: per trip its doors, its departure and its user class (the demand's), the mode it took,
//! its realised door-to-door time and whether it finished (`trips.parquet`), the legs it took
//! (`routes.parquet`); the user classes' ids; and the rows of the links table (`links.parquet`).
//! A trip the run did not run (no row in `trips.parquet`: dropped, outside its window) is not
//! kept. Read-only once built.

use openmobisim_demand::demand::Demand;
use openmobisim_graph::geometry::Xy;
use openmobisim_io_parquet::{Links, RouteLeg, Routes, TripRows};
use openmobisim_types::ids::{EntityId, TripId};
use openmobisim_types::mode::Mode;
use openmobisim_types::time::{Second, StepGrid};

/// One trip as the run's pages read it.
#[derive(Clone, Debug)]
pub struct Trip {
    /// Where it starts, metres in the network's plane: its door.
    pub origin: Xy,
    /// Where it ends: its other door.
    pub destination: Xy,
    /// Its departure, a second of the clock.
    pub departure: Second,
    /// Its user class, by its place in [`Kept::classes`].
    pub class: u32,
    /// The mode it took; none for a trip without an alternative.
    pub mode: Option<Mode>,
    /// The legs it took, in order (`routes.parquet`); none without an alternative; `None` when it
    /// is left out of the journeys (its routes do not agree with the trips table).
    pub legs: Option<Vec<RouteLeg>>,
    /// Door to door, seconds, as realised; meaningful only when it finished.
    pub travel_time: f64,
    /// Whether it reached its door before the window's end.
    pub finished: bool,
}

/// What the run's pages need from a finished run (see the module documentation), built by
/// [`Kept::of`] and read by [`crate::write`] and the summary page.
#[derive(Clone, Debug)]
pub struct Kept {
    pub(crate) trips: Vec<Trip>,
    pub(crate) classes: Vec<String>,
    /// The run's window, in the bins of its links table.
    pub(crate) window: StepGrid,
    pub(crate) rows: Links,
}

impl Kept {
    /// The pages' part of a run of `demand` over `window`: its trips table, its routes (per
    /// trip, in the demand's order, `None` for a trip left out) and its links table, as
    /// `openmobisim_io_parquet::read` reads them back; the trips without a row left out.
    ///
    /// # Panics
    ///
    /// When `trips` or `routes` has fewer trips than `demand`: not the run of this demand.
    #[must_use]
    pub fn of(
        demand: &Demand,
        trips: &TripRows,
        routes: Routes,
        links: Links,
        window: StepGrid,
    ) -> Self {
        let trips = routes
            .into_iter()
            .enumerate()
            .filter(|&(i, _)| trips.present[i])
            .map(|(i, legs)| {
                let trip = TripId::from_index(i);
                Trip {
                    origin: demand.origin(trip),
                    destination: demand.destination(trip),
                    departure: demand.departure(trip),
                    class: demand.class(trip).raw(),
                    mode: trips.mode[i],
                    legs,
                    travel_time: trips.travel_time[i],
                    finished: trips.finished[i],
                }
            })
            .collect();
        let classes = demand.class_ids().iter().map(str::to_owned).collect();
        Self { trips, classes, window, rows: links }
    }

    /// How many trips are left out of the journeys: their routes do not agree with the trips
    /// table.
    #[must_use]
    pub fn left_out(&self) -> usize {
        self.trips.iter().filter(|t| t.legs.is_none()).count()
    }

    /// How many trips.
    #[must_use]
    pub fn trip_count(&self) -> usize {
        self.trips.len()
    }

    /// The trips, in trip id order.
    #[must_use]
    pub fn trips(&self) -> &[Trip] {
        &self.trips
    }

    /// The user classes' external ids, in class id order.
    #[must_use]
    pub fn classes(&self) -> &[String] {
        &self.classes
    }

    /// The run's window, in the bins of its links table.
    #[must_use]
    pub fn window(&self) -> StepGrid {
        self.window
    }

    /// The rows of the links table.
    #[must_use]
    pub fn rows(&self) -> &Links {
        &self.rows
    }
}
