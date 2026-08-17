//! The phase-3 remote registry (plan §5.4): a git-hosted (or plain-directory)
//! tree of TOML index entries, one file per package at `<index>/packages/
//! <name>.toml`, each listing `[versions."<v>"]` tables with a `tarball_url`
//! and a `sha256`:
//!
//! ```toml
//! description = "A great SATySFi package"   # optional (this port's addition, for `search`)
//! [versions."1.0.0"]
//! tarball_url = "https://example.com/great-package-1.0.0.tar.gz"
//! sha256 = "…"
//! [versions."1.1.0"]
//! tarball_url = "https://example.com/great-package-1.1.0.tar.gz"
//! sha256 = "…"
//! ```
//!
//! ## What this module does
//!
//! - **Acquire** the index into a local directory ([`acquire`]): a
//!   plain-directory index (a directory that already holds `packages/`, whether
//!   hand-made or a checked-out git worktree) is read in place; a git index (a
//!   bare local repo via `file://`, or any remote URL) is shallow-cloned/fetched
//!   into a cache dir by shelling out to `git` (no libgit2 dependency, plan §8).
//! - **Look up** `packages/<name>.toml` and **select a version** ([`lookup`],
//!   [`select_version`]): the exact requested version, else the highest by a
//!   simple dotted-numeric comparison (plan §5.4 step 2).
//! - **Fetch** a `tarball_url` into a destination file ([`fetch_tarball`]):
//!   `file://`/plain paths are copied directly (offline); `http(s)://` goes
//!   through the feature-gated [`http`] client (plan §8: path/archive/local
//!   installs stay offline with the `http` feature off).
//! - **Verify** the tarball's SHA-256 against the index entry ([`verify_sha256`])
//!   — the caller does this *before* touching `dist/` (plan §5.4 step 3).
//!
//! The fetch → verify → materialize orchestration lives in
//! [`crate::ops::install`] (registry form) and [`crate::ops::reconcile`]
//! (manifest form); this module is the index/transport layer only.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::error::Error;
use crate::util;

/// The environment variable consulted for the registry URL when no
/// `--registry` flag and no `Satyrfile.toml` `[registry]` url is given
/// (plan §5.4 step 1).
pub const REGISTRY_ENV: &str = "SATYSFI_REGISTRY";

/// Environment override for the git-index cache directory (used by tests to
/// stay hermetic; production defaults to `$XDG_CACHE_HOME/satysfi-rust/
/// registry/`).
pub const CACHE_ENV: &str = "SATYSFI_REGISTRY_CACHE";

/// How to reach and cache a registry index (plan §5.4 step 1).
#[derive(Debug, Default, Clone)]
pub struct RegistryOptions {
    /// The `--registry URL` flag, if given (highest precedence).
    pub url: Option<String>,
    /// The git-index cache directory; falls back to `$SATYSFI_REGISTRY_CACHE`
    /// then `$XDG_CACHE_HOME/satysfi-rust/registry/`.
    pub cache_dir: Option<PathBuf>,
    /// Re-fetch a git index even if it is already cloned in the cache
    /// (`satyrographos update` sets this; plain `install` reuses the cache).
    pub refresh: bool,
}

impl RegistryOptions {
    /// Resolve the registry URL, preferring the explicit `--registry` flag,
    /// then `$SATYSFI_REGISTRY`, then the `fallback` (a `Satyrfile.toml`
    /// `[registry] url`, if any). No built-in default is shipped (the hosting
    /// repo is undecided, plan §10), so an unresolved URL is
    /// [`Error::NoRegistry`].
    pub fn resolve_url(&self, fallback: Option<&str>) -> Result<String, Error> {
        if let Some(u) = &self.url {
            return Ok(u.clone());
        }
        if let Some(u) = std::env::var_os(REGISTRY_ENV) {
            if !u.is_empty() {
                return Ok(u.to_string_lossy().into_owned());
            }
        }
        if let Some(u) = fallback {
            return Ok(u.to_string());
        }
        Err(Error::NoRegistry)
    }

