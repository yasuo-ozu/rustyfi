//! Corpus typecheck coverage, as ordinary integration tests.
//!
//! Every input this crate expects to typecheck is listed explicitly below,
//! with its source embedded via `include_str!`, and asserted directly — no
//! snapshot file.
//!
//! Two families:
//!
//! - [`DOCUMENTS`] — entry documents. The embedded source is materialized
//!   BESIDE the fixture it came from, so a relative `@import:` resolves
//!   exactly as it does for the original, then loaded, elaborated and
//!   `typecheck_verbose`d.
//! - [`PACKAGES`] — the bundled packages under `lib-rustyfi/dist*/packages/`,
//!   each probed the way a document reaches it: a synthetic `@require: <pkg>
//!   in ()` entry (`probe_src`).
//!
//! Each case pins two things: that the input typechecks with NO warnings,
//! and WHICH generation the loader resolved it as — a file silently
//! switching generation (e.g. `@require:` resolving to a different corpus)
//! is a real regression that leaves the verdict alone.
//!
//! Every case here SUCCEEDS, so no error message text is exercised;
//! negative typecheck coverage lives in the unit suites. Inputs that
//! cannot typecheck at all are recorded in `typecheck_known_gaps.rs` with
//! the reason.

use rustyfi_lang::{elaborate, primitives, typecheck, v1};
use rustyfi_loader::{LoadOptions, LoadedCst, LoadedFile, LoadedProgram};
use rustyfi_syntax::RustyfiVersion;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

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

/// Merges a loader-resolved V0.0.6 program's preludes into one synthetic
/// `cst::File`.
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

/// V0_1 analogue of `merge_program_v006`, mirroring `lib.rs`'s
/// `compile_document_v1_with_trials` assembly step, stopping short of
/// eval. Also returns the `v006_indices` set so callers can wrap
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
        // mirrors production's dep loop, a MIXED-version list — a
        // V0_1 dep is lowered; a V0_0 dep splices its `cst::File.prelude`
        // directly and records its indices into `v006_indices`, so
        // `elaborate_program_with_versions` can run exactly as production
        // does. Production also runs the forked-name guard here; this
        // harness splices guard-free (only needs to not panic and emit a
        // stable line).
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

/// Tries V0_0 first, falling back to V0_1 if the V0_0 load fails (the
/// corpus mixes both generations). Returns the generation that actually
/// loaded, plus the typechecker's warnings.
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
            // The scope is tagged V0_1 (`Scope::new` defaults V0_0, which
            // would elaborate every mixed-version input as 0.0.6); when a
            // V0_0 dependency was spliced the name set is the UNION of both
            // versions', since elaboration resolves names before
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
                elaborate::elaborate_program_with_versions(
                    &file,
                    &scope,
                    &v006_indices,
                    &std::collections::HashMap::new(),
                    None,
                )
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

struct Doc {
    path: &'static str,
    /// Kept beside `path` so the case table reads as source-plus-location even
    /// though the loader reads the file itself.
    #[allow(dead_code)]
    src: &'static str,
    version: RustyfiVersion,
}

struct Pkg {
    name: &'static str,
    version: RustyfiVersion,
}

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

/// Runs on a 64 MiB stack and reports EVERY failure at once — elaboration
/// over a real document class's merged prelude overflows the default
/// 2 MiB test stack.
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
            // `doc.src`'s `include_str!` IS the file's bytes; do NOT write
            // it back out beside the original — a no-op that would cost a
            // sweep/Drop/gitignore rules for debris an aborted run leaves.
            // `include_str!` also fails the build if the path stops
            // existing.
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
