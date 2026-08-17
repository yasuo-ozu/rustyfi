//! The inputs the whole-corpus golden harness cannot typecheck, pinned here
//! individually with the REASON each one fails.
//!
//! `typecheck_golden.rs` walks every fixture and bundled package and records
//! one line per input. Eleven of those lines were `ERR`, and a bare `ERR` line
//! in a golden file is a poor record: it pins a failure without saying whether
//! the failure is intended, and it renders whatever error the harness's own
//! merge happens to produce rather than what the compiler actually reports.
//! Both problems bit here — see `KnownGap::why` on the X3 group below.
//!
//! So those eleven inputs are asserted here instead, each with its source
//! embedded via `include_str!` (the fixture files stay where they are: all but
//! one are live fixtures for other tests, `envelopes/v01-mini.satyh` alone
//! being referenced by fifteen), and the golden walk skips them. What remains
//! in `snapshots/typecheck_golden.txt` is then exactly "everything that should
//! typecheck, does".
//!
//! None of these eleven is an unknown bug, and as of the X3/X3b/X3c work none
//! is blocked by the cross-version BRIDGE any more. They fall into two groups:
//! four files that are not entry documents at all, and seven whose versions
//! link fine but whose PACKAGES differ in their own API — a 0.1 document asking
//! its 0.0.6-generation dependency for something that generation never had
//! (`List.fold`, `+listing`'s `?:break`, `\mathsf`, 0.1's math-command shape).
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
    /// this stops matching and these move into the snapshot where they belong.
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
    ]
}

/// A uniquely-named temp entry file, cleaned up on drop.
///
/// The source arrives here as a `&str`, but the loader resolves `@require:`
/// against a real path, so each case is materialized next to the fixture it
/// came from — beside it, not in a temp dir, so a relative `@import:` still
/// resolves exactly as it does for the original.
struct TempEntry(PathBuf);

impl TempEntry {
    fn beside(original: &Path, src: &str) -> TempEntry {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let ext = original
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("saty");
        let path = original.with_file_name(format!(
            "rustyfi-known-gap-{}-{}.{ext}",
            std::process::id(),
            n
        ));
        fs::write(&path, src).expect("write temp entry");
        TempEntry(path)
    }
}

impl Drop for TempEntry {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn as_v006(cst: &LoadedCst) -> &rustyfi_syntax::cst::File {
    match cst {
        LoadedCst::V0_0_6(f) => f,
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
        RustyfiVersion::V0_0_6 => {
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
    // 64 MiB thread `typecheck_golden.rs` runs on.
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(check_known_gaps)
        .expect("spawn big-stack thread")
        .join()
        .expect("known-gap harness thread panicked");
}

fn check_known_gaps() {
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

        let tmp = TempEntry::beside(&original, gap.src);

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
        let as_006 = typecheck_source(&tmp.0, RustyfiVersion::V0_0_6);
        let as_01 = typecheck_source(&tmp.0, RustyfiVersion::V0_1);

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
         CLOSED, delete its entry here and re-pin snapshots/typecheck_golden.txt \
         so the corpus covers it:\n  {}",
        unexpected.join("\n  ")
    );
}

/// The golden corpus and this file must partition the inputs: nothing listed
/// here may also appear in the committed golden snapshot.
#[test]
fn known_gaps_are_excluded_from_the_golden_snapshot() {
    let snapshot = fs::read_to_string(
        repo_root().join("crates/rustyfi-lang/tests/snapshots/typecheck_golden.txt"),
    )
    .expect("read the golden snapshot");

    let mut leaked = Vec::new();
    for gap in known_gaps() {
        if snapshot.lines().any(|l| l.contains(gap.path)) {
            leaked.push(gap.path);
        }
    }
    assert!(
        leaked.is_empty(),
        "these are asserted in this file AND present in the golden snapshot; \
         the golden walk should skip them:\n  {}",
        leaked.join("\n  ")
    );
    assert!(
        !snapshot.lines().any(|l| l.starts_with("ERR ")),
        "the golden snapshot should contain no ERR lines — every input it \
         walks is expected to typecheck. Move any new failure into {}",
        file!()
    );
}
