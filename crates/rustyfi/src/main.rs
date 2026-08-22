#![doc = include_str!("../README.md")]
//!
//! Multicall binary: behaves as `rustyfi` (compiler, default) or
//! `satyrographos` (package manager), dispatched on `argv[0]`'s basename and
//! on the first subcommand.

use std::path::{Path, PathBuf};

use clap::ArgMatches;

mod cache;
mod dispatch;
mod format;
mod man;

/// Read an auxiliary cross-reference table, or an empty one if the file is
/// absent, unreadable, or not the flat `{"key": "value"}` object upstream
/// SATySFi writes. Best-effort: a corrupt or foreign file costs a fixpoint
/// trial, never a wrong render or an error.
fn read_aux(path: &Path) -> rustyfi_lang::crossref::AuxTable {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Default::default();
    };
    match serde_json::from_str::<std::collections::BTreeMap<String, String>>(&text) {
        Ok(t) => t,
        Err(_) => Default::default(),
    }
}

/// Write `aux` back out, as the same flat JSON object upstream SATySFi reads.
/// `AuxTable` is a `BTreeMap`, so the bytes are deterministic. Failure is
/// ignored: a missing aux file only costs a fixpoint trial next time.
fn write_aux(path: &Path, aux: &rustyfi_lang::crossref::AuxTable) {
    if let Ok(text) = serde_json::to_string(aux) {
        let _ = std::fs::write(path, text);
    }
}

fn main() {
    // The recursive-descent parser + elaborator use deep stacks on real
    // documents (the ~300-line official SATySFi demo overflows the default
    // 8 MB main-thread stack), hence the large worker-thread stack below.
    let code = std::thread::Builder::new()
        .name("rustyfi-main".into())
        .stack_size(256 * 1024 * 1024)
        .spawn(run)
        .expect("failed to spawn the main worker thread")
        .join()
        .expect("the main worker thread panicked");
    std::process::exit(code);
}

/// Exit codes: `0` success; `2` clap usage error (clap exits `2`
/// itself on parse failure); `3` root resolution; `4` receipt collision /
/// not-installed; `5` filesystem/archive/manifest; `1` compile error or a
/// `status` mismatch.
fn run() -> i32 {
    // `dispatch::get_matches` (not a bare `build_cli().get_matches()`) so a
    // global flag given BEFORE the subcommand (`rustyfi --config F install
    // NAME`) works the same as after it.
    let matches = dispatch::get_matches();

    match matches.subcommand() {
        Some(("rustyfi", m)) => match m.subcommand() {
            Some((name, sm)) if is_package_command(name) => run_package(name, sm),
            Some(("multicall", sm)) => run_multicall(sm),
            Some(("man", _)) => match man::render(&mut std::io::stdout().lock()) {
                Ok(()) => 0,
                Err(e) => {
                    eprintln!("error: writing the man page: {e}");
                    1
                }
            },
            _ => run_compile(m),
        },
        Some(("satyrographos", sm)) => run_satyrographos(sm),
        // Unreachable: clap requires one of the personalities.
        _ => {
            eprintln!("error: no command given");
            2
        }
    }
}

fn run_compile(m: &ArgMatches) -> i32 {
    match cmd_compile(m) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("Error: {e:#}");
            1
        }
    }
}

