//! THE CROSS-VERSION IMPORT CAPSTONE: a real SATySFi 0.1 document
//! (`tests/fixtures/xver-capstone.saty`) `@require:`-ing a REAL, unmodified
//! upstream SATySFi 0.0.6 package (`lib-satysfi/dist/packages/list.satyg`,
//! which itself `@require:`s `option.satyg`) rendered to an actual PDF —
//! the cross-version analogue of `e2e.rs`'s
//! `v01_stdja_capstone_renders_to_extractable_text` (the 0.1-only marquee
//! capstone) and `tier4_stdjabook_capstone_renders_to_extractable_text`
//! (the 0.0.6-only one).
//!
//! ## The lib-root question
//!
//! The entry document is `V0_1`, so its own 0.1 scaffolding (`document`/
//! `+p`/`\math`) would normally come from a `dist-v01/packages/` package
//! (mirroring `v01-stdja.saty` -> `std-ja.satyh`) — but the `@require:`
//! target this capstone actually needs (`list`/`option`) is a REAL 0.0.6
//! package that lives under `dist/packages/`, and
//! `satysfi_loader::v006::resolve::resolve_require`'s candidate list is
//! ordered `<lib_root>/dist/packages/<name>`, then `<lib_root>/<name>`,
//! then the nested Satyrographos layout — there is no candidate that
//! reaches BOTH `dist/packages/` and `dist-v01/packages/` from a single
//! `lib_root` at once (they are siblings, not one nested in the other), so
//! `e2e.rs`'s own `v01_stdja_capstone` (`lib_root =
//! lib-satysfi/dist-v01/packages`) could never simultaneously resolve a
//! `dist/packages` target via candidate 1 (it would look under
//! `dist-v01/packages/dist/packages/`, which does not exist).
//!
//! The fix needs no loader changes: this capstone's own 0.1 scaffolding
//! (`tests/fixtures/xver-capstone-helper.satyh`, a trimmed `v01-mini.satyh`)
//! is reached via `@import:` — a same-directory-relative header resolved
//! independently of `lib_root` entirely (`resolve_import`, not
//! `resolve_require`) — exactly like `xver_import.rs`'s own
//! `XVER_HELPER_SRC`/`XVER_HELPER` sibling file. That leaves `lib_root`
//! free to point straight at this repo's `lib-satysfi/` root (NOT
//! `dist-v01/packages`, unlike `v01-stdja.saty`'s test), so
//! `resolve_require`'s candidate 1 (`<lib_root>/dist/packages/<name>`)
//! reaches the REAL frozen 0.0.6 corpus directly — no temp-dir copy needed
//! (unlike `xver_import.rs`'s synthetic fixtures, which copy `list.satyg`/
//! `option.satyg` into a throwaway `lib_root` precisely because their
//! entries are hand-written strings, not real files under this repo's own
//! `tests/fixtures/`).
//!
//! In short: a single, realistic `LoadOptions { lib_root: Some(lib_root()),
//! version: V0_1, .. }` DOES reach both corpora at once, provided the 0.1
//! side of the graph is pulled in via `@import:` (local sibling) rather
//! than `@require:` (a `dist-v01/packages` corpus lookup) — no loader
//! wiring gap here.

use std::path::{Path, PathBuf};
use std::process::Command;

/// This repo's `lib-satysfi/` directory — resolved relative to this crate's
/// own manifest directory, same as `e2e.rs`'s `lib_root()`. Deliberately
/// the plain `lib-satysfi/` root (not `dist-v01/packages`, unlike
/// `e2e.rs`'s `v01_stdja_capstone`) — see this file's module doc comment
/// for why.
fn lib_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lib-satysfi")
}

/// `annot`/`list`/`option`'s combined parse depth mirrors the other e2e
/// capstones' stack needs — same big-stack helper as `e2e.rs`.
fn run_with_big_stack(f: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(f)
        .expect("spawn big-stack thread")
        .join()
        .expect("big-stack thread panicked (see assertion above)");
}

/// Locate a real DejaVu TrueType face for `TtfFontStore`, exactly
/// `e2e.rs`'s `find_regular_ttf` (duplicated here since integration test
/// binaries share no code across files).
fn find_regular_ttf() -> Option<PathBuf> {
    for family in ["DejaVuSerif", "DejaVuSans"] {
        if let Ok(output) = Command::new("fc-match")
            .args(["--format=%{file}", family])
            .output()
        {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !path.is_empty() && Path::new(&path).is_file() && path.ends_with(".ttf") {
                    return Some(PathBuf::from(path));
                }
            }
        }
    }
    for candidate in [
        "/usr/share/fonts/truetype/dejavu/DejaVuSerif.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/dejavu/DejaVuSans.ttf",
        "/run/current-system/sw/share/fonts/truetype/DejaVuSans.ttf",
    ] {
        if Path::new(candidate).is_file() {
            return Some(PathBuf::from(candidate));
        }
    }
    None
}

