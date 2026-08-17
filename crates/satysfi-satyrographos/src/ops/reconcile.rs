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

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::error::Error;
use crate::lockfile::{self, LockEntry, Lockfile};
use crate::ops::install::{self, InstallOptions, InstallReport};
use crate::ops::registry_install::{self, Resolved};
use crate::ops::uninstall::RootOptions;
use crate::registry::{self, RegistryDepSource, RegistryOptions};
use crate::roots::RootSelection;
use crate::satyrfile::{self, SourceKind};
use crate::solve;
use crate::version::Constraint;
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

    // Registry-sourced entries are handled in a second pass (below): the
    // solver needs every direct registry root at once to compute one
    // consistent transitive closure (design §5.1), so path/git entries are
    // materialised first, in manifest order, exactly as before.
    let mut reg_directs: Vec<RegDirect> = Vec::new();

    for lib in &manifest.libraries {
        let locked = old_lock.get(&lib.name);
        let install_opts = |force: bool| InstallOptions {
            lib_root: opts.lib_root.clone(),
            dest: opts.dest.clone(),
            libraries: None,
            force,
        };

        match lib.source.kind()? {
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
                new_entries.push(LockEntry {
                    name: lib.name.clone(),
                    source: lib.source.clone(),
                    sha256: current_hash,
                    url: None,
                    resolved_at: util::now_rfc3339(),
                });
            }
            SourceKind::Registry {
                registry: pkg,
                version: req,
            } => {
                let constraint = match req {
                    Some(r) => Constraint::parse(r)?,
                    None => Constraint::Any,
                };
                reg_directs.push(RegDirect {
                    alias: lib.name.clone(),
                    pkg: pkg.to_string(),
                    constraint,
                });
            }
            SourceKind::Git { .. } => {
                // Git sources stay rejected through phase 3 (plan §5.4).
                return Err(Error::UnsupportedSource {
                    kind: "git (not supported)",
                });
            }
        }
    }

    if !reg_directs.is_empty() {
        install_registry_closure(&reg_directs, &manifest, &old_lock, &root, opts, reg_opts, &mut report, &mut new_entries)?;
    }

    // Entries dropped from the manifest are left installed, only reported —
    // "dropped" means neither a manifest library still names it NOR the fresh
    // closure just re-locked it (a transitive-only registry package has no
    // manifest entry of its own but is still part of the current lock).
    for old in &old_lock.libraries {
        let still_named = manifest.libraries.iter().any(|l| l.name == old.name);
        let still_locked = new_entries.iter().any(|e| e.name == old.name);
        if !still_named && !still_locked {
            report.removed.push(old.name.clone());
        }
    }

    let new_lock = Lockfile {
        libraries: new_entries,
    };
    lockfile::write(&lock_path, &new_lock)?;

    Ok(report)
}

/// One manifest `{ registry = … }` direct dependency, decoded into the
/// solver's vocabulary: `pkg` is the registry package id the solver recurses
/// on; `alias` is the local `[[library]] name` this port's lockfile/receipts
/// key entries by (they may differ — a project can name its dependency
/// something other than the registry package's own name).
struct RegDirect {
    alias: String,
    pkg: String,
    constraint: Constraint,
}

