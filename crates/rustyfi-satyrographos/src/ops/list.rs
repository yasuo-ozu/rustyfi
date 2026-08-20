//! `list` (plan §4.3): one summary per installed receipt, sorted; an empty
//! or absent `.satyrographos/receipts/` is not an error.

use crate::error::Error;
use crate::ops::uninstall::RootOptions;
use crate::receipts;
use crate::roots::RootSelection;

/// A one-line summary of an installed package.
#[derive(Debug, Clone)]
pub struct PackageSummary {
    pub name: String,
    /// Which corpus it is installed into — the same name may appear twice,
    /// once per generation.
    pub lang: crate::manifest::Lang,
    pub version: String,
    pub file_count: usize,
    /// Where its files actually are: the root joined with the directory they
    /// share, e.g. `<root>/dist/packages/<name>`. A package whose files do not
    /// share a directory — `(hash …)` lands flat in `dist/hash/` — falls back
    /// to the root itself.
    pub path: std::path::PathBuf,
}

/// List every installed package, sorted by name.
pub fn list(opts: &RootOptions) -> Result<Vec<PackageSummary>, Error> {
    let root = opts.resolve_root()?;
    let receipts = receipts::list_all(&root)?;
    Ok(receipts
        .into_iter()
        .map(|r| PackageSummary {
            lang: r.lang,
            path: install_path(&root, &r),
            name: r.name,
            version: r.package_version,
            file_count: r.files.len(),
        })
        .collect())
}

/// The directory a receipt's files share, under `root`.
///
/// Receipts record root-relative destinations, so the answer is the longest
/// common directory prefix of them — `dist/packages/<name>` for an ordinary
/// package, and the root itself when a package spreads across `dist/` (a font
/// package installing both `dist/fonts/<name>/…` and a flat
/// `dist/hash/<name>.satysfi-hash`, say).
fn install_path(root: &std::path::Path, receipt: &receipts::Receipt) -> std::path::PathBuf {
    let mut common: Option<Vec<&str>> = None;
    for file in &receipt.files {
        let dirs: Vec<&str> = file.dst.split('/').filter(|s| !s.is_empty()).collect();
        // The file name itself is not part of the directory.
        let dirs = &dirs[..dirs.len().saturating_sub(1)];
        common = Some(match common {
            None => dirs.to_vec(),
            Some(prev) => prev
                .into_iter()
                .zip(dirs.iter())
                .take_while(|(a, b)| a == *b)
                .map(|(a, _)| a)
                .collect(),
        });
    }
    match common {
        Some(parts) if !parts.is_empty() => root.join(parts.join("/")),
        _ => root.to_path_buf(),
    }
}
