//! `@require:`/`@import:` name resolution.
//!
//! Transcribed from v0.0.6's `src/frontend/main.ml` (lines ~95-140): a header
//! name is turned into a list of candidate file paths, tried in order; the
//! first candidate that exists on disk wins. We do NOT implement the
//! mode-specific `.satyh-<mode>` extension SATySFi also tries — out of scope
//! here, matching the task's transcription instructions.

use crate::SourceProvider;
use rustyfi_syntax::RustyfiVersion;
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
pub(crate) fn resolve_import(
    sources: &dyn SourceProvider,
    dir: &Path,
    name: &str,
) -> Result<PathBuf, Vec<PathBuf>> {
    let candidates = candidates_in(dir, name);
    for candidate in &candidates {
        if sources.is_file(candidate) {
            return Ok(candidate.clone());
        }
    }
    Err(candidates)
}

/// Resolve `@require: name` against the package/library root.
///
/// v0.0.6's `Config.resolve_package` searches a configurable list of library
/// directories; we approximate that with five fixed candidates under
/// `lib_root`, in order:
///   1. `<lib_root>/dist/packages/<name>` (the standard SATySFi package
///      layout used by `rustyfi-dist`/opam installs, and this port's own
///      no-manifest flat-copy fallback).
///   2. `<lib_root>/<name>` (a plain fallback, for a `lib_root` that already
///      points directly at a package tree, e.g. in tests).
///   3. `<lib_root>/dist/packages/<name>/<name>` (the *nested* per-library
///      layout real Satyrographos produces and this port's manifest-driven
///      installer materialises).
///   4. `<lib_root>/dist-v01/packages/<name>` (the 0.1
///      corpus, mirroring candidate 1). This is what lets a `V0_0`-rooted
///      load's `@require:` reach a 0.1 package under
///      `lib-rustyfi/dist-v01/packages/` from the SAME `lib_root` a 0.0.6
///      document also `@require:`s the 0.0.6 corpus from. Ordered LAST for a
///      0.0.6 load, so it only ever adds resolutions and never changes which
///      candidate wins for a name candidates 1-3 already resolve.
///   5. `<lib_root>/dist-v01/packages/<name>/<name>` — candidate 3's analogue
///      for the 0.1 corpus. `install --lang 0.1` of a package whose manifest
///      declares `(packageDir ...)`, which is what real Satyrographos packages
///      declare, materialises exactly this nested layout; without this
///      candidate such a package installs successfully and is then
///      unreachable from any `@require:`.
///
/// If `lib_root` is `None`, there is nowhere to search: returns `Err(vec![])`
/// immediately (surfaced by `UnresolvedRequire` as "no candidates").
pub(crate) fn resolve_require(
    sources: &dyn SourceProvider,
    roots: &[&Path],
    name: &str,
    version: RustyfiVersion,
) -> Result<PathBuf, Vec<PathBuf>> {
    // Every root in turn, nearest first: a project-local root that carries one
    // package must not hide the development tree or the system install that
    // carry the rest.
    let mut searched = Vec::new();
    for root in roots {
        match resolve_require_in(sources, root, name, version) {
            Ok(found) => return Ok(found),
            Err(tried) => searched.extend(tried),
        }
    }
    Err(searched)
}

fn resolve_require_in(
    sources: &dyn SourceProvider,
    root: &Path,
    name: &str,
    version: RustyfiVersion,
) -> Result<PathBuf, Vec<PathBuf>> {
    let dist_packages = root.join("dist").join("packages");
    let dist_v01_packages = root.join("dist-v01").join("packages");
    // Search the load's OWN generation first. Both corpora are bundled side by
    // side and many names exist in both (`itemize`, `list`, `code`, `deco`, …)
    // with genuinely different APIs — 0.1's `itemize` has `+listing`'s
    // `?(break : bool)` label and 0.1's `list` has `fold`, neither of which
    // their 0.0.6 counterparts ever had. Resolving a 0.1 document's
    // `@require:` to the 0.0.6 package therefore fails at the USE site with a
    // missing label or an unbound member, which reads like a compiler gap and
    // is really just the wrong file.
    //
    // Cross-generation resolution stays available as a FALLBACK in both
    // directions — that is what makes cross-version import reachable at
    // all — so this only reorders which candidate wins for a name present
    // in both, and never removes a resolution.
    let bases: Vec<PathBuf> = if version == RustyfiVersion::V0_1 {
        vec![
            dist_v01_packages.clone(),
            dist_v01_packages.join(name),
            dist_packages.clone(),
            root.to_path_buf(),
            dist_packages.join(name),
        ]
    } else {
        vec![
            dist_packages.clone(),
            root.to_path_buf(),
            dist_packages.join(name),
            dist_v01_packages.clone(),
            dist_v01_packages.join(name),
        ]
    };
    let mut candidates = Vec::new();
    for base in &bases {
        candidates.extend(candidates_in(base, name));
    }
    for candidate in &candidates {
        if sources.is_file(candidate) {
            return Ok(candidate.clone());
        }
    }
    Err(candidates)
}
