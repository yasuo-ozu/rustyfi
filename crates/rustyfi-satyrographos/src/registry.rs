//! The phase-3 remote registry: a git-hosted (or plain-directory)
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
//! `acquire`/`lookup`/`select_version` read the index; `fetch_tarball`
//! + `verify_sha256` fetch and verify a tarball through
//! [`crate::cache`]'s content-addressed archive cache (slice S2);
//! `acquire_git_source`
//! (slice S3) clones a `Satyristes` `{ git = …, rev = … }` source directly, no
//! archive/verify step. Optional bearer-token auth on the HTTP transport is
//! [`REGISTRY_TOKEN_ENV`] (slice S3).
//!
//! The fetch → verify → materialize orchestration lives in
//! [`crate::ops::install`]/[`crate::ops::reconcile`]; this module is the
//! index/transport layer only.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::error::Error;
use crate::solve::DepSource;
use crate::util;
use crate::version::{Constraint, Version};

/// Registry URL, used when no `--registry` flag / `Satyristes` url is given.
pub const REGISTRY_ENV: &str = "RUSTYFI_REGISTRY";

/// Git-index cache directory override; default `$XDG_CACHE_HOME/rustyfi/registry/`.
pub const CACHE_ENV: &str = "RUSTYFI_REGISTRY_CACHE";

/// `RUSTYFI_OFFLINE=1` behaves like `--offline` for every registry operation
/// (slice S2).
pub const OFFLINE_ENV: &str = "RUSTYFI_OFFLINE";

/// Git package-source clone cache directory override (slice S3); sibling of
/// [`CACHE_ENV`]/[`crate::cache::ARCHIVE_CACHE_ENV`] under its own leaf so a
/// `{ git = … }` source never collides with an index clone or a tarball blob.
/// Default `$XDG_CACHE_HOME/rustyfi/git-sources/`.
pub const GIT_SOURCE_CACHE_ENV: &str = "RUSTYFI_GIT_SOURCE_CACHE";

/// Overrides the HTTP connect/read timeout in seconds (default 30s); tests set
/// this small so a stalled-server case does not wait out the real default.
/// Only consulted when the `http` cargo feature is compiled in.
#[cfg(feature = "http")]
pub const HTTP_TIMEOUT_ENV: &str = "RUSTYFI_HTTP_TIMEOUT";

/// Bearer-token auth for the HTTP transport (slice S3): when set, every `GET`
/// carries an `Authorization: Bearer <token>` header. Documented user-facing
/// in the CLI's man page. Only consulted when the `http` cargo feature is
/// compiled in.
#[cfg(feature = "http")]
pub const REGISTRY_TOKEN_ENV: &str = "RUSTYFI_REGISTRY_TOKEN";

/// How to reach and cache a registry index.
#[derive(Debug, Default, Clone)]
pub struct RegistryOptions {
    /// The `--registry URL` flag, if given (highest precedence).
    pub url: Option<String>,
    /// Git-index cache dir; falls back to `$RUSTYFI_REGISTRY_CACHE` then
    /// `$XDG_CACHE_HOME/rustyfi/registry/`.
    pub cache_dir: Option<PathBuf>,
    /// Re-fetch a git index even if already cloned in the cache
    /// (`satyrographos update` sets this; plain `install` reuses the cache).
    pub refresh: bool,
    /// Content-addressed archive cache dir (slice S2); falls back to
    /// `$RUSTYFI_ARCHIVE_CACHE` then `$XDG_CACHE_HOME/rustyfi/archives/`
    /// (see [`crate::cache`]).
    pub archive_cache_dir: Option<PathBuf>,
    /// Git package-source clone cache dir (`acquire_git_source`); falls
    /// back to `$RUSTYFI_GIT_SOURCE_CACHE` then
    /// `$XDG_CACHE_HOME/rustyfi/git-sources/`.
    pub git_source_cache_dir: Option<PathBuf>,
    /// Forbid network requests (`--offline`); see `RegistryOptions::is_offline`.
    pub offline: bool,
    /// Fallback registry base URLs, tried in order after the primary URL
    /// when a fetch fails: a tarball fetch (`fetch_tarball`) or, once a
    /// registry's `kind` is `Sparse`, a per-package index GET. Each
    /// candidate is a host/prefix substitution applied to the primary URL's
    /// path+query (`rewrite_to_mirror`), verified against the same sha256
    /// for a tarball. An explicit flag/env value takes precedence over a
    /// `Satyristes` `[registry] mirrors` fallback (`RegistryOptions::resolve_mirrors`).
    pub mirrors: Vec<String>,
    /// The index transport. `None` = local-dir/git dispatch by URL shape
    /// ([`crate::source::RegistryKind::Auto`]); only `Some(Sparse)` selects the
    /// per-package HTTP index path.
    pub kind: Option<crate::source::RegistryKind>,
}

impl RegistryOptions {
    /// Whether network access is forbidden (slice S2): `offline` (set by
    /// `--offline`) or `$RUSTYFI_OFFLINE=1`. When true, both index acquisition
    /// ([`acquire`]) and the archive fetch must resolve entirely from what
    /// is already cached, returning [`Error::Offline`] on any miss.
    pub(crate) fn is_offline(&self) -> bool {
        self.offline
            || std::env::var_os(OFFLINE_ENV)
                .map(|v| v == "1")
                .unwrap_or(false)
    }

