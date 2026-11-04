//! The order and the identity of routes, with no link or node id in them: one rule for every
//! part of the code.
//!
//! A simulation's results must not depend on the names of links and nodes: renaming them, or
//! listing them in another order, is not a change of the network. Dense ids follow the sorted
//! external ids ([`RoadNetwork`]), so an index is a name; wherever two routes or two links must be
//! told apart, this module does it from what the network *is* instead.
//!
//! **The order of routes** ([`route_order`]): where their cost does not decide, the **shorter in
//! metres** comes first; at the same length, the one of **fewer links**; and at that too, the
//! one whose nodes' projected coordinates come first, read from the downstream end of the route
//! back towards its origin (the downstream node of its last link, then the upstream node of each
//! link from the last back; east coordinate first, then north: [`node_order`]). Two routes that
//! coincide node for node differ only in parallel links between the same two nodes: the one
//! whose links are shorter, read from the downstream end back, comes first, and only links that
//! also share their length are left to their ids. The searches apply the same rule label by
//! label ([`crate::search`]); a route set is sorted by it ([`crate::generate`]); the run's
//! choice reads it through the set's order.
//!
//! **The identity of links and routes** ([`LinkKeys`]): a 64-bit key per link made from its two
//! nodes' projected coordinates and its length, so the key of a link is the same whatever the
//! link is called. Links that share all of those (parallel links of one length between one pair
//! of nodes) are told apart by their rank among themselves, by id: the one place an id remains,
//! for links nothing else distinguishes. A route's identity ([`LinkKeys::route_identity`]) is a
//! hash of its links' keys in driving order; the random draws of a route choice are addressed
//! by it, so a route keeps its draws under any naming. A node's key ([`node_key`]) is a hash of
//! its coordinates: what seeds a random draw tied to a place.

use core::cmp::Ordering;

use openmobisim_core_graph::network::RoadNetwork;
use openmobisim_core_types::hash::Fnv1a;
use openmobisim_core_types::ids::{EntityId, LinkId, NodeId};

/// A link as a route holds it: a [`LinkId`] or its raw `u32`.
pub trait RouteLink: Copy {
    /// The link's id.
    fn link(self) -> LinkId;
}

impl RouteLink for LinkId {
    #[inline]
    fn link(self) -> LinkId {
        self
    }
}

impl RouteLink for u32 {
    #[inline]
    fn link(self) -> LinkId {
        LinkId::new(self)
    }
}

/// Two nodes by their projected coordinates: east first, then north. `Equal` only for one
/// node, or for two nodes at the same point.
#[must_use]
#[inline]
pub fn node_order(network: &RoadNetwork, a: NodeId, b: NodeId) -> Ordering {
    if a == b {
        return Ordering::Equal;
    }
    let (pa, pb) = (network.node_position(a), network.node_position(b));
    pa.x.total_cmp(&pb.x).then(pa.y.total_cmp(&pb.y))
}

/// The length in metres of a route, summed over its links in driving order (as the searches
/// sum it, so equal routes give equal numbers).
#[must_use]
pub fn route_metres<L: RouteLink>(network: &RoadNetwork, links: &[L]) -> f64 {
    let mut total = 0.0;
    for &l in links {
        total += network.link_length(l.link()).get();
    }
    total
}

/// The order of two routes where their cost does not decide (the [module docs](self)): the
/// shorter in metres, then the one of fewer links, then the nodes' coordinates read from the
/// downstream end back, then the links' lengths read the same way, then (links that share all
/// of that) their ids. `Equal` only for the same links.
#[must_use]
pub fn route_order<L: RouteLink>(network: &RoadNetwork, a: &[L], b: &[L]) -> Ordering {
    route_metres(network, a)
        .total_cmp(&route_metres(network, b))
        .then(a.len().cmp(&b.len()))
        .then_with(|| nodes_back(network, a, b))
        .then_with(|| {
            for (&x, &y) in a.iter().rev().zip(b.iter().rev()) {
                let o = network
                    .link_length(x.link())
                    .get()
                    .total_cmp(&network.link_length(y.link()).get());
                if o != Ordering::Equal {
                    return o;
                }
            }
            Ordering::Equal
        })
        .then_with(|| a.iter().rev().map(|l| l.link()).cmp(b.iter().rev().map(|l| l.link())))
}

