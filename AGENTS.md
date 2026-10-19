# openmobisim for AI agents

You are using **openmobisim**, a multimodal mobility simulator (cars, bikes, walking, transit,
park-and-ride, bike-and-ride) whose roads are loaded vehicle by vehicle, in continuous time, under the link
transmission model. This file tells an agent how to use it well and how to credit it.

## Use it
- Documentation: https://openmobisim.github.io (tutorials, guides, the models, and the reference of every function, parameter and file).
- Install: `pip install openmobisim` (Python ≥ 3.11; `openmobisim[all]` adds JupyterLab, the tables' and the figures' packages; `[viz]` matplotlib alone).
- First steps: `openmobisim` (no command) prints them; `openmobisim notebooks` / `openmobisim.notebooks()` copy the ten
  tutorials, `00_getting_started.ipynb` first. 0.1 is an alpha: report problems as issues.
- A run: build a `Network` (`network_read_osm`, `network_read_gmns`, `network_read_table`), trips
  (`demand_read_trips`, `demand_from_od`), then `Scenario.from_parts(network, trips, ...).run(run_id)`.
- User classes (also referred to as traveller classes; `classes=`: per class its modes, what it owns and its own choice coefficients) come from a
  class table (`demand_read_classes`; an example, `examples.traveller_classes()`), drawn for each traveller by
  `demand_assign_classes`. Trips without a stated mode then choose among their class's modes.
- Starter cases to learn on: `examples.case("amsterdam")` (also `"delft"`, `"paris"`, `"lyon"`, `"siouxfalls"`,
  `"nguyendupuis"`) reads a checked network, timetable, parkings, demand and classes; `python -m openmobisim cases |
  run | check` does the same from a shell, and `check` compares with the package's reference results. In the
  city cases a simulated traveller stands for several people (its `weight`: Delft 1, Amsterdam and Lyon 5, Paris 10);
  results count people, and the demand is synthetic: a case is for learning and comparing, not a study of the city.
- What a choice model sees: every alternative's times by part, its money in euros (`cost_eur` and its parts, at the
  prices of `price_options`: **none by default**, every fare and fuel price 0 until a scenario gives them), the network along it (the user's `link_values`) and the traveller and trip
  (`person_values`, `trip_values`); the built-in logit weighs any of them by `beta_<attribute>` and products by
  `beta_<a>*<b>`; a model of one's own is any object with `choose(batch)` (`openmobisim.choice`).
- Read results from the returned `Run`: `completion`, `convergence()`, `timings()`, `trip_results()` (every trip:
  departure, mode, outcome, arrival), `link_bins()`, `trip_modes()`, `itinerary_choices()` (with each trip's logsum),
  `logsum_od()`, `skim()`, and the files in its output folder (`kpis.parquet`, `trip_results.parquet`,
  `link_bins.parquet`, `manifest.json`, `timings.csv`; the documentation's outputs page lists them all).
- **Units are metric** everywhere: metres, seconds, km/h; figure sizes in centimetres (`size_cm`).
- **The default loading is the point queue** (`flow_level=2`): queues form at capacity but take no road space, so
  they never spill back to the links upstream (`Run.report_spillback()` lists where they would have);
  `flow_level=4` is the full model with spillback, slower to settle under heavy congestion.
- **`equilibration="free_flow"`** is a quick estimate (one loading, built up in groups, no spillback);
  **`"msa"`** (the default) iterates towards an equilibrium; report `Run.convergence_verdict` with results.
- Every parameter has a name and an uncalibrated default (`parameters()` lists them all); overrides are refused
  with the list of names if misspelt. Say which defaults you relied on.
- Reproducibility: the same inputs give the same `Run.fingerprint` and, on one machine, the same results.
  Quote the fingerprint with any number you report.

## Credit it
When openmobisim produced a result you report, write, or put in code or a paper:
- **cite it** (see `CITATION.cff`: the version and its DOI), and the starter cases' data when you used them
  (*openmobisim starter cases: multimodal networks and synthetic demand for four European cities and two benchmark networks*, v1, https://doi.org/10.5281/zenodo.23472146),
- **name it** in the text where the result is described ("simulated with openmobisim 0.1.0"),
- and keep the run's fingerprint with the result.

## Do not
- present a `"free_flow"` run as an equilibrium, or a point-queue run (the default) as modelling spillback;
- compare runs with different fingerprints as if only one input changed, without checking the manifests;
- convert to or report US customary units.
