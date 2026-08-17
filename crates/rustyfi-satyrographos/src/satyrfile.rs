//! `Satyrfile.toml` — the phase-2 project-level manifest (plan §5.3): a
//! sibling to `lib-rustyfi/` declaring one project's dependencies, each an
//! entry with a `name` and a `source`. This is the *project* analog of the
//! per-package `rustyfi-package.toml` (`manifest.rs`, §5.1): where that
//! describes *one* installable source, `Satyrfile.toml` lists *several*
//! sources a project wants materialised into its library root.
//!
//! ```toml
//! # Satyrfile.toml — sibling to lib-rustyfi/
//! [[library]]
//! name = "great-package"
//! source = { path = "../vendor/great-package" }
//! # or: source = { git = "https://...", rev = "abcdef0" }        (phase 3+)
//! # or: source = { registry = "great-package", version = "1.0.0" } (phase 3)
//! ```
//!
//! All three source kinds are materialisable: `{ path = … }` since phase 2,
//! `{ registry = … }` since phase 3, and `{ git = … }` since saphe 7d slice S3
//! (`docs/plans/design-saphe-7d-network.md`). A `source = { … }` table naming
//! none of `path`/`git`/`registry` is [`Error::UnsupportedSource`].

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::Error;
use crate::util;

/// The manifest filename located by [`find_upward`].
pub const MANIFEST_NAME: &str = "Satyrfile.toml";

/// A parsed `Satyrfile.toml`.
#[derive(Debug, Default, Clone, Deserialize, Serialize)]
pub struct Satyrfile {
    /// Optional `[registry]` section (plan §5.4 / phase 3): a project-level
    /// registry URL, used as the fallback when neither `--registry` nor
    /// `$RUSTYFI_REGISTRY` is given, so a project's `registry` sources reconcile
    /// without a flag.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registry: Option<RegistryConfig>,
    /// Declared dependency entries (`[[library]]` tables).
    #[serde(default, rename = "library")]
    pub libraries: Vec<LibraryEntry>,
}

impl Satyrfile {
    /// The project's declared registry URL, if any.
    pub fn registry_url(&self) -> Option<&str> {
        self.registry.as_ref().and_then(|r| r.url.as_deref())
    }

    /// The project's declared mirror base URLs (design
    /// `docs/plans/design-saphe-mirrors-sparse.md` §2.1), empty when no
    /// `[registry] mirrors` is declared.
    pub fn registry_mirrors(&self) -> &[String] {
        self.registry
            .as_ref()
            .map(|r| r.mirrors.as_slice())
            .unwrap_or(&[])
    }

    /// The project's declared index transport kind (design §3.2), `None`
    /// when no `[registry] kind` is declared (equivalent to
    /// [`RegistryKind::Auto`]).
    pub fn registry_kind(&self) -> Option<RegistryKind> {
        self.registry.as_ref().and_then(|r| r.kind)
    }
}

/// The `[registry]` section of a `Satyrfile.toml`.
#[derive(Debug, Default, Clone, Deserialize, Serialize)]
pub struct RegistryConfig {
    /// The registry index URL (a git repo or a plain-directory index).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Fallback registry base URLs, tried in order after `url` when a fetch
    /// from it fails (design §2.1: a tarball fetch, or — Slice S — a sparse
    /// per-package index GET). Additive; each candidate is verified against
    /// the same sha256 for a tarball fetch. Empty by default, so an existing
    /// manifest with no `mirrors` key behaves exactly as before (this field
    /// is `#[serde(default)]`, and the crate uses no `deny_unknown_fields`
    /// anywhere, so old and new manifests parse under either binary).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mirrors: Vec<String>,
    /// The index transport (design §3.2). `None` (absent from the TOML) =
    /// [`RegistryKind::Auto`] — exactly today's local-dir/git dispatch by URL
    /// shape, byte for byte. Only `kind = "sparse"` selects the new
    /// per-package HTTP index path (Slice S).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<RegistryKind>,
}

