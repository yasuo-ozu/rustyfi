//! Manifest-mode install (phase 2): the no-`PATH`
//! `satyrographos install`. Read `Satyristes` + `Satyristes.lock`, diff
//! each entry's freshly-computed source hash against the lock (and against the
//! installed receipt), and re-materialise **only** the entries that actually
//! changed — leaving unchanged entries' files bit-for-bit untouched (so their
//! mtimes survive a no-op install). Every re-materialisation goes through the
//! phase-1 [`install`](crate::ops::install::install) primitive; this
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
//! place** — phase 2 does not prune dropped dependencies. The contract is
//! only "diff … and re-materialise entries whose hash changed"; it is
//! silent on removal, so removed entries keep their installed files and
//! receipt, and are merely reported (and drop out of the rewritten lockfile).

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::error::Error;
use crate::lockfile::{self, LockEntry, Lockfile};
use crate::ops::install::{self, InstallOptions, InstallReport};
use crate::ops::registry_install::{self, Resolved};
use crate::ops::uninstall::RootOptions;
use crate::registry::{self, MultiRegistryDepSource, RegistryOptions};
use crate::roots::RootSelection;
use crate::satyristes;
use crate::solve;
use crate::source::{RegistryConfig, SourceKind};
use crate::version::Constraint;
use crate::{receipts, stage, util};

#[derive(Debug, Default)]
pub struct ManifestReport {
    /// Entries that were (re-)materialised, in manifest order.
    pub installed: Vec<InstallReport>,
    pub skipped: Vec<String>,
    /// Entry names present in the old lockfile but no longer in the manifest;
    /// left installed (not pruned), only reported (see module docs).
    pub removed: Vec<String>,
    /// Configured repositories that could not be reached while re-solving the
    /// registry closure (url, error) — a warning, not a failure, as long as
    /// at least one repository was reachable (mirrors `cmd_search`'s "one
    /// unreachable repository must not hide the others"). Always empty unless
    /// [`install_manifest_reg_multi`]'s registry sub-graph actually needed to
    /// re-consult the index (the reused-pin fast path never touches it).
    pub unreachable_registries: Vec<(String, Error)>,
}

/// Reconcile against the sibling lockfile/receipts with no *explicit*
/// `--registry` override ([`RegistryOptions::default`]). `{ path = … }` sources
/// materialise as in phase 2; a `{ registry = … }` source still resolves if the
/// registry URL is discoverable from `$RUSTYFI_REGISTRY` or the manifest's own
/// `[registry]` url (else [`Error::NoRegistry`]); a `{ git = … }` source
/// clones/reuses its cache under the default (`$RUSTYFI_GIT_SOURCE_CACHE` /
/// XDG) location (saphe 7d slice S3). Callers that need to pass a `--registry`
/// flag, an explicit cache dir, or `--offline` use [`install_manifest_reg`]
/// directly.
pub fn install_manifest(
    manifest_path: &Path,
    opts: &RootOptions,
) -> Result<ManifestReport, Error> {
    install_manifest_reg(manifest_path, opts, &RegistryOptions::default())
}

/// Reconcile `manifest_path`'s `Satyristes` against its sibling
/// `Satyristes.lock` and the receipts in the resolved root, materialising only
/// changed/missing/new entries and rewriting the lockfile to mirror the
/// manifest. `reg_opts` supplies the registry URL/cache for
/// `{ registry = … }` sources and the clone cache/`--offline` for `{ git = …
/// }` sources (saphe 7d slice S3); `{ path = … }` sources never consult it.
pub fn install_manifest_reg(
    manifest_path: &Path,
    opts: &RootOptions,
    reg_opts: &RegistryOptions,
) -> Result<ManifestReport, Error> {
    install_manifest_reg_multi(manifest_path, opts, reg_opts, &[])
}

