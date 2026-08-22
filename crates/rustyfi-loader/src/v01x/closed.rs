//! The closed (per-envelope) resolvers — `saphe-split @ b836d512`,
//! `src/frontend/closedFileDependencyResolver.ml` (module-name graph inside
//! one envelope) + `src/frontend/closedEnvelopeDependencyResolver.ml`
//! (envelope-level graph across the deps config). Pure graph logic over the
//! already-decoded configs — no YAML of its own.
//!
//! Both reuse [`crate::graph::toposort`] (u32 adjacency, dependency-first)
//! with a local name↔u32 interning map, mapping cycle ids back to
//! names/paths themselves.

use std::collections::HashMap;
use std::path::PathBuf;

use rustyfi_syntax::cst_v1::{FileV1, HeaderV1};

use crate::error::LoadError;
use crate::graph;
use crate::v01x::deps::{DepsConfig, EnvelopeSpec};
use crate::v01x::envelope::EnvelopeSource;

/// Per-envelope module-name resolution — transcription of
/// `closedFileDependencyResolver.ml:12-66`.
///
/// Vertices are keyed by DECLARED module name (a duplicate is
/// [`LoadError::FileModuleNameConflict`], `:20-22`); edges come from bare
/// `use` headers only (an unknown target is [`LoadError::FileModuleNotFound`],
/// `:37-41`); `use package` is a no-op (`:48-49`); `use … of` is a hard error
/// ([`LoadError::UseOfInsidePackage`], `:51-52`); a Legacy `@`-header is
/// [`LoadError::LegacyHeaderUnderEnvelopes`] (this port's divergence, same as
/// `open_doc`). Returns the sources dependency-first (a cycle is
/// [`LoadError::Cycle`], upstream `CyclicFileDependency`).
pub(crate) fn sort_modules(sources: Vec<EnvelopeSource>) -> Result<Vec<EnvelopeSource>, LoadError> {
    // Intern module names → ids (id == index into `sources`).
    let mut id_of_module: HashMap<String, u32> = HashMap::new();
    let mut path_of: HashMap<u32, PathBuf> = HashMap::new();
    for (i, source) in sources.iter().enumerate() {
        let id = i as u32;
        if let Some(&prev_id) = id_of_module.get(&source.module_name) {
            return Err(LoadError::FileModuleNameConflict {
                module: source.module_name.clone(),
                prev: path_of[&prev_id].clone(),
                path: source.path.clone(),
            });
        }
        id_of_module.insert(source.module_name.clone(), id);
        path_of.insert(id, source.path.clone());
    }

    // Edges from bare `use` headers (dependent -> dependency).
    let mut adjacency: HashMap<u32, Vec<u32>> = HashMap::new();
    for (i, source) in sources.iter().enumerate() {
        let id = i as u32;
        let mut deps: Vec<u32> = Vec::new();
        for header in headers_of(&source.file) {
            match header {
                HeaderV1::Use { path: modpath, .. } => {
                    let target = modpath.head_name();
                    match id_of_module.get(&target) {
                        Some(&dep_id) => deps.push(dep_id),
                        None => {
                            return Err(LoadError::FileModuleNotFound {
                                module: target,
                                from: source.path.clone(),
                            });
                        }
                    }
                }
                // `use package` is resolved at the open (document) level, not
                // inside an envelope — a no-op for the closed graph.
                HeaderV1::UsePackage { .. } => {}
                HeaderV1::UseOf { path: modpath, .. } => {
                    return Err(LoadError::UseOfInsidePackage {
                        module: modpath.head_name(),
                        from: source.path.clone(),
                    });
                }
                HeaderV1::Legacy(_) => {
                    return Err(LoadError::LegacyHeaderUnderEnvelopes {
                        header: header.display_name(),
                        from: source.path.clone(),
                    });
                }
            }
        }
        adjacency.insert(id, deps);
    }

    let order = graph::toposort(&adjacency).map_err(|chain_ids| LoadError::Cycle {
        chain: graph::chain_to_paths(&chain_ids, &path_of),
    })?;

    let mut by_id: HashMap<u32, EnvelopeSource> = sources
        .into_iter()
        .enumerate()
        .map(|(i, s)| (i as u32, s))
        .collect();
    Ok(order
        .into_iter()
        .map(|id| {
            by_id
                .remove(&id)
                .expect("every source id was interned before toposort")
        })
        .collect())
}

