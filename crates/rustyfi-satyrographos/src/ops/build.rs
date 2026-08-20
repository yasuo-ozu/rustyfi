//! `satyrographos build` — run a `(libraryDoc ...)`'s own `(build ...)`
//! command lines.
//!
//! Everything else in this crate COPIES files; this is the one operation that
//! runs a program, because that is what a doc target is: a document produced by
//! invoking the typesetter, not a file to install. The commands come from the
//! manifest, so the rule is that the caller supplies the typesetter's path
//! ([`BuildOptions::typesetter`]) and a command line naming `rustyfi` or
//! `satysfi` is rewritten to it — a manifest cannot pick which binary runs, and
//! an unpacked archive builds its own docs without anything on `PATH`.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::Error;
use crate::ops::install::{self, InstallOptions};
use crate::ops::uninstall::RootOptions;
use crate::receipts::Source;
use crate::roots::RootSelection;
use crate::satyristes::{self, DocTarget};

/// Which doc(s) to build, and what to build them with.
#[derive(Default)]
pub struct BuildOptions {
    /// Restrict to these `(libraryDoc (name ...))`s. Empty means "the only one
    /// declared", and refuses when the manifest declares several.
    pub docs: Vec<String>,
    /// The typesetter a `rustyfi`/`satysfi` command line resolves to. `None`
    /// runs the name as written, from `PATH`.
    pub typesetter: Option<PathBuf>,
    /// Print each command before running it.
    pub verbose: bool,
    /// Restrict to doc targets written for this generation. `None` accepts
    /// either, which is only unambiguous when the name occurs once.
    pub lang: Option<crate::manifest::Lang>,
    /// Passed to the build commands as `$RUSTYFI_LIB_ROOT`. A doc almost
    /// always `@require:`s the standard library, and the child resolves its
    /// root from the DOCUMENT's directory — which is inside the package being
    /// documented, where there is usually no root at all. Without this the
    /// caller has to export the variable by hand.
    pub lib_root: Option<PathBuf>,
    /// When set, install each built doc's declared
    /// `(sources ((doc "dst" "src") ...))` products into this root after a
    /// successful build — `dist/doc/<name>/<dst>` (`FileKind::Doc`), the same
    /// destination convention any other manifest's own `doc` sources get
    /// (task: a `(libraryDoc ...)`'s products used to have nowhere to go).
    /// `None` (the default) leaves `build` exactly as before: it runs the
    /// commands and reports which products exist, and installs nothing. A
    /// product the build declares but did not actually write is skipped
    /// rather than failing the install — `products` already surfaces that as
    /// a manifest bug. Re-running a build always replaces its own previous
    /// install (there is no separate `--force`: a doc target is a build
    /// artifact, and rebuilding it is expected to be idempotent).
    pub install: Option<RootOptions>,
}

/// What a build ran, and what it says it produced.
#[derive(Debug)]
pub struct BuildReport {
    pub name: String,
    pub commands: Vec<Vec<String>>,
    /// The files the build says it produces — the `src` side of
    /// `(sources ((doc "dst" "src")))`, since that is the path on disk — each
    /// paired with whether it is actually there afterwards. A build that exits
    /// 0 without writing its product is a manifest bug worth surfacing.
    pub products: Vec<(String, bool)>,
    /// The top-level destinations [`BuildOptions::install`] materialised
    /// (relative to the resolved root), empty when `install` was `None` or no
    /// declared product actually existed to install.
    pub installed: Vec<PathBuf>,
}

/// Run the selected doc target's build commands, in `source_root`.
pub fn build(source_root: &Path, opts: &BuildOptions) -> Result<Vec<BuildReport>, Error> {
    let declared = satyristes::doc_targets(source_root)?;
    let selected = select(declared, &opts.docs, opts.lang)?;

    // The commands run in the doc's directory, so a relative root would be
    // read from THERE — `--lib-root ./lib-rustyfi` means the one beside the
    // caller, not one beside the document being built.
    let explicit = opts
        .lib_root
        .as_ref()
        .map(|p| std::path::absolute(p).unwrap_or_else(|_| p.clone()));

    let mut reports = Vec::new();
    for doc in selected {
        // Commands run in the doc's own directory when it names one; the
        // products it declares stay relative to the manifest.
        let workdir = match &doc.working_directory {
            Some(rel) => source_root.join(rel),
            None => source_root.to_path_buf(),
        };
        let lib_root = explicit.clone().or_else(|| inherited_root(&workdir));
        for line in &doc.build {
            let (program, args) = resolve(line, opts.typesetter.as_deref());
            if opts.verbose {
                eprintln!("  {} {}", program.display(), args.join(" "));
            }
            let mut cmd = Command::new(&program);
            cmd.args(&args).current_dir(&workdir);
            if let Some(root) = &lib_root {
                cmd.env("RUSTYFI_LIB_ROOT", root);
            }
            let status = cmd
                .status()
                .map_err(|e| Error::io(&program, e))?;
            if !status.success() {
                return Err(Error::DocBuild {
                    name: doc.name.clone(),
                    command: line.join(" "),
                    code: status.code(),
                });
            }
        }
        let products = doc
            .sources
            .iter()
            .map(|(_dst, src)| (src.clone(), source_root.join(src).is_file()))
            .collect();

        let installed = match &opts.install {
            Some(root_opts) => install_doc_products(source_root, &doc, root_opts)?,
            None => Vec::new(),
        };

        reports.push(BuildReport {
            name: doc.name,
            commands: doc.build,
            products,
            installed,
        });
    }
    Ok(reports)
}