/// Resolve every `{ registry = … }` entry in `reg_directs` to the full
/// transitive closure and materialise it (design §5.1), pushing one
/// [`LockEntry`] per closure member (direct **and** transitive) onto
/// `new_entries`.
///
/// Reproducibility is preserved the same way the pre-solver code achieved it
/// for a single direct entry: when every direct root's constraint is already
/// satisfied by a url-bearing pin in `old_lock`, this whole pass reuses the
/// existing closure (every registry-kind entry in `old_lock`, direct or
/// transitive) via its locked `(version, url, sha256)` — **without touching
/// the index at all**. The solver (and thus the index) is consulted only when
/// at least one direct root is new or its requested version/constraint
/// changed; in that case the *entire* registry sub-graph is re-solved fresh
/// (not incrementally pinned), and the resulting closure replaces every prior
/// registry-kind lock entry.
#[allow(clippy::too_many_arguments)]
fn install_registry_closure(
    reg_directs: &[RegDirect],
    manifest: &satyrfile::Satyrfile,
    old_lock: &Lockfile,
    root: &Path,
    opts: &RootOptions,
    reg_opts: &RegistryOptions,
    report: &mut ManifestReport,
    new_entries: &mut Vec<LockEntry>,
) -> Result<(), Error> {
    let all_reusable = reg_directs.iter().all(|d| direct_is_reusable(d, old_lock));

    if all_reusable {
        // Fast path: no index consultation. Re-materialise every direct
        // entry from its existing pin…
        for d in reg_directs {
            let locked = old_lock.get(&d.alias).expect("checked reusable above");
            let resolved = resolved_from_lock(locked);
            let entry = materialize_registry_pin(&d.alias, &d.pkg, &resolved, Some(locked), root, opts, report)?;
            new_entries.push(entry);
        }
        // …and carry forward every transitive-only registry entry (a
        // registry-kind old-lock entry not named by any direct alias) the
        // same way.
        let direct_aliases: HashSet<&str> = reg_directs.iter().map(|d| d.alias.as_str()).collect();
        for l in &old_lock.libraries {
            if direct_aliases.contains(l.name.as_str()) {
                continue;
            }
            if l.url.is_none() || !matches!(l.source.kind(), Ok(SourceKind::Registry { .. })) {
                continue;
            }
            let pkg = l
                .source
                .registry
                .clone()
                .expect("SourceKind::Registry implies source.registry is Some");
            let resolved = resolved_from_lock(l);
            let entry = materialize_registry_pin(&l.name, &pkg, &resolved, Some(l), root, opts, report)?;
            new_entries.push(entry);
        }
        return Ok(());
    }

    // At least one direct root is new or changed: re-solve the whole
    // registry sub-graph fresh (design §5.1 step 2).
    let url = reg_opts.resolve_url(manifest.registry_url())?;
    let reg = registry::acquire(&url, reg_opts)?;
    let solve_root: Vec<(String, Constraint)> =
        reg_directs.iter().map(|d| (d.pkg.clone(), d.constraint.clone())).collect();
    let src = RegistryDepSource::new(&reg);
    let solution = solve::solve(&solve_root, &src)?;

    for (pkg, version) in &solution.packages {
        let idx = registry::lookup(&reg, pkg)?;
        let entry = registry::entry_for(&idx, version).ok_or_else(|| Error::VersionNotFound {
            name: pkg.clone(),
            version: version.to_string(),
        })?;
        let resolved = Resolved {
            version: version.to_string(),
            url: entry.tarball_url.clone(),
            sha256: entry.sha256.clone(),
        };
        // A direct entry's own alias is used as the lock/receipt key when one
        // names this package id; otherwise (transitive-only) the package id
        // itself is the key.
        let label = reg_directs
            .iter()
            .find(|d| &d.pkg == pkg)
            .map(|d| d.alias.as_str())
            .unwrap_or(pkg.as_str());
        let locked = old_lock.get(label);
        let lock_entry = materialize_registry_pin(label, pkg, &resolved, locked, root, opts, report)?;
        new_entries.push(lock_entry);
    }
    Ok(())
}

/// Whether `d`'s constraint is already satisfied by a url-bearing pin in
/// `old_lock`, keyed by its alias — the no-index-consultation fast path
/// (design §5.1).
fn direct_is_reusable(d: &RegDirect, old_lock: &Lockfile) -> bool {
    old_lock
        .get(&d.alias)
        .map(|l| {
            l.url.is_some()
                && l.source.registry.as_deref() == Some(d.pkg.as_str())
                && l.source
                    .version
                    .as_deref()
                    .and_then(|v| crate::version::Version::parse(v).ok())
                    .map(|v| d.constraint.matches(&v))
                    .unwrap_or(false)
        })
        .unwrap_or(false)
}

/// Rebuild a [`Resolved`] pin from an already-locked registry entry (no
/// index access — everything a re-materialise needs is already in the lock).
fn resolved_from_lock(l: &LockEntry) -> Resolved {
    Resolved {
        version: l.source.version.clone().unwrap_or_default(),
        url: l.url.clone().expect("registry lock entry always carries a url"),
        sha256: l.sha256.clone(),
    }
}

/// Materialise (or skip, if unchanged and the receipt is intact) one
/// resolved registry package under lock/receipt key `label`, and return its
/// fresh [`LockEntry`] (`source.registry` pinned to `pkg`, the real registry
/// package id — which may differ from `label` for an aliased direct entry).
fn materialize_registry_pin(
    label: &str,
    pkg: &str,
    resolved: &Resolved,
    locked: Option<&LockEntry>,
    root: &Path,
    opts: &RootOptions,
    report: &mut ManifestReport,
) -> Result<LockEntry, Error> {
    let same_pin = locked
        .map(|l| l.url.as_deref() == Some(resolved.url.as_str()) && l.sha256 == resolved.sha256)
        .unwrap_or(false);
    if same_pin && receipt_intact(root, label) {
        report.skipped.push(label.to_string());
        return Ok(locked.expect("same_pin implies Some").clone());
    }
    let force = receipts::exists(root, label);
    let install_opts = InstallOptions {
        lib_root: opts.lib_root.clone(),
        dest: opts.dest.clone(),
        libraries: None,
        force,
    };
    let ir = registry_install::install_resolved(label, resolved, &install_opts)?;
    report.installed.push(ir);
    Ok(registry_lock_entry(label, pkg, resolved))
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
