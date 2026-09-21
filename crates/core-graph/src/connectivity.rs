//! Strong connectivity: can every place be reached from every other?
//!
//! A route set exists for a pair of nodes only if the second is reachable from
//! the first. A network in which some pairs are not connected — a one-way spur
//! leading nowhere, a street the extract's edge cut off, a gated estate that is
//! drawn as a road — has trips with no route, and the failure shows up far
//! from its cause. This module answers the question directly, in two forms:
//!
//! * **at nodes** — the usual graph definition: `A` and `B` are in the same
//!   component if there is a directed path each way;
//! * **at links, through the turn table** — every legal turn is an edge. A
//!   two-way triangle of streets is *two* link components, one for each way
//!   round (a vehicle cannot U-turn except at a dead end), and a link that can
//!   only be entered by starting on it is a component of its own. So this is
//!   **not** the test to demand be one component: it is reported because it
//!   shows what the turn rule does, and because it is the one that will matter
//!   when OSM turn restrictions are read (a prohibited turn can cut a route
//!   without cutting a road).
//!
//! **The verdict is the node test.** A trip starts on any link leaving its
//! origin and may end on any link entering its destination, and a *simple* path
//! never turns back on itself — so while the U-turn rule is the only turn
//! restriction, node-level strong connectivity is exactly what every trip needs
//! (a test checks it against brute force on random graphs).
//!
//! # By mode
//!
//! A network holds streets a car may not use (footways, steps, cycleways: the
//! walk and cycle layers of later phases). Connectivity is a property of a
//! mode: a footway that joins two road fragments does not make them one to a
//! car. [`analyse_by`] takes the links to include, so the question is asked of
//! the drivable network alone.
//!
//! Components are found with an iterative Tarjan search (no recursion: a city
//! network is a path a million links long), in time linear in nodes plus
//! edges. The result is a pure function of the input order, as everything
//! that reaches an output must be.

use openmobisim_core_types::ids::{EntityId, LinkId, NodeId};

use crate::network::RoadNetwork;
use crate::turns::TurnTable;

/// The strongly connected components of a directed graph.
///
/// Component ids are dense, `0..count()`, assigned in the order Tarjan's
/// search completes them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Components {
    component: Vec<u32>,
    sizes: Vec<u32>,
}

impl Components {
    /// How many components there are (a vertex on no cycle is a component of
    /// its own).
    #[must_use]
    pub fn count(&self) -> usize {
        self.sizes.len()
    }

    /// How many vertices the graph has.
    #[must_use]
    pub fn vertex_count(&self) -> usize {
        self.component.len()
    }

    /// The component a vertex belongs to.
    #[must_use]
    pub fn component_of(&self, vertex: usize) -> u32 {
        self.component[vertex]
    }

    /// How many vertices component `c` holds.
    #[must_use]
    pub fn size_of(&self, c: u32) -> u32 {
        self.sizes[c as usize]
    }

    /// The largest component; the lowest id on a tie, so the choice is
    /// deterministic. `None` for an empty graph.
    #[must_use]
    pub fn largest(&self) -> Option<u32> {
        let mut best: Option<(u32, u32)> = None;
        for (c, &s) in (0u32..).zip(&self.sizes) {
            if best.is_none_or(|(_, bs)| s > bs) {
                best = Some((c, s));
            }
        }
        best.map(|(c, _)| c)
    }

    /// Whether the whole graph is one component (an empty graph is not).
    #[must_use]
    pub fn is_strongly_connected(&self) -> bool {
        self.sizes.len() == 1
    }

    /// The component sizes, largest first.
    #[must_use]
    pub fn sizes_descending(&self) -> Vec<u32> {
        let mut s = self.sizes.clone();
        s.sort_unstable_by(|a, b| b.cmp(a));
        s
    }
}

