//! `Satyristes.lock` — the phase-2 lockfile: mirrors `Satyristes` 1:1, but
//! with every entry's `source` pinned to a concrete, content-addressed form
//! (`sha256` of the resolved source tree) plus a `resolved_at` timestamp.
//! `reconcile.rs` diffs a fresh source hash against the recorded one to
//! decide whether an entry's files need re-materialising or can be left
//! untouched.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::Error;
use crate::satyristes::SATYRISTES_NAME as MANIFEST_NAME;
use crate::source::SourceSpec;
use crate::util;

/// The lockfile filename (a sibling of `Satyristes`).
const LOCK_NAME: &str = "Satyristes.lock";

/// A parsed `Satyristes.lock`.
#[derive(Debug, Default, Clone, Deserialize, Serialize)]
pub struct Lockfile {
    #[serde(default, rename = "library")]
    pub libraries: Vec<LockEntry>,
}

/// One locked entry: the manifest entry plus its resolved content hash.
///
/// For a `{ path = … }` source, `sha256` is a deterministic digest over the
/// source tree ([`source_digest`]) and `url` is
/// `None`. For a `{ registry = … }` source (phase 3), the entry
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

/// The content digest a `{ path = … }` [`LockEntry::sha256`] records —
/// exposed so an external tool (a `Satyristes.lock` writer or verifier that
/// is not this crate, e.g. the `rustyfi-saphe` port) can reproduce or check a
/// lock entry byte-for-byte without re-deriving the algorithm from scratch.
///
/// The algorithm, precisely:
///
/// - **A regular file** at `path` digests to the lowercase-hex SHA-256 of its
///   raw bytes.
/// - **A directory** at `path` digests as a Merkle-style hash over every
///   regular file it contains, at any depth: each contributing file emits one
///   row `"<rel-path>\0<file-sha256>\n"`, where `rel-path` is its path
///   relative to `path` with `/` separators (regardless of host OS) and
///   `file-sha256` is that file's own lowercase-hex SHA-256 digest as defined
///   above. The rows are sorted lexicographically by their exact `rel-path`
///   text, concatenated in that order, and the whole concatenation is hashed
///   with SHA-256 (lowercase hex). Sorting first makes the result independent
///   of directory-iteration order; the row shape (`\0` before the digest,
///   `\n` after it) makes it depend on both a file's path AND its content,
///   so either one changing changes the digest.
///
/// Sub-directories contribute no row of their own — only the files they
/// (transitively) contain do.
pub fn source_digest(path: &Path) -> Result<String, Error> {
    util::sha256_tree(path)
}

impl Lockfile {
    /// The locked entry for `name`, if any.
    pub(crate) fn get(&self, name: &str) -> Option<&LockEntry> {
        self.libraries.iter().find(|e| e.name == name)
    }

    /// A stable content fingerprint of the resolved graph (C3): a SHA-256
    /// over every entry's `(name, resolved version or source path, sha256)`
    /// triple, sorted by name so the digest does not depend on the
    /// lockfile's on-disk `[[library]]` order. Feeds the compiler's cache
    /// key (`rustyfi/src/cache.rs::compute_key`): a re-solve that changes
    /// any locked version invalidates every cached render keyed on the old
    /// digest.
    ///
    /// The "resolved version" component is `source.version` for a registry
    /// entry, `source.path` for a path entry (no version, but the path
    /// still changes the digest if re-pointed), or `source.rev` for a git
    /// entry (the resolved commit sha, slice S3) — a never-materialised
    /// entry falls back to `""`.
    pub fn digest(&self) -> String {
        let mut rows: Vec<(String, String, String)> = self
            .libraries
            .iter()
            .map(|e| {
                let version_or_path = e
                    .source
                    .version
                    .clone()
                    .or_else(|| e.source.path.clone())
                    .or_else(|| e.source.rev.clone())
                    .unwrap_or_default();
                (e.name.clone(), version_or_path, e.sha256.clone())
            })
            .collect();
        rows.sort();

        let mut hasher = Sha256::new();
        hasher.update(b"satyrfile-lock-digest\x00v1\x00");
        for (name, version, sha256) in &rows {
            hasher.update(name.as_bytes());
            hasher.update(b"\0");
            hasher.update(version.as_bytes());
            hasher.update(b"\0");
            hasher.update(sha256.as_bytes());
            hasher.update(b"\n");
        }
        util::hex(&hasher.finalize())
    }
}

/// The lockfile path sibling to a `Satyristes` at `manifest_path`.
pub fn lock_path_for(manifest_path: &Path) -> PathBuf {
    match manifest_path.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir.join(LOCK_NAME),
        // A bare `Satyristes` with no directory component.
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
    let text = util::read_to_string(path)?;
    toml::from_str(&text).map_err(|source| Error::Lockfile {
        path: path.to_path_buf(),
        source,
    })
}

/// Serialise and atomically write `lock` to `path` (temp file + rename, so a
/// reader never sees a half-written lockfile — same discipline as receipts).
pub(crate) fn write(path: &Path, lock: &Lockfile) -> Result<(), Error> {
    util::write_toml_atomic(path, lock)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, version: &str, sha256: &str) -> LockEntry {
        LockEntry {
            name: name.to_string(),
            source: SourceSpec {
                registry: Some(name.to_string()),
                version: Some(version.to_string()),
                ..Default::default()
            },
            sha256: sha256.to_string(),
            url: Some(format!("file:///{name}-{version}.tar.gz")),
            resolved_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn digest_is_stable_for_the_same_content() {
        let lock = Lockfile {
            libraries: vec![entry("a", "1.0.0", "aaaa"), entry("b", "2.0.0", "bbbb")],
        };
        assert_eq!(lock.digest(), lock.digest(), "deterministic across calls");
    }

    #[test]
    fn digest_is_order_independent() {
        let forward = Lockfile {
            libraries: vec![entry("a", "1.0.0", "aaaa"), entry("b", "2.0.0", "bbbb")],
        };
        let backward = Lockfile {
            libraries: vec![entry("b", "2.0.0", "bbbb"), entry("a", "1.0.0", "aaaa")],
        };
        assert_eq!(
            forward.digest(),
            backward.digest(),
            "the [[library]] array's on-disk order must not affect the digest"
        );
    }

    #[test]
    fn digest_changes_when_a_locked_version_changes() {
        let before = Lockfile {
            libraries: vec![entry("a", "1.0.0", "aaaa")],
        };
        let after = Lockfile {
            libraries: vec![entry("a", "1.1.0", "cccc")],
        };
        assert_ne!(before.digest(), after.digest());
    }

    #[test]
    fn empty_lockfile_has_a_digest_too() {
        // Not a hard requirement beyond "does not panic" — an empty closure
        // is a legitimate (if unusual) resolved graph.
        let empty = Lockfile::default();
        assert_eq!(empty.digest(), empty.digest());
    }
}
