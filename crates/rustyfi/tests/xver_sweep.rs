//! Cross-version import sweep, IN-PROCESS: real Satyrographos packages,
//! `@require:`d from a 0.1 document against a lib root that holds ONLY the
//! 0.0.6 corpus.
//!
//! This is the Rust successor to `layout-tests/xver_sweep.py`, which measured the
//! same 22 packages by spawning `target/debug/rustyfi` once per compile and
//! grepping its stderr for `output written on`. Same cases, same baseline, same
//! two-compiles-per-package shape — but the port is CALLED, not spawned:
//! `rustyfi_loader::load` + the `compile_document_*` entry points, straight out
//! of this test binary. What that buys is not speed, it is honesty about WHAT
//! is being measured: a subprocess sweep measures the binary that happens to be
//! in `target/debug/` (which need not be the code `cargo test` just built), and
//! everything it can say about a failure is one grepped line. Here a failure is
//! a `CompileError` value, the dispatch below is visibly the same dispatch
//! `main.rs` performs, and there is no `--no-cache`/`--no-aux` to remember —
//! there is no cache and no aux file on this path at all.
//!
//! # What it measures
//!
//! For each case, two compiles (`xver_sweep_data/cases.rs` holds the documents;
//! they live in Rust source rather than as `.saty` files in the repo, so a
//! probe cannot be edited without a diff on this test):
//!
//! * the CROSSING case — a minimal **0.1** document that `@require:`s the
//!   package. The lib root contains `dist/` only, never `dist-v01/`, so
//!   `v006::resolve::resolve_require`'s same-generation preference has nothing
//!   to prefer and every `@require:` genuinely crosses. This is the trap
//!   `xver_capstone.rs` documents at length: with both corpora visible a 0.1
//!   entry (correctly) gets the 0.1 package and the sweep quietly stops
//!   measuring the bridge while still passing. `check_root` below refuses a
//!   root with a `dist-v01/` in it for exactly that reason.
//! * the **0.0.6 CONTROL** — the same package exercised the same way from an
//!   ordinary 0.0.6 document. This is what separates a BRIDGE failure from a
//!   pre-existing 0.0.6-side gap, and it reclassified 5 of the 22 cases in the
//!   original audit (`CLAUDE.md` §1, "What the REAL corpus says": `azmath` and
//!   `ruby` fail IDENTICALLY as plain 0.0.6 documents — a 0.0.6 parser gap and
//!   three missing primitives, never the boundary's fault). Never read a
//!   `refuse` without its control.
//!
//! Both directions of drift are failures. An unexpected PASS is reported and
//! fails the test just as loudly as a regression, because a baseline that
//! silently absorbs improvements stops describing anything.
//!
//! # The lib root, and why it is not assembled by default
//!
//! The 22 packages are third-party; they cannot be embedded in Rust source the
//! way the probe documents are. The root therefore has to be assembled:
//! `lib-rustyfi/dist/packages/` (the port's own bundled 0.0.6 corpus) copied
//! into `<root>/dist/packages/`, then each registry package installed on top.
//! That needs the network and takes minutes, so, exactly like
//! `layout_fidelity.rs`, this test is `#[ignore]`d and opts in:
//!
//! ```text
//! # against an already-assembled root (seconds):
//! RUSTYFI_XVER_ROOT=/path/to/root \
//!   cargo test -p rustyfi --test xver_sweep -- --ignored --nocapture
//!
//! # assemble one first (network, minutes; needs the `http` feature for
//! # `https://` tarballs), and keep it for next time:
//! RUSTYFI_XVER_ROOT=/path/to/root RUSTYFI_XVER_ASSEMBLE=1 \
//!   cargo test -p rustyfi --features http --test xver_sweep -- --ignored --nocapture
//!
//! # ... and RUSTYFI_XVER_OFFLINE=1 assembles with no network at all when the
//! # archive cache is already warm — no `http` feature needed, since a cache
//! # hit never reaches for the transport. RUSTYFI_XVER_ONLY=a,b restricts the
//! # sweep (the Python's repeatable `--only`).
//! ```
//!
//! An assembled root is ~1.4 MB and the whole 44-compile sweep takes ~19 s
//! against it (measured, debug build) — the Python's per-compile process spawn
//! and PDF render were most of its wall clock, and neither is measuring
//! anything the baseline records.
//!
//! Assembly calls `rustyfi_satyrographos` as a LIBRARY —
//! `sg::install_registry`, the same call `main.rs`'s `install_one` makes for a
//! registry `NAME` argument, reading this repo's own `config.toml` for the
//! repository URL/mirrors/kind just as the Python passed `--config`. Nothing
//! here spawns `rustyfi install`. (`sg::registry::acquire` does shell out to
//! `git` for a git-kind index, and the archive fetch is `sg`'s HTTP client;
//! both are the package manager's own business, inside the library, not this
//! harness reaching for a CLI.)
//!
//! **A missing root never produces a green test.** Not a skip, not zero cases
//! quietly asserted over: [`resolve_root`] panics naming the two variables
//! above. This file also refuses a root with no `dist/packages/`, a root with a
//! `dist-v01/` sibling, and — the vacuity check with real teeth — a run in
//! which every crossing case that PASSED loaded no `V0_0` dependency at all,
//! which is what "the 0.1 entry never crossed anything" looks like from the
//! outside.
//!
//! # The dispatch is the point
//!
//! `compile_one` reproduces `crates/rustyfi/src/main.rs`'s `cmd_compile` from
//! `resolve_version_and_mode` down to the `match version` at its line ~299:
//! Axis A pinned by the case (the `--lang` the Python passed), Axis B sniffed
//! from the document's own headers (`sniff_headers().envelope_headers`), then
//! `V0_1 => compile_document_v1_with_aux`, and otherwise the X4a split on
//! "does any loaded dependency carry a `LoadedCst::V0_1`" —
//! `compile_document_v006_xver_with_aux` if so, else `merge_program` +
//! `compile_document_cst_with_stages`. That third arm is not decoration: the
//! 0.0.6 CONTROLS take it, and it is the only path that carries `@stage:`
//! headers through prelude concatenation, so a control compiled through the
//! xver entry point would not be the same measurement.
//!
//! `merge_program` is duplicated below because it lives in a `[[bin]]` and a
//! binary crate exports nothing. It is a verbatim copy; if `main.rs`'s ever
//! changes, this one must follow.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use rustyfi_satyrographos as sg;
use rustyfi_syntax::RustyfiVersion;

