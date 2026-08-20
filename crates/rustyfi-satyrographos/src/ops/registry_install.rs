//! Registry-source install (plan §5.4's fetch/verify/materialize algorithm):
//! the `satyrographos install <name>[@version]` form, and the reproducible
//! "install from an already-locked (url, sha256)" form that
//! [`reconcile`](crate::ops::reconcile) uses.
//!
//! The algorithm, step for step against plan §5.4:
//!
//! 1. Resolve the registry URL and **acquire** the index
//!    ([`registry::acquire`]); look up `packages/<name>.toml` and **select**
//!    the version ([`registry::select_version`], steps 1-2).
//! 2. Resolve the library root and download the chosen `tarball_url` into
//!    `<root>/.satyrographos/tmp/` (step 3).
//! 3. **Verify** the SHA-256 and *abort before touching `dist/`* on mismatch
//!    ([`registry::verify_sha256`], step 3) — a failed verify leaves the root's
//!    `dist/` and receipts completely untouched.
//! 4. Feed the verified tarball through the exact phase-1
//!    [`install`](crate::ops::install) materializer, recording a
//!    `registry`-flavored receipt source (step 4).
//!
//! No transitive dependency resolution (step 5) — that stays out of scope
//! through phase 3.

use std::path::PathBuf;

use crate::error::Error;
use crate::ops::install::{self, InstallOptions, InstallReport};
use crate::receipts::Source;
use crate::registry::{self, RegistryOptions};
use crate::roots::{self, RootSelection};
use crate::stage;

/// The concrete `(version, tarball_url, sha256)` an index lookup resolved to —
/// exactly what the lockfile pins so a later install is reproducible without
/// re-consulting the index (plan §5.4 step 4 / §5.3 lock semantics).
#[derive(Debug, Clone)]
pub struct Resolved {
    pub version: String,
    pub url: String,
    pub sha256: String,
    /// The sha512, when the index declared that instead (an OPAM repository
    /// publishes md5 and sha512, no sha256).
    pub sha512: Option<String>,
}

/// Resolve `name`[`@version`] through the registry index, returning the
/// concrete pin without downloading anything (used by `reconcile`/`update`).
pub fn resolve(
    name: &str,
    version_req: Option<&str>,
    reg_opts: &RegistryOptions,
    registry_url_fallback: Option<&str>,
) -> Result<Resolved, Error> {
    let url = reg_opts.resolve_url(registry_url_fallback)?;
    let reg = registry::acquire(&url, reg_opts)?;
    let idx = registry::lookup(&reg, name)?;
    let (version, entry) = registry::select_version(&idx, name, version_req)?;
    Ok(Resolved {
        version,
        url: entry.tarball_url.clone(),
        sha256: entry.sha256.clone(),
        sha512: entry.sha512.clone(),
    })
}

/// The full index-consulting install (plan §5.4): resolve → download → verify →
/// materialize. Returns both the install report and the resolved pin (so a
/// caller can record it in the lockfile).
pub fn install_registry(
    name: &str,
    version_req: Option<&str>,
    opts: &InstallOptions,
    reg_opts: &RegistryOptions,
    registry_url_fallback: Option<&str>,
) -> Result<(InstallReport, Resolved), Error> {
    let resolved = resolve(name, version_req, reg_opts, registry_url_fallback)?;
    let report = install_resolved(name, &resolved, opts, reg_opts)?;
    Ok((report, resolved))
}

/// Download + verify + materialize an already-resolved pin (plan §5.4 steps
/// 3-4). This is the reproducible path: given a `(url, sha256)` — whether just
/// looked up or carried forward from a lockfile — it never consults the index.
/// `reg_opts` supplies the archive cache location and `--offline`/
/// `$RUSTYFI_OFFLINE` (phase 7d S2) for [`registry::fetch_tarball`]; it is
/// otherwise unused here (no index lookup happens on this path).
pub fn install_resolved(
    name: &str,
    resolved: &Resolved,
    opts: &InstallOptions,
    reg_opts: &RegistryOptions,
) -> Result<InstallReport, Error> {
    // Resolve the root now so the download lands under it (same filesystem as
    // `dist/`, plan §5.4 step 3 / §6) and is verified before anything is staged.
    let root = opts.resolve_managed_root()?;

    let tarball = roots::tmp_dir(&root).join(format!(
        "download-{}-{}.tar.gz",
        name,
        stage::unique_suffix()
    ));
    let _guard = FileGuard(tarball.clone());

    // Step 3: fetch (via the archive cache when the url is a network url,
    // phase 7d S2), then verify BEFORE touching dist/ — a mismatch aborts here.
    let checksum = registry::Checksum::new(&resolved.sha256, resolved.sha512.as_deref());
    registry::fetch_tarball(&resolved.url, &checksum, &tarball, reg_opts)?;
    checksum.verify(&tarball)?;

    // Step 4: reuse the phase-1 materializer, recording a registry receipt.
    let source = Source {
        kind: "registry".to_string(),
        value: name.to_string(),
        version: Some(resolved.version.clone()),
        url: Some(resolved.url.clone()),
        sha256: Some(resolved.sha256.clone()),
    };
    install::install_inner(&tarball, opts, Some(source))
}

/// Removes a downloaded tarball when dropped (best-effort), so a verified or
/// failed install leaves no stray file in `.satyrographos/tmp/`.
struct FileGuard(PathBuf);

impl Drop for FileGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
