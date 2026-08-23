//! `project::check`: the whole-program tier.

use std::path::{Path, PathBuf};

use rustyfi_lsp::project::{self, CheckOptions, Depth};
use rustyfi_lsp::{Diag, RustyfiVersion};

fn repo() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

fn lib_root() -> PathBuf {
    repo().join("lib-rustyfi")
}

/// The options a real editor session against this repository would have.
fn opts() -> CheckOptions {
    CheckOptions {
        lib_roots: vec![lib_root()],
        ..Default::default()
    }
}

/// [`opts`] with the library tier turned on.
fn library_opts() -> CheckOptions {
    CheckOptions {
        check_libraries: true,
        ..opts()
    }
}

fn at(d: &Diag) -> (u32, u32, u32, u32) {
    (d.line, d.character, d.end_line, d.end_character)
}

fn only(diags: &[Diag]) -> &Diag {
    assert_eq!(diags.len(), 1, "expected exactly one diagnostic, got {diags:?}");
    &diags[0]
}

/// A path in a real directory inside the repository — so `@import:` has
/// somewhere to resolve against — naming a file that does NOT exist, which is
/// the interesting half: an editor's buffer is analysed long before it is
/// first saved, and every one of these tests would still pass against a
/// loader that quietly read the path instead of the buffer if the file were
/// there.
fn unsaved(name: &str) -> PathBuf {
    repo().join("layout-tests").join(name)
}

// ---------------------------------------------------------------------------
// The point of the whole tier: names that only a resolved program can know
// ---------------------------------------------------------------------------

#[test]
fn an_imported_name_is_not_reported_as_unbound() {
    // Every name here comes from `stdjabook`/`math`/`annot`, none of which is
    // in the buffer. This is the exact failure mode single-file elaboration
    // has and the reason `analyze` stops at parsing.
    let src = "@require: stdjabook\n\
               @require: annot\n\
               document (| title = {t}; author = {a}; show-title = true; show-toc = false |) '<\n\
               \x20 +p { Hello \\href(`https://example.com`){ world }. }\n\
               >\n";
    let a = project::check(&unsaved("unsaved-doc.saty"), src, &opts());
    assert_eq!(a.depth, Depth::Program, "note: {:?}", a.note);
    assert_eq!(a.diagnostics, Vec::new());
    assert!(a.files > 3, "the program must really have been resolved: {a:?}");
}

#[test]
fn a_genuine_type_error_is_reported_at_its_position_in_0_0_6() {
    //          0123456789012
    // line 4 is `let m = succ s in`; `succ` starts at character 8, and the
    // span the checker records for a bad application is the FUNCTION's.
    let src = "@require: stdjabook\n\
               let succ n = n + 1 in\n\
               let s = `oops` in\n\
               let m = succ s in\n\
               document (| title = {t}; author = {a}; show-title = false; show-toc = false |) '<\n\
               \x20 +p { hi }\n\
               >\n";
    let a = project::check(&unsaved("unsaved-type-error.saty"), src, &opts());
    assert_eq!(a.depth, Depth::Program, "note: {:?}", a.note);
    let d = only(&a.diagnostics);
    assert_eq!(at(d), (3, 8, 3, 12), "{}", d.message);
    assert!(
        d.message.contains("int") && d.message.contains("string"),
        "the message should name both types: {}",
        d.message
    );
}

