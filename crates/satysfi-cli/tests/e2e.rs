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
