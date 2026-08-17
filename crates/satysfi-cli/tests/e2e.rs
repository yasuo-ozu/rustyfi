//! End-to-end: compile the fixture .saty to a PDF, then verify the text —
//! via pdftotext when available, otherwise by grepping the uncompressed
//! content streams for the `Tj` string operands.

use std::path::Path;
use std::process::Command;

fn compile_fixture() -> Vec<u8> {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/minimal.saty");
    let src = std::fs::read_to_string(&fixture).unwrap();
    let metrics = satysfi_pdf::Base14Metrics;
    let doc = satysfi_lang::compile_document(&src, &metrics).expect("fixture must compile");
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
            for expected in ["(Hello,)", "(world!)", "(SATySFi-in-Rust.)"] {
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
    let src = std::fs::read_to_string(&fixture).unwrap();
    let metrics = satysfi_pdf::Base14Metrics;
    let doc = satysfi_lang::compile_document(&src, &metrics).expect("phase2 fixture must compile");
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
    let src = std::fs::read_to_string(&fixture).unwrap();
    let metrics = satysfi_pdf::Base14Metrics;
    let doc = satysfi_lang::compile_document(&src, &metrics).expect("phase2b fixture must compile");
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

#[test]
fn non_winansi_text_errors_politely() {
    let metrics = satysfi_pdf::Base14Metrics;
    let err = satysfi_lang::compile_document("document (||) '< +p { こんにちは } >", &metrics)
        .unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("WinAnsi") || msg.contains("not available"),
        "unhelpful error: {msg}"
    );
}

/// Multi-file loading through the loader crate: a document `@import:`s a
/// library, whose bindings (a value, a command, a function) all resolve.
#[test]
fn multifile_import_compiles_and_renders() {
    let entry = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/multifile/main.saty");
    let program =
        satysfi_loader::load(&entry, &satysfi_loader::LoadOptions { lib_root: None }).unwrap();
    assert_eq!(program.files.len(), 2, "helpers.satyh + main.saty");

    // Merge exactly as the CLI does.
    let mut files = program.files;
    let entry_file = files.pop().unwrap();
    let mut prelude = Vec::new();
    for lib in files {
        prelude.extend(lib.cst.prelude);
    }
    prelude.extend(entry_file.cst.prelude);
    let merged = satysfi_syntax::cst::File {
        headers: Vec::new(),
        prelude,
        in_kw: entry_file.cst.in_kw,
        body: entry_file.cst.body,
        eoi: entry_file.cst.eoi,
    };

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