/// The probe documents and the baseline, kept out of this file so the harness
/// and the corpus it measures can be edited independently. A `#[path]` module
/// under a `tests/` SUBDIRECTORY is not itself compiled as a test binary, which
/// is what makes this shape safe.
#[path = "xver_sweep_data/cases.rs"]
mod cases;

use cases::{Case, CASES, HELPER_SATYH};

/// The registry packages to install, in `layout-tests/xver_sweep.py`'s own
/// dependency-friendly order.
///
/// This is deliberately ITS OWN constant rather than a projection of
/// `CASES`, because the two lists genuinely differ at both ends:
///
/// * `fss` is here and is probed by no case — several packages depend on it,
///   nothing measures it directly;
/// * `math` is a case (`mathpkg`) and is NOT here — it is the port's own
///   bundled 0.0.6 corpus, copied out of `lib-rustyfi/dist/packages/`, and
///   asking the registry for it would fail.
///
/// `Case::package` still earns its place: it is what the installer knows a
/// package by, which differs from the case stem exactly once (`codeprinter` ->
/// `code-printer`). `check_cases_are_installable` below cross-checks the two
/// lists so a new case cannot silently probe a package nothing installed.
const PACKAGES: &[&str] = &[
    "base",
    "fss",
    "algorithm",
    "arrows",
    "azmath",
    "chemfml",
    "code-printer",
    "colorbox",
    "derive",
    "easytable",
    "enumitem",
    "figbox",
    "latexcmds",
    "lipsum",
    "matrixcd",
    "pagenumber",
    "quotation",
    "railway",
    "ruby",
    "siunitx",
    "texlogo",
    "uline",
];

// ---------------------------------------------------------------------------
// Paths, fonts, environment
// ---------------------------------------------------------------------------

/// The repo root — `crates/rustyfi/` up two, the same derivation
/// `xver_capstone.rs`'s `lib_root()` and `layout_fidelity.rs`'s `repo_root()`
/// use, so the test is independent of the working directory `cargo test` ran
/// from.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

