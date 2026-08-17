//! The filesystem transaction (plan §6): stage every file under
//! `<root>/.satyrographos/tmp/…` (same filesystem as `dist/`, so the final
//! move is an atomic rename), reject path-traversal while staging, then swap
//! into place — orphaning any files a prior receipt for the same package
//! claimed, so a crash mid-swap leaves either the old or the new tree, never
//! a half-written one.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::error::Error;
use crate::roots;

/// A process/thread-unique suffix for temp directory names.
pub fn unique_suffix() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{}-{}-{}", std::process::id(), nanos, n)
}

/// A staging directory under `<root>/.satyrographos/tmp/`, removed on drop.
pub struct StagingArea {
    root: PathBuf,
}

impl StagingArea {
    /// Create `<root>/.satyrographos/tmp/<name>-<unique>/`.
    pub fn new(lib_root: &Path, name: &str) -> Result<Self, Error> {
        let path = roots::tmp_dir(lib_root).join(format!("{name}-{}", unique_suffix()));
        std::fs::create_dir_all(&path).map_err(|e| Error::io(&path, e))?;
        Ok(StagingArea { root: path })
    }

    pub fn path(&self) -> &Path {
        &self.root
    }

    /// Copy `src` into the staging tree at the root-relative `dst`
    /// (`/`-separated). Rejects a `dst` that escapes the staging root.
    pub fn stage(&self, dst: &str, src: &Path) -> Result<(), Error> {
        let target = safe_join(&self.root, dst)?;
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
        }
        std::fs::copy(src, &target).map_err(|e| Error::io(src, e))?;
        Ok(())
    }
}

impl Drop for StagingArea {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// Turn a `/`-separated root-relative destination into a `PathBuf` under
/// `base`, rejecting absolute paths and `..` components (path-traversal
/// guard, plan §6).
pub fn safe_join(base: &Path, dst: &str) -> Result<PathBuf, Error> {
    let rel = rel_to_path(dst);
    if rel.is_absolute() {
        return Err(Error::PathTraversal { entry: rel });
    }
    for comp in rel.components() {
        if matches!(comp, std::path::Component::ParentDir) {
            return Err(Error::PathTraversal { entry: rel });
        }
    }
    Ok(base.join(rel))
}

fn rel_to_path(dst: &str) -> PathBuf {
    let mut p = PathBuf::new();
    for part in dst.split('/').filter(|s| !s.is_empty()) {
        p.push(part);
    }
    p
}

/// Move the staged files into `<root>`, first orphaning any files listed in
/// `old_dsts` (a prior receipt for the same package, `--force` path) so they
/// are gone even if the new install no longer produces them. Orphaned files
/// live under `<staging_root>/__orphan/` and vanish when the staging area is
/// dropped.
pub fn materialize(
    lib_root: &Path,
    staging_root: &Path,
    new_dsts: &[String],
    old_dsts: &[String],
) -> Result<(), Error> {
    let orphan = staging_root.join("__orphan");

    // Step 1: orphan the previous receipt's files (force reinstall).
    for dst in old_dsts {
        let live = safe_join(lib_root, dst)?;
        if live.exists() {
            let aside = safe_join(&orphan, dst)?;
            if let Some(parent) = aside.parent() {
                std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
            }
            std::fs::rename(&live, &aside).map_err(|e| Error::io(&live, e))?;
        }
    }

    // Step 2: move each staged file into its final destination.
    for dst in new_dsts {
        let from = safe_join(staging_root, dst)?;
        let to = safe_join(lib_root, dst)?;
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
        }
        if to.exists() {
            remove_path(&to)?;
        }
        std::fs::rename(&from, &to).map_err(|e| Error::io(&to, e))?;
    }
    Ok(())
}

fn remove_path(p: &Path) -> Result<(), Error> {
    let meta = std::fs::symlink_metadata(p).map_err(|e| Error::io(p, e))?;
    if meta.is_dir() {
        std::fs::remove_dir_all(p).map_err(|e| Error::io(p, e))
    } else {
        std::fs::remove_file(p).map_err(|e| Error::io(p, e))
    }
}
