//! The inputs the whole-corpus walk cannot typecheck, pinned here
//! individually with the REASON each one fails.
//!
//! `typecheck_corpus.rs` walks every fixture and bundled package and records
//! one line per input. Eleven of those lines were `ERR`, and a bare `ERR` line
//! in a golden file is a poor record: it pins a failure without saying whether
//! the failure is intended, and it renders whatever error the harness's own
//! merge happens to produce rather than what the compiler actually reports.
//! Both problems bit here — see `KnownGap::why` on the X3 group below.
//!
//! So those eleven inputs are asserted here instead, each with its source
//! embedded via `include_str!` (the fixture files stay where they are: all but
//! one are live fixtures for other tests, `envelopes/v01-mini.satyh` alone
//! being referenced by fifteen), and the corpus walk skips them. What remains in
//! `typecheck_corpus.rs`'s explicit case list is then exactly "everything that
//! should typecheck, does".
//!
//! Five remain, and none is a port bug. Four are simply not standalone entry
//! documents: the corpus walk feeds every source file in as an entry, including
//! ones that only ever appear behind an `@require:`.
//!
//! The fifth is the cross-version capstone, excluded for a CORRECT reason
//! rather than a missing feature: `@require:` now prefers the entry's own
//! generation, so as a 0.1 entry against the full lib root it resolves `list`
//! to the 0.1 corpus, whose `List` has `fold` where the fixture (deliberately
//! written against 0.0.6's) calls `fold-left`. `xver_capstone.rs` exercises it
//! properly, against a lib root exposing only the 0.0.6 corpus.
//!
//! The other seven this file used to carry were 0.1 documents reaching 0.0.6
//! packages. They compile and render (X3/X3b/X3c plus same-generation
//! `@require:` resolution) AND now typecheck in the golden harness, so they sit
//! in `typecheck_corpus.rs`'s `DOCUMENTS` list with the rest of the corpus.
//!
//! `Why::CrossVersionForkedBuiltin` is deliberately KEPT with no current users.
//! It is the discriminating machinery for the gate, and the gate still exists
//! and still rejects (`font`, `math-text`, `math-boxes`, `page`); nothing in
//! the corpus happens to trip it today. Deleting it would mean rebuilding it
//! the next time something does.

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

/// Why an input cannot be typechecked.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Why {
    /// The file is not a standalone entry document — it is a LIBRARY (`.satyh`
    /// with no `in …` body), or a 0.1 file the 0.0.6 parser rejects and whose
    /// 0.1 load then also refuses it as an entry.
    ///
    /// Intended, and an artifact of the harness rather than a property of the
    /// port: the corpus walk feeds every source file in as an entry, including
    /// ones that only ever appear behind an `@require:`. The compiler never
    /// asks these files to stand alone.
    NotAnEntryDocument,
    /// A 0.1 document reaching a 0.0.6 package whose exports mention a builtin
    /// that really is version-forked. The compiler refuses these deliberately:
    ///
    /// ```text
    /// cross-version import (X3): dependency …/math.satyh references `paren`,
    /// a version-forked builtin — X3 only supports the version-neutral subset
    /// of the 0.0.6 corpus
    /// ```
    ///
    /// `name` is the forked builtin the gate names and `slice` the gate that
    /// fires (`X3`, `X3b`, …). Both are asserted, so a case that starts failing
    /// on a DIFFERENT builtin, or at a different gate, shows up as a change
    /// instead of silently continuing to "pass" for a stale reason. That is not
    /// hypothetical: every one of these used to be recorded as `graphics`,
    /// which turned out not to be forked at all, and the math trio then moved
    /// from `math-char-class` to `paren` for the same kind of reason.
    ///
    /// This is CLAUDE.md Design TODO 1 ("Cross-version importation"); the
    /// slices are specced in `docs/plans/design-cross-version-import.md`.
    ///
    /// Worth knowing: the golden harness reported something ELSE for these —
    /// `unbound variable 'text-in-math'` and `unbound variable 'List.fold'` —
    /// because it merges preludes itself and so never runs the loader's gate.
    /// Those messages are symptoms of the same cause (a 0.0.6 package pulled
    /// into a 0.1 program), but reading them as separate missing features would
    /// be a mistake. They are why this file records the compiler's verdict.
    #[allow(dead_code)] // see the module doc comment: kept deliberately
    CrossVersionForkedBuiltin {
        slice: &'static str,
        name: &'static str,
    },
    /// NOT a port gap: these compile and render through the real pipeline.
    /// What cannot handle them is the GOLDEN HARNESS, which merges every
    /// dependency's prelude itself and elaborates the result under ONE
    /// version — so a mixed-version program hits a 0.1 dependency's labeled
    /// optional arguments while "compiled as 0.0.6".
    ///
    /// They are pinned here so the exclusion is a recorded fact with a reason
    /// rather than a silent omission, and the assertion is deliberately on the
    /// HARNESS's failure text: if the harness ever learns to merge per-version,
    /// this stops matching and these move into `typecheck_corpus.rs` instead.
    HarnessIsSingleVersion,
}

