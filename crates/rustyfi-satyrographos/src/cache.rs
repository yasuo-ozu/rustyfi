//! Content-addressed archive cache (saphe phase 7d slice S2, design
//! §2.5/§3 S2): a persistent, sha256-keyed store for fetched `.tar.gz`
//! package archives, so re-materialising an already-locked registry pin
//! never re-downloads bytes it already verified once, and `--offline`
//! reconcile can succeed straight from disk.
//!
//! ## Key and location
//!
//! The cache key is the archive's SHA-256 — exactly what a lockfile
//! [`crate::lockfile::LockEntry`] / registry [`crate::registry::VersionEntry`]
//! already carries, so the key is known *before* any fetch. The cache root
//! mirrors [`crate::registry::RegistryOptions::cache_root`]'s XDG-cache-dir
//! resolution (both share [`crate::util::xdg_cache_base`]), but under a
//! sibling `archives/` leaf instead of `registry/` — the two caches hold
//! different content (a git-cloned index tree vs. content-addressed tarball
//! blobs) and must never collide:
//!
//! 1. [`RegistryOptions::archive_cache_dir`] (highest precedence — set by a
//!    caller/test directly), else
//! 2. `$RUSTYFI_ARCHIVE_CACHE`, else
//! 3. `$XDG_CACHE_HOME/rustyfi/archives/` (else `$HOME/.cache/…`, else
//!    `.cache/…`).
//!
//! ## Fetch flow ([`get_or_fetch`])
//!
//! 1. If `<cache_root>/<sha256>.tar.gz` exists **and re-verifies** against
//!    `sha256`, copy it to `dest` — zero network. A cache entry that exists
//!    but fails to re-verify (corrupted on disk, or a same-hash collision
//!    that somehow doesn't match — should never happen, but is not trusted
//!    blindly either way) is discarded and treated as a miss.
//! 2. Otherwise: if [`RegistryOptions::is_offline`], [`Error::Offline`] —
//!    no request is attempted.
//! 3. Otherwise: fetch `url` into a private temp file under the cache root,
//!    verify it against `sha256` (a mismatch deletes the temp file and
//!    returns [`Error::ChecksumMismatch`] without populating the cache),
//!    then atomically rename it into `<cache_root>/<sha256>.tar.gz` and copy
//!    it to `dest`.

use std::path::{Path, PathBuf};

use crate::error::Error;
use crate::registry::{self, RegistryOptions};
use crate::util;

/// Environment override for the archive cache directory (mirrors
/// [`crate::registry::CACHE_ENV`]'s role for the registry-index cache).
pub const ARCHIVE_CACHE_ENV: &str = "RUSTYFI_ARCHIVE_CACHE";

/// The archive cache root directory (design §2.5's cache location
/// precedence). Archives themselves live at `<cache_root>/<sha256>.tar.gz`
/// (see [`cache_path`]).
pub fn cache_root(opts: &RegistryOptions) -> PathBuf {
    if let Some(dir) = &opts.archive_cache_dir {
        return dir.clone();
    }
    if let Some(dir) = std::env::var_os(ARCHIVE_CACHE_ENV) {
        if !dir.is_empty() {
            return PathBuf::from(dir);
        }
    }
    util::xdg_cache_base().join("rustyfi").join("archives")
}

/// The on-disk path a `sha256`-keyed archive would live at under `opts`'s
/// cache root. `sha256` is normalised to lowercase (the same
/// case-insensitive comparison [`registry::verify_sha256`] uses) so an
/// index/lockfile entry with uppercase hex still hits the same cache file.
pub fn cache_path(opts: &RegistryOptions, sha256: &str) -> PathBuf {
    cache_root(opts).join(format!("{}.tar.gz", sha256.trim().to_lowercase()))
}

