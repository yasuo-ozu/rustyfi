// The crate's front page IS the README: one description of what this program
// is, kept in one file, so the two cannot drift. Everything below it is the
// implementation note for this file specifically.
#![doc = include_str!("../../../README.md")]
//!
//! # Implementation notes
//!
//! The chimera CLI (plan §1/§4/§7.3): a single multicall
//! (busybox-/rustup-style) binary that behaves as three tools, dispatched on
//! its `argv[0]` basename and on its first subcommand:
//!
//! - `rustyfi` — the compiler (default) plus the `satyrographos` and
//!   `multicall` subcommand trees;
//! - `satyrographos` — the package manager only.
//!
//! Package-management logic lives in the clap-free `rustyfi-satyrographos`
//! crate; this file only parses arguments, resolves `lib_root`/`dest`, and
//! calls in. The compile path (`cmd_compile`) is byte-for-byte the old
//! `main`: positional input, `--output`, `--lib-root` with upward
//! `lib-rustyfi/` discovery, and `--target-version` with header sniffing.

use std::path::{Path, PathBuf};

use clap::ArgMatches;

mod cache;
mod dispatch;
mod format;
mod man;

/// Read an auxiliary cross-reference table, or an empty one if the file is
/// absent, unreadable, or not the flat `{"key": "value"}` object upstream
/// SATySFi writes.
///
/// Best-effort by design: an aux file is a hint that lets the fixpoint start
/// closer to its answer, never an input the result depends on, so a corrupt or
/// foreign one costs a trial rather than a wrong render or an error.
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
/// `AuxTable` is a `BTreeMap`, so the bytes are deterministic for a given
/// table. Failure is ignored for the same reason a read failure is: a missing
/// aux file only costs a fixpoint trial next time.
fn write_aux(path: &Path, aux: &rustyfi_lang::crossref::AuxTable) {
    if let Ok(text) = serde_json::to_string(aux) {
        let _ = std::fs::write(path, text);
    }
}

fn main() {
    // Compat fix: the recursive-descent parser + elaborator use deep stacks on
    // real documents (the ~300-line official SATySFi demo overflows the default
    // 8 MB main-thread stack). The test suite already runs the compile on a
    // large-stack worker thread; do the same for the CLI so real documents don't
    // crash with a bare "stack overflow" before any diagnostic prints.
    let code = std::thread::Builder::new()
        .name("rustyfi-main".into())
        .stack_size(256 * 1024 * 1024)
        .spawn(run)
        .expect("failed to spawn the main worker thread")
        .join()
        .expect("the main worker thread panicked");
    std::process::exit(code);
}

