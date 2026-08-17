//! End-to-end: compile each fixture `.saty` to a PDF through the real
//! multi-file loader (`satysfi_loader::load` + the same prelude-merge the
//! CLI's `merge_program` does), then verify the text — via pdftotext when
//! available, otherwise by grepping the uncompressed content streams for the
//! `Tj` string operands.
//!
//! Phase 4: `document`/`+p`/`\emph` are no longer hardcoded Rust natives —
//! every fixture now `@require:`s the real `stdja-mini` stdlib package
//! (`lib-satysfi/dist/packages/stdja-mini.satyh`), so every compile below
//! goes through the loader with a `lib_root` pointing at this repo's
//! `lib-satysfi/`, not `satysfi_lang::compile_document` directly.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The repo's `lib-satysfi/` directory, resolved the same way the task
/// describes for tests: relative to this crate's own manifest directory.
fn lib_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lib-satysfi")
}

/// Load `entry` and its full `@require:`/`@import:` dependency graph
/// (against [`lib_root`]), then concatenate the dependency-ordered library
/// preludes ahead of the entry document's own prelude — exactly
/// `satysfi-cli`'s `merge_program` (src/main.rs).
fn load_and_merge(entry: &Path) -> satysfi_syntax::cst::File {
    let program = satysfi_loader::load(
        entry,
        &satysfi_loader::LoadOptions {
            lib_root: Some(lib_root()),
            ..Default::default()
        },
    )
    .unwrap_or_else(|e| panic!("failed to load {}: {e}", entry.display()));

    let mut files = program.files;
    let entry_file = files.pop().expect("loader always yields the entry last");
    let mut prelude = Vec::new();
    for lib in files {
        prelude.extend(lib.cst.prelude);
    }
    prelude.extend(entry_file.cst.prelude);
    satysfi_syntax::cst::File {
        headers: Vec::new(),
        prelude,
        in_kw: entry_file.cst.in_kw,
        body: entry_file.cst.body,
        eoi: entry_file.cst.eoi,
    }
}

fn compile_fixture() -> Vec<u8> {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/minimal.saty");
    let merged = load_and_merge(&fixture);
    let metrics = satysfi_pdf::Base14Metrics;
    let doc = satysfi_lang::compile_document_cst(&merged, &metrics).expect("fixture must compile");
    assert!(!doc.pages.is_empty());
    assert!(
        doc.pages[0].lines.len() >= 3,
        "the long paragraph must wrap: got {} lines",
        doc.pages[0].lines.len()
    );
    satysfi_pdf::render_pdf(&doc.geometry, &doc.pages).expect("PDF rendering must succeed")
}

#[test]
fn fixture_compiles_to_valid_pdf_with_expected_text() {
    let bytes = compile_fixture();
    assert!(bytes.starts_with(b"%PDF-"), "not a PDF header");

    let tmp = std::env::temp_dir().join(format!("satysfi-rust-e2e-{}.pdf", std::process::id()));
    std::fs::write(&tmp, &bytes).unwrap();

    let pdftotext = Command::new("pdftotext")
        .arg(&tmp)
        .arg("-")
        .output();

    match pdftotext {
        Ok(out) if out.status.success() => {
            let text = String::from_utf8_lossy(&out.stdout);
            for expected in [
                "Hello, world!",
                "SATySFi-in-Rust",
                "second paragraph",
                "end to end",
            ] {
                assert!(
                    text.contains(expected),
                    "pdftotext output missing {expected:?}:\n{text}"
                );
            }
        }
        _ => {
            // Fallback: content streams are uncompressed, so the Tj string
            // operands are directly visible in the bytes.
            let hay = String::from_utf8_lossy(&bytes);
            // `\emph{SATySFi-in-Rust}.` sets the emphasized word (oblique) and
            // the trailing `.` as separate text runs, so the period is not part
            // of this operand.
            for expected in ["(Hello,)", "(world!)", "(SATySFi-in-Rust)"] {
                assert!(
                    hay.contains(expected),
                    "content stream missing {expected:?}"
                );
            }
        }
    }
    let _ = std::fs::remove_file(&tmp);
}

