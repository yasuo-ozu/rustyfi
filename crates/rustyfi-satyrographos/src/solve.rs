//! The phase-7c transitive dependency solver (design §4): a hand-rolled DFS
//! backtracking resolver over the compat-bucket role model from
//! `version.rs`, reproducing `saphe-split:src-saphe/packageConstraintSolver.ml`
//! without a SAT library (design §7 — `std` only, no `pubgrub`/`semver`).
//!
//! [`DepSource`] keeps the algorithm registry-agnostic: `registry.rs`'s
//! `RegistryDepSource` adapts the live index; this module's own tests use a
//! plain in-memory graph, mirroring `packageConstraintSolverTest.ml`.

use std::collections::BTreeMap;

use crate::error::Error;
use crate::version::{Constraint, Version};

/// Where the solver gets a package's available versions and each version's
/// declared dependencies. One implementor per "backend" (the live registry
/// index, an in-memory test graph, …) — the search algorithm below never
/// touches a filesystem or registry type directly.
pub trait DepSource {
    /// Every version of `name` that could be chosen, in any order (the
    /// solver sorts). [`Error::PackageNotFound`] if `name` is not known to
    /// this source at all (as opposed to merely having no *compatible*
    /// version — that is [`Error::Unsatisfiable`], raised by the solver
    /// itself once it has the (possibly non-matching) version list).
    fn versions(&self, name: &str) -> Result<Vec<Version>, Error>;

    /// The declared `(dependency name, requirement)` pairs of the concrete
    /// `(name, v)`. `v` is always a version previously returned by
    /// [`DepSource::versions`] for the same `name`.
    fn deps(&self, name: &str, v: &Version) -> Result<Vec<(String, Constraint)>, Error>;
}

/// A fully-resolved dependency graph: one chosen version per package name
/// (direct **and** transitive — the whole closure).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Solution {
    pub packages: BTreeMap<String, Version>,
}

/// One accumulated requirement on a package, remembering who asked for it
/// (for [`Error::Unsatisfiable`]/[`Error::VersionConflict`] diagnostics).
#[derive(Debug, Clone)]
struct Req {
    constraint: Constraint,
    /// A human-readable "who required this" label: `"<root>"` for a direct
    /// dependency, or `"<name>@<version>"` for a transitive one.
    requirer: String,
}

/// Search state threaded through the recursion: which packages are pinned so
/// far, and every requirement accumulated on every (assigned or not yet
/// assigned) package name.
struct State {
    assigned: BTreeMap<String, Version>,
    constraints: BTreeMap<String, Vec<Req>>,
}

/// Resolve `root`'s dependencies (and everything they transitively require)
/// to one concrete [`Version`] each, or fail with [`Error::Unsatisfiable`] /
/// [`Error::VersionConflict`] / [`Error::PackageNotFound`] (design §4.2).
///
/// Backtracking DFS: repeatedly pick the unassigned package with the fewest
/// remaining compatible candidates (MRV / fail-fast), try its candidates
/// highest-version-first, and recurse; a conflict (either against an
/// already-assigned dependency, or an empty candidate set once every
/// accumulated constraint is applied) unwinds to the previous choice point
/// and tries the next candidate there. Termination: version sets are finite
/// and each successful step assigns one more package without revisiting a
/// prior assignment except via backtracking, so the search tree is finite
/// (worst case exponential, as with any such resolver — real SATySFi
/// dependency graphs are small enough that MRV + highest-first keeps this
/// fast in practice).
pub fn solve(root: &[(String, Constraint)], src: &dyn DepSource) -> Result<Solution, Error> {
    let mut state = State {
        assigned: BTreeMap::new(),
        constraints: BTreeMap::new(),
    };
    for (name, c) in root {
        state.constraints.entry(name.clone()).or_default().push(Req {
            constraint: c.clone(),
            requirer: "<root>".to_string(),
        });
    }
    backtrack(&mut state, src)?;
    Ok(Solution {
        packages: state.assigned,
    })
}