    /// Whether the first two rungs of [`Self::resolve_url`] — the
    /// `--registry` flag and `$RUSTYFI_REGISTRY` — already settle which
    /// repository is meant, so no `fallback` will be consulted.
    ///
    /// [`crate::ops::publish`] needs the question separately from the answer:
    /// it must refuse a *list* of configured repositories rather than take the
    /// first, and only an explicit choice makes that list irrelevant. Kept
    /// beside `resolve_url` so the two cannot disagree about what "explicit"
    /// means.
    pub(crate) fn has_explicit_url(&self) -> bool {
        self.url.is_some()
            || std::env::var_os(REGISTRY_ENV)
                .map(|u| !u.is_empty())
                .unwrap_or(false)
    }

    /// Resolve the registry URL: explicit `--registry` flag, then
    /// `$RUSTYFI_REGISTRY`, then `fallback` (a `Satyristes` `[registry]
    /// url`). No built-in default is shipped, so an unresolved URL is
    /// [`Error::NoRegistry`].
    pub(crate) fn resolve_url(&self, fallback: Option<&str>) -> Result<String, Error> {
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

    /// Resolve the mirror candidate list: explicit `self.mirrors` takes
    /// precedence over `fallback` (a `Satyristes` `[registry] mirrors`),
    /// same precedence as [`Self::resolve_url`]. `fallback` is used verbatim (not
    /// merged) when `self.mirrors` is empty.
    pub(crate) fn resolve_mirrors(&self, fallback: &[String]) -> Vec<String> {
        if !self.mirrors.is_empty() {
            self.mirrors.clone()
        } else {
            fallback.to_vec()
        }
    }

    fn cache_root(&self) -> PathBuf {
        if let Some(dir) = &self.cache_dir {
            return dir.clone();
        }
        if let Some(dir) = std::env::var_os(CACHE_ENV) {
            if !dir.is_empty() {
                return PathBuf::from(dir);
            }
        }
        util::xdg_cache_base().join("rustyfi").join("registry")
    }

    fn git_source_cache_root(&self) -> PathBuf {
        if let Some(dir) = &self.git_source_cache_dir {
            return dir.clone();
        }
        if let Some(dir) = std::env::var_os(GIT_SOURCE_CACHE_ENV) {
            if !dir.is_empty() {
                return PathBuf::from(dir);
            }
        }
        util::xdg_cache_base().join("rustyfi").join("git-sources")
    }
}

/// An acquired registry index: either a live local directory holding
/// `packages/` (a git clone or a plain directory), or a sparse HTTP index
/// fetched one `packages/<name>.toml` at a time. `commit` is the resolved
/// git commit sha for a git index (`None` for a plain directory or a sparse
/// index — neither has a single-snapshot commit to report).
#[derive(Debug)]
pub struct Registry {
    backend: RegistryBackend,
    pub commit: Option<String>,
}

/// The concrete transport behind an acquired [`Registry`].
#[derive(Debug)]
enum RegistryBackend {
    /// A local directory holding `packages/<name>.toml` — a git clone or a
    /// hand-made/checked-out plain directory.
    Local(PathBuf),
    /// A sparse HTTP index: `packages/<name>.toml` is GET-ed lazily
    /// (mirror-rewritten via `opts.mirrors`) the first time each package
    /// name is looked up, and cached in `cache` for the remainder of this
    /// `Registry`'s lifetime — a within-process cache, not a persistent
    /// on-disk one.
    Sparse {
        base: String,
        opts: RegistryOptions,
        cache: std::cell::RefCell<BTreeMap<String, String>>,
    },
}

/// Acquire the registry index named by `url` into a queryable index handle.
///
/// `opts.kind` selects the transport:
/// - `None`/`Auto` (the default): a **plain-directory index** — a local path
///   (or `file://` URL) pointing at a directory that already contains
///   `packages/` — is used in place, no git; anything else is treated as a git
///   remote (or bare local repo) and shallow-cloned/fetched.
/// - `Git`: always the git path, even if `url` happens to resolve to a local
///   directory that already has `packages/` (skips `Auto`'s short-circuit).
/// - `Sparse`: `url` is a sparse-index base — no clone at all; `lookup` fetches
///   `packages/<name>.toml` over HTTP on demand.
pub(crate) fn acquire(url: &str, opts: &RegistryOptions) -> Result<Registry, Error> {
    use crate::source::RegistryKind;
    match opts.kind.unwrap_or(RegistryKind::Auto) {
        RegistryKind::Sparse => Ok(Registry {
            backend: RegistryBackend::Sparse {
                base: url.trim_end_matches('/').to_string(),
                opts: opts.clone(),
                cache: std::cell::RefCell::new(BTreeMap::new()),
            },
            commit: None,
        }),
        RegistryKind::Git => git_acquire(url, opts),
        RegistryKind::Auto => {
            if let Some(local) = local_path_from_url(url) {
                if local.join("packages").is_dir() {
                    return Ok(Registry {
                        backend: RegistryBackend::Local(local),
                        commit: None,
                    });
                }
            }
            git_acquire(url, opts)
        }
    }
}

fn git_acquire(url: &str, opts: &RegistryOptions) -> Result<Registry, Error> {
    let cache_root = opts.cache_root();
    std::fs::create_dir_all(&cache_root).map_err(|e| Error::io(&cache_root, e))?;
    let dest = cache_root.join(cache_key(url));
    let already_cloned = dest.join(".git").is_dir();

    // Offline (slice S2): never shell out to `git`. Already-cloned is reused as-is
    // (a stale `refresh` is silently skipped); never-cloned is a clean
    // `Error::Offline` rather than a `git clone` against a real network.
    if opts.is_offline() {
        if !already_cloned {
            return Err(Error::Offline {
                url: url.to_string(),
            });
        }
    } else if already_cloned {
        if opts.refresh {
            run_git(&["-C", &dest.to_string_lossy(), "fetch", "--depth", "1", "origin"])?;
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
        backend: RegistryBackend::Local(dest),
        commit,
    })
}

/// Maps a non-zero exit (or missing `git`) to [`Error::GitFailed`].
pub(crate) fn run_git(args: &[&str]) -> Result<(), Error> {
    git_capture(args).map(|_| ())
}

/// [`run_git`] keeping stdout (trimmed). The whole argument list goes into the
/// failure message rather than just the subcommand: with `-C DIR` in front,
/// naming only `args[0]` would report every failure as "git -C failed".
pub(crate) fn git_capture(args: &[&str]) -> Result<String, Error> {
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
                args.join(" "),
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn git_head(repo: &Path) -> Result<String, Error> {
    git_capture(&["-C", &repo.to_string_lossy(), "rev-parse", "HEAD"])
}

/// A filesystem-safe cache subdirectory name for a registry URL: the first 8
/// bytes of its SHA-256 as hex.
fn cache_key(url: &str) -> String {
    util::sha256_hex(url.as_bytes())[..16].to_string()
}

// ---------------------------------------------------------------------------
// Git package sources (slice S3): a `Satyristes`
// `[[library]] source = { git = "…", rev = "…" }` entry, fetched directly
// from a git repo via the same `git` CLI shell-out `git_acquire` uses, then
// handed to `ops::reconcile` as a plain directory (the `{ path = … }` path).
// ---------------------------------------------------------------------------

/// An acquired git package-source checkout: a live local working tree,
/// checked out at the resolved commit.
#[derive(Debug)]
pub(crate) struct GitSource {
    /// The checkout's working-tree root — readable like a `{ path = … }`
    /// source; no archive/verify step applies to a git source.
    pub root: PathBuf,
    /// The commit sha HEAD points at after checkout: what `reconcile` pins
    /// as `Satyristes.lock`'s `rev`.
    pub resolved_rev: String,
}

/// Acquire a `{ git = url, rev }` package source: clone/reuse `url` in the
/// git-source cache ([`RegistryOptions::git_source_cache_dir`]), checked out
/// at `rev` (branch, tag, or commit-ish) when given, else the default-branch
/// tip. Mirrors `git_acquire`'s cache/refresh/offline shape.
///
/// A `rev`-pinned source does a full (non-shallow) clone so `git checkout`
/// can reach any commit, not just a shallow clone's branch tip. A `rev`-less
/// source shallow clones (`--depth 1`), matching `git_acquire`.
pub(crate) fn acquire_git_source(
    url: &str,
    rev: Option<&str>,
    opts: &RegistryOptions,
) -> Result<GitSource, Error> {
    let cache_root = opts.git_source_cache_root();
    std::fs::create_dir_all(&cache_root).map_err(|e| Error::io(&cache_root, e))?;
    let key = match rev {
        Some(r) => cache_key(&format!("{url}#{r}")),
        None => cache_key(url),
    };
    let dest = cache_root.join(key);
    let already_cloned = dest.join(".git").is_dir();

    if opts.is_offline() {
        if !already_cloned {
            return Err(Error::Offline {
                url: url.to_string(),
            });
        }
    } else if already_cloned {
        if opts.refresh {
            run_git(&["-C", &dest.to_string_lossy(), "fetch", "origin"])?;
            match rev {
                Some(r) => run_git(&["-C", &dest.to_string_lossy(), "checkout", r])?,
                None => run_git(&[
                    "-C",
                    &dest.to_string_lossy(),
                    "reset",
                    "--hard",
                    "FETCH_HEAD",
                ])?,
            }
        }
    } else {
        if dest.exists() {
            std::fs::remove_dir_all(&dest).map_err(|e| Error::io(&dest, e))?;
        }
        match rev {
            Some(r) => {
                run_git(&["clone", "-q", url, &dest.to_string_lossy()])?;
                run_git(&["-C", &dest.to_string_lossy(), "checkout", "-q", r])?;
            }
            None => {
                run_git(&["clone", "-q", "--depth", "1", url, &dest.to_string_lossy()])?;
            }
        }
    }

    let resolved_rev = git_head(&dest)?;
    Ok(GitSource {
        root: dest,
        resolved_rev,
    })
}

// ---------------------------------------------------------------------------
// Index entry format + lookup.
// ---------------------------------------------------------------------------

/// A parsed `packages/<name>.toml` index entry.
///
/// `Serialize` as well as `Deserialize` because [`crate::ops::publish`] WRITES
/// this shape: emitting it through the same struct the installer reads is what
/// makes a published entry parse back by construction rather than by a
/// hand-kept-in-step formatter.
#[derive(Debug, Clone, Default, Deserialize, serde::Serialize)]
pub(crate) struct PackageIndex {
    /// Optional package description (this port's addition; surfaced by
    /// `search`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Released versions, keyed by version string.
    #[serde(default)]
    pub versions: BTreeMap<String, VersionEntry>,
}

/// One `[versions."<v>"]` table.
#[derive(Debug, Clone, Deserialize, serde::Serialize)]
pub(crate) struct VersionEntry {
    pub tarball_url: String,
    /// Empty when the index publishes only a [`Self::sha512`] — an OPAM
    /// repository does.
    #[serde(default)]
    pub sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha512: Option<String>,
    /// `name -> constraint text` (`"^1.2.0"`, `"1.2.0"`, `"*"` —
    /// `version::Constraint::parse` syntax), feeding the solver's transitive
    /// walk. `#[serde(default)]` so an index predating this field still
    /// parses, as a leaf.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub dependencies: BTreeMap<String, String>,
}

impl Registry {
    /// `None` for a `Sparse` index (there is no on-disk index root).
    fn local_package_path(&self, name: &str) -> Option<PathBuf> {
        match &self.backend {
            RegistryBackend::Local(root) => Some(root.join("packages").join(format!("{name}.toml"))),
            RegistryBackend::Sparse { .. } => None,
        }
    }
}

/// Read and parse `packages/<name>.toml` from an acquired index:
/// **Local** reads `<root>/packages/<name>.toml` from disk (a missing file
/// is [`Error::PackageNotFound`]); **Sparse** GETs
/// `<base>/packages/<name>.toml` (mirror-rewritten via `opts.mirrors`) and
/// caches the parsed text in the `Registry` for its lifetime, so the solver
/// visiting a package more than once triggers only one GET.
pub(crate) fn lookup(reg: &Registry, name: &str) -> Result<PackageIndex, Error> {
    if let Some(path) = reg.local_package_path(name) {
        if !path.is_file() {
            // Satyrographos' own index is an OPAM repository — one directory
            // per package, holding `<name>.<version>/opam`. Read that shape
            // too, so packages SATySFi users already publish are reachable.
            if let Some(idx) = opam_index::lookup(&path, name)? {
                return Ok(idx);
            }
            // Satyrographos publishes library `xpath` as opam package
            // `satysfi-xpath`, so naming the library still finds it.
            let prefixed = path.with_file_name(format!("satysfi-{name}.toml"));
            if let Some(idx) = opam_index::lookup(&prefixed, &format!("satysfi-{name}"))? {
                return Ok(idx);
            }
            return Err(Error::PackageNotFound {
                name: name.to_string(),
            });
        }
        let text = util::read_to_string(&path)?;
        return toml::from_str(&text).map_err(|source| Error::RegistryIndex {
            message: format!("{}: {source}", path.display()),
        });
    }
    let RegistryBackend::Sparse { base, opts, cache } = &reg.backend else {
        unreachable!("local_package_path returned None only for the Sparse backend");
    };
    if let Some(text) = cache.borrow().get(name) {
        return parse_package_index(text, name);
    }
    let primary = format!("{base}/packages/{name}.toml");
    let candidates = candidate_urls(&primary, &opts.mirrors);
    let text = sparse_fetch_index_text(&candidates, opts, name)?;
    let parsed = parse_package_index(&text, name)?;
    cache.borrow_mut().insert(name.to_string(), text);
    Ok(parsed)
}

fn parse_package_index(text: &str, name: &str) -> Result<PackageIndex, Error> {
    toml::from_str(text).map_err(|source| Error::RegistryIndex {
        message: format!("packages/{name}.toml: {source}"),
    })
}

/// A 404 from every candidate is reported as [`Error::PackageNotFound`], not
/// a generic `HttpFailed`, matching a local/git index's missing-file error.
fn sparse_fetch_index_text(candidates: &[String], opts: &RegistryOptions, name: &str) -> Result<String, Error> {
    if opts.is_offline() {
        return Err(Error::Offline {
            url: candidates.first().cloned().unwrap_or_default(),
        });
    }
    match try_candidates(candidates, |url| http::get_to_string(url)) {
        Ok(text) => Ok(text),
        Err(Error::HttpFailed { message, .. }) if message.contains("404") => {
            Err(Error::PackageNotFound {
                name: name.to_string(),
            })
        }
        Err(e) => Err(e),
    }
}

/// The exact `req` version if given (else [`Error::VersionNotFound`]),
/// otherwise the highest by [`version_cmp`].
pub(crate) fn select_version<'a>(
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

/// Every version of `idx` that parses as a [`Version`], sorted descending
/// (highest first). An entry whose key does not parse as
/// `major.minor.patch[-pre]` is silently skipped rather than failing the
/// whole lookup.
pub(crate) fn available_versions(idx: &PackageIndex) -> Vec<Version> {
    let mut versions: Vec<Version> = idx.versions.keys().filter_map(|s| Version::parse(s).ok()).collect();
    versions.sort();
    versions.reverse();
    versions
}

/// Matched by re-parsing each key, since [`PackageIndex::versions`] is keyed
/// by the original version string, not a [`Version`].
pub(crate) fn entry_for<'a>(idx: &'a PackageIndex, v: &Version) -> Option<&'a VersionEntry> {
    idx.versions
        .iter()
        .find(|(k, _)| Version::parse(k).ok().as_ref() == Some(v))
        .map(|(_, entry)| entry)
}