/// How to reach a registry's package index (design §3.2). Serialized as a
/// lowercase TOML string (`"auto"` / `"git"` / `"sparse"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RegistryKind {
    /// Today's dispatch: a local path (or `file://`) already holding
    /// `packages/` is read in place; anything else is git-cloned. The
    /// default when `kind` is absent from the TOML.
    Auto,
    /// Always git-clone/fetch `url`, even if it happens to resolve to a
    /// local path that already has a `packages/` subdirectory (skips the
    /// plain-directory short-circuit `Auto` takes).
    Git,
    /// A sparse HTTP index (design §3): `packages/<name>.toml` is fetched
    /// over HTTP on demand, one package at a time, instead of being cloned
    /// as a whole tree.
    Sparse,
}

/// One `[[library]]` entry: a name plus where to get it.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LibraryEntry {
    pub name: String,
    pub source: SourceSpec,
}

/// A `source = { … }` inline table. Represented as a flat struct of optional
/// fields (rather than an `#[serde(untagged)]` enum, which the `toml` crate
/// handles unreliably) so it round-trips cleanly through both `Satyrfile.toml`
/// and `Satyrfile.lock`. [`SourceSpec::kind`] interprets which variant a given
/// table denotes.
#[derive(Debug, Default, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct SourceSpec {
    /// A local directory or `.tar.gz` (phase 2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// A git URL (saphe 7d slice S3): cloned via the `git` CLI, pinned to
    /// `rev` when given, else the remote's default-branch tip.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git: Option<String>,
    /// The pinned git revision (branch, tag, or commit-ish) that accompanies
    /// `git`; a resolved lockfile entry always carries the concrete resolved
    /// commit sha here (saphe 7d slice S3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rev: Option<String>,
    /// A registry package name (phase 3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registry: Option<String>,
    /// The registry version that accompanies `registry`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

/// Which concrete source kind a [`SourceSpec`] table denotes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceKind<'a> {
    /// `{ path = "…" }`.
    Path(&'a str),
    /// `{ git = "…", rev = "…" }`.
    Git { git: &'a str, rev: Option<&'a str> },
    /// `{ registry = "…", version = "…" }`.
    Registry {
        registry: &'a str,
        version: Option<&'a str>,
    },
}

impl SourceSpec {
    /// Interpret this table into a [`SourceKind`], preferring the most
    /// specific field present. An empty/unrecognised table is
    /// [`Error::UnsupportedSource`].
    pub fn kind(&self) -> Result<SourceKind<'_>, Error> {
        if let Some(path) = &self.path {
            Ok(SourceKind::Path(path))
        } else if let Some(git) = &self.git {
            Ok(SourceKind::Git {
                git,
                rev: self.rev.as_deref(),
            })
        } else if let Some(registry) = &self.registry {
            Ok(SourceKind::Registry {
                registry,
                version: self.version.as_deref(),
            })
        } else {
            Err(Error::UnsupportedSource {
                kind: "empty (no path/git/registry field)",
            })
        }
    }
}

/// Read and parse the `Satyrfile.toml` at `path`.
pub fn read(path: &Path) -> Result<Satyrfile, Error> {
    let text = util::read_to_string(path)?;
    toml::from_str(&text).map_err(|source| Error::Satyrfile {
        path: path.to_path_buf(),
        source,
    })
}

/// Locate the nearest `Satyrfile.toml` at or above `start`, walking upward
/// through ancestor directories (mirrors the compile-mode `lib-rustyfi/`
/// upward search, plan §3). Returns the file's absolute-ish path.
pub fn find_upward(start: &Path) -> Option<PathBuf> {
    let mut dir = start.to_path_buf();
    loop {
        let candidate = dir.join(MANIFEST_NAME);
        if candidate.is_file() {
            return Some(candidate);
        }
        if !dir.pop() {
            return None;
        }
    }
}
