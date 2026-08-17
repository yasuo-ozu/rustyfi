//! Solver tests (design §6 Slice B) over a plain in-memory [`Graph`]
//! `DepSource` — no filesystem, no registry. Cases mirror the shapes
//! upstream's `saphe-split:test/saphe/packageConstraintSolverTest.ml`
//! exercises (that file is not vendored into this port; these are
//! equivalent constructions per the shapes named in
//! `docs/plans/design-saphe-solver.md` §6): a linear chain, a diamond that
//! shares a common compatible version, a diamond that forces the solver to
//! backtrack off its first (higher, conflicting) candidate, a genuine
//! compat-bucket conflict, a reference to an unknown package, and a
//! highest-version preference check.

use std::collections::HashMap;

use rustyfi_satyrographos::{solve, Constraint, DepSource, Error, Version};

// ---------------------------------------------------------------------------
// In-memory dependency graph fixture.
// ---------------------------------------------------------------------------

struct Graph {
    packages: HashMap<String, Vec<(Version, Vec<(String, Constraint)>)>>,
}

impl Graph {
    fn new() -> Self {
        Graph {
            packages: HashMap::new(),
        }
    }

    /// Register `name`'s versions: each `(version_str, deps)` where `deps` is
    /// a list of `(dep_name, constraint_str)`.
    fn pkg(mut self, name: &str, versions: Vec<(&str, Vec<(&str, &str)>)>) -> Self {
        let entries = versions
            .into_iter()
            .map(|(v, deps)| {
                let ver = Version::parse(v).expect("fixture version parses");
                let deps = deps
                    .into_iter()
                    .map(|(n, c)| (n.to_string(), Constraint::parse(c).expect("fixture constraint parses")))
                    .collect();
                (ver, deps)
            })
            .collect();
        self.packages.insert(name.to_string(), entries);
        self
    }
}

impl DepSource for Graph {
    fn versions(&self, name: &str) -> Result<Vec<Version>, Error> {
        self.packages
            .get(name)
            .map(|entries| entries.iter().map(|(v, _)| v.clone()).collect())
            .ok_or_else(|| Error::PackageNotFound {
                name: name.to_string(),
            })
    }

    fn deps(&self, name: &str, v: &Version) -> Result<Vec<(String, Constraint)>, Error> {
        let entries = self.packages.get(name).ok_or_else(|| Error::PackageNotFound {
            name: name.to_string(),
        })?;
        entries
            .iter()
            .find(|(ev, _)| ev == v)
            .map(|(_, deps)| deps.clone())
            .ok_or_else(|| Error::VersionNotFound {
                name: name.to_string(),
                version: v.to_string(),
            })
    }
}

fn root(pairs: &[(&str, &str)]) -> Vec<(String, Constraint)> {
    pairs
        .iter()
        .map(|(n, c)| (n.to_string(), Constraint::parse(c).unwrap()))
        .collect()
}

fn v(s: &str) -> Version {
    Version::parse(s).unwrap()
}

// ---------------------------------------------------------------------------
// Satisfiable graphs.
// ---------------------------------------------------------------------------

#[test]
fn linear_chain_resolves() {
    let g = Graph::new()
        .pkg("a", vec![("1.0.0", vec![("b", "^1.0.0")])])
        .pkg("b", vec![("1.0.0", vec![("c", "^1.0.0")])])
        .pkg("c", vec![("1.0.0", vec![])]);

    let sol = solve(&root(&[("a", "^1.0.0")]), &g).expect("linear chain solves");
    assert_eq!(sol.packages.get("a"), Some(&v("1.0.0")));
    assert_eq!(sol.packages.get("b"), Some(&v("1.0.0")));
    assert_eq!(sol.packages.get("c"), Some(&v("1.0.0")));
    assert_eq!(sol.packages.len(), 3);
}

#[test]
fn diamond_shares_common_compatible_version_and_prefers_highest() {
    // a -> b, c ; b -> d ^1.0.0 ; c -> d ^1.0.0 ; d has 1.0.0 and 1.1.0.
    let g = Graph::new()
        .pkg("a", vec![("1.0.0", vec![("b", "^1.0.0"), ("c", "^1.0.0")])])
        .pkg("b", vec![("1.0.0", vec![("d", "^1.0.0")])])
        .pkg("c", vec![("1.0.0", vec![("d", "^1.0.0")])])
        .pkg("d", vec![("1.0.0", vec![]), ("1.1.0", vec![])]);

    let sol = solve(&root(&[("a", "^1.0.0")]), &g).expect("diamond solves");
    assert_eq!(sol.packages.get("d"), Some(&v("1.1.0")), "prefers the highest common version");
}

