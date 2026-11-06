//! The choice models behind one interface, [`Choice`]: the deterministic least cost, and the
//! logit family (logit, nested logit, path-size logit).
//!
//! # Deterministic
//!
//! [`Deterministic`] takes the row of least cost; of rows of equal least cost, the first. Its
//! probability is 1 on that row and 0 on the rest. No draw is made.
//!
//! # The logit family: the nested logit with one nest scale
//!
//! [`Logit`] is the nested logit with one nest scale `μ` in (0, 1], the nests being the rows'
//! nest ids. With `v_j = −cost_j / μ`, the closed form (Ben-Akiva and Lerman 1985, chapter 10):
//!
//! | Quantity | Formula |
//! |---|---|
//! | inclusive value of nest `n` | `I_n = ln Σ_{j∈n} exp(v_j)` |
//! | probability of nest `n` | `P(n) = exp(μ I_n) / Σ_m exp(μ I_m)` |
//! | probability of row `j` within its nest | `P(j | n) = exp(v_j) / Σ_{k∈n} exp(v_k)` |
//! | probability of row `j` | `P(j) = P(n) · P(j | n)` |
//!
//! With `μ = 1`, or with every row in its own nest, it is the plain logit,
//! `P(j) = exp(−cost_j) / Σ_k exp(−cost_k)`. The path-size logit is the plain logit with an
//! `ln PS` column weighed −1 (see [`crate::cost`]). Sums of exponentials are taken with the
//! largest exponent factored out, so a cost of thousands of seconds neither overflows nor
//! underflows to an empty sum.
//!
//! # Sampling: Gumbel-max in two stages, exact
//!
//! The traveller takes the nest of greatest `μ I_n + ε_n`, then within it the row of greatest
//! `v_j + ε_j`, each `ε` an independent standard Gumbel draw. The first stage picks `n` with
//! probability `P(n)` and the second, independent of it, `j` with `P(j | n)` (the Gumbel-max
//! property of the logit, McFadden 1978): the draw is exactly from the closed form. A nest of
//! one row needs no second stage. Ties (of probability zero) go to the first nest or row.
//!
//! The draws are from [`Stream::Choice`], each at an address made of who and what it is for,
//! never of how many draws came before. A draw block holds sixteen words for the price of one
//! keystream block, so each block holds the draws of eight rows (even words) and eight nests
//! (odd words):
//!
//! | Draw | Address (trip, block) | Word of the block |
//! |---|---|---|
//! | `ε_j` of the row at index `j` | (trip, `j / 8`) | `2 · (j mod 8)` |
//! | `ε_n` of the nest with id `n` | (trip, `n / 8`) | `2 · (n mod 8) + 1` |
//!
//! So no two draws share a bit: two rows, or two nests, differ in their address or their
//! word, and a row and a nest in the parity of their word. The nest id is a `u32` by type and
//! enters the address as it is: two nests with different ids never share a draw.
//!
//! The draws are the traveller's taste, fixed for the whole run: they depend on its trip, the
//! row's position in its set (the set is append-only, so a position is stable) or the nest id,
//! and the seed, never on the iteration. The choice changes only when the costs change, and is
//! the same whatever order travellers are asked in and on whatever thread.

use openmobisim_types::ids::EntityId;
use openmobisim_types::rng::{DrawAddress, DrawBlock, RngKey, Stream, StreamRng};

use crate::cost::{ChoiceError, Coefficients, Row, Traveller};

/// The rows (and the nests) whose draws share one draw block.
const PER_BLOCK: u32 = 8;

/// Which kind of draw: the offset of its word within its pair of words.
#[derive(Clone, Copy)]
enum Kind {
    Row = 0,
    Nest = 1,
}

/// The draws of one traveller's choice, with the last block read kept.
struct Draws<'a> {
    stream: &'a StreamRng,
    trip: u32,
    last: Option<(u32, DrawBlock)>,
}

impl Draws<'_> {
    /// The standard Gumbel draw of the row at index `index`, or of the nest with id `index`.
    fn gumbel(&mut self, kind: Kind, index: u32) -> f64 {
        let at = index / PER_BLOCK;
        let block = match self.last {
            Some((held, block)) if held == at => block,
            _ => {
                let address = DrawAddress::from_pair(self.trip, at);
                let block = self.stream.block(address);
                self.last = Some((at, block));
                block
            }
        };
        block.gumbel(2 * (index % PER_BLOCK) as usize + kind as usize)
    }
}