fn cmd_compile(m: &ArgMatches) -> anyhow::Result<()> {
    use anyhow::Context as _;

    let input = m
        .get_one::<PathBuf>("input")
        .expect("input is required by clap")
        .clone();
    // `--format` has a clap `.default_value("pdf")`, so this `get_one` is
    // always `Some`.
    let format: format::OutputFormat = m
        .get_one::<String>("format")
        .expect("--format has a clap default")
        .parse()
        .map_err(|e: String| anyhow::anyhow!(e))?;
    let output = m
        .get_one::<PathBuf>("output")
        .cloned()
        .unwrap_or_else(|| input.with_extension(format.extension()));
    // A NAMED root is exactly that one root — no discovered roots appended
    // behind it, or a build would depend on what happens to be installed on
    // the machine. Discovery instead supplies the WHOLE chain when nothing
    // is named.
    let named = m
        .get_one::<PathBuf>("lib_root")
        .cloned()
        .or_else(|| std::env::var_os("RUSTYFI_LIB_ROOT").map(PathBuf::from));
    let (lib_root, fallback_roots) = match named {
        Some(root) => (Some(root), Vec::new()),
        None => {
            let mut chain = discover_lib_roots(&input).into_iter();
            (chain.next(), chain.collect())
        }
    };

    let lang = m.get_one::<String>("lang").map(String::as_str);
    let deps_flag = m.get_one::<PathBuf>("deps").map(PathBuf::as_path);
    let (version, mode) = resolve_version_and_mode(lang, deps_flag, &input)?;
    // Whether this compile is package-manager
    // driven (Envelopes/manifest mode) — decides below whether a sibling
    // `Satyristes.lock`'s digest folds into the cache key. Captured before
    // `mode` is moved into `LoadOptions`.
    let is_envelopes_mode = matches!(mode, rustyfi_loader::LoadMode::Envelopes { .. });

    // `--timing` (or RUSTYFI_TIMING=1): propagates to the library phases via
    // the env var, and forces `--no-cache` below so every phase actually
    // runs rather than a cache hit.
    let timing = m.get_flag("timing") || std::env::var_os("RUSTYFI_TIMING").is_some();
    if timing {
        std::env::set_var("RUSTYFI_TIMING", "1");
    }
    let t_load = std::time::Instant::now();

    let program = rustyfi_loader::load(
        &input,
        &rustyfi_loader::LoadOptions {
            lib_root: lib_root.clone(),
            fallback_roots: fallback_roots.clone(),
            version,
            mode,
        },
    )
    .map_err(|e| anyhow::anyhow!("{e}"))?;
    if timing {
        eprintln!(
            "TIMING load(lex+parse)   {:>8.1}ms",
            t_load.elapsed().as_secs_f64() * 1e3
        );
    }

    let font_store = resolve_font_store(m, lib_root.as_deref())?;

    // Fold the resolved `Satyristes`/lock digest
    // into the cache key, so a `saphe update`/reconcile that changes a
    // locked package's version invalidates the cache even when the entry
    // document's own bytes did not change.
    let deps_lock_digest: Option<String> = is_envelopes_mode
        .then(|| discover_deps_lock_digest(&input))
        .flatten();

    // Content-addressed compile cache. Caching is ON by default; `--no-cache`
    // disables both read and write. The key is computed from the program
    // *before* the expensive compile+render, so a hit skips them entirely.
    let cache = if m.get_flag("no_cache") || timing {
        None
    } else {
        cache::Cache::open(m.get_one::<PathBuf>("cache_dir").cloned())
    };
    let cache_key = cache.as_ref().and_then(|_| {
        cache::compute_key(
            &program,
            env!("CARGO_PKG_VERSION"),
            version,
            &input,
            font_store.as_ref(),
            format,
            deps_lock_digest.as_deref(),
        )
    });
    if let (Some(cache), Some(key)) = (&cache, &cache_key) {
        if let Some(hit) = cache.get(key) {
            std::fs::write(&output, &hit.pdf)
                .with_context(|| format!("cannot write {}", output.display()))?;
            eprintln!(
                " ---- ---- ---- ----\n  \
                 output written on {} ({} page(s), {} line(s)) (cached).",
                output.display(),
                hit.pages,
                hit.lines
            );
            return Ok(());
        }
    }

    let base14 = rustyfi_pdf::Base14Metrics;
    let metrics: &dyn rustyfi_backend::FontMetrics = match &font_store {
        Some(store) => store,
        None => &base14,
    };
    // Auxiliary cross-reference file (interoperates with upstream's
    // `<doc>.satysfi-aux`). Unlike the compile cache, NOT forced off by
    // `--timing`: it changes how many fixpoint trials are needed, not which
    // phases run, so a profiling run still wants it (use `--no-aux` for a
    // cold measurement). Cannot change the output: a seeded value the final
    // trial didn't re-derive forces a cold redo
    // (`CrossRefs::seed_unvalidated`).
    let aux_path: Option<PathBuf> = if m.get_flag("no_aux") {
        None
    } else {
        Some(
            m.get_one::<PathBuf>("aux_file")
                .cloned()
                .unwrap_or_else(|| input.with_extension("satysfi-aux")),
        )
    };
    let mut aux = aux_path.as_deref().map(read_aux).unwrap_or_default();

    let t_compile = std::time::Instant::now();
    let doc = match version {
        rustyfi_syntax::RustyfiVersion::V0_1 => {
            // 0.1 libraries are modules, not prelude-concatenable flat
            // binding lists — no merge_program; each file keeps its own
            // FileV1 CST.
            rustyfi_lang::compile_document_v1_with_aux(&program.files, metrics, &mut aux)
                .map_err(|e| anyhow::anyhow!("{}: {e}", input.display()))?
                .0
        }
        _ => {
            // A V0_0-rooted load with a foreign V0_1 dependency
            // (a `@require:` into `dist-v01/packages/`, per the loader's
            // per-file version-detection rule) routes through `compile_document_v006_xver`
            // instead of the pure-0.0.6 path — only when such a dependency is
            // present, so a pure-0.0.6 load stays byte-identical.
            let has_v01_dep = program
                .files
                .iter()
                .any(|f| matches!(f.cst, rustyfi_loader::LoadedCst::V0_1(_)));
            if has_v01_dep {
                rustyfi_lang::compile_document_v006_xver_with_aux(&program.files, metrics, &mut aux)
                    .map_err(|e| anyhow::anyhow!("{}: {e}", input.display()))?
                    .0
            } else {
                let (merged, stages) = merge_program(program);
                rustyfi_lang::compile_document_cst_with_stages(&merged, metrics, &mut aux, &stages)
                    .map_err(|e| anyhow::anyhow!("{}: {e}", input.display()))?
                    .0
            }
        }
    };

    if timing {
        eprintln!(
            "TIMING compile(elab+eval) {:>8.1}ms  ({} pages)",
            t_compile.elapsed().as_secs_f64() * 1e3,
            doc.pages.len()
        );
    }
    let t_render = std::time::Instant::now();
    let bytes: Vec<u8> = match format {
        format::OutputFormat::Pdf => match &font_store {
            Some(store) => rustyfi_pdf::render_pdf_ttf_with(
                &doc.geometry,
                &doc.pages,
                store,
                &doc.images,
                &doc.extras,
            )?,
            None => {
                rustyfi_pdf::render_pdf_with(&doc.geometry, &doc.pages, &doc.images, &doc.extras)?
            }
        },
    };
    if timing {
        eprintln!(
            "TIMING render(pdf)        {:>8.1}ms",
            t_render.elapsed().as_secs_f64() * 1e3
        );
    }
    std::fs::write(&output, &bytes)
        .with_context(|| format!("cannot write {}", output.display()))?;

    let line_count: usize = doc.pages.iter().map(|p| p.lines.len()).sum();

    // Written only after a successful render, and only on the compile path
    // — a cache hit returned long before here, leaving the previous run's
    // file exactly as it was.
    if let Some(path) = &aux_path {
        write_aux(path, &aux);
    }

    // Best-effort: a cache-write failure must never fail an otherwise-
    // successful compile.
    if let (Some(cache), Some(key)) = (&cache, &cache_key) {
        let _ = cache.put(key, &bytes, doc.pages.len(), line_count);
    }

    eprintln!(
        " ---- ---- ---- ----\n  output written on {} ({} page(s), {} line(s)).",
        output.display(),
        doc.pages.len(),
        line_count
    );
    Ok(())
}