/// Envelope-level ordering — transcription of
/// `closedEnvelopeDependencyResolver.ml:22-91` MINUS the eager
/// `EnvelopeReader.main` call per vertex (this port reads each envelope in
/// `open_doc` after ordering — lazy; upstream reads-then-sorts, same set,
/// same result).
///
/// `test_only` envelopes are skipped (`:38-40`, always — no `saphe test`
/// yet); a duplicate name is [`LoadError::EnvelopeNameConflict`] (`:50-51`);
/// each `spec.dependencies[].name` is an edge (an unknown target is
/// [`LoadError::DependencyOnUnknownEnvelope`], `:71-76`; `used_as` is
/// deliberately unused — upstream's own TODO, `:69`); a cycle is
/// [`LoadError::CyclicEnvelopeDependency`]. Returns the (non-test) specs
/// dependency-first.
pub(crate) fn sort_envelopes(deps: &DepsConfig) -> Result<Vec<&EnvelopeSpec>, LoadError> {
    // Intern (non-test) envelope names → ids (id == index into `specs`).
    let mut id_of_name: HashMap<String, u32> = HashMap::new();
    let mut name_of: HashMap<u32, String> = HashMap::new();
    let mut specs: Vec<&EnvelopeSpec> = Vec::new();
    for spec in &deps.envelopes {
        if spec.test_only {
            continue;
        }
        if id_of_name.contains_key(&spec.name) {
            return Err(LoadError::EnvelopeNameConflict {
                name: spec.name.clone(),
            });
        }
        let id = specs.len() as u32;
        id_of_name.insert(spec.name.clone(), id);
        name_of.insert(id, spec.name.clone());
        specs.push(spec);
    }

    // Edges per dependency name (dependent -> dependency).
    let mut adjacency: HashMap<u32, Vec<u32>> = HashMap::new();
    for (i, spec) in specs.iter().enumerate() {
        let id = i as u32;
        let mut dep_ids: Vec<u32> = Vec::new();
        for dep in &spec.dependencies {
            match id_of_name.get(&dep.name) {
                Some(&dep_id) => dep_ids.push(dep_id),
                None => {
                    return Err(LoadError::DependencyOnUnknownEnvelope {
                        depending: spec.name.clone(),
                        depended: dep.name.clone(),
                    });
                }
            }
        }
        adjacency.insert(id, dep_ids);
    }

    let order =
        graph::toposort(&adjacency).map_err(|chain_ids| LoadError::CyclicEnvelopeDependency {
            chain: chain_ids.iter().map(|id| name_of[id].clone()).collect(),
        })?;

    Ok(order.into_iter().map(|id| specs[id as usize]).collect())
}

