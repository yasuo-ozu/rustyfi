//! `install` (plan §4.1, §6): resolve root → prepare source (dir or
//! `.tar.gz`) → discover plan (manifest-first, flat-copy fallback) → stage
//! with path-traversal guard → collision check → atomic swap → write
//! receipt.

use std::path::{Path, PathBuf};

use crate::error::Error;
use crate::receipts::{self, FileEntry, Receipt, Source, SCHEMA_VERSION};
use crate::roots::RootSelection;
use crate::util;
use crate::{archive, manifest, stage};

/// Options for [`install`] (plan §7.2).
#[derive(Debug, Default, Clone)]
pub struct InstallOptions {
    /// The library to prefer when the manifest declares several and no
    /// `--library` filter was given: asking for package `xpath` and being told
    /// it declares `xpath` and `xpath-gr` is friction the user cannot act on
    /// without reading the manifest. A name that matches nothing is ignored,
    /// so this never turns a working install into a failing one.
    pub prefer_library: Option<String>,
    /// Refuse to fetch an `.opam` `extra-source` that is not already present.
    pub offline: bool,
    /// Echo each fetch and build command an `.opam` asks for. They run a
    /// program and reach the network, so by default the user sees them.
    pub verbose: bool,
    /// Restrict to blocks written for this generation. `None` accepts either,
    /// which is only unambiguous when the name occurs once.
    pub lang: Option<manifest::Lang>,
    pub lib_root: Option<PathBuf>,
    pub dest: Option<PathBuf>,
    /// `-l`/`--library NAME` filter (repeatable). `None` means no filter.
    pub libraries: Option<Vec<String>>,
    pub force: bool,
}

impl RootSelection for InstallOptions {
    fn lib_root(&self) -> Option<&Path> {
        self.lib_root.as_deref()
    }
    fn dest(&self) -> Option<&Path> {
        self.dest.as_deref()
    }
}

/// What [`install`] materialised.
#[derive(Debug)]
pub struct InstallReport {
    /// What the package's `.opam` preparation fetched and ran, if anything.
    pub prepared: crate::ops::prepare::PrepareReport,
    pub name: String,
    pub version: String,
    /// Distinct top-level destination subtrees (relative to the library
    /// root), sorted — one line per entry when the CLI prints them.
    pub files: Vec<PathBuf>,
}

/// Install the package at `source` (a directory or `.tar.gz`).
pub fn install(source: &Path, opts: &InstallOptions) -> Result<InstallReport, Error> {
    install_inner(source, opts, None)
}

/// Install from an `http(s)://` URL naming a `.tar.gz`: fetch it, then install
/// it exactly as a local archive, and record the URL in the receipt so `list`
/// and `status` say where the package actually came from rather than naming a
/// temporary file that no longer exists.
///
/// **A bare URL is unverified.** A registry install checks the index's
/// checksum and an `.opam` `extra-source` checks its `sha256=`; a URL typed on
/// a command line carries no such claim, and this must not be mistaken for the
/// same guarantee. Pass `sha256` to demand one — the caller is expected to say
/// out loud when it cannot.
pub fn install_url(
    url: &str,
    sha256: Option<&str>,
    opts: &InstallOptions,
) -> Result<InstallReport, Error> {
    if opts.offline {
        return Err(Error::Offline {
            url: url.to_string(),
        });
    }
    let dir = std::env::temp_dir().join(format!("rustyfi-url-{}", crate::stage::unique_suffix()));
    std::fs::create_dir_all(&dir).map_err(|e| Error::io(&dir, e))?;
    let archive = dir.join(url_file_name(url));

    let result = (|| {
        crate::registry::http::get_to_file(url, &archive)?;
        if let Some(want) = sha256 {
            let got = util::sha256_file(&archive)?;
            if !got.eq_ignore_ascii_case(want) {
                return Err(Error::ChecksumMismatch {
                    expected: want.to_string(),
                    actual: got,
                });
            }
        }
        let mut source = Source::plain("url", url);
        source.sha256 = sha256.map(str::to_string);
        install_inner(&archive, opts, Some(source))
    })();

    // The download is scratch either way: a failure must not leave a half-
    // fetched archive behind, and a success has already extracted it.
    let _ = std::fs::remove_dir_all(&dir);
    result
}

