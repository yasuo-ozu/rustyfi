//! `status` (plan §4.4): per-package presence check. With a `NAME`, one
//! package's full file list flagging any missing path; without, a summary
//! per receipt. The CLI exits `1` when anything is missing (CI-friendly).

use std::path::PathBuf;

use crate::error::Error;
use crate::ops::uninstall::RootOptions;
use crate::{receipts, roots, stage};

/// Presence report for one package.
#[derive(Debug, Clone)]
pub struct PackageStatus {
    pub name: String,
    pub version: String,
    /// Total files the receipt records.
    pub total_files: usize,
    /// Files the receipt lists that are no longer present on disk
    /// (root-relative).
    pub missing_files: Vec<PathBuf>,
}

impl PackageStatus {
    /// Files present = total − missing.
    pub fn present_files(&self) -> usize {
        self.total_files - self.missing_files.len()
    }
}

/// Aggregate report for [`status`].
#[derive(Debug)]
pub struct StatusReport {
    pub packages: Vec<PackageStatus>,
}

impl StatusReport {
    /// True when at least one recorded file is missing (CLI exit `1`).
    pub fn any_missing(&self) -> bool {
        self.packages.iter().any(|p| !p.missing_files.is_empty())
    }
}

/// Report status for `name` (if given) or every installed package.
pub fn status(name: Option<&str>, opts: &RootOptions) -> Result<StatusReport, Error> {
    let root = roots::resolve_root(opts.lib_root.as_deref(), opts.dest.as_deref())?;

    let receipts = match name {
        Some(name) => vec![receipts::read(&root, name)?],
        None => receipts::list_all(&root)?,
    };

    let mut packages = Vec::with_capacity(receipts.len());
    for receipt in receipts {
        let mut missing = Vec::new();
        for file in &receipt.files {
            let path = stage::safe_join(&root, &file.dst)?;
            if !path.exists() {
                missing.push(PathBuf::from(&file.dst));
            }
        }
        packages.push(PackageStatus {
            name: receipt.name,
            version: receipt.package_version,
            total_files: receipt.files.len(),
            missing_files: missing,
        });
    }
    Ok(StatusReport { packages })
}
