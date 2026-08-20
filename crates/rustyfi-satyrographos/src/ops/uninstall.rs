//! `uninstall` (plan §4.2, §6): remove *only* the files a receipt lists,
//! then the receipt, then now-empty parent directories — never a recursive
//! `rm -rf`, so hand-added files under the package's directory survive.

use std::path::{Path, PathBuf};

use crate::error::Error;
use crate::roots::RootSelection;
use crate::{receipts, stage, util};

/// Shared root-selection flags for uninstall/list/status (plan §7.2).
#[derive(Debug, Default, Clone)]
pub struct RootOptions {
    pub lib_root: Option<PathBuf>,
    pub dest: Option<PathBuf>,
}

impl RootSelection for RootOptions {
    fn lib_root(&self) -> Option<&Path> {
        self.lib_root.as_deref()
    }
    fn dest(&self) -> Option<&Path> {
        self.dest.as_deref()
    }
}

/// The `dist/<category>` directories that pruning must never remove (plan
/// §6): a package's *own* directory (`dist/packages/<name>/`) may go once
/// empty, but the shared category roots stay.
const PRUNE_STOP_DEPTH: usize = 3;

/// Uninstall the package named `name`.
pub fn uninstall(name: &str, opts: &RootOptions) -> Result<(), Error> {
    let root = opts.resolve_root()?;

    // No receipt → `NotInstalled` (CLI exit 4). This is the only source of
    // truth for what we may delete.
    let receipt = receipts::read(&root, name)?;

    let mut dirs_to_prune: Vec<PathBuf> = Vec::new();
    for file in &receipt.files {
        let path = stage::safe_join(&root, &file.dst)?;
        if file.keys.is_some() {
            // A shared `*.satysfi-hash`: take out this package's font entries
            // and leave the other packages' standing. Deleting the file would
            // uninstall every font in the root.
            crate::ops::install::withdraw_keys(&root, file)?;
        } else {
            util::remove_file_if_exists(&path)?;
        }
        if let Some(parent) = path.parent() {
            dirs_to_prune.push(parent.to_path_buf());
        }
    }

    // Remove now-empty parent directories, deepest first, stopping before
    // the shared `dist/<category>/` roots.
    dirs_to_prune.sort_by_key(|p| std::cmp::Reverse(p.components().count()));
    dirs_to_prune.dedup();
    for dir in dirs_to_prune {
        prune_empty_upward(&root, &dir)?;
    }

    receipts::remove_for(&root, name, receipt.lang)?;
    Ok(())
}

/// Remove `dir` and its ancestors while they are empty, stopping before any
/// directory whose depth below `root` is less than [`PRUNE_STOP_DEPTH`]
/// (i.e. keep `dist/`, `dist/packages/`, `dist/fonts/`, …).
fn prune_empty_upward(root: &Path, dir: &Path) -> Result<(), Error> {
    let mut cur = dir.to_path_buf();
    loop {
        // Outside the root, or shallower than the shared `dist/<category>/`
        // roots: never touch.
        let Ok(rel) = cur.strip_prefix(root) else {
            break;
        };
        if rel.components().count() < PRUNE_STOP_DEPTH {
            break;
        }
        if !is_empty_dir(&cur) {
            break;
        }
        match std::fs::remove_dir(&cur) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(Error::io(&cur, e)),
        }
        if !cur.pop() {
            break;
        }
    }
    Ok(())
}

fn is_empty_dir(dir: &Path) -> bool {
    match std::fs::read_dir(dir) {
        Ok(mut entries) => entries.next().is_none(),
        Err(_) => false,
    }
}
