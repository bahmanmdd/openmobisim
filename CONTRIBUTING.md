# Contributing to openmobisim

> **External pull requests are not accepted until a CLA is in place.** Issues and
> discussion are welcome in the meantime.

> **The core is small, and it stays small. New capability arrives as an
> addition, not as a modification.**

That sentence is the whole contribution model. Everything below is what it
means in practice.

---

## 1. Add, don't touch

1. **A new feature must not change existing core mechanisms.** It registers at
   an existing plugin point, or it proposes a new one. Modifying core
   behaviour is reserved for bug fixes — and a bug is where the code does not
   do what the documentation says.
2. **A default run behaves identically before and after your change.** New
   functionality arrives as a parameter that defaults to off.
3. **Off means absent, not neutral.** A feature that is switched off must cost
   nothing — not a branch, not a counter, not a byte. This is why plugin names
   are resolved into concrete callables once, at build time, and never looked
   up in a loop.
4. **Every addition documents its defaults and where they come from**, with a
   row in the references table. "It looked about right" is not a default; it
   is an unrecorded assumption.

---

## 2. The two rules people get quietly wrong

These are not style preferences. Each one, broken, produces results that are
wrong in a way no test failure announces.

### 2.1 Never write a bare parallel reduction

```rust
let total: f64 = xs.par_iter().sum();          // ❌ never
let total = openmobisim_core_types::reduce::fixed_order_sum(&xs);   // ✅
```

Rayon splits work according to how many threads happen to steal it, and
floating-point addition is not associative. So the same binary, on the same
machine, with the same input, can produce a different sum on two runs.

That would be tolerable if it announced itself. It does not. It silently
destroys common random numbers across designs, makes the convergence gap
wander below its own sampling floor, and makes two evaluations of the *same*
design differ. Use `openmobisim_core_types::reduce`. If it does not have the shape
you need, add it there rather than working around it.

The run-twice bit-identity check in CI is what catches violations. Do not
disable it.

### 2.2 Sweeps are Jacobi, not Gauss–Seidel

Every node update within a sweep reads **the previous sweep's state**, never
the state another node just wrote in the same sweep. Reading a neighbour's
freshly written value makes the result depend on the order nodes were visited,
which makes it depend on how the work was parallelised — the same failure mode
as above, arriving by a different road.

Write the new state into a separate buffer and swap at the end of the sweep.

---

## 3. Things that are decided

Some choices are load-bearing enough that changing them means changing every
stored artifact and every seeded run. They are decided and not open to casual
revision:

- **Internal ids are dense `u32`** with `u32::MAX` as the null sentinel. Never
  `Option<Id>` in a stored array — it is eight bytes where the sentinel is
  four, and these arrays have one entry per link or per traveller.
- **The clock is `u32` seconds.** Never a float clock.
- **Internal units are SI and per-second**, converted exactly once at the
  scenario build boundary. Never convert inside a loop.
- **A random draw is a pure function of its key**, never of a sequence
  position. No shared mutable generator, no thread-local, anywhere.
- **`core-types` depends on nothing**, and **`py-bindings` is the only crate
  that knows Python exists.** Behaviour goes in Python, physics goes in Rust.
- **Python names put the category first**, then the thing: `map_link`,
  `map_demand`, `link_bins`, not `link_map`. Names that share a prefix sort and
  complete together, so a group of functions, methods, variables or figures
  that do the same kind of thing is found in one place.

If you believe one of these is wrong, that is a design discussion to open
first, not a pull request to write.

---

## 4. What a change should come with

- **Tests.** Unit tests for the mechanism, property tests for anything with an
  invariant (conservation, monotonicity, FIFO), and a note in the test itself
  about *what property* it is defending. A test whose failure message does not
  tell you what broke is half a test.
- **Documentation on every public item.** `missing_docs` is a warning and CI
  treats warnings as errors, so this is enforced rather than encouraged. Say
  what the thing is for and what it costs, not what its name already says.
- **A performance note if it sits in a loop.** `crates/core-types/tests/perf.rs`
  is the pattern: a loose floor that catches an order-of-magnitude regression
  without flaking on a busy CI runner.

---

## 5. Running the checks

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test --workspace --release     # performance floors are enforced here only
maturin develop && pytest
```

CI runs all of these on Linux, macOS and Windows, and builds a wheel on each.

---

## 6. Licence and provenance

openmobisim is Apache-2.0. By contributing you agree your contribution is licensed
under it.

**Provenance matters here.** Several reference implementations in this field
are GPL or LGPL. If you consulted the source of another simulator while
writing a contribution, say so in the pull request — including which one and
which part. Reading a paper is not the same as reading a GPL implementation,
and the difference is one the project has to be able to demonstrate.
