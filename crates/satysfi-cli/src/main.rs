//! The chimera CLI (plan §1/§4/§7.3): a single multicall
//! (busybox-/rustup-style) binary that behaves as three tools, dispatched on
//! its `argv[0]` basename and on its first subcommand:
//!
//! - `satysfi-rust` — the compiler (default) plus the `satyrographos` and
//!   `multicall` subcommand trees;
//! - `satysfi` — the compiler only;
//! - `satyrographos` — the package manager only.
//!
//! Package-management logic lives in the clap-free `satysfi-satyrographos`
//! crate; this file only parses arguments, resolves `lib_root`/`dest`, and
//! calls in. The compile path (`cmd_compile`) is byte-for-byte the old
//! `main`: positional input, `--output`, `--lib-root` with upward
//! `lib-satysfi/` discovery, and `--target-version` with header sniffing.

use std::path::{Path, PathBuf};

use clap::ArgMatches;

mod dispatch;

fn main() {
    let code = run();
    std::process::exit(code);
}

/// Exit codes (plan §4): `0` success; `2` clap usage error (clap exits `2`
/// itself on parse failure); `3` root resolution; `4` receipt collision /
/// not-installed; `5` filesystem/archive/manifest; `1` compile error or a
/// `status` mismatch.
fn run() -> i32 {
    let matches = dispatch::build_cli().get_matches();

    // `multicall(true)` turns argv[0]'s basename into the top-level
    // subcommand: `satysfi-rust` | `satysfi` | `satyrographos`.
    match matches.subcommand() {
        Some(("satysfi-rust", m)) => match m.subcommand() {
            Some(("satyrographos", sm)) => run_satyrographos(sm),
            Some(("multicall", sm)) => run_multicall(sm),
            _ => run_compile(m),
        },
        // The `satysfi` personality is compile-only (no nested subcommands).
        Some(("satysfi", m)) => run_compile(m),
        // Bare `satyrographos` personality.
        Some(("satyrographos", sm)) => run_satyrographos(sm),
        // Unreachable: clap requires one of the personalities.
        _ => {
            eprintln!("error: no command given");
            2
        }
    }
}

// ---------------------------------------------------------------------------
// Compile mode (unchanged behavior from the pre-chimera `main`).
// ---------------------------------------------------------------------------

fn run_compile(m: &ArgMatches) -> i32 {
    match cmd_compile(m) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("Error: {e:#}");
            1
        }
    }
}

/// The former `main` body, verbatim in behavior: load the document through
/// the multi-file loader, merge preludes, compile, render, and write the PDF.
fn cmd_compile(m: &ArgMatches) -> anyhow::Result<()> {
    use anyhow::Context as _;

    let input = m
        .get_one::<PathBuf>("input")
        .expect("input is required by clap")
        .clone();
    let output = m
        .get_one::<PathBuf>("output")
        .cloned()
        .unwrap_or_else(|| input.with_extension("pdf"));
    let lib_root = m
        .get_one::<PathBuf>("lib_root")
        .cloned()
        .or_else(|| std::env::var_os("SATYSFI_LIB_ROOT").map(PathBuf::from))
        .or_else(|| discover_lib_root(&input));

    let target_version = m.get_one::<String>("target_version").map(String::as_str);
    let version = resolve_version(target_version, &input)?;

    let program =
        satysfi_loader::load(&input, &satysfi_loader::LoadOptions { lib_root, version })
            .map_err(|e| anyhow::anyhow!("{e}"))?;
    let merged = merge_program(program);

    let metrics = satysfi_pdf::Base14Metrics;
    let doc = satysfi_lang::compile_document_cst(&merged, &metrics)
        .map_err(|e| anyhow::anyhow!("{}: {e}", input.display()))?;

    let bytes = satysfi_pdf::render_pdf(&doc.geometry, &doc.pages)?;
    std::fs::write(&output, bytes)
        .with_context(|| format!("cannot write {}", output.display()))?;

    let line_count: usize = doc.pages.iter().map(|p| p.lines.len()).sum();
    eprintln!(
        " ---- ---- ---- ----\n  output written on {} ({} page(s), {} line(s)).",
        output.display(),
        doc.pages.len(),
        line_count
    );
    Ok(())
}