/// A choice model: which row a traveller takes, and with what probability each.
pub trait Choice {
    /// The index of the row `traveller` takes among `rows`.
    ///
    /// # Errors
    ///
    /// As [`Coefficients::costs`]: no rows, a class the model does not have, a row whose
    /// attributes are not one per column, a cost that is not finite.
    fn choose(&self, traveller: &Traveller, rows: &[Row<'_>]) -> Result<usize, ChoiceError>;

    /// The probability of each row for `traveller`, in row order; they sum to 1.
    ///
    /// # Errors
    ///
    /// As [`Choice::choose`].
    fn probabilities(
        &self,
        traveller: &Traveller,
        rows: &[Row<'_>],
    ) -> Result<Vec<f64>, ChoiceError>;
}

/// The least cost: the first row of least cost, with probability 1.
#[derive(Clone, Debug, PartialEq)]
pub struct Deterministic {
    coefficients: Coefficients,
}

impl Deterministic {
    /// The deterministic model weighing rows by `coefficients`.
    #[must_use]
    pub fn new(coefficients: Coefficients) -> Self {
        Self { coefficients }
    }
}

impl Choice for Deterministic {
    fn choose(&self, traveller: &Traveller, rows: &[Row<'_>]) -> Result<usize, ChoiceError> {
        let costs = self.coefficients.costs(traveller, rows)?;
        let mut best = 0;
        for (j, &cost) in costs.iter().enumerate() {
            if cost < costs[best] {
                best = j;
            }
        }
        Ok(best)
    }

    fn probabilities(
        &self,
        traveller: &Traveller,
        rows: &[Row<'_>],
    ) -> Result<Vec<f64>, ChoiceError> {
        let best = self.choose(traveller, rows)?;
        let mut p = vec![0.0; rows.len()];
        p[best] = 1.0;
        Ok(p)
    }
}

/// The logit family: the nested logit with nest scale `μ`, the plain logit at `μ = 1` or with
/// every row in its own nest, the path-size logit by an `ln PS` column. Sampled exactly by
/// Gumbel-max in two stages (see the module documentation).
#[derive(Clone, Debug, PartialEq)]
pub struct Logit {
    coefficients: Coefficients,
    mu: f64,
    draws: StreamRng,
}

/// The nests of one choice: the nest ids in order of first appearance, and each row's nest as
/// an index into them.
struct Nests {
    ids: Vec<u32>,
    of: Vec<usize>,
}

impl Nests {
    fn of(rows: &[Row<'_>]) -> Self {
        let mut ids: Vec<u32> = Vec::new();
        let of = rows
            .iter()
            .map(|r| match ids.iter().position(|&id| id == r.nest) {
                Some(n) => n,
                None => {
                    ids.push(r.nest);
                    ids.len() - 1
                }
            })
            .collect();
        Self { ids, of }
    }
}

/// `ln Σ exp(x)` over `values`, the largest factored out; negative infinity for none.
fn log_sum_exp(values: impl Iterator<Item = f64> + Clone) -> f64 {
    let top = values.clone().fold(f64::NEG_INFINITY, f64::max);
    if top == f64::NEG_INFINITY {
        return top;
    }
    top + values.map(|x| (x - top).exp()).sum::<f64>().ln()
}

impl Logit {
    /// The logit family weighing rows by `coefficients`, with nest scale `mu` (no unit; 1 is the
    /// plain logit), its draws from [`Stream::Choice`] of the run keyed `key`.
    ///
    /// # Errors
    ///
    /// [`ChoiceError::NestScale`] for a `mu` not in (0, 1].
    pub fn new(coefficients: Coefficients, mu: f64, key: RngKey) -> Result<Self, ChoiceError> {
        if !(mu > 0.0 && mu <= 1.0) {
            return Err(ChoiceError::NestScale(mu));
        }
        Ok(Self { coefficients, mu, draws: StreamRng::new(key, Stream::Choice) })
    }

    /// The nest scale μ.
    #[must_use]
    pub fn mu(&self) -> f64 {
        self.mu
    }

    /// The scaled values `v_j = −cost_j / μ`, the nests, and each nest's inclusive value `I_n`.
    fn inclusive(
        &self,
        traveller: &Traveller,
        rows: &[Row<'_>],
    ) -> Result<(Vec<f64>, Nests, Vec<f64>), ChoiceError> {
        let costs = self.coefficients.costs(traveller, rows)?;
        let v: Vec<f64> = costs.iter().map(|c| -c / self.mu).collect();
        let nests = Nests::of(rows);
        let inclusive = (0..nests.ids.len())
            .map(|n| {
                log_sum_exp(v.iter().zip(&nests.of).filter(move |&(_, &m)| m == n).map(|(x, _)| *x))
            })
            .collect();
        Ok((v, nests, inclusive))
    }

    /// The draws of `traveller`'s choice.
    fn draws(&self, traveller: &Traveller) -> Draws<'_> {
        Draws { stream: &self.draws, trip: traveller.trip.raw(), last: None }
    }
}

impl Choice for Logit {
    fn choose(&self, traveller: &Traveller, rows: &[Row<'_>]) -> Result<usize, ChoiceError> {
        let (v, nests, inclusive) = self.inclusive(traveller, rows)?;
        let mut draws = self.draws(traveller);
        let mut nest = (f64::NEG_INFINITY, 0);
        for (n, (&id, &i_n)) in nests.ids.iter().zip(&inclusive).enumerate() {
            let value = self.mu * i_n + draws.gumbel(Kind::Nest, id);
            if value > nest.0 {
                nest = (value, n);
            }
        }
        let members: Vec<usize> = (0..rows.len()).filter(|&j| nests.of[j] == nest.1).collect();
        if let [only] = members[..] {
            return Ok(only);
        }
        let mut row = (f64::NEG_INFINITY, members[0]);
        for (j, (&x, &m)) in (0u32..).zip(v.iter().zip(&nests.of)) {
            if m != nest.1 {
                continue;
            }
            let value = x + draws.gumbel(Kind::Row, j);
            if value > row.0 {
                row = (value, j as usize);
            }
        }
        Ok(row.1)
    }

    fn probabilities(
        &self,
        traveller: &Traveller,
        rows: &[Row<'_>],
    ) -> Result<Vec<f64>, ChoiceError> {
        let (v, nests, inclusive) = self.inclusive(traveller, rows)?;
        let all = log_sum_exp(inclusive.iter().map(|i| self.mu * i));
        Ok(v.iter()
            .zip(&nests.of)
            .map(|(x, &n)| (self.mu * inclusive[n] - all).exp() * (x - inclusive[n]).exp())
            .collect())
    }
}
