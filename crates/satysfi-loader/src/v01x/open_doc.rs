//! The open (document-level) resolver — `saphe-split @ b836d512`,
//! `src/frontend/openFileDependencyResolver.ml`.
//!
//! Walks an entry document and its transitive `` use … of `path` `` local
//! dependencies, returning them dependency-first (the same shape/invariant as
//! the Legacy backend's [`crate::load_legacy`], so
//! `satysfi_lang::compile_document_v1` consumes the output unchanged). The
//! other two header families are typed errors here, matching upstream:
//!
//! - `use package Mod` (upstream records an envelope edge) — with `deps:
//!   None` there is no `used_as` map, so it is
//!   [`LoadError::PackageDependencyUnresolved`]; with `deps: Some(_)` (Ld3b-2)
//!   the head is looked up in the config's `used_as` map (miss →
//!   [`LoadError::UnknownPackageDependency`]) and, on a hit, records nothing
//!   (the envelope's files are already prepended to the program).
//! - bare `use Mod` (upstream `CannotUseHeaderUse`) —
//!   [`LoadError::BareUseOutsidePackage`], a *permanent* error at document
//!   level.
//! - `@require:`/`@import:` (upstream's lexer never lexes `@`) —
//!   [`LoadError::LegacyHeaderUnderEnvelopes`], this port's documented
//!   divergence (Ld3a spec §4.1: the error moves from lex-time to load-time).
//!
//! Ld3b-2 adds the deps-config phase (upstream `build_document`,
//! `main.ml:285-316`): decode → envelope topo-sort → read + closed-sort each
//! envelope → prepend its modules → local walk → validated `use package`.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use satysfi_syntax::cst_v1::{FileV1, HeaderV1};
use satysfi_syntax::SatysfiVersion;

use crate::v01x::{closed, deps as deps_mod, envelope};
use crate::{
    alloc_id, canonicalize, graph, FileOrigin, LoadError, LoadOptions, LoadedCst, LoadedFile,
    LoadedProgram,
};

/// Candidate extensions for `` use … of `rel` ``, PDF mode — upstream
/// `get_candidate_file_extensions PdfMode = [".satyh"; ".satyg"]`, the same
/// list and order as 0.0.6. Declared here rather than shared with `v006`'s
/// `CANDIDATE_EXTS` so the two backends can diverge later (e.g. when text-mode
/// `.satyh-<mode>` extensions land for one but not the other).
const CANDIDATE_EXTS: [&str; 2] = [".satyh", ".satyg"];

/// Envelopes-mode load: a `--deps <satysfi-deps.yaml>` envelope graph (when
/// `deps` is `Some`) prepended before the document's local `` use … of `path` ``
/// files. Mirrors upstream's `build_document` load pipeline
/// (`main.ml:285-316`): decode the deps config, topo-sort its envelopes,
/// read + closed-sort each envelope's modules into a dependency-first prefix,
/// then walk the entry document and its local files, validating `use package`
/// headers against the config's `used_as` aliases.
pub(crate) fn load(
    entry: &Path,
    deps: Option<&Path>,
    _opts: &LoadOptions,
) -> Result<LoadedProgram, LoadError> {
    // 1. Decode the deps config (upstream `main.ml:287`). 2. Its `used_as`
    //    map (`main.ml:288` / `make_used_as_map`), keyed by the consumer's
    //    alias, valued by the envelope name (later duplicate wins).
    let deps_config = deps.map(deps_mod::load).transpose()?;
    let used_as_map = deps_config
        .as_ref()
        .map(|c| deps_mod::make_used_as_map(&c.explicit_dependencies));

    // 3. Envelope phase (only with a deps config): topo-sort the envelopes,
    //    then read + closed-sort each in dependency-first order, prepending
    //    its module sources to `prefix`. Envelope files are NOT local-walk
    //    worklist nodes — their per-envelope order comes from the closed
    //    resolver, their global order from the envelope resolver. (This port
    //    keeps the two worlds' paths separate, exactly as upstream does in
    //    practice — a local `use … of` reaching into an envelope tree would
    //    load a second copy, but no realistic input does that.)
    let mut prefix: Vec<LoadedFile> = Vec::new();
    if let Some(config) = &deps_config {
        for spec in closed::sort_envelopes(config)? {
            let read = envelope::read(&spec.path)?;
            let sorted = closed::sort_modules(read.sources)?;
            for source in sorted {
                prefix.push(LoadedFile {
                    path: source.path,
                    cst: LoadedCst::V0_1(source.file),
                    origin: FileOrigin::Envelope {
                        envelope: spec.name.clone(),
                        module: source.module_name,
                    },
                    // Envelopes mode is `V0_1`-only (`load`'s guard) — every
                    // file it produces is `V0_1`, unconditionally.
                    version: SatysfiVersion::V0_1,
                });
            }
        }
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
                // `use package Mod` — with a deps config, validate the head
                // against the `used_as` map and record nothing (the envelope's
                // files are already in `prefix`; upstream records no file
                // edge either, `openFileDependencyResolver.ml:90-91,116-117`).
                // Without one, there is no map to validate against, so any
                // package dependency is unresolvable.
                HeaderV1::UsePackage { path: modpath, .. } => {
                    let head = modpath.head_name();
                    match &used_as_map {
                        Some(map) => {
                            if !map.contains_key(&head) {
                                return Err(LoadError::UnknownPackageDependency {
                                    module: head,
                                    from: path.clone(),
                                });
                            }
                        }
                        None => {
                            return Err(LoadError::PackageDependencyUnresolved {
                                module: head,
                                from: path.clone(),
                            });
                        }
                    }
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

    // Assemble: envelope sources (already dependency-first) first, then the
    // local files dependency-first with the entry document last — upstream's
    // `libs_dep ++ libs_local` then the document (`main.ml:323`). Local files
    // and the entry are `FileOrigin::Local`.
    let mut files = prefix;
    files.extend(order.into_iter().map(|id| LoadedFile {
        path: path_of[&id].clone(),
        cst: cst_of
            .remove(&id)
            .expect("every graph node id was parsed before toposort"),
        origin: FileOrigin::Local,
        // Envelopes mode is `V0_1`-only (`load`'s guard) — every file it
        // produces is `V0_1`, unconditionally.
        version: SatysfiVersion::V0_1,
    }));

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