fn compile_phase2_fixture() -> Vec<u8> {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/phase2.saty");
    let merged = load_and_merge(&fixture);
    let metrics = satysfi_pdf::Base14Metrics;
    let doc =
        satysfi_lang::compile_document_cst(&merged, &metrics).expect("phase2 fixture must compile");
    assert_eq!(doc.pages.len(), 1);
    assert!(
        doc.pages[0].lines.len() >= 3,
        "expected at least one line per +p paragraph, got {}",
        doc.pages[0].lines.len()
    );
    satysfi_pdf::render_pdf(&doc.geometry, &doc.pages).expect("PDF rendering must succeed")
}

/// End-to-end coverage for the phase-2 elaborator (operator-precedence fold,
/// `let-rec`, `match`, and both `let-inline` forms) via a real `.saty`
/// document, checked the same way as the milestone-1 fixture: pdftotext
/// when available, otherwise a direct scan of the uncompressed content
/// stream's `Tj` string operands.
#[test]
fn phase2_fixture_compiles_and_renders_expected_text() {
    let bytes = compile_phase2_fixture();
    assert!(bytes.starts_with(b"%PDF-"), "not a PDF header");

    let tmp =
        std::env::temp_dir().join(format!("satysfi-rust-e2e-phase2-{}.pdf", std::process::id()));
    std::fs::write(&tmp, &bytes).unwrap();

    let pdftotext = Command::new("pdftotext").arg(&tmp).arg("-").output();

    match pdftotext {
        Ok(out) if out.status.success() => {
            let text = String::from_utf8_lossy(&out.stdout);
            for expected in [
                "Bracketed text via let-inline.",
                "Announced text via the lightweight let-inline form.",
                "Countdown complete.",
            ] {
                assert!(
                    text.contains(expected),
                    "pdftotext output missing {expected:?}:\n{text}"
                );
            }
            assert!(
                !text.contains("Countdown incomplete."),
                "the let-rec/match should have selected the 'finished' branch"
            );
        }
        _ => {
            let hay = String::from_utf8_lossy(&bytes);
            for expected in ["(Bracketed)", "(Announced)", "(Countdown)", "(complete.)"] {
                assert!(hay.contains(expected), "content stream missing {expected:?}");
            }
        }
    }
    let _ = std::fs::remove_file(&tmp);
}

fn compile_phase2b_fixture() -> Vec<u8> {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/phase2b.saty");
    let merged = load_and_merge(&fixture);
    let metrics = satysfi_pdf::Base14Metrics;
    let doc = satysfi_lang::compile_document_cst(&merged, &metrics)
        .expect("phase2b fixture must compile");
    assert_eq!(doc.pages.len(), 1);
    assert!(
        !doc.pages[0].lines.is_empty(),
        "expected at least one line, got {}",
        doc.pages[0].lines.len()
    );
    satysfi_pdf::render_pdf(&doc.geometry, &doc.pages).expect("PDF rendering must succeed")
}

/// End-to-end coverage for the phase-2b elaborator additions (a module +
/// qualified reference, `#label` field access, `let-mutable`/`while`/
/// `before`-built countdown string, and `+p`) via a real `.saty` document,
/// checked the same way as the earlier fixtures: pdftotext when available,
/// otherwise a direct scan of the uncompressed content stream's `Tj` string
/// operands.
#[test]
fn phase2b_fixture_compiles_and_renders_expected_text() {
    let bytes = compile_phase2b_fixture();
    assert!(bytes.starts_with(b"%PDF-"), "not a PDF header");

    let tmp =
        std::env::temp_dir().join(format!("satysfi-rust-e2e-phase2b-{}.pdf", std::process::id()));
    std::fs::write(&tmp, &bytes).unwrap();

    let pdftotext = Command::new("pdftotext").arg(&tmp).arg("-").output();

    match pdftotext {
        Ok(out) if out.status.success() => {
            let text = String::from_utf8_lossy(&out.stdout);
            for expected in ["Countdowncomplete."] {
                assert!(
                    text.replace(char::is_whitespace, "").contains(expected),
                    "pdftotext output missing {expected:?}:\n{text}"
                );
            }
            assert!(
                !text.contains("incomplete"),
                "the let-mutable/while countdown should have reached zero: {text}"
            );
        }
        _ => {
            let hay = String::from_utf8_lossy(&bytes);
            for expected in ["(Countdown)", "(complete.)"] {
                assert!(hay.contains(expected), "content stream missing {expected:?}");
            }
        }
    }
    let _ = std::fs::remove_file(&tmp);
}