/// A simple "semver-ish" ordering: dotted components compared numerically
/// where both parse as integers, else lexically; a shorter prefix (`1.0` vs
/// `1.0.1`) sorts lower.
fn version_cmp(a: &str, b: &str) -> std::cmp::Ordering {
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

/// Stems of `packages/*.toml`, sorted. Not supported for a `Sparse` backend:
/// no listing endpoint, only per-package GETs (matches Cargo's own
/// sparse-index `search` limitation).
pub(crate) fn all_package_names(reg: &Registry) -> Result<Vec<String>, Error> {
    let root = match &reg.backend {
        RegistryBackend::Local(root) => root,
        RegistryBackend::Sparse { base, .. } => {
            return Err(Error::RegistryIndex {
                message: format!(
                    "listing/searching a sparse HTTP index ({base}) is not supported: \
                     no whole-index listing endpoint, only per-package GETs"
                ),
            });
        }
    };
    let dir = root.join("packages");
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut names = Vec::new();
    for path in util::read_dir_paths(&dir)? {
        if path.extension().and_then(|e| e.to_str()) == Some("toml") {
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                names.push(stem.to_string());
            }
        } else if path.is_dir() && opam_index::is_package_dir(&path) {
            if let Some(n) = path.file_name().and_then(|s| s.to_str()) {
                names.push(n.to_string());
            }
        }
    }
    names.sort();
    names.dedup();
    Ok(names)
}