/// The CLI's default `--lib-root` rule (used only when neither `--lib-root`
/// nor `$SATYSFI_LIB_ROOT` is given): starting at `input`'s own directory,
/// walk upward through its ancestors looking for a `lib-satysfi/`
/// subdirectory, returning the first one found. This is the simplest rule
/// that makes `satysfi-rust some/nested/doc.saty` "just work" from anywhere
/// inside a checkout that has one top-level `lib-satysfi/` (this repo
/// included), with no flag or environment variable needed, while still
/// resolving relative to the *document*, not the current working directory
/// (so it behaves the same regardless of where the command is run from).
fn discover_lib_root(input: &std::path::Path) -> Option<PathBuf> {
    let mut dir = input.parent()?.to_path_buf();
    loop {
        let candidate = dir.join("lib-satysfi");
        if candidate.is_dir() {
            return Some(candidate);
        }
        if !dir.pop() {
            return None;
        }
    }
}

/// Combine the `--target-version` flag with best-effort version detection
/// from the input's header lines (`satysfi_syntax::sniff_version`):
///
/// - flag given, sniffer disagrees: warn to stderr, obey the flag;
/// - flag given: obey it (the loader still rejects unimplemented versions);
/// - no flag, sniffer detects an unimplemented version: fail now with a
///   hint, rather than confuse the user with a 0.0.6 parse error;
/// - otherwise: the default, 0.0.6.
fn resolve_version(
    flag: Option<&str>,
    input: &Path,
) -> anyhow::Result<satysfi_syntax::SatysfiVersion> {
    use satysfi_syntax::SatysfiVersion;

    let flag = flag
        .map(str::parse::<SatysfiVersion>)
        .transpose()
        .map_err(|e| anyhow::anyhow!("--target-version: {e}"))?;
    // Sniffing is advisory only: if the file is unreadable, let the loader
    // report the I/O error on its own terms.
    let sniffed = std::fs::read_to_string(input)
        .ok()
        .and_then(|src| satysfi_syntax::sniff_version(&src));

    match (flag, sniffed) {
        (Some(v), Some(s)) if s != v => {
            eprintln!(
                "warning: {} looks like a SATySFi {s} document, but --target-version {v} \
                 was given; proceeding as {v}",
                input.display()
            );
            Ok(v)
        }
        (Some(v), _) => Ok(v),
        (None, Some(s)) if !s.is_implemented() => Err(anyhow::anyhow!(
            "{}: SATySFi {s} documents are not supported yet; supported: 0.0.6 \
             (detected a 0.1-style `use` header; pass `--target-version 0.0.6` to \
             force 0.0.6 interpretation)",
            input.display()
        )),
        (None, _) => Ok(SatysfiVersion::DEFAULT),
    }
}

/// Concatenate the dependency-ordered library preludes ahead of the entry
/// document's own prelude, producing one synthetic file for elaboration.
/// (The v0.0.6 analog type-checks each library into a shared environment in
/// dependency order; untyped elaboration gets the same scoping by prelude
/// concatenation.)
fn merge_program(program: satysfi_loader::LoadedProgram) -> satysfi_syntax::cst::File {
    let mut files = program.files;
    let entry = files.pop().expect("loader always yields the entry last");
    let mut prelude = Vec::new();
    for lib in files {
        prelude.extend(lib.cst.prelude);
    }
    prelude.extend(entry.cst.prelude);
    satysfi_syntax::cst::File {
        headers: Vec::new(),
        prelude,
        in_kw: entry.cst.in_kw,
        body: entry.cst.body,
        eoi: entry.cst.eoi,
    }
}

// ---------------------------------------------------------------------------
// Package-manager mode (satyrographos <cmd>).
// ---------------------------------------------------------------------------

use satysfi_satyrographos as sg;

/// Map a `satysfi_satyrographos::Error` to the plan's §4 exit codes.
fn sg_exit_code(err: &sg::Error) -> i32 {
    use sg::Error::*;
    match err {
        // Nothing to operate on (no root, no Satyrfile, or no registry config).
        RootResolution | SatyrfileNotFound | NoRegistry => 3,
        // Not-found: a missing receipt, or a package/version absent from the index.
        AlreadyInstalled { .. } | NotInstalled { .. } | PackageNotFound { .. }
        | VersionNotFound { .. } => 4,
        // Library-selection / filter usage errors (plan §4.1).
        LibraryFilter { .. } | AmbiguousLibrary { .. } => 2,
        // Filesystem / archive / manifest / Satyristes / lockfile / registry-
        // fetch / integrity failures.
        Io { .. } | Manifest { .. } | Receipt { .. } | UnmanagedCollision { .. }
        | PathTraversal { .. } | UnknownSource { .. } | EmptySource { .. } | Archive(_)
        | MissingDst { .. } | Satyristes { .. } | AmbiguousSource { .. }
        | Satyrfile { .. } | Lockfile { .. } | UnsupportedSource { .. }
        | GitFailed { .. } | RegistryIndex { .. } | ChecksumMismatch { .. }
        | HttpDisabled { .. } | HttpFailed { .. } => 5,
    }
}

