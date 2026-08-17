//! Manifest-mode install (plan §5.3, §8 phase 2): the no-`PATH`
//! `satyrographos install`. Read `Satyrfile.toml` + `Satyrfile.lock`, diff
//! each entry's freshly-computed source hash against the lock (and against the
//! installed receipt), and re-materialise **only** the entries that actually
//! changed — leaving unchanged entries' files bit-for-bit untouched (so their
//! mtimes survive a no-op install). Every re-materialisation goes through the
//! phase-1 [`install`](crate::ops::install::install) primitive (§4.1); this
//! module adds only the diff-and-drive layer on top.
//!
//! ## Reconcile decision (per manifest entry `name` with source `src`)
//!
//! | lock has `name`? | lock sha == hash(src)? | receipt intact? | action |
//! |---|---|---|---|
//! | yes | yes | yes | **skip** — touch nothing (mtimes preserved) |
//! | yes | yes | no (drifted) | **re-install** — self-heal a deleted/partial receipt |
//! | yes | no (source changed) | — | **re-install** — force-swap the new content |
//! | no (new entry) | — | — | **install** — fresh |
//!
//! An entry that is in the old lock but *not* in the manifest is **left in
//! place** — phase 2 does not prune dropped dependencies. The plan's §5.3
//! specifies only "diff … and re-materialise entries whose hash changed"; it
//! is silent on removal, so removed entries keep their installed files and
//! receipt, and are merely reported (and drop out of the rewritten lockfile).

use std::path::{Path, PathBuf};

use crate::error::Error;
use crate::lockfile::{self, LockEntry, Lockfile};
use crate::ops::install::{self, InstallOptions, InstallReport};
use crate::ops::registry_install::{self, Resolved};
use crate::ops::uninstall::RootOptions;
use crate::registry::RegistryOptions;
use crate::roots::RootSelection;
use crate::satyrfile::{self, SourceKind};
use crate::{receipts, stage, util};

/// What manifest-mode [`install_manifest`] did.
#[derive(Debug, Default)]
pub struct ManifestReport {
    /// Entries that were (re-)materialised, in manifest order.
    pub installed: Vec<InstallReport>,
    /// Entry names skipped because nothing changed (files left untouched).
    pub skipped: Vec<String>,
    /// Entry names present in the old lockfile but no longer in the manifest;
    /// left installed (not pruned), only reported (see module docs).
    pub removed: Vec<String>,
}

/// Reconcile against the sibling lockfile/receipts with no *explicit*
/// `--registry` override ([`RegistryOptions::default`]). `{ path = … }` sources
/// materialise as in phase 2; a `{ registry = … }` source still resolves if the
/// registry URL is discoverable from `$SATYSFI_REGISTRY` or the manifest's own
/// `[registry]` url (else [`Error::NoRegistry`]). Callers that need to pass a
/// `--registry` flag or a cache dir use [`install_manifest_reg`] directly.
pub fn install_manifest(
    manifest_path: &Path,
    opts: &RootOptions,
) -> Result<ManifestReport, Error> {
    install_manifest_reg(manifest_path, opts, &RegistryOptions::default())
}

