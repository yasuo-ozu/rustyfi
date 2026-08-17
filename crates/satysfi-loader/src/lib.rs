//! Multi-file loading layer for SATySFi documents: resolves `@require:` /
//! `@import:` headers to files on disk, recursively parses the whole
//! dependency graph, and returns it in dependency-first (topological) load
//! order.
//!
//! Transcribed from v0.0.6's `src/frontend/main.ml` (lines ~95-140):
//!
//! - `@import: name` resolves relative to the directory of the file
//!   *containing* the header (not the entry document's directory).
//! - `@require: name` resolves against the package/library root
//!   (`LoadOptions::lib_root`).
//! - Candidate extensions, tried in order: `.satyh`, then `.satyg` (the
//!   mode-specific `.satyh-<mode>` extensions from `main.ml` are out of
//!   scope here).
//! - The same file reached through two different headers is one graph node
//!   (deduplicated by canonical path).
//! - Every dependency must be a library file (`body: None`); the entry must
//!   be a document (`body: Some(..)`).
//! - A cycle in the dependency graph is an error naming the files involved.

mod error;
mod v006;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

pub use error::LoadError;
pub use satysfi_syntax::SatysfiVersion;

/// Options controlling header resolution.
///
/// Implements [`Default`] (rather than requiring every field to be named at
/// each construction site) precisely so that adding `version` here did not
/// force edits to every existing `LoadOptions { lib_root: ... }` call site —
/// they now read `LoadOptions { lib_root: ..., ..Default::default() }`.
#[derive(Default)]
pub struct LoadOptions {
    /// Root used to resolve `@require: name` (searched as
    /// `<lib_root>/dist/packages/name.{satyh,satyg}`, then
    /// `<lib_root>/name.{satyh,satyg}`, then the nested
    /// `<lib_root>/dist/packages/name/name.{satyh,satyg}` layout the
    /// Satyrographos installer produces — see `resolve::resolve_require`).
    /// `None` means there is no package root configured, so any `@require:`
    /// header fails to resolve.
    pub lib_root: Option<PathBuf>,
    /// The SATySFi language version the input is expected to conform to.
    /// Defaults to [`SatysfiVersion::DEFAULT`] (0.0.6, the only version this
    /// loader implements). [`load`] rejects any version for which
    /// [`SatysfiVersion::is_implemented`] is false before doing any work.
    pub version: SatysfiVersion,
}

/// A parsed file's CST, tagged by which grammar generation produced it.
/// `load()` picks the variant per `LoadOptions::version` (`V0_0_6` →
/// `V0_0_6`, `V0_1` → `V0_1`) — see `load`'s dispatch, below. Every file
/// in one `LoadedProgram` carries the same variant, since `opts.version` is
/// one value for the whole load; the enum exists so `LoadedFile` has a
/// single field type rather than forcing every consumer of `LoadedProgram`
/// to be generic over the CST type.
#[derive(Debug)]
pub enum LoadedCst {
    V0_0_6(satysfi_syntax::cst::File),
    V0_1(satysfi_syntax::cst_v1::FileV1),
}

impl LoadedCst {
    /// Whether this file is a document (has a body) rather than a library.
    /// Used by `load()`'s entry/dependency-shape validation
    /// (`DocumentAsDependency`/`LibraryAsEntry`) uniformly across both
    /// generations, so that validation logic itself needs no `match` at its
    /// call sites.
    pub fn is_document(&self) -> bool {
        match self {
            Self::V0_0_6(f) => f.body.is_some(),
            Self::V0_1(f) => matches!(f, satysfi_syntax::cst_v1::FileV1::Document { .. }),
        }
    }

    /// This file's `@require:`/`@import:`/`@stage:` headers. Both
    /// generations reuse `cst::Header` verbatim under `LoadMode::Legacy`
    /// (dev-0-1-0's header lexing is byte-identical to 0.0.6's), so there
    /// is exactly one `Header` type for both variants to share, not two.
    fn headers(&self) -> &[satysfi_syntax::cst::Header] {
        match self {
            Self::V0_0_6(f) => &f.headers,
            Self::V0_1(f) => match f {
                satysfi_syntax::cst_v1::FileV1::Document { headers, .. }
                | satysfi_syntax::cst_v1::FileV1::Library { headers, .. } => headers,
            },
        }
    }
}

