//! The open (document-level) resolver — `saphe-split @ b836d512`,
//! `src/frontend/openFileDependencyResolver.ml`.
//!
//! Walks an entry document and its transitive `` use … of `path` `` local
//! dependencies, returning them dependency-first (the same shape/invariant as
//! the Legacy backend's [`crate::load_legacy`], so
//! `satysfi_lang::compile_document_v1` consumes the output unchanged). The
//! other two header families are typed errors here, matching upstream:
//!
//! - `use package Mod` (upstream records an envelope edge) — Ld3a has no
//!   envelope graph (`deps: None`), so it is
//!   [`LoadError::PackageDependencyUnresolved`]; Ld3b replaces this arm with a
//!   `used_as`-map lookup.
//! - bare `use Mod` (upstream `CannotUseHeaderUse`) —
//!   [`LoadError::BareUseOutsidePackage`], a *permanent* error at document
//!   level.
//! - `@require:`/`@import:` (upstream's lexer never lexes `@`) —
//!   [`LoadError::LegacyHeaderUnderEnvelopes`], this port's documented
//!   divergence (Ld3a spec §4.1: the error moves from lex-time to load-time).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use satysfi_syntax::cst_v1::{FileV1, HeaderV1};

use crate::{
    alloc_id, canonicalize, graph, LoadError, LoadOptions, LoadedCst, LoadedFile, LoadedProgram,
};

/// Candidate extensions for `` use … of `rel` ``, PDF mode — upstream
/// `get_candidate_file_extensions PdfMode = [".satyh"; ".satyg"]`, the same
/// list and order as 0.0.6. Declared here rather than shared with `v006`'s
/// `CANDIDATE_EXTS` so the two backends can diverge later (e.g. when text-mode
/// `.satyh-<mode>` extensions land for one but not the other).
const CANDIDATE_EXTS: [&str; 2] = [".satyh", ".satyg"];

