//! `manifest.json` — the file that makes a run reproducible (Foundations
//! §6), Phase 1's subset of it. See the crate docs for exactly which
//! Foundations §6 fields are not here yet, and why.

use std::fs;
use std::path::Path;

use openmobisim_core_demand::Travellers;
use openmobisim_core_sim::{RunDescription, RunResult};
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
    /// The scenario's master seed (S168): the one number that starts every
    /// random stream. Under a sampled choice model it decides who takes which
    /// route; under the all-or-nothing default it changes only `run_fingerprint`.
    pub master_seed: u64,
    /// A hash of every input that decides the results (16 hex digits): the
    /// network, the demand, each setting, the route method, the seed, the code
    /// version. Two runs with the same one were given the same inputs.
    pub run_fingerprint: String,
    /// A hash of the network's ids (16 hex digits): what an internal link id in
    /// any of this run's files is meaningful next to.
    pub network_fingerprint: String,
    /// The loading engine's level: 0 for free flow, 2, 3 or 4 for the link
    /// transmission model.
    pub flow_level: u32,
    /// The loading step, in seconds, under the link transmission model.
    pub flow_step_seconds: Option<f64>,
    /// The route method's name.
    pub route_method: String,
    /// The route method with every option and default, canonical.
    pub route_descriptor: String,
    /// The bin length of `link_bins.parquet`, if the run recorded it.
    pub link_bin_seconds: Option<u32>,
    /// The choice model's name (S169).
    pub choice_model: String,
    /// The choice model with every option and default, canonical.
    pub choice_descriptor: String,
    /// The random streams the run drew from: `choice` under a sampled choice
    /// model, `msa_reselection` under an equilibration that iterates.
    pub live_streams: Vec<String>,
    /// The equilibration strategy's name (S170).
    pub equilibration: String,
    /// The strategy with every option and default, canonical.
    pub equilibration_descriptor: String,
    /// The route update's name (S176): `none` if the route sets stayed as the
    /// method made them, otherwise what grew them between iterations.
    pub route_update: String,
    /// The update with every option and default, canonical.
    pub route_update_descriptor: String,
    /// How many loadings the run made.
    pub iterations_run: u32,
    /// Whether the strategy stopped before its most iterations because it had
    /// converged.
    pub converged: bool,
}

impl Manifest {
    /// Build a manifest from a finished run.
    ///
    /// `default_weight` is the same value passed to
    /// `core_demand::build_travellers` — this module has no way to recover
    /// it from `travellers` alone, since a traveller whose weight was
    /// explicitly stated never needed it. `description` is the run's
    /// [`Run::description`](openmobisim_core_sim::Run::description), taken
    /// before it executed.
    #[must_use]
    pub fn for_run(
        travellers: &Travellers,
        result: &RunResult,
        window: Second,
        default_weight: u32,
        description: &RunDescription,
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
            master_seed: description.master_seed,
            run_fingerprint: description.fingerprint_hex(),
            network_fingerprint: description.network_fingerprint_hex(),
            flow_level: description.flow_level,
            flow_step_seconds: description.flow_step_seconds,
            route_method: description.route_method.clone(),
            route_descriptor: description.route_descriptor.clone(),
            link_bin_seconds: description.link_bin_seconds,
            choice_model: description.choice_model.clone(),
            choice_descriptor: description.choice_descriptor.clone(),
            live_streams: description.live_streams.clone(),
            equilibration: description.equilibration.clone(),
            equilibration_descriptor: description.equilibration_descriptor.clone(),
            route_update: description.route_update.clone(),
            route_update_descriptor: description.route_update_descriptor.clone(),
            iterations_run: u32::try_from(result.iterations.len().max(1)).unwrap_or(u32::MAX),
            converged: result.converged,
        }
    }

    /// Render as JSON.
    ///
    /// Hand-written rather than pulled from `serde_json`: every field here
    /// is a version string, a platform triple, a name or a number this crate
    /// itself produced (the route descriptor is the one string that can hold
    /// arbitrary characters, and goes through `escape`), so the escaping a
    /// general JSON serializer buys is not needed — matching `core-types`' own
    /// "depends on nothing" reasoning for its hand-written `Display` impls.
    #[must_use]
    pub fn to_json(&self) -> String {
        let text = |v: &str| format!("\"{}\"", escape(v));
        let optional = |v: Option<String>| v.unwrap_or_else(|| "null".to_string());
        let fields: [(&str, String); 28] = [
            ("openmobisim_version", text(&self.openmobisim_version)),
            ("code_version", self.code_version.to_string()),
            ("defaults_version", self.defaults_version.to_string()),
            ("platform", text(&self.platform)),
            ("window_seconds", self.window_seconds.to_string()),
            ("default_weight", self.default_weight.to_string()),
            ("simulated_travellers", self.simulated_travellers.to_string()),
            ("car_owning_travellers", self.car_owning_travellers.to_string()),
            ("total_trips", self.total_trips.to_string()),
            ("kpi_weighting", text(&self.kpi_weighting)),
            ("master_seed", self.master_seed.to_string()),
            ("run_fingerprint", text(&self.run_fingerprint)),
            ("network_fingerprint", text(&self.network_fingerprint)),
            ("flow_level", self.flow_level.to_string()),
            ("flow_step_seconds", optional(self.flow_step_seconds.map(|s| s.to_string()))),
            ("route_method", text(&self.route_method)),
            ("route_descriptor", text(&self.route_descriptor)),
            ("choice_model", text(&self.choice_model)),
            ("choice_descriptor", text(&self.choice_descriptor)),
            (
                "live_streams",
                format!(
                    "[{}]",
                    self.live_streams.iter().map(|s| text(s)).collect::<Vec<_>>().join(", ")
                ),
            ),
            ("equilibration", text(&self.equilibration)),
            ("equilibration_descriptor", text(&self.equilibration_descriptor)),
            ("route_update", text(&self.route_update)),
            ("route_update_descriptor", text(&self.route_update_descriptor)),
            ("iterations_run", self.iterations_run.to_string()),
            ("converged", self.converged.to_string()),
            ("link_bin_seconds", optional(self.link_bin_seconds.map(|s| s.to_string()))),
            ("link_bins_file", optional(self.link_bin_seconds.map(|_| text("link_bins.parquet")))),
        ];
        let body: Vec<String> =
            fields.iter().map(|(key, value)| format!("  \"{key}\": {value}")).collect();
        format!("{{\n{}\n}}\n", body.join(",\n"))
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
