//! Ld3a: `LoadMode::Envelopes` (Axis B) CLI end-to-end tests.

use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_rustyfi"))
}

fn out_pdf(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "rustyfi-envelopes-{tag}-{}-{}.pdf",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

/// The sniffer pins `LoadMode::Envelopes` from the `use` header alone
/// (detection ladder step 3), so no new flag is needed. Acceptance capstone
/// for Ld3a.
#[test]
fn envelopes_use_of_document_compiles_to_pdf() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/envelopes/doc.saty");
    let out = out_pdf("compile");

    let output = Command::new(bin())
        .arg(&fixture)
        .arg("--lang")
        .arg("0.1")
        .arg("-o")
        .arg(&out)
        .arg("--no-cache")
        .output()
        .expect("failed to run the rustyfi binary");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "envelopes compile failed (exit {:?}):\n{stderr}",
        output.status.code()
    );
    assert!(
        stderr.contains("page(s)"),
        "expected a page-count report:\n{stderr}"
    );

    let bytes = std::fs::read(&out).expect("output PDF must exist");
    assert!(bytes.starts_with(b"%PDF-"), "output must be a PDF");
    assert!(!bytes.is_empty(), "output PDF must be non-empty");

    let _ = std::fs::remove_file(&out);
}

/// `--deps` (Axis B) with `--lang 0.0` (Axis A) has no upstream analogue; the
/// CLI rejects it early naming the flag that pinned each axis (the loader's
/// `InvalidModeVersion` is only the backstop).
#[test]
fn deps_with_v006_errors_naming_both_axes() {
    let dir = std::env::temp_dir().join(format!(
        "rustyfi-envelopes-reject-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let input = dir.join("doc.saty");
    std::fs::write(&input, "let x = 1 in x").unwrap();
    let deps = dir.join("rustyfi-deps.yaml");
    std::fs::write(&deps, "envelopes: []\n").unwrap();

    let output = Command::new(bin())
        .arg(&input)
        .arg("--deps")
        .arg(&deps)
        .arg("--lang")
        .arg("0.0")
        .arg("--no-cache")
        .output()
        .expect("failed to run the rustyfi binary");

    assert!(!output.status.success(), "0.0.6 + --deps must be rejected");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--deps"),
        "message must name --deps:\n{stderr}"
    );
    assert!(
        stderr.contains("0.1"),
        "message must name the required version 0.1:\n{stderr}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
