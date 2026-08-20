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
//!   through [`crate::cache`]'s content-addressed archive cache first (phase
//!   7d S2 — a cache hit is zero-network), falling back to the feature-gated
//!   [`http`] client on a miss (plan §8: path/archive/local installs stay
//!   offline with the `http` feature off; `RegistryOptions::is_offline`
//!   forbids the network fallback entirely).
//! - **Verify** the tarball's SHA-256 against the index entry ([`verify_sha256`])
//!   — the caller does this *before* touching `dist/` (plan §5.4 step 3).
//! - **Acquire a git package source** ([`acquire_git_source`], saphe 7d slice
//!   S3): a `Satyristes` `{ git = …, rev = … }` entry — distinct from a
//!   registry index — clones the repo (pinned to `rev` when given) into its
//!   own cache leaf; the checkout is then handed to `ops::reconcile` as a
//!   plain directory, the same materialisation path a `{ path = … }` source
//!   takes (no archive/verify step — there is no tarball/sha256 for a git
//!   source, only the resolved commit).
//! - Optional **bearer-token auth** for the HTTP tarball transport
//!   (`RUSTYFI_REGISTRY_TOKEN`, `http::AUTH_TOKEN_ENV`, saphe 7d slice S3).
//!
//! The fetch → verify → materialize orchestration lives in
//! [`crate::ops::install`] (registry form) and [`crate::ops::reconcile`]
//! (manifest and git-source form); this module is the index/transport layer
//! only.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::error::Error;
use crate::solve::DepSource;
use crate::util;
use crate::version::{Constraint, Version};

/// The environment variable consulted for the registry URL when no
/// `--registry` flag and no `Satyristes` `[registry]` url is given
/// (plan §5.4 step 1).
pub const REGISTRY_ENV: &str = "RUSTYFI_REGISTRY";

/// Environment override for the git-index cache directory (used by tests to
/// stay hermetic; production defaults to `$XDG_CACHE_HOME/rustyfi/
/// registry/`).
pub const CACHE_ENV: &str = "RUSTYFI_REGISTRY_CACHE";

/// Environment override for offline mode (saphe phase 7d slice S2, design
/// §2.5): `RUSTYFI_OFFLINE=1` behaves like `--offline` for every registry
/// operation, so CI/scripted reconciles can force it without a flag.
pub const OFFLINE_ENV: &str = "RUSTYFI_OFFLINE";

/// Environment override for the git package-source clone cache directory
/// (saphe 7d slice S3, design §3 S3): a sibling of the git-index cache
/// ([`CACHE_ENV`]) and the archive cache ([`crate::cache::ARCHIVE_CACHE_ENV`])
/// under its own leaf, so a `{ git = … }` `Satyristes` package source
/// never collides with an index clone or a tarball blob. Production default is
/// `$XDG_CACHE_HOME/rustyfi/git-sources/`.
pub const GIT_SOURCE_CACHE_ENV: &str = "RUSTYFI_GIT_SOURCE_CACHE";

