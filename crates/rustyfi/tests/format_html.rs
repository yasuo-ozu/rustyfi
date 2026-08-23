//! `--format html-fixed` end-to-end, driven through the *built* `rustyfi`
//! binary ("Slice-1 e2e"), mirroring `tests/cache.rs`'s process-spawn harness
//! style.
//!
//! `html-fixed` is the layout-faithful serialization — one `.page` div per
//! page, every run absolutely positioned. It used to answer to plain
//! `--format html`; that name now means the reflowable backend
//! (`tests/format_html_reflow.rs`), so the two assertions this file makes
//! about the page grid moved with the format they describe. The one test
//! here that still says `--format html` is
//! [`format_html_is_the_reflowable_backend`], whose whole point is that it
//! no longer produces a page grid.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_rustyfi"))
}

/// This repo's `lib-rustyfi/`, resolved from the crate manifest dir exactly
/// as `tests/e2e.rs`/`tests/cache.rs` do — the fixture `@require:
/// stdja-mini` from there.
fn repo_lib_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lib-rustyfi")
}

fn minimal_fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/minimal.saty")
}

fn tmpdir(tag: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let p = std::env::temp_dir().join(format!(
        "rustyfi-format-html-{tag}-{}-{}-{}",
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

/// Compile `minimal.saty` under `--format {fmt}`, returning the written
/// HTML. Shared by every test below, since the only thing that varies
/// between them is the format name and what the output is expected to
/// contain.
fn compile_html(tag: &str, fmt: &str) -> (PathBuf, String) {
    let work = tmpdir(tag);
    let out = work.join("out.html");

    let result = Command::new(bin())
        .arg(minimal_fixture())
        .args(["-o".as_ref(), out.as_os_str()])
        .args(["--lib-root".as_ref(), repo_lib_root().as_os_str()])
        .args(["--cache-dir".as_ref(), work.join("cache").as_os_str()])
        .args(["--format", fmt])
        .output()
        .expect("spawn rustyfi");
    assert_ok(&result, &format!("compile --format {fmt}"));

    let html = std::fs::read_to_string(&out).expect("--format html* must write the output file");
    (work, html)
}

/// `--format html-fixed` writes an `.html` file (the default `-o` extension
/// derived from the format, `main.rs`'s `cmd_compile`) whose page div and a
/// known word span are present — the CLI-level twin of
/// `crates/rustyfi-pdf/tests/html.rs`'s unit-level assertions.
#[test]
fn format_html_fixed_writes_a_page_div_and_a_word_span() {
    let (work, html) = compile_html("basic", "html-fixed");
    assert!(
        html.starts_with("<!doctype html>"),
        "missing doctype:\n{html}"
    );
    assert!(
        html.contains("<div class=\"page\""),
        "missing page div:\n{html}"
    );
    assert!(
        html.contains("<span"),
        "missing at least one run span:\n{html}"
    );
    // Checked against the TEXT, tags stripped: a word is not one `InnerString`.
    // Hyphenation is on by default (as upstream loads english.satysfi-hyph into
    // every initial context), so `Hello,` is carried as `Hel` + `lo,` either
    // side of a discretionary and reaches the faithful HTML as two `<span>`s —
    // adjacent and rendered identically, but two elements in the markup.
    let text = rendered_text(&html);
    assert!(
        text.contains("Hello,"),
        "missing expected fixture word:\n{html}"
    );
    assert!(
        text.contains("world!"),
        "missing expected fixture word:\n{html}"
    );

    std::fs::remove_dir_all(&work).ok();
}

/// The page's rendered TEXT: tags stripped and all whitespace dropped, since
/// each `<span>` sits on its own source line and a word can span several of
/// them.
fn rendered_text(html: &str) -> String {
    let mut out = String::new();
    let mut depth = 0usize;
    for ch in html.chars() {
        match ch {
            '<' => depth += 1,
            '>' => depth = depth.saturating_sub(1),
            c if depth == 0 && !c.is_whitespace() => out.push(c),
            _ => {}
        }
    }
    out
}

/// The rename's own regression floor: plain `--format html` is now the
/// REFLOWABLE backend, so the very assertion the test above makes must FAIL
/// here — no page grid, no absolutely-positioned run — while the document
/// still says what it says. See `crates/rustyfi/src/format.rs`'s type doc
/// comment for why this name went to the reflowable side.
#[test]
fn format_html_is_the_reflowable_backend() {
    let (work, html) = compile_html("reflow-default", "html");
    assert!(
        html.starts_with("<!doctype html>"),
        "missing doctype:\n{html}"
    );
    assert!(
        !html.contains("<div class=\"page\""),
        "--format html must no longer emit a page grid:\n{html}"
    );
    assert!(
        !html.contains("position:absolute") && !html.contains("position: absolute"),
        "--format html must not position anything absolutely:\n{html}"
    );
    assert!(
        html.contains("<p class=\"para\""),
        "--format html must emit real flowing paragraphs:\n{html}"
    );
    let text = rendered_text(&html);
    assert!(
        text.contains("Hello,") && text.contains("world!"),
        "missing expected fixture words:\n{html}"
    );

    std::fs::remove_dir_all(&work).ok();
}

/// `--format html-reflow` — the name the reflowable backend answered to
/// while `html` still meant the faithful one — keeps working as a pure
/// alias, so no existing script breaks over the rename. Byte-identical
/// output, not merely a similar shape.
#[test]
fn format_html_reflow_is_an_alias_of_html() {
    let (work_a, via_alias) = compile_html("alias", "html-reflow");
    let (work_b, via_html) = compile_html("alias-canonical", "html");
    assert_eq!(
        via_alias, via_html,
        "--format html-reflow must be an exact alias of --format html"
    );

    std::fs::remove_dir_all(&work_a).ok();
    std::fs::remove_dir_all(&work_b).ok();
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
        .expect("spawn rustyfi");
    assert_ok(&result, "compile with no --format");

    let bytes = std::fs::read(&out).expect("default format must write the output file");
    assert!(
        bytes.starts_with(b"%PDF-"),
        "default --format must still produce a PDF"
    );

    std::fs::remove_dir_all(&work).ok();
}