/// Exit codes (plan §4): `0` success; `2` clap usage error (clap exits `2`
/// itself on parse failure); `3` root resolution; `4` receipt collision /
/// not-installed; `5` filesystem/archive/manifest; `1` compile error or a
/// `status` mismatch.
fn run() -> i32 {
    let matches = dispatch::build_cli().get_matches();

    // `multicall(true)` turns argv[0]'s basename into the top-level
    // subcommand: `rustyfi` | `satyrographos`.
    match matches.subcommand() {
        Some(("rustyfi", m)) => match m.subcommand() {
            // The package commands sit at top level now; `satyrographos …`
            // remains as the personality invoked by that argv[0].
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
    // HTML output backend, Slice 1 (surface): `--format` has a clap
    // `.default_value("pdf")`, so this `get_one` is always `Some`;
    // `str::parse` mirrors how `--target-version` is parsed below
    // (`format.rs`'s doc comment).
    let format: format::OutputFormat = m
        .get_one::<String>("format")
        .expect("--format has a clap default")
        .parse()
        .map_err(|e: String| anyhow::anyhow!(e))?;
    let output = m
        .get_one::<PathBuf>("output")
        .cloned()
        .unwrap_or_else(|| input.with_extension(format.extension()));
    // A NAMED root is exactly that one root: `--lib-root`/`$RUSTYFI_LIB_ROOT`
    // says where to look, and adding discovered roots behind it would make a
    // build depend on what happens to be installed on the machine. Discovery
    // instead supplies the WHOLE chain when nothing is named, so a
    // project-local `.rustyfi/` layers over the development tree and the
    // system install rather than replacing them.
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

    let target_version = m.get_one::<String>("target_version").map(String::as_str);
    let deps_flag = m.get_one::<PathBuf>("deps").map(PathBuf::as_path);
    let (version, mode) = resolve_version_and_mode(target_version, deps_flag, &input)?;
    // Phase-7c saphe solver, C3: whether this compile is package-manager
    // driven (Envelopes/manifest mode) — decides below whether a sibling
    // `Satyristes.lock`'s digest folds into the cache key. Captured before
    // `mode` is moved into `LoadOptions`.
    let is_envelopes_mode = matches!(mode, rustyfi_loader::LoadMode::Envelopes { .. });

    // `--timing` (or RUSTYFI_TIMING=1): coarse per-phase wall-clock breakdown to
    // stderr, for profiling the load→compile→render pipeline without an external
    // profiler. The flag propagates to the library phases (elaborate/typecheck/
    // eval trials, which read RUSTYFI_TIMING) via the env var, and forces
    // `--no-cache` below so every phase actually runs rather than a cache hit.
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

    // Text-rendering plan, Slice 1: resolve font configuration (flags >
    // --font-dir/$RUSTYFI_FONT_DIR > --lib-root) into a real TtfFontStore,
    // or `None` when nothing is configured anywhere — in which case every
    // remaining step below is byte-for-byte the pre-Slice-1 base-14 path.
    let font_store = resolve_font_store(m, lib_root.as_deref())?;

    // Phase-7c saphe solver, C3: when this compile is package-manager driven
    // (Envelopes/manifest mode), best-effort locate the project's
    // `Satyristes`/`Satyristes.lock` (upward search from the input,
    // exactly like `--lib-root` discovery) and fold the lock's digest into
    // the cache key below, so a `saphe update`/reconcile that changes a
    // locked package's version invalidates the cache even when the entry
    // document's own bytes did not change. A project with no
    // `Satyristes`/lock (or a Legacy-mode compile) simply folds in
    // `None`, unchanged from before this fold existed.
    let deps_lock_digest: Option<String> = is_envelopes_mode
        .then(|| discover_deps_lock_digest(&input))
        .flatten();

    // Content-addressed compile cache (plan: "make recompiling an unchanged
    // document near-instant"). Caching is ON by default; `--no-cache` disables
    // both read and write. The key is computed from the just-loaded program —
    // its resolved input bytes, the compiler/target version, the entry name,
    // (Slice 1) the resolved font identity, and (C3) the resolved deps-lock
    // digest — *before* the expensive compile+render, so a hit skips them
    // entirely and writes byte-for-byte the PDF an earlier miss rendered and
    // stored.
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

    // `Some(store)` ⇒ typeset and render through the real embedded TrueType
    // face (Type0/CIDFontType2, `render_pdf_ttf`); `None` ⇒ today's base-14
    // path, verbatim (same `Base14Metrics` instance, same `render_pdf` call),
    // so existing fixtures/behavior are untouched with no font configured.
    let base14 = rustyfi_pdf::Base14Metrics;
    let metrics: &dyn rustyfi_backend::FontMetrics = match &font_store {
        Some(store) => store,
        None => &base14,
    };
    // Auxiliary cross-reference file (upstream SATySFi's `<doc>.satysfi-aux`,
    // same name and same flat JSON object, so the two interoperate): seeds the
    // cross-reference fixpoint from the previous run so a forward reference
    // resolves on the first trial instead of forcing a second. Disabled by
    // `--no-aux`.
    //
    // Unlike the compile cache, this is NOT forced off by `--timing`: a cache
    // hit skips every phase, leaving a profiling run nothing to measure, but an
    // aux file skips no phase — it only changes how many fixpoint trials are
    // needed, which is exactly what a profiling run wants to see. A cold
    // measurement asks for `--no-aux` (and, comparing against upstream
    // SATySFi, deletes its `.satysfi-aux` too — upstream reads one by default
    // just the same).
    //
    // This cannot change the output: `rustyfi_lang` discards the seed and
    // redoes the fixpoint cold if the final trial turned out to depend on a
    // seeded value it never re-derived (`CrossRefs::seed_unvalidated`).
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
            // FileV1 CST (plan C1). Slice 1's lowering erases the module
            // wrapper lang-side (rustyfi_lang::v1::lower).
            rustyfi_lang::compile_document_v1_with_aux(&program.files, metrics, &mut aux)
                .map_err(|e| anyhow::anyhow!("{}: {e}", input.display()))?
                .0
        }
        _ => {
            // Slice X4a: a V0_0-rooted load whose dependency graph contains
            // at least one foreign V0_1 node (a `@require:` of a
            // `dist-v01/packages/` package, per the loader's Q4-mirror
            // rule) routes through the new `compile_document_v006_xver`
            // entry point instead of the pure-0.0.6
            // `merge_program`/`compile_document_cst` path — ONLY when such
            // a dependency is actually present, so a pure-0.0.6 load (the
            // overwhelming majority — every existing fixture) takes the
            // exact old path, byte-identical.
            let has_v01_dep = program
                .files
                .iter()
                .any(|f| matches!(f.cst, rustyfi_loader::LoadedCst::V0_1(_)));
            if has_v01_dep {
                rustyfi_lang::compile_document_v006_xver_with_aux(&program.files, metrics, &mut aux)
                    .map_err(|e| anyhow::anyhow!("{}: {e}", input.display()))?
                    .0
            } else {
                let merged = merge_program(program);
                rustyfi_lang::compile_document_cst_with_aux(&merged, metrics, &mut aux)
                    .map_err(|e| anyhow::anyhow!("{}: {e}", input.display()))?
                    .0
            }
        }
    };

    // HTML output backend, Slice 1 (surface, point 2): everything above
    // (load, version resolution, font store, cache lookup, compile) is
    // shared; only this terminal render+write step differs, branching on
    // `--format`. `Html` reuses the exact same
    // `doc.geometry`/`doc.pages`/`doc.images`/`doc.extras` inputs the PDF
    // arm does — `render_html` is argument-for-argument with
    // `render_pdf_with` (`rustyfi_html`'s crate doc comment). Since survey
    // #6 the HTML backend lives in its own `rustyfi-html` crate (a peer of
    // `rustyfi-pdf`), not inside `rustyfi-pdf` itself.
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
        // HTML output backend, Slice 3: mirrors the PDF arm immediately
        // above — a configured `font_store` renders through
        // `render_html_ttf_with` (real `@font-face`-embedded fonts,
        // metric-faithful with the layout), `None` keeps Slice 1/2's
        // base-14 `render_html` path exactly.
        format::OutputFormat::Html => match &font_store {
            Some(store) => rustyfi_html::render_html_ttf_with(
                &doc.geometry,
                &doc.pages,
                store,
                &doc.images,
                &doc.extras,
            )?
            .into_bytes(),
            None => rustyfi_html::render_html(&doc.geometry, &doc.pages, &doc.images, &doc.extras)?
                .into_bytes(),
        },
        // Reflowable/semantic HTML output ("CLI"): a THIRD, independent
        // serialization of the SAME compiled `doc` above — `doc.reflow_source`
        // (the pre-page-break flat `Vec<VertBox>`, `DocumentValue`'s newest
        // field, populated unconditionally by every `compile_document_*` path
        // through the shared `page_break_core`) feeds the new reflow backend
        // instead of `doc.pages`. S2 (§4 "Links/metadata") additionally threads
        // `doc.reflow_links`/`reflow_dests` — the `DecoId`-keyed link/
        // destination side-channel `eval_document_trials` fills alongside
        // `extras`, once `fire_hooks` has run — so `\href`s become real `<a
        // href>`s. Mirrors the `Html` arm immediately above for the font-store
        // branch.
        format::OutputFormat::HtmlReflow => match &font_store {
            Some(store) => rustyfi_html::render_html_reflow_ttf_with(
                doc.reflow_source.as_deref(),
                &doc.geometry,
                store,
                &doc.images,
                &doc.extras,
                &doc.reflow_links,
                &doc.reflow_dests,
            )?
            .into_bytes(),
            None => rustyfi_html::render_html_reflow(
                doc.reflow_source.as_deref(),
                &doc.geometry,
                &doc.images,
                &doc.extras,
                &doc.reflow_links,
                &doc.reflow_dests,
            )?
            .into_bytes(),
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

    // Persist the cross-reference table for next time, so a forward reference
    // resolves on the first trial (see `aux_path` above). Written only after a
    // successful render, and only on the compile path — a cache hit returned
    // long before here, leaving the previous run's file exactly as it was.
    if let Some(path) = &aux_path {
        write_aux(path, &aux);
    }

    // Populate the cache for next time (best-effort: a cache-write failure
    // must never fail an otherwise-successful compile).
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

/// Resolve the text-rendering plan's Slice-1 font configuration into a
/// ready [`rustyfi_pdf::TtfFontStore`], or `None` when nothing is configured
/// (the caller then keeps the base-14 path exactly as before this feature
/// existed).
///
/// Precedence (highest first), mirroring how `lib_root` itself resolves
/// `--lib-root` > `$RUSTYFI_LIB_ROOT` > upward discovery:
///
/// 1. `--font` (+ optional `--font-bold`/`--font-oblique`, enforced by clap
///    to require `--font`) — a config-less one-off, no `fonts.rustyfi-hash`
///    involved.
/// 2. `--font-dir <DIR>`.
/// 3. `$RUSTYFI_FONT_DIR`.
/// 4. `lib_root` itself (already resolved by the caller), so a project that
///    keeps its font config alongside `dist/packages/` needs no extra flag.
///
/// A missing configuration at whichever root ends up being examined (or no
/// root at all) is "nothing configured" (`Ok(None)`); once a
/// `fonts.rustyfi-hash` *is* found, any further problem — malformed JSON, an
/// unknown default-face abbrev, a font file that fails to load — is a real
/// error, surfaced to the user via the normal compile-error path rather than
/// silently substituting base-14 for what looks like a real, if broken,
/// font configuration (see `rustyfi_pdf::fonts`'s module docs).
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

/// The CLI's default `--lib-root` rule, used only when neither `--lib-root`
/// nor `$RUSTYFI_LIB_ROOT` is given: [`sg::roots::discover_all`], the same search
/// the package manager runs, so a document compiles against exactly the root
/// an install would have written to — a development `lib-rustyfi/`, then a
/// project-local `.rustyfi/` beside its `Satyristes`, then the user and
/// system prefixes.
///
/// It starts at the DOCUMENT's own directory rather than the working
/// directory, so `rustyfi some/nested/doc.saty` behaves the same regardless of
/// where the command was run from.
fn discover_lib_roots(input: &std::path::Path) -> Vec<PathBuf> {
    input
        .parent()
        .map(sg::roots::discover_all)
        .unwrap_or_default()
}

/// Phase-7c saphe solver, C3: find the nearest `Satyristes` at or above
/// `input`'s directory (same upward-search shape as [`discover_lib_root`])
/// and, if its sibling `Satyristes.lock` exists and has at least one locked
/// entry, return `Some(digest)`. `None` covers every "nothing to fold in"
/// case uniformly — no `Satyristes` found, no lockfile yet, or an empty
/// one — so the cache key is byte-for-byte unchanged from before this fold
/// for any project that has not adopted the package manager.
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

/// Resolve BOTH axes of the load — the language version (Axis A) and the
/// packaging mode (Axis B, `rustyfi_loader::LoadMode`) — from the
/// `--target-version` flag, the `--deps` flag, and best-effort header
/// detection (`rustyfi_syntax::sniff_headers`). This is the plan's detection
/// ladder (§1.4) in its Ld3a-minimal form.
///
/// Axis A (version), exactly as before this became two-axis:
/// - flag given, sniffer disagrees: warn to stderr, obey the flag;
/// - flag given: obey it (the loader still rejects unimplemented versions);
/// - no flag, sniffer detects an unimplemented version: fail now with a hint;
/// - otherwise: the default, 0.0.6.
///
/// Axis B (mode):
/// - `--deps <FILE>` given → `Envelopes { deps: Some(FILE) }` (ladder step 2);
/// - else a `use`-shaped header sniffed → `Envelopes { deps: None }` (step 3);
/// - else `Legacy` (step 4).
///
/// The rejected combination (Axis A = 0.0.6, Axis B = Envelopes) is surfaced
/// here, early and naming the flag that pinned each axis, rather than left to
/// the loader's `InvalidModeVersion` backstop.
fn resolve_version_and_mode(
    flag: Option<&str>,
    deps_flag: Option<&Path>,
    input: &Path,
) -> anyhow::Result<(rustyfi_syntax::RustyfiVersion, rustyfi_loader::LoadMode)> {
    use rustyfi_syntax::RustyfiVersion;

    let flag = flag
        .map(str::parse::<RustyfiVersion>)
        .transpose()
        .map_err(|e| anyhow::anyhow!("--target-version: {e}"))?;
    // Sniffing is advisory only: if the file is unreadable, let the loader
    // report the I/O error on its own terms.
    let sniff = std::fs::read_to_string(input)
        .ok()
        .map(|src| rustyfi_syntax::sniff_headers(&src))
        .unwrap_or_default();

    // ---- Axis A: the language version ----
    let version = match (flag, sniff.version) {
        (Some(v), Some(s)) if s != v => {
            eprintln!(
                "warning: {} looks like a SATySFi {s} document, but --target-version {v} \
                 was given; proceeding as {v}",
                input.display()
            );
            v
        }
        (Some(v), _) => v,
        (None, Some(s)) if !s.is_implemented() => {
            return Err(anyhow::anyhow!(
                "{}: SATySFi {s} documents are not supported yet; supported: 0.0.6, 0.1 \
                 (detected a 0.1-style `use` header; pass `--target-version 0.0.6` to \
                 force 0.0.6 interpretation)",
                input.display()
            ));
        }
        (None, _) => RustyfiVersion::DEFAULT,
    };

    // ---- Axis B: the packaging mode ----
    let mode = if let Some(deps) = deps_flag {
        rustyfi_loader::LoadMode::Envelopes {
            deps: Some(deps.to_path_buf()),
        }
    } else if sniff.envelope_headers {
        rustyfi_loader::LoadMode::Envelopes { deps: None }
    } else {
        rustyfi_loader::LoadMode::Legacy
    };

    // The rejected combination (plan §1.3 row 4): 0.0.6 has no `use` headers.
    // Name the flag that pinned each axis — a diagnostic the loader's
    // `InvalidModeVersion` backstop cannot give.
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
             but the language version resolved to {version}; pass --target-version 0.1, \
             or drop --deps / the `use` header",
            input.display()
        ));
    }

    Ok((version, mode))
}