/// How to reach and cache a registry index (plan §5.4 step 1).
#[derive(Debug, Default, Clone)]
pub struct RegistryOptions {
    /// The `--registry URL` flag, if given (highest precedence).
    pub url: Option<String>,
    /// The git-index cache directory; falls back to `$RUSTYFI_REGISTRY_CACHE`
    /// then `$XDG_CACHE_HOME/rustyfi/registry/`.
    pub cache_dir: Option<PathBuf>,
    /// Re-fetch a git index even if it is already cloned in the cache
    /// (`satyrographos update` sets this; plain `install` reuses the cache).
    pub refresh: bool,
    /// The content-addressed archive cache directory (phase 7d S2, design
    /// §2.5/§3); falls back to `$RUSTYFI_ARCHIVE_CACHE` then
    /// `$XDG_CACHE_HOME/rustyfi/archives/` (see [`crate::cache`]).
    pub archive_cache_dir: Option<PathBuf>,
    /// The git package-source clone cache directory (phase 7d S3, design §3
    /// S3, [`acquire_git_source`]); falls back to `$RUSTYFI_GIT_SOURCE_CACHE`
    /// then `$XDG_CACHE_HOME/rustyfi/git-sources/`.
    pub git_source_cache_dir: Option<PathBuf>,
    /// Forbid network requests (`--offline`); see [`RegistryOptions::is_offline`].
    pub offline: bool,
    /// Fallback registry base URLs (mirrors design §2.1), tried in order
    /// after the primary URL when a fetch fails: a tarball fetch
    /// ([`fetch_tarball`]) or, once a registry's `kind` is `Sparse`, a
    /// per-package index GET. Each candidate is a **host/prefix
    /// substitution** applied to the primary URL's path+query (see
    /// [`rewrite_to_mirror`]), verified against the *same* sha256 for a
    /// tarball. Empty by default — an explicit flag/env value (were one
    /// added) would take precedence over a `Satyristes` `[registry]
    /// mirrors` fallback; see [`RegistryOptions::resolve_mirrors`].
    pub mirrors: Vec<String>,
    /// The index transport (mirrors/sparse design §3.2). `None` = today's
    /// local-dir/git dispatch by URL shape ([`satyrfile::RegistryKind::Auto`]);
    /// only `Some(Sparse)` selects the per-package HTTP index path.
    pub kind: Option<crate::source::RegistryKind>,
}

impl RegistryOptions {
    /// Whether network access is forbidden for this operation (phase 7d S2,
    /// design §2.5): either the `offline` field (set by `--offline`) or
    /// `$RUSTYFI_OFFLINE=1`. When true, both the registry-index acquisition
    /// ([`acquire`]) and the archive fetch ([`fetch_tarball`] /
    /// [`crate::cache::get_or_fetch`]) must resolve entirely from what is
    /// already cached on disk, returning [`Error::Offline`] on any miss
    /// rather than silently reaching the network.
    pub fn is_offline(&self) -> bool {
        self.offline
            || std::env::var_os(OFFLINE_ENV)
                .map(|v| v == "1")
                .unwrap_or(false)
    }

    /// Resolve the registry URL, preferring the explicit `--registry` flag,
    /// then `$RUSTYFI_REGISTRY`, then the `fallback` (a `Satyristes`
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

    /// Resolve the mirror candidate list (mirrors design §2.1), preferring an
    /// explicit `self.mirrors` (e.g. a future `--registry-mirror` flag) over
    /// `fallback` (a `Satyristes` `[registry] mirrors`) — the same
    /// explicit-wins-over-manifest precedence [`resolve_url`] uses for the
    /// registry URL itself. `fallback` is used verbatim (not merged) when
    /// `self.mirrors` is empty.
    pub fn resolve_mirrors(&self, fallback: &[String]) -> Vec<String> {
        if !self.mirrors.is_empty() {
            self.mirrors.clone()
        } else {
            fallback.to_vec()
        }
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
        util::xdg_cache_base().join("rustyfi").join("registry")
    }