/// A real TTF to set text in, so a failure is never a font-discovery one.
///
/// Any serif face does — the sweep only ever asks whether a document COMPILES,
/// and `find_regular_ttf`'s callers in `e2e.rs`/`xver_capstone.rs` want the
/// same thing. Deliberately process-free, unlike those two: they probe
/// `fc-match` first, and the entire premise of this file is that it shells out
/// to nothing. The static candidate list is the union of theirs and
/// `layout-tests/xver_sweep.py`'s `find_font`, plus a NARROW walk of `/nix/store`
/// (only into directory names containing `dejavu`, so this stays cheap on a
/// store with 10^5 entries).
///
/// `None` falls back to base-14, exactly as the Python did when it found no
/// `--font`.
fn find_regular_ttf() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("RUSTYFI_SWEEP_FONT").map(PathBuf::from) {
        if p.is_file() {
            return Some(p);
        }
    }
    for candidate in [
        "/usr/share/fonts/truetype/dejavu/DejaVuSerif.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/dejavu/DejaVuSerif.ttf",
        "/usr/share/fonts/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/TTF/DejaVuSerif.ttf",
        "/run/current-system/sw/share/fonts/truetype/DejaVuSans.ttf",
    ] {
        if Path::new(candidate).is_file() {
            return Some(PathBuf::from(candidate));
        }
    }
    if let Some(hit) = find_named_ttf(Path::new("/usr/share/fonts"), "DejaVuSerif.ttf", 6) {
        return Some(hit);
    }
    let store = Path::new("/nix/store");
    if store.is_dir() {
        for entry in std::fs::read_dir(store).ok()?.flatten() {
            let name = entry.file_name().to_string_lossy().to_lowercase();
            if !name.contains("dejavu") {
                continue;
            }
            if let Some(hit) = find_named_ttf(&entry.path(), "DejaVuSerif.ttf", 6) {
                return Some(hit);
            }
        }
    }
    None
}

/// Depth-bounded search for one file name under `dir`.
fn find_named_ttf(dir: &Path, name: &str, depth: usize) -> Option<PathBuf> {
    if depth == 0 || !dir.is_dir() {
        return None;
    }
    let mut subdirs = Vec::new();
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path.is_dir() {
            subdirs.push(path);
        } else if path.file_name().map(|f| f == name).unwrap_or(false) {
            return Some(path);
        }
    }
    subdirs
        .into_iter()
        .find_map(|d| find_named_ttf(&d, name, depth - 1))
}

/// `RUSTYFI_XVER_ONLY=a,b` — the Python's repeatable `--only`, as one env var.
/// A filter matching nothing is an error, never an empty (and therefore
/// vacuously green) sweep.
fn only_filter() -> Option<BTreeSet<String>> {
    let raw = std::env::var("RUSTYFI_XVER_ONLY").ok()?;
    let set: BTreeSet<String> = raw
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    (!set.is_empty()).then_some(set)
}

// ---------------------------------------------------------------------------
// Lib-root policy
// ---------------------------------------------------------------------------