#[test]
fn a_genuine_type_error_is_reported_at_its_position_in_0_1() {
    // A 0.1 document is `header* expr EOI` — it has no top-level `val`, so
    // nothing in it can signal its generation and the detection ladder reads
    // it as 0.0.6 (which is also what the CLI does with no `--lang`; see
    // `a_0_1_document_carries_no_version_signal_of_its_own` below). This test
    // is about the 0.1 pipeline, so it pins the generation the way a user
    // whose project is 0.1 would: `rustyfi lsp --lang 0.1`.
    let src = "@require: v01-mini\n\
               let succ n = n + 1 in\n\
               let s = `oops` in\n\
               let m = succ s in\n\
               let open V01Mini in\n\
               document (| title = `t` |) '<\n\
               \x20 +p { hi }\n\
               >\n";
    let a = project::check(
        &unsaved("unsaved-v01.saty"),
        src,
        &CheckOptions {
            lang: Some(RustyfiVersion::V0_1),
            ..opts()
        },
    );
    assert_eq!(a.version, RustyfiVersion::V0_1);
    assert_eq!(a.depth, Depth::Program, "note: {:?}", a.note);
    let d = only(&a.diagnostics);
    assert_eq!(at(d), (3, 8, 3, 12), "{}", d.message);
}

#[test]
fn a_0_1_document_is_found_by_the_ambiguity_recheck_not_by_a_sniff() {
    // A 0.1 *document* has no top-level `val` — it is `header* expr EOI` —
    // so `sniff_version` finds no signal and the ladder starts at 0.0.6.
    // What rescues it is the re-check `analyze_detected` already does for
    // libraries: the 0.0.6 grammar cannot read `StdJa.document (| … |)`'s
    // 0.1 record commas, the 0.1 one can, and a clean re-check wins. The
    // whole-program tier inherits that decision, which is why this file is
    // checked as 0.1 with no `--lang` at all.
    let path = repo().join("crates/rustyfi/tests/fixtures/v01-stdja.saty");
    let src = std::fs::read_to_string(&path).expect("the fixture must exist");
    let a = project::check(&path, &src, &opts());
    assert_eq!(a.version, RustyfiVersion::V0_1);
    assert_eq!(a.depth, Depth::Program, "note: {:?}", a.note);
    assert_eq!(a.diagnostics, Vec::new());
    assert!(a.files > 10, "std-ja pulls a real dependency tree: {a:?}");
}

// ---------------------------------------------------------------------------
// Degradation: never a wall of red for something that is not the user's fault
// ---------------------------------------------------------------------------

#[test]
fn no_library_root_degrades_to_parse_only_rather_than_reporting_every_import() {
    let src = "@require: stdjabook\nlet x = 1 in\ndocument (| |) '<>\n";
    let a = project::check(
        &unsaved("unsaved-noroot.saty"),
        src,
        &CheckOptions::default(),
    );
    assert_eq!(a.depth, Depth::Parse);
    assert_eq!(a.diagnostics, Vec::new(), "silence, not a wall of red");
    let note = a.note.expect("the degradation must be recorded");
    assert!(note.contains("cannot resolve"), "{note}");
}

#[test]
fn discovery_supplies_the_roots_when_none_is_named() {
    // The hook `rustyfi lsp` fills with `sg::roots::discover_all`. It is a
    // function pointer rather than a dependency (this crate must not pull
    // tar/flate2/TLS in for an editor front end), so the thing worth pinning
    // is that it is actually consulted.
    fn discover(_dir: &Path) -> Vec<PathBuf> {
        vec![PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../lib-rustyfi"
        ))]
    }
    let src = "@require: stdja-mini\n\
               document (| title = {T}; author = {A} |) '< +p { hi } >\n";
    let a = project::check(
        &unsaved("unsaved-discovery.saty"),
        src,
        &CheckOptions {
            discover_roots: Some(discover),
            ..Default::default()
        },
    );
    assert_eq!(a.depth, Depth::Program, "note: {:?}", a.note);
    assert_eq!(a.diagnostics, Vec::new());
    assert!(a.files >= 2, "{a:?}");
}

#[test]
fn an_uninstalled_package_degrades_the_same_way() {
    let src = "@require: definitely-not-a-real-package\ndocument (| |) '<>\n";
    let a = project::check(&unsaved("unsaved-missing.saty"), src, &opts());
    assert_eq!(a.depth, Depth::Parse);
    assert_eq!(a.diagnostics, Vec::new());
    assert!(a.note.unwrap_or_default().contains("definitely-not-a-real-package"));
}