/// Populate `dest` with the archive at one of `urls` (the primary tarball URL
/// first, then any mirror candidates — mirrors design §2.2) matching
/// `sha256`, preferring the cache (see the module docs for the full flow).
/// This is [`crate::registry::fetch_tarball`]'s sole seam for any `http(s)://`
/// URL — `file://`/plain-path URLs never reach here (they are cheap enough,
/// and already-local, that caching them would just be a second copy of the
/// same bytes).
///
/// **The cache key is `sha256` alone, independent of which URL supplied the
/// bytes** (mirrors design §2.2 step 2): a warm cache entry is copied to
/// `dest` before any candidate in `urls` is even looked at, so a repeat
/// install with a populated cache never touches the network — primary or
/// mirror. On a cache miss, each candidate is fetched to a temp file and
/// verified against `sha256` *before* the cache is populated (design §2.2
/// step 3) — a mirror serving wrong bytes cannot poison the cache; it is
/// simply discarded and the next candidate is tried
/// ([`registry::try_candidates`]).
pub(crate) fn get_or_fetch(
    urls: &[String],
    sha256: &str,
    dest: &Path,
    opts: &RegistryOptions,
) -> Result<(), Error> {
    let cached = cache_path(opts, sha256);
    if cached.is_file() && registry::verify_sha256(&cached, sha256).is_ok() {
        std::fs::copy(&cached, dest).map_err(|e| Error::io(&cached, e))?;
        return Ok(());
    }
    // A stale/corrupted cache entry (present but failed to re-verify) is
    // dropped rather than trusted — fall through to a fresh fetch below,
    // which will overwrite it once the new download re-verifies.
    if cached.is_file() {
        let _ = std::fs::remove_file(&cached);
    }

    if opts.is_offline() {
        return Err(Error::Offline {
            url: urls.first().cloned().unwrap_or_default(),
        });
    }

    let root = cache_root(opts);
    std::fs::create_dir_all(&root).map_err(|e| Error::io(&root, e))?;

    registry::try_candidates(urls, |url| {
        let tmp = root.join(format!(
            "download-{}.tar.gz.tmp",
            crate::stage::unique_suffix()
        ));
        let _guard = TmpGuard(tmp.clone());

        registry::raw_http_fetch(url, &tmp)?;
        registry::verify_sha256(&tmp, sha256)?; // mismatch: TmpGuard cleans `tmp`, cache stays unpopulated; try the next candidate.

        // Atomically populate the cache (rename, not copy, so a reader never
        // observes a half-written cache entry), then copy the verified bytes
        // to `dest`. `rename` may fail across filesystems (e.g. a cache root
        // on a different mount than itself, which cannot happen here since
        // both paths are under `root`) — fall back to copy+remove
        // defensively.
        if std::fs::rename(&tmp, &cached).is_err() {
            std::fs::copy(&tmp, &cached).map_err(|e| Error::io(&cached, e))?;
            let _ = std::fs::remove_file(&tmp);
        }
        std::fs::copy(&cached, dest).map_err(|e| Error::io(&cached, e))?;
        Ok(())
    })
}

/// Removes the temp download file when dropped (best-effort) — covers both
/// the early-return on a checksum mismatch and the ordinary case where the
/// file has already been renamed away (removing an already-gone path is a
/// silent no-op).
struct TmpGuard(PathBuf);

impl Drop for TmpGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_path_lowercases_and_keys_by_sha() {
        let opts = RegistryOptions {
            archive_cache_dir: Some(PathBuf::from("/tmp/archives")),
            ..Default::default()
        };
        assert_eq!(
            cache_path(&opts, "ABCDEF"),
            PathBuf::from("/tmp/archives/abcdef.tar.gz")
        );
    }

    #[test]
    fn cache_root_prefers_explicit_dir_over_env() {
        let opts = RegistryOptions {
            archive_cache_dir: Some(PathBuf::from("/explicit")),
            ..Default::default()
        };
        assert_eq!(cache_root(&opts), PathBuf::from("/explicit"));
    }
}