/// Finish a package-manager or multicall handler: success is exit `0`; an
/// error is reported to stderr under the shared lowercase `error: {e}`
/// convention and mapped to an exit code by `code` (`sg_exit_code` for the
/// package manager, a constant `1` for multicall). The compile path keeps its
/// own `Error: {e:#}` reporting and is deliberately not routed through here.
fn finish<E: std::fmt::Display>(result: Result<(), E>, code: impl FnOnce(&E) -> i32) -> i32 {
    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("error: {e}");
            code(&e)
        }
    }
}

fn run_satyrographos(m: &ArgMatches) -> i32 {
    let result = match m.subcommand() {
        Some(("install", sm)) => cmd_install(sm),
        Some(("uninstall", sm)) => cmd_uninstall(sm),
        Some(("list", sm)) => cmd_list(sm),
        Some(("status", sm)) => return cmd_status(sm),
        Some(("search", sm)) => cmd_search(sm),
        Some(("update", sm)) => cmd_update(sm),
        _ => {
            eprintln!("error: no satyrographos subcommand given");
            return 2;
        }
    };
    finish(result, sg_exit_code)
}

fn root_options(m: &ArgMatches) -> sg::RootOptions {
    sg::RootOptions {
        lib_root: m.get_one::<PathBuf>("lib_root").cloned(),
        dest: m.get_one::<PathBuf>("dest").cloned(),
    }
}

/// Registry options from the shared `--registry` flag (plan §5.4 step 1). The
/// cache dir / refresh come from `$SATYSFI_REGISTRY_CACHE` and each command's
/// own semantics; `update` sets `refresh` itself.
fn registry_options(m: &ArgMatches) -> sg::RegistryOptions {
    sg::RegistryOptions {
        url: m.get_one::<String>("registry").cloned(),
        ..Default::default()
    }
}

/// The nearest `Satyrfile.toml`, searched upward from the current directory
/// (plan §5.3), or `None` if there is none. Callers map the `None` case to the
/// exit-`3` [`sg::Error::SatyrfileNotFound`]. Shared by manifest-mode `install`
/// and by `update`.
fn find_manifest() -> Option<PathBuf> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    sg::find_upward(&cwd)
}

fn cmd_install(m: &ArgMatches) -> Result<(), sg::Error> {
    // No PATH → phase-2 manifest mode (reconcile the nearest Satyrfile.toml).
    let Some(arg) = m.get_one::<String>("path") else {
        return cmd_install_manifest(m);
    };
    let libraries: Option<Vec<String>> = m
        .get_many::<String>("library")
        .map(|vals| vals.cloned().collect());
    let sg::RootOptions { lib_root, dest } = root_options(m);
    let opts = sg::InstallOptions {
        lib_root,
        dest,
        libraries,
        force: m.get_flag("force"),
    };

    // Registry form (plan §5.4): the argument is a registry NAME[@VERSION] when
    // it does not name a path on disk. A path on disk is a phase-1 install.
    let report = if Path::new(arg).exists() {
        sg::install(Path::new(arg), &opts)?
    } else {
        let (name, version) = split_name_version(arg);
        let reg_opts = registry_options(m);
        // Registry-URL precedence (plan §5.4 / this port's [registry] section):
        // --registry flag > $SATYSFI_REGISTRY > the nearest Satyrfile's
        // [registry] url. The first two live in `reg_opts`; supply the third as
        // the fallback so a project with a declared registry needs no flag.
        let fallback = nearest_registry_url();
        let (report, resolved) =
            sg::install_registry(name, version, &opts, &reg_opts, fallback.as_deref())?;
        eprintln!("fetched {name} {} from registry", resolved.version);
        report
    };
    println!(
        "installed {} {} ({} path(s)):",
        report.name,
        report.version,
        report.files.len()
    );
    for file in &report.files {
        println!("  {}", file.display());
    }
    Ok(())
}