/// One parsed file in a loaded program.
#[derive(Debug)]
pub struct LoadedFile {
    /// Canonicalized path to the file on disk.
    pub path: PathBuf,
    /// The file's parsed concrete syntax tree, tagged by grammar
    /// generation. Was `cst: satysfi_syntax::cst::File` before `V0_1`
    /// support; every consumer must now match on the variant. See
    /// `LoadedCst`'s doc comment.
    pub cst: LoadedCst,
}

/// A fully loaded, dependency-resolved program.
#[derive(Debug)]
pub struct LoadedProgram {
    /// Dependency-first order: every file appears after all the files it
    /// `@require:`s / `@import:`s. The entry document is last.
    pub files: Vec<LoadedFile>,
}

/// Load `entry` (a `.saty` document) and its full transitive `@require:` /
/// `@import:` dependency graph.
pub fn load(entry: &Path, opts: &LoadOptions) -> Result<LoadedProgram, LoadError> {
    if !opts.version.is_implemented() {
        return Err(LoadError::UnsupportedVersion {
            requested: opts.version,
            supported: SatysfiVersion::supported().to_vec(),
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
        let cst: LoadedCst = match opts.version {
            SatysfiVersion::V0_0_6 => LoadedCst::V0_0_6(
                satysfi_syntax::parse_file(&src).map_err(|source| LoadError::Parse {
                    path: path.clone(),
                    source,
                })?,
            ),
            SatysfiVersion::V0_1 => LoadedCst::V0_1(
                satysfi_syntax::parse_file_v1(&src).map_err(|source| LoadError::Parse {
                    path: path.clone(),
                    source,
                })?,
            ),
            // `SatysfiVersion` is `#[non_exhaustive]` — a catch-all is
            // required even though `load`'s `is_implemented()` guard above
            // already rejects every version this crate doesn't handle
            // before the loop starts. Unreachable in practice; a clear
            // message rather than a silent wrong-parse if `is_implemented()`
            // and this match ever drift apart.
            other => unreachable!(
                "SatysfiVersion::is_implemented() admitted {other} but load()'s \
                 parse dispatch has no arm for it"
            ),
        };

        if id == entry_id {
            if !cst.is_document() {
                return Err(LoadError::LibraryAsEntry { path });
            }
        } else if cst.is_document() {
            return Err(LoadError::DocumentAsDependency { path });
        }

        let dir = path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));

        let mut deps = Vec::new();
        for header in cst.headers() {
            let resolved = match header {
                satysfi_syntax::cst::Header::Import(tok) => {
                    v006::resolve::resolve_import(&dir, &tok.content).map_err(|searched| {
                        LoadError::UnresolvedImport {
                            name: tok.content.clone(),
                            from: path.clone(),
                            searched,
                        }
                    })?
                }
                satysfi_syntax::cst::Header::Require(tok) => {
                    v006::resolve::resolve_require(opts.lib_root.as_deref(), &tok.content)
                        .map_err(|searched| LoadError::UnresolvedRequire {
                            name: tok.content.clone(),
                            searched,
                        })?
                }
                // `@stage: persistent` / `@stage: 0` / `@stage: 1` — this
                // port is single-stage only, so the header carries no
                // loader-visible information (see `cst::Header::Stage`'s
                // doc comment); it drives no dependency edge.
                satysfi_syntax::cst::Header::Stage(_) => continue,
            };
            let dep_canon = canonicalize(&resolved)?;
            let dep_id = alloc_id(dep_canon, &mut next_id, &mut id_of, &mut path_of);
            deps.push(dep_id);
            worklist.push(dep_id);
        }

        adjacency.insert(id, deps);
        cst_of.insert(id, cst);
    }

    let order = v006::graph::toposort(&adjacency)
        .map_err(|chain_ids| LoadError::Cycle {
            chain: v006::graph::chain_to_paths(&chain_ids, &path_of),
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

fn canonicalize(path: &Path) -> Result<PathBuf, LoadError> {
    std::fs::canonicalize(path).map_err(|source| LoadError::Io {
        path: path.to_path_buf(),
        source,
    })
}

fn alloc_id(
    path: PathBuf,
    next_id: &mut u32,
    id_of: &mut HashMap<PathBuf, u32>,
    path_of: &mut HashMap<u32, PathBuf>,
) -> u32 {
    if let Some(&id) = id_of.get(&path) {
        return id;
    }
    let id = *next_id;
    *next_id += 1;
    id_of.insert(path.clone(), id);
    path_of.insert(id, path);
    id
}
