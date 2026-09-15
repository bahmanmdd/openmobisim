//! `manifest.json` — the file that makes a run reproducible (Foundations
//! §6), Phase 1's subset of it. See the crate docs for exactly which
//! Foundations §6 fields are not here yet, and why.

use std::fs;
use std::path::Path;

use openmobisim_core_demand::Travellers;
use openmobisim_core_sim::RunResult;
use openmobisim_core_types::ids::{EntityId, TravellerId};
use openmobisim_core_types::time::Second;

use crate::WriteError;

/// What Phase 1 knows about a run, in the shape that becomes
/// `manifest.json`.
#[derive(Clone, Debug)]
pub struct Manifest {
    /// The `openmobisim` crate version (`CARGO_PKG_VERSION`).
    pub openmobisim_version: String,
    /// `openmobisim_core_types::CODE_VERSION` — bumped whenever an artifact's
    /// layout or build semantics change (Foundations §8).
    pub code_version: u32,
    /// `openmobisim_core_graph::DEFAULTS_VERSION` — bumped whenever the OSM
    /// defaults table changes.
    pub defaults_version: u32,
    /// The target triple this binary was built for (`crate::TARGET`). S60's
    /// determinism guarantee is same-platform; this is the platform.
    pub platform: String,
    /// Trips still in progress after this second are truncated (S57).
    pub window_seconds: u32,
    /// The scenario's `traveller_weight` (S89): 1 for `default`, 10 for
    /// `fast`, for a trip no row gives an explicit weight.
    pub default_weight: u32,
    /// How many travellers this run's demand named.
    pub simulated_travellers: u32,
    /// How many of them own a car — Phase 1's only mode, so this is also
    /// the most a run could ever simulate.
    pub car_owning_travellers: u32,
    /// Every trip in the demand, mirroring
    /// [`TripCompletionStats::total_trips`](openmobisim_core_sim::TripCompletionStats::total_trips).
    pub total_trips: u32,
    /// How [`RunResult::total_travel_time`] was computed — always
    /// `"traveller_weight_scaled"` today. Recorded here, not just in a code
    /// comment, because the manifest is the file an analyst reads to know
    /// what a number means (confirmed by the user, 2026-09-14: both a
    /// weighted and an unweighted form will be reported once comprehensive
    /// KPIs exist, so this field is what tells a reader which one a given
    /// `kpis.parquet` row is).
    pub kpi_weighting: String,
}

impl Manifest {
    /// Build a manifest from a finished run.
    ///
    /// `default_weight` is the same value passed to
    /// `core_demand::build_travellers` — this module has no way to recover
    /// it from `travellers` alone, since a traveller whose weight was
    /// explicitly stated never needed it.
    #[must_use]
    pub fn for_run(
        travellers: &Travellers,
        result: &RunResult,
        window: Second,
        default_weight: u32,
    ) -> Self {
        let car_owning_travellers = (0..travellers.len())
            .filter(|&raw| travellers.ownership(TravellerId::new(raw)).car)
            .count();
        #[allow(
            clippy::cast_possible_truncation,
            reason = "bounded by travellers.len(), itself a u32"
        )]
        let car_owning_travellers = car_owning_travellers as u32;

        Self {
            openmobisim_version: openmobisim_core_types::VERSION.to_string(),
            code_version: openmobisim_core_types::CODE_VERSION,
            defaults_version: openmobisim_core_graph::DEFAULTS_VERSION,
            platform: crate::TARGET.to_string(),
            window_seconds: window.get(),
            default_weight,
            simulated_travellers: travellers.len(),
            car_owning_travellers,
            total_trips: result.completion.total_trips,
            kpi_weighting: "traveller_weight_scaled".to_string(),
        }
    }

    /// Render as JSON.
    ///
    /// Hand-written rather than pulled from `serde_json`: every field here
    /// is a version string, a platform triple or a count this crate itself
    /// produced, never arbitrary user text, so the escaping a general JSON
    /// serializer buys is not needed — matching `core-types`' own "depends
    /// on nothing" reasoning for its hand-written `Display` impls.
    #[must_use]
    pub fn to_json(&self) -> String {
        format!(
            "{{\n  \"openmobisim_version\": \"{}\",\n  \"code_version\": {},\n  \
             \"defaults_version\": {},\n  \"platform\": \"{}\",\n  \"window_seconds\": {},\n  \
             \"default_weight\": {},\n  \"simulated_travellers\": {},\n  \
             \"car_owning_travellers\": {},\n  \"total_trips\": {},\n  \"kpi_weighting\": \"{}\"\n}}\n",
            escape(&self.openmobisim_version),
            self.code_version,
            self.defaults_version,
            escape(&self.platform),
            self.window_seconds,
            self.default_weight,
            self.simulated_travellers,
            self.car_owning_travellers,
            self.total_trips,
            escape(&self.kpi_weighting),
        )
    }
}

/// Escape the two characters that would break a JSON string.
fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Write `manifest.json` for one run.
///
/// # Errors
///
/// [`WriteError`] if the file cannot be created or written.
pub fn write_manifest(path: impl AsRef<Path>, manifest: &Manifest) -> Result<(), WriteError> {
    fs::write(path, manifest.to_json())?;
    Ok(())
}
