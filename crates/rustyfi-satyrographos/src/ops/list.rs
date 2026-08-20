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
}

/// List every installed package, sorted by name.
pub fn list(opts: &RootOptions) -> Result<Vec<PackageSummary>, Error> {
    let root = opts.resolve_root()?;
    let receipts = receipts::list_all(&root)?;
    Ok(receipts
        .into_iter()
        .map(|r| PackageSummary {
            lang: r.lang,
            name: r.name,
            version: r.package_version,
            file_count: r.files.len(),
        })
        .collect())
}