/// Two routes of as many links by their nodes read from the downstream end back: the
/// downstream node of the last link, then the upstream node of each link from the last.
fn nodes_back<L: RouteLink>(network: &RoadNetwork, a: &[L], b: &[L]) -> Ordering {
    if let (Some(&x), Some(&y)) = (a.last(), b.last()) {
        let o = node_order(network, network.link_to(x.link()), network.link_to(y.link()));
        if o != Ordering::Equal {
            return o;
        }
    }
    for (&x, &y) in a.iter().rev().zip(b.iter().rev()) {
        let o = node_order(network, network.link_from(x.link()), network.link_from(y.link()));
        if o != Ordering::Equal {
            return o;
        }
    }
    Ordering::Equal
}

/// A node's key: a hash of its projected coordinates, the same whatever the node is called.
#[must_use]
pub fn node_key(network: &RoadNetwork, node: NodeId) -> u64 {
    let p = network.node_position(node);
    let mut h = Fnv1a::new();
    h.write_f64(p.x);
    h.write_f64(p.y);
    h.finish()
}

/// What a link is, without its name: its two nodes' coordinates and its length.
fn link_signature(network: &RoadNetwork, link: LinkId) -> u64 {
    let (a, b) = (
        network.node_position(network.link_from(link)),
        network.node_position(network.link_to(link)),
    );
    let mut h = Fnv1a::new();
    h.write_f64(a.x);
    h.write_f64(a.y);
    h.write_f64(b.x);
    h.write_f64(b.y);
    h.write_f64(network.link_length(link).get());
    h.finish()
}

/// Every link's key (the [module docs](self)): 8 bytes per link, made in one pass and a sort.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkKeys {
    keys: Box<[u64]>,
}

impl LinkKeys {
    /// The keys of every link of `network`.
    #[must_use]
    pub fn new(network: &RoadNetwork) -> Self {
        let n = network.link_count();
        let mut keys: Vec<u64> = (0..n).map(|l| link_signature(network, LinkId::new(l))).collect();
        // Links that share a signature (parallel links of one length): their rank among
        // themselves, by id, is mixed in. Nothing else tells them apart.
        let mut order: Vec<u32> = (0..n).collect();
        order.sort_unstable_by_key(|&l| (keys[l as usize], l));
        let mut i = 0;
        while i < order.len() {
            let sig = keys[order[i] as usize];
            let mut j = i + 1;
            while j < order.len() && keys[order[j] as usize] == sig {
                j += 1;
            }
            if j - i > 1 {
                for (rank, &l) in order[i..j].iter().enumerate() {
                    let mut h = Fnv1a::new();
                    h.write_u64(sig);
                    h.write_u64(rank as u64);
                    keys[l as usize] = h.finish();
                }
            }
            i = j;
        }
        Self { keys: keys.into_boxed_slice() }
    }

    /// The key of `link`.
    #[must_use]
    #[inline]
    pub fn key(&self, link: LinkId) -> u64 {
        self.keys[link.index()]
    }

    /// Every link's key, by link.
    #[must_use]
    pub fn as_slice(&self) -> &[u64] {
        &self.keys
    }

    /// A route's identity: a 32-bit hash of its links' keys in driving order, so it does not
    /// change when links are renamed, nor when other routes join or leave its set.
    #[must_use]
    pub fn route_identity<L: RouteLink>(&self, links: &[L]) -> u32 {
        let mut h = Fnv1a::new();
        for &l in links {
            h.write_u64(self.key(l.link()));
        }
        h.finish32()
    }
}