/// Where the sweep's lib root comes from, and the guarantee that there IS one.
///
/// * `$RUSTYFI_XVER_ROOT` pointing at an assembled root → use it as is.
/// * `$RUSTYFI_XVER_ASSEMBLE` set → assemble one (network, minutes): into
///   `$RUSTYFI_XVER_ROOT` when named (kept, so the next run is seconds), else
///   into a temp dir the caller deletes when the sweep finishes — disk here is
///   tight and an unnamed root is nobody's cache.
/// * neither → panic naming both variables. There is deliberately no "skip
///   quietly" arm: a sweep over zero packages that reports success is the exact
///   failure mode this project has already been bitten by
///   (`xver-forked-set-verify-upstream` memory: "regex delete emptied
///   known_gaps() (vacuous green)").
///
/// Returns `(root, delete_when_done)`.
fn resolve_root() -> (PathBuf, bool) {
    let named = std::env::var_os("RUSTYFI_XVER_ROOT").map(PathBuf::from);
    let assemble = std::env::var_os("RUSTYFI_XVER_ASSEMBLE").is_some();

    match (named, assemble) {
        (Some(root), false) => {
            assert!(
                root.join("dist").join("packages").is_dir(),
                "RUSTYFI_XVER_ROOT={} has no dist/packages/ — that is not an assembled \
                 sweep root. Pass RUSTYFI_XVER_ASSEMBLE=1 to build one there \
                 (network + minutes, and `--features http` for https tarballs).",
                root.display()
            );
            (root, false)
        }
        (Some(root), true) => {
            assemble_root(&root);
            (root, false)
        }
        (None, true) => {
            let root = std::env::temp_dir()
                .join(format!("rustyfi-xver-sweep-root-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            assemble_root(&root);
            // Deleted at the end: this machine's disk sits near full, and an
            // unnamed root cannot be reused by a later run anyway (the pid is
            // in its name). Name one with RUSTYFI_XVER_ROOT to keep it.
            (root, true)
        }
        (None, false) => panic!(
            "no lib root for the cross-version sweep.\n\
             \n\
             Set RUSTYFI_XVER_ROOT=<dir> to an already-assembled root (the port's \n\
             bundled 0.0.6 corpus under <dir>/dist/packages/, plus the {} registry \n\
             packages installed on top), or set RUSTYFI_XVER_ASSEMBLE=1 to have this \n\
             test assemble one — that reaches the network and takes minutes, and \n\
             needs `--features http` for the registry's https tarballs.\n\
             \n\
             This test does NOT skip: a sweep with no packages to sweep would report \n\
             success over zero cases, which is worse than a failure.",
            PACKAGES.len()
        ),
    }
}

/// The two structural facts the sweep's meaning depends on, checked before a
/// single document is compiled.
///
/// The `dist-v01/` half is the one that matters. `resolve_require` prefers the
/// requesting file's own generation, so a root that ALSO carries the 0.1 corpus
/// hands a 0.1 probe the 0.1 package: every case passes, nothing crosses, and
/// the sweep has measured nothing at all. `xver_capstone.rs` builds a symlink
/// farm to avoid precisely this; a pre-assembled root handed in by env var
/// could just as easily be `lib-rustyfi/` itself, so say so out loud.
fn check_root(root: &Path) {
    let packages = root.join("dist").join("packages");
    assert!(
        packages.is_dir(),
        "{} has no dist/packages/ — nothing to @require:",
        root.display()
    );
    assert!(
        std::fs::read_dir(&packages)
            .map(|mut d| d.next().is_some())
            .unwrap_or(false),
        "{} is empty",
        packages.display()
    );
    assert!(
        !root.join("dist-v01").exists(),
        "{} carries a dist-v01/ — the 0.1 corpus must NOT be visible to this sweep. \
         With both corpora present, `resolve_require` prefers the requesting file's \
         own generation, so every 0.1 probe would resolve the 0.1 package, cross \
         nothing, and pass. (This is the trap `xver_capstone.rs`'s \
         `v006_only_lib_root` exists to dodge; do not point RUSTYFI_XVER_ROOT at \
         `lib-rustyfi/` itself.)",
        root.display()
    );
}

/// Every case's package must be something the root actually has: either a
/// member of [`PACKAGES`] (installed from the registry) or a name already in
/// the bundled 0.0.6 corpus (`mathpkg` -> `math`, which is `math.satyh` and is
/// deliberately never fetched).
///
/// The two lists are independent by necessity, and independent lists drift. A
/// case added without its install would otherwise show up as an ordinary
/// `refuse` with an unresolved-`@require:` message and be read as a bridge
/// result; this turns it into what it is, a harness mistake.
fn check_cases_are_installable(root: &Path) {
    let dir = root.join("dist").join("packages");
    for case in CASES {
        if case.package.is_empty() || PACKAGES.contains(&case.package) {
            continue;
        }
        let bundled = dir.join(case.package).is_dir()
            || dir.join(format!("{}.satyh", case.package)).is_file()
            || dir.join(format!("{}.satyg", case.package)).is_file();
        assert!(
            bundled,
            "case `{}` probes package `{}`, which is neither in this file's PACKAGES \
             list nor present in the bundled 0.0.6 corpus at {} — it would refuse for \
             want of an install, not for anything the bridge did",
            case.name,
            case.package,
            dir.display()
        );
    }
}

/// `<root>/dist/packages/` = the port's bundled 0.0.6 corpus + registry
/// installs — `layout-tests/xver_sweep.py`'s `assemble_root`, as library calls.
///
/// The order is [`PACKAGES`]'s own, which puts the shared dependencies
/// (`base`, `fss`) ahead of their dependents. Installs are independent
/// tarballs, but
/// `force` orphans a colliding prior receipt's files, so the order is not
/// purely cosmetic.
///
/// `dist-v01/` is never created. That is the whole point — see [`check_root`].
fn assemble_root(root: &Path) {
    let src = repo_root()
        .join("lib-rustyfi")
        .join("dist")
        .join("packages");
    let dst = root.join("dist").join("packages");
    std::fs::create_dir_all(&dst).expect("create <root>/dist/packages");
    copy_dir_all(&src, &dst).unwrap_or_else(|e| panic!("copy the bundled 0.0.6 corpus: {e}"));

    // The repository the Python named with `--config <repo>/config.toml`: this
    // repo's shipped default, Satyrographos' own opam registry. Read here
    // rather than through `sg::config::load()` so the sweep does not silently
    // depend on the developer's `~/.config/rustyfi/config.toml`.
    let cfg_path = repo_root().join("config.toml");
    let config =
        sg::config::read(&cfg_path).unwrap_or_else(|e| panic!("read {}: {e}", cfg_path.display()));
    let registry = config
        .registry()
        .cloned()
        .unwrap_or_else(|| panic!("{} declares no [registry]", cfg_path.display()));
    let offline = std::env::var_os("RUSTYFI_XVER_OFFLINE").is_some();
    let reg_opts = sg::RegistryOptions {
        url: None,
        offline,
        mirrors: registry.mirrors.clone(),
        kind: registry.kind,
        ..Default::default()
    };

    eprintln!("assembling lib root at {}", root.display());
    for name in PACKAGES {
        // `main.rs`'s `install_one`, registry branch: `prefer_library` so that a
        // manifest declaring several libraries yields the one whose name was
        // asked for, `dest` (not `lib_root`) because the sweep root is a
        // destination we chose rather than a discovered install prefix, and
        // `force` because the sweep re-runs into a warm root.
        let opts = sg::InstallOptions {
            prefer_library: Some((*name).to_string()),
            offline,
            verbose: true,
            lang: None,
            lib_root: None,
            dest: Some(root.to_path_buf()),
            libraries: None,
            force: true,
        };
        match sg::install_registry(name, None, &opts, &reg_opts, registry.url.as_deref()) {
            Ok((_, resolved)) => eprintln!("  installed {name} {}", resolved.version),
            Err(e) => panic!(
                "install {name}: {e}\n\
                 (an `http(s)://` registry tarball needs `--features http`; \
                 RUSTYFI_XVER_OFFLINE=1 resolves from the archive cache only)"
            ),
        }
    }
}

fn copy_dir_all(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let to = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_all(&entry.path(), &to)?;
        } else {
            std::fs::copy(entry.path(), &to)?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// The compile, mirroring `main.rs`'s `cmd_compile`
// ---------------------------------------------------------------------------

/// What one compile produced: whether it succeeded, and how many of the loaded
/// dependencies were 0.0.6-tagged.
///
/// The second number is the anti-vacuity instrument. A crossing case that
/// compiles having loaded ZERO `V0_0` dependencies did not cross anything — it
/// resolved a same-generation package, or resolved nothing at all — and a sweep
/// where that is true of every passing case is measuring its own lib root
/// rather than the bridge.
///
/// It is PRINTED per case and ENFORCED only in aggregate (see `sweep`'s closing
/// assert). Per case it would be too sharp: `load_legacy`'s `sniff_version`
/// runs before the `/dist/packages/` provenance fallback, so a published 0.0.6
/// package that happened to sniff `Some(V0_1)` would be a legitimate zero, and
/// this must not invent a failure out of that. Measured on the real corpus the
/// counts run 1..34 and no passing case is zero.
struct Outcome {
    ok: bool,
    err: String,
    v006_deps: usize,
}

/// Load + compile one document exactly as `cmd_compile` does, minus the render,
/// the cache and the aux file (a fresh empty `AuxTable` per compile is
/// `--no-aux`; there is no cache to disable).
///
/// `version` is the case's `--lang`: `V0_1` for a crossing probe, `V0_0` for a
/// control. Axis B is sniffed from the document rather than assumed, because
/// that is what `resolve_version_and_mode` does — the probes carry `@require:`
/// headers and so land on `Legacy`, and if one ever grew a `use` header the
/// mirror should notice rather than silently mis-load it.
fn compile_one(
    entry: &Path,
    root: &Path,
    version: RustyfiVersion,
    metrics: &dyn rustyfi_backend::FontMetrics,
) -> Outcome {
    let sniff = std::fs::read_to_string(entry)
        .ok()
        .map(|src| rustyfi_syntax::sniff_headers(&src))
        .unwrap_or_default();
    let mode = if sniff.envelope_headers {
        rustyfi_loader::LoadMode::Envelopes { deps: None }
    } else {
        rustyfi_loader::LoadMode::Legacy
    };

    let program = match rustyfi_loader::load(
        entry,
        &rustyfi_loader::LoadOptions {
            lib_root: Some(root.to_path_buf()),
            fallback_roots: Vec::new(),
            version,
            mode,
        },
    ) {
        Ok(p) => p,
        Err(e) => {
            return Outcome {
                ok: false,
                err: e.to_string(),
                v006_deps: 0,
            }
        }
    };

    // Counted over the DEPENDENCIES only (the entry is the last file, and its
    // own version is whatever we just pinned).
    let v006_deps = program.files[..program.files.len().saturating_sub(1)]
        .iter()
        .filter(|f| matches!(f.version, RustyfiVersion::V0_0))
        .count();

    let mut aux = rustyfi_lang::crossref::AuxTable::default();
    let result = match version {
        RustyfiVersion::V0_1 => {
            // 0.1 libraries are modules, not prelude-concatenable flat binding
            // lists — no merge_program; each file keeps its own FileV1 CST.
            rustyfi_lang::compile_document_v1_with_aux(&program.files, metrics, &mut aux)
                .map(|_| ())
        }
        _ => {
            // Slice X4a: a V0_0-rooted load whose dependency graph contains at
            // least one foreign V0_1 node routes through the xver entry point;
            // a pure-0.0.6 load (every control here, and every existing
            // fixture) takes the old merge_program path, byte-identical.
            let has_v01_dep = program
                .files
                .iter()
                .any(|f| matches!(f.cst, rustyfi_loader::LoadedCst::V0_1(_)));
            if has_v01_dep {
                rustyfi_lang::compile_document_v006_xver_with_aux(&program.files, metrics, &mut aux)
                    .map(|_| ())
            } else {
                let (merged, stages) = merge_program(program);
                rustyfi_lang::compile_document_cst_with_stages(&merged, metrics, &mut aux, &stages)
                    .map(|_| ())
            }
        }
    };

    match result {
        Ok(()) => Outcome {
            ok: true,
            err: String::new(),
            v006_deps,
        },
        Err(e) => Outcome {
            ok: false,
            err: e.to_string(),
            v006_deps,
        },
    }
}

/// Verbatim copy of `main.rs`'s `merge_program` — concatenate the
/// dependency-ordered library preludes ahead of the entry's own, recording each
/// library's `@stage:` header against the prelude slots its bindings land in
/// (concatenation drops the headers themselves, and a stage is a property of
/// the BINDINGS, not of the file as a document).
///
/// Duplicated rather than imported because it lives in a `[[bin]]`, which
/// exports nothing. Keep the two in sync.
fn merge_program(
    program: rustyfi_loader::LoadedProgram,
) -> (
    rustyfi_syntax::cst::File,
    HashMap<usize, rustyfi_lang::types::Stage>,
) {
    fn as_v006(cst: rustyfi_loader::LoadedCst) -> rustyfi_syntax::cst::File {
        match cst {
            rustyfi_loader::LoadedCst::V0_0(f) => f,
            rustyfi_loader::LoadedCst::V0_1(_) => unreachable!(
                "merge_program is the V0_0-only path; a V0_1 dependency routes through \
                 compile_document_v006_xver_with_aux"
            ),
        }
    }

    let mut files = program.files;
    let entry = files.pop().expect("loader always yields the entry last");
    let entry_cst = as_v006(entry.cst);
    let mut prelude = Vec::new();
    let mut stages = HashMap::new();
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

/// One line, with the (long, temp-dir-flavoured) lib-root and document paths
/// stripped, so two runs' output diffs cleanly — `layout-tests/xver_sweep.py`'s
/// `short`. There is no `Error: ` prefix to strip here and no `<doc>.saty: `
/// preamble: those were `main.rs`'s framing of a `CompileError`, and this
/// harness holds the error value itself.
fn short(msg: &str, subs: &[(&Path, &str)]) -> String {
    let mut s = msg.split_whitespace().collect::<Vec<_>>().join(" ");
    for (path, with) in subs {
        s = s.replace(&format!("{}/", path.display()), with);
    }
    const LIMIT: usize = 220;
    if s.chars().count() > LIMIT {
        s = s.chars().take(LIMIT - 1).collect::<String>() + "…";
    }
    s
}

/// Both documents of one case, written where the loader expects them: the entry
/// beside `h.satyh`, because a crossing probe reaches its 0.1 scaffolding with
/// `@import: h` — a same-directory-relative header resolved by `resolve_import`
/// independently of `lib_root`, which is exactly what leaves `lib_root` free to
/// be a 0.0.6-only corpus (see `xver_capstone.rs`'s module doc). The helper is
/// written for the control too even though a 0.0.6 document never imports it;
/// an unreferenced file is never opened.
fn write_case(dir: &Path, case: &Case) -> (PathBuf, Option<PathBuf>) {
    std::fs::create_dir_all(dir).expect("create the case work dir");
    std::fs::write(dir.join("h.satyh"), HELPER_SATYH).expect("write h.satyh");
    let cross = dir.join(format!("{}.saty", case.name));
    std::fs::write(&cross, case.v01).expect("write the crossing probe");
    let control = case.v006.map(|src| {
        let path = dir.join(format!("{}-v006.saty", case.name));
        std::fs::write(&path, src).expect("write the 0.0.6 control");
        path
    });
    (cross, control)
}

// ---------------------------------------------------------------------------
// The sweep
// ---------------------------------------------------------------------------

#[test]
#[ignore = "needs an assembled lib root of 22 registry packages (network + minutes); \
            run with --ignored and RUSTYFI_XVER_ROOT / RUSTYFI_XVER_ASSEMBLE"]
fn xver_sweep_matches_baseline() {
    // These documents pull in whole published packages, so the recursive-
    // descent parser and the elaborator go far deeper than a fixture makes
    // them: `xver_capstone.rs`'s 64 MB is NOT enough here (`algorithm`, the
    // first case, overflows it in a debug build), which is precisely why
    // `main.rs` spawns its own work on 256 MB rather than the default 8. Match
    // the CLI's figure — the Python sweep was measuring documents compiled on
    // that stack, and a stack overflow is an abort, not a `refuse`: it takes
    // the whole harness down and reports nothing.
    let handle = std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(sweep)
        .expect("spawn the big-stack sweep thread");
    // Re-raise the worker's own panic payload rather than `expect`ing over it:
    // every failure this file reports is a panic MESSAGE (the drift list, the
    // missing-root instructions, the dist-v01 refusal), and `expect` would
    // replace all of them with one `Any { .. }`.
    if let Err(payload) = handle.join() {
        std::panic::resume_unwind(payload);
    }
}

fn sweep() {
    let only = only_filter();
    let selected: Vec<&Case> = CASES
        .iter()
        .filter(|c| only.as_ref().map(|f| f.contains(c.name)).unwrap_or(true))
        .collect();
    if let Some(filter) = &only {
        assert!(
            !selected.is_empty(),
            "RUSTYFI_XVER_ONLY={:?} matches no case; known: {:?}",
            filter,
            CASES.iter().map(|c| c.name).collect::<Vec<_>>()
        );
    }
    // The corpus itself must not be empty, whatever the reason. A sweep over
    // zero cases that prints `crossing 0/0` and exits 0 is the failure mode
    // this whole file is written to make impossible.
    assert!(!selected.is_empty(), "no cases to sweep");

    let (root, cleanup) = resolve_root();
    check_root(&root);
    check_cases_are_installable(&root);

    // `main.rs`'s `resolve_font_store`, with `--font <ttf>` when one was found
    // (flags win over any `dist/hash/fonts.satysfi-hash` the installs left in
    // the root) and the root's own configuration otherwise — the same
    // precedence, so a font problem here would be a font problem there.
    let font = find_regular_ttf();
    match &font {
        Some(p) => eprintln!("font: {}", p.display()),
        None => eprintln!("font: none found; using the built-in base-14 metrics"),
    }
    let flags = rustyfi_pdf::FontFlags {
        regular: font.clone(),
        bold: None,
        oblique: None,
    };
    let store = rustyfi_pdf::FontRegistry::discover(Some(&root), None, &flags)
        .unwrap_or_else(|e| panic!("font configuration: {e}"))
        .map(|registry| {
            registry
                .build_store()
                .unwrap_or_else(|e| panic!("font configuration: {e}"))
        });
    let base14 = rustyfi_pdf::Base14Metrics;
    let metrics: &dyn rustyfi_backend::FontMetrics = match &store {
        Some(s) => s,
        None => &base14,
    };

    // Compile in a scratch directory so nothing is written next to the repo.
    let work = std::env::temp_dir().join(format!("rustyfi-xver-docs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&work);

    struct Row {
        name: &'static str,
        cross: Outcome,
        control: Option<Outcome>,
    }
    let mut rows: Vec<Row> = Vec::new();
    for case in &selected {
        let dir = work.join(case.name);
        let (cross_doc, control_doc) = write_case(&dir, case);
        let cross = compile_one(&cross_doc, &root, RustyfiVersion::V0_1, metrics);
        let control = control_doc
            .as_deref()
            .map(|doc| compile_one(doc, &root, RustyfiVersion::V0_0, metrics));
        rows.push(Row {
            name: case.name,
            cross,
            control,
        });
    }
    let _ = std::fs::remove_dir_all(&work);

    // ---- report, exactly `layout-tests/xver_sweep.py`'s layout ----
    let subs: [(&Path, &str); 2] = [(root.as_path(), "<root>/"), (work.as_path(), "<work>/")];
    let width = rows.iter().map(|r| r.name.len()).max().unwrap_or(0);
    let mut drift: Vec<String> = Vec::new();
    let mut crossing = 0usize;
    let mut controls = 0usize;
    let mut crossed_something = 0usize;

    for (row, case) in rows.iter().zip(&selected) {
        let control_ok = row.control.as_ref().map(|c| c.ok).unwrap_or(false);
        if row.cross.ok {
            crossing += 1;
            if row.cross.v006_deps > 0 {
                crossed_something += 1;
            }
        }
        if control_ok {
            controls += 1;
        }
        println!(
            "{:<width$}  xver={}  v0.0.6={}  ({} v0.0.6 dep(s))",
            row.name,
            if row.cross.ok { "CROSS " } else { "refuse" },
            if control_ok { "ok  " } else { "FAIL" },
            row.cross.v006_deps,
            width = width
        );
        if !row.cross.ok {
            println!(
                "{:<width$}    xver : {}",
                "",
                short(&row.cross.err, &subs),
                width = width
            );
        }
        if let Some(control) = &row.control {
            if !control.ok {
                // Printed for every failing control, not only when the crossing
                // also failed: a package whose plain-0.0.6 compile is broken was
                // never the boundary's to fix, and that is the single most
                // misread line in the original audit.
                println!(
                    "{:<width$}    0.0.6: {}",
                    "",
                    short(&control.err, &subs),
                    width = width
                );
            }
        }

        if row.cross.ok != case.expect_cross {
            drift.push(format!(
                "{}: crossing {} (baseline {}, got {})",
                row.name,
                if case.expect_cross {
                    "REGRESSED"
                } else {
                    "now PASSES"
                },
                case.expect_cross,
                row.cross.ok
            ));
        }
        if control_ok != case.expect_v006 {
            drift.push(format!(
                "{}: 0.0.6 control {} (baseline {}, got {})",
                row.name,
                if case.expect_v006 {
                    "REGRESSED"
                } else {
                    "now PASSES"
                },
                case.expect_v006,
                control_ok
            ));
        }
    }

    println!(
        "\ncrossing {crossing}/{total}   0.0.6 controls {controls}/{total}",
        total = rows.len()
    );

    if cleanup {
        let _ = std::fs::remove_dir_all(&root);
    }

    // The vacuity gate. Enforced in aggregate rather than per case (see
    // `Outcome`): if NOTHING that passed loaded a 0.0.6 dependency, the lib
    // root is not the one this sweep needs and every green above is an
    // artefact — a far more likely mistake than 22 simultaneous bridge
    // regressions, and one the numbers alone look perfectly healthy under.
    assert!(
        crossing == 0 || crossed_something > 0,
        "{crossing} crossing case(s) compiled, but not one of them loaded a single \
         V0_0 dependency — the 0.1 probes resolved same-generation packages instead \
         of crossing. Check that {} really holds the 0.0.6 corpus and no dist-v01/.",
        root.display()
    );

    assert!(
        drift.is_empty(),
        "DRIFT vs the baseline in xver_sweep_data/cases.rs:\n  {}\n\n\
         (re-record `expect_cross`/`expect_v006` there once the change is intended; \
         an unexpected PASS is drift too — a baseline that absorbs improvements \
         silently stops describing anything)",
        drift.join("\n  ")
    );
}