/// As [`install_manifest_reg`], but consulting every repository in `repos`,
/// in order, when `reg_opts` does not already pin one explicit
/// `--registry`/`$RUSTYFI_REGISTRY` URL — the same coverage `search`/`install
/// NAME` have. Only the "re-solve the whole registry sub-graph fresh" path in
/// `install_registry_closure` ever consults `repos`; the reused-pin fast path
/// is index-free either way.
pub fn install_manifest_reg_multi(
    manifest_path: &Path,
    opts: &RootOptions,
    reg_opts: &RegistryOptions,
    repos: &[RegistryConfig],
) -> Result<ManifestReport, Error> {
    let manifest = satyristes::read_project(manifest_path)?;
    let manifest_dir = manifest_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));

    // Merge the manifest's own `[registry] mirrors`/`kind` into `reg_opts`
    // when the caller did not already set
    // them explicitly (same explicit-wins-over-manifest precedence as
    // `resolve_url`/`resolve_mirrors`) — a project can declare mirrors or a
    // sparse index kind in `Satyristes` and every registry-sourced entry
    // below (both the direct install and the transitive solver closure) picks
    // it up with no extra flag.
    let effective_reg_opts = RegistryOptions {
        mirrors: reg_opts.resolve_mirrors(manifest.registry_mirrors()),
        kind: reg_opts.kind.or_else(|| manifest.registry_kind()),
        ..reg_opts.clone()
    };
    let reg_opts = &effective_reg_opts;

    let root = opts.resolve_managed_root()?;

    let lock_path = lockfile::lock_path_for(manifest_path);
    let old_lock = lockfile::read(&lock_path)?;

    let mut report = ManifestReport::default();
    let mut new_entries: Vec<LockEntry> = Vec::with_capacity(manifest.libraries.len());

    // Registry-sourced entries are handled in a second pass (below): the
    // solver needs every direct registry root at once to compute one
    // consistent transitive closure, so path/git entries are
    // materialised first, in manifest order.
    let mut reg_directs: Vec<RegDirect> = Vec::new();

    for lib in &manifest.libraries {
        let locked = old_lock.get(&lib.name);
        let install_opts = |force: bool| InstallOptions {
            // A reconciled entry names its library, so the manifest decides.
            prefer_library: Some(lib.name.clone()),
            offline: reg_opts.is_offline(),
            verbose: true,
            // A reconciled dependency takes whatever its own manifest declares.
            lang: None,
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
            SourceKind::Git { git, rev } => {
                // Saphe 7d slice S3 (see this fn's doc for the cache/--offline behavior).
                let checkout = registry::acquire_git_source(git, rev, reg_opts)?;
                let current_hash = util::sha256_tree(&checkout.root)?;
                let hash_matches = locked.map(|l| l.sha256 == current_hash).unwrap_or(false);
                if hash_matches && receipt_intact(&root, &lib.name) {
                    report.skipped.push(lib.name.clone());
                    new_entries.push(locked.expect("hash_matches implies Some").clone());
                    continue;
                }
                let force = receipts::exists(&root, &lib.name);
                if !force && locked.is_some() {
                    clear_orphans(&root, &checkout.root)?;
                }
                let source = receipts::Source {
                    kind: "git".to_string(),
                    value: git.to_string(),
                    version: Some(checkout.resolved_rev.clone()),
                    url: Some(git.to_string()),
                    sha256: None,
                };
                let ir =
                    install::install_inner(&checkout.root, &install_opts(force), Some(source))?;
                report.installed.push(ir);
                new_entries.push(LockEntry {
                    name: lib.name.clone(),
                    source: crate::source::SourceSpec {
                        git: Some(git.to_string()),
                        rev: Some(checkout.resolved_rev.clone()),
                        ..Default::default()
                    },
                    sha256: current_hash,
                    url: None,
                    resolved_at: util::now_rfc3339(),
                });
            }
        }
    }

    if !reg_directs.is_empty() {
        install_registry_closure(
            &reg_directs, &manifest, &old_lock, &root, opts, reg_opts, repos, &mut report, &mut new_entries,
        )?;
    }

    // "Dropped" means neither a manifest library still names it NOR the fresh
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
/// transitive closure and materialise it, pushing one
/// [`LockEntry`] per closure member (direct **and** transitive) onto
/// `new_entries`.
///
/// Reproducibility: when every direct root's constraint is already satisfied
/// by a url-bearing pin in `old_lock`, this whole pass reuses the existing
/// closure (every registry-kind entry in `old_lock`, direct or transitive)
/// via its locked `(version, url, sha256)` — **without touching the index at
/// all**. The solver (and thus the index) is consulted only when
/// at least one direct root is new or its requested version/constraint
/// changed; in that case the *entire* registry sub-graph is re-solved fresh
/// (not incrementally pinned), and the resulting closure replaces every prior
/// registry-kind lock entry.
#[allow(clippy::too_many_arguments)]
fn install_registry_closure(
    reg_directs: &[RegDirect],
    manifest: &satyristes::Project,
    old_lock: &Lockfile,
    root: &Path,
    opts: &RootOptions,
    reg_opts: &RegistryOptions,
    repos: &[RegistryConfig],
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
            let entry = materialize_registry_pin(
                &d.alias, &d.pkg, &resolved, Some(locked), root, opts, reg_opts, report,
            )?;
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
            let entry = materialize_registry_pin(
                &l.name, &pkg, &resolved, Some(l), root, opts, reg_opts, report,
            )?;
            new_entries.push(entry);
        }
        return Ok(());
    }

    let fallback = manifest.registry_url();
    let (acquired, unreachable) = registry::acquire_all(repos, reg_opts, fallback)?;
    report.unreachable_registries.extend(unreachable);
    let solve_root: Vec<(String, Constraint)> =
        reg_directs.iter().map(|d| (d.pkg.clone(), d.constraint.clone())).collect();
    let src = MultiRegistryDepSource::new(&acquired);
    let solution = solve::solve(&solve_root, &src)?;

    for (pkg, version) in &solution.packages {
        let (_, idx) = src.lookup_first(pkg)?;
        let entry = registry::entry_for(&idx, version).ok_or_else(|| Error::VersionNotFound {
            name: pkg.clone(),
            version: version.to_string(),
        })?;
        let resolved = Resolved {
            version: version.to_string(),
            url: entry.tarball_url.clone(),
            sha256: entry.sha256.clone(),
            sha512: entry.sha512.clone(),
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
        let lock_entry = materialize_registry_pin(
            label, pkg, &resolved, locked, root, opts, reg_opts, report,
        )?;
        new_entries.push(lock_entry);
    }
    Ok(())
}