    /// The cache directory for git-cloned indexes.
    fn cache_root(&self) -> PathBuf {
        if let Some(dir) = &self.cache_dir {
            return dir.clone();
        }
        if let Some(dir) = std::env::var_os(CACHE_ENV) {
            if !dir.is_empty() {
                return PathBuf::from(dir);
            }
        }
        let base = std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
            .unwrap_or_else(|| PathBuf::from(".cache"));
        base.join("satysfi-rust").join("registry")
    }
}

/// An acquired registry index: a live local directory holding `packages/`,
/// plus the resolved git commit sha when the index is a git repo.
#[derive(Debug)]
pub struct Registry {
    /// The index root (contains `packages/<name>.toml`).
    pub root: PathBuf,
    /// The resolved commit sha for a git index (`None` for a plain directory).
    pub commit: Option<String>,
}

/// Acquire the registry index named by `url` into a readable local directory
/// (plan §5.4 step 1).
///
/// - A **plain-directory index** — a local path (or `file://` URL) pointing at
///   a directory that already contains `packages/` — is used in place, no git.
/// - A **git index** — a bare local repo (`file:///…/foo.git`) or any remote
///   URL — is shallow-cloned into `opts.cache_dir` (or fetched, when
///   `opts.refresh`) by shelling out to `git`.
pub fn acquire(url: &str, opts: &RegistryOptions) -> Result<Registry, Error> {
    // Plain-directory index: a local path already holding `packages/`.
    if let Some(local) = local_path_from_url(url) {
        if local.join("packages").is_dir() {
            return Ok(Registry {
                root: local,
                commit: None,
            });
        }
    }
    // Otherwise treat `url` as a git remote (or bare local repo) and clone/fetch.
    git_acquire(url, opts)
}

fn git_acquire(url: &str, opts: &RegistryOptions) -> Result<Registry, Error> {
    let cache_root = opts.cache_root();
    std::fs::create_dir_all(&cache_root).map_err(|e| Error::io(&cache_root, e))?;
    let dest = cache_root.join(cache_key(url));

    if dest.join(".git").is_dir() {
        if opts.refresh {
            run_git(&["-C", &dest.to_string_lossy(), "fetch", "--depth", "1", "origin"])?;
            // Move the working tree to the freshly fetched tip.
            run_git(&[
                "-C",
                &dest.to_string_lossy(),
                "reset",
                "--hard",
                "FETCH_HEAD",
            ])?;
        }
    } else {
        // A stale non-git dir would make clone fail; clear it first.
        if dest.exists() {
            std::fs::remove_dir_all(&dest).map_err(|e| Error::io(&dest, e))?;
        }
        run_git(&[
            "clone",
            "--depth",
            "1",
            url,
            &dest.to_string_lossy(),
        ])?;
    }

    let commit = git_head(&dest).ok();
    if !dest.join("packages").is_dir() {
        return Err(Error::RegistryIndex {
            message: format!(
                "cloned index at {} has no packages/ directory",
                dest.display()
            ),
        });
    }
    Ok(Registry {
        root: dest,
        commit,
    })
}