    /// The cache directory for git package-source clones (phase 7d S3).
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
/// `packages/` (a git clone or a plain directory), or — Slice S — a sparse
/// HTTP index fetched one `packages/<name>.toml` at a time. `commit` is the
/// resolved git commit sha for a git index (`None` for a plain directory or a
/// sparse index — neither has a single-snapshot commit to report).
#[derive(Debug)]
pub struct Registry {
    backend: RegistryBackend,
    pub commit: Option<String>,
}

/// The concrete transport behind an acquired [`Registry`] (design §3.2).
#[derive(Debug)]
enum RegistryBackend {
    /// A local directory holding `packages/<name>.toml` — a git clone or a
    /// hand-made/checked-out plain directory. Byte-identical to the
    /// pre-Slice-S `Registry { root, .. }` shape.
    Local(PathBuf),
    /// A sparse HTTP index (design §3): `packages/<name>.toml` is GET-ed
    /// lazily from `<base>/packages/<name>.toml` (mirror-rewritten via
    /// `opts.mirrors`), the *first* time each package name is looked up, and
    /// cached in `cache` for the remainder of this `Registry`'s lifetime (one
    /// `acquire` call = one solve/lookup pass) — design §3.3's "minimal first
    /// cut": a within-process cache, not a persistent on-disk one.
    Sparse {
        base: String,
        opts: RegistryOptions,
        cache: std::cell::RefCell<BTreeMap<String, String>>,
    },
}

/// Acquire the registry index named by `url` into a queryable index handle
/// (plan §5.4 step 1; mirrors/sparse design §3.2).
///
/// `opts.kind` selects the transport:
/// - `None`/`Auto` (default, unchanged from before Slice S): a **plain-directory
///   index** — a local path (or `file://` URL) pointing at a directory that
///   already contains `packages/` — is used in place, no git; anything else is
///   treated as a git remote (or bare local repo) and shallow-cloned/fetched.
/// - `Git`: always the git path, even if `url` happens to resolve to a local
///   directory that already has `packages/` (skips `Auto`'s short-circuit).
/// - `Sparse`: `url` is a sparse-index base — no clone at all; `lookup` fetches
///   `packages/<name>.toml` over HTTP on demand (§3.3).
pub fn acquire(url: &str, opts: &RegistryOptions) -> Result<Registry, Error> {
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
            // Plain-directory index: a local path already holding `packages/`.
            if let Some(local) = local_path_from_url(url) {
                if local.join("packages").is_dir() {
                    return Ok(Registry {
                        backend: RegistryBackend::Local(local),
                        commit: None,
                    });
                }
            }
            // Otherwise treat `url` as a git remote (or bare local repo) and
            // clone/fetch.
            git_acquire(url, opts)
        }
    }
}

