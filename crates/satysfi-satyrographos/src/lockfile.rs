//! `Satyrfile.lock` — the phase-2 lockfile (plan §5.3): mirrors
//! `Satyrfile.toml` 1:1, but with every entry's `source` pinned to a
//! concrete, content-addressed form (`sha256` of the resolved source tree)
//! plus a `resolved_at` timestamp. `reconcile.rs` diffs a fresh source hash
//! against the recorded one to decide whether an entry's files need
//! re-materialising or can be left untouched.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::Error;
use crate::satyrfile::{SourceSpec, MANIFEST_NAME};

/// The lockfile filename (a sibling of `Satyrfile.toml`).
pub const LOCK_NAME: &str = "Satyrfile.lock";

/// A parsed `Satyrfile.lock`.
#[derive(Debug, Default, Clone, Deserialize, Serialize)]
pub struct Lockfile {
    #[serde(default, rename = "library")]
    pub libraries: Vec<LockEntry>,
}

/// One locked entry: the manifest entry plus its resolved content hash.
///
/// For a `{ path = … }` source, `sha256` is a deterministic digest over the
/// source tree ([`util::sha256_tree`](crate::util::sha256_tree)) and `url` is
/// `None`. For a `{ registry = … }` source (phase 3, plan §5.4), the entry
/// pins the concrete resolved *(version, url, sha256)* — `source.version`
/// carries the resolved version, `url` the tarball URL, and `sha256` the
/// verified tarball digest — so a later reconcile can re-materialise it
/// **without re-consulting the index**.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LockEntry {
    pub name: String,
    /// The manifest's `source` table, carried through with `version` pinned to
    /// the resolved concrete version for registry entries.
    pub source: SourceSpec,
    /// Lowercase-hex SHA-256 of the resolved source (a directory-tree digest
    /// for path sources, or the verified tarball digest for registry sources).
    pub sha256: String,
    /// The resolved tarball URL (registry sources only; `None` for path).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// RFC 3339 UTC timestamp of when this entry was last resolved.
    pub resolved_at: String,
}

impl Lockfile {
    /// The locked entry for `name`, if any.
    pub fn get(&self, name: &str) -> Option<&LockEntry> {
        self.libraries.iter().find(|e| e.name == name)
    }
}

/// The lockfile path sibling to a `Satyrfile.toml` at `manifest_path`.
pub fn lock_path_for(manifest_path: &Path) -> PathBuf {
    match manifest_path.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir.join(LOCK_NAME),
        // A bare `Satyrfile.toml` with no directory component.
        _ => {
            debug_assert_eq!(
                manifest_path.file_name().and_then(|n| n.to_str()),
                Some(MANIFEST_NAME)
            );
            PathBuf::from(LOCK_NAME)
        }
    }
}

/// Read the lockfile at `path`. An absent lockfile is an empty [`Lockfile`],
/// not an error (a first `install` has none yet).
pub fn read(path: &Path) -> Result<Lockfile, Error> {
    if !path.is_file() {
        return Ok(Lockfile::default());
    }
    let text = std::fs::read_to_string(path).map_err(|e| Error::io(path, e))?;
    toml::from_str(&text).map_err(|source| Error::Lockfile {
        path: path.to_path_buf(),
        source,
    })
}

/// Serialise and atomically write `lock` to `path` (temp file + rename, so a
/// reader never sees a half-written lockfile — same discipline as receipts).
pub fn write(path: &Path, lock: &Lockfile) -> Result<(), Error> {
    let text = toml::to_string_pretty(lock).expect("lockfile serialises");
    let tmp = match path.file_name().and_then(|n| n.to_str()) {
        Some(name) => path.with_file_name(format!(".{name}.tmp")),
        None => path.with_extension("tmp"),
    };
    std::fs::write(&tmp, text).map_err(|e| Error::io(&tmp, e))?;
    std::fs::rename(&tmp, path).map_err(|e| Error::io(path, e))?;
    Ok(())
}