/// Resolve font configuration, highest precedence first: `--font` (+
/// `--font-bold`/`--font-oblique`) > `--font-dir` > `$RUSTYFI_FONT_DIR` >
/// `lib_root`. `Ok(None)` when nothing is configured anywhere; once a
/// `fonts.satysfi-hash` config IS found, further problems are real errors
/// (see `rustyfi_pdf::fonts`'s module docs).
fn resolve_font_store(
    m: &ArgMatches,
    lib_root: Option<&Path>,
) -> anyhow::Result<Option<rustyfi_pdf::TtfFontStore>> {
    let font_dir = m
        .get_one::<PathBuf>("font_dir")
        .cloned()
        .or_else(|| std::env::var_os("RUSTYFI_FONT_DIR").map(PathBuf::from));
    let flags = rustyfi_pdf::FontFlags {
        regular: m.get_one::<PathBuf>("font").cloned(),
        bold: m.get_one::<PathBuf>("font_bold").cloned(),
        oblique: m.get_one::<PathBuf>("font_oblique").cloned(),
    };

    let registry = rustyfi_pdf::FontRegistry::discover(lib_root, font_dir.as_deref(), &flags)
        .map_err(|e| anyhow::anyhow!("font configuration: {e}"))?;
    let Some(registry) = registry else {
        return Ok(None);
    };
    let store = registry
        .build_store()
        .map_err(|e| anyhow::anyhow!("font configuration: {e}"))?;
    Ok(Some(store))
}

/// The CLI's default `--lib-root`: [`sg::roots::discover_all`], starting at
/// the DOCUMENT's own directory (not the working directory), so
/// `rustyfi some/nested/doc.saty` behaves the same regardless of where the
/// command was run from.
fn discover_lib_roots(input: &std::path::Path) -> Vec<PathBuf> {
    input
        .parent()
        .map(sg::roots::discover_all)
        .unwrap_or_default()
}