/// A non-fixture source string, compiled through the same loader path by
/// writing it to a temp file that itself `@require:`s `stdja-mini` — `\emph`
/// is no longer a Rust native, so exercising it (even for this error-path
/// test) needs the real package.
#[test]
fn non_winansi_text_errors_politely() {
    let tmp = std::env::temp_dir().join(format!(
        "satysfi-rust-e2e-nonwinansi-{}.saty",
        std::process::id()
    ));
    std::fs::write(
        &tmp,
        "@require: stdja-mini\ndocument (||) '< +p { こんにちは } >",
    )
    .unwrap();

    let merged = load_and_merge(&tmp);
    let metrics = satysfi_pdf::Base14Metrics;
    let err = satysfi_lang::compile_document_cst(&merged, &metrics).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("WinAnsi") || msg.contains("not available"),
        "unhelpful error: {msg}"
    );
    let _ = std::fs::remove_file(&tmp);
}

fn compile_graphics_fixture() -> Vec<u8> {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/graphics.saty");
    let merged = load_and_merge(&fixture);
    let metrics = satysfi_pdf::Base14Metrics;
    let doc = satysfi_lang::compile_document_cst(&merged, &metrics)
        .expect("graphics fixture must compile");
    assert_eq!(doc.pages.len(), 1);
    satysfi_pdf::render_pdf(&doc.geometry, &doc.pages).expect("PDF rendering must succeed")
}

/// End-to-end coverage for the Slice 1 graphics primitives
/// (`docs/plans/graphics-subsystem.md`): `start-path`/`line-to`/
/// `close-with-line` build a 20pt-square `path`, `fill`/`stroke` turn it
/// into `graphics`, and a local `\graphics` command (`inline-graphics`)
/// places it on the page. Checked by scanning the uncompressed content
/// stream for the path operators the rectangle must produce — the box's
/// local path coordinates are exact regardless of where real line/page
/// layout ends up placing the box (`place_graphics` translates the whole
/// box via one `cm`, never per-coordinate).
#[test]
fn graphics_fixture_compiles_and_renders_path_operators() {
    let bytes = compile_graphics_fixture();
    assert!(bytes.starts_with(b"%PDF-"), "not a PDF header");

    let hay = String::from_utf8_lossy(&bytes);
    // Path construction: move to the rectangle's start, three line-tos, then
    // `close_path` (`h`, zero operands, so bounded by newlines not spaces).
    for op in ["0 0 m", "20 0 l", "20 20 l", "0 20 l", "\nh\n"] {
        assert!(hay.contains(op), "content stream missing {op:?}:\n{hay}");
    }
    // Fill (even-odd — upstream's `op_f'`) in RGB red, then a 1pt gray
    // stroke — each re-emits its own copy of the path before painting it.
    for op in ["1 0 0 rg", "f*", "1 w", "0 G", "\nS\n"] {
        assert!(hay.contains(op), "content stream missing {op:?}:\n{hay}");
    }
    // The whole box is placed via a single `cm` translate, not a per-
    // coordinate flip.
    assert!(
        hay.contains(" cm\n"),
        "content stream missing the box's placement transform:\n{hay}"
    );
}

/// Multi-file loading through the loader crate: a document `@require:`s the
/// `stdja-mini` stdlib package and `@import:`s a local library, whose
/// bindings (a value, a command, a function) all resolve.
#[test]
fn multifile_import_compiles_and_renders() {
    let entry = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/multifile/main.saty");
    let program = satysfi_loader::load(
        &entry,
        &satysfi_loader::LoadOptions {
            lib_root: Some(lib_root()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        program.files.len(),
        3,
        "stdja-mini.satyh + helpers.satyh + main.saty"
    );

    let merged = load_and_merge(&entry);

    let metrics = satysfi_pdf::Base14Metrics;
    let doc = satysfi_lang::compile_document_cst(&merged, &metrics).unwrap();
    let bytes = satysfi_pdf::render_pdf(&doc.geometry, &doc.pages).unwrap();

    let tmp = std::env::temp_dir().join(format!("satysfi-rust-e2e-mf-{}.pdf", std::process::id()));
    std::fs::write(&tmp, &bytes).unwrap();
    if let Ok(out) = Command::new("pdftotext").arg(&tmp).arg("-").output() {
        if out.status.success() {
            let text = String::from_utf8_lossy(&out.stdout);
            assert!(text.contains("Imported command works."), "missing: {text}");
            assert!(text.contains("Twice twenty-one is 42 indeed."), "missing: {text}");
        }
    }
    let _ = std::fs::remove_file(&tmp);
}