#[test]
fn a_library_buffer_is_left_alone_unless_asked_for() {
    let src = "@require: stdjabook\nlet-inline \\greet it = {Hello #it;}\n";
    let a = project::check(&unsaved("unsaved-lib.satyh"), src, &opts());
    assert_eq!(a.depth, Depth::Parse);
    assert_eq!(a.diagnostics, Vec::new());
    assert!(a.note.unwrap_or_default().contains("library"));
}

// ---------------------------------------------------------------------------
// Library buffers, when they are asked for
// ---------------------------------------------------------------------------

#[test]
fn a_library_is_checked_against_its_own_headers() {
    // `Option.map` is not in the buffer; `@require: option` is what makes it
    // exist. The stub document the library is checked underneath has to carry
    // that header, or this reports an unbound variable.
    let src = "@require: option\n\
               let-rec twice o = Option.map (fun x -> x * 2) o\n";
    let a = project::check(&unsaved("unsaved-lib.satyh"), src, &library_opts());
    assert_eq!(a.depth, Depth::Program, "note: {:?}", a.note);
    assert_eq!(a.diagnostics, Vec::new());
    assert!(a.files >= 2, "option.satyg must really be in the program: {a:?}");

    // The control: drop the header and the same buffer stops resolving the
    // name — which is also the failure mode `check_libraries` is off by
    // default for, since a library may legitimately leave that to a consumer.
    let src = "let-rec twice o = Option.map (fun x -> x * 2) o\n";
    let a = project::check(&unsaved("unsaved-lib.satyh"), src, &library_opts());
    assert!(
        only(&a.diagnostics).message.contains("Option.map"),
        "{:?}",
        a.diagnostics
    );
}

#[test]
fn a_library_buffers_own_stage_header_still_applies() {
    // `&e` (quote) is legal at stage 0 and not at stage 1. A library carries
    // its stage in a `@stage:` header, which the merge reads off the file
    // itself — the buffer is a *dependency* of the synthetic entry here, so
    // that still works. If the stub swallowed the header, or the buffer were
    // spliced as the entry, this would be a stage error.
    let staged = "@stage: 0\nlet quoted = &(1 + 1)\n";
    let a = project::check(&unsaved("unsaved-staged.satyh"), staged, &library_opts());
    assert_eq!(a.depth, Depth::Program, "note: {:?}", a.note);
    assert_eq!(a.diagnostics, Vec::new());

    // The control: the same buffer at the default stage.
    let unstaged = "let quoted = &(1 + 1)\n";
    let a = project::check(&unsaved("unsaved-staged.satyh"), unstaged, &library_opts());
    assert!(
        !a.diagnostics.is_empty(),
        "a stage-1 quote must not be accepted: {a:?}"
    );
}

#[test]
fn a_parse_error_stops_before_the_program_tier() {
    let src = "@require: stdjabook\nlet y = ] in y\n";
    let a = project::check(&unsaved("unsaved-broken.saty"), src, &opts());
    assert_eq!(a.depth, Depth::Parse);
    assert_eq!(at(only(&a.diagnostics)), (1, 8, 1, 9));
    assert_eq!(a.note, None, "a parse error is not a degradation");
}

#[test]
fn an_empty_buffer_is_not_an_error_at_this_tier_either() {
    for src in ["", "   \n\n", "% nothing yet\n"] {
        let a = project::check(&unsaved("unsaved-empty.saty"), src, &opts());
        assert_eq!(a.diagnostics, Vec::new(), "{src:?}");
    }
}

// ---------------------------------------------------------------------------
// The buffer, not the file on disk
// ---------------------------------------------------------------------------

