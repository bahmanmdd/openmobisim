# openmobisim
A simulator of multimodal dynamic traffic assignment: individual travellers choose mode and route; cars are loaded on a point-queue network, bikes and walkers at free flow, transit on its timetable; choice and loading are iterated to a stable state. A Rust core with a Python interface.
This tree rebuilds the core for release 0.2, module by module; each module enters with a README contract and a test against an answer from outside the simulator (theory, a hand derivation, a published result or another tool). Release 0.1 stays installable from PyPI.
Bikes are first-class in every module, test and output. Metric units throughout.

## The ten tests
Release 0.2 is done when these ten tests pass, each against an answer from outside the simulator. Where the code and its documents say "test N" or "test N of the specification", they mean a row of this table.

| # | Test | External answer |
|---|---|---|
| 1 | Bottleneck: one link, inflow above capacity for a period; and the same link with unlimited capacity | queue and delay by Vickrey's recursion within 1 s; free-flow times reproduced exactly |
| 2 | Merge: two links into one of lower capacity, the approaches' demands unequal | Daganzo's merge rule with demand-proportional priorities, which is the deterministic queue on the merged link: discharge at its capacity, delay by arrival order (Vickrey's recursion by hand), total delay λ(λ−μ)T²/(2μ), each approach's share of the discharge equal to its share of the demand |
| 3 | Two routes under steady demand | equal times on both used routes, split by capacity (Wardrop) |
| 4 | Braess network with capacities | the static equilibrium of Braess (1968) as the window grows |
| 5 | Nguyen–Dupuis | the capacity-constrained equilibrium (the route flows that minimise total free-flow time with no link above its capacity; Larsson and Patriksson 1995) solved as a linear program by an outside solver: total free-flow time, mean trip, origin–destination times within tolerance |
| 6 | Sioux Falls (TNTP) | the same capacity-constrained equilibrium from the TNTP files, solved by one outside solver and confirmed by a second: total free-flow time and the set of queued links tightly at a large population run to a fixed count, the mean trip at 1 %; a single link's queue delay is reported, never judged (Bar-Gera's BPR flows exceed capacity on 60 of 76 links and are not a queue model's answer) |
| 7 | Choice closed forms | logit, nested logit and path-size logit shares against hand-computed values, chi-square at sampling error |
| 8 | Transit | Spiess–Florian common lines (expected time and shares); RAPTOR equal to a brute force on random timetables |
| 9 | Bike and walk | trip times equal length over speed by hand; routes equal networkx; path-size shares at closed form |
| 10 | Delft, signals ignored | free-flow routes of 1 000 node pairs per mode (car, bike, walk) equal to networkx's shortest times on the same network; identical results at 1 and 8 threads and over two runs; every trip finishes; under 15 s; the stopped state within 1 % of a 60-iteration run |

Licensed under Apache-2.0; see LICENSE and NOTICE.
