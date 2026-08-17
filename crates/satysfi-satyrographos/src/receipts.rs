//! Per-package receipts (plan §5.2): `<root>/.satyrographos/receipts/
//! <name>.toml`. This port's own bookkeeping, deliberately richer than
//! upstream Satyrographos' metadata sexp (which carries no file list),
//! because uninstall here is *incremental* — the receipt's `[[files]]` list
//! is the single source of truth for what `uninstall` may delete (plan §6).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::Error;
use crate::roots;

/// The current receipt schema version (plan §5.2).
pub const SCHEMA_VERSION: u32 = 1;

/// A single installed-package receipt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Receipt {
    pub schema_version: u32,
    pub name: String,
    pub package_version: String,
    /// RFC 3339 UTC, e.g. `2026-07-04T12:00:00Z`.
    pub installed_at: String,
    pub source: Source,
    #[serde(default)]
    pub files: Vec<FileEntry>,
}

/// Where the package came from (plan §5.2/§5.4). Phase 1's `path`/`archive`
/// variants carry only `kind` + `value`; the phase-3 `registry` variant
/// additionally records the resolved `version`, tarball `url`, and verified
/// `sha256`, so a later `list`/`status` can report exactly what was fetched.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Source {
    /// `path` | `archive` | `registry`.
    pub kind: String,
    /// The absolute source path (phase 1) or the registry package name (phase 3).
    pub value: String,
    /// The resolved concrete version (registry sources only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// The tarball URL fetched (registry sources only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// The verified tarball SHA-256 (registry sources only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
}

impl Source {
    /// A phase-1 `path`/`archive` source (no registry fields).
    pub fn plain(kind: impl Into<String>, value: impl Into<String>) -> Self {
        Source {
            kind: kind.into(),
            value: value.into(),
            version: None,
            url: None,
            sha256: None,
        }
    }
}

/// One materialised file, recorded relative to the library root.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    /// Path relative to `lib_root`, e.g.
    /// `dist/packages/great-package/great-package.satyh`. Always stored with
    /// `/` separators.
    pub dst: String,
    /// Lowercase-hex SHA-256 (optional in phase 1, plan §5.2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
}

/// Path of the receipt for `name` under `root`.
pub fn path(root: &Path, name: &str) -> PathBuf {
    roots::receipts_dir(root).join(format!("{name}.toml"))
}

/// Whether a receipt for `name` exists under `root`.
pub fn exists(root: &Path, name: &str) -> bool {
    path(root, name).is_file()
}

/// Read and parse the receipt for `name`. Returns [`Error::NotInstalled`] if
/// absent.
pub fn read(root: &Path, name: &str) -> Result<Receipt, Error> {
    let p = path(root, name);
    if !p.is_file() {
        return Err(Error::NotInstalled {
            name: name.to_string(),
            receipt: p,
        });
    }
    read_file(&p)
}

fn read_file(p: &Path) -> Result<Receipt, Error> {
    let text = std::fs::read_to_string(p).map_err(|e| Error::io(p, e))?;
    toml::from_str(&text).map_err(|source| Error::Receipt {
        path: p.to_path_buf(),
        source,
    })
}

/// Serialise and atomically write `receipt` (write to a sibling temp file,
/// then rename over the final path so a reader never sees a half-written
/// receipt).
pub fn write(root: &Path, receipt: &Receipt) -> Result<(), Error> {
    let dir = roots::receipts_dir(root);
    std::fs::create_dir_all(&dir).map_err(|e| Error::io(&dir, e))?;
    let final_path = path(root, &receipt.name);
    let tmp_path = dir.join(format!(".{}.toml.tmp", receipt.name));
    let text = toml::to_string_pretty(receipt).expect("receipt serialises");
    std::fs::write(&tmp_path, text).map_err(|e| Error::io(&tmp_path, e))?;
    std::fs::rename(&tmp_path, &final_path).map_err(|e| Error::io(&final_path, e))?;
    Ok(())
}

/// Remove the receipt for `name` (best-effort: a missing file is not an
/// error, so uninstall stays idempotent once the files are gone).
pub fn remove(root: &Path, name: &str) -> Result<(), Error> {
    let p = path(root, name);
    match std::fs::remove_file(&p) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(Error::io(&p, e)),
    }
}

/// All receipts under `root`, sorted by package name. An absent
/// `receipts/` directory is *empty*, not an error (plan §4.3: empty isn't an
/// error, only an unmanaged root is).
pub fn list_all(root: &Path) -> Result<Vec<Receipt>, Error> {
    let dir = roots::receipts_dir(root);
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut receipts = Vec::new();
    let entries = std::fs::read_dir(&dir).map_err(|e| Error::io(&dir, e))?;
    for entry in entries {
        let entry = entry.map_err(|e| Error::io(&dir, e))?;
        let p = entry.path();
        // Skip the `.<name>.toml.tmp` write-staging files and anything that
        // is not a `.toml`.
        let is_toml = p.extension().and_then(|e| e.to_str()) == Some("toml");
        let is_hidden = p
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.starts_with('.'))
            .unwrap_or(true);
        if is_toml && !is_hidden {
            receipts.push(read_file(&p)?);
        }
    }
    receipts.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(receipts)
}