/// Whether `d`'s constraint is already satisfied by a url-bearing pin in
/// `old_lock`, keyed by its alias — the no-index-consultation fast path.
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
        // A lock pins one digest, in the sha256 field regardless of which
        // algorithm resolved it.
        sha512: None,
    }
}

/// Materialise (or skip, if unchanged and the receipt is intact) one
/// resolved registry package under lock/receipt key `label`, and return its
/// fresh [`LockEntry`] (`source.registry` pinned to `pkg`, the real registry
/// package id — which may differ from `label` for an aliased direct entry).
/// `reg_opts` carries the archive cache location and `--offline`/
/// `$RUSTYFI_OFFLINE` (phase 7d S2) through to the fetch, when a
/// re-materialisation is actually needed (the `same_pin`/receipt-intact skip
/// below is the *first*, cheaper offline win — no cache lookup at all).
#[allow(clippy::too_many_arguments)]
fn materialize_registry_pin(
    label: &str,
    pkg: &str,
    resolved: &Resolved,
    locked: Option<&LockEntry>,
    root: &Path,
    opts: &RootOptions,
    reg_opts: &RegistryOptions,
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
        prefer_library: Some(label.to_string()),
        offline: reg_opts.is_offline(),
        verbose: true,
        lang: None,
        lib_root: opts.lib_root.clone(),
        dest: opts.dest.clone(),
        libraries: None,
        force,
    };
    let ir = registry_install::install_resolved(label, resolved, &install_opts, reg_opts)?;
    report.installed.push(ir);
    Ok(registry_lock_entry(label, pkg, resolved))
}

/// Build a lockfile entry for a freshly-resolved registry install, pinning the
/// concrete resolved version (in `source.version`), tarball url, and verified
/// tarball sha256.
fn registry_lock_entry(name: &str, pkg: &str, resolved: &Resolved) -> LockEntry {
    LockEntry {
        name: name.to_string(),
        source: crate::source::SourceSpec {
            registry: Some(pkg.to_string()),
            version: Some(resolved.version.clone()),
            ..Default::default()
        },
        sha256: resolved.sha256.clone(),
        url: Some(resolved.url.clone()),
        resolved_at: util::now_rfc3339(),
    }
}

/// Resolve a manifest `path` source relative to the `Satyristes`'s own
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
