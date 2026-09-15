//! Turns: the node model's unit of work.
//!
//! A turn is an **(incoming link, outgoing link) pair at a node**. The node
//! model reads demand per turn and returns flow per turn; nothing in the
//! loading reasons about a node as a whole. Giving turns their own dense id
//! space (Foundations §1) means turn capacity, turn delay and eventually turn
//! restrictions are all one array lookup rather than a search through a node's
//! movements.
//!
//! # How many turns there are
//!
//! A node with `i` approaches and `o` exits has up to `i · o` turns. A typical
//! four-arm urban junction is 4 × 4 = 16, minus 4 U-turns, so 12. City-scale
//! that is roughly ten turns per node and a few million turns — which is why
//! the arrays here are as narrow as they are, and why the count is reported at
//! build time rather than discovered when memory runs out.
//!
//! # The U-turn rule
//!
//! U-turns are **excluded except at dead ends**. Including them everywhere
//! would let a route reverse at every node, which is both behaviourally wrong
//! and an enormous widening of the route search; excluding them everywhere
//! would strand any traveller who enters a cul-de-sac. The rule is therefore:
//! a U-turn exists only where the approach has no other exit.
//!
//! Explicit OSM turn restrictions are **not** applied yet — they arrive with
//! the importer, as an addition that only ever *removes* turns.

use openmobisim_core_types::ids::{EntityId, LinkId, NodeId, TurnId};

use crate::csr::Csr;
use crate::defaults::SignalDefaults;
use crate::network::RoadNetwork;

/// Every turn in a network, with its capacity fraction.
///
/// Built once from a [`RoadNetwork`] and shared with it.
#[derive(Debug)]
pub struct TurnTable {
    turn_in: Vec<LinkId>,
    turn_out: Vec<LinkId>,
    turn_node: Vec<NodeId>,
    /// Share of the movement's saturation flow the control lets through.
    ///
    /// `f32` rather than `f64`: it is a multiplier in `[0, 1]` read once per
    /// turn per sweep, and halving the array halves the memory the node model
    /// streams through.
    turn_capacity_fraction: Vec<f32>,
    by_in_link: Csr<LinkId, TurnId>,
    by_node: Csr<NodeId, TurnId>,
    u_turn_count: u32,
}

impl TurnTable {
    /// Build every turn of a network.
    ///
    /// Turns are generated in `(node, incoming link, outgoing link)` order, so
    /// the id assignment is a pure function of the network — which it must be,
    /// since turn ids reach the diagnostics report and the observation API.
    ///
    /// # Panics
    ///
    /// Panics if the turn count exceeds the `u32` id space. At roughly ten
    /// turns per node that needs a network of hundreds of millions of nodes,
    /// which is a bug rather than a city.
    #[must_use]
    pub fn build(network: &RoadNetwork, signals: SignalDefaults) -> Self {
        let node_count = network.node_count();

        let mut turn_in = Vec::new();
        let mut turn_out = Vec::new();
        let mut turn_node = Vec::new();
        let mut turn_capacity_fraction = Vec::new();
        let mut u_turn_count = 0u32;

        for node in NodeId::iter_space(node_count) {
            let incoming = network.in_links(node);
            let outgoing = network.out_links(node);
            if incoming.is_empty() || outgoing.is_empty() {
                continue;
            }
            let fraction = if network.is_signalised(node) {
                #[allow(
                    clippy::cast_possible_truncation,
                    reason = "a green-time fraction is in [0, 1]; f32 is ample"
                )]
                let f = signals.turn_capacity_fraction() as f32;
                f
            } else {
                1.0f32
            };

            for &in_link in incoming {
                // A U-turn is allowed only where the approach has no other exit.
                let has_other_exit =
                    outgoing.iter().any(|&out| !network.is_reverse_of(in_link, out));

                for &out_link in outgoing {
                    let is_u_turn = network.is_reverse_of(in_link, out_link);
                    if is_u_turn && has_other_exit {
                        continue;
                    }
                    if is_u_turn {
                        u_turn_count += 1;
                    }
                    turn_in.push(in_link);
                    turn_out.push(out_link);
                    turn_node.push(node);
                    turn_capacity_fraction.push(fraction);
                }
            }
        }

        let turn_count = u32::try_from(turn_in.len()).expect("turn count fits the u32 id space");

        let by_in_link = Csr::from_pairs(
            network.link_count(),
            (0..turn_count).map(|t| (turn_in[t as usize], TurnId::new(t))),
        );
        let by_node = Csr::from_pairs(
            node_count,
            (0..turn_count).map(|t| (turn_node[t as usize], TurnId::new(t))),
        );

        Self {
            turn_in,
            turn_out,
            turn_node,
            turn_capacity_fraction,
            by_in_link,
            by_node,
            u_turn_count,
        }
    }

    /// How many turns.
    #[inline]
    #[must_use]
    pub fn len(&self) -> usize {
        self.turn_in.len()
    }

    /// Whether the network has no turns at all.
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.turn_in.is_empty()
    }

    /// How many of the turns are U-turns at dead ends.
    ///
    /// Reported at build time: an unexpected number usually means the network
    /// is more fragmented than the author thinks.
    #[inline]
    #[must_use]
    pub fn u_turn_count(&self) -> u32 {
        self.u_turn_count
    }

    /// A turn's incoming link.
    #[inline]
    #[must_use]
    pub fn incoming(&self, turn: TurnId) -> LinkId {
        self.turn_in[turn.index()]
    }

    /// A turn's outgoing link.
    #[inline]
    #[must_use]
    pub fn outgoing(&self, turn: TurnId) -> LinkId {
        self.turn_out[turn.index()]
    }

    /// The node a turn happens at.
    #[inline]
    #[must_use]
    pub fn node(&self, turn: TurnId) -> NodeId {
        self.turn_node[turn.index()]
    }

    /// The share of saturation flow the control lets through this turn.
    #[inline]
    #[must_use]
    pub fn capacity_fraction(&self, turn: TurnId) -> f32 {
        self.turn_capacity_fraction[turn.index()]
    }

    /// The turns leaving one approach.
    #[inline]
    #[must_use]
    pub fn turns_from(&self, in_link: LinkId) -> &[TurnId] {
        self.by_in_link.targets(in_link)
    }

    /// Every turn at a node — the node model's work unit.
    #[inline]
    #[must_use]
    pub fn turns_at(&self, node: NodeId) -> &[TurnId] {
        self.by_node.targets(node)
    }

    /// The turn from `in_link` to `out_link`, if it exists.
    ///
    /// A linear scan of one approach's turns, which is at most a handful. Used
    /// when a route says "then take that link"; never in a per-step loop.
    #[must_use]
    pub fn find(&self, in_link: LinkId, out_link: LinkId) -> Option<TurnId> {
        self.turns_from(in_link).iter().copied().find(|&t| self.outgoing(t) == out_link)
    }

    /// The largest number of turns at any node.
    ///
    /// Sizes the node model's per-node scratch buffers.
    #[must_use]
    pub fn max_turns_per_node(&self) -> usize {
        self.by_node.max_degree()
    }

    /// Every turn's capacity fraction, for the node model's vectorised pass.
    #[inline]
    #[must_use]
    pub fn capacity_fractions(&self) -> &[f32] {
        &self.turn_capacity_fraction
    }

    /// Approximate bytes held, for the manifest.
    #[must_use]
    pub fn bytes(&self) -> usize {
        self.turn_in.len() * (2 * size_of::<LinkId>() + size_of::<NodeId>() + size_of::<f32>())
            + self.by_in_link.bytes()
            + self.by_node.bytes()
    }
}