/// A file's header slice, for either `FileV1` shape (envelope sources are
/// always libraries, but this stays total).
fn headers_of(file: &FileV1) -> &[HeaderV1] {
    match file {
        FileV1::Document { headers, .. } | FileV1::Library { headers, .. } => headers,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v01x::deps::EnvelopeDependency;

    /// Build an [`EnvelopeSource`] from inline source text, parsed as V0_1.
    fn source(path: &str, src: &str) -> EnvelopeSource {
        let file = rustyfi_syntax::parse_file_v1(src)
            .unwrap_or_else(|e| panic!("{path}: parse failed: {e}"));
        let module_name = match &file {
            FileV1::Library { name, .. } => name.name.clone(),
            FileV1::Document { .. } => panic!("{path}: expected a library"),
        };
        EnvelopeSource {
            path: PathBuf::from(path),
            file,
            module_name,
        }
    }

    fn spec(name: &str, deps: &[&str]) -> EnvelopeSpec {
        EnvelopeSpec {
            name: name.to_string(),
            path: PathBuf::from(format!("/synthetic/{name}/rustyfi-envelope.yaml")),
            dependencies: deps
                .iter()
                .map(|d| EnvelopeDependency {
                    name: d.to_string(),
                    // `used_as` is unused by `sort_envelopes`; any uppercased
                    // identifier is fine for the test.
                    used_as: "Dep".to_string(),
                })
                .collect(),
            test_only: false,
        }
    }

    fn module_names(sorted: &[EnvelopeSource]) -> Vec<String> {
        sorted.iter().map(|s| s.module_name.clone()).collect()
    }

    fn envelope_names(sorted: &[&EnvelopeSpec]) -> Vec<String> {
        sorted.iter().map(|s| s.name.clone()).collect()
    }

    #[test]
    fn sort_modules_orders_bare_use_dependency_first() {
        // C uses B, B uses A → A before B before C regardless of input order.
        let sources = vec![
            source("c.satyh", "use B\nmodule C = struct\nval c = 1\nend"),
            source("b.satyh", "use A\nmodule B = struct\nval b = 1\nend"),
            source("a.satyh", "module A = struct\nval a = 1\nend"),
        ];
        let sorted = sort_modules(sources).expect("acyclic module graph sorts");
        assert_eq!(module_names(&sorted), vec!["A", "B", "C"]);
    }

    #[test]
    fn sort_modules_no_edges_keeps_all() {
        let sources = vec![
            source("a.satyh", "module A = struct\nval a = 1\nend"),
            source("b.satyh", "module B = struct\nval b = 1\nend"),
        ];
        let sorted = sort_modules(sources).expect("independent modules sort");
        assert_eq!(sorted.len(), 2);
    }

    #[test]
    fn sort_modules_duplicate_module_name_conflicts() {
        let sources = vec![
            source("a.satyh", "module A = struct\nval a = 1\nend"),
            source("a2.satyh", "module A = struct\nval a = 2\nend"),
        ];
        match sort_modules(sources) {
            Err(LoadError::FileModuleNameConflict { module, .. }) => assert_eq!(module, "A"),
            other => panic!("expected FileModuleNameConflict, got {other:?}"),
        }
    }

    #[test]
    fn sort_modules_unknown_bare_use_is_not_found() {
        let sources = vec![source(
            "a.satyh",
            "use Missing\nmodule A = struct\nval a = 1\nend",
        )];
        match sort_modules(sources) {
            Err(LoadError::FileModuleNotFound { module, .. }) => assert_eq!(module, "Missing"),
            other => panic!("expected FileModuleNotFound, got {other:?}"),
        }
    }

    #[test]
    fn sort_modules_use_of_inside_package_rejected() {
        let sources = vec![source(
            "a.satyh",
            "use B of `./b`\nmodule A = struct\nval a = 1\nend",
        )];
        match sort_modules(sources) {
            Err(LoadError::UseOfInsidePackage { module, .. }) => assert_eq!(module, "B"),
            other => panic!("expected UseOfInsidePackage, got {other:?}"),
        }
    }

    #[test]
    fn sort_modules_cycle_detected() {
        let sources = vec![
            source("a.satyh", "use B\nmodule A = struct\nval a = 1\nend"),
            source("b.satyh", "use A\nmodule B = struct\nval b = 1\nend"),
        ];
        match sort_modules(sources) {
            Err(LoadError::Cycle { chain }) => assert!(chain.len() >= 2, "chain: {chain:?}"),
            other => panic!("expected Cycle, got {other:?}"),
        }
    }

    #[test]
    fn sort_modules_use_package_is_a_no_op() {
        // `use package X` inside an envelope contributes no closed edge.
        let sources = vec![source(
            "a.satyh",
            "use package X\nmodule A = struct\nval a = 1\nend",
        )];
        let sorted = sort_modules(sources).expect("use package is a no-op in the closed graph");
        assert_eq!(module_names(&sorted), vec!["A"]);
    }

    #[test]
    fn sort_envelopes_orders_dependency_first() {
        // B depends on A; listed B-first → A ordered before B.
        let cfg = DepsConfig {
            envelopes: vec![spec("B", &["A"]), spec("A", &[])],
            explicit_dependencies: vec![],
            explicit_test_dependencies: vec![],
        };
        let sorted = sort_envelopes(&cfg).expect("acyclic envelope graph sorts");
        assert_eq!(envelope_names(&sorted), vec!["A", "B"]);
    }

    #[test]
    fn sort_envelopes_skips_test_only() {
        let mut t = spec("T", &[]);
        t.test_only = true;
        let cfg = DepsConfig {
            envelopes: vec![spec("A", &[]), t],
            explicit_dependencies: vec![],
            explicit_test_dependencies: vec![],
        };
        let sorted = sort_envelopes(&cfg).expect("test-only envelope is skipped");
        assert_eq!(envelope_names(&sorted), vec!["A"]);
    }

    #[test]
    fn sort_envelopes_duplicate_name_conflicts() {
        let cfg = DepsConfig {
            envelopes: vec![spec("A", &[]), spec("A", &[])],
            explicit_dependencies: vec![],
            explicit_test_dependencies: vec![],
        };
        match sort_envelopes(&cfg) {
            Err(LoadError::EnvelopeNameConflict { name }) => assert_eq!(name, "A"),
            other => panic!("expected EnvelopeNameConflict, got {other:?}"),
        }
    }

    #[test]
    fn sort_envelopes_unknown_dependency() {
        let cfg = DepsConfig {
            envelopes: vec![spec("A", &["Nope"])],
            explicit_dependencies: vec![],
            explicit_test_dependencies: vec![],
        };
        match sort_envelopes(&cfg) {
            Err(LoadError::DependencyOnUnknownEnvelope {
                depending,
                depended,
            }) => {
                assert_eq!(depending, "A");
                assert_eq!(depended, "Nope");
            }
            other => panic!("expected DependencyOnUnknownEnvelope, got {other:?}"),
        }
    }

    #[test]
    fn sort_envelopes_cycle_detected() {
        let cfg = DepsConfig {
            envelopes: vec![spec("A", &["B"]), spec("B", &["A"])],
            explicit_dependencies: vec![],
            explicit_test_dependencies: vec![],
        };
        match sort_envelopes(&cfg) {
            Err(LoadError::CyclicEnvelopeDependency { chain }) => {
                assert!(chain.len() >= 2, "chain: {chain:?}")
            }
            other => panic!("expected CyclicEnvelopeDependency, got {other:?}"),
        }
    }
}
