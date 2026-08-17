//! `Satyrfile.toml` — the phase-2 project-level manifest (plan §5.3): a
//! sibling to `lib-satysfi/` declaring one project's dependencies, each an
//! entry with a `name` and a `source`. This is the *project* analog of the
//! per-package `satysfi-package.toml` (`manifest.rs`, §5.1): where that
//! describes *one* installable source, `Satyrfile.toml` lists *several*
//! sources a project wants materialised into its library root.
//!
//! ```toml
//! # Satyrfile.toml — sibling to lib-satysfi/
//! [[library]]
//! name = "great-package"
//! source = { path = "../vendor/great-package" }
//! # or: source = { git = "https://...", rev = "abcdef0" }        (phase 3+)
//! # or: source = { registry = "great-package", version = "1.0.0" } (phase 3)
//! ```
//!
//! Only the `{ path = … }` source is materialisable in phase 2; `git`/
//! `registry` sources parse but are rejected at reconcile time
//! ([`Error::UnsupportedSource`]) until later phases.

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
    /// `$SATYSFI_REGISTRY` is given, so a project's `registry` sources reconcile
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
}

/// The `[registry]` section of a `Satyrfile.toml`.
#[derive(Debug, Default, Clone, Deserialize, Serialize)]
pub struct RegistryConfig {
    /// The registry index URL (a git repo or a plain-directory index).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
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
    /// A local directory or `.tar.gz` (phase 2 — the only materialisable kind).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// A git URL (phase 3+; parsed, not yet fetched).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git: Option<String>,
    /// The pinned git revision that accompanies `git`.
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
/// through ancestor directories (mirrors the compile-mode `lib-satysfi/`
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
