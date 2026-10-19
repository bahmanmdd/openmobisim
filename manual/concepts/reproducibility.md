# Reproducibility

A run of openmobisim can be repeated exactly. The same inputs and settings on the same platform give the same
results, bit for bit, however many processor cores the run uses. Every run also records what went into it, so you can
tell whether two sets of results came from the same inputs.

Three things make this work:
- **the master seed**, from which every random draw is derived;
- **the run fingerprint**, a short code computed from every input that can change the results;
- **`manifest.json`**, the file that records the fingerprint next to the settings, the versions and the platform.

## The master seed and the random streams

% src: crates/core-types/src/rng.rs (module docs, Stream); python/openmobisim/scenario.py Scenario(master_seed=)
Every random number a run uses comes from one number, the master seed (`Scenario(master_seed=...)`, 0 by default). A
draw is not taken from a generator that advances one step at a time. It is computed from its **address**: the master
seed, the stream it belongs to, and the identity of what it is for. The streams a run draws from today:

| Stream | Used for | Addressed by |
|---|---|---|
| `choice` | Every random choice: route, itinerary, mode, when the choice model samples | traveller, trip, iteration, alternative |
| `msa_reselection` | Which travellers choose again at an iteration of `msa` | traveller, iteration |

`manifest.json` lists the streams a run drew from (`live_streams`). The deterministic choice model draws nothing; with
it, the master seed changes only the fingerprint.

Because a draw depends only on its address, it does not depend on the order in which draws are made, on how the work
is split between cores, or on whether some other feature is switched on. Adding or removing one alternative leaves the
draws of all the others unchanged.

**Common random numbers.** Comparing two scenarios that differ only in a design (a new road, a new bus line) with the
same master seed gives every traveller the same draws in both, wherever their alternatives are the same. The
difference between the results is then the design's, not the luck of the draw. For replications, change the master
seed.

### Draws in Python

% src: python/openmobisim/demand.py _draw, _key, demand_sample, demand_assign_classes, demand_from_od
The demand helpers draw too, each with its own `seed` argument:
- {py:func}`openmobisim.demand_sample` and {py:func}`openmobisim.demand_assign_classes` order the travellers by a hash
  of the seed and the traveller's id;
- {py:func}`openmobisim.demand_from_od` uses one generator seeded by `seed`, consumed in a fixed order of the matrix's
  cells.

The hashed orders are the same on any machine; `demand_from_od`'s departures and places are the same for one NumPy
version (NumPy does not promise the same stream across its releases): keep the trips, not only the seed.

## The run fingerprint

% src: crates/core-sim/src/identity.rs (describe, hash_network, hash_modes, hash_mode_choice, hash_transit, hash_parking)
The fingerprint is a 64-bit hash (FNV-1a), written as 16 hexadecimal digits, of every input that can change the
results. It is computed before the run starts. Two runs with the same fingerprint were given the same inputs. On the
same platform they produce the same numbers.

**What it covers.** Each item below enters the hash. An item that a run does not use (no timetable, no parkings, no
mode choice) is left out entirely, so adding a feature does not change the fingerprints of runs without it.

| Group | What is hashed |
|---|---|
| Code | The code version (`CODE_VERSION`: raised whenever the layout or the meaning of what a run builds changes) |
| Seed | The master seed |
| Network | Every node (position, signal) and link (ends, class, lanes, roundabout, length, free-flow time, storage, speed, capacity, jam density, wave speed, control delay, signal per direction), closed links, and the network options that differ from their defaults |
| Demand | Every traveller (id, class, weight, what they own) and every trip (traveller, departure, origin, destination) |
| Window | Its length and its start |
| Loading | The flow level, the time step, the bin length of the per-link results, and the loading rules (priority, rerouting with its options, turn pockets with their length) |
| Methods | The route method, the choice model, the equilibration strategy and the route update, each with every option; the choice-set limits (detour limit, most routes offered) |
| Modes | Each trip's mode and the layers trips use (their graphs, speeds and search costs); with mode choice, the modes offered, which trips state their mode, the walk and ride limits, the modes and limits of each class; the car fallback, when it is on |
| Your values | The per-link and per-traveller or per-trip values a choice model reads |
| Prices | Every price, when the choice model reads money |
| Disruptions | Every road closure and transit cancellation or delay, with its times, and whether travellers know of it in advance |
| Transit | The service day, every stop, run and call, the transfers, the stops' links to the walk network, and every transit option |
| Parkings | Every parking (position, kind, capacity, initial occupancy, fees, nearby stops) and the parking options |

**What it does not cover**, and why:
- **The platform and the package version.** The manifest records them next to the fingerprint: the fingerprint says
  *what was run*, the manifest *what ran it*.