/// Split a registry `NAME[@VERSION]` argument into `(name, Some(version))` or
/// `(name, None)`. A trailing `@` with no version (`"foo@"`) yields
/// `("foo", None)`, never a name with a stray `@`.
fn split_name_version(arg: &str) -> (&str, Option<&str>) {
    match arg.split_once('@') {
        Some((name, version)) if !version.is_empty() => (name, Some(version)),
        Some((name, _)) => (name, None),
        None => (arg, None),
    }
}

/// The `[registry] url` of the nearest `Satyrfile.toml` (searched upward from
/// the working directory), or `None` if there is none / it declares no url.
/// Used as the lowest-precedence registry-URL fallback for a direct
/// `install NAME[@VERSION]` (parse failures are silently ignored — a broken
/// manifest simply provides no fallback).
fn nearest_registry_url() -> Option<String> {
    let cwd = std::env::current_dir().ok()?;
    let manifest = sg::find_upward(&cwd)?;
    let satyrfile = sg::satyrfile::read(&manifest).ok()?;
    satyrfile.registry_url().map(str::to_owned)
}

/// Phase-2 manifest mode (plan §5.3, §8): locate `Satyrfile.toml` by upward
/// search from the current directory, reconcile it against `Satyrfile.lock`
/// and the installed receipts, and re-materialise only the changed/missing
/// entries. When no `--lib-root`/`--dest`/`$SATYSFI_LIB_ROOT` is given, default
/// the root to a `lib-satysfi/` sibling of the manifest if one exists (§3:
/// "Satyrfile.toml — sibling to lib-satysfi/").
fn cmd_install_manifest(m: &ArgMatches) -> Result<(), sg::Error> {
    let manifest = find_manifest().ok_or(sg::Error::SatyrfileNotFound)?;

    let sg::RootOptions { mut lib_root, dest } = root_options(m);
    if lib_root.is_none() && dest.is_none() && std::env::var_os("SATYSFI_LIB_ROOT").is_none() {
        if let Some(dir) = manifest.parent() {
            let sibling = dir.join("lib-satysfi");
            if sibling.is_dir() {
                lib_root = Some(sibling);
            }
        }
    }
    let opts = sg::RootOptions { lib_root, dest };

    let report = sg::install_manifest_reg(&manifest, &opts, &registry_options(m))?;
    println!("reconciled {}", manifest.display());
    for ir in &report.installed {
        println!("  installed {} {}", ir.name, ir.version);
    }
    for name in &report.skipped {
        println!("  unchanged {name}");
    }
    for name in &report.removed {
        println!("  dropped {name} (left installed; not pruned)");
    }
    if report.installed.is_empty() && report.skipped.is_empty() {
        println!("  (no libraries declared)");
    }
    Ok(())
}

fn cmd_uninstall(m: &ArgMatches) -> Result<(), sg::Error> {
    let name = m.get_one::<String>("name").expect("NAME is required by clap");
    sg::uninstall(name, &root_options(m))?;
    println!("uninstalled {name}");
    Ok(())
}

fn cmd_list(m: &ArgMatches) -> Result<(), sg::Error> {
    let packages = sg::list(&root_options(m))?;
    if packages.is_empty() {
        println!("(no packages installed)");
    } else {
        for pkg in &packages {
            println!("{} {} ({} files)", pkg.name, pkg.version, pkg.file_count);
        }
    }
    Ok(())
}

/// `search <term>` (plan §8): list matching registry packages, one
/// `name version — description` line each, sorted by name.
fn cmd_search(m: &ArgMatches) -> Result<(), sg::Error> {
    let term = m.get_one::<String>("term").expect("TERM is required by clap");
    let hits = sg::search(term, &registry_options(m), None)?;
    if hits.is_empty() {
        println!("(no matching packages)");
        return Ok(());
    }
    for hit in &hits {
        match &hit.description {
            Some(desc) => println!("{} {} — {desc}", hit.name, hit.version),
            None => println!("{} {}", hit.name, hit.version),
        }
    }
    Ok(())
}