/// Whether `arg` names a remote archive rather than a path or a registry name.
pub fn is_url(arg: &str) -> bool {
    arg.starts_with("http://") || arg.starts_with("https://")
}

/// The file name to save a URL's download under. Only the extension really
/// matters — [`crate::archive::prepare`] dispatches on it — so a URL whose last
/// segment does not look like a tarball (a redirect endpoint, `.../download`,
/// a trailing slash) still gets a name that says what this crate is willing to
/// unpack, and unpacking fails honestly if it turns out to be something else.
fn url_file_name(url: &str) -> String {
    let path = url
        .split(['?', '#'])
        .next()
        .unwrap_or(url)
        .trim_end_matches('/');
    let last = path.rsplit('/').next().unwrap_or("");
    if last.ends_with(".tar.gz") || last.ends_with(".tgz") {
        last.to_string()
    } else {
        "package.tar.gz".to_string()
    }
}

/// The install pipeline, shared by the phase-1 `path`/`archive` primitive and
/// the phase-3 registry source. `source_override`, when `Some`, replaces the
/// receipt's `[source]` table (the registry form records the package name,
/// version, tarball url, and verified sha256 there instead of a bare local
/// path); when `None`, the receipt records the prepared `path`/`archive`
/// source as before.
pub(crate) fn install_inner(
    source: &Path,
    opts: &InstallOptions,
    source_override: Option<Source>,
) -> Result<InstallReport, Error> {
    let root = opts.resolve_managed_root()?;

    let prepared = archive::prepare(source, &root)?;

    // A package may not carry the files it declares: a font package ships an
    // `extra-source` and a `build:` line that produces them, and its
    // `Satyristes` names paths that only exist afterwards. Run that first, so
    // planning sees the finished tree.
    let prep = crate::ops::prepare::prepare(&prepared.source_root, opts.offline, opts.verbose)?;

    let plans = manifest::discover(&prepared.source_root)?;

    // `-l`/`--library` filter / library selection (plan §4.1). A single
    // `rustyfi-package.toml` or `packages/` fallback yields exactly one plan;
    // a `Satyristes` (phase 4) may declare several `(library ...)` blocks, in
    // which case `--library` must narrow the selection to exactly one (one
    // library is materialised per install).
    let plan = select_plan(
        plans,
        opts.libraries.as_deref(),
        opts.lang,
        opts.prefer_library.as_deref(),
    )?;

    // Collision policy (plan §6).
    let old_receipt = if receipts::exists_for(&root, &plan.name, plan.lang) {
        if !opts.force {
            return Err(Error::AlreadyInstalled { name: plan.name });
        }
        Some(receipts::read_for(&root, &plan.name, plan.lang)?)
    } else {
        // No receipt for this name: refuse to clobber any pre-existing
        // (unmanaged) file at a destination path. A shared destination is
        // exempt — a `fonts.satysfi-hash` already being there is the normal
        // case, since the standard library ships one, and this install adds its
        // entries to it rather than taking it over.
        for pf in plan.files.iter().filter(|pf| !pf.merge) {
            let live = stage::safe_join(&root, &pf.dst)?;
            if live.exists() {
                return Err(Error::UnmanagedCollision { path: live });
            }
        }
        None
    };

    // Stage every file (path-traversal-checked) and hash it.
    let staging = stage::StagingArea::new(&root, &plan.name)?;
    let mut file_entries = Vec::with_capacity(plan.files.len());
    for pf in &plan.files {
        if pf.merge {
            let merged = merge_shared(&root, pf, old_receipt.as_ref())?;
            staging.stage_contents(&pf.dst, &merged.text)?;
            // No `sha256`: the next font package to install merges into this
            // same file and changes it, so a hash recorded here would describe
            // a state that is not meant to hold. `keys` is what this package
            // owns, and the only thing worth recording about a shared file.
            file_entries.push(FileEntry {
                dst: pf.dst.clone(),
                sha256: None,
                keys: Some(merged.keys),
            });
            continue;
        }
        staging.stage(&pf.dst, &pf.src)?;
        let staged_path = stage::safe_join(staging.path(), &pf.dst)?;
        let sha = util::sha256_file(&staged_path)?;
        file_entries.push(FileEntry {
            dst: pf.dst.clone(),
            sha256: Some(sha),
            keys: None,
        });
    }

    let new_dsts: Vec<String> = file_entries.iter().map(|f| f.dst.clone()).collect();
    // Only files this package OWNS may be orphaned. A shared hash file is
    // rewritten from the merge above, which already dropped this package's
    // previous keys; orphaning it would take the other packages' entries with
    // it.
    let old_dsts: Vec<String> = old_receipt
        .as_ref()
        .map(|r| {
            r.files
                .iter()
                .filter(|f| f.keys.is_none())
                .map(|f| f.dst.clone())
                .collect()
        })
        .unwrap_or_default();

    // Atomic swap into place.
    stage::materialize(&root, staging.path(), &new_dsts, &old_dsts)?;

    // A shared file the previous install contributed to and this one no longer
    // ships: its keys still have to go, and the file itself has to stay.
    if let Some(old) = &old_receipt {
        for entry in old.files.iter().filter(|f| f.keys.is_some()) {
            if !new_dsts.contains(&entry.dst) {
                withdraw_keys(&root, entry)?;
            }
        }
    }

    // Record the receipt (after materialisation, so a crash never leaves a
    // receipt pointing at files that were not placed).
    let receipt = Receipt {
        lang: plan.lang,
        schema_version: SCHEMA_VERSION,
        name: plan.name.clone(),
        package_version: plan.version.clone(),
        installed_at: util::now_rfc3339(),
        source: source_override.unwrap_or_else(|| {
            Source::plain(prepared.kind, prepared.value.to_string_lossy().into_owned())
        }),
        files: file_entries,
    };
    receipts::write(&root, &receipt)?;

    Ok(InstallReport {


        prepared: prep,
        name: plan.name,
        version: plan.version,
        files: top_level_paths(&new_dsts),
    })
}

