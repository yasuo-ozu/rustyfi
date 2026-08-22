//! The inputs the whole-corpus walk (`typecheck_corpus.rs`) cannot
//! typecheck, pinned here individually with the REASON each one fails —
//! not as a bare `ERR` in a golden file, which cannot say whether a
//! failure is intended or is an artifact of the harness's own prelude
//! merge rather than the compiler's actual verdict. Each source is
//! embedded via `include_str!` and the corpus walk skips these paths, so
//! its explicit case list becomes exactly "everything that should
//! typecheck, does".
//!
//! Five remain, none a port bug. Four are simply not standalone entry
//! documents: the corpus walk feeds every source file in as an entry,
//! including ones that only ever appear behind an `@require:`.
//!
//! The fifth is the cross-version capstone, excluded for a CORRECT reason:
//! `@require:` now prefers the entry's own generation, so as a 0.1 entry
//! against the full lib root it resolves `list` to the 0.1 corpus, whose
//! `List` has `fold` where the fixture (deliberately written against
//! 0.0.6's) calls `fold-left`. `xver_capstone.rs` exercises it properly,
//! against a lib root exposing only the 0.0.6 corpus.
//!
//! `Why::CrossVersionForkedBuiltin` is deliberately KEPT with no current
//! users: it is the discriminating machinery for the gate, which still
//! exists and still rejects (`font`, `math-text`, `math-boxes`, `page`).

use rustyfi_lang::{elaborate, primitives, typecheck};
use rustyfi_loader::{LoadOptions, LoadedCst, LoadedProgram};
use rustyfi_syntax::RustyfiVersion;
use std::fs;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root should exist")
}