/// The strongly connected components of the graph in compressed-sparse-row
/// form: vertex `v`'s successors are `targets[offsets[v]..offsets[v + 1]]`.
///
/// # Panics
///
/// Panics if `offsets` is empty, or a target is not a vertex; both are bugs in
/// whatever built the arrays.
#[must_use]
pub fn strong_components(offsets: &[u32], targets: &[u32]) -> Components {
    assert!(!offsets.is_empty(), "offsets hold one entry more than there are vertices");
    let n = offsets.len() - 1;
    const UNVISITED: u32 = u32::MAX;

    let mut index = vec![UNVISITED; n];
    let mut low = vec![0u32; n];
    let mut on_stack = vec![false; n];
    let mut component = vec![UNVISITED; n];
    let mut sizes: Vec<u32> = Vec::new();
    let mut stack: Vec<u32> = Vec::new();
    // The explicit call stack: a vertex, and the position of its next edge.
    let mut calls: Vec<(u32, u32)> = Vec::new();
    let mut next_index = 0u32;

    for root in 0..n {
        if index[root] != UNVISITED {
            continue;
        }
        let root_id = u32::try_from(root).expect("vertex count fits u32");
        index[root] = next_index;
        low[root] = next_index;
        next_index += 1;
        stack.push(root_id);
        on_stack[root] = true;
        calls.push((root_id, offsets[root]));

        while let Some(&(v, pos)) = calls.last() {
            let vi = v as usize;
            if pos < offsets[vi + 1] {
                if let Some(top) = calls.last_mut() {
                    top.1 += 1;
                }
                let w = targets[pos as usize];
                let wi = w as usize;
                if index[wi] == UNVISITED {
                    index[wi] = next_index;
                    low[wi] = next_index;
                    next_index += 1;
                    stack.push(w);
                    on_stack[wi] = true;
                    calls.push((w, offsets[wi]));
                } else if on_stack[wi] {
                    low[vi] = low[vi].min(index[wi]);
                }
            } else {
                calls.pop();
                if low[vi] == index[vi] {
                    let id = u32::try_from(sizes.len()).expect("component count fits u32");
                    let mut size = 0u32;
                    loop {
                        let w = stack.pop().expect("the component's root is on the stack");
                        on_stack[w as usize] = false;
                        component[w as usize] = id;
                        size += 1;
                        if w == v {
                            break;
                        }
                    }
                    sizes.push(size);
                }
                if let Some(&(parent, _)) = calls.last() {
                    let p = parent as usize;
                    low[p] = low[p].min(low[vi]);
                }
            }
        }
    }

    Components { component, sizes }
}

/// The node graph's components: an edge for every link.
///
/// # Panics
///
/// Panics if the network has more than `u32::MAX` links (it cannot).
#[must_use]
pub fn node_components(network: &RoadNetwork) -> Components {
    let n = network.node_count();
    let mut offsets = Vec::with_capacity(n as usize + 1);
    let mut targets = Vec::with_capacity(network.link_count() as usize);
    offsets.push(0u32);
    for node in NodeId::iter_space(n) {
        for &link in network.out_links(node) {
            targets.push(network.link_to(link).raw());
        }
        offsets.push(u32::try_from(targets.len()).expect("link count fits u32"));
    }
    strong_components(&offsets, &targets)
}

/// The link graph's components: an edge for every legal turn.
///
/// A component here is a set of links each of which can be driven to every
/// other, turn rules included. Read the module docs before demanding that
/// there be only one.
///
/// # Panics
///
/// Panics if the network has more than `u32::MAX` turns (it cannot).
#[must_use]
pub fn link_components(network: &RoadNetwork, turns: &TurnTable) -> Components {
    let n = network.link_count();
    let mut offsets = Vec::with_capacity(n as usize + 1);
    let mut targets = Vec::with_capacity(turns.len());
    offsets.push(0u32);
    for link in LinkId::iter_space(n) {
        for &turn in turns.turns_from(link) {
            targets.push(turns.outgoing(turn).raw());
        }
        offsets.push(u32::try_from(targets.len()).expect("turn count fits u32"));
    }
    strong_components(&offsets, &targets)
}

/// What the two tests say about a network, in one place.
///
/// The numbers a network's author should read before trusting a run on it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConnectivityReport {
    /// Nodes in the network.
    pub nodes: u32,
    /// Directed links in the network.
    pub links: u32,
    /// Strongly connected components of the node graph, singletons included.
    pub node_components: u32,
    /// Nodes in the largest of them.
    pub node_largest: u32,
    /// Links with both ends in that component.
    pub links_in_node_largest: u32,
    /// Strongly connected components of the link graph (legal turns).
    pub link_components: u32,
    /// Links in the largest of them.
    pub link_largest: u32,
    /// Nodes with links leaving but none arriving: places a vehicle can start
    /// from and never return to.
    pub sources: u32,
    /// Nodes with links arriving but none leaving: places a vehicle can reach
    /// and never leave.
    pub sinks: u32,
    /// Component sizes of the node graph, largest first, at most ten.
    pub node_component_sizes_top: Vec<u32>,
}

impl ConnectivityReport {
    /// Whether every node reaches every other node — the node graph is one
    /// component. This is the verdict; see the module docs for why it is enough.
    #[must_use]
    pub fn is_strongly_connected(&self) -> bool {
        self.node_components == 1
    }
}