/// A merged shared file: the text to stage, and the keys this package owns in
/// it (recorded in the receipt, so uninstall can take exactly those back out).
struct Merged {
    text: String,
    keys: Vec<String>,
}

/// Merge the package's own `*.satysfi-hash` into what is already at `pf.dst`.
///
/// The result is the live file plus this package's entries — so the standard
/// library's fonts and every other font package's survive an install, which a
/// copy would not allow.
fn merge_shared(
    root: &Path,
    pf: &manifest::PlannedFile,
    old: Option<&Receipt>,
) -> Result<Merged, Error> {
    let read = |path: &Path| -> Result<crate::hashfile::HashFile, Error> {
        let text = util::read_to_string(path)?;
        crate::hashfile::HashFile::parse(&text).map_err(|e| Error::HashFile {
            path: path.to_path_buf(),
            message: e.message,
        })
    };

    let mine = read(&pf.src)?;
    let keys: Vec<String> = mine.keys().map(str::to_string).collect();

    let live_path = stage::safe_join(root, &pf.dst)?;
    let mut merged = if live_path.is_file() {
        read(&live_path)?
    } else {
        crate::hashfile::HashFile::default()
    };

    // A `--force` reinstall: this package's previous keys come out first, so
    // re-adding them is not a conflict with itself.
    if let Some(prev) = old.and_then(|r| r.files.iter().find(|f| f.dst == pf.dst)) {
        if let Some(prev_keys) = &prev.keys {
            merged.remove_keys(prev_keys);
        }
    }

    merged.merge_in(&mine).map_err(|clashes| Error::HashKeyConflict {
        path: live_path,
        keys: clashes.join(", "),
    })?;

    Ok(Merged {
        text: merged.to_text(),
        keys,
    })
}

/// Take one package's keys back out of a shared file, leaving the others. The
/// file goes only when nothing is left in it.
pub(crate) fn withdraw_keys(root: &Path, entry: &FileEntry) -> Result<(), Error> {
    let Some(keys) = &entry.keys else {
        return Ok(());
    };
    let path = stage::safe_join(root, &entry.dst)?;
    if !path.is_file() {
        return Ok(());
    }
    let text = util::read_to_string(&path)?;
    let mut file = crate::hashfile::HashFile::parse(&text).map_err(|e| Error::HashFile {
        path: path.clone(),
        message: e.message,
    })?;
    file.remove_keys(keys);
    if file.is_empty() {
        util::remove_file_if_exists(&path)?;
    } else {
        util::write_atomic(&path, file.to_text().as_bytes())?;
    }
    Ok(())
}