/// Nearest `Satyristes`'s `Satyristes.lock`
/// digest, if it has at least one locked entry; `None` otherwise (no
/// manifest, no lock, or an empty one).
fn discover_deps_lock_digest(input: &std::path::Path) -> Option<String> {
    let dir = input.parent()?;
    let manifest_path = rustyfi_satyrographos::find_upward(dir)?;
    let lock_path = rustyfi_satyrographos::lockfile::lock_path_for(&manifest_path);
    let lock = rustyfi_satyrographos::lockfile::read(&lock_path).ok()?;
    if lock.libraries.is_empty() {
        return None;
    }
    Some(lock.digest())
}

/// Resolve BOTH axes of the load — language version (Axis A) and packaging
/// mode (Axis B, `rustyfi_loader::LoadMode`) — from `--lang`, `--deps`, and
/// header sniffing (Ld3a-minimal). Axis A: an explicit `--lang` wins
/// (warning if the sniffer disagrees); otherwise the sniffer's verdict,
/// unless it names an unimplemented version, in which case this fails with a
/// hint; default 0.0.6. Axis B: `--deps` ⇒ `Envelopes { deps: Some(_) }`;
/// else a sniffed `use` header ⇒ `Envelopes { deps: None }`; else `Legacy`.
/// The rejected combination (0.0.6 + Envelopes) is reported here, naming
/// which flag pinned each axis, rather than left to the loader's
/// `InvalidModeVersion` backstop.
fn resolve_version_and_mode(
    flag: Option<&str>,
    deps_flag: Option<&Path>,
    input: &Path,
) -> anyhow::Result<(rustyfi_syntax::RustyfiVersion, rustyfi_loader::LoadMode)> {
    use rustyfi_syntax::RustyfiVersion;

    let flag = flag
        .map(str::parse::<RustyfiVersion>)
        .transpose()
        .map_err(|e| anyhow::anyhow!("--lang: {e}"))?;
    // Sniffing is advisory only: if the file is unreadable, let the loader
    // report the I/O error on its own terms.
    let sniff = std::fs::read_to_string(input)
        .ok()
        .map(|src| rustyfi_syntax::sniff_headers(&src))
        .unwrap_or_default();

    let version = match (flag, sniff.version) {
        (Some(v), Some(s)) if s != v => {
            eprintln!(
                "warning: {} looks like a SATySFi {s} document, but --lang {v} \
                 was given; proceeding as {v}",
                input.display()
            );
            v
        }
        (Some(v), _) => v,
        (None, Some(s)) if !s.is_implemented() => {
            return Err(anyhow::anyhow!(
                "{}: SATySFi {s} documents are not supported yet; supported: 0.0, 0.1 \
                 (detected a 0.1-style `use` header; pass `--lang 0.0` to \
                 force 0.0.6 interpretation)",
                input.display()
            ));
        }
        (None, _) => RustyfiVersion::DEFAULT,
    };

    let mode = if let Some(deps) = deps_flag {
        rustyfi_loader::LoadMode::Envelopes {
            deps: Some(deps.to_path_buf()),
        }
    } else if sniff.envelope_headers {
        rustyfi_loader::LoadMode::Envelopes { deps: None }
    } else {
        rustyfi_loader::LoadMode::Legacy
    };

    if matches!(mode, rustyfi_loader::LoadMode::Envelopes { .. })
        && !matches!(version, RustyfiVersion::V0_1)
    {
        let axis_b = if deps_flag.is_some() {
            "--deps"
        } else {
            "a `use` header"
        };
        return Err(anyhow::anyhow!(
            "{}: {axis_b} selects Envelopes packaging mode, which requires SATySFi 0.1, \
             but the language version resolved to {version}; pass --lang 0.1, \
             or drop --deps / the `use` header",
            input.display()
        ));
    }

    Ok((version, mode))
}

/// Concatenate the dependency-ordered library preludes ahead of the entry
/// document's own prelude, producing one synthetic file for elaboration.
fn merge_program(
    program: rustyfi_loader::LoadedProgram,
) -> (
    rustyfi_syntax::cst::File,
    std::collections::HashMap<usize, rustyfi_lang::types::Stage>,
) {
    fn as_v006(cst: rustyfi_loader::LoadedCst) -> rustyfi_syntax::cst::File {
        match cst {
            rustyfi_loader::LoadedCst::V0_0(f) => f,
            rustyfi_loader::LoadedCst::V0_1(_) => unreachable!(
                "merge_program is the V0_0-only path; V0_1 goes through compile_document_v1 once it exists"
            ),
        }
    }

    let mut files = program.files;
    let entry = files.pop().expect("loader always yields the entry last");
    let entry_cst = as_v006(entry.cst);
    let mut prelude = Vec::new();
    // Concatenation drops each file's headers, so `@stage:` — a property of
    // its BINDINGS, not of the file as a document — is recorded here
    // against the slots they land in. The entry document is stage 1 by
    // definition and contributes nothing.
    let mut stages = std::collections::HashMap::new();
    for lib in files {
        let cst = as_v006(lib.cst);
        let stage = rustyfi_lang::declared_stage(&cst);
        let start = prelude.len();
        prelude.extend(cst.prelude);
        if let Some(stage) = stage.filter(|s| *s != rustyfi_lang::types::Stage::default()) {
            stages.extend((start..prelude.len()).map(|i| (i, stage)));
        }
    }
    prelude.extend(entry_cst.prelude);
    (
        rustyfi_syntax::cst::File {
            headers: Vec::new(),
            prelude,
            in_kw: entry_cst.in_kw,
            body: entry_cst.body,
            eoi: entry_cst.eoi,
        },
        stages,
    )
}



