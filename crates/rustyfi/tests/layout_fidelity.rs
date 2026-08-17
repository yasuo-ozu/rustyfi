//! Layout-fidelity regression test: this Rust port vs. upstream SATySFi.
//!
//! The vendored corpus ships a reference PDF built by the *original*
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
//! (`CARGO_BIN_EXE_rustyfi`) so `cargo test` picks up the current code, and
//! fails if any document's layout regresses past `scripts/layout_fidelity_
//! baseline.json`. It is `#[ignore]`d (like `typecheck_corpus` /
//! `pdf_image_diff`) because it needs poppler, python3, and the corpus
//! the vendored corpus; the script itself self-skips (exit 0) when a prerequisite is
//! absent, so this never produces a false failure in a bare checkout.
//!
//! Run with:  cargo test -p rustyfi --test layout_fidelity -- --ignored --nocapture

use std::path::PathBuf;
use std::process::Command;

fn repo_root() -> PathBuf {
    // crates/rustyfi -> repo root
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
#[ignore = "needs poppler + python3; run with --ignored"]
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
    if !root
        .join("scripts/layout_fidelity_corpus/latexcmds/doc/latexcmds-doc.pdf")
        .exists()
    {
        eprintln!("SKIP layout_fidelity: vendored corpus missing");
        return;
    }

    // Drive the freshly built binary so the test reflects the current code.
    let bin = env!("CARGO_BIN_EXE_rustyfi");

    let mut cmd = Command::new("python3");
    cmd.arg(&script).arg("--bin").arg(bin).arg("--keep-going");
    // If the ORIGINAL SATySFi is on PATH (e.g. inside `nix develop`, see
    // flake.nix), generate the reference PDFs with it so the comparison is
    // against freshly-produced official output. Otherwise the committed
    // vendored reference PDFs are used — they are the same official
    // SATySFi 0.0.11 output (the baseline was recorded via --gen-refs and the
    // two agree to <0.01 text_match), so the baseline holds either way.
    if tool_present("satysfi") {
        cmd.arg("--gen-refs");
    }

    let output = cmd
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