#[test]
fn the_buffers_own_text_wins_over_the_file_on_disk() {
    // A real file in the corpus, analysed with different text than it holds.
    // If the loader read the path instead of the buffer, the injected error
    // would not be reported (and the real file's content would be).
    let path = repo().join("crates/rustyfi/tests/fixtures/minimal.saty");
    let on_disk = std::fs::read_to_string(&path).expect("the fixture must exist");
    let clean = project::check(&path, &on_disk, &opts());
    assert_eq!(clean.depth, Depth::Program, "note: {:?}", clean.note);
    assert_eq!(clean.diagnostics, Vec::new());

    // The headers have to stay first, so the edit goes after them — one line
    // that only exists in the buffer.
    let edited = on_disk.replace(
        "let author-name = `yasuo`",
        "let author-name = `yasuo`\nlet succ n = n + 1\nlet oops = succ `not an int`",
    );
    assert_ne!(edited, on_disk, "the fixture's shape changed under this test");
    let a = project::check(&path, &edited, &opts());
    assert_eq!(a.depth, Depth::Program, "note: {:?}", a.note);
    let d = only(&a.diagnostics);
    let line = edited
        .lines()
        .position(|l| l.starts_with("let oops"))
        .expect("the edited line") as u32;
    assert_eq!(d.line, line, "the diagnostic must land on the EDITED line: {d:?}");
}

// ---------------------------------------------------------------------------
// Positions stay UTF-16
// ---------------------------------------------------------------------------

#[test]
fn a_type_errors_column_is_counted_in_utf16_units() {
    // `こんにちは` is 5 chars / 15 bytes / 5 UTF-16 units, and it sits BEFORE
    // the error on the same line — so a byte-offset implementation is ten
    // columns out, and `Loc::col`'s char column happens to agree here only
    // because there is no astral character in the way.
    let src = "@require: stdjabook\n\
               let succ n = n + 1 in\n\
               let s = `こんにちは` in let m = succ s in\n\
               document (| title = {t}; author = {a}; show-title = false; show-toc = false |) '<\n\
               \x20 +p { hi }\n\
               >\n";
    let a = project::check(&unsaved("unsaved-utf16.saty"), src, &opts());
    assert_eq!(a.depth, Depth::Program, "note: {:?}", a.note);
    let d = only(&a.diagnostics);

    let line = src.lines().nth(2).expect("line 3");
    let byte = line.find("succ s").expect("the application");
    let utf16: u32 = line[..byte].chars().map(|c| c.len_utf16() as u32).sum();
    assert_ne!(utf16 as usize, byte, "the test would be vacuous without the kana");
    assert_eq!((d.line, d.character), (2, utf16), "{d:?}");
}

// ---------------------------------------------------------------------------
// A dependency's error is not painted onto the buffer as if it were the
// buffer's own
// ---------------------------------------------------------------------------

#[test]
fn an_error_that_belongs_to_another_file_is_reported_without_a_false_position() {
    let dir = tempdir("xver-refusal");
    let dep = dir.join("broken.satyh");
    // Type-correct on its own, but its ONE binding names a variable nothing
    // defines — an elaboration error whose span is an offset into THIS file,
    // deliberately far enough down that the same offset exists in the entry
    // buffer at a different line.
    std::fs::write(&dep, "\n\n\n\n\n\n\nlet from-the-dependency = no-such-name-anywhere\n")
        .expect("write the dependency");
    let entry = dir.join("doc.saty");
    let src = "@import: broken\ndocument (| |) '<>\n";

    let a = project::check(&entry, src, &opts());
    assert_eq!(a.depth, Depth::Program, "note: {:?}", a.note);
    let d = only(&a.diagnostics);
    assert_eq!(
        (d.line, d.character),
        (0, 0),
        "an unattributable span belongs at the top of the file, not at a guess: {d:?}"
    );
    assert!(
        d.message.contains("no-such-name-anywhere") && d.message.contains("another file"),
        "{}",
        d.message
    );
}