/// THE CAPSTONE: `xver-capstone.saty` `@require:`s the real
/// `lib-satysfi/dist/packages/list.satyg` (which itself `@require:`s
/// `option.satyg`) and actually calls `List.map`/`List.fold-left`/
/// `Option.from` to compute `36` (`List.map (+10) [1,2,3] = [11,12,13]`,
/// `List.fold-left (+) 0 .. = 36`, `Option.from 0 (Some 36) = 36`), then
/// renders that number as digits in the body text — `pdftotext` must find
/// "36" in the output, proving the 0.0.6 package's computation, not just
/// its type, crossed the version boundary into the rendered PDF.
#[test]
fn xver_capstone_renders_to_extractable_text() {
    let font = match find_regular_ttf() {
        Some(p) => p,
        None => {
            eprintln!("skipping xver capstone: no DejaVu TrueType font found");
            return;
        }
    };
    run_with_big_stack(move || {
        let entry = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/xver-capstone.saty");
        let program = satysfi_loader::load(
            &entry,
            &satysfi_loader::LoadOptions {
                lib_root: Some(lib_root()),
                version: satysfi_syntax::SatysfiVersion::V0_1,
                ..Default::default()
            },
        )
        .expect(
            "xver-capstone.saty + its real 0.0.6 list/option @require: targets + the local \
             0.1 @import: helper must all load through one LoadOptions",
        );

        // Sanity: `list`/`option` were actually detected as `V0_0_6` corpus
        // targets (the X1 Q4 per-file rule) — if this ever regresses to
        // "everything V0_1" the capstone would still fail loudly at parse
        // (0.0.6 module syntax isn't valid 0.1), but pin the provenance
        // explicitly so a loader regression here fails with a clear message.
        let saw_v006 = program.files[..program.files.len() - 1]
            .iter()
            .filter(|f| matches!(f.version, satysfi_syntax::SatysfiVersion::V0_0_6))
            .count();
        assert_eq!(
            saw_v006, 2,
            "list.satyg + option.satyg should both be V0_0_6-tagged deps: {:?}",
            program.files.iter().map(|f| (&f.path, f.version)).collect::<Vec<_>>()
        );

        let store = satysfi_pdf::TtfFontStore::load(&font, None, None)
            .expect("load DejaVu regular face");
        let doc = satysfi_lang::compile_document_v1(&program.files, &store).expect(
            "the xver capstone must compile end-to-end: a real 0.0.6 list/option dependency \
             spliced into a 0.1 whole-program compile, through real elaborate/typecheck/eval",
        );
        assert!(!doc.pages.is_empty(), "expected at least one page");
        assert!(
            doc.pages.iter().any(|p| !p.lines.is_empty()),
            "expected at least one non-empty page"
        );

        let bytes = satysfi_pdf::render_pdf_ttf(&doc.geometry, &doc.pages, &store, &doc.images)
            .expect("PDF rendering must succeed");
        assert!(bytes.starts_with(b"%PDF-"), "not a PDF header");
        assert!(
            bytes.windows(9).any(|w| w == b"FontFile2"),
            "expected an embedded TrueType font (FontFile2) in the capstone PDF"
        );

        let tmp = std::env::temp_dir()
            .join(format!("satysfi-rust-e2e-xver-capstone-{}.pdf", std::process::id()));
        std::fs::write(&tmp, &bytes).unwrap();
        let pdftotext = Command::new("pdftotext").arg(&tmp).arg("-").output();
        match pdftotext {
            Ok(out) if out.status.success() => {
                let text = String::from_utf8_lossy(&out.stdout);
                // The load-bearing assertion: `36` is the result of
                // `List.map`/`List.fold-left`/`Option.from`, all real
                // bindings from the REAL upstream 0.0.6 `list.satyg`/
                // `option.satyg` — this can only appear if the cross-
                // version splice actually evaluated those bindings.
                assert!(
                    text.contains("36"),
                    "pdftotext output missing \"36\" — the cross-version List/Option \
                     computation must have reached the rendered PDF:\n{text}"
                );
                for word in ["quick", "brown", "fox"] {
                    assert!(
                        text.contains(word),
                        "pdftotext output missing {word:?} — the capstone must render \
                         extractable Latin body text:\n{text}"
                    );
                }
            }
            _ => eprintln!(
                "pdftotext unavailable; the PDF-header + FontFile2-embed checks already passed"
            ),
        }
        let _ = std::fs::remove_file(&tmp);
    });
}
