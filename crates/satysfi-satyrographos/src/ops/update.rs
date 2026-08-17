//! `satyrographos update` (plan §8, §5.4 step 1): re-fetch the registry index
//! (a git index is fetched into the cache and its resolved commit sha
//! recorded) and **report** which locked registry dependencies have a newer
//! version available — without applying them. Applying an upgrade is a
//! deliberate second step (`install <name>@<newer>` or editing the manifest and
//! reconciling), mirroring how `update` only refreshes the index (§5.4 step 1
//! is silent on applying, so this port does not).

use std::path::Path;

use crate::error::Error;
use crate::lockfile::{self, LockEntry};
use crate::registry::{self, RegistryOptions};
use crate::satyrfile::{self, SourceKind};

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
/// registry entry against the freshest available version. `manifest_path`
/// locates both the `Satyrfile.toml` (for its `[registry]` url fallback) and
/// the sibling `Satyrfile.lock` (for the currently-locked versions).
pub fn update(
    manifest_path: &Path,
    reg_opts: &RegistryOptions,
) -> Result<UpdateReport, Error> {
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

    for entry in &lock.libraries {
        // Only registry-sourced entries have an index to compare against.
        if !matches!(entry.source.kind(), Ok(SourceKind::Registry { .. })) {
            continue;
        }
        let current = locked_version(entry);
        let idx = match registry::lookup(&reg, &entry.name) {
            Ok(i) => i,
            // A locked package that has since vanished from the index: skip it
            // rather than fail the whole report.
            Err(Error::PackageNotFound { .. }) => continue,
            Err(e) => return Err(e),
        };
        let latest = match registry::select_version(&idx, &entry.name, None) {
            Ok((v, _)) => v,
            Err(_) => continue,
        };
        match (current.as_deref(), registry::version_cmp(&latest, current.as_deref().unwrap_or(""))) {
            (Some(cur), std::cmp::Ordering::Greater) => report.upgrades.push(Upgrade {
                name: entry.name.clone(),
                current: cur.to_string(),
                latest,
            }),
            _ => report.up_to_date.push(entry.name.clone()),
        }
    }

    Ok(report)
}

/// The concrete version a lock entry pins (registry entries record it in
/// `source.version`).
fn locked_version(entry: &LockEntry) -> Option<String> {
    entry.source.version.clone()
}
