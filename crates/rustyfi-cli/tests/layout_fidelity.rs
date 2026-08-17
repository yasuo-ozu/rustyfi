//! Layout-fidelity regression test: this Rust port vs. upstream SATySFi.
//!
//! Each `docs-corpus/` submodule ships a reference PDF built by the *original*
//! OCaml SATySFi. This test rebuilds the same sources with the port and
//! compares the two PDFs' LAYOUT (word bounding boxes, via poppler
//! `pdftotext -bbox`) across every complex construct the corpus exercises —
//! tables, nested lists, figures/floats, math + framed boxes, and vector
//! graphics. Because the port bundles the same fonts SATySFi uses, glyph
//! metrics are identical, so any divergence is the layout ENGINE's (line
//! breaking, spacing, pagination, box placement).
//!
//! The heavy lifting lives in `scripts/layout_fidelity.py` (lib-root assembly,
//! per-doc build, poppler extraction, metric computation, baseline compare) —
//! reused by a plain CLI so a developer can run/inspect/update it directly:
//!
//!   scripts/layout_fidelity.py                 # check against baseline
//!   scripts/layout_fidelity.py --update        # re-record the baseline
//!   scripts/layout_fidelity.py --doc easytable # one construct
//!
//! This wrapper just drives that script against the FRESHLY BUILT binary
//! (`CARGO_BIN_EXE_rustyfi-rust`) so `cargo test` picks up the current code, and
//! fails if any document's layout regresses past `scripts/layout_fidelity_
//! baseline.json`. It is `#[ignore]`d (like `typecheck_golden` /
//! `pdf_image_diff`) because it needs poppler, python3, and the corpus
//! submodules; the script itself self-skips (exit 0) when a prerequisite is
//! absent, so this never produces a false failure in a bare checkout.
//!
//! Run with:  cargo test -p rustyfi-cli --test layout_fidelity -- --ignored --nocapture

use std::path::PathBuf;
use std::process::Command;

fn repo_root() -> PathBuf {
    // crates/rustyfi-cli -> repo root
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

fn tool_present(name: &str) -> bool {
    Command::new(name)
        .arg("--version")
        .output()
        .map(|o| o.status.success() || !o.stdout.is_empty() || !o.stderr.is_empty())
        .unwrap_or(false)
}

#[test]
#[ignore = "needs poppler + python3 + docs-corpus submodules; run with --ignored"]
fn layout_matches_upstream_satysfi_within_baseline() {
    let root = repo_root();
    let script = root.join("scripts/layout_fidelity.py");
    assert!(script.exists(), "missing {}", script.display());

    // Self-skip if a prerequisite is missing, mirroring the script's own
    // behavior (and pdf_image_diff.rs's poppler probe): a bare checkout must
    // not fail this test.
    if !tool_present("python3") {
        eprintln!("SKIP layout_fidelity: python3 not available");
        return;
    }
    if !tool_present("pdftotext") {
        eprintln!("SKIP layout_fidelity: poppler pdftotext not available");
        return;
    }
    if !root.join("docs-corpus/latexcmds/doc/latexcmds-doc.pdf").exists() {
        eprintln!("SKIP layout_fidelity: docs-corpus submodules not checked out");
        return;
    }

    // Drive the freshly built binary so the test reflects the current code.
    let bin = env!("CARGO_BIN_EXE_rustyfi-rust");

    let output = Command::new("python3")
        .arg(&script)
        .arg("--bin")
        .arg(bin)
        .arg("--keep-going")
        .current_dir(&root)
        .output()
        .expect("failed to spawn python3");

    // Always surface the report (`--nocapture` shows it live; on failure the
    // captured copy is printed below).
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    print!("{stdout}");
    if !stderr.is_empty() {
        eprint!("{stderr}");
    }

    assert!(
        output.status.success(),
        "layout fidelity regressed vs upstream SATySFi (or a corpus doc failed to \
         build). Full report above; re-baseline intentional changes with \
         `scripts/layout_fidelity.py --update`.\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}"
    );
}
