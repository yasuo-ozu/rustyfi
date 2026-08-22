//! Making a package's declared files exist before it is installed. A font
//! package ships no fonts: `satysfi-fonts-theano` declares an `extra-source`
//! (an upstream zip and its checksum) and a `build:` line that unpacks it, and
//! only then do the paths its `Satyristes` names exist. `prepare` is that
//! step — fetch, verify, build — run by `install` when the source directory
//! carries a `.opam`.
//!
//! Two rules this holds to, because it fetches and executes:
//!
//! - **Verify before use.** An `extra-source` with a `sha256=` checksum is
//!   checked before anything runs, and a mismatch aborts. A file already
//!   present and matching is not re-fetched.
//! - **Never silently skip.** A declared checksum this crate cannot verify, or
//!   an absent one, is reported rather than passed over quietly.

use std::path::Path;
use std::process::Command;

use crate::error::Error;
use crate::opam::{self, Opam};
use crate::registry;
use crate::util;

#[derive(Debug, Default)]
pub struct PrepareReport {
    /// Files fetched now, as (name, url).
    pub fetched: Vec<(String, String)>,
    /// Files already present with the right checksum.
    pub reused: Vec<String>,
    pub ran: Vec<Vec<String>>,
    /// Sources whose checksum could not be verified, as (name, why).
    pub unverified: Vec<(String, String)>,
    /// `build:` lines that hand the job to Satyrographos or OPAM, which this
    /// port does itself — recorded rather than run.
    pub delegated: Vec<Vec<String>>,
}

/// Fetch every `extra-source`, verify what can be verified, then run each
/// `build:` line in `source_root`. A directory with no `.opam`, or one
/// declaring neither field, is a no-op.
pub(crate) fn prepare(source_root: &Path, offline: bool, verbose: bool) -> Result<PrepareReport, Error> {
    // Only the files the manifest's own `(library ... (opam "x.opam"))`
    // claims; a directory with no manifest falls back to whatever `.opam` it
    // holds.
    let declared = crate::satyristes::library_opam_files(source_root);
    let files = if declared.is_empty() {
        opam::opam_files(source_root)
    } else {
        declared
    };
    let mut merged = Opam::default();
    for file in files {
        let one = opam::read(&file)?;
        merged.extra_sources.extend(one.extra_sources);
        merged.build.extend(one.build);
    }
    prepare_with(source_root, &merged, offline, verbose)
}

/// `prepare` against an already-parsed opam — the seam the tests drive.
pub fn prepare_with(
    source_root: &Path,
    opam: &Opam,
    offline: bool,
    verbose: bool,
) -> Result<PrepareReport, Error> {
    let mut report = PrepareReport::default();
    if opam.is_empty() {
        return Ok(report);
    }

    for src in &opam.extra_sources {
        let dest = source_root.join(&src.name);
        match &src.sha256 {
            Some(want) if dest.is_file() && &util::sha256_file(&dest)? == want => {
                report.reused.push(src.name.clone());
                continue;
            }
            _ => {}
        }
        if offline {
            return Err(Error::Offline {
                url: src.url.clone(),
            });
        }
        if verbose {
            eprintln!("  fetch {} <- {}", src.name, src.url);
        }
        registry::http::get_to_file(&src.url, &dest)?;

        match &src.sha256 {
            Some(want) => {
                let got = util::sha256_file(&dest)?;
                if &got != want {
                    // A file that failed its checksum must not be left where
                    // the build would pick it up.
                    let _ = std::fs::remove_file(&dest);
                    return Err(Error::ChecksumMismatch {
                        expected: want.clone(),
                        actual: got,
                    });
                }
            }
            None => report
                .unverified
                .push((src.name.clone(), "no sha256 checksum declared".to_string())),
        }
        report.fetched.push((src.name.clone(), src.url.clone()));
    }

    for line in &opam.build {
        // `satyrographos opam install/build …` is OPAM handing the job to
        // Satyrographos. This port IS that half, so running the delegation
        // would either fail (no such program) or recurse.
        if matches!(line[0].as_str(), "satyrographos" | "opam") {
            report.delegated.push(line.clone());
            continue;
        }
        if verbose {
            eprintln!("  {}", line.join(" "));
        }
        let status = Command::new(&line[0])
            .args(&line[1..])
            .current_dir(source_root)
            .status()
            .map_err(|e| Error::io(Path::new(&line[0]), e))?;
        if !status.success() {
            return Err(Error::OpamBuild {
                command: line.join(" "),
                code: status.code(),
            });
        }
        report.ran.push(line.clone());
    }
    Ok(report)
}