struct KnownGap {
    /// Repo-relative, for the assertion message.
    path: &'static str,
    /// The source itself, so the case is readable without opening the fixture.
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
            // The cross-version capstone itself. Written against 0.0.6's
            // `List` (`fold-left`), and `@require:` now prefers the entry's
            // own generation, so as a 0.1 entry against the full lib root it
            // gets the 0.1 `List` (which has `fold`). That is correct
            // behaviour, and the capstone proves what it is meant to prove in
            // `xver_capstone.rs`, which loads it against a 0.0.6-ONLY root.
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

/// Run the front end over one source and return `Err(reason)` when it cannot
/// be typechecked. `Ok(())` means it typechecked cleanly.
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
            // The X3 gate does NOT fire in the loader — `load` succeeds — it
            // fires in the v1 export adapter, which only the real compile
            // entry point runs. That is exactly why the golden harness saw
            // `unbound variable …` instead: its own prelude merge skips the
            // adapter. Call what the compiler calls, so this records the
            // verdict a user actually gets.
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

/// Every known gap still fails, and fails for the reason recorded against it.
///
/// This is a CHANGE-DETECTOR by design: implementing cross-version import
/// (CLAUDE.md Design TODO 1) will make the X3 cases start passing, and this
/// test will fail. That is the intended signal — move the case out of this
/// file and let the golden corpus cover it.
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
    // Tripwire. Both tests in this file iterate `known_gaps()`, so an EMPTY
    // list makes both pass while asserting nothing — which is exactly what
    // happened once: a bulk edit meant to drop the seven closed cross-version
    // entries took the rest with them, and the suite stayed green for it.
    // If the last real gap is ever genuinely closed, delete this file rather
    // than leaving an empty harness behind.
    assert!(
        !known_gaps().is_empty(),
        "known_gaps() is empty — this file's tests would pass vacuously"
    );
    let root = repo_root();
    // A panic or an abort skips `TempEntry::drop`, and these files are written
    // BESIDE real fixtures (the loader resolves a relative `@import:` against
    // the entry's own directory, so a temp dir would not do). Sweep any left by
    // an earlier crashed run, so a failure never contaminates the corpus the
    // golden harness walks. Also gitignored, belt and braces.
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

        // The fixture itself: `gap.src` is `include_str!` of this very path, so
        // writing it back out beside the original was a no-op that only created
        // debris to sweep and ignore. The literal stays as the readable record.
        let entry = original.clone();

        // The two groups are told apart by WHERE they fail, not by a label we
        // could get wrong:
        //
        //   NotAnEntryDocument — not a document at all, so neither version's
        //     loader will take it as an entry.
        //   CrossVersionImportX3 — a perfectly good 0.1 document: it LOADS,
        //     and is refused later, by the export adapter, with the X3 gate.
        //
        // Checking both halves is what makes a mislabelled case fail. Asserting
        // only "it errors somehow" would not: every one of these errors under
        // some version, so a swapped label would sail through.
        let as_006 = typecheck_source(&entry, RustyfiVersion::V0_0);
        let as_01 = typecheck_source(&entry, RustyfiVersion::V0_1);

        // Both groups error under BOTH versions, so "it errors" proves nothing.
        // What separates them is HOW the 0.1 side fails: a non-document is
        // refused by the LOADER, while an X3 case loads cleanly and is refused
        // afterwards by the export adapter.
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
            // Assert the gate AND the builtin it names. Matching only
            // "cross-version import" would keep passing after the cause moved
            // to an entirely different forked name.
            // No entry uses this today (see the module doc comment); the arm
            // stays so the variant keeps working the moment one does.
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
            // Must NOT be a cross-version rejection: the point of this group is
            // that the bridge works and the package APIs differ.
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