/// Reconcile `manifest_path`'s `Satyrfile.toml` against its sibling
/// `Satyrfile.lock` and the receipts in the resolved root, materialising only
/// changed/missing/new entries and rewriting the lockfile to mirror the
/// manifest (plan §5.3/§5.4). `reg_opts` supplies the registry URL/cache for
/// `{ registry = … }` sources; `{ path = … }` sources never consult it.
pub fn install_manifest_reg(
    manifest_path: &Path,
    opts: &RootOptions,
    reg_opts: &RegistryOptions,
) -> Result<ManifestReport, Error> {
    let manifest = satyrfile::read(manifest_path)?;
    let manifest_dir = manifest_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));

    let root = opts.resolve_managed_root()?;

    let lock_path = lockfile::lock_path_for(manifest_path);
    let old_lock = lockfile::read(&lock_path)?;

    let mut report = ManifestReport::default();
    let mut new_entries: Vec<LockEntry> = Vec::with_capacity(manifest.libraries.len());

    for lib in &manifest.libraries {
        let locked = old_lock.get(&lib.name);
        let install_opts = |force: bool| InstallOptions {
            lib_root: opts.lib_root.clone(),
            dest: opts.dest.clone(),
            libraries: None,
            force,
        };

        let new_entry = match lib.source.kind()? {
            SourceKind::Path(rel) => {
                let src_path = resolve_source_path(&manifest_dir, rel);
                let current_hash = util::sha256_tree(&src_path)?;
                let hash_matches = locked.map(|l| l.sha256 == current_hash).unwrap_or(false);
                if hash_matches && receipt_intact(&root, &lib.name) {
                    report.skipped.push(lib.name.clone());
                    new_entries.push(locked.expect("hash_matches implies Some").clone());
                    continue;
                }
                let force = receipts::exists(&root, &lib.name);
                if !force && locked.is_some() {
                    // Self-heal a hand-deleted receipt whose files linger.
                    clear_orphans(&root, &src_path)?;
                }
                let ir = install::install(&src_path, &install_opts(force))?;
                report.installed.push(ir);
                LockEntry {
                    name: lib.name.clone(),
                    source: lib.source.clone(),
                    sha256: current_hash,
                    url: None,
                    resolved_at: util::now_rfc3339(),
                }
            }
            SourceKind::Registry {
                registry: pkg,
                version: req,
            } => {
                // Reproducible reuse (plan §5.4/§5.3): a lock entry that already
                // pins a url+sha256 and satisfies the manifest's version request
                // is re-materialised (or skipped) WITHOUT consulting the index.
                let reusable = locked.filter(|l| {
                    l.url.is_some()
                        // The locked pin must be for the *same* registry package
                        // (a changed `registry = ` under an unchanged library name
                        // must re-resolve, not reuse the stale url/sha256)…
                        && l.source.registry.as_deref() == Some(pkg)
                        // …and satisfy the requested version, if any.
                        && (req.is_none() || l.source.version.as_deref() == req)
                });
                if let Some(l) = reusable {
                    if receipt_intact(&root, &lib.name) {
                        report.skipped.push(lib.name.clone());
                        new_entries.push(l.clone());
                        continue;
                    }
                    // Receipt drifted: reinstall straight from the locked pin.
                    let resolved = Resolved {
                        version: l.source.version.clone().unwrap_or_default(),
                        url: l.url.clone().expect("reusable implies url is Some"),
                        sha256: l.sha256.clone(),
                    };
                    let force = receipts::exists(&root, &lib.name);
                    let ir = registry_install::install_resolved(
                        &lib.name,
                        &resolved,
                        &install_opts(force),
                    )?;
                    report.installed.push(ir);
                    l.clone()
                } else {
                    // Fresh, or the requested version changed: consult the index.
                    let resolved = registry_install::resolve(
                        pkg,
                        req,
                        reg_opts,
                        manifest.registry_url(),
                    )?;
                    let force = receipts::exists(&root, &lib.name);
                    let ir = registry_install::install_resolved(
                        &lib.name,
                        &resolved,
                        &install_opts(force),
                    )?;
                    report.installed.push(ir);
                    registry_lock_entry(&lib.name, pkg, &resolved)
                }
            }
            SourceKind::Git { .. } => {
                // Git sources stay rejected through phase 3 (plan §5.4).
                return Err(Error::UnsupportedSource {
                    kind: "git (not supported)",
                });
            }
        };
        new_entries.push(new_entry);
    }

    // Entries dropped from the manifest are left installed, only reported.
    for old in &old_lock.libraries {
        if !manifest.libraries.iter().any(|l| l.name == old.name) {
            report.removed.push(old.name.clone());
        }
    }

    let new_lock = Lockfile {
        libraries: new_entries,
    };
    lockfile::write(&lock_path, &new_lock)?;

    Ok(report)
}

/// Build a lockfile entry for a freshly-resolved registry install, pinning the
/// concrete resolved version (in `source.version`), tarball url, and verified
/// tarball sha256 (plan §5.4 step 4).
fn registry_lock_entry(name: &str, pkg: &str, resolved: &Resolved) -> LockEntry {
    LockEntry {
        name: name.to_string(),
        source: crate::satyrfile::SourceSpec {
            registry: Some(pkg.to_string()),
            version: Some(resolved.version.clone()),
            ..Default::default()
        },
        sha256: resolved.sha256.clone(),
        url: Some(resolved.url.clone()),
        resolved_at: util::now_rfc3339(),
    }
}

/// Resolve a manifest `path` source relative to the `Satyrfile.toml`'s own
/// directory (an absolute source path is used verbatim).
fn resolve_source_path(manifest_dir: &Path, rel: &str) -> PathBuf {
    let p = Path::new(rel);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        manifest_dir.join(p)
    }
}

/// Remove any destination file the source at `src` would occupy that is still
/// present on disk with no receipt claiming it — the orphans a hand-deleted
/// receipt leaves behind. A directory source is planned via the phase-1
/// [`manifest::discover`](crate::manifest::discover); an archive source (which
/// `discover` cannot plan without extraction) is left to the fresh install to
/// report if it genuinely collides.
fn clear_orphans(root: &Path, src: &Path) -> Result<(), Error> {
    if !src.is_dir() {
        return Ok(());
    }
    // `discover` may yield several plans (a multi-library Satyristes,
    // phase 4); orphan-clearing sweeps every destination any of them would
    // occupy — over-clearing is safe here because everything swept is about
    // to be re-materialized or genuinely orphaned.
    let plans = crate::manifest::discover(src)?;
    for plan in &plans {
        for pf in &plan.files {
            let live = stage::safe_join(root, &pf.dst)?;
            util::remove_file_if_exists(&live)?;
        }
    }
    Ok(())
}

/// Whether the receipt for `name` exists *and* every file it records is still
/// present on disk. A missing receipt or any missing file means the install
/// has drifted and must be re-materialised (self-heal).
fn receipt_intact(root: &Path, name: &str) -> bool {
    let receipt = match receipts::read(root, name) {
        Ok(r) => r,
        Err(_) => return false,
    };
    receipt.files.iter().all(|f| {
        stage::safe_join(root, &f.dst)
            .map(|p| p.exists())
            .unwrap_or(false)
    })
}