use rustyfi_satyrographos as sg;

/// Map a `rustyfi_satyrographos::Error` to `run`'s exit codes.
fn sg_exit_code(err: &sg::Error) -> i32 {
    use sg::Error::*;
    match err {
        RootResolution | ManifestNotFound | NoRegistry => 3,
        // Not-found: a missing receipt, package/version absent, or the
        // solver's Unsatisfiable/VersionConflict (its analogue of
        // VersionNotFound).
        AlreadyInstalled { .. }
        | NotInstalled { .. }
        | PackageNotFound { .. }
        | VersionNotFound { .. }
        | Unsatisfiable { .. }
        | VersionConflict { .. } => 4,
        LibraryFilter { .. } | AmbiguousLibrary { .. } | AmbiguousDoc { .. } | DocFilter { .. } => 2,
        NoDocTarget => 3,
        DocBuild { .. } | OpamBuild { .. } => 1,
        Io { .. }
        | Manifest { .. }
        | Config { .. }
        | Receipt { .. }
        | UnmanagedCollision { .. }
        | PathTraversal { .. }
        | UnknownSource { .. }
        | EmptySource { .. }
        | Archive(_)
        | MissingDst { .. }
        | Satyristes { .. }
        | AmbiguousSource { .. }
        | ProjectManifest { .. }
        | Lockfile { .. }
        | UnsupportedSource { .. }
        | GitFailed { .. }
        | RegistryIndex { .. }
        | ChecksumMismatch { .. }
        | HttpDisabled { .. }
        | HttpFailed { .. }
        | InvalidVersion { .. }
        | HashFile { .. }
        | HashKeyConflict { .. }
        | Offline { .. } => 5,
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

fn is_package_command(name: &str) -> bool {
    matches!(
        name,
        "install" | "uninstall" | "build" | "list" | "status" | "search" | "update"
    )
}

fn run_package(name: &str, sm: &ArgMatches) -> i32 {
    let result = match name {
        "install" => cmd_install(sm),
        "uninstall" => cmd_uninstall(sm),
        "build" => cmd_build(sm),
        "list" => cmd_list(sm),
        // `status` reports through its own exit code rather than an error.
        "status" => return cmd_status(sm),
        "search" => cmd_search(sm),
        "update" => cmd_update(sm),
        other => {
            eprintln!("error: unknown command `{other}`");
            return 2;
        }
    };
    finish(result, sg_exit_code)
}

fn run_satyrographos(m: &ArgMatches) -> i32 {
    match m.subcommand() {
        Some((name, sm)) => run_package(name, sm),
        None => {
            eprintln!("error: no satyrographos subcommand given");
            2
        }
    }
}

/// `satyrographos build [PATH] [--doc NAME]` — run a `(libraryDoc ...)`'s own
/// build commands. The typesetter is THIS executable, so an unpacked archive
/// builds its docs with the binary beside them rather than whatever `rustyfi`
/// a `PATH` lookup happens to find.
fn cmd_build(m: &ArgMatches) -> Result<(), sg::Error> {
    let source = m
        .get_one::<PathBuf>("path")
        .cloned()
        .unwrap_or_else(|| PathBuf::from("."));
    // `--install` materialises the built products into the resolved root
    // (dist/doc/<name>/<dst>); with no `--install`, `build` only runs the
    // commands and reports which products exist.
    let install = m.get_flag("install").then(|| root_options(m));
    let opts = sg::BuildOptions {
        lang: m.get_one::<String>("lang").and_then(|s| sg::Lang::parse(s)),
        docs: m
            .get_many::<String>("doc")
            .map(|v| v.cloned().collect())
            .unwrap_or_default(),
        typesetter: std::env::current_exe().ok(),
        verbose: !m.get_flag("quiet"),
        lib_root: m.get_one::<PathBuf>("lib_root").cloned(),
        install,
    };
    for report in sg::build(&source, &opts)? {
        println!("built {} ({} command(s))", report.name, report.commands.len());
        for (product, present) in &report.products {
            println!("  {} {}", if *present { "->" } else { "!!" }, product);
        }
        for path in &report.installed {
            println!("  installed {}", path.display());
        }
    }
    Ok(())
}

fn root_options(m: &ArgMatches) -> sg::RootOptions {
    sg::RootOptions {
        lib_root: m.get_one::<PathBuf>("lib_root").cloned(),
        dest: m.get_one::<PathBuf>("dest").cloned(),
    }
}

/// Registry options from the shared `--registry`/`--offline` flags.
/// The cache dir / refresh come from
/// `$RUSTYFI_REGISTRY_CACHE` and each command's own semantics;
/// `update` sets `refresh` itself. `offline` also honors `$RUSTYFI_OFFLINE`
/// (via `RegistryOptions::is_offline`) even when `--offline` is not passed.
fn registry_options(m: &ArgMatches) -> Result<sg::RegistryOptions, sg::Error> {
    // The flag wins; below it the crate consults `$RUSTYFI_REGISTRY`, and
    // below that this fallback — the project's own `(registry …)`, then the
    // user's `config.toml`. Mirrors/kind come from whichever of those two
    // supplied the url, so a config-declared sparse index stays sparse.
    let fallback = registry_fallback(m)?;
    Ok(sg::RegistryOptions {
        url: m.get_one::<String>("registry").cloned(),
        offline: m.get_flag("offline"),
        mirrors: fallback
            .as_ref()
            .map(|f| f.mirrors.clone())
            .unwrap_or_default(),
        kind: fallback.as_ref().and_then(|f| f.kind),
        ..Default::default()
    })
}

/// The project's `(registry …)`, else the user's `config.toml` — the two
/// declared sources of a default repository, in that order.
fn registry_fallbacks(m: &ArgMatches) -> Result<Vec<sg::RegistryConfig>, sg::Error> {
    // A project that names a repository names THE repository: its manifest is
    // about this project, so it replaces the personal list rather than being
    // prepended to it.
    if let Some(cfg) = std::env::current_dir()
        .ok()
        .and_then(|cwd| sg::find_upward(&cwd))
        .and_then(|manifest| sg::satyristes::read_project(&manifest).ok())
        .and_then(|project| project.registry)
    {
        return Ok(vec![cfg]);
    }
    // `--config FILE` replaces the DISCOVERED config, not the layers above it:
    // it says which file to read, where `--registry` says which URL to use. A
    // file named on the command line that cannot be read is an error — unlike
    // a discovered one, whose absence is ordinary.
    let config = match m.get_one::<PathBuf>("config") {
        Some(path) => sg::config::read(path)?,
        None => sg::config::load()?,
    };
    Ok(config.registries().to_vec())
}

/// The first configured repository — what the single-registry paths use.
fn registry_fallback(m: &ArgMatches) -> Result<Option<sg::RegistryConfig>, sg::Error> {
    Ok(registry_fallbacks(m)?.into_iter().next())
}

/// The nearest `Satyristes`, searched upward from the current directory,
/// or `None` if there is none. Callers map the `None` case to the
/// exit-`3` [`sg::Error::ManifestNotFound`]. Shared by manifest-mode `install`
/// and by `update`.
fn find_manifest() -> Option<PathBuf> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    sg::find_upward(&cwd)
}

fn cmd_install(m: &ArgMatches) -> Result<(), sg::Error> {
    // No PATH → manifest mode (reconcile the nearest Satyristes).
    let Some(args) = m.get_many::<String>("path") else {
        return cmd_install_manifest(m);
    };
    // In order, stopping at the first failure. Each success has already
    // printed what it placed, so a partial run says exactly how far it got --
    // and the sources are independent, so nothing has to be undone.
    for arg in args {
        install_one(arg, m)?;
    }
    Ok(())
}

fn install_one(arg: &str, m: &ArgMatches) -> Result<(), sg::Error> {
    let libraries: Option<Vec<String>> = m
        .get_many::<String>("library")
        .map(|vals| vals.cloned().collect());
    let sg::RootOptions { lib_root, dest } = root_options(m);
    let opts = sg::InstallOptions {
        lib_root,
        dest,
        prefer_library: None,
        offline: m.get_flag("offline"),
        verbose: true,
        libraries,
        lang: m
            .get_one::<String>("lang")
            .and_then(|s| sg::Lang::parse(s)),
        force: m.get_flag("force"),
    };

    // Registry form: the argument is a registry NAME[@VERSION] when
    // it does not name a path on disk. A path on disk is installed directly.
    let report = if sg::is_url(arg) {
        let (url, sha256) = split_url_checksum(arg);
        if sha256.is_none() {
            // Everything else this command installs is checked against
            // something: the registry index's checksum, an `.opam`'s
            // `sha256=`, or bytes already on disk. A URL is the one form with
            // no such claim, and it must not be mistaken for one.
            eprintln!("warning: nothing verifies {url} (append `#sha256=...` to require a digest)");
        }
        sg::install_url(url, sha256, &opts)?
    } else if Path::new(arg).exists() {
        sg::install(Path::new(arg), &opts)?
    } else {
        let (name, version) = split_name_version(arg);
        let reg_opts = registry_options(m)?;
        // Each configured repository in turn: the first that HAS the
        // package wins, and a package missing from one is not an error
        // until every one has been asked.
        let opts = sg::InstallOptions {
            prefer_library: Some(name.to_string()),
            ..opts
        };
        let repos = registry_fallbacks(m)?;
        let urls: Vec<Option<String>> = if reg_opts.url.is_some() || repos.is_empty() {
            vec![None]
        } else {
            repos.iter().map(|r| r.url.clone()).collect()
        };
        let mut last: Option<sg::Error> = None;
        let mut done = None;
        for url in urls {
            match sg::install_registry(name, version, &opts, &reg_opts, url.as_deref()) {
                Ok((report, resolved)) => {
                    let from = url.unwrap_or_else(|| "the registry".to_string());
                    eprintln!("fetched {name} {} from {from}", resolved.version);
                    done = Some(report);
                    break;
                }
                Err(e) => last = Some(e),
            }
        }
        match done {
            Some(report) => report,
            None => return Err(last.unwrap_or(sg::Error::NoRegistry)),
        }
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

/// Split a URL argument's optional `#sha256=<hex>` fragment off the URL.
///
/// The digest rides on the argument rather than living in a flag because
/// `install` takes several sources at once: one `--sha256` could not say which
/// URL it belonged to, and a flag that silently applies to "the first one" is
/// worse than no flag.
fn split_url_checksum(arg: &str) -> (&str, Option<&str>) {
    match arg.split_once("#sha256=") {
        Some((url, digest)) if !digest.is_empty() => (url, Some(digest)),
        Some((url, _)) => (url, None),
        None => (arg, None),
    }
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



/// Manifest mode: locate `Satyristes` by upward search from the current
/// directory, reconcile the dependencies that name a source against
/// `Satyristes.lock` and the installed receipts, and re-materialise only the
/// changed/missing entries. When no `--lib-root`/`--dest`/`$RUSTYFI_LIB_ROOT`
/// is given, default the root to a `lib-rustyfi/` sibling of the manifest if
/// one exists.
fn cmd_install_manifest(m: &ArgMatches) -> Result<(), sg::Error> {
    let manifest = find_manifest().ok_or(sg::Error::ManifestNotFound)?;

    let sg::RootOptions { mut lib_root, dest } = root_options(m);
    if lib_root.is_none() && dest.is_none() && std::env::var_os("RUSTYFI_LIB_ROOT").is_none() {
        if let Some(dir) = manifest.parent() {
            let sibling = dir.join("lib-rustyfi");
            if sibling.is_dir() {
                lib_root = Some(sibling);
            }
        }
    }
    let opts = sg::RootOptions { lib_root, dest };

    let repos = registry_fallbacks(m)?;
    let report = sg::install_manifest_reg_multi(&manifest, &opts, &registry_options(m)?, &repos)?;
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
    for (url, e) in &report.unreachable_registries {
        eprintln!("warning: registry `{url}` could not be reached: {e}");
    }
    if report.installed.is_empty() && report.skipped.is_empty() {
        println!("  (no libraries declared)");
    }
    Ok(())
}

fn cmd_uninstall(m: &ArgMatches) -> Result<(), sg::Error> {
    let names = m
        .get_many::<String>("name")
        .expect("NAME is required by clap");
    let opts = root_options(m);
    for name in names {
        sg::uninstall(name, &opts)?;
        println!("uninstalled {name}");
    }
    Ok(())
}

fn cmd_list(m: &ArgMatches) -> Result<(), sg::Error> {
    let packages = sg::list(&root_options(m))?;
    if packages.is_empty() {
        println!("(no packages installed)");
    } else {
        for pkg in &packages {
            println!(
                "{} {} (lang {}, {} files)\n  {}",
                pkg.name,
                pkg.version,
                pkg.lang.as_str(),
                pkg.file_count,
                pkg.path.display()
            );
        }
    }
    Ok(())
}

/// How many repositories a search round covers.
fn urls_len(repos: &[sg::RegistryConfig], opts: &sg::RegistryOptions) -> usize {
    if opts.url.is_some() || repos.is_empty() {
        1
    } else {
        repos.len()
    }
}

/// `search <keyword>...`: list matching registry packages, one
/// `name version — description` line each, sorted by name.
fn cmd_search(m: &ArgMatches) -> Result<(), sg::Error> {
    let terms: Vec<&str> = m
        .get_many::<String>("term")
        .expect("KEYWORD is required by clap")
        .map(String::as_str)
        .collect();
    // Every configured repository, in order. `--registry`/`$RUSTYFI_REGISTRY`
    // pin one, in which case there is exactly one round.
    let opts = registry_options(m)?;
    let repos = registry_fallbacks(m)?;
    let urls: Vec<Option<String>> = if opts.url.is_some() || repos.is_empty() {
        vec![None]
    } else {
        repos.iter().map(|r| r.url.clone()).collect()
    };

    let mut any = false;
    let mut failures: Vec<(String, sg::Error)> = Vec::new();
    for url in urls {
        let label = url.clone().unwrap_or_default();
        match sg::search(&terms, &opts, url.as_deref()) {
            Ok(hits) => {
                for hit in &hits {
                    any = true;
                    let where_ = if repos.len() > 1 {
                        format!("  [{label}]")
                    } else {
                        String::new()
                    };
                    // `hit.name` is always what `install NAME` accepts; the
                    // registry's own raw id is shown alongside in parens
                    // when it differs.
                    let raw = hit
                        .registry_name
                        .as_deref()
                        .map(|r| format!(" ({r})"))
                        .unwrap_or_default();
                    match &hit.description {
                        Some(desc) => println!("{}{raw} {} — {desc}{where_}", hit.name, hit.version),
                        None => println!("{}{raw} {}{where_}", hit.name, hit.version),
                    }
                }
            }
            // One unreachable repository must not hide the others' results;
            // report it once the reachable ones have been searched.
            Err(e) => failures.push((label, e)),
        }
    }
    for (label, e) in &failures {
        eprintln!("warning: registry `{label}` could not be searched: {e}");
    }
    if !any {
        if failures.len() == urls_len(&repos, &opts) {
            return Err(failures.into_iter().next().map(|(_, e)| e).unwrap_or(sg::Error::NoRegistry));
        }
        println!("(no matching packages)");
    }
    Ok(())
}

/// `update`: re-fetch the index and report available
/// upgrades against the nearest `Satyristes.lock` (does not apply them).
fn cmd_update(m: &ArgMatches) -> Result<(), sg::Error> {
    let manifest = find_manifest().ok_or(sg::Error::ManifestNotFound)?;
    let repos = registry_fallbacks(m)?;
    let report = sg::update_multi(&manifest, &registry_options(m)?, &repos)?;

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
    for (url, e) in &report.unreachable {
        eprintln!("warning: registry `{url}` could not be refreshed: {e}");
    }
    Ok(())
}

/// `status` maps a missing-files result to exit `1`, so it
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
/// executable to `<DIR>/rustyfi` and `<DIR>/satyrographos`, so those names
/// become opt-in aliases of this multicall binary. Refuses to clobber a
/// target that is not already a link/copy of this exe.
fn multicall_install(m: &ArgMatches) -> anyhow::Result<()> {
    use anyhow::Context as _;

    let dir = m
        .get_one::<PathBuf>("dir")
        .expect("--dir is required by clap");
    let exe = std::env::current_exe().context("cannot locate the current executable")?;

    std::fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;

    for alias in ["rustyfi", "satyrographos"] {
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

#[cfg(test)]
mod arg_tests {
    use super::*;

    #[test]
    fn a_url_carries_its_own_checksum() {
        let (url, sha) = split_url_checksum("https://x/p.tar.gz#sha256=abc123");
        assert_eq!(url, "https://x/p.tar.gz");
        assert_eq!(sha, Some("abc123"));
    }

    #[test]
    fn a_url_without_one_keeps_its_whole_self() {
        assert_eq!(
            split_url_checksum("https://x/p.tar.gz"),
            ("https://x/p.tar.gz", None)
        );
        // An empty digest is no digest -- and must not leave a stray `#` on
        // the URL that gets fetched.
        assert_eq!(
            split_url_checksum("https://x/p.tar.gz#sha256="),
            ("https://x/p.tar.gz", None)
        );
    }

    #[test]
    fn a_registry_name_with_a_version_is_unaffected() {
        assert_eq!(split_name_version("xpath@0.3.0"), ("xpath", Some("0.3.0")));
        assert_eq!(split_name_version("xpath"), ("xpath", None));
    }
}
