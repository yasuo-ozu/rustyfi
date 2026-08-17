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
/// directories; we approximate that with four fixed candidates under
/// `lib_root`, in order:
///   1. `<lib_root>/dist/packages/<name>` (the standard SATySFi package
///      layout used by `satysfi-dist`/opam installs, and this port's own
///      no-manifest flat-copy fallback).
///   2. `<lib_root>/<name>` (a plain fallback, for a `lib_root` that already
///      points directly at a package tree, e.g. in tests).
///   3. `<lib_root>/dist/packages/<name>/<name>` (the *nested* per-library
///      layout real Satyrographos produces and this port's manifest-driven
///      installer materialises — see the chimera plan §3). Purely additive:
///      candidates 1 and 2 are unchanged.
///   4. `<lib_root>/dist-v01/packages/<name>` (Slice X4a,
///      `docs/plans/design-cross-version-import.md` §X4.3 item 1 — the 0.1
///      corpus, mirroring candidate 1 with no nested-layout analogue yet).
///      This is what lets a `V0_0_6`-rooted load's `@require:` reach a 0.1
///      package under `lib-satysfi/dist-v01/packages/` from the SAME
///      `lib_root` a 0.0.6 document also `@require:`s the 0.0.6 corpus
///      from. Purely additive — appended LAST, so it only ever adds NEW
///      successful resolutions; it never changes which candidate wins for
///      any name that candidates 1-3 already resolve (a pure-0.0.6 load with
///      no `dist-v01/packages/<name>` target is completely unaffected).
///
/// If `lib_root` is `None`, there is nowhere to search: returns `Err(vec![])`
/// immediately (surfaced by `UnresolvedRequire` as "no candidates").
pub(crate) fn resolve_require(lib_root: Option<&Path>, name: &str) -> Result<PathBuf, Vec<PathBuf>> {
    let Some(root) = lib_root else {
        return Err(Vec::new());
    };
    let dist_packages = root.join("dist").join("packages");
    let dist_v01_packages = root.join("dist-v01").join("packages");
    let bases = [
        dist_packages.clone(),
        root.to_path_buf(),
        dist_packages.join(name),
        dist_v01_packages,
    ];
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