/// `update` (plan §8, §5.4 step 1): re-fetch the index and report available
/// upgrades against the nearest `Satyrfile.lock` (does not apply them).
fn cmd_update(m: &ArgMatches) -> Result<(), sg::Error> {
    let manifest = find_manifest().ok_or(sg::Error::SatyrfileNotFound)?;
    let report = sg::update(&manifest, &registry_options(m))?;

    if let Some(commit) = &report.commit {
        println!("index at {commit}");
    }
    if report.upgrades.is_empty() {
        println!("all registry dependencies up to date");
    } else {
        for up in &report.upgrades {
            println!("{}: {} -> {} available", up.name, up.current, up.latest);
        }
    }
    Ok(())
}

/// `status` maps a missing-files result to exit `1` (plan §4.4), so it
/// returns a code directly rather than through the shared `Ok/Err` path.
fn cmd_status(m: &ArgMatches) -> i32 {
    let name = m.get_one::<String>("name").map(String::as_str);
    let report = match sg::status(name, &root_options(m)) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            return sg_exit_code(&e);
        }
    };

    if name.is_some() {
        // Full presence report for the single named package.
        for pkg in &report.packages {
            println!("{} {}", pkg.name, pkg.version);
            for path in &pkg.missing_files {
                println!("  MISSING: {}", path.display());
            }
            if pkg.missing_files.is_empty() {
                println!("  {} file(s) present", pkg.total_files);
            }
        }
    } else {
        for pkg in &report.packages {
            println!(
                "{}: {}/{} files present",
                pkg.name,
                pkg.present_files(),
                pkg.total_files
            );
        }
        if report.packages.is_empty() {
            println!("(no packages installed)");
        }
    }

    if report.any_missing() {
        1
    } else {
        0
    }
}

// ---------------------------------------------------------------------------
// `satysfi-rust multicall install --dir DIR` (plan §4.5).
// ---------------------------------------------------------------------------

fn run_multicall(m: &ArgMatches) -> i32 {
    match m.subcommand() {
        Some(("install", sm)) => finish(multicall_install(sm), |_| 1),
        _ => {
            eprintln!("error: no multicall subcommand given");
            2
        }
    }
}

/// Hardlink (falling back to copy on cross-device `EXDEV`) the running
/// executable to `<DIR>/satysfi` and `<DIR>/satyrographos`, so those names
/// become opt-in aliases of this multicall binary. Refuses to clobber a
/// target that is not already a link/copy of this exe.
fn multicall_install(m: &ArgMatches) -> anyhow::Result<()> {
    use anyhow::Context as _;

    let dir = m.get_one::<PathBuf>("dir").expect("--dir is required by clap");
    let exe = std::env::current_exe().context("cannot locate the current executable")?;

    std::fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;

    for alias in ["satysfi", "satyrographos"] {
        let target = dir.join(alias);
        if target.exists() {
            // Only overwrite if it already *is* this exe (a prior alias) —
            // compared by identity (same file), which hardlinks share even
            // though their paths differ.
            if !is_same_file(&exe, &target) {
                anyhow::bail!(
                    "{} already exists and is not this executable; refusing to overwrite",
                    target.display()
                );
            }
            std::fs::remove_file(&target)
                .with_context(|| format!("cannot replace {}", target.display()))?;
        }
        link_or_copy(&exe, &target)
            .with_context(|| format!("cannot install alias {}", target.display()))?;
        println!("installed alias {}", target.display());
    }
    Ok(())
}

/// Whether `a` and `b` are the *same* file. On unix that means identical
/// `(device, inode)` — true for a hardlink alias even though its path
/// differs. Elsewhere, fall back to comparing canonical paths.
fn is_same_file(a: &Path, b: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        if let (Ok(ma), Ok(mb)) = (std::fs::metadata(a), std::fs::metadata(b)) {
            return ma.dev() == mb.dev() && ma.ino() == mb.ino();
        }
        false
    }
    #[cfg(not(unix))]
    {
        match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
            (Ok(ca), Ok(cb)) => ca == cb,
            _ => false,
        }
    }
}

/// Try a hard link first; on any failure (e.g. `EXDEV` across filesystems)
/// fall back to a plain copy.
fn link_or_copy(from: &Path, to: &Path) -> std::io::Result<()> {
    match std::fs::hard_link(from, to) {
        Ok(()) => Ok(()),
        Err(_) => std::fs::copy(from, to).map(|_| ()),
    }
}