/// Pick the single library to install from the discovered plan(s), honouring
/// the `-l`/`--library` filter (plan §4.1):
///
/// - no filter + exactly one plan → that plan;
/// - filter given → keep plans whose declared name is in the set;
/// - end state must be exactly one plan: zero → [`Error::LibraryFilter`],
///   more than one → [`Error::AmbiguousLibrary`].
fn select_plan(
    plans: Vec<manifest::PackagePlan>,
    libraries: Option<&[String]>,
    lang: Option<manifest::Lang>,
    prefer: Option<&str>,
) -> Result<manifest::PackagePlan, Error> {
    // A name is no longer unique: one manifest may declare the same library
    // for both generations, so the pair (name, lang) is what identifies it.
    let declared: Vec<String> = plans
        .iter()
        .map(|p| format!("{} (lang {})", p.name, p.lang.as_str()))
        .collect();
    let mut selected: Vec<manifest::PackagePlan> = match libraries {
        Some(filter) => plans
            .into_iter()
            .filter(|p| filter.iter().any(|n| n == &p.name))
            .collect(),
        None => plans,
    };
    if let Some(lang) = lang {
        selected.retain(|p| p.lang == lang);
    }
    // Only when it would otherwise be ambiguous, and only if it matches.
    if selected.len() > 1 {
        if let Some(prefer) = prefer {
            if selected.iter().any(|p| p.name == prefer) {
                selected.retain(|p| p.name == prefer);
            }
        }
    }
    match selected.len() {
        1 => Ok(selected.pop().unwrap()),
        0 => Err(Error::LibraryFilter {
            declared: declared.join(", "),
        }),
        _ => Err(Error::AmbiguousLibrary {
            names: selected
                .iter()
                .map(|p| format!("{} (lang {})", p.name, p.lang.as_str()))
                .collect::<Vec<_>>()
                .join(", "),
        }),
    }
}

/// Collapse a flat file list to distinct top-level subtrees: the first three
/// path components (`dist/<category>/<name>`) for nested per-library layouts,
/// or the whole path when shorter (a flat `dist/packages/foo.satyh` or a
/// root-relative `dist/<dst>`).
fn top_level_paths(dsts: &[String]) -> Vec<PathBuf> {
    let mut tops: Vec<PathBuf> = dsts
        .iter()
        .map(|d| {
            let comps: Vec<&str> = d.split('/').filter(|s| !s.is_empty()).collect();
            let take = comps.len().min(3);
            comps[..take].iter().collect::<PathBuf>()
        })
        .collect();
    tops.sort();
    tops.dedup();
    tops
}

#[cfg(test)]
mod url_tests {
    use super::*;

    #[test]
    fn a_url_is_told_apart_from_a_path_and_a_registry_name() {
        assert!(is_url("https://example.org/p.tar.gz"));
        assert!(is_url("http://example.org/p.tar.gz"));
        // A registry name and a local path must not be mistaken for one.
        assert!(!is_url("xpath"));
        assert!(!is_url("./satysfi-xpath"));
        assert!(!is_url("/srv/pkgs/xpath.tar.gz"));
    }

    #[test]
    fn the_download_keeps_a_tarball_name_and_invents_one_otherwise() {
        assert_eq!(url_file_name("https://x/y/pkg-1.0.tar.gz"), "pkg-1.0.tar.gz");
        assert_eq!(url_file_name("https://x/y/pkg.tgz"), "pkg.tgz");
        // Query and fragment are not part of the name.
        assert_eq!(url_file_name("https://x/y/pkg.tar.gz?token=1"), "pkg.tar.gz");
        assert_eq!(
            url_file_name("https://x/y/pkg.tar.gz#sha256=ab"),
            "pkg.tar.gz"
        );
        // A download endpoint that names no file still gets an extension this
        // crate can dispatch on, so unpacking fails honestly rather than on a
        // missing suffix.
        assert_eq!(url_file_name("https://x/api/download"), "package.tar.gz");
        assert_eq!(url_file_name("https://x/y/"), "package.tar.gz");
    }

    #[test]
    fn offline_refuses_before_reaching_the_network() {
        let opts = InstallOptions {
            offline: true,
            dest: Some(std::env::temp_dir().join("rustyfi-url-offline-never-created")),
            ..Default::default()
        };
        let err = install_url("https://example.invalid/p.tar.gz", None, &opts)
            .expect_err("offline must refuse a URL install");
        assert!(matches!(err, Error::Offline { .. }), "got {err}");
    }
}