/// Pick the unassigned package with a pending constraint that has the fewest
/// matching candidate versions (highest-first), or `None` when every
/// constrained package is already assigned (success). A [`DepSource`] error
/// (e.g. an unknown package name) propagates immediately.
fn select_next(state: &State, src: &dyn DepSource) -> Result<Option<(String, Vec<Version>)>, Error> {
    let mut best: Option<(String, Vec<Version>)> = None;
    for name in state.constraints.keys() {
        if state.assigned.contains_key(name) {
            continue;
        }
        let reqs = &state.constraints[name];
        let mut candidates = src.versions(name)?;
        candidates.sort();
        candidates.reverse(); // highest first
        candidates.retain(|v| reqs.iter().all(|r| r.constraint.matches(v)));
        let is_better = match &best {
            None => true,
            Some((_, best_candidates)) => candidates.len() < best_candidates.len(),
        };
        if is_better {
            let empty = candidates.is_empty();
            best = Some((name.clone(), candidates));
            if empty {
                // Zero candidates is already the minimum possible — no later
                // package can beat it, so stop scanning (fail-fast).
                break;
            }
        }
    }
    Ok(best)
}

/// Recurse: assign one package (the MRV pick) and try each of its candidates
/// in order, undoing a candidate's effects before trying the next. Returns
/// `Ok(())` once every constrained package is assigned; otherwise the most
/// specific [`Error`] encountered while exhausting the current package's
/// candidates.
fn backtrack(state: &mut State, src: &dyn DepSource) -> Result<(), Error> {
    let Some((name, candidates)) = select_next(state, src)? else {
        return Ok(());
    };
    if candidates.is_empty() {
        return Err(diagnose_failure(state, &name));
    }

    let mut last_err: Option<Error> = None;
    for v in candidates {
        state.assigned.insert(name.clone(), v.clone());

        let deps = match src.deps(&name, &v) {
            Ok(d) => d,
            Err(e) => {
                // A hard data error (bad index entry, I/O, …) — not a
                // constraint conflict, so abort the whole search rather than
                // keep trying other candidates.
                state.assigned.remove(&name);
                return Err(e);
            }
        };

        let mut added: Vec<String> = Vec::new();
        let mut immediate_conflict: Option<Error> = None;
        for (dep_name, c) in &deps {
            if let Some(assigned_v) = state.assigned.get(dep_name) {
                if !c.matches(assigned_v) {
                    immediate_conflict = Some(Error::VersionConflict {
                        name: dep_name.clone(),
                        a: format!("{c} (required by {name}@{v})"),
                        b: format!("{assigned_v} (already resolved)"),
                    });
                    break;
                }
            }
            state
                .constraints
                .entry(dep_name.clone())
                .or_default()
                .push(Req {
                    constraint: c.clone(),
                    requirer: format!("{name}@{v}"),
                });
            added.push(dep_name.clone());
        }

        let result = match immediate_conflict {
            Some(e) => Err(e),
            None => backtrack(state, src),
        };

        match result {
            Ok(()) => return Ok(()),
            Err(e) => {
                for dep_name in added.iter().rev() {
                    if let Some(list) = state.constraints.get_mut(dep_name) {
                        list.pop();
                        if list.is_empty() {
                            state.constraints.remove(dep_name);
                        }
                    }
                }
                state.assigned.remove(&name);
                last_err = Some(e);
            }
        }
    }
    Err(last_err.unwrap_or_else(|| diagnose_failure(state, &name)))
}

/// Explain why `name` has no viable candidate: if two or more of its
/// accumulated requirements pin different compat buckets (design §3.3's
/// `Constraint::pinned_bucket`), that mutual incompatibility is the root
/// cause ([`Error::VersionConflict`]); otherwise every requirement shares a
/// bucket but no *published* version satisfies all of them at once
/// ([`Error::Unsatisfiable`]).
fn diagnose_failure(state: &State, name: &str) -> Error {
    let reqs = &state.constraints[name];
    let bucketed: Vec<&Req> = reqs.iter().filter(|r| r.constraint.pinned_bucket().is_some()).collect();
    for i in 0..bucketed.len() {
        for j in (i + 1)..bucketed.len() {
            if bucketed[i].constraint.pinned_bucket() != bucketed[j].constraint.pinned_bucket() {
                return Error::VersionConflict {
                    name: name.to_string(),
                    a: format!("{} (via {})", bucketed[i].constraint, bucketed[i].requirer),
                    b: format!("{} (via {})", bucketed[j].constraint, bucketed[j].requirer),
                };
            }
        }
    }
    Error::Unsatisfiable {
        name: name.to_string(),
        constraints: reqs.iter().map(|r| r.constraint.to_string()).collect(),
        requirers: reqs.iter().map(|r| r.requirer.clone()).collect(),
    }
}
