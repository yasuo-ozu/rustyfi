//! Corpus typecheck coverage, as ordinary integration tests.
//!
//! Every input this crate expects to typecheck is listed explicitly below, with
//! its source embedded via `include_str!`, and asserted directly. There is no
//! snapshot file: the expectation for each case is written next to the case.
//!
//! Two families:
//!
//! - [`DOCUMENTS`] — entry documents. The embedded source is materialized BESIDE
//!   the fixture it came from, so a relative `@import:` resolves exactly as it
//!   does for the original, then loaded, elaborated and `typecheck_verbose`d.
//! - [`PACKAGES`] — the bundled packages under `lib-rustyfi/dist*/packages/`,
//!   each probed the way a document reaches it: a synthetic `@require: <pkg> in
//!   ()` entry, which is the string literal in `probe_src`.
//!
//! Each case pins two things: that the input typechecks with NO warnings, and
//! WHICH generation the loader resolved it as. The second matters because a
//! file silently switching generation — say `@require:` resolution changing
//! which corpus a name comes from — is a real regression that leaves the
//! verdict alone.
//!
//! What this does NOT cover, stated plainly so it is not mistaken for more than
//! it is: every case here SUCCEEDS, so no error message text is exercised.
//! Negative typecheck coverage lives in the unit suites (`types_unify.rs`,
//! `typecheck.rs`'s own tests, and the `expect_err` cases scattered through the
//! integration tests). Inputs that cannot typecheck at all are recorded in
//! `typecheck_known_gaps.rs` with the reason.

use rustyfi_lang::{elaborate, primitives, typecheck, v1};
use rustyfi_loader::{LoadOptions, LoadedCst, LoadedFile, LoadedProgram};
use rustyfi_syntax::RustyfiVersion;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Inputs asserted in `typecheck_known_gaps.rs` instead of here. Keep the two

/// This repo's root, resolved relative to this crate's own manifest
/// directory (`crates/rustyfi-lang`) — same convention as
/// `stdlib_tier0.rs`'s `lib_root`.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root should exist")
}

fn lib_root() -> PathBuf {
    repo_root().join("lib-rustyfi")
}

/// A uniquely-named temp `.saty` entry file, cleaned up on drop — used only
/// for the synthetic `@require: <pkg> in ()` package-probe entries.
struct TempDoc(PathBuf);

impl TempDoc {
    fn new(tag: &str, src: &str) -> TempDoc {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "rustyfi-lang-typecheck-golden-{tag}-{}-{}.saty",
            std::process::id(),
            n
        ));
        fs::write(&path, src).expect("write temp golden entry");
        TempDoc(path)
    }
}