/// Run `git <args>`, mapping a non-zero exit (or a missing `git`) to
/// [`Error::GitFailed`].
fn run_git(args: &[&str]) -> Result<(), Error> {
    let output = std::process::Command::new("git")
        .args(args)
        .output()
        .map_err(|e| Error::GitFailed {
            message: format!("cannot run git: {e}"),
        })?;
    if !output.status.success() {
        return Err(Error::GitFailed {
            message: format!(
                "git {} failed: {}",
                args.first().copied().unwrap_or(""),
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        });
    }
    Ok(())
}

fn git_head(repo: &Path) -> Result<String, Error> {
    let output = std::process::Command::new("git")
        .args(["-C", &repo.to_string_lossy(), "rev-parse", "HEAD"])
        .output()
        .map_err(|e| Error::GitFailed {
            message: format!("cannot run git: {e}"),
        })?;
    if !output.status.success() {
        return Err(Error::GitFailed {
            message: "git rev-parse HEAD failed".to_string(),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// A filesystem-safe cache subdirectory name for a registry URL (the first 8
/// bytes of its SHA-256 as hex, so two different registries never collide in the
/// cache).
fn cache_key(url: &str) -> String {
    util::sha256_hex(url.as_bytes())[..16].to_string()
}

// ---------------------------------------------------------------------------
// Index entry format + lookup.
// ---------------------------------------------------------------------------

/// A parsed `packages/<name>.toml` index entry.
#[derive(Debug, Clone, Deserialize)]
pub struct PackageIndex {
    /// Optional package description (this port's addition; surfaced by
    /// `search`).
    #[serde(default)]
    pub description: Option<String>,
    /// Released versions, keyed by version string.
    #[serde(default)]
    pub versions: BTreeMap<String, VersionEntry>,
}

/// One `[versions."<v>"]` table.
#[derive(Debug, Clone, Deserialize)]
pub struct VersionEntry {
    pub tarball_url: String,
    pub sha256: String,
}

impl Registry {
    /// The on-disk path of `packages/<name>.toml`.
    pub fn package_path(&self, name: &str) -> PathBuf {
        self.root.join("packages").join(format!("{name}.toml"))
    }
}

/// Read and parse `packages/<name>.toml` from an acquired index. A missing
/// file is [`Error::PackageNotFound`].
pub fn lookup(reg: &Registry, name: &str) -> Result<PackageIndex, Error> {
    let path = reg.package_path(name);
    if !path.is_file() {
        return Err(Error::PackageNotFound {
            name: name.to_string(),
        });
    }
    let text = util::read_to_string(&path)?;
    toml::from_str(&text).map_err(|source| Error::RegistryIndex {
        message: format!("{}: {source}", path.display()),
    })
}

/// Select a version from an index entry (plan §5.4 step 2): the exact `req`
/// version if given (else [`Error::VersionNotFound`]), otherwise the highest
/// available by [`version_cmp`].
pub fn select_version<'a>(
    idx: &'a PackageIndex,
    name: &str,
    req: Option<&str>,
) -> Result<(String, &'a VersionEntry), Error> {
    if let Some(req) = req {
        return idx
            .versions
            .get(req)
            .map(|v| (req.to_string(), v))
            .ok_or_else(|| Error::VersionNotFound {
                name: name.to_string(),
                version: req.to_string(),
            });
    }
    idx.versions
        .iter()
        .max_by(|(a, _), (b, _)| version_cmp(a, b))
        .map(|(v, entry)| (v.clone(), entry))
        .ok_or_else(|| Error::VersionNotFound {
            name: name.to_string(),
            version: "<any>".to_string(),
        })
}

/// A simple "semver-ish" ordering (plan §5.4 step 2: "simple string/semver-ish
/// comparison"): compare dotted components numerically where both parse as
/// integers, else lexically; a shorter prefix (`1.0` vs `1.0.1`) sorts lower.
pub fn version_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let mut ai = a.split('.');
    let mut bi = b.split('.');
    loop {
        match (ai.next(), bi.next()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) => {
                let ord = match (x.parse::<u64>(), y.parse::<u64>()) {
                    (Ok(nx), Ok(ny)) => nx.cmp(&ny),
                    _ => x.cmp(y),
                };
                if ord != Ordering::Equal {
                    return ord;
                }
            }
        }
    }
}

/// Every package name in the index (the stems of `packages/*.toml`), sorted —
/// used by `search`/`update` to enumerate the whole index.
pub fn all_package_names(reg: &Registry) -> Result<Vec<String>, Error> {
    let dir = reg.root.join("packages");
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut names = Vec::new();
    for path in util::read_dir_paths(&dir)? {
        if path.extension().and_then(|e| e.to_str()) == Some("toml") {
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                names.push(stem.to_string());
            }
        }
    }
    names.sort();
    Ok(names)
}

// ---------------------------------------------------------------------------
// Tarball fetch + verify.
// ---------------------------------------------------------------------------

/// Fetch `tarball_url` into the file at `dest` (plan §5.4 step 3).
///
/// - `file://` / plain local paths are copied directly (fully offline).
/// - `http(s)://` requires the `http` cargo feature; without it,
///   [`Error::HttpDisabled`].
pub fn fetch_tarball(url: &str, dest: &Path) -> Result<(), Error> {
    if let Some(local) = local_path_from_url(url) {
        std::fs::copy(&local, dest).map_err(|e| Error::io(&local, e))?;
        return Ok(());
    }
    http::get_to_file(url, dest)
}

/// Verify that the file at `path` hashes to `expected` (lowercase-hex SHA-256,
/// compared case-insensitively). [`Error::ChecksumMismatch`] otherwise — the
/// caller aborts before touching `dist/` (plan §5.4 step 3).
pub fn verify_sha256(path: &Path, expected: &str) -> Result<String, Error> {
    let actual = util::sha256_file(path)?;
    if !actual.eq_ignore_ascii_case(expected.trim()) {
        return Err(Error::ChecksumMismatch {
            expected: expected.trim().to_lowercase(),
            actual,
        });
    }
    Ok(actual)
}

/// Interpret a registry/tarball URL as a local filesystem path, or `None` for a
/// network URL. `file://` (optionally `file://localhost/…`) is stripped to its
/// path; a string with no `scheme://` is treated as a plain local path.
pub fn local_path_from_url(url: &str) -> Option<PathBuf> {
    if let Some(rest) = url.strip_prefix("file://") {
        // The authority must be empty (`file:///abs/path`) or exactly
        // `localhost` (`file://localhost/abs/path`); any other host is a
        // non-local URL we do not treat as an on-disk path (so it falls through
        // to the git-clone path instead of silently reading a bogus directory).
        if let Some(after) = rest.strip_prefix("localhost/") {
            return Some(PathBuf::from(format!("/{after}")));
        }
        if rest == "localhost" {
            return None; // no path component
        }
        if rest.starts_with('/') {
            return Some(PathBuf::from(rest));
        }
        return None;
    }
    if url.contains("://") {
        // http://, https://, git://, ssh://, … — not a local path.
        return None;
    }
    Some(PathBuf::from(url))
}

// ---------------------------------------------------------------------------
// Feature-gated HTTP transport (plan §8).
// ---------------------------------------------------------------------------

mod http {
    use super::*;

    #[cfg(feature = "http")]
    pub fn get_to_file(url: &str, dest: &Path) -> Result<(), Error> {
        let resp = ureq::get(url).call().map_err(|e| Error::HttpFailed {
            url: url.to_string(),
            message: e.to_string(),
        })?;
        let mut reader = resp.into_reader();
        let mut file = std::fs::File::create(dest).map_err(|e| Error::io(dest, e))?;
        std::io::copy(&mut reader, &mut file).map_err(|e| Error::io(dest, e))?;
        Ok(())
    }

    #[cfg(not(feature = "http"))]
    pub fn get_to_file(url: &str, _dest: &Path) -> Result<(), Error> {
        Err(Error::HttpDisabled {
            url: url.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cmp::Ordering;

    #[test]
    fn version_cmp_is_dotted_numeric() {
        assert_eq!(version_cmp("1.10.0", "1.9.0"), Ordering::Greater);
        assert_eq!(version_cmp("1.0.0", "1.0.0"), Ordering::Equal);
        assert_eq!(version_cmp("1.0", "1.0.1"), Ordering::Less);
        assert_eq!(version_cmp("2.0.0", "1.99.99"), Ordering::Greater);
    }

    #[test]
    fn local_path_from_url_handles_file_and_plain() {
        assert_eq!(
            local_path_from_url("file:///tmp/index"),
            Some(PathBuf::from("/tmp/index"))
        );
        assert_eq!(
            local_path_from_url("file://localhost/tmp/index"),
            Some(PathBuf::from("/tmp/index"))
        );
        assert_eq!(
            local_path_from_url("/plain/abs/path"),
            Some(PathBuf::from("/plain/abs/path"))
        );
        // A non-empty, non-localhost authority is NOT a local path (falls
        // through to the git-clone transport) — and no host is mangled.
        assert_eq!(local_path_from_url("file://realhost/srv/index"), None);
        assert_eq!(local_path_from_url("file://localhost-mirror/pkg"), None);
        assert_eq!(local_path_from_url("file://localhost"), None);
        // Network URLs are never local.
        assert_eq!(local_path_from_url("https://example.com/x.tar.gz"), None);
        assert_eq!(local_path_from_url("git://host/repo.git"), None);
    }
}
