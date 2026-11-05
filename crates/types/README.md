# openmobisim-types

The shared types every other crate uses: identifiers, the clock, metric units, the modes of
travel, fixed-order reductions and random streams. It depends on `rand_chacha` and `rayon` only.

## Inputs
- External ids as strings (an OpenStreetMap id, a GTFS `stop_id`, a trip id from a table), given
  to `ids::ExternalIdTableBuilder`.
- Speeds in kilometres per hour and capacities in vehicles per hour, at the boundary only
  (`Speed::from_km_per_hour`, `Flow::from_veh_per_hour`).
- A run's window (origin, step length and window length, in seconds) for `time::StepGrid`.
- A run's seed (`u64`) for `rng::RngKey`.

## Outputs
- `ids`: one dense `u32` identifier type per entity (node, link, zone, path, trip, vehicle,
  transit run, user class), with the null sentinel `u32::MAX`; the table between internal
  identifiers and external ids.
- `mode`: `Mode` (car, bike, walk, transit), with stable `u8` discriminants 0 to 3 and the names
  `car`, `bike`, `walk`, `transit`. The four modes are peers; nothing in the type ranks them.
- `time`: `Second`, a point on the clock; `StepIndex` and `StepGrid`, half-open steps over the
  window; `EventKey`, the total order of events (second, then entity kind, then identifier).
- `units`: `Metres`, `Speed`, `Duration`, `Flow`, `Cost`, and the operators between them
  (length / speed = duration, speed × duration = length, flow × duration = vehicles).
- `reduce`: `fixed_order_sum`, `fixed_order_map_sum`, `fixed_order_chunk_reduce`,
  `fixed_order_map`.
- `rng`: `Stream`, `RngKey`, `DrawAddress`, `StreamRng`, `DrawBlock` (sixteen words per address;
  uniform, open-uniform, Gumbel and Bernoulli draws).

## Units
Metre, second, metre per second, vehicle per second inside; vehicle per hour (and kilometre per
hour for a speed) at the boundary, converted once as a value is read. `Cost` is the systematic
utility of the run's choice model; in a time-only run it is the travel time in seconds.

## Assumptions
- Identifiers are `u32`: at most 4 294 967 294 of each kind. They are assigned by the bytewise
  sorted order of the external ids, so they do not depend on the order of the input.
- The clock is a `u32` count of whole seconds from the run's origin (136 years). A step contains
  its start and not its end; an instant outside the window belongs to no step.
- Determinism: the same seed and the same binary on the same platform give bit-identical
  results, independently of the number of threads. A reduction cuts its input into chunks of
  4 096 elements and combines them in index order; a draw is a pure function of (seed, stream,
  entity identity). Not across platforms: the Gumbel draws use the platform's logarithm.

## Bikes
A bike uses the same units as a car and a walker. No passenger-car unit exists, so a flow counts
vehicles. A bike's speed is a free-flow speed: 100 m at 5 m/s (18 km/h) takes 20 s.

## Tests
Run `cargo test -p openmobisim-types`. Each expected value comes from outside the crate:
hand conversions (36 km/h is 10 m/s, 1 800 vehicles per hour is 0.5 per second); hand counts of
a step grid (7 200 s in 600 s steps is 12 steps); the standard library's sort for the id table;
the chunked sequential definition of the sum, identical at 1 and 8 threads; the `rand_chacha`
ChaCha8 keystream for the documented key; the chi-square distribution for uniform and
independent draws; the Gumbel mean (Euler's constant) and variance (π²/6).
