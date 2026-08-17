//! `@require:`/`@import:` name resolution.
//!
//! Transcribed from v0.0.6's `src/frontend/main.ml` (lines ~95-140): a header
//! name is turned into a list of candidate file paths, tried in order; the
//! first candidate that exists on disk wins. We do NOT implement the
//! mode-specific `.satyh-<mode>` extension SATySFi also tries — out of scope
//! here, matching the task's transcription instructions.

use std::path::{Path, PathBuf};

/// Extensions tried, in preference order, when a header name has none of its
/// own. `.satyh` (the "normal" library extension) beats `.satyg` (the
/// "governed"/restricted-grammar library extension) — same order as
/// `main.ml`'s candidate list.
pub(crate) const CANDIDATE_EXTS: [&str; 2] = [".satyh", ".satyg"];

fn has_candidate_ext(name: &str) -> bool {
    CANDIDATE_EXTS.iter().any(|ext| name.ends_with(ext))
}

/// All paths `base/name` could resolve to, without checking existence.
fn candidates_in(base: &Path, name: &str) -> Vec<PathBuf> {
    if has_candidate_ext(name) {
        vec![base.join(name)]
    } else {
        CANDIDATE_EXTS
            .iter()
            .map(|ext| base.join(format!("{name}{ext}")))
            .collect()
    }
}

/// Resolve `@import: name` relative to `dir`, the directory of the file that
/// contains the header (NOT the entry document's directory — see main.ml,
/// where imports are resolved relative to the current file being processed,
/// which matters once library files import each other from different
/// subdirectories).
///
/// Returns the first candidate that exists, or `Err` with the full list of
/// paths tried (for `UnresolvedImport::searched`).
pub(crate) fn resolve_import(dir: &Path, name: &str) -> Result<PathBuf, Vec<PathBuf>> {
    let candidates = candidates_in(dir, name);
    for candidate in &candidates {
        if candidate.is_file() {
            return Ok(candidate.clone());
        }
    }
    Err(candidates)
}

/// Resolve `@require: name` against the package/library root.
///
/// v0.0.6's `Config.resolve_package` searches a configurable list of library
/// directories; we approximate that with two fixed candidates under
/// `lib_root`, in order:
///   1. `<lib_root>/dist/packages/<name>` (the standard SATySFi package
///      layout used by `satysfi-dist`/opam installs).
///   2. `<lib_root>/<name>` (a plain fallback, for a `lib_root` that already
///      points directly at a package tree, e.g. in tests).
///
/// If `lib_root` is `None`, there is nowhere to search: returns `Err(vec![])`
/// immediately (surfaced by `UnresolvedRequire` as "no candidates").
pub(crate) fn resolve_require(lib_root: Option<&Path>, name: &str) -> Result<PathBuf, Vec<PathBuf>> {
    let Some(root) = lib_root else {
        return Err(Vec::new());
    };
    let bases = [root.join("dist").join("packages"), root.to_path_buf()];
    let mut candidates = Vec::new();
    for base in &bases {
        candidates.extend(candidates_in(base, name));
    }
    for candidate in &candidates {
        if candidate.is_file() {
            return Ok(candidate.clone());
        }
    }
    Err(candidates)
}
