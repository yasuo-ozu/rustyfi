//! Small dependency-free helpers: an RFC 3339 UTC timestamp for receipts
//! (plan §5.2's `installed_at`) and a SHA-256 file digest (plan §5.2's
//! optional-phase-1 `sha256`). Kept here rather than pulling in `chrono`:
//! the plan (§7.1) names `sha2`/`toml`/`tar`/`flate2` as the crate's deps
//! and nothing for time, and the timestamp is only ever *written*, never
//! parsed back, so a hand-rolled formatter suffices.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

use crate::error::Error;

/// Current wall-clock time as an RFC 3339 UTC string, e.g.
/// `2026-07-04T12:00:00Z`. Uses Howard Hinnant's `civil_from_days` to turn
/// a Unix day count into a Gregorian date without any calendar crate.
pub fn now_rfc3339() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    rfc3339_from_unix(secs)
}

fn rfc3339_from_unix(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (hh, mm, ss) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let (y, mo, d) = civil_from_days(days);
    format!("{y:04}-{mo:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}Z")
}

/// Days since 1970-01-01 → (year, month, day). Hinnant's algorithm
/// (`http://howardhinnant.github.io/date_algorithms.html#civil_from_days`).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Lowercase-hex SHA-256 of the file at `path`.
pub fn sha256_file(path: &Path) -> Result<String, Error> {
    let bytes = std::fs::read(path).map_err(|e| Error::io(path, e))?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    Ok(hex(&hasher.finalize()))
}

/// A deterministic content digest of the source at `path` (plan §5.3's
/// content-addressed lockfile hash). A regular file hashes to its own
/// [`sha256_file`]; a directory hashes to a Merkle-style digest over its
/// files — each contributing `"<rel-path>\0<file-sha256>\n"` in sorted order,
/// so the digest is stable regardless of directory-iteration order and
/// changes iff any file's path or content changes.
pub fn sha256_tree(path: &Path) -> Result<String, Error> {
    if path.is_file() {
        return sha256_file(path);
    }
    let mut entries: Vec<(String, String)> = Vec::new();
    let mut stack = vec![(path.to_path_buf(), String::new())];
    while let Some((dir, rel)) = stack.pop() {
        let read = std::fs::read_dir(&dir).map_err(|e| Error::io(&dir, e))?;
        for entry in read {
            let entry = entry.map_err(|e| Error::io(&dir, e))?;
            let p = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            let child_rel = if rel.is_empty() {
                name
            } else {
                format!("{rel}/{name}")
            };
            if p.is_dir() {
                stack.push((p, child_rel));
            } else if p.is_file() {
                entries.push((child_rel, sha256_file(&p)?));
            }
        }
    }
    entries.sort();
    let mut hasher = Sha256::new();
    for (rel, digest) in &entries {
        hasher.update(rel.as_bytes());
        hasher.update(b"\0");
        hasher.update(digest.as_bytes());
        hasher.update(b"\n");
    }
    Ok(hex(&hasher.finalize()))
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_formats() {
        assert_eq!(rfc3339_from_unix(0), "1970-01-01T00:00:00Z");
    }

    #[test]
    fn known_date_formats() {
        // 2026-07-04T12:00:00Z == 1_783_166_400 seconds since the epoch.
        assert_eq!(rfc3339_from_unix(1_783_166_400), "2026-07-04T12:00:00Z");
    }
}
