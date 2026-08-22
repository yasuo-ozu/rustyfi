//! Where a dependency comes from, and where the registry is — the parts of
//! the old `Satyrfile.toml` schema that outlived it.
//!
//! These types are shared by the manifest (`Satyristes`, which declares them)
//! and the lockfile (`Satyristes.lock`, which records what they resolved to),
//! so they keep their serde derives even though the manifest itself is now
//! S-expression rather than TOML.

use serde::{Deserialize, Serialize};

use crate::error::Error;

/// A `[[registry]]` entry of a `config.toml` (see `config.rs`).
#[derive(Debug, Default, Clone, Deserialize, Serialize)]
pub struct RegistryConfig {
    /// The registry index URL (a git repo or a plain-directory index).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Fallback registry base URLs, tried in order after `url` when a fetch
    /// from it fails (a tarball fetch, or — Slice S — a sparse
    /// per-package index GET). Additive; each candidate is verified against
    /// the same sha256 for a tarball fetch. Empty by default: this field is
    /// `#[serde(default)]` and the crate uses no `deny_unknown_fields`
    /// anywhere, so a manifest with or without a `mirrors` key parses under
    /// either binary.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mirrors: Vec<String>,
    /// The index transport. `None` (absent from the TOML) =
    /// [`RegistryKind::Auto`], the local-dir/git dispatch by URL shape. Only
    /// `kind = "sparse"` selects the per-package HTTP index path (Slice S).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<RegistryKind>,
}

/// How to reach a registry's package index. Serialized as a
/// lowercase TOML string (`"auto"` / `"git"` / `"sparse"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RegistryKind {
    /// The default when `kind` is absent from the TOML: a local path (or
    /// `file://`) already holding `packages/` is read in place; anything else
    /// is git-cloned.
    Auto,
    /// Always git-clone/fetch `url`, even if it happens to resolve to a
    /// local path that already has a `packages/` subdirectory (skips the
    /// plain-directory short-circuit `Auto` takes).
    Git,
    /// A sparse HTTP index: `packages/<name>.toml` is fetched
    /// over HTTP on demand, one package at a time, instead of being cloned
    /// as a whole tree.
    Sparse,
}

/// One dependency entry from a `Satyristes` `(library (dependencies …))`
/// block: a name plus where to get it.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LibraryEntry {
    pub name: String,
    pub source: SourceSpec,
}

/// A `source = { … }` inline table. Represented as a flat struct of optional
/// fields (rather than an `#[serde(untagged)]` enum, which the `toml` crate
/// handles unreliably) so it round-trips cleanly through both the manifest
/// and `Satyristes.lock`. [`SourceSpec::kind`] interprets which variant a given
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

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
