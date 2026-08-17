//! `install` (plan §4.1, §6): resolve root → prepare source (dir or
//! `.tar.gz`) → discover plan (manifest-first, flat-copy fallback) → stage
//! with path-traversal guard → collision check → atomic swap → write
//! receipt.

use std::path::{Path, PathBuf};

use crate::error::Error;
use crate::receipts::{self, FileEntry, Receipt, Source, SCHEMA_VERSION};
use crate::roots::RootSelection;
use crate::util;
use crate::{archive, manifest, stage};

/// Options for [`install`] (plan §7.2).
#[derive(Debug, Default, Clone)]
pub struct InstallOptions {
    pub lib_root: Option<PathBuf>,
    pub dest: Option<PathBuf>,
    /// `-l`/`--library NAME` filter (repeatable). `None` means no filter.
    pub libraries: Option<Vec<String>>,
    pub force: bool,
}

impl RootSelection for InstallOptions {
    fn lib_root(&self) -> Option<&Path> {
        self.lib_root.as_deref()
    }
    fn dest(&self) -> Option<&Path> {
        self.dest.as_deref()
    }
}

/// What [`install`] materialised.
#[derive(Debug)]
pub struct InstallReport {
    pub name: String,
    pub version: String,
    /// Distinct top-level destination subtrees (relative to the library
    /// root), sorted — one line per entry when the CLI prints them.
    pub files: Vec<PathBuf>,
}

/// Install the package at `source` (a directory or `.tar.gz`).
pub fn install(source: &Path, opts: &InstallOptions) -> Result<InstallReport, Error> {
    install_inner(source, opts, None)
}

/// The install pipeline, shared by the phase-1 `path`/`archive` primitive and
/// the phase-3 registry source. `source_override`, when `Some`, replaces the
/// receipt's `[source]` table (the registry form records the package name,
/// version, tarball url, and verified sha256 there instead of a bare local
/// path); when `None`, the receipt records the prepared `path`/`archive`
/// source as before.
pub(crate) fn install_inner(
    source: &Path,
    opts: &InstallOptions,
    source_override: Option<Source>,
) -> Result<InstallReport, Error> {
    let root = opts.resolve_managed_root()?;

    let prepared = archive::prepare(source, &root)?;
    let plans = manifest::discover(&prepared.source_root)?;

    // `-l`/`--library` filter / library selection (plan §4.1). A single
    // `satysfi-package.toml` or `packages/` fallback yields exactly one plan;
    // a `Satyristes` (phase 4) may declare several `(library ...)` blocks, in
    // which case `--library` must narrow the selection to exactly one (one
    // library is materialised per install).
    let plan = select_plan(plans, opts.libraries.as_deref())?;

    // Collision policy (plan §6).
    let old_receipt = if receipts::exists(&root, &plan.name) {
        if !opts.force {
            return Err(Error::AlreadyInstalled { name: plan.name });
        }
        Some(receipts::read(&root, &plan.name)?)
    } else {
        // No receipt for this name: refuse to clobber any pre-existing
        // (unmanaged) file at a destination path.
        for pf in &plan.files {
            let live = stage::safe_join(&root, &pf.dst)?;
            if live.exists() {
                return Err(Error::UnmanagedCollision { path: live });
            }
        }
        None
    };

    // Stage every file (path-traversal-checked) and hash it.
    let staging = stage::StagingArea::new(&root, &plan.name)?;
    let mut file_entries = Vec::with_capacity(plan.files.len());
    for pf in &plan.files {
        staging.stage(&pf.dst, &pf.src)?;
        let staged_path = stage::safe_join(staging.path(), &pf.dst)?;
        let sha = util::sha256_file(&staged_path)?;
        file_entries.push(FileEntry {
            dst: pf.dst.clone(),
            sha256: Some(sha),
        });
    }

    let new_dsts: Vec<String> = file_entries.iter().map(|f| f.dst.clone()).collect();
    let old_dsts: Vec<String> = old_receipt
        .as_ref()
        .map(|r| r.files.iter().map(|f| f.dst.clone()).collect())
        .unwrap_or_default();

    // Atomic swap into place.
    stage::materialize(&root, staging.path(), &new_dsts, &old_dsts)?;

    // Record the receipt (after materialisation, so a crash never leaves a
    // receipt pointing at files that were not placed).
    let receipt = Receipt {
        schema_version: SCHEMA_VERSION,
        name: plan.name.clone(),
        package_version: plan.version.clone(),
        installed_at: util::now_rfc3339(),
        source: source_override.unwrap_or_else(|| {
            Source::plain(prepared.kind, prepared.value.to_string_lossy().into_owned())
        }),
        files: file_entries,
    };
    receipts::write(&root, &receipt)?;

    Ok(InstallReport {
        name: plan.name,
        version: plan.version,
        files: top_level_paths(&new_dsts),
    })
}

/// Pick the single library to install from the discovered plan(s), honouring
/// the `-l`/`--library` filter (plan §4.1):
///
/// - no filter + exactly one plan → that plan;
/// - filter given → keep plans whose declared name is in the set;
/// - end state must be exactly one plan: zero → [`Error::LibraryFilter`],
///   more than one → [`Error::AmbiguousLibrary`].
fn select_plan(
    plans: Vec<manifest::PackagePlan>,
    libraries: Option<&[String]>,
) -> Result<manifest::PackagePlan, Error> {
    let declared: Vec<String> = plans.iter().map(|p| p.name.clone()).collect();
    let mut selected: Vec<manifest::PackagePlan> = match libraries {
        Some(filter) => plans
            .into_iter()
            .filter(|p| filter.iter().any(|n| n == &p.name))
            .collect(),
        None => plans,
    };
    match selected.len() {
        1 => Ok(selected.pop().unwrap()),
        0 => Err(Error::LibraryFilter {
            declared: declared.join(", "),
        }),
        _ => Err(Error::AmbiguousLibrary {
            names: selected
                .iter()
                .map(|p| p.name.clone())
                .collect::<Vec<_>>()
                .join(", "),
        }),
    }
}

/// Collapse a flat file list to distinct top-level subtrees: the first three
/// path components (`dist/<category>/<name>`) for nested per-library layouts,
/// or the whole path when shorter (a flat `dist/packages/foo.satyh` or a
/// root-relative `dist/<dst>`).
fn top_level_paths(dsts: &[String]) -> Vec<PathBuf> {
    let mut tops: Vec<PathBuf> = dsts
        .iter()
        .map(|d| {
            let comps: Vec<&str> = d.split('/').filter(|s| !s.is_empty()).collect();
            let take = comps.len().min(3);
            comps[..take].iter().collect::<PathBuf>()
        })
        .collect();
    tops.sort();
    tops.dedup();
    tops
}