#[test]
fn diamond_backtracks_off_a_conflicting_high_candidate() {
    // root -> a (any), b (any).
    // b has one version, forcing d to 1.0.0.
    // a has two versions: 2.0.0 (needs d ^2.0.0 -- conflicts with the
    // already-resolved d 1.0.0) and 1.0.0 (needs d ^1.0.0 -- fine). The
    // highest-first candidate order means the solver must try (and reject)
    // a@2.0.0 before backtracking to a@1.0.0.
    let g = Graph::new()
        .pkg(
            "a",
            vec![
                ("2.0.0", vec![("d", "^2.0.0")]),
                ("1.0.0", vec![("d", "^1.0.0")]),
            ],
        )
        .pkg("b", vec![("1.0.0", vec![("d", "^1.0.0")])])
        .pkg("d", vec![("1.0.0", vec![]), ("2.0.0", vec![])]);

    let sol = solve(&root(&[("a", "*"), ("b", "*")]), &g).expect("backtrack finds a solution");
    assert_eq!(sol.packages.get("a"), Some(&v("1.0.0")), "backtracked off the conflicting 2.0.0");
    assert_eq!(sol.packages.get("b"), Some(&v("1.0.0")));
    assert_eq!(sol.packages.get("d"), Some(&v("1.0.0")));
}

#[test]
fn prefers_highest_compatible_version() {
    let g = Graph::new().pkg(
        "a",
        vec![("1.0.0", vec![]), ("1.2.0", vec![]), ("1.1.0", vec![])],
    );
    let sol = solve(&root(&[("a", "^1.0.0")]), &g).expect("solves");
    assert_eq!(sol.packages.get("a"), Some(&v("1.2.0")));
}

// ---------------------------------------------------------------------------
// Unsatisfiable / conflicting graphs.
// ---------------------------------------------------------------------------

#[test]
fn bucket_conflict_is_unsatisfiable() {
    // a -> d ^1.0.0 ; b -> d ^2.0.0 : disjoint compat buckets, never
    // satisfiable by any single version of d.
    let g = Graph::new()
        .pkg("a", vec![("1.0.0", vec![("d", "^1.0.0")])])
        .pkg("b", vec![("1.0.0", vec![("d", "^2.0.0")])])
        .pkg("d", vec![("1.0.0", vec![]), ("2.0.0", vec![])]);

    let err = solve(&root(&[("a", "*"), ("b", "*")]), &g).expect_err("bucket conflict must fail");
    match err {
        Error::VersionConflict { name, .. } => assert_eq!(name, "d"),
        other => panic!("expected VersionConflict, got {other:?}"),
    }
}

#[test]
fn zero_major_minor_lock_is_also_a_bucket_conflict() {
    // Two 0.x requirements at different minors are different buckets too.
    let g = Graph::new()
        .pkg("a", vec![("1.0.0", vec![("d", "^0.3.0")])])
        .pkg("b", vec![("1.0.0", vec![("d", "^0.4.0")])])
        .pkg("d", vec![("0.3.0", vec![]), ("0.4.0", vec![])]);

    let err = solve(&root(&[("a", "*"), ("b", "*")]), &g).expect_err("0.x minor mismatch conflicts");
    assert!(matches!(err, Error::VersionConflict { name, .. } if name == "d"));
}

#[test]
fn same_bucket_but_no_published_version_covers_both_is_unsatisfiable_not_conflict() {
    // Both requirements pin the SAME bucket (major 1), but no single
    // published version satisfies both: a wants >=1.0.0 (fine with 1.5.0),
    // b wants exactly 1.9.0 (not published). This is "nothing fits", not a
    // bucket clash, so it must come back as Unsatisfiable.
    let g = Graph::new()
        .pkg("a", vec![("1.0.0", vec![("d", "^1.5.0")])])
        .pkg("b", vec![("1.0.0", vec![("d", "1.9.0")])])
        .pkg("d", vec![("1.5.0", vec![]), ("1.6.0", vec![])]);

    let err = solve(&root(&[("a", "*"), ("b", "*")]), &g).expect_err("no version covers both");
    match err {
        Error::Unsatisfiable { name, .. } => assert_eq!(name, "d"),
        other => panic!("expected Unsatisfiable, got {other:?}"),
    }
}

#[test]
fn missing_package_reports_package_not_found() {
    let g = Graph::new().pkg("a", vec![("1.0.0", vec![("ghost", "^1.0.0")])]);
    let err = solve(&root(&[("a", "*")]), &g).expect_err("ghost dependency is unresolvable");
    assert!(matches!(err, Error::PackageNotFound { name } if name == "ghost"));
}

#[test]
fn missing_root_package_reports_package_not_found() {
    let g = Graph::new();
    let err = solve(&root(&[("nowhere", "*")]), &g).expect_err("root package missing");
    assert!(matches!(err, Error::PackageNotFound { name } if name == "nowhere"));
}