/// Reading an OPAM repository as a package index.
///
/// Satyrographos publishes through OPAM: `packages/<name>/<name>.<version>/opam`,
/// with the source in the opam's own `url { src: … checksum: … }` block.
/// This maps that onto [`PackageIndex`] so the rest of the installer does
/// not need to know which shape it came from.
///
/// `depends:` is deliberately NOT translated into [`VersionEntry::dependencies`]:
/// - **Names.** Opam names a dependency by its opam package id
///   (`satysfi-fonts-theano`, or non-library tooling like `ocaml`/`dune`/
///   `satysfi`) while this port's solver keys by library name
///   (`fonts-theano`); guessing that mapping would resolve the wrong thing.
/// - **Constraints.** Opam's constraint grammar (ranges, `&`/`|`, non-version
///   filters) has no faithful translation into this crate's [`Constraint`];
///   see [`crate::opam::Dependency`]'s doc.
///
/// [`crate::opam`] parses and records `depends:` for a package's own
/// `.opam` (used by [`crate::ops::prepare`]), so the data is not lost, just
/// not wired into the solver here.
pub(crate) mod opam_index {
    use std::collections::BTreeMap;
    use std::path::Path;

    use super::{PackageIndex, VersionEntry};
    use crate::error::Error;
    use crate::util;