#[test]
fn an_error_with_no_recorded_position_says_that_rather_than_guessing() {
    // `1 + \`oops\``: the failing application's function is itself an
    // application (`(+) 1`), and `typecheck::ast_span` only recovers a span
    // from a `Var`/`Overwrite`/`AccessField`. There is genuinely nowhere to
    // point, and the message must not pretend otherwise.
    let src = "@require: stdjabook\n\
               let n = 1 + `oops` in\n\
               document (| title = {t}; author = {a}; show-title = false; show-toc = false |) '<\n\
               \x20 +p { hi }\n\
               >\n";
    let a = project::check(&unsaved("unsaved-nospan.saty"), src, &opts());
    assert_eq!(a.depth, Depth::Program, "note: {:?}", a.note);
    let d = only(&a.diagnostics);
    assert_eq!((d.line, d.character), (0, 0));
    assert!(d.message.contains("no position"), "{}", d.message);
}

/// A fresh directory under the OS temp dir. Not `tempfile` — this crate has no
/// dev-dependencies and one directory per test does not justify the first.
fn tempdir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rustyfi-lsp-{name}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create the temp directory");
    dir
}

// ---------------------------------------------------------------------------
// Staying responsive
// ---------------------------------------------------------------------------

/// The parse budget still governs this tier, because this tier never runs
/// without a clean parse.
///
/// `budget.rs` caps a parse because the 0.1 grammar backtracks exponentially
/// on some half-typed buffers — 11.5 s on the 14 KB prefix below. The loader
/// applies the *compiler's* budget, which is deliberately larger, so the
/// property that matters is that a buffer which exhausts this crate's budget
/// is reported and *stops*, rather than being handed to `rustyfi_loader::load`
/// to be parsed a second time on the compiler's terms.
#[test]
fn a_pathological_buffer_never_reaches_the_loader() {
    let path = repo().join("lib-rustyfi/dist-v01/packages/std-ja.satyh");
    let src = std::fs::read_to_string(&path).expect("the vendored 0.1 corpus must be present");
    let src = &src[..14_223.min(src.len())];

    let started = std::time::Instant::now();
    let a = project::check(&path, src, &library_opts());
    let elapsed = started.elapsed();

    assert_eq!(a.depth, Depth::Parse);
    assert_eq!(a.files, 0, "nothing may have been loaded");
    assert!(only(&a.diagnostics).message.contains("gave up"), "{a:?}");
    assert!(elapsed < std::time::Duration::from_secs(60), "{elapsed:?}");
}

/// What the tier costs on the largest document this repository ships, printed
/// (`--nocapture`) rather than merely asserted.
///
/// The bound is deliberately loose. Its job is to catch a change that makes
/// the tier pathological — a re-resolution per binding, a lost parse budget,
/// an accidental evaluation — not to police milliseconds on a shared runner
/// where this runs in a debug build alongside every other test. The real
/// numbers, from a release build on one machine: 2 ms for a two-file
/// document, 100–200 ms for the ones with a full document class behind them
/// (`v01-stdja-book.saty`, 28 files, 203 ms), and 1.7 ms for the smallest.
#[test]
fn checking_the_largest_shipped_document_stays_within_an_editors_patience() {
    let path = repo().join("crates/rustyfi/tests/fixtures/v01-stdja-book.saty");
    let src = std::fs::read_to_string(&path).expect("the fixture must exist");
    let started = std::time::Instant::now();
    let a = project::check(&path, &src, &opts());
    let elapsed = started.elapsed();
    eprintln!(
        "v01-stdja-book.saty: {} files, {:.0}ms",
        a.files,
        elapsed.as_secs_f64() * 1e3
    );
    assert_eq!(a.depth, Depth::Program, "note: {:?}", a.note);
    assert!(a.files >= 20, "{a:?}");
    assert!(elapsed < std::time::Duration::from_secs(20), "{elapsed:?}");
}

// ---------------------------------------------------------------------------
// Known-good real files: the sweep
// ---------------------------------------------------------------------------

/// Every `.saty` DOCUMENT this repository ships, through the whole-program
/// tier, with zero diagnostics required.
///
/// These all compile — the corpus documents are rendered to PDF by
/// `layout-tests`, the fixtures by the `rustyfi` crate's own end-to-end
/// tests — so any diagnostic here is a false positive, the failure mode that
/// makes a language server worse than no language server.
#[test]
fn every_shipped_document_typechecks_clean() {
    sweep(&documents(), &opts(), 40);
}