impl Drop for TempDoc {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn as_v006(cst: &LoadedCst) -> &rustyfi_syntax::cst::File {
    match cst {
        LoadedCst::V0_0(f) => f,
        LoadedCst::V0_1(_) => unreachable!("as_v006 called on a V0_1-loaded file"),
    }
}

/// Merge a loader-resolved V0.0.6 program's preludes into one synthetic
/// `cst::File` — mirrors `rustyfi`'s `merge_program` /
/// `stdlib_tier0.rs`'s helper of the same name.
fn merge_program_v006(program: LoadedProgram) -> rustyfi_syntax::cst::File {
    let mut files = program.files;
    let entry = files.pop().expect("loader always yields the entry last");
    let entry_cst = as_v006(&entry.cst).clone();
    let mut prelude = Vec::new();
    for lib in &files {
        prelude.extend(as_v006(&lib.cst).prelude.clone());
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

fn as_v01(f: &LoadedFile) -> &rustyfi_syntax::cst_v1::FileV1 {
    match &f.cst {
        LoadedCst::V0_1(cst) => cst,
        LoadedCst::V0_0(_) => unreachable!("as_v01 called on a V0_0-loaded file"),
    }
}

/// V0_1 analogue of `merge_program_v006` — mirrors `lib.rs`'s
/// `compile_document_v1_with_trials` assembly step (lowering each
/// dependency + the entry through `v1::lower`), stopping short of eval.
/// Also returns the X2a `v006_indices` set (mirrors production's — see
/// `elaborate::elaborate_program_with_versions`), so callers can wrap
/// spliced V0_0 bindings the same way production does.
fn merge_program_v01(
    files: &[LoadedFile],
) -> Result<(rustyfi_syntax::cst::File, std::collections::HashSet<usize>), v1::lower::LowerError> {
    let (entry, deps) = files
        .split_last()
        .expect("loader always yields at least the entry file");
    let mut prelude = Vec::new();
    let mut v006_indices = std::collections::HashSet::new();
    for dep in deps {
        // X1/X2a (design-cross-version-import.md §5, §"Slice X2 — per-group
        // primitive environment"): mirror production's
        // `compile_document_v1_with_trials` dep loop, which is now a
        // MIXED-version list. A V0_1 dep is lowered as before; a V0_0 dep (a
        // 0.0.6-corpus `@require:` target reached under a V0_1 entry) splices
        // its `cst::File.prelude` bindings directly, and its contributed
        // top-level indices are recorded into `v006_indices` — mirroring
        // production's index bookkeeping — so `one_line`'s V0_1 branch can
        // call `elaborate::elaborate_program_with_versions` the same way
        // `compile_document_v1_with_trials` does, keeping the two in sync
        // for a mixed-version fixture's `Ast::VersionScope` shape. Production
        // ALSO runs the forked-name guard on this path, but this
        // typecheck-differential harness only needs to not panic + emit a
        // stable line, so it still splices unconditionally, guard-free. (The
        // V0_0-first branch in `one_line` — the 0.0.6 golden lines — never
        // reaches here and is untouched by X1/X2a.)
        match &dep.cst {
            LoadedCst::V0_1(cst) => prelude.extend(v1::lower::lower_file_v1(cst)?),
            LoadedCst::V0_0(cst) => {
                let start = prelude.len();
                prelude.extend(cst.prelude.iter().cloned());
                v006_indices.extend(start..prelude.len());
            }
        }
    }
    let entry_cst = as_v01(entry);
    let body = v1::lower::lower_document_v1(entry_cst)?;
    let eoi = match entry_cst {
        rustyfi_syntax::cst_v1::FileV1::Document { eoi, .. } => eoi.clone(),
        _ => unreachable!("lower_document_v1 already rejected a Library entry"),
    };
    Ok((
        rustyfi_syntax::cst::File {
            headers: Vec::new(),
            prelude,
            in_kw: Some(rustyfi_syntax::leaf::KwIn(rustyfi_syntax::Span::default())),
            body: Some(body),
            eoi,
        },
        v006_indices,
    ))
}

/// Load + elaborate + `typecheck_verbose` one entry file, trying V0_0 first
/// and falling back to V0_1 if the V0_0 load itself fails (the corpus mixes
/// both generations). Returns the generation that actually loaded plus the
/// typechecker's warnings, or a stage-tagged error string.
fn typecheck_entry(entry: &Path) -> Result<(RustyfiVersion, Vec<String>), String> {
    let opts_006 = LoadOptions {
        lib_root: Some(lib_root()),
        version: RustyfiVersion::V0_0,
        ..Default::default()
    };
    match rustyfi_loader::load(entry, &opts_006) {
        Ok(program) => {
            let file = merge_program_v006(program);
            let env = primitives::base_env();
            let store = rustyfi_lang::symbol::SymbolStore::new();
            let scope = elaborate::Scope::new(&store, env.names());
            let program = elaborate::elaborate_program(&file, &scope)
                .map_err(|e| format!("elaborate: {e}"))?;
            let warns = typecheck::typecheck_verbose(&program).map_err(|e| format!("{e}"))?;
            Ok((
                RustyfiVersion::V0_0,
                warns.iter().map(|w| format!("{w:?}")).collect(),
            ))
        }
        Err(v006_err) => {
            let opts_01 = LoadOptions {
                lib_root: Some(lib_root()),
                version: RustyfiVersion::V0_1,
                ..Default::default()
            };
            let program =
                rustyfi_loader::load(entry, &opts_01).map_err(|_| format!("load: {v006_err}"))?;
            let (file, v006_indices) =
                merge_program_v01(&program.files).map_err(|e| format!("lower: {e}"))?;
            let env = primitives::base_env_with_version(RustyfiVersion::V0_1);
            let store = rustyfi_lang::symbol::SymbolStore::new();
            // Mirror `lib.rs`'s `compile_document_v1_with_trials`: the scope is
            // tagged V0_1 (`Scope::new` is V0_0 — building this branch's scope
            // with it elaborated every mixed-version input as 0.0.6), and when a
            // V0_0 dependency was spliced the name set is the UNION of both
            // versions', since that dependency may legitimately name a
            // 0.0.6-only primitive and elaboration resolves names before
            // `Ast::VersionScope` means anything.
            let names: Vec<String> = if v006_indices.is_empty() {
                env.names()
            } else {
                let mut n = env.names();
                n.extend(primitives::base_env_with_version(RustyfiVersion::V0_0).names());
                n.sort();
                n.dedup();
                n
            };
            let scope = elaborate::Scope::new_with_version(&store, names, RustyfiVersion::V0_1);
            let program =
                elaborate::elaborate_program_with_versions(&file, &scope, &v006_indices, None)
                    .map_err(|e| format!("elaborate: {e}"))?;
            let warns = typecheck::typecheck_verbose_with_version(&program, RustyfiVersion::V0_1)
                .map_err(|e| format!("{e}"))?;
            Ok((
                RustyfiVersion::V0_1,
                warns.iter().map(|w| format!("{w:?}")).collect(),
            ))
        }
    }
}

/// An entry document: its own source, and the generation it must load as.
struct Doc {
    path: &'static str,
    src: &'static str,
    version: RustyfiVersion,
}

/// A bundled package, reached the way a document reaches it.
struct Pkg {
    name: &'static str,
    version: RustyfiVersion,
}

/// The synthetic entry that pulls one package in — the whole `.saty` source.
fn probe_src(pkg: &str) -> String {
    format!("@require: {pkg}\nin\n()")
}

const DOCUMENTS: &[Doc] = &[
    Doc {
        path: "crates/rustyfi/tests/fixtures/annot-hook.saty",
        src: include_str!("../../rustyfi/tests/fixtures/annot-hook.saty"),
        version: RustyfiVersion::V0_0,
    },
    Doc {
        path: "crates/rustyfi/tests/fixtures/cjk.saty",
        src: include_str!("../../rustyfi/tests/fixtures/cjk.saty"),
        version: RustyfiVersion::V0_0,
    },
    Doc {
        path: "crates/rustyfi/tests/fixtures/footnote.saty",
        src: include_str!("../../rustyfi/tests/fixtures/footnote.saty"),
        version: RustyfiVersion::V0_0,
    },
    Doc {
        path: "crates/rustyfi/tests/fixtures/graphics.saty",
        src: include_str!("../../rustyfi/tests/fixtures/graphics.saty"),
        version: RustyfiVersion::V0_0,
    },
    Doc {
        path: "crates/rustyfi/tests/fixtures/hook-page.saty",
        src: include_str!("../../rustyfi/tests/fixtures/hook-page.saty"),
        version: RustyfiVersion::V0_0,
    },
    Doc {
        path: "crates/rustyfi/tests/fixtures/href.saty",
        src: include_str!("../../rustyfi/tests/fixtures/href.saty"),
        version: RustyfiVersion::V0_0,
    },
    Doc {
        path: "crates/rustyfi/tests/fixtures/math-cramped.saty",
        src: include_str!("../../rustyfi/tests/fixtures/math-cramped.saty"),
        version: RustyfiVersion::V0_1,
    },
    Doc {
        path: "crates/rustyfi/tests/fixtures/math.saty",
        src: include_str!("../../rustyfi/tests/fixtures/math.saty"),
        version: RustyfiVersion::V0_0,
    },
    Doc {
        path: "crates/rustyfi/tests/fixtures/minimal.saty",
        src: include_str!("../../rustyfi/tests/fixtures/minimal.saty"),
        version: RustyfiVersion::V0_0,
    },
    Doc {
        path: "crates/rustyfi/tests/fixtures/multicolumn.saty",
        src: include_str!("../../rustyfi/tests/fixtures/multicolumn.saty"),
        version: RustyfiVersion::V0_0,
    },
    Doc {
        path: "crates/rustyfi/tests/fixtures/multifile/main.saty",
        src: include_str!("../../rustyfi/tests/fixtures/multifile/main.saty"),
        version: RustyfiVersion::V0_0,
    },
    Doc {
        path: "crates/rustyfi/tests/fixtures/page-footer.saty",
        src: include_str!("../../rustyfi/tests/fixtures/page-footer.saty"),
        version: RustyfiVersion::V0_0,
    },
    Doc {
        path: "crates/rustyfi/tests/fixtures/phase2.saty",
        src: include_str!("../../rustyfi/tests/fixtures/phase2.saty"),
        version: RustyfiVersion::V0_0,
    },
    Doc {
        path: "crates/rustyfi/tests/fixtures/phase2b.saty",
        src: include_str!("../../rustyfi/tests/fixtures/phase2b.saty"),
        version: RustyfiVersion::V0_0,
    },
    Doc {
        path: "crates/rustyfi/tests/fixtures/realfont.saty",
        src: include_str!("../../rustyfi/tests/fixtures/realfont.saty"),
        version: RustyfiVersion::V0_0,
    },
    Doc {
        path: "crates/rustyfi/tests/fixtures/table.saty",
        src: include_str!("../../rustyfi/tests/fixtures/table.saty"),
        version: RustyfiVersion::V0_0,
    },
    Doc {
        path: "crates/rustyfi/tests/fixtures/tier4.saty",
        src: include_str!("../../rustyfi/tests/fixtures/tier4.saty"),
        version: RustyfiVersion::V0_0,
    },
    Doc {
        path: "crates/rustyfi/tests/fixtures/twocolumn.saty",
        src: include_str!("../../rustyfi/tests/fixtures/twocolumn.saty"),
        version: RustyfiVersion::V0_0,
    },
    Doc {
        path: "crates/rustyfi/tests/fixtures/v01-annot-package.saty",
        src: include_str!("../../rustyfi/tests/fixtures/v01-annot-package.saty"),
        version: RustyfiVersion::V0_0,
    },
    Doc {
        path: "crates/rustyfi/tests/fixtures/v01-code-package.saty",
        src: include_str!("../../rustyfi/tests/fixtures/v01-code-package.saty"),
        version: RustyfiVersion::V0_0,
    },
    Doc {
        path: "crates/rustyfi/tests/fixtures/v01-equiv-006.saty",
        src: include_str!("../../rustyfi/tests/fixtures/v01-equiv-006.saty"),
        version: RustyfiVersion::V0_0,
    },
    Doc {
        path: "crates/rustyfi/tests/fixtures/v01-font.saty",
        src: include_str!("../../rustyfi/tests/fixtures/v01-font.saty"),
        version: RustyfiVersion::V0_1,
    },
    Doc {
        path: "crates/rustyfi/tests/fixtures/v01-footnote-scheme.saty",
        src: include_str!("../../rustyfi/tests/fixtures/v01-footnote-scheme.saty"),
        version: RustyfiVersion::V0_1,
    },
    Doc {
        path: "crates/rustyfi/tests/fixtures/v01-itemize.saty",
        src: include_str!("../../rustyfi/tests/fixtures/v01-itemize.saty"),
        version: RustyfiVersion::V0_1,
    },
    Doc {
        path: "crates/rustyfi/tests/fixtures/v01-math-equiv-006.saty",
        src: include_str!("../../rustyfi/tests/fixtures/v01-math-equiv-006.saty"),
        version: RustyfiVersion::V0_0,
    },
    Doc {
        path: "crates/rustyfi/tests/fixtures/v01-math-full.saty",
        src: include_str!("../../rustyfi/tests/fixtures/v01-math-full.saty"),
        version: RustyfiVersion::V0_1,
    },
    Doc {
        path: "crates/rustyfi/tests/fixtures/v01-math-package.saty",
        src: include_str!("../../rustyfi/tests/fixtures/v01-math-package.saty"),
        version: RustyfiVersion::V0_0,
    },
    Doc {
        path: "crates/rustyfi/tests/fixtures/v01-math-scripts.saty",
        src: include_str!("../../rustyfi/tests/fixtures/v01-math-scripts.saty"),
        version: RustyfiVersion::V0_1,
    },
    Doc {
        path: "crates/rustyfi/tests/fixtures/v01-math-seal-probe.saty",
        src: include_str!("../../rustyfi/tests/fixtures/v01-math-seal-probe.saty"),
        version: RustyfiVersion::V0_0,
    },
    Doc {
        path: "crates/rustyfi/tests/fixtures/v01-math.saty",
        src: include_str!("../../rustyfi/tests/fixtures/v01-math.saty"),
        version: RustyfiVersion::V0_1,
    },
    Doc {
        path: "crates/rustyfi/tests/fixtures/v01-minimal.saty",
        src: include_str!("../../rustyfi/tests/fixtures/v01-minimal.saty"),
        version: RustyfiVersion::V0_1,
    },
    Doc {
        path: "crates/rustyfi/tests/fixtures/v01-sealed.saty",
        src: include_str!("../../rustyfi/tests/fixtures/v01-sealed.saty"),
        version: RustyfiVersion::V0_1,
    },
    Doc {
        path: "crates/rustyfi/tests/fixtures/v01-stdja-book.saty",
        src: include_str!("../../rustyfi/tests/fixtures/v01-stdja-book.saty"),
        version: RustyfiVersion::V0_1,
    },
    Doc {
        path: "crates/rustyfi/tests/fixtures/v01-stdja-report.saty",
        src: include_str!("../../rustyfi/tests/fixtures/v01-stdja-report.saty"),
        version: RustyfiVersion::V0_1,
    },
    Doc {
        path: "crates/rustyfi/tests/fixtures/v01-stdja.saty",
        src: include_str!("../../rustyfi/tests/fixtures/v01-stdja.saty"),
        version: RustyfiVersion::V0_1,
    },
    Doc {
        path: "crates/rustyfi/tests/fixtures/v01-strings.saty",
        src: include_str!("../../rustyfi/tests/fixtures/v01-strings.saty"),
        version: RustyfiVersion::V0_1,
    },
];

const PACKAGES: &[Pkg] = &[
    Pkg {
        name: "annot",
        version: RustyfiVersion::V0_0,
    },
    Pkg {
        name: "bnf",
        version: RustyfiVersion::V0_0,
    },
    Pkg {
        name: "cd",
        version: RustyfiVersion::V0_0,
    },
    Pkg {
        name: "code",
        version: RustyfiVersion::V0_0,
    },
    Pkg {
        name: "color",
        version: RustyfiVersion::V0_0,
    },
    Pkg {
        name: "deco",
        version: RustyfiVersion::V0_0,
    },
    Pkg {
        name: "footnote-scheme",
        version: RustyfiVersion::V0_0,
    },
    Pkg {
        name: "geom",
        version: RustyfiVersion::V0_0,
    },
    Pkg {
        name: "gr",
        version: RustyfiVersion::V0_0,
    },
    Pkg {
        name: "hdecoset",
        version: RustyfiVersion::V0_0,
    },
    Pkg {
        name: "itemize",
        version: RustyfiVersion::V0_0,
    },
    Pkg {
        name: "list",
        version: RustyfiVersion::V0_0,
    },
    Pkg {
        name: "math",
        version: RustyfiVersion::V0_0,
    },
    Pkg {
        name: "mdja",
        version: RustyfiVersion::V0_0,
    },
    Pkg {
        name: "mitou-detail",
        version: RustyfiVersion::V0_0,
    },
    Pkg {
        name: "mitou-report",
        version: RustyfiVersion::V0_0,
    },
    Pkg {
        name: "option",
        version: RustyfiVersion::V0_0,
    },
    Pkg {
        name: "pervasives",
        version: RustyfiVersion::V0_0,
    },
    Pkg {
        name: "picture",
        version: RustyfiVersion::V0_0,
    },
    Pkg {
        name: "progsynt",
        version: RustyfiVersion::V0_0,
    },
    Pkg {
        name: "proof",
        version: RustyfiVersion::V0_0,
    },
    Pkg {
        name: "standalone",
        version: RustyfiVersion::V0_0,
    },
    Pkg {
        name: "stdja",
        version: RustyfiVersion::V0_0,
    },
    Pkg {
        name: "stdja-mini",
        version: RustyfiVersion::V0_0,
    },
    Pkg {
        name: "stdjabook",
        version: RustyfiVersion::V0_0,
    },
    Pkg {
        name: "stdjareport",
        version: RustyfiVersion::V0_0,
    },
    Pkg {
        name: "table",
        version: RustyfiVersion::V0_0,
    },
    Pkg {
        name: "tabular",
        version: RustyfiVersion::V0_0,
    },
    Pkg {
        name: "tabularx",
        version: RustyfiVersion::V0_0,
    },
    Pkg {
        name: "vdecoset",
        version: RustyfiVersion::V0_0,
    },
];

/// Run `cases` on a 64 MiB stack and report EVERY failure at once — elaboration
/// and typechecking recurse deeply over a real document class's merged prelude,
/// which overflows the default 2 MiB test stack.
fn check_all(what: &str, run: impl FnOnce() -> Vec<String> + Send + 'static) {
    let failures = std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(run)
        .expect("spawn big-stack thread")
        .join()
        .expect("corpus typecheck thread panicked");
    assert!(
        failures.is_empty(),
        "{} of {what} did not typecheck as expected:\n  {}",
        failures.len(),
        failures.join("\n  ")
    );
}

#[test]
fn every_corpus_document_typechecks() {
    check_all("corpus document(s)", || {
        let root = repo_root();
        let mut failures = Vec::new();
        for doc in DOCUMENTS {
            // Load the fixture itself. `doc.src` is `include_str!` of this very
            // path, so it IS the file's bytes — writing it back out to a
            // stand-in beside the original and loading that was a provable
            // no-op, and it cost a sweep, a Drop, and two `.gitignore` rules
            // for debris an aborted run could leave among real fixtures. The
            // literal stays because it is what makes each case readable here,
            // and `include_str!` fails the build if the path stops existing.
            match typecheck_entry(&root.join(doc.path)) {
                Ok((version, warns)) if version == doc.version && warns.is_empty() => {}
                Ok((version, warns)) => failures.push(format!(
                    "{}: expected {:?} with no warnings, got {version:?} with {warns:?}",
                    doc.path, doc.version
                )),
                Err(e) => failures.push(format!("{}: {e}", doc.path)),
            }
        }
        failures
    });
}

#[test]
fn every_bundled_package_typechecks() {
    check_all("bundled package(s)", || {
        let mut failures = Vec::new();
        for pkg in PACKAGES {
            let tmp = TempDoc::new(pkg.name, &probe_src(pkg.name));
            match typecheck_entry(&tmp.0) {
                Ok((version, warns)) if version == pkg.version && warns.is_empty() => {}
                Ok((version, warns)) => failures.push(format!(
                    "package {}: expected {:?} with no warnings, got {version:?} with {warns:?}",
                    pkg.name, pkg.version
                )),
                Err(e) => failures.push(format!("package {}: {e}", pkg.name)),
            }
        }
        failures
    });
}