/// [`BuildOptions::install`]'s step: install whichever of `doc`'s declared
/// products actually exist on disk into `root_opts`' resolved root, via the
/// same [`crate::ops::install::install_plan`] machinery a directory/archive
/// install uses. Returns the top-level destinations materialised (empty when
/// nothing declared actually exists yet).
fn install_doc_products(
    source_root: &Path,
    doc: &DocTarget,
    root_opts: &RootOptions,
) -> Result<Vec<PathBuf>, Error> {
    let present: Vec<(String, String)> = doc
        .sources
        .iter()
        .filter(|(_dst, src)| source_root.join(src).is_file())
        .cloned()
        .collect();
    if present.is_empty() {
        return Ok(Vec::new());
    }
    let plan = satyristes::doc_target_plan(source_root, &doc.name, &doc.version, doc.lang, &present)?;
    let root = root_opts.resolve_managed_root()?;
    let install_opts = InstallOptions {
        lib_root: root_opts.lib_root.clone(),
        dest: root_opts.dest.clone(),
        // A doc target is a build artifact: rebuilding it and reinstalling
        // over the previous run is the whole point, so this never refuses on
        // an existing receipt the way a plain `install` does.
        force: true,
        ..Default::default()
    };
    let source = Source::plain("build", source_root.display().to_string());
    let report = install::install_plan(&root, plan, &install_opts, source)?;
    Ok(report.files)
}

/// The root to hand a build command when the caller named none.
///
/// A doc lives inside the package it documents, which is usually not a root,
/// so the child would otherwise resolve nothing and fail on its first
/// `@require:` — the caller standing in a checkout has the root, and the child
/// cannot see it. Passing the caller's is the fix, with two abstentions:
///
/// - `$RUSTYFI_LIB_ROOT` already set — the child inherits it, and overriding
///   someone's exported root would be worse than useless;
/// - the document's own directory discovers a root — a project that vendors
///   its dependencies must keep them, so its `.rustyfi/` is not overridden by
///   whatever tree the caller happens to be standing in.
fn inherited_root(workdir: &Path) -> Option<PathBuf> {
    if std::env::var_os("RUSTYFI_LIB_ROOT").is_some() {
        return None;
    }
    if !crate::roots::discover_all(workdir).is_empty() {
        return None;
    }
    crate::roots::discover(&std::env::current_dir().ok()?)
}

/// The program to run and its arguments. A command line that names the
/// typesetter — `rustyfi` or upstream's `satysfi`, since manifests written for
/// upstream say the latter — runs the typesetter the caller passed.
fn resolve(line: &[String], typesetter: Option<&Path>) -> (PathBuf, Vec<String>) {
    let program = match (line[0].as_str(), typesetter) {
        ("rustyfi" | "satysfi", Some(path)) => path.to_path_buf(),
        _ => PathBuf::from(&line[0]),
    };
    (program, line[1..].to_vec())
}

/// Mirrors `install`'s `--library` rule: a filter keeps the named targets, and
/// with no filter the manifest must declare exactly one.
fn select(
    declared: Vec<DocTarget>,
    wanted: &[String],
    lang: Option<crate::manifest::Lang>,
) -> Result<Vec<DocTarget>, Error> {
    fn describe(targets: &[DocTarget]) -> String {
        targets
            .iter()
            .map(|d| format!("{} (lang {})", d.name, d.lang.as_str()))
            .collect::<Vec<_>>()
            .join(", ")
    }
    if declared.is_empty() {
        return Err(Error::NoDocTarget);
    }
    let all = describe(&declared);
    // Same name, different generation is legitimate, so narrow before counting.
    let declared: Vec<DocTarget> = match lang {
        Some(lang) => declared.into_iter().filter(|d| d.lang == lang).collect(),
        None => declared,
    };
    if declared.is_empty() {
        return Err(Error::DocFilter { declared: all });
    }
    if wanted.is_empty() {
        if declared.len() > 1 {
            return Err(Error::AmbiguousDoc {
                names: describe(&declared),
            });
        }
        return Ok(declared);
    }
    let picked: Vec<DocTarget> = declared
        .into_iter()
        .filter(|d| wanted.iter().any(|w| w == &d.name))
        .collect();
    if picked.is_empty() {
        return Err(Error::DocFilter { declared: all });
    }
    Ok(picked)
}