/// The two vendored package corpora, checked as **libraries** — the tier that
/// is off by default, swept here so that turning it on is a measured choice
/// rather than a hopeful one.
///
/// The parse-tier sweep in `tests/analysis.rs` covers these same 77 files;
/// this is the same files through elaboration, typechecking and `:>` sealing.
#[test]
fn every_bundled_package_typechecks_clean_as_a_library() {
    sweep(&bundled_packages(), &library_opts(), 70);
}

/// The document corpus's own package sources — 114 files of real
/// third-party SATySFi, none of it written with this port in mind.
///
/// Most of them degrade rather than check: they `@require:` packages
/// (`base/list-ext`, `easytable/easytable`, `fss/fss`) that this repository
/// does not install, and `layout-tests` compiles them by pointing the loader
/// at the corpus's own directories. The ones that do resolve are the point.
#[test]
fn the_corpus_package_sources_typecheck_clean_as_libraries() {
    sweep(&corpus_libraries(), &library_opts(), 60);
}

/// Every excluded file must still be failing.
///
/// An exclusion list is only honest if it cannot outlive its reasons. Three of
/// these seven are waiting on something that could plausibly land — the missing
/// `get-space-ratio-between-scripts` primitive, `inline.satyh`'s signature —
/// and a name left here after its file started passing would be a quietly
/// narrower sweep. So each one is checked to still produce a diagnostic; when
/// it stops, this fails and the name comes off the list.
#[test]
fn the_exclusions_are_still_needed() {
    let all: Vec<PathBuf> = documents()
        .into_iter()
        .chain(bundled_packages())
        .chain(corpus_libraries())
        .collect();
    let mut checked = 0usize;
    for path in all.iter().filter(|p| excluded(p).is_some()) {
        checked += 1;
        let src = std::fs::read_to_string(path).expect("read");
        let a = project::check(path, &src, &library_opts());
        assert!(
            a.depth == Depth::Program && !a.diagnostics.is_empty(),
            "{} no longer fails ({:?}, {:?}) — remove it from `excluded`",
            path.display(),
            a.depth,
            a.note
        );
    }
    assert_eq!(checked, 7, "the exclusion list and the corpus disagree");
}

/// Check every file in `paths` and fail on any diagnostic at all, naming each
/// one.
///
/// Files that never reach [`Depth::Program`] are skipped, not counted:
/// degrading is a legitimate outcome (an uninstalled package), but it is also
/// how this sweep could quietly stop testing anything — hence `floor`, the
/// number of files that must actually have been *checked*.
fn sweep(paths: &[PathBuf], opts: &CheckOptions, floor: usize) {
    let mut checked = 0usize;
    let mut complaints = Vec::new();
    for path in paths {
        if let Some(why) = excluded(path) {
            eprintln!("skipped {}: {why}", path.display());
            continue;
        }
        let src = std::fs::read_to_string(path).expect("read");
        let a = project::check(path, &src, opts);
        if a.depth != Depth::Program {
            continue;
        }
        checked += 1;
        for d in a.diagnostics {
            complaints.push(format!(
                "{} [as {}] line {}, char {}: {}",
                path.display(),
                a.version,
                d.line + 1,
                d.character,
                d.message
            ));
        }
    }
    assert!(
        complaints.is_empty(),
        "false positives on {} file(s) that compile:\n{}",
        complaints.len(),
        complaints.join("\n")
    );
    assert!(checked >= floor, "only {checked} of {} files reached the program tier", paths.len());
}

