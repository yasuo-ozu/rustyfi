//! Per-package receipts: `<root>/.satyrographos/receipts/
//! <name>.toml`. This port's own bookkeeping, deliberately richer than
//! upstream Satyrographos' metadata sexp (which carries no file list),
//! because uninstall here is *incremental* — the receipt's `[[files]]` list
//! is the single source of truth for what `uninstall` may delete.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::Error;
use crate::roots;
use crate::util;

/// The current receipt schema version.
pub const SCHEMA_VERSION: u32 = 1;

/// A single installed-package receipt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Receipt {
    /// Which corpus this install went into. Absent in receipts written before
    /// `lang` existed, which were all 0.0.
    #[serde(default)]
    pub lang: crate::manifest::Lang,
    pub schema_version: u32,
    pub name: String,
    pub package_version: String,
    /// RFC 3339 UTC, e.g. `2026-07-04T12:00:00Z`.
    pub installed_at: String,
    pub source: Source,
    #[serde(default)]
    pub files: Vec<FileEntry>,
}

/// Where the package came from. Phase 1's `path`/`archive`
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
    /// Lowercase-hex SHA-256 (optional in phase 1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    /// For a *shared* destination — a `*.satysfi-hash` that several packages
    /// contribute to — the keys THIS package put in. Uninstall removes these
    /// and leaves the rest of the file standing; without them the only options
    /// would be deleting other packages' fonts or leaking this one's.
    ///
    /// `None` is the ordinary case: the package owns the whole file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keys: Option<Vec<String>>,
}

/// Path of the receipt for `name` under `root`.
///
/// One manifest may install the same library for both generations, so a name
/// alone no longer identifies an install. 0.0 keeps the historical
/// `<name>.toml` — every receipt written before `lang` existed is a 0.0 one —
/// and 0.1 gets its own `<name>@0.1.toml` beside it.
pub fn path_for(root: &Path, name: &str, lang: crate::manifest::Lang) -> PathBuf {
    let file = match lang {
        crate::manifest::Lang::V0_0 => format!("{name}.toml"),
        other => format!("{name}@{}.toml", other.as_str()),
    };
    roots::receipts_dir(root).join(file)
}

/// Path of the 0.0 receipt for `name` — the common case.
pub fn path(root: &Path, name: &str) -> PathBuf {
    path_for(root, name, crate::manifest::Lang::default())
}

/// Whether a receipt exists for one (name, generation) pair — what an install
/// must ask, since the same name may be installed for both generations
/// independently.
pub fn exists_for(root: &Path, name: &str, lang: crate::manifest::Lang) -> bool {
    path_for(root, name, lang).is_file()
}

/// Read the receipt for one (name, generation) pair.
pub fn read_for(root: &Path, name: &str, lang: crate::manifest::Lang) -> Result<Receipt, Error> {
    let p = path_for(root, name, lang);
    if !p.is_file() {
        return Err(Error::NotInstalled {
            name: name.to_string(),
            receipt: p,
        });
    }
    read_file(&p)
}

/// Whether a receipt for `name` exists under `root`, in either generation.
pub fn exists(root: &Path, name: &str) -> bool {
    [crate::manifest::Lang::V0_0, crate::manifest::Lang::V0_1]
        .into_iter()
        .any(|lang| path_for(root, name, lang).is_file())
}

/// Read and parse the receipt for `name`. Returns [`Error::NotInstalled`] if
/// absent.
pub fn read(root: &Path, name: &str) -> Result<Receipt, Error> {
    // Either generation, 0.0 first — an unqualified name means "the one that
    // is installed", and only a manifest declaring both makes that ambiguous.
    for lang in [crate::manifest::Lang::V0_0, crate::manifest::Lang::V0_1] {
        let p = path_for(root, name, lang);
        if p.is_file() {
            return read_file(&p);
        }
    }
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
    let text = util::read_to_string(p)?;
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
    util::write_toml_atomic(&path_for(root, &receipt.name, receipt.lang), receipt)
}

/// Remove the receipt for `name` (best-effort: a missing file is not an
/// error, so uninstall stays idempotent once the files are gone).
pub fn remove(root: &Path, name: &str) -> Result<(), Error> {
    remove_for(root, name, crate::manifest::Lang::default())
}

/// Remove the receipt for one (name, generation) pair.
pub fn remove_for(root: &Path, name: &str, lang: crate::manifest::Lang) -> Result<(), Error> {
    util::remove_file_if_exists(&path_for(root, name, lang))
}

/// All receipts under `root`, sorted by package name. An absent
/// `receipts/` directory is *empty*, not an error (only an unmanaged root
/// is).
pub fn list_all(root: &Path) -> Result<Vec<Receipt>, Error> {
    let dir = roots::receipts_dir(root);
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut receipts = Vec::new();
    for p in util::read_dir_paths(&dir)? {
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