- **The shape of the streets.** Only the figures use it.
- **The turn table.** It follows from the network and the code.
- **The number of processor cores.** It cannot change the results (below).
- **`DEFAULTS_VERSION`.** The counter of the shipped defaults' versions is recorded in the manifest, not hashed. A
  changed default still changes the fingerprint wherever it changes an input: the network's link parameters, a
  method's options and the run's limits are hashed as values.

The fingerprint is shown on the summary page and in the figures' footers, and is written into `manifest.json` and
`link_bins.parquet`'s metadata, so a figure or a table can be traced to the run that made it. It is also how
`openmobisim check` tells "the same inputs, different numbers" (a problem) from "other inputs" (expected).

## `manifest.json`

% src: crates/io-parquet/src/manifest.rs (Manifest)
Every run with an output folder writes `manifest.json`. It holds:
- the fingerprint and the network's own fingerprint;
- the package version, the code version, `DEFAULTS_VERSION` and the platform;
- the master seed and the streams drawn from;
- every method with its options in canonical form;
- the window, and the trips and people left out because they depart outside it;
- the demand's totals;
- whether the run converged, and after how many iterations.

Every key is listed in [Output files](../reference/outputs.md).

Asking for outputs does not change a run's results: `link_bin_s` sets the bins of the flow map alone, while the times
the next iteration's choices read are always binned at the strategy's `cost_bin_s`, whether or not a flow map was
asked for.

## Independent of the number of cores

% src: crates/core-types/src/reduce.rs; crates/core-types/src/rng.rs; .github/workflows/ci.yml (the determinism job)
The core runs its parallel work on all the processor's cores. Two rules make the results independent of how many:
- **Sums over many items are taken in a fixed order.** A parallel sum of floating-point numbers can differ in its
  last digits depending on how the work was split, because floating-point addition is not associative. Here the items
  are split into chunks of a fixed size, whatever the data, the machine or the number of threads. Each chunk is summed
  in order, and the chunks' sums are combined in order.
- **Random draws are addressed by identity**, never by thread (above).

**How this is checked.** Every change to the code is tested with the same binary:
- run twice;
- run with 1 thread and with 7;
- built without parallelism at all.

The outputs must agree byte for byte. This covers the reductions, and a whole run from demand to the written files,
including a run whose route sets grow between iterations.

% src: python/openmobisim/scenario.py Scenario.run(threads=); crates/io-parquet/src/manifest.rs threads;
% python/tests/test_threads.py
**Setting the number of cores.** A run uses every logical core unless you say otherwise:
`scenario.run("name", threads=4)`. The run's parallel work then uses a pool of its own of that size. The number is
recorded in `manifest.json` (`threads`) and is not part of the fingerprint. The package's tests run a multimodal case
with 1 and 4 threads, and the Delft starter case with 1 thread and with every core, and require every result file to
be identical.

**A test that the fingerprint is complete.** On a multimodal scenario (mode choice, classes, transit, parkings,
prices, equilibration, the full loading), the package's tests change every parameter the scenario uses, one at a
time, and require the fingerprint to change each time.

## Across platforms

% src: crates/core-types/src/lib.rs ("Determinism, precisely"); crates/io-parquet/src/manifest.rs (platform);
% python/openmobisim/_cases.py CHECK_TOLERANCE, case_check
The bit-for-bit guarantee holds on one platform (operating system and processor architecture); the manifest records
it, for example `aarch64-apple-darwin`. On another platform the same inputs may give results that differ slightly: the
exponential and logarithm functions of each platform's mathematics library differ in their last bits, and the logit
model and its random draws use them, so a choice that is a near tie can go the other way.
`openmobisim check` therefore compares a starter case's run with its reference within tolerances when the platforms
differ:

| Compared | Tolerance |
|---|---|
| Trips completed | 0.5 % of the reference's |
| Mean trip time | 1 % |
| Each mode's share of the people | 0.5 percentage points |

Its verdicts are `same` (same platform, same numbers), `same numbers (another platform)`, `within tolerance` and
`different`.

## For bikes

Bike trips use the same streams and the same fingerprint as every other mode. The bike layer's graph, speeds and search
costs are hashed whenever a trip rides or may choose to ride. Change the cycling network or the bike cost, and the
fingerprint changes.

## Limits

% src: crates/core-types/src/rng.rs (RngKey: design_id, replication); crates/io-parquet/src/lib.rs
- A design space with replications is run as a loop of runs, each labelled with its `design_id` and `replication`;
  a replication is made by changing the master seed, and the random-number key's own design and replication parts
  stay 0. There is no batch runner yet.
- The guarantee is per platform (above).
