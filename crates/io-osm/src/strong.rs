//! The largest strongly connected component of a directed graph: Tarjan's algorithm (R. Tarjan,
//! "Depth-first search and linear graph algorithms", *SIAM Journal on Computing* 1 (2), 1972),
//! with an explicit stack so that a long street does not overflow the call stack.

/// Which of `node_count` nodes lie in the largest strongly connected component of the graph
/// whose edges are `edges` (from, to). The largest is the one with the most nodes; on a tie,
/// the one Tarjan's search completes first from the lowest node, a pure function of the input.
#[must_use]
pub fn largest_component(node_count: usize, edges: &[(usize, usize)]) -> Vec<bool> {
    // Out-edges in compressed rows.
    let mut start = vec![0usize; node_count + 1];
    for &(from, _) in edges {
        start[from + 1] += 1;
    }
    for i in 0..node_count {
        start[i + 1] += start[i];
    }
    let mut next = start.clone();
    let mut target = vec![0usize; edges.len()];
    for &(from, to) in edges {
        target[next[from]] = to;
        next[from] += 1;
    }

    const UNSEEN: usize = usize::MAX;
    let mut index = vec![UNSEEN; node_count];
    let mut low = vec![0usize; node_count];
    let mut on_stack = vec![false; node_count];
    let mut stack: Vec<usize> = Vec::new();
    let mut component = vec![UNSEEN; node_count];
    let mut sizes: Vec<usize> = Vec::new();
    let mut counter = 0usize;
    // The search's own stack: a node and the position of its next out-edge.
    let mut path: Vec<(usize, usize)> = Vec::new();
    for root in 0..node_count {
        if index[root] != UNSEEN {
            continue;
        }
        path.push((root, start[root]));
        index[root] = counter;
        low[root] = counter;
        counter += 1;
        stack.push(root);
        on_stack[root] = true;
        while let Some(&mut (v, ref mut edge)) = path.last_mut() {
            if *edge < start[v + 1] {
                let w = target[*edge];
                *edge += 1;
                if index[w] == UNSEEN {
                    index[w] = counter;
                    low[w] = counter;
                    counter += 1;
                    stack.push(w);
                    on_stack[w] = true;
                    path.push((w, start[w]));
                } else if on_stack[w] {
                    low[v] = low[v].min(index[w]);
                }
                continue;
            }
            path.pop();
            if let Some(&(parent, _)) = path.last() {
                low[parent] = low[parent].min(low[v]);
            }
            if low[v] == index[v] {
                let id = sizes.len();
                let mut size = 0;
                while let Some(w) = stack.pop() {
                    on_stack[w] = false;
                    component[w] = id;
                    size += 1;
                    if w == v {
                        break;
                    }
                }
                sizes.push(size);
            }
        }
    }
    let mut largest = 0;
    for (id, &size) in sizes.iter().enumerate() {
        if size > sizes[largest] {
            largest = id;
        }
    }
    component.iter().map(|&c| !sizes.is_empty() && c == largest).collect()
}