/// Run both tests on a network, over all its links.
///
/// A node no link touches is a component of its own here, as it is in the graph
/// — unlike in [`analyse_by`], which asks about a mode and so counts only the
/// nodes that mode's links reach.
#[must_use]
pub fn analyse(network: &RoadNetwork, turns: &TurnTable) -> ConnectivityReport {
    let mut r = analyse_by(network, turns, |_| true);
    let isolated = network.node_count() - r.nodes;
    r.nodes += isolated;
    r.node_components += isolated;
    if r.node_largest == 0 && isolated > 0 {
        r.node_largest = 1;
    }
    r.node_component_sizes_top.extend(std::iter::repeat_n(1, isolated as usize));
    r.node_component_sizes_top.truncate(10);
    r
}

/// Run both tests on the part of a network made of the links `include` accepts —
/// the drivable network, say. Nodes count only if an included link touches
/// them; the link graph keeps only turns between included links (with the turn
/// table as built: it does not know that a footway is not an exit for a car).
///
/// # Panics
///
/// Panics if the network has more than `u32::MAX` links (it cannot).
#[must_use]
pub fn analyse_by(
    network: &RoadNetwork,
    turns: &TurnTable,
    include: impl Fn(LinkId) -> bool,
) -> ConnectivityReport {
    let included: Vec<LinkId> =
        LinkId::iter_space(network.link_count()).filter(|&l| include(l)).collect();

    // The subgraph, with dense ids of its own.
    let mut node_index = vec![u32::MAX; network.node_count() as usize];
    let mut node_of: Vec<NodeId> = Vec::new();
    let mut touch = |n: NodeId| {
        let slot = &mut node_index[n.raw() as usize];
        if *slot == u32::MAX {
            *slot = u32::try_from(node_of.len()).expect("node count fits u32");
            node_of.push(n);
        }
    };
    for &l in &included {
        touch(network.link_from(l));
        touch(network.link_to(l));
    }
    let mut link_index = vec![u32::MAX; network.link_count() as usize];
    for (k, &l) in included.iter().enumerate() {
        link_index[l.raw() as usize] = u32::try_from(k).expect("link count fits u32");
    }

    // Node graph.
    let mut n_offsets = vec![0u32; node_of.len() + 1];
    for &l in &included {
        n_offsets[node_index[network.link_from(l).raw() as usize] as usize + 1] += 1;
    }
    for i in 0..node_of.len() {
        n_offsets[i + 1] += n_offsets[i];
    }
    let mut n_targets = vec![0u32; included.len()];
    let mut fill = n_offsets.clone();
    for &l in &included {
        let f = node_index[network.link_from(l).raw() as usize] as usize;
        n_targets[fill[f] as usize] = node_index[network.link_to(l).raw() as usize];
        fill[f] += 1;
    }
    let nodes = strong_components(&n_offsets, &n_targets);

    // Link graph: a legal turn between two included links is an edge.
    let mut l_offsets = Vec::with_capacity(included.len() + 1);
    let mut l_targets = Vec::new();
    l_offsets.push(0u32);
    for &l in &included {
        for &t in turns.turns_from(l) {
            let out = link_index[turns.outgoing(t).raw() as usize];
            if out != u32::MAX {
                l_targets.push(out);
            }
        }
        l_offsets.push(u32::try_from(l_targets.len()).expect("turn count fits u32"));
    }
    let links = strong_components(&l_offsets, &l_targets);

    let node_largest_id = nodes.largest();
    let links_in_node_largest = node_largest_id.map_or(0, |big| {
        let n = included
            .iter()
            .filter(|&&l| {
                nodes.component_of(node_index[network.link_from(l).raw() as usize] as usize) == big
                    && nodes.component_of(node_index[network.link_to(l).raw() as usize] as usize)
                        == big
            })
            .count();
        u32::try_from(n).expect("link count fits u32")
    });

    let (mut sources, mut sinks) = (0u32, 0u32);
    for &n in &node_of {
        let count = |ls: &[LinkId]| ls.iter().filter(|&&l| include(l)).count();
        let (i, o) = (count(network.in_links(n)), count(network.out_links(n)));
        if i == 0 && o > 0 {
            sources += 1;
        }
        if o == 0 && i > 0 {
            sinks += 1;
        }
    }

    let count = |c: &Components| u32::try_from(c.count()).expect("component count fits u32");
    ConnectivityReport {
        nodes: u32::try_from(node_of.len()).expect("node count fits u32"),
        links: u32::try_from(included.len()).expect("link count fits u32"),
        node_components: count(&nodes),
        node_largest: node_largest_id.map_or(0, |c| nodes.size_of(c)),
        links_in_node_largest,
        link_components: count(&links),
        link_largest: links.largest().map_or(0, |c| links.size_of(c)),
        sources,
        sinks,
        node_component_sizes_top: nodes.sizes_descending().into_iter().take(10).collect(),
    }
}