    /// Holds at least one `<name>.<version>/opam`.
    pub(crate) fn is_package_dir(dir: &Path) -> bool {
        util::read_dir_paths(dir)
            .map(|paths| paths.iter().any(|p| p.join("opam").is_file()))
            .unwrap_or(false)
    }

    /// Build an index for `name` from `<packages>/<name>/`. `Ok(None)` when
    /// that directory is not an OPAM package directory.
    pub(crate) fn lookup(toml_path: &Path, name: &str) -> Result<Option<PackageIndex>, Error> {
        // `packages/<name>.toml`'s sibling directory is `packages/<name>`.
        let dir = toml_path.with_extension("");
        if !dir.is_dir() || !is_package_dir(&dir) {
            return Ok(None);
        }

        let mut versions = BTreeMap::new();
        let mut description = None;
        for version_dir in util::read_dir_paths(&dir)? {
            let opam_file = version_dir.join("opam");
            if !opam_file.is_file() {
                continue;
            }
            let Some(dir_name) = version_dir.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            // `<name>.<version>`, e.g. `2.0+satysfi0.0.3+satyrographos0.0.2`.
            let version = dir_name
                .strip_prefix(&format!("{name}."))
                .unwrap_or(dir_name)
                .to_string();

            let text = util::read_to_string(&opam_file)?;
            if description.is_none() {
                description = field(&text, "synopsis:");
            }
            let Some((tarball_url, sha256, sha512)) = source_of(&text) else {
                continue;
            };
            versions.insert(
                version,
                VersionEntry {
                    tarball_url,
                    sha256: sha256.unwrap_or_default(),
                    sha512,
                    dependencies: BTreeMap::new(),
                },
            );
        }
        if versions.is_empty() {
            return Ok(None);
        }
        Ok(Some(PackageIndex {
            description,
            versions,
        }))
    }

    /// The `url { src: "…" checksum: [ "sha256=…" ] }` block's archive and digests.
    fn source_of(text: &str) -> Option<(String, Option<String>, Option<String>)> {
        let at = text.find("url {").or_else(|| text.find("url{"))?;
        let rest = &text[at..];
        let end = rest.find('}')? ;
        let block = &rest[..end];
        let url = field_string(block, "src:").or_else(|| field_string(block, "archive:"))?;
        let digest = |prefix: &str| {
            block
                .split('"')
                .find(|s| s.starts_with(prefix))
                .map(|s| s.trim_start_matches(prefix).to_string())
        };
        // md5 is ignored; Satyrographos' index actually publishes sha512.
        Some((url, digest("sha256="), digest("sha512=")))
    }