/// Concatenate the dependency-ordered library preludes ahead of the entry
/// document's own prelude, producing one synthetic file for elaboration.
/// (The v0.0.6 analog type-checks each library into a shared environment in
/// dependency order; untyped elaboration gets the same scoping by prelude
/// concatenation.)
fn merge_program(program: rustyfi_loader::LoadedProgram) -> rustyfi_syntax::cst::File {
    // `merge_program` is the V0_0-only path; V0_1 goes through
    // `compile_document_v1` once it exists. `LoadedCst::V0_1` is genuinely
    // unreachable today: `is_implemented()` still gates `V0_1` out at
    // `rustyfi_loader::load()`.
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
    for lib in files {
        prelude.extend(as_v006(lib.cst).prelude);
    }
    prelude.extend(entry_cst.prelude);
    rustyfi_syntax::cst::File {
        headers: Vec::new(),
        prelude,
        in_kw: entry_cst.in_kw,
        body: entry_cst.body,
        eoi: entry_cst.eoi,
    }
}

// ---------------------------------------------------------------------------
// Package-manager mode (satyrographos <cmd>).
// ---------------------------------------------------------------------------

use rustyfi_satyrographos as sg;

/// Map a `rustyfi_satyrographos::Error` to the plan's §4 exit codes.
fn sg_exit_code(err: &sg::Error) -> i32 {
    use sg::Error::*;
    match err {
        // Nothing to operate on (no root, no Satyristes, or no registry config).
        RootResolution | ManifestNotFound | NoRegistry => 3,
        // Not-found: a missing receipt, or a package/version absent from the
        // index — including the phase-7c solver's own "no version fits"
        // outcomes ([`Unsatisfiable`]/[`VersionConflict`]), which are the
        // solver-graph analogue of a plain [`VersionNotFound`].
        AlreadyInstalled { .. }
        | NotInstalled { .. }
        | PackageNotFound { .. }
        | VersionNotFound { .. }
        | Unsatisfiable { .. }
        | VersionConflict { .. } => 4,
        // Library/doc-selection / filter usage errors (plan §4.1).
        LibraryFilter { .. } | AmbiguousLibrary { .. } | AmbiguousDoc { .. } | DocFilter { .. } => 2,
        // Nothing to build here.
        NoDocTarget => 3,
        // A doc's own build command failed: its exit status is the story, and
        // the typesetter has already said why on stderr.
        DocBuild { .. } | OpamBuild { .. } => 1,
        // Filesystem / archive / manifest / Satyristes / lockfile / registry-
        // fetch / integrity failures.
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

/// Whether `name` is one of the package-manager commands, which the compiler
/// personality and the `satyrographos` personality both carry.
fn is_package_command(name: &str) -> bool {
    matches!(
        name,
        "install" | "uninstall" | "build" | "list" | "status" | "search" | "update"
    )
}

/// Run one package-manager command, whichever personality it arrived through.
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
    let opts = sg::BuildOptions {
        lang: m.get_one::<String>("lang").and_then(|s| sg::Lang::parse(s)),
        docs: m
            .get_many::<String>("doc")
            .map(|v| v.cloned().collect())
            .unwrap_or_default(),
        typesetter: std::env::current_exe().ok(),
        verbose: !m.get_flag("quiet"),
        lib_root: m.get_one::<PathBuf>("lib_root").cloned(),
    };
    for report in sg::build(&source, &opts)? {
        println!("built {} ({} command(s))", report.name, report.commands.len());
        for (product, present) in &report.products {
            println!("  {} {}", if *present { "->" } else { "!!" }, product);
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

/// Registry options from the shared `--registry`/`--offline` flags (plan
/// §5.4 step 1; phase 7d slice S2 design §2.5/§4). The cache dir / refresh
/// come from `$RUSTYFI_REGISTRY_CACHE` and each command's own semantics;
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

/// The nearest `Satyristes`, searched upward from the current directory
/// (plan §5.3), or `None` if there is none. Callers map the `None` case to the
/// exit-`3` [`sg::Error::ManifestNotFound`]. Shared by manifest-mode `install`
/// and by `update`.
fn find_manifest() -> Option<PathBuf> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    sg::find_upward(&cwd)
}

fn cmd_install(m: &ArgMatches) -> Result<(), sg::Error> {
    // No PATH → phase-2 manifest mode (reconcile the nearest Satyristes).
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
        prefer_library: None,
        offline: m.get_flag("offline"),
        verbose: true,
        libraries,
        lang: m
            .get_one::<String>("lang")
            .and_then(|s| sg::Lang::parse(s)),
        force: m.get_flag("force"),
    };

    // Registry form (plan §5.4): the argument is a registry NAME[@VERSION] when
    // it does not name a path on disk. A path on disk is a phase-1 install.
    let report = if Path::new(arg).exists() {
        sg::install(Path::new(arg), &opts)?
    } else {
        let (name, version) = split_name_version(arg);
        let reg_opts = registry_options(m)?;
        // Registry-URL precedence (plan §5.4 / this port's [registry] section):
        // --registry flag > $RUSTYFI_REGISTRY > the nearest Satyristes's
        // (registry …) > the user's config.toml
        // [registry] url. The first two live in `reg_opts`; supply the third as
        // the fallback so a project with a declared registry needs no flag.
        // Each configured repository in turn: the first that HAS the package
        // wins, and a package missing from one is not an error until every
        // one has been asked.
        // The user named a package; if its manifest declares a library of
        // that name, that is the one they meant.
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

    let report = sg::install_manifest_reg(&manifest, &opts, &registry_options(m)?)?;
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
    let name = m
        .get_one::<String>("name")
        .expect("NAME is required by clap");
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

/// `search <term>` (plan §8): list matching registry packages, one
/// `name version — description` line each, sorted by name.
/// How many repositories a search round covers.
fn urls_len(repos: &[sg::RegistryConfig], opts: &sg::RegistryOptions) -> usize {
    if opts.url.is_some() || repos.is_empty() {
        1
    } else {
        repos.len()
    }
}

fn cmd_search(m: &ArgMatches) -> Result<(), sg::Error> {
    let term = m
        .get_one::<String>("term")
        .expect("TERM is required by clap");
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
        match sg::search(term, &opts, url.as_deref()) {
            Ok(hits) => {
                for hit in &hits {
                    any = true;
                    let where_ = if repos.len() > 1 {
                        format!("  [{label}]")
                    } else {
                        String::new()
                    };
                    match &hit.description {
                        Some(desc) => println!("{} {} — {desc}{where_}", hit.name, hit.version),
                        None => println!("{} {}{where_}", hit.name, hit.version),
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
            // Nothing was searched successfully at all.
            return Err(failures.into_iter().next().map(|(_, e)| e).unwrap_or(sg::Error::NoRegistry));
        }
        println!("(no matching packages)");
    }
    Ok(())
}

/// `update` (plan §8, §5.4 step 1): re-fetch the index and report available
/// upgrades against the nearest `Satyristes.lock` (does not apply them).
fn cmd_update(m: &ArgMatches) -> Result<(), sg::Error> {
    let manifest = find_manifest().ok_or(sg::Error::ManifestNotFound)?;
    let report = sg::update(&manifest, &registry_options(m)?)?;

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
// `rustyfi multicall install --dir DIR` (plan §4.5).
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
