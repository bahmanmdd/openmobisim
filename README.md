# openmobisim
A simulator of multimodal dynamic traffic assignment: individual travellers choose mode and route; cars are loaded on a point-queue network, bikes and walkers at free flow, transit on its timetable; choice and loading are iterated to a stable state. A Rust core with a Python interface.
This tree rebuilds the core for release 0.2, module by module; each module enters with a README contract and a test against an answer from outside the simulator (theory, a hand derivation, a published result or another tool). Release 0.1 stays installable from PyPI.
Bikes are first-class in every module, test and output. Metric units throughout.
Licensed under Apache-2.0; see LICENSE and NOTICE.