/// The seven files the sweeps do not check, each with the reason.
///
/// Every one of them fails for a reason that is **not** the analysis being
/// wrong: three do not compile in this port at all (verified by asking
/// `rustyfi` itself), and three are valid files that the language relies on a
/// *consumer* to complete. Listing them by name, with the reason, is the
/// point — the alternative is a sweep whose bar quietly drops.
fn excluded(path: &Path) -> Option<&'static str> {
    let name = path.file_name()?.to_str()?;
    let parent = path.parent()?.to_str()?;
    Some(match name {
        // Compiled by its own harness (`xver_capstone.rs`) against a
        // deliberately 0.0.6-ONLY library root — a temp directory that
        // symlinks `dist/` and hides `dist-v01/` — because the fixture exists
        // to prove `@require: list` crosses a version boundary. Against the
        // full bundled root, `list` resolves to the 0.1 corpus's own `list`,
        // which has no `List.fold-left`. `rustyfi --lib-root lib-rustyfi`
        // says exactly the same thing; nothing in the buffer, the path or the
        // filesystem could tell a language server about the harness's root.
        "xver-capstone.saty" => "compiled against a 0.0.6-only root by its own harness",
        // A genuine, pre-existing breakage in the vendored 0.1 corpus, not an
        // artefact of checking it standalone: `Logo` calls `Inline.\kern`,
        // and `inline.satyh`'s signature declares the value `kern` but not
        // the command `\kern`, so the `:>` seal rejects it. Reproduced
        // outside this crate with `rustyfi --lang 0.1` on a document that
        // `@require: logo` — same message, same cause.
        "logo.satyh" if parent.ends_with("dist-v01/packages") => {
            "vendored `logo.satyh` calls `Inline.\\kern`, which `Inline`'s signature does not export"
        }
        // Two `satysfi-base` files reach for `get-space-ratio-between-
        // scripts`, a primitive this port does not implement (it is in
        // neither generation's `PRIM_DEFS`). No program containing them
        // compiles here, standalone or not.
        "context.satyh" | "satysfi-it.satyh" if parent.contains("satysfi-base") => {
            "uses `get-space-ratio-between-scripts`, a primitive this port does not implement"
        }
        // The global-merge case, and the reason `check_libraries` is off by
        // default: each of these uses a module (`Option`, `Color`) that it
        // never `@require:`s, relying on whichever document pulls it in to
        // have required that package first. They are valid, they compile as
        // part of a real program, and they cannot typecheck alone.
        "tabular2.satyh" if parent.contains("satysfi-base") => {
            "uses `Color.*` without requiring `color` (global-merge module model)"
        }
        "ast.satyh" if parent.contains("satysfi-base") => {
            "uses `Option.*` without requiring `option` (global-merge module model)"
        }
        "selection-cond.satyg" if parent.contains("fss") => {
            "uses `Option.*` without requiring `option` (global-merge module model)"
        }
        _ => return None,
    })
}

/// Every `.saty` file under the directories this repository keeps documents
/// in.
fn documents() -> Vec<PathBuf> {
    collect(
        &[
            "layout-tests/corpus",
            "layout-tests/probes",
            "crates/rustyfi/tests/fixtures",
            "manual",
        ],
        &["saty"],
    )
}

/// The two vendored corpora, both generations.
fn bundled_packages() -> Vec<PathBuf> {
    collect(
        &["lib-rustyfi/dist/packages", "lib-rustyfi/dist-v01/packages"],
        &["satyh", "satyg"],
    )
}

/// The document corpus's package sources (its `.saty` documents are in
/// [`documents`]).
fn corpus_libraries() -> Vec<PathBuf> {
    collect(&["layout-tests/corpus"], &["satyh", "satyg"])
}

fn collect(dirs: &[&str], extensions: &[&str]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for rel in dirs {
        visit(&repo().join(rel), &mut |path| {
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or_default();
            if extensions.contains(&ext) {
                out.push(path.to_path_buf());
            }
        });
    }
    out.sort();
    out
}

fn visit(dir: &Path, f: &mut impl FnMut(&Path)) {
    let entries = std::fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
    let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            visit(&path, f);
        } else {
            f(&path);
        }
    }
}