fn git_acquire(url: &str, opts: &RegistryOptions) -> Result<Registry, Error> {
    let cache_root = opts.cache_root();
    std::fs::create_dir_all(&cache_root).map_err(|e| Error::io(&cache_root, e))?;
    let dest = cache_root.join(cache_key(url));
    let already_cloned = dest.join(".git").is_dir();

    // Offline (phase 7d S2, design §2.5): never shell out to `git` for
    // anything that would touch the network. An already-cloned index is
    // reused as-is (a stale `refresh` request is silently skipped rather than
    // erroring — the caller asked to stay offline, which takes precedence);
    // a never-cloned index has nothing to reuse, so it is a clean
    // [`Error::Offline`] rather than a `git clone` that hangs/fails against a
    // real network with a confusing [`Error::GitFailed`].
    if opts.is_offline() {
        if !already_cloned {
            return Err(Error::Offline {
                url: url.to_string(),
            });
        }
    } else if already_cloned {
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
        backend: RegistryBackend::Local(dest),
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
// Git package sources (saphe 7d slice S3, design §3 S3): a `Satyristes`
// `[[library]] source = { git = "…", rev = "…" }` entry — a package fetched
// directly from a git repo, as opposed to a `{ registry = … }` entry (a
// package looked up in a registry index, which may itself be git-hosted;
// see `git_acquire` above). This was rejected as `Error::UnsupportedSource`
// through phase 7d S2; S3 lifts that by cloning the repo through the same
// `git` CLI shell-out `git_acquire` already uses, then handing the checkout
// to `ops::reconcile` as a plain directory — the exact same materialisation
// path a `{ path = … }` source takes.
// ---------------------------------------------------------------------------

/// An acquired git package-source checkout (design §3 S3): a live local
/// working tree, checked out at the resolved commit.
#[derive(Debug)]
pub struct GitSource {
    /// The checkout's working-tree root — readable exactly like a `{ path =
    /// … }` source (`archive::prepare`/`ops::install::install_inner` treat it
    /// as a plain directory; no archive/verify step applies to a git source).
    pub root: PathBuf,
    /// The commit sha HEAD points at after checkout: what `reconcile` pins as
    /// `Satyristes.lock`'s `rev`, so a later reconcile is reproducible without
    /// re-resolving a moving branch/tag name (design §3 S3: "record the git
    /// url + resolved rev").
    pub resolved_rev: String,
}

/// Acquire a `{ git = url, rev }` package source (design §3 S3): clone/reuse
/// `url` in the git-source cache ([`RegistryOptions::git_source_cache_dir`]),
/// checked out at `rev` (a branch, tag, or commit-ish — `git checkout`
/// resolves any of them) when given, else the remote's default-branch tip.
///
/// Mirrors [`git_acquire`]'s cache/refresh/offline shape exactly:
/// - an already-cloned checkout is reused as-is unless `opts.refresh`;
/// - [`RegistryOptions::is_offline`] forbids any `git` invocation that would
///   touch the network — an offline request against an unseen `(url, rev)`
///   pair is a clean [`Error::Offline`] rather than a hanging/failing clone.
///
/// A `rev`-pinned source does a full (non-shallow) clone so `git checkout`
/// can reach any commit, not just the branch tip a shallow clone would carry
/// (a heavier but robust default; a shallow-clone-of-one-commit optimisation
/// is a deferred edge case — see the design doc). A `rev`-less source shallow
/// clones (`--depth 1`) the default branch, matching [`git_acquire`].
pub fn acquire_git_source(
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
        // A stale non-git dir would make clone fail; clear it first.
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
    /// The sha256 an index entry declares. Empty when the index publishes only
    /// a [`Self::sha512`] — an OPAM repository does.
    #[serde(default)]
    pub sha256: String,
    /// A sha512, when that is what the index declares.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha512: Option<String>,
    /// This version's own declared dependencies (`name -> constraint text`,
    /// e.g. `"^1.2.0"`, `"1.2.0"`, or `"*"` — `version::Constraint::parse`
    /// syntax), feeding the phase-7c solver's transitive walk
    /// (`solve::RegistryDepSource`). `#[serde(default)]` so an index written
    /// before this field existed still parses — such a package is simply
    /// treated as a leaf (no declared dependencies).
    #[serde(default)]
    pub dependencies: BTreeMap<String, String>,
}

impl Registry {
    /// The on-disk path of `packages/<name>.toml`, for a [`Local`](RegistryBackend::Local)
    /// backend — `None` for a `Sparse` index (there is no on-disk index root).
    fn local_package_path(&self, name: &str) -> Option<PathBuf> {
        match &self.backend {
            RegistryBackend::Local(root) => Some(root.join("packages").join(format!("{name}.toml"))),
            RegistryBackend::Sparse { .. } => None,
        }
    }
}

/// Read and parse `packages/<name>.toml` from an acquired index (backend-aware,
/// design §3.3):
///
/// - **Local**: read `<root>/packages/<name>.toml` from disk. A missing file
///   is [`Error::PackageNotFound`] (unchanged from before Slice S).
/// - **Sparse**: GET `<base>/packages/<name>.toml` (mirror-rewritten via
///   `opts.mirrors`, §2.1/§3.3), through the same configured `ureq` agent +
///   bearer auth as a tarball fetch. The parsed text is cached in the
///   `Registry` for the remainder of its lifetime, so a package the solver
///   visits more than once during one solve triggers only one GET
///   (design §3.4: zero solver change — `RegistryDepSource` calls `lookup`
///   exactly as before).
pub fn lookup(reg: &Registry, name: &str) -> Result<PackageIndex, Error> {
    if let Some(path) = reg.local_package_path(name) {
        if !path.is_file() {
            // Satyrographos' own index is an OPAM repository — one DIRECTORY
            // per package, holding `<name>.<version>/opam` — rather than this
            // port's flat `packages/<name>.toml`. Read that shape too, so the
            // packages SATySFi users already publish are reachable.
            if let Some(idx) = opam_index::lookup(&path, name)? {
                return Ok(idx);
            }
            // Satyrographos publishes library `xpath` as opam package
            // `satysfi-xpath`, so a user naming the library still finds it.
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

/// Fetch a sparse index file's raw text, trying `candidates` in order
/// (mirrors design §2.2's fallback loop, reused here per §3.3). A 404 from
/// every candidate (the package genuinely does not exist in the index) is
/// reported as [`Error::PackageNotFound`], not a generic `HttpFailed`, so a
/// sparse lookup surfaces the same error shape a local/git index's missing
/// file does.
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

/// Every version of `idx` that parses as a [`Version`] (design §3.2),
/// **sorted descending** (highest first) — the order the solver's
/// highest-version-first candidate search wants. An index entry whose key
/// does not parse as `major.minor.patch[-pre]` is silently skipped rather
/// than failing the whole lookup (mirrors `#[serde(default)]` on
/// [`VersionEntry::dependencies`]'s back-compat stance: a malformed one entry
/// should not break every other version).
pub fn available_versions(idx: &PackageIndex) -> Vec<Version> {
    let mut versions: Vec<Version> = idx.versions.keys().filter_map(|s| Version::parse(s).ok()).collect();
    versions.sort();
    versions.reverse();
    versions
}

/// The index entry for the exact version `v` (matched by re-parsing each key,
/// since [`PackageIndex::versions`] is keyed by the original version string,
/// not a [`Version`]).
pub fn entry_for<'a>(idx: &'a PackageIndex, v: &Version) -> Option<&'a VersionEntry> {
    idx.versions
        .iter()
        .find(|(k, _)| Version::parse(k).ok().as_ref() == Some(v))
        .map(|(_, entry)| entry)
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
/// used by `search`/`update` to enumerate the whole index. Not supported for a
/// `Sparse` backend (design §3.6): there is no listing endpoint, only
/// per-package GETs, matching Cargo's own sparse-index `search` limitation.
pub fn all_package_names(reg: &Registry) -> Result<Vec<String>, Error> {
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
/// Satyrographos publishes through OPAM, so its index is
/// `packages/<name>/<name>.<version>/opam` — a directory per package, a
/// directory per version, and the package's source in the opam's own `url {
/// src: … checksum: … }` block. This maps that onto [`PackageIndex`] so the
/// rest of the installer does not need to know which shape it came from.
///
/// Dependencies are deliberately NOT translated: opam names them as opam
/// packages (`satysfi-fonts-theano`) while this port keys everything by
/// library name (`fonts-theano`), and guessing that mapping would resolve the
/// wrong package. Such an entry is treated as a leaf.
pub(crate) mod opam_index {
    use std::collections::BTreeMap;
    use std::path::Path;

    use super::{PackageIndex, VersionEntry};
    use crate::error::Error;
    use crate::util;

    /// Whether `dir` looks like an OPAM package directory: it holds at least
    /// one `<name>.<version>/opam`.
    pub(crate) fn is_package_dir(dir: &Path) -> bool {
        util::read_dir_paths(dir)
            .map(|paths| paths.iter().any(|p| p.join("opam").is_file()))
            .unwrap_or(false)
    }

    /// Build an index for `name` from `<packages>/<name>/`. `Ok(None)` when
    /// that directory is not an OPAM package directory.
    pub(crate) fn lookup(toml_path: &Path, name: &str) -> Result<Option<PackageIndex>, Error> {
        // `local_package_path` gave us `packages/<name>.toml`; the directory
        // beside it is `packages/<name>`.
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
            // `<name>.<version>` — everything after the first dot is the
            // version, which for these packages looks like
            // `2.0+satysfi0.0.3+satyrographos0.0.2`.
            let version = dir_name
                .strip_prefix(&format!("{name}."))
                .unwrap_or(dir_name)
                .to_string();

            let text = util::read_to_string(&opam_file)?;
            if description.is_none() {
                description = field(&text, "synopsis:");
            }
            let Some((tarball_url, sha256, sha512)) = source_of(&text) else {
                // A version with no fetchable source is not installable; skip
                // it rather than offering something that cannot be resolved.
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

    /// The `url { src: "…" checksum: [ "sha256=…" ] }` block's archive and
    /// sha256. A version whose checksum this crate cannot verify is not
    /// offered: an unverifiable download that looks verified is worse than an
    /// absent one.
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
        // sha256 if the entry has one; otherwise sha512, which is what
        // Satyrographos' index actually publishes. md5 is ignored.
        Some((url, digest("sha256="), digest("sha512=")))
    }

    /// A `field: "value"` string at the top level of an opam file.
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

/// Adapts a live, already-[`acquire`]d [`Registry`] index to the solver's
/// [`DepSource`] trait (design §4.1/§5.1): each call does a fresh
/// `packages/<name>.toml` lookup, so the solver can recurse into any package
/// name its dependency edges name without the caller having to pre-load the
/// whole index.
pub struct RegistryDepSource<'a> {
    reg: &'a Registry,
}

impl<'a> RegistryDepSource<'a> {
    pub fn new(reg: &'a Registry) -> Self {
        RegistryDepSource { reg }
    }
}

impl<'a> DepSource for RegistryDepSource<'a> {
    fn versions(&self, name: &str) -> Result<Vec<Version>, Error> {
        let idx = lookup(self.reg, name)?;
        Ok(available_versions(&idx))
    }

    fn deps(&self, name: &str, v: &Version) -> Result<Vec<(String, Constraint)>, Error> {
        let idx = lookup(self.reg, name)?;
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

/// Fetch `tarball_url` into the file at `dest` (plan §5.4 step 3), preferring
/// the content-addressed archive cache (phase 7d S2, design §2.5/§3) for any
/// network URL.
///
/// - `file://` / plain local paths are copied directly (fully offline,
///   never cached — a local path is already as cheap as the cache, and
///   caching it would just be a second copy of the same bytes).
/// - `http(s)://` is resolved through [`crate::cache::get_or_fetch`]: a
///   cache hit that re-verifies against `sha256` is copied to `dest` with
///   zero network; a miss fetches over HTTP (or, with `opts.is_offline()`
///   set, fails with [`Error::Offline`] instead of fetching). Without the
///   `http` cargo feature, any fetch attempt is [`Error::HttpDisabled`].
pub fn fetch_tarball(
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

/// The raw network transport for a `http(s)://` URL, with no cache lookup —
/// [`crate::cache::get_or_fetch`]'s sole seam into the feature-gated `http`
/// module (kept private to this file otherwise).
pub(crate) fn raw_http_fetch(url: &str, dest: &Path) -> Result<(), Error> {
    http::get_to_file(url, dest)
}

// ---------------------------------------------------------------------------
// Mirror fallback (design §2): a registry may declare mirror base URLs; a
// failed fetch from the primary URL falls through to each mirror in order.
// ---------------------------------------------------------------------------

/// Build the ordered candidate URL list for a fetch (design §2.2 step 1): the
/// primary URL first, then each of `mirrors` rewritten against it
/// ([`rewrite_to_mirror`]). Shared by [`fetch_tarball`] and (Slice S)
/// `lookup`'s sparse per-package GET.
pub(crate) fn candidate_urls(primary: &str, mirrors: &[String]) -> Vec<String> {
    let mut urls = Vec::with_capacity(1 + mirrors.len());
    urls.push(primary.to_string());
    urls.extend(mirrors.iter().map(|m| rewrite_to_mirror(primary, m)));
    urls
}

/// Rewrite `primary` to be served by `mirror_base` instead (design §2.1): a
/// mirror base URL is a **host/prefix substitution** — take the primary URL's
/// path (and query) and re-root it under the mirror base, so the index stays
/// authoritative for the path layout and a mirror is just an alternate origin
/// serving the identical tree. E.g. primary
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

/// Try each of `urls` in order via `f`, returning the first `Ok`, or (if
/// every candidate fails) the **last** error encountered (design §2.2 step 4)
/// — a mirror serving wrong bytes or a 5xx/transport failure both fall
/// through to the next candidate. Shared by [`crate::cache::get_or_fetch`]'s
/// tarball-fetch loop and the sparse index's per-package GET.
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

/// Verify that the file at `path` hashes to `expected` (lowercase-hex SHA-256,
/// compared case-insensitively). [`Error::ChecksumMismatch`] otherwise — the
/// caller aborts before touching `dist/` (plan §5.4 step 3).
/// What an index entry says a download must hash to.
///
/// An OPAM repository publishes md5 and sha512; this port's own index
/// publishes sha256. Carrying both through the fetch means the CACHE KEY and
/// the VERIFICATION always speak the same algorithm — keying by an empty
/// sha256 while verifying a sha512 would collide every unverified download
/// onto one cache entry.
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
/// has one, else sha512. A version with neither is refused — an unverified
/// download that looks verified is the outcome to avoid.
pub fn verify_entry(path: &Path, sha256: &str, sha512: Option<&str>) -> Result<String, Error> {
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

pub(crate) mod http {
    use super::*;

    /// Overrides the connect/read timeout in seconds (design §2.6); production
    /// defaults to [`DEFAULT_TIMEOUT_SECS`]. Tests set this to a small value so
    /// the stalled-server timeout case does not have to wait out the real
    /// default.
    #[cfg(feature = "http")]
    pub const TIMEOUT_ENV: &str = "RUSTYFI_HTTP_TIMEOUT";

    #[cfg(feature = "http")]
    const DEFAULT_TIMEOUT_SECS: u64 = 30;

    /// Bounded redirect following (design §2.6) — enough for a CDN/registry
    /// mirror hop, not an open-ended chain.
    #[cfg(feature = "http")]
    const MAX_REDIRECTS: u32 = 5;

    /// Refuse to stream more than this many bytes from a single tarball
    /// response (design §2.6's "max response-body size guard"): a
    /// hostile/misconfigured server must not be able to exhaust disk/memory on
    /// a fetch. Generous for any real SATySFi package archive.
    #[cfg(feature = "http")]
    const MAX_BODY_BYTES: u64 = 512 * 1024 * 1024; // 512 MiB

    /// Bearer-token auth for the registry HTTP transport (saphe 7d slice S3,
    /// design §3 S3: "token auth via an env var … that ureq already
    /// supports"). When set, every tarball `GET` carries an `Authorization:
    /// Bearer <token>` header — `ureq`'s request builder already supports
    /// arbitrary headers, so this needs no auth-specific dependency. The
    /// value is read fresh per request and never included in any `Error`
    /// message/log (only the URL and status/transport text are).
    #[cfg(feature = "http")]
    pub const AUTH_TOKEN_ENV: &str = "RUSTYFI_REGISTRY_TOKEN";

    #[cfg(feature = "http")]
    fn auth_token() -> Option<String> {
        std::env::var(AUTH_TOKEN_ENV)
            .ok()
            .filter(|s| !s.is_empty())
    }

    #[cfg(feature = "http")]
    fn timeout() -> std::time::Duration {
        std::env::var(TIMEOUT_ENV)
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .map(std::time::Duration::from_secs)
            .unwrap_or(std::time::Duration::from_secs(DEFAULT_TIMEOUT_SECS))
    }

    /// A configured agent (design §2.4/§2.6): explicit connect+read timeout
    /// and a bounded redirect count, rather than the bare `ureq::get` this
    /// replaces — a stalled or endlessly-redirecting server must fail within a
    /// bounded budget instead of hanging the install.
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

    /// `get_to_file`'s sibling for the sparse index (Slice S, design §3.3):
    /// GET `url` and return its body as a `String` rather than writing it to
    /// disk — a `packages/<name>.toml` is small text, not a tarball, and has
    /// no sha256 to verify against (the index entry itself is what a
    /// tarball's sha256 checks). Reuses the same configured [`agent`], bearer
    /// auth, timeout, redirect bound, and body-size cap as [`get_to_file`].
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
        // Network URLs are never local.
        assert_eq!(local_path_from_url("https://example.com/x.tar.gz"), None);
        assert_eq!(local_path_from_url("git://host/repo.git"), None);
    }
}
