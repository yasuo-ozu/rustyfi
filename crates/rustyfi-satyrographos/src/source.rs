//! Where a dependency comes from, and where the registry is — the parts of
//! the old `Satyrfile.toml` schema that outlived it.
//!
//! These types are shared by the manifest (`Satyristes`, which declares them)
//! and the lockfile (`Satyristes.lock`, which records what they resolved to),
//! so they keep their serde derives even though the manifest itself is now
//! S-expression rather than TOML.

use serde::{Deserialize, Serialize};

use crate::error::Error;

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
                kind: "empty (no path/git/registry field)".to_string(),
            })
        }
    }
}