/// Envelopes-mode load of a document whose dependencies are all
/// `` use … of `path` `` local files. `deps` is the `--deps <satysfi-deps.yaml>`
/// path (Ld3a: any `Some(_)` errors with a named Ld3b follow-up).
pub(crate) fn load(
    entry: &Path,
    deps: Option<&Path>,
    _opts: &LoadOptions,
) -> Result<LoadedProgram, LoadError> {
    // Deps gate (Ld3a stub): a supplied deps config is not consumed yet — the
    // documented cut line to Ld3b. A named, testable error, not a `todo!()`.
    if let Some(path) = deps {
        return Err(LoadError::DepsConfigUnsupported {
            path: path.to_path_buf(),
        });
    }

    let entry_canon = canonicalize(entry)?;

    let mut next_id: u32 = 0;
    let mut id_of: HashMap<PathBuf, u32> = HashMap::new();
    let mut path_of: HashMap<u32, PathBuf> = HashMap::new();
    let mut cst_of: HashMap<u32, LoadedCst> = HashMap::new();
    let mut adjacency: HashMap<u32, Vec<u32>> = HashMap::new();
    let mut processed: HashSet<u32> = HashSet::new();

    let entry_id = alloc_id(entry_canon, &mut next_id, &mut id_of, &mut path_of);

    let mut worklist = vec![entry_id];
    while let Some(id) = worklist.pop() {
        if processed.contains(&id) {
            continue;
        }
        processed.insert(id);

        let path = path_of[&id].clone();
        let src = std::fs::read_to_string(&path).map_err(|source| LoadError::Io {
            path: path.clone(),
            source,
        })?;
        // The entry (and every local `use … of`ed file) is always parsed with
        // the `V0_1` grammar — `load()`'s guard already pinned `version ==
        // V0_1` before dispatching here.
        let file = satysfi_syntax::parse_file_v1(&src).map_err(|source| LoadError::Parse {
            path: path.clone(),
            source,
        })?;

        // Entry must be a document (upstream `DocumentLacksWholeReturnValue`);
        // every local dependency must be a library (upstream
        // `LibraryContainsWholeReturnValue`).
        let is_document = matches!(file, FileV1::Document { .. });
        if id == entry_id {
            if !is_document {
                return Err(LoadError::LibraryAsEntry { path });
            }
        } else if is_document {
            return Err(LoadError::DocumentAsDependency { path });
        }

        let dir = path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));

        let headers: &[HeaderV1] = match &file {
            FileV1::Document { headers, .. } | FileV1::Library { headers, .. } => headers,
        };

        let mut resolved_deps: Vec<PathBuf> = Vec::new();
        for header in headers {
            match header {
                // `use … of `rel`` — resolve the backtick relpath against the
                // *referencing* file's directory over the candidate
                // extensions; first existing wins. The header's module name is
                // deliberately ignored (upstream `Local(_modident_sub, ..)`):
                // name/shape checking is typecheck's job, not the loader's.
                HeaderV1::UseOf { relpath, .. } => {
                    let resolved = resolve_use_of(&dir, &relpath.body).ok_or_else(|| {
                        LoadError::UnresolvedUseOf {
                            relpath: relpath.body.clone(),
                            from: path.clone(),
                            searched: candidates(&dir, &relpath.body),
                        }
                    })?;
                    resolved_deps.push(resolved);
                }
                // `use package Mod` — no deps config (Ld3a), so there is no
                // used_as map to validate against: any package dependency is
                // unresolvable. Ld3b replaces this with the used_as lookup.
                HeaderV1::UsePackage { path: modpath, .. } => {
                    return Err(LoadError::PackageDependencyUnresolved {
                        module: modpath.head_name(),
                        from: path.clone(),
                    });
                }
                // bare `use Mod` — upstream `CannotUseHeaderUse`; permanent.
                HeaderV1::Use { path: modpath, .. } => {
                    return Err(LoadError::BareUseOutsidePackage {
                        module: modpath.head_name(),
                        from: path.clone(),
                    });
                }
                // `@require:`/`@import:` under Envelopes — mode error.
                HeaderV1::Legacy(_) => {
                    return Err(LoadError::LegacyHeaderUnderEnvelopes {
                        header: header.display_name(),
                        from: path.clone(),
                    });
                }
            }
        }

        let mut dep_ids = Vec::new();
        for resolved in resolved_deps {
            let dep_canon = canonicalize(&resolved)?;
            let dep_id = alloc_id(dep_canon, &mut next_id, &mut id_of, &mut path_of);
            dep_ids.push(dep_id);
            worklist.push(dep_id);
        }

        adjacency.insert(id, dep_ids);
        cst_of.insert(id, LoadedCst::V0_1(file));
    }

    // Toposort the local-file graph (upstream `CyclicFileDependency`).
    let order = graph::toposort(&adjacency).map_err(|chain_ids| LoadError::Cycle {
        chain: graph::chain_to_paths(&chain_ids, &path_of),
    })?;

    let files = order
        .into_iter()
        .map(|id| LoadedFile {
            path: path_of[&id].clone(),
            cst: cst_of
                .remove(&id)
                .expect("every graph node id was parsed before toposort"),
        })
        .collect();

    Ok(LoadedProgram { files })
}

/// All paths `` use … of `rel` `` could resolve to under `dir`, without
/// checking existence (used both for resolution and for the `searched` list
/// in [`LoadError::UnresolvedUseOf`]). A relpath that already ends in a
/// candidate extension is taken verbatim; otherwise each extension is tried.
fn candidates(dir: &Path, rel: &str) -> Vec<PathBuf> {
    if CANDIDATE_EXTS.iter().any(|ext| rel.ends_with(ext)) {
        vec![dir.join(rel)]
    } else {
        CANDIDATE_EXTS
            .iter()
            .map(|ext| dir.join(format!("{rel}{ext}")))
            .collect()
    }
}

/// The first existing candidate file for `` use … of `rel` `` under `dir`, or
/// `None` if none exists.
fn resolve_use_of(dir: &Path, rel: &str) -> Option<PathBuf> {
    candidates(dir, rel).into_iter().find(|c| c.is_file())
}
