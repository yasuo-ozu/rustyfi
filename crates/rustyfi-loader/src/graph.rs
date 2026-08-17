//! Dependency-graph bookkeeping: topological ordering (dependencies first)
//! via `safegraph`, and cycle-chain reconstruction when it fails.

use std::collections::HashMap;
use std::path::PathBuf;

/// Given the "depends on" adjacency (`file id -> Vec<dependency id>`)
/// discovered while walking headers, compute a dependency-first load order.
///
/// `safegraph::BTreeGraph<u32, u32>` keys nodes by their own value (so a
/// `u32` file id doubles as its own stable `NodeIx`), matching the
/// `find_cycle_sccs` usage pattern in
/// `syan2/macro/recurse/graph.rs`. We insert one node per file, then one
/// edge per dependency — but reversed (`dependency -> dependent`), since
/// `safegraph::algo::toposort::toposort` orders a graph so that for every
/// edge `u -> v`, `u` comes before `v`; reversing puts each dependency before
/// everything that depends on it, with the entry document (which nothing
/// depends on) last.
///
/// On success: the dependency-first order of file ids.
/// On failure: `safegraph`'s `CycleError` only names one node known to lie on
/// a cycle. We re-run a small DFS over our own (un-reversed) adjacency map,
/// starting at that node, to reconstruct the actual cycle as a sequence of
/// ids (see [`find_cycle`]).
pub(crate) fn toposort(adjacency: &HashMap<u32, Vec<u32>>) -> Result<Vec<u32>, Vec<u32>> {
    use safegraph::algo::toposort::toposort as safegraph_toposort;
    use safegraph::graph::Graph;
    use safegraph::BTreeGraph;

    let mut g = BTreeGraph::<u32, u32>::default();
    for &id in adjacency.keys() {
        g.insert_node(id).unwrap();
    }
    let mut edge_id = 0u32;
    for (&from, deps) in adjacency {
        for &to in deps {
            // Reversed: dependency (`to`) before dependent (`from`).
            g.push_edge(edge_id, [to, from]).unwrap();
            edge_id += 1;
        }
    }

    match safegraph_toposort(&g) {
        Ok(order) => Ok(order),
        Err(err) => Err(find_cycle(adjacency, err.node)),
    }
}

/// SATySFi's own load order: a post-order depth-first traversal from the entry
/// document, visiting each file's dependencies in the exact order their
/// `@require:`/`@import:` headers appear (`adjacency[id]` is built in header
/// order). A file is appended to the order only after all its transitive
/// dependencies, so the result is dependency-first with the entry last — the
/// same as a topological sort, but with a *deterministic* tie-break that
/// mirrors upstream (`main.ml`'s recursive header resolution).
///
/// This matters because SATySFi's global-merge module model lets a library use
/// a module (e.g. `Option.map`) it never `@require:`s itself, relying on some
/// *earlier-loaded* file to have defined it. An arbitrary valid topological
/// order (what a generic toposort yields) can legally place the user before
/// that definition; header-order DFS reproduces the order the source was
/// written against, so a document `@require:`ing `option` before `fss/fss`
/// gets `Option` defined before `fss`'s internals reference it.
///
/// On a cycle, returns `Err` with the concrete cycle chain (via
/// [`find_cycle`]), matching the previous `safegraph` path's error shape.
pub(crate) fn header_order_toposort(
    adjacency: &HashMap<u32, Vec<u32>>,
    entry_id: u32,
) -> Result<Vec<u32>, Vec<u32>> {
    #[derive(Clone, Copy, PartialEq)]
    enum State {
        InProgress,
        Done,
    }

    // Iterative post-order DFS: each stack frame carries the node and the index
    // of the next child (dependency) to descend into. Recursion would be
    // simpler but the frozen corpus's `base` tree nests deeply enough to make
    // an explicit stack the safe choice.
    let mut state: HashMap<u32, State> = HashMap::new();
    let mut order: Vec<u32> = Vec::new();

    // A single DFS rooted at `entry_id` reaches every node (the loader only
    // ever allocates an id by discovering it as a dependency of a file it is
    // already walking from the entry), so one root suffices; a defensive
    // sweep over any stragglers follows.
    let mut roots: Vec<u32> = vec![entry_id];
    roots.extend(adjacency.keys().copied().filter(|&k| k != entry_id));

    for root in roots {
        if state.contains_key(&root) {
            continue;
        }
        let mut stack: Vec<(u32, usize)> = vec![(root, 0)];
        state.insert(root, State::InProgress);
        while let Some(&(node, child_idx)) = stack.last() {
            let children = adjacency.get(&node).map(Vec::as_slice).unwrap_or(&[]);
            if child_idx < children.len() {
                stack.last_mut().unwrap().1 += 1;
                let child = children[child_idx];
                match state.get(&child).copied() {
                    Some(State::Done) => {}
                    Some(State::InProgress) => {
                        // Back-edge into a node still on the stack — a cycle.
                        return Err(find_cycle(adjacency, child));
                    }
                    None => {
                        state.insert(child, State::InProgress);
                        stack.push((child, 0));
                    }
                }
            } else {
                // All dependencies emitted: this node comes next (post-order).
                order.push(node);
                state.insert(node, State::Done);
                stack.pop();
            }
        }
    }

    Ok(order)
}

/// Reconstruct one concrete cycle passing through `start`, using the
/// original (dependent -> dependency) adjacency map. `start` is guaranteed by
/// the caller to lie on some cycle (it came out of `safegraph`'s
/// `CycleError`), so a plain colored DFS from `start` is guaranteed to find a
/// back-edge into a node still on the current recursion stack. The returned
/// list is that stack slice with the repeated node appended at the end, e.g.
/// `[a, b, a]` for a two-file mutual cycle.
fn find_cycle(adjacency: &HashMap<u32, Vec<u32>>, start: u32) -> Vec<u32> {
    #[derive(Clone, Copy, PartialEq)]
    enum State {
        Unvisited,
        InProgress,
        Done,
    }

    fn dfs(
        node: u32,
        adjacency: &HashMap<u32, Vec<u32>>,
        state: &mut HashMap<u32, State>,
        stack: &mut Vec<u32>,
    ) -> Option<Vec<u32>> {
        state.insert(node, State::InProgress);
        stack.push(node);
        if let Some(children) = adjacency.get(&node) {
            for &child in children {
                match state.get(&child).copied().unwrap_or(State::Unvisited) {
                    State::Unvisited => {
                        if let Some(cycle) = dfs(child, adjacency, state, stack) {
                            return Some(cycle);
                        }
                    }
                    State::InProgress => {
                        let pos = stack.iter().position(|&n| n == child).unwrap();
                        let mut cycle: Vec<u32> = stack[pos..].to_vec();
                        cycle.push(child);
                        return Some(cycle);
                    }
                    State::Done => {}
                }
            }
        }
        stack.pop();
        state.insert(node, State::Done);
        None
    }

    let mut state = HashMap::new();
    let mut stack = Vec::new();
    dfs(start, adjacency, &mut state, &mut stack).unwrap_or_else(|| vec![start])
}

/// Map a chain of file ids back to canonical paths, in order.
pub(crate) fn chain_to_paths(chain: &[u32], path_of: &HashMap<u32, PathBuf>) -> Vec<PathBuf> {
    chain.iter().map(|id| path_of[id].clone()).collect()
}