fn lib_root() -> PathBuf {
    repo_root().join("lib-rustyfi")
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Why {
    /// Not a standalone entry: a LIBRARY (`.satyh` with no `in …` body), or a
    /// 0.1 file the 0.0.6 parser rejects and whose 0.1 load also refuses it.
    NotAnEntryDocument,
    /// A 0.1 document reaching a 0.0.6 package whose exports mention a
    /// builtin that really is version-forked; the compiler refuses these
    /// deliberately. `name`
    /// and `slice` are both asserted, so a case failing on a DIFFERENT
    /// builtin or gate shows up as a change rather than a stale "pass".
    #[allow(dead_code)] // see the module doc comment: kept deliberately
    CrossVersionForkedBuiltin {
        slice: &'static str,
        name: &'static str,
    },
    /// NOT a port gap: these compile and render through the real pipeline.
    /// The GOLDEN HARNESS can't handle them because it merges every
    /// dependency's prelude and elaborates under ONE version — a mixed-
    /// version program then hits a 0.1 dependency's labeled optionals while
    /// "compiled as 0.0.6". The assertion is deliberately on the HARNESS's
    /// failure text: if the harness learns to merge per-version, this stops
    /// matching and the case moves into `typecheck_corpus.rs`.
    HarnessIsSingleVersion,
}

struct KnownGap {
    path: &'static str,
    src: &'static str,
    why: Why,
}

fn known_gaps() -> Vec<KnownGap> {
    vec![
        // ---- group 1: not standalone entry documents ---------------------
        KnownGap {
            path: "crates/rustyfi/tests/fixtures/envelopes/doc.saty",
            src: include_str!("../../rustyfi/tests/fixtures/envelopes/doc.saty"),
            why: Why::NotAnEntryDocument,
        },
        KnownGap {
            path: "crates/rustyfi/tests/fixtures/envelopes/v01-mini.satyh",
            src: include_str!("../../rustyfi/tests/fixtures/envelopes/v01-mini.satyh"),
            why: Why::NotAnEntryDocument,
        },
        KnownGap {
            path: "crates/rustyfi/tests/fixtures/multifile/helpers.satyh",
            src: include_str!("../../rustyfi/tests/fixtures/multifile/helpers.satyh"),
            why: Why::NotAnEntryDocument,
        },
        KnownGap {
            path: "crates/rustyfi/tests/fixtures/xver-capstone-helper.satyh",
            src: include_str!("../../rustyfi/tests/fixtures/xver-capstone-helper.satyh"),
            why: Why::NotAnEntryDocument,
        },
        // ---- group 2: mixed-version programs the golden harness can't hold ----
        KnownGap {
            path: "crates/rustyfi/tests/fixtures/xver-capstone.saty",
            src: include_str!("../../rustyfi/tests/fixtures/xver-capstone.saty"),
            why: Why::HarnessIsSingleVersion,
        },
    ]
}


fn as_v006(cst: &LoadedCst) -> &rustyfi_syntax::cst::File {
    match cst {
        LoadedCst::V0_0(f) => f,
        LoadedCst::V0_1(_) => unreachable!("as_v006 on a V0_1 file"),
    }
}

fn merge_v006(program: LoadedProgram) -> rustyfi_syntax::cst::File {
    let mut files = program.files;
    let entry = files.pop().expect("loader yields the entry last");
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

/// Font metrics are irrelevant here — every case fails before layout — so this
/// is the smallest thing that satisfies the signature.
struct NoMetrics;

impl rustyfi_backend::FontMetrics for NoMetrics {
    fn advance(
        &self,
        _font: rustyfi_backend::FontKey,
        _c: char,
        size: rustyfi_backend::Length,
    ) -> Option<rustyfi_backend::Length> {
        Some(size * 0.5)
    }
    fn ascender(
        &self,
        _font: rustyfi_backend::FontKey,
        size: rustyfi_backend::Length,
    ) -> rustyfi_backend::Length {
        size * 0.75
    }
    fn descender(
        &self,
        _font: rustyfi_backend::FontKey,
        size: rustyfi_backend::Length,
    ) -> rustyfi_backend::Length {
        size * 0.25
    }
}

/// Returns `Err(reason)` when `entry` cannot be typechecked.
fn typecheck_source(entry: &Path, version: RustyfiVersion) -> Result<(), String> {
    let opts = LoadOptions {
        lib_root: Some(lib_root()),
        version,
        ..Default::default()
    };
    let program = rustyfi_loader::load(entry, &opts).map_err(|e| format!("load: {e}"))?;
    match version {
        RustyfiVersion::V0_0 => {
            let file = merge_v006(program);
            let env = primitives::base_env();
            let store = rustyfi_lang::symbol::SymbolStore::new();
            let scope = elaborate::Scope::new(&store, env.names());
            let prog = elaborate::elaborate_program(&file, &scope)
                .map_err(|e| format!("elaborate: {e}"))?;
            typecheck::typecheck_verbose(&prog).map_err(|e| format!("typecheck: {e}"))?;
        }
        _ => {
            // The gate fires in the v1 export adapter, not the loader —
            // only the real compile entry point runs it. Call what the
            // compiler calls, so this records the verdict a user gets.
            let metrics = NoMetrics;
            rustyfi_lang::compile_document_v1(&program.files, &metrics)
                .map(|_| ())
                .map_err(|e| format!("compile: {e}"))?;
        }
    }
    Ok(())
}

/// One line of an outcome — a loader `ParseError` Debug runs to thousands of
/// characters and buries the assertion message.
fn brief(r: &Result<(), String>) -> String {
    match r {
        Ok(()) => "Ok".to_string(),
        Err(e) => {
            let one = e.lines().next().unwrap_or(e);
            if one.len() > 110 {
                format!("{}…", &one[..110])
            } else {
                one.to_string()
            }
        }
    }
}

/// Every known gap still fails, and fails for the reason recorded against
/// it — a CHANGE-DETECTOR: if a gap closes, this test fails, signaling to
/// move the case out of this file and into the golden corpus.
#[test]
fn known_gaps_still_fail_for_the_recorded_reason() {
    // Elaboration and typechecking recurse deeply over the merged prelude of a
    // real document class; the default 2 MiB test stack overflows on it. Same
    // 64 MiB thread `typecheck_corpus.rs` runs on.
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(check_known_gaps)
        .expect("spawn big-stack thread")
        .join()
        .expect("known-gap harness thread panicked");
}

fn check_known_gaps() {
    // Tripwire: an EMPTY `known_gaps()` list would make this test pass while
    // asserting nothing — exactly what happened once, when a bulk edit
    // dropping closed cross-version entries took the rest with them and the
    // suite stayed green. If the last real gap is ever closed, delete this
    // file rather than leaving an empty harness behind.
    assert!(
        !known_gaps().is_empty(),
        "known_gaps() is empty — this file's tests would pass vacuously"
    );
    let root = repo_root();
    // Sweep any `rustyfi-known-gap-`-prefixed files left beside a fixture by
    // an earlier crashed run, so a failure never contaminates the corpus the
    // golden harness walks.
    for gap in known_gaps() {
        let dir = root.join(gap.path);
        let Some(dir) = dir.parent() else { continue };
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        for e in entries.flatten() {
            let name = e.file_name();
            if name.to_string_lossy().starts_with("rustyfi-known-gap-") {
                let _ = fs::remove_file(e.path());
            }
        }
    }

    let mut unexpected = Vec::new();

    for gap in known_gaps() {
        let original = root.join(gap.path);
        assert!(
            original.exists(),
            "{}: fixture is gone — if it was deleted, drop its entry here too",
            gap.path
        );
        assert!(
            !gap.src.is_empty(),
            "{}: include_str! produced an empty source",
            gap.path
        );

        // `gap.src`'s `include_str!` IS the file's bytes; the literal stays
        // only as a readable record. Do NOT materialize it beside the
        // original — that would leave debris when a run aborts.
        let entry = original.clone();

        // Checking BOTH `as_006` and `as_01` is what makes a mislabelled case
        // fail — asserting only "it errors somehow" would not, since every
        // case errors under some version.
        let as_006 = typecheck_source(&entry, RustyfiVersion::V0_0);
        let as_01 = typecheck_source(&entry, RustyfiVersion::V0_1);

        match gap.why {
            Why::NotAnEntryDocument => match (&as_006, &as_01) {
                (Err(_), Err(e)) if e.starts_with("load:") => {}
                _ => unexpected.push(format!(
                    "{}: recorded as not-an-entry-document, but 0.1 did not refuse it at \
                     LOAD time (0.0.6: {}, 0.1: {})",
                    gap.path,
                    brief(&as_006),
                    brief(&as_01)
                )),
            },
            // No entry uses this today; kept so the variant works the
            // moment one does.
            Why::CrossVersionForkedBuiltin { slice, name } => match &as_01 {
                Err(e)
                    if e.contains(&format!("cross-version import ({slice})"))
                        && e.contains(&format!("references `{name}`")) => {}
                other => unexpected.push(format!(
                    "{}: recorded as the {} gate on `{}`, got {}",
                    gap.path,
                    slice,
                    name,
                    brief(other)
                )),
            },
            Why::HarnessIsSingleVersion => match &as_01 {
                Err(e)
                    if e.contains("are SATySFi 0.1 syntax")
                        || e.contains("unbound variable 'List.fold-left'") => {}
                other => unexpected.push(format!(
                    "{}: recorded as a single-version-harness limitation, got {}",
                    gap.path,
                    brief(other)
                )),
            },
            // Must NOT be a cross-version rejection: the bridge works here,
            // the package APIs just differ.
        }
    }

    assert!(
        unexpected.is_empty(),
        "these inputs no longer fail the way this file records — if a gap was \
         CLOSED, delete its entry here and add it to `typecheck_corpus.rs`'s \
         DOCUMENTS list so the corpus covers it:\n  {}",
        unexpected.join("\n  ")
    );
}