    fn field(text: &str, field: &str) -> Option<String> {
        text.lines()
            .find(|l| l.starts_with(field))
            .and_then(|l| field_string(l, field))
    }

    fn field_string(text: &str, field: &str) -> Option<String> {
        let at = text.find(field)? + field.len();
        let rest = &text[at..];
        let start = rest.find('"')? + 1;
        let end = rest[start..].find('"')? + start;
        Some(rest[start..end].to_string())
    }
}

// ---------------------------------------------------------------------------
// Several repositories at once: `update`/reconcile consult every configured
// registry, in order — the same coverage `search` and `install NAME` have.
// ---------------------------------------------------------------------------

/// One repository, acquired, paired with the URL it was acquired from (for
/// labeling/error messages) — `acquire_all`'s success list.
pub struct AcquiredRepo {
    pub url: String,
    pub registry: Registry,
}

/// Acquire every configured repository in `repos`, in order: one
/// unreachable repository must not hide the others.
///
/// When `reg_opts.url` already pins one explicit registry (`--registry` /
/// `$RUSTYFI_REGISTRY`, resolved through `single_fallback`) or `repos` is
/// empty, this acquires exactly that one registry.
///
/// Returns the registries successfully acquired (each labeled with its URL)
/// alongside every failure (`(url, error)`, empty when all acquired
/// cleanly). [`Error::NoRegistry`] (or the first failure) only when nothing
/// could be acquired at all.
pub(crate) fn acquire_all(
    repos: &[crate::source::RegistryConfig],
    reg_opts: &RegistryOptions,
    single_fallback: Option<&str>,
) -> Result<(Vec<AcquiredRepo>, Vec<(String, Error)>), Error> {
    if repos.is_empty() || reg_opts.url.is_some() {
        let url = reg_opts.resolve_url(single_fallback)?;
        let reg = acquire(&url, reg_opts)?;
        return Ok((vec![AcquiredRepo { url, registry: reg }], Vec::new()));
    }

    let mut ok = Vec::new();
    let mut failed = Vec::new();
    for repo in repos {
        let Some(url) = repo.url.clone() else { continue };
        // Each repository may declare its own mirrors/kind; only
        // `reg_opts`' cache/offline/token settings carry over.
        let per_repo_opts = RegistryOptions {
            url: None,
            mirrors: reg_opts.resolve_mirrors(&repo.mirrors),
            kind: reg_opts.kind.or(repo.kind),
            ..reg_opts.clone()
        };
        match acquire(&url, &per_repo_opts) {
            Ok(reg) => ok.push(AcquiredRepo { url, registry: reg }),
            Err(e) => failed.push((url, e)),
        }
    }
    if ok.is_empty() {
        return Err(failed
            .into_iter()
            .next()
            .map(|(_, e)| e)
            .unwrap_or(Error::NoRegistry));
    }
    Ok((ok, failed))
}

/// Adapts several already-[`acquire`]d registries to the solver's
/// [`DepSource`] trait at once: each lookup tries every registry in order
/// and uses the first that has the package (matching `install NAME`'s
/// first-repository-wins rule, `crates/rustyfi/src/main.rs`'s `install_one`).
pub(crate) struct MultiRegistryDepSource<'a> {
    regs: &'a [AcquiredRepo],
}

impl<'a> MultiRegistryDepSource<'a> {
    pub(crate) fn new(regs: &'a [AcquiredRepo]) -> Self {
        MultiRegistryDepSource { regs }
    }

    /// The first registry (in order) whose index has `name`, and which one
    /// that was.
    pub(crate) fn lookup_first(&self, name: &str) -> Result<(&'a AcquiredRepo, PackageIndex), Error> {
        let mut last_err: Option<Error> = None;
        for repo in self.regs {
            match lookup(&repo.registry, name) {
                Ok(idx) => return Ok((repo, idx)),
                Err(e) => last_err = Some(e),
            }
        }
        Err(last_err.unwrap_or_else(|| Error::PackageNotFound {
            name: name.to_string(),
        }))
    }
}

impl<'a> DepSource for MultiRegistryDepSource<'a> {
    fn versions(&self, name: &str) -> Result<Vec<Version>, Error> {
        let (_, idx) = self.lookup_first(name)?;
        Ok(available_versions(&idx))
    }

