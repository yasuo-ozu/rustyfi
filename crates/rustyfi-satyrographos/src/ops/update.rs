//! `satyrographos update` (plan §8, §5.4 step 1): re-fetch the registry index
//! (a git index is fetched into the cache and its resolved commit sha
//! recorded) and **report** which locked registry dependencies have a newer
//! version available — without applying them. Applying an upgrade is a
//! deliberate second step (`install <name>@<newer>` or editing the manifest and
//! reconciling), mirroring how `update` only refreshes the index (§5.4 step 1
//! is silent on applying, so this port does not).
//!
//! Phase-7c: the diff is now solver-based (design §5.2), not a bare
//! per-package "highest available" lookup — every currently-locked registry
//! package (direct **and** transitive) is re-solved *together* against the
//! freshly-refreshed index, so a package whose latest published version would
//! violate some other locked package's declared dependency is correctly
//! reported at the highest version that still fits the *whole* graph, not the
//! index's bare maximum. `Satyrfile.toml`'s own `source.version` pin is
//! deliberately **not** fed into the solve here (it is an exact-or-absent
//! install pin, matching `registry::select_version`'s pre-solver "exact if
//! given, else highest" contract — feeding it in as a root constraint would
//! make an exact pin *never* report an upgrade, which is the opposite of what
//! `update` is for): every currently-locked registry package is a root with
//! [`Constraint::Any`], exactly mirroring the pre-solver code's
//! `select_version(idx, name, None)`.

use std::collections::HashMap;
use std::path::Path;

use crate::error::Error;
use crate::lockfile::{self, LockEntry};
use crate::registry::{self, RegistryDepSource, RegistryOptions};
use crate::satyrfile::{self, SourceKind};
use crate::solve;
use crate::version::Constraint;

/// One dependency with a newer version available than the lockfile records.
#[derive(Debug, Clone)]
pub struct Upgrade {
    pub name: String,
    pub current: String,
    pub latest: String,
}

/// What `update` found.
#[derive(Debug, Default)]
pub struct UpdateReport {
    /// The resolved index commit sha (`None` for a plain-directory index).
    pub commit: Option<String>,
    /// Locked registry entries with a newer version available.
    pub upgrades: Vec<Upgrade>,
    /// Locked registry entries already at the highest available version.
    pub up_to_date: Vec<String>,
}

/// Re-fetch the index for `manifest_path`'s project and diff every locked
/// registry entry (direct and transitive) against the highest version the
/// solver finds when it re-solves the whole registry sub-graph fresh.
/// `manifest_path` locates both the `Satyrfile.toml` (for its `[registry]`
/// url fallback) and the sibling `Satyrfile.lock` (for the currently-locked
/// versions and package identities).
pub fn update(manifest_path: &Path, reg_opts: &RegistryOptions) -> Result<UpdateReport, Error> {
    // The manifest's own `[registry]` url is the fallback when no flag/env is set.
    let manifest = satyrfile::read(manifest_path)?;
    let fallback = manifest.registry_url();
    let url = reg_opts.resolve_url(fallback)?;

    // Always refresh the index (that is what `update` is for).
    let refresh_opts = RegistryOptions {
        refresh: true,
        ..reg_opts.clone()
    };
    let reg = registry::acquire(&url, &refresh_opts)?;

    let lock_path = lockfile::lock_path_for(manifest_path);
    let lock = lockfile::read(&lock_path)?;

    let mut report = UpdateReport {
        commit: reg.commit.clone(),
        ..Default::default()
    };

    // Every currently-locked registry package (direct or transitive) becomes
    // an unconstrained root, so the solver re-derives the whole sub-graph's
    // highest mutually-compatible versions — including any transitive
    // dependency the fresh index now declares that the old lock predates.
    let mut alias_by_pkg: HashMap<&str, &str> = HashMap::new();
    let mut current_by_pkg: HashMap<&str, &str> = HashMap::new();
    let mut root: Vec<(String, Constraint)> = Vec::new();
    for entry in &lock.libraries {
        if let Ok(SourceKind::Registry { registry: pkg, .. }) = entry.source.kind() {
            alias_by_pkg.insert(pkg, entry.name.as_str());
            if let Some(v) = entry.source.version.as_deref() {
                current_by_pkg.insert(pkg, v);
            }
            root.push((pkg.to_string(), Constraint::Any));
        }
    }
    if root.is_empty() {
        return Ok(report);
    }

    let src = RegistryDepSource::new(&reg);
    let solution = solve::solve(&root, &src)?;

    for (pkg, latest) in &solution.packages {
        let label = alias_by_pkg.get(pkg.as_str()).copied().unwrap_or(pkg.as_str());
        match current_by_pkg.get(pkg.as_str()) {
            Some(&cur) => {
                let is_newer = crate::version::Version::parse(cur)
                    .map(|c| *latest > c)
                    .unwrap_or(true);
                if is_newer {
                    report.upgrades.push(Upgrade {
                        name: label.to_string(),
                        current: cur.to_string(),
                        latest: latest.to_string(),
                    });
                } else {
                    report.up_to_date.push(label.to_string());
                }
            }
            // A package the solver pulled in that the old lock has no pin
            // for at all (a brand new transitive dependency introduced by an
            // index change) — nothing to diff against; report it up to date
            // at its freshly-solved version rather than as a spurious
            // "upgrade" with no prior baseline.
            None => report.up_to_date.push(label.to_string()),
        }
    }

    Ok(report)
}

/// The concrete version a lock entry pins (registry entries record it in
/// `source.version`). Kept for callers that still want the raw pin without
/// going through [`update`]'s solver-based diff.
pub fn locked_version(entry: &LockEntry) -> Option<String> {
    entry.source.version.clone()
}
