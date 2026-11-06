# openmobisim-choice

Which alternative a traveller takes among those offered, and with what probability each: the
deterministic least cost, and the logit family (logit, nested logit, path-size logit), behind
one interface. Depends on `openmobisim-types` only. It knows nothing of links, routes,
timetables or the network.

## Inputs
- `Coefficients::new(columns, classes, by_class)`: the names of the columns (for example
  `time_s`, `money_eur`, `ln_path_size`), the run's number of user classes, and per class one
  coefficient per column. Fixed once built.
- Per choice, a `Traveller { trip, class }` and the alternatives as `Row
  { attributes, mode, nest }`: the attributes one per column, in the columns' order and units;
  the mode, a label nothing here reads; the nest, a `u32` (rows with the same nest share it).
- For the logit family, the nest scale `μ` in (0, 1] and the run's seed (`RngKey`).

## Outputs
- `Choice::choose(&traveller, &rows)`: the index of the row taken.
- `Choice::probabilities(&traveller, &rows)`: each row's probability, in row order, summing
  to 1.
- `Coefficients::costs(&traveller, &rows)`: each row's cost.

## Units
The cost of a row for a traveller of class `c` is `Σ_k β[c][k] · x[k]`, each attribute `x[k]`
in its column's unit (seconds, euros, none for `ln PS`) and each coefficient in cost per that
unit. A time-only run has a coefficient of 1 on `time_s` and 0 on the rest: its cost is in
seconds. Probabilities have no unit; `μ` has none.

## Meanings and assumptions
- Least cost is best. The cost is the systematic part of the choice model, the same for the
  two implementations.
- `Deterministic`: the row of least cost, the first of equal least costs; probability 1 on it.
- `Logit`: the nested logit with one nest scale (Ben-Akiva and Lerman 1985, chapter 10). With
  `v_j = −cost_j / μ`: `I_n = ln Σ_{j∈n} exp(v_j)`, `P(n) = exp(μ I_n) / Σ_m exp(μ I_m)`,
  `P(j | n) = exp(v_j) / Σ_{k∈n} exp(v_k)`. At `μ = 1`, or with every row in its own nest, it is
  the plain logit `exp(−cost_j) / Σ_k exp(−cost_k)`.
- The path-size logit (Ben-Akiva and Bierlaire 1999) is the logit with a column holding
  `ln PS`, the route's path-size factor computed by whoever knows the routes, and a coefficient
  of −1 on it. There is no third implementation.
- Sampling is exact, by Gumbel-max in two stages: the nest of greatest `μ I_n + ε_n`, then in it
  the row of greatest `v_j + ε_j`, each `ε` an independent standard Gumbel draw.
- The draws come from the choice stream of the run's seed, at addresses made of the trip and the
  row's index or the nest's id: row `j` at block `j / 8`, word `2 (j mod 8)`; nest `n` at block
  `n / 8`, word `2 (n mod 8) + 1`. No two draws share a bit; two nests with different ids never
  share a draw.
- The draws are the traveller's taste, fixed for the whole run: they depend on its trip, the
  row's position in its set (the set is append-only, so a position is stable) or the nest id,
  and the seed, never on the iteration. The choice changes only when the costs change, and is
  the same in any order, on any thread, over two runs.

## Errors
Refused when built: a class's coefficients not one per column, a coefficient not finite, a
class of the run without coefficients, coefficients for a class the run does not have or for
one class twice (`Coefficients::new`); a nest scale outside (0, 1] (`Logit::new`). Refused at
choice: no rows, a row whose attributes are not one per column, a traveller's class the model
does not have, a cost that is not finite. Nothing panics.

## Bikes
A bike route is a row like any other: its time at the bike's speed, its money, its `ln PS`
among the other bike routes; its mode label says bike and nothing reads it. A bike, a car, a
walk and a transit itinerary in different nests make mode and route choice one nested logit.
The path-size test is on three bike routes; test 9's path-size shares for bikes rest on it.

## Tests
`cargo test -p openmobisim-choice`, about 2 s in debug. Each expected value comes from outside
the crate: six rows with costs `ln(21 / k)` take `k / 21` by hand within 1e-12, and 200 000 draws
fit by chi-square (below 20.52, the 0.999 quantile at 5 degrees of freedom); three nests of
three, one and two rows at `μ` of 0.2, 0.5, 0.9 and 1 against the formula written out in the
test within 1e-12, 20 000 draws each by chi-square, the plain logit at `μ = 1`, nest ids 7 and
4 000 000 000 giving the shares of 0 and 1; the path-size logit on three bike routes of equal
time, `P_A = 1 / (3 − φ)` (0.4545 at φ = 0.8, 0.4 at φ = 0.5, 1/3 without the term); a car
nest of 288 s and 345.6 s routes against a 3 000 s walk at 0.03 per minute, `P(walk)` and
`P(short | car)` by hand at `μ` of 0.5 and 1, 6 000 draws within three standard deviations; two
classes over the same rows, `1 / (1 + e³)` and `1 / (1 + e⁻³)`; the deterministic least cost
and its ties; the refusals; and determinism, the one self-comparison: 10 000 travellers give
the same choices over two runs, in a shuffled order and on eight threads.