    fn deps(&self, name: &str, v: &Version) -> Result<Vec<(String, Constraint)>, Error> {
        let (_, idx) = self.lookup_first(name)?;
        let entry = entry_for(&idx, v).ok_or_else(|| Error::VersionNotFound {
            name: name.to_string(),
            version: v.to_string(),
        })?;
        entry
            .dependencies
            .iter()
            .map(|(dep_name, req)| Ok((dep_name.clone(), Constraint::parse(req)?)))
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Tarball fetch + verify.
// ---------------------------------------------------------------------------

/// Fetch `tarball_url` into the file at `dest`, preferring the
/// content-addressed archive cache for any network URL.
///
/// - `file://` / plain local paths are copied directly (never cached — a
///   local path is already as cheap as the cache).
/// - `http(s)://` is resolved through `cache::get_or_fetch`: a
///   cache hit re-verifying against `sha256` costs zero network; a miss
///   fetches over HTTP, or fails with [`Error::Offline`] under
///   `opts.is_offline()`. Without the `http` cargo feature, any fetch
///   attempt is [`Error::HttpDisabled`].
pub(crate) fn fetch_tarball(
    url: &str,
    checksum: &Checksum,
    dest: &Path,
    opts: &RegistryOptions,
) -> Result<(), Error> {
    if let Some(local) = local_path_from_url(url) {
        std::fs::copy(&local, dest).map_err(|e| Error::io(&local, e))?;
        return Ok(());
    }
    let candidates = candidate_urls(url, &opts.mirrors);
    crate::cache::get_or_fetch(&candidates, checksum, dest, opts)
}

/// [`crate::cache::get_or_fetch`]'s sole seam into the feature-gated `http`
/// module (kept private to this file otherwise).
pub(crate) fn raw_http_fetch(url: &str, dest: &Path) -> Result<(), Error> {
    http::get_to_file(url, dest)
}

// ---------------------------------------------------------------------------
// Mirror fallback: a registry may declare mirror base URLs; a
// failed fetch from the primary URL falls through to each mirror in order.
// ---------------------------------------------------------------------------

/// Primary URL first, then each of `mirrors` rewritten against it
/// ([`rewrite_to_mirror`]). Shared by [`fetch_tarball`] and `lookup`'s
/// sparse per-package GET.
pub(crate) fn candidate_urls(primary: &str, mirrors: &[String]) -> Vec<String> {
    let mut urls = Vec::with_capacity(1 + mirrors.len());
    urls.push(primary.to_string());
    urls.extend(mirrors.iter().map(|m| rewrite_to_mirror(primary, m)));
    urls
}

/// A host/prefix substitution: re-root the primary URL's path (and query)
/// under the mirror base. E.g. primary
/// `https://packages.example.org/dist/foo-1.2.0.tar.gz` + mirror
/// `https://mirror-eu.example.org` →
/// `https://mirror-eu.example.org/dist/foo-1.2.0.tar.gz`.
pub(crate) fn rewrite_to_mirror(primary: &str, mirror_base: &str) -> String {
    let path = primary
        .find("://")
        .map(|idx| &primary[idx + 3..])
        .and_then(|after_scheme| after_scheme.find('/').map(|slash| &after_scheme[slash..]))
        .unwrap_or("");
    format!("{}{path}", mirror_base.trim_end_matches('/'))
}

/// Returns the first `Ok`, or (if every candidate fails) the last error
/// encountered. Shared by [`crate::cache::get_or_fetch`]'s tarball-fetch
/// loop and the sparse index's per-package GET.
pub(crate) fn try_candidates<T>(
    urls: &[String],
    mut f: impl FnMut(&str) -> Result<T, Error>,
) -> Result<T, Error> {
    let mut last_err: Option<Error> = None;
    for url in urls {
        match f(url) {
            Ok(v) => return Ok(v),
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.unwrap_or_else(|| Error::HttpFailed {
        url: urls.first().cloned().unwrap_or_default(),
        message: "no candidate URLs".to_string(),
    }))
}

/// What an index entry says a download must hash to.
///
/// An OPAM repository publishes md5 and sha512; this port's own index
/// publishes sha256. Carrying both through the fetch keeps the cache key and
/// the verification speaking the same algorithm — keying by an empty sha256
/// while verifying a sha512 would collide every unverified download onto
/// one cache entry.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Checksum {
    pub sha256: String,
    pub sha512: Option<String>,
}

impl Checksum {
    pub fn new(sha256: &str, sha512: Option<&str>) -> Self {
        Checksum {
            sha256: sha256.to_string(),
            sha512: sha512.map(str::to_string),
        }
    }

    /// The digest to key a cache entry by: sha256 when declared, else sha512.
    pub fn key(&self) -> &str {
        if !self.sha256.trim().is_empty() {
            self.sha256.trim()
        } else {
            self.sha512.as_deref().map(str::trim).unwrap_or("")
        }
    }

    pub fn verify(&self, path: &Path) -> Result<String, Error> {
        verify_entry(path, &self.sha256, self.sha512.as_deref())
    }
}

/// Verify `path` against whichever digest the index declared: sha256 when it
/// has one, else sha512. A version with neither is refused.
pub(crate) fn verify_entry(path: &Path, sha256: &str, sha512: Option<&str>) -> Result<String, Error> {
    if !sha256.trim().is_empty() {
        return verify_sha256(path, sha256);
    }
    let Some(expected) = sha512.map(str::trim).filter(|s| !s.is_empty()) else {
        return Err(Error::ChecksumMismatch {
            expected: "a sha256 or sha512 in the index entry".to_string(),
            actual: "no checksum declared".to_string(),
        });
    };
    let actual = util::sha512_file(path)?;
    if !actual.eq_ignore_ascii_case(expected) {
        return Err(Error::ChecksumMismatch {
            expected: expected.to_lowercase(),
            actual,
        });
    }
    Ok(actual)
}

/// [`Error::ChecksumMismatch`] on mismatch (case-insensitive hex compare) —
/// the caller aborts before touching `dist/`.
pub(crate) fn verify_sha256(path: &Path, expected: &str) -> Result<String, Error> {
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
pub(crate) fn local_path_from_url(url: &str) -> Option<PathBuf> {
    if let Some(rest) = url.strip_prefix("file://") {
        // Authority must be empty (`file:///abs/path`) or `localhost`; any
        // other host falls through to the git-clone path instead of
        // silently reading a bogus directory.
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
// Feature-gated HTTP transport.
// ---------------------------------------------------------------------------

pub(crate) mod http {
    use super::*;

    #[cfg(feature = "http")]
    const DEFAULT_TIMEOUT_SECS: u64 = 30;

    #[cfg(feature = "http")]
    const MAX_REDIRECTS: u32 = 5;

    /// Max response-body size: a hostile/misconfigured server must not be
    /// able to exhaust disk/memory on a fetch.
    #[cfg(feature = "http")]
    const MAX_BODY_BYTES: u64 = 512 * 1024 * 1024; // 512 MiB

    /// Bearer-token auth (slice S3), read fresh per request and never
    /// included in any `Error` message/log (only the URL and status/transport
    /// text are). See [`super::REGISTRY_TOKEN_ENV`].
    #[cfg(feature = "http")]
    fn auth_token() -> Option<String> {
        std::env::var(super::REGISTRY_TOKEN_ENV)
            .ok()
            .filter(|s| !s.is_empty())
    }

    #[cfg(feature = "http")]
    fn timeout() -> std::time::Duration {
        std::env::var(super::HTTP_TIMEOUT_ENV)
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .map(std::time::Duration::from_secs)
            .unwrap_or(std::time::Duration::from_secs(DEFAULT_TIMEOUT_SECS))
    }

    /// Explicit connect+read timeout and bounded redirect count, so a
    /// stalled or endlessly-redirecting server fails within a bounded
    /// budget instead of hanging the install.
    #[cfg(feature = "http")]
    fn agent() -> ureq::Agent {
        let t = timeout();
        ureq::AgentBuilder::new()
            .timeout_connect(t)
            .timeout_read(t)
            .redirects(MAX_REDIRECTS)
            .build()
    }

    #[cfg(feature = "http")]
    fn to_error(url: &str, e: ureq::Error) -> Error {
        match e {
            ureq::Error::Status(status, response) => Error::HttpFailed {
                url: url.to_string(),
                message: format!("http status {status} ({})", response.status_text()),
            },
            ureq::Error::Transport(transport) => Error::HttpFailed {
                url: url.to_string(),
                message: transport.to_string(),
            },
        }
    }

    #[cfg(feature = "http")]
    pub fn get_to_file(url: &str, dest: &Path) -> Result<(), Error> {
        let mut req = agent().get(url);
        if let Some(token) = auth_token() {
            req = req.set("Authorization", &format!("Bearer {token}"));
        }
        let resp = req.call().map_err(|e| to_error(url, e))?;
        let mut file = std::fs::File::create(dest).map_err(|e| Error::io(dest, e))?;
        // Cap the read at one byte past the limit so an oversized body is a
        // clean typed error rather than a silently truncated (and therefore
        // checksum-mismatching) file.
        let mut limited = std::io::Read::take(resp.into_reader(), MAX_BODY_BYTES + 1);
        let copied = std::io::copy(&mut limited, &mut file).map_err(|e| Error::HttpFailed {
            url: url.to_string(),
            message: format!("reading response body: {e}"),
        })?;
        if copied > MAX_BODY_BYTES {
            drop(file);
            let _ = std::fs::remove_file(dest);
            return Err(Error::HttpFailed {
                url: url.to_string(),
                message: format!("response body exceeds the {MAX_BODY_BYTES}-byte size cap"),
            });
        }
        Ok(())
    }

    #[cfg(not(feature = "http"))]
    pub fn get_to_file(url: &str, _dest: &Path) -> Result<(), Error> {
        Err(Error::HttpDisabled {
            url: url.to_string(),
        })
    }

    /// `get_to_file`'s sibling for the sparse index: returns the body as a
    /// `String` rather than writing it to disk — a `packages/<name>.toml`
    /// has no sha256 to verify against. Reuses [`agent`], bearer auth,
    /// timeout, redirect bound, and body-size cap.
    #[cfg(feature = "http")]
    pub fn get_to_string(url: &str) -> Result<String, Error> {
        let mut req = agent().get(url);
        if let Some(token) = auth_token() {
            req = req.set("Authorization", &format!("Bearer {token}"));
        }
        let resp = req.call().map_err(|e| to_error(url, e))?;
        let mut buf = Vec::new();
        let mut limited = std::io::Read::take(resp.into_reader(), MAX_BODY_BYTES + 1);
        let copied = std::io::copy(&mut limited, &mut buf).map_err(|e| Error::HttpFailed {
            url: url.to_string(),
            message: format!("reading response body: {e}"),
        })?;
        if copied > MAX_BODY_BYTES {
            return Err(Error::HttpFailed {
                url: url.to_string(),
                message: format!("response body exceeds the {MAX_BODY_BYTES}-byte size cap"),
            });
        }
        String::from_utf8(buf).map_err(|e| Error::HttpFailed {
            url: url.to_string(),
            message: format!("response body is not valid UTF-8: {e}"),
        })
    }

    #[cfg(not(feature = "http"))]
    pub fn get_to_string(url: &str) -> Result<String, Error> {
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
        assert_eq!(local_path_from_url("https://example.com/x.tar.gz"), None);
        assert_eq!(local_path_from_url("git://host/repo.git"), None);
    }
}
