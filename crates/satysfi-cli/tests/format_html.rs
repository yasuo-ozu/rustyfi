//! `--format html` end-to-end, driven through the *built* `satysfi-rust`
//! binary (`docs/plans/design-html-output.md` §Verification, "Slice-1 e2e"),
//! mirroring `tests/cache.rs`'s process-spawn harness style.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_satysfi-rust"))
}

/// This repo's `lib-satysfi/`, resolved from the crate manifest dir exactly
/// as `tests/e2e.rs`/`tests/cache.rs` do — the fixture `@require:
/// stdja-mini` from there.
fn repo_lib_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lib-satysfi")
}

fn minimal_fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/minimal.saty")
}

fn tmpdir(tag: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let p = std::env::temp_dir().join(format!(
        "satysfi-cli-format-html-{tag}-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        n
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn assert_ok(out: &Output, ctx: &str) {
    assert!(
        out.status.success(),
        "{ctx}: compile failed (code {:?})\nstdout:\n{}\nstderr:\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
}

/// `--format html` writes an `.html` file (the default `-o` extension
/// derived from the format, `main.rs`'s `cmd_compile`) whose page div and a
/// known word span are present — the CLI-level twin of
/// `crates/satysfi-pdf/tests/html.rs`'s unit-level assertions.
#[test]
fn format_html_writes_a_page_div_and_a_word_span() {
    let work = tmpdir("basic");
    let out = work.join("out.html");

    let result = Command::new(bin())
        .arg(minimal_fixture())
        .args(["-o".as_ref(), out.as_os_str()])
        .args(["--lib-root".as_ref(), repo_lib_root().as_os_str()])
        .args(["--cache-dir".as_ref(), work.join("cache").as_os_str()])
        .args(["--format", "html"])
        .output()
        .expect("spawn satysfi-rust");
    assert_ok(&result, "compile --format html");

    let html = std::fs::read_to_string(&out).expect("--format html must write the output file");
    assert!(html.starts_with("<!doctype html>"), "missing doctype:\n{html}");
    assert!(html.contains("<div class=\"page\""), "missing page div:\n{html}");
    assert!(html.contains("<span"), "missing at least one run span:\n{html}");
    assert!(html.contains("Hello,"), "missing expected fixture word:\n{html}");
    assert!(html.contains("world!"), "missing expected fixture word:\n{html}");

    std::fs::remove_dir_all(&work).ok();
}

/// Omitting `--format` keeps today's default (`pdf`) byte-identical
/// behaviour: no `--format` flag, no `-o`, defaults to `<input>.pdf` and
/// writes real PDF bytes — the regression floor this whole feature must not
/// disturb.
#[test]
fn default_format_is_still_pdf() {
    let work = tmpdir("default");
    let out = work.join("out.pdf");

    let result = Command::new(bin())
        .arg(minimal_fixture())
        .args(["-o".as_ref(), out.as_os_str()])
        .args(["--lib-root".as_ref(), repo_lib_root().as_os_str()])
        .args(["--cache-dir".as_ref(), work.join("cache").as_os_str()])
        .output()
        .expect("spawn satysfi-rust");
    assert_ok(&result, "compile with no --format");

    let bytes = std::fs::read(&out).expect("default format must write the output file");
    assert!(bytes.starts_with(b"%PDF-"), "default --format must still produce a PDF");

    std::fs::remove_dir_all(&work).ok();
}
