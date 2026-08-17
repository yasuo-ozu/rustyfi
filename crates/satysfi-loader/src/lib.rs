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
mod graph;
mod resolve;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

pub use error::LoadError;

/// Options controlling header resolution.
pub struct LoadOptions {
    /// Root used to resolve `@require: name` (searched as
    /// `<lib_root>/dist/packages/name.{satyh,satyg}`, falling back to
    /// `<lib_root>/name.{satyh,satyg}`). `None` means there is no package
    /// root configured, so any `@require:` header fails to resolve.
    pub lib_root: Option<PathBuf>,
}

/// One parsed file in a loaded program.
#[derive(Debug)]
pub struct LoadedFile {
    /// Canonicalized path to the file on disk.
    pub path: PathBuf,
    /// The file's parsed concrete syntax tree.
    pub cst: satysfi_syntax::cst::File,
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
    let entry_canon = canonicalize(entry)?;

    let mut next_id: u32 = 0;
    let mut id_of: HashMap<PathBuf, u32> = HashMap::new();
    let mut path_of: HashMap<u32, PathBuf> = HashMap::new();
    let mut cst_of: HashMap<u32, satysfi_syntax::cst::File> = HashMap::new();
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
        let cst = satysfi_syntax::parse_file(&src).map_err(|source| LoadError::Parse {
            path: path.clone(),
            source,
        })?;

        if id == entry_id {
            if cst.body.is_none() {
                return Err(LoadError::LibraryAsEntry { path });
            }
        } else if cst.body.is_some() {
            return Err(LoadError::DocumentAsDependency { path });
        }

        let dir = path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));

        let mut deps = Vec::new();
        for header in &cst.headers {
            let resolved = match header {
                satysfi_syntax::cst::Header::Import(tok) => {
                    resolve::resolve_import(&dir, &tok.content).map_err(|searched| {
                        LoadError::UnresolvedImport {
                            name: tok.content.clone(),
                            from: path.clone(),
                            searched,
                        }
                    })?
                }
                satysfi_syntax::cst::Header::Require(tok) => {
                    resolve::resolve_require(opts.lib_root.as_deref(), &tok.content).map_err(
                        |searched| LoadError::UnresolvedRequire {
                            name: tok.content.clone(),
                            searched,
                        },
                    )?
                }
            };
            let dep_canon = canonicalize(&resolved)?;
            let dep_id = alloc_id(dep_canon, &mut next_id, &mut id_of, &mut path_of);
            deps.push(dep_id);
            worklist.push(dep_id);
        }

        adjacency.insert(id, deps);
        cst_of.insert(id, cst);
    }

    let order = graph::toposort(&adjacency)
        .map_err(|chain_ids| LoadError::Cycle {
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
