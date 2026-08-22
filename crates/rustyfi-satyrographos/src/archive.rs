//! Install-source sniffing and archive extraction. A source is
//! either a directory (used in place) or a `.tar.gz`/`.tgz` archive
//! (extracted, under `<root>/.satyrographos/tmp/`, with path-traversal
//! rejection — the zip-slip guard).

use std::path::{Path, PathBuf};

use flate2::read::GzDecoder;

use crate::error::Error;
use crate::manifest::MANIFEST_NAME;
use crate::roots;
use crate::stage::unique_suffix;

/// A source made ready to plan against: `source_root` is a live directory
/// tree either way; `guard`, when present, owns a temp extraction dir that
/// is removed on drop.
pub struct Prepared {
    pub source_root: PathBuf,
    /// `"path"` for a directory source, `"archive"` for a `.tar.gz`.
    pub kind: &'static str,
    /// Absolute path of the original source (recorded in the receipt).
    pub value: PathBuf,
    #[allow(dead_code)]
    guard: Option<ExtractGuard>,
}

/// Removes an extraction directory when dropped (best-effort).
struct ExtractGuard(PathBuf);

impl Drop for ExtractGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Prepare `source`: a directory is used verbatim; a `.tar.gz`/`.tgz` is
/// extracted into a temp dir under `root`. `root` must already be a managed
/// root (`.satyrographos/tmp/` present).
pub fn prepare(source: &Path, root: &Path) -> Result<Prepared, Error> {
    let value = std::fs::canonicalize(source).unwrap_or_else(|_| source.to_path_buf());
    if source.is_dir() {
        return Ok(Prepared {
            source_root: source.to_path_buf(),
            kind: "path",
            value,
            guard: None,
        });
    }
    if source.is_file() && is_tarball(source) {
        let dest = roots::tmp_dir(root).join(format!("extract-{}", unique_suffix()));
        std::fs::create_dir_all(&dest).map_err(|e| Error::io(&dest, e))?;
        let guard = ExtractGuard(dest.clone());
        extract_tar_gz(source, &dest)?;
        let source_root = find_source_root(&dest)?;
        return Ok(Prepared {
            source_root,
            kind: "archive",
            value,
            guard: Some(guard),
        });
    }
    Err(Error::UnknownSource {
        path: source.to_path_buf(),
    })
}

fn is_tarball(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    name.ends_with(".tar.gz") || name.ends_with(".tgz")
}

fn extract_tar_gz(archive: &Path, dest: &Path) -> Result<(), Error> {
    let file = std::fs::File::open(archive).map_err(|e| Error::io(archive, e))?;
    let mut tar = tar::Archive::new(GzDecoder::new(file));
    let entries = tar
        .entries()
        .map_err(|e| Error::Archive(format!("{}: {e}", archive.display())))?;
    for entry in entries {
        let mut entry = entry.map_err(|e| Error::Archive(format!("{}: {e}", archive.display())))?;
        let entry_path = entry
            .path()
            .map_err(|e| Error::Archive(format!("{}: {e}", archive.display())))?
            .into_owned();
        // `unpack_in` refuses (returns Ok(false)) any entry whose path is
        // absolute or climbs out of `dest` via `..` — that is our zip-slip
        // guard. A refused entry is a hard error, not a skip.
        let unpacked = entry
            .unpack_in(dest)
            .map_err(|e| Error::io(dest.join(&entry_path), e))?;
        if !unpacked {
            return Err(Error::PathTraversal { entry: entry_path });
        }
    }
    Ok(())
}

/// After extraction, locate the directory that actually holds the package:
/// the extraction dir itself if it has a manifest/`packages/`, otherwise its
/// single child directory (the common `tar czf x.tar.gz great-package/`
/// layout).
fn find_source_root(dest: &Path) -> Result<PathBuf, Error> {
    if has_package_markers(dest) {
        return Ok(dest.to_path_buf());
    }
    let mut children: Vec<PathBuf> = std::fs::read_dir(dest)
        .map_err(|e| Error::io(dest, e))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    if children.len() == 1 {
        let only = children.pop().unwrap();
        if has_package_markers(&only) {
            return Ok(only);
        }
        return Ok(only);
    }
    // Nothing obvious; let `manifest::discover` report EmptySource against
    // the extraction root.
    Ok(dest.to_path_buf())
}

fn has_package_markers(dir: &Path) -> bool {
    dir.join(MANIFEST_NAME).is_file()
        || dir.join(crate::satyristes::SATYRISTES_NAME).is_file()
        || dir.join("packages").is_dir()
}
