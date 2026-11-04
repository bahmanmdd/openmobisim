//! The order and the identity of the objects of the input tables that stand at a point —
//! transit stops, parkings, docks, chargers, fleet vehicles — with no id in them: one rule for
//! every part of the code.
//!
//! A simulation's results must not depend on names: renaming the rows of a table, or listing
//! them in another order, is not a change of the city. Wherever two such objects must be put in
//! an order (which candidate is tried first, which of two equal journeys is kept) or must be
//! told apart in a hash that addresses random draws (an itinerary's identity), it is done from
//! where they are instead of from what they are called.
//!
//! **The order** ([`point_order`]): by longitude (east first), then latitude (north). Each kind
//! of object then compares what else it is (a parking its vehicle and capacity, a dock its
//! slots, a stop the lines through it, as their owners say), and the id only last, for objects
//! that share all of that.
//!
//! **The identity** ([`point_key`]): a 64-bit hash of the point's two coordinates, with what else
//! the object is hashed in by its owner; objects that share all of it are told apart by their
//! order ([`distinct`]).
//!
//! **Exact, the same on every platform.** The coordinates are compared and hashed as they were
//! read: a table's text is parsed to the nearest double the same way everywhere, and nothing
//! computes with them before they are compared here. (Projected coordinates and lengths, which
//! come from trigonometric functions, are rounded to millimetres before they are compared:
//! `openmobisim_core_routes::canonical`.) A negative zero counts as zero.

use core::cmp::Ordering;

use openmobisim_core_types::hash::Fnv1a;

use crate::geometry::LonLat;

/// `v` with a negative zero made positive, so `-0.0` and `0.0` are one coordinate.
#[inline]
fn zeroed(v: f64) -> f64 {
    v + 0.0
}

/// Two points: by longitude, then latitude (the [module docs](self)). `Equal` only for one
/// point.
#[must_use]
#[inline]
pub fn point_order(a: LonLat, b: LonLat) -> Ordering {
    zeroed(a.lon).total_cmp(&zeroed(b.lon)).then_with(|| zeroed(a.lat).total_cmp(&zeroed(b.lat)))
}

/// The 64-bit key of a point: what a hash names an object at it by (the [module docs](self)).
#[must_use]
#[inline]
pub fn point_key(p: LonLat) -> u64 {
    let mut h = Fnv1a::new();
    h.write_f64(zeroed(p.lon));
    h.write_f64(zeroed(p.lat));
    h.finish()
}

/// `keys` made distinct: visiting the objects in `order` (their owner's order of them), the
/// `n`-th one (from 0) whose key an earlier one has gets `n` hashed into it, so objects that share
/// their key are told apart by their order, which only their ids decide (the
/// [module docs](self)). The others keep their keys.
#[must_use]
pub fn distinct(mut keys: Vec<u64>, order: impl IntoIterator<Item = usize>) -> Vec<u64> {
    let mut seen: std::collections::HashMap<u64, u32> = std::collections::HashMap::new();
    for i in order {
        let n = seen.entry(keys[i]).or_insert(0);
        if *n > 0 {
            let mut h = Fnv1a::new();
            h.write_u64(keys[i]);
            h.write_u32(*n);
            keys[i] = h.finish();
        }
        *n += 1;
    }
    keys
}

/// The dense ranks of `n` objects under `cmp` (a total preorder on their indices): objects that
/// compare `Equal` share a rank, and the ranks count the distinct places before, from 0.
///
/// # Panics
///
/// If `n` does not fit `u32`.
#[must_use]
pub fn dense_ranks(n: usize, cmp: impl Fn(usize, usize) -> Ordering) -> Vec<u32> {
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| cmp(a, b));
    let mut rank = vec![0u32; n];
    let mut r = 0u32;
    for (k, &i) in order.iter().enumerate() {
        if k > 0 && cmp(order[k - 1], i) != Ordering::Equal {
            r += 1;
        }
        rank[i] = r;
    }
    rank
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn points_are_ordered_east_then_north() {
        let (w, e) = (LonLat::new(4.80, 45.75), LonLat::new(4.81, 45.70));
        assert_eq!(point_order(w, e), Ordering::Less);
        let (s, n) = (LonLat::new(4.80, 45.70), LonLat::new(4.80, 45.71));
        assert_eq!(point_order(s, n), Ordering::Less);
        assert_eq!(point_order(LonLat::new(-0.0, 1.0), LonLat::new(0.0, 1.0)), Ordering::Equal);
        assert_eq!(point_key(LonLat::new(-0.0, 1.0)), point_key(LonLat::new(0.0, 1.0)));
        assert_ne!(point_key(w), point_key(e));
    }

    #[test]
    fn objects_at_one_point_get_distinct_keys_in_their_order() {
        let p = point_key(LonLat::new(4.8, 45.7));
        let q = point_key(LonLat::new(4.9, 45.7));
        let k = distinct(vec![p, q, p], [0, 1, 2]);
        assert_eq!((k[0], k[1]), (p, q));
        assert_ne!(k[2], p);
        // Visited the other way, the first visited keeps the point's key.
        let k = distinct(vec![p, q, p], [2, 1, 0]);
        assert_eq!(k[2], p);
        assert_ne!(k[0], p);
    }

    #[test]
    fn ranks_follow_the_order() {
        let y = [3.0_f64, 1.0, 3.0, 2.0];
        assert_eq!(dense_ranks(4, |a, b| y[a].total_cmp(&y[b])), vec![2, 0, 2, 1]);
    }
}
