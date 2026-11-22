# openmobisim-types

The shared types every other crate uses: identifiers, the clock, metric units, the modes of
travel, fixed-order reductions and random streams. It depends on `libm`, `rand_chacha` and
`rayon` only.

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
  `ModeSet`, a set of modes as four bits: the modes allowed on a link, or open to a traveller.
- `time`: `Second`, a point on the clock; `StepIndex` and `StepGrid`, half-open steps over the
  window; `EventKey`, the total order of events (second, then entity kind, then identifier).
- `units`: `Metres`, `Speed`, `Duration`, `Flow`, `Cost`, and the operators between them
  (length / speed = duration, speed × duration = length, flow × duration = vehicles).
- `reduce`: `fixed_order_sum`, `fixed_order_map_sum`, `fixed_order_chunk_reduce`,
  `fixed_order_map`.
- `rng`: `Stream`, `RngKey`, `DrawAddress`, `StreamRng`, `DrawBlock` (sixteen words per address;
  uniform, open-uniform, Gumbel and Bernoulli draws).
- `fingerprint`: `Fingerprint`, the 64-bit FNV-1a hash (offset basis `0xcbf29ce484222325`,
  prime `0x100000001b3`) that the network, the timetable and the demand feed what they were
  built from. One way per type: raw bytes as given; a `u8`, `u32` or `u64` little-endian; a
  flag as one byte, 1 for true; an `f64` by its bits (`to_bits`), little-endian; a string as its
  length in bytes (`u64`) then its UTF-8 bytes; a set of modes as one byte, bit `k` for the mode
  of discriminant `k`. Not a cryptographic hash.

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
  entity identity). The Gumbel draws' logarithm is `libm`'s (pure Rust), not the platform's maths
  library, whose last bits differ from one platform to another.

## Bikes
A bike uses the same units as a car and a walker. No passenger-car unit exists, so a flow counts
vehicles. A bike's speed is a free-flow speed: 100 m at 5 m/s (18 km/h) takes 20 s. In a
fingerprint a set of modes is one byte and the bike is its bit 1, so a link or a trip that gains
or loses the bike changes the fingerprint that feeds it.

## Tests
Run `cargo test -p openmobisim-types`. Each expected value comes from outside the crate:
hand conversions (36 km/h is 10 m/s, 1 800 vehicles per hour is 0.5 per second); hand counts of
a step grid (7 200 s in 600 s steps is 12 steps); hand membership of a mode set; the standard library's sort for the id table;
the published FNV-1a 64-bit vectors (`""` is `cbf29ce484222325`, `"a"` `af63dc4c8601ec8c`,
`"foobar"` `85944171f73967e8`) and each typed feed equal to the bytes it names;
the chunked sequential definition of the sum, identical at 1 and 8 threads; the `rand_chacha`
ChaCha8 keystream for the documented key; the chi-square distribution for uniform and
independent draws; the Gumbel mean (Euler's constant) and variance (π²/6).
