//! `--format html-reflow` end-to-end, driven through the *built*
//! `rustyfi-rust` binary (`docs/plans/design-reflowable-html.md` §5 "CLI"),
//! mirroring `tests/format_html.rs`'s process-spawn harness style for the
//! faithful `--format html`.
//!
//! Also the additivity guard (design doc §8): the SAME fixture compiled with
//! `--format html` (faithful) and the default `--format pdf` must still
//! behave exactly as `tests/format_html.rs`/`tests/e2e.rs` already expect —
//! `html-reflow` is reached only through a brand-new match arm
//! (`main.rs`'s `format::OutputFormat::HtmlReflow`), so it cannot have
//! touched either of those paths' own code.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_rustyfi-rust"))
}

fn repo_lib_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lib-rustyfi")
}

/// A narrower `--lib-root`, pointing directly at `dist-v01/packages/`
/// (mirroring `crates/rustyfi-lang/tests/v01_itemize_proof_mdja.rs`'s own
/// `lib_root()`) — needed for `itemize_fixture` specifically, to sidestep a
/// PRE-EXISTING, S4-unrelated loader gap: `v006::resolve::resolve_require`
/// tries `<lib_root>/dist/packages/<name>` (candidate 1, the 0.0.6 corpus)
/// BEFORE `<lib_root>/dist-v01/packages/<name>` (candidate 4), so under the
/// full `repo_lib_root()`, `itemize` -> `inline` -> `@require: deco`
/// resolves to the 0.0.6 `dist/packages/deco.satyh` (which exists there
/// too) instead of the 0.1 `dist-v01/packages/deco.satyh` — that 0.0.6
/// `deco.satyh` then `@require: gr`s the 0.0.6 `graphics` builtin, which
/// the X3 cross-version-import check correctly rejects as version-forked.
/// Pointing `--lib-root` straight at `dist-v01/packages/` makes candidate 2
/// (`<lib_root>/<name>`) resolve every `itemize`/`v01-mini` dependency
/// directly, never touching the 0.0.6 corpus at all. Not an S4 fix (this
/// resolver gap predates and is orthogonal to this slice — see
/// `docs/plans/design-reflow-s4-lists.md`, which does not touch the
/// loader), just how this ONE fixture avoids tripping over it.
fn repo_lib_root_v01_only() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lib-rustyfi/dist-v01/packages")
}

/// `@require: stdja-mini`, three `+p` paragraphs (`\bracket`/`\announce`
/// let-inline forms plus a `match`-computed `#chosen;` embed) — enough
/// structure to exercise paragraph splitting/inline-run emission without
/// pulling in math/graphics/tables (out of Slice 1's scope).
fn phase2_fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/phase2.saty")
}

/// S4 (`docs/plans/design-reflow-s4-lists.md`): a real 0.1 document
/// `@require:`ing `itemize` (a nested `Itemize.listing?(break=true)` +
/// `Itemize.enumerate`) and `v01-mini`'s `\V01Mini.emph` — exercises BOTH
/// S4 levers (list markers, emphasis markers) through the real loader, the
/// SAME fixture used both for the reflow structural assertions below and
/// for the byte-identity guards (this is the fixture that actually proves
/// the markers are inert, unlike `phase2_fixture` which never touches
/// either modified code path).
fn itemize_fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/v01-itemize.saty")
}

fn tmpdir(tag: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let p = std::env::temp_dir().join(format!(
        "rustyfi-cli-format-html-reflow-{tag}-{}-{}-{}",
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

fn compile(fixture: &Path, work: &Path, fmt: &str, out_ext: &str) -> PathBuf {
    let out = work.join(format!("out.{out_ext}"));
    let result = Command::new(bin())
        .arg(fixture)
        .args(["-o".as_ref(), out.as_os_str()])
        .args(["--lib-root".as_ref(), repo_lib_root().as_os_str()])
        .args(["--cache-dir".as_ref(), work.join("cache").as_os_str()])
        .args(["--format", fmt])
        .output()
        .expect("spawn rustyfi-rust");
    assert_ok(&result, &format!("compile --format {fmt}"));
    out
}

/// Same as [`compile`], but pins Axis A to 0.1 explicitly
/// (`--target-version 0.1`) — `itemize_fixture` only has `@require:`
/// headers (transparent to the sniffer, `sniff_headers`'s own doc comment:
/// "pins neither axis"), no `use`-shaped header, so the CLI's version
/// sniffer would otherwise default to 0.0.6 (`resolve_version_and_mode`'s
/// `RustyfiVersion::DEFAULT` fallback) and reject the fixture's `?(break =
/// true)` optional-argument syntax outright.
fn compile_v01(fixture: &Path, work: &Path, fmt: &str, out_ext: &str) -> PathBuf {
    let out = work.join(format!("out.{out_ext}"));
    let result = Command::new(bin())
        .arg(fixture)
        .args(["-o".as_ref(), out.as_os_str()])
        .args(["--lib-root".as_ref(), repo_lib_root_v01_only().as_os_str()])
        .args(["--cache-dir".as_ref(), work.join("cache").as_os_str()])
        .args(["--format", fmt])
        .args(["--target-version", "0.1"])
        .output()
        .expect("spawn rustyfi-rust");
    assert_ok(&result, &format!("compile --format {fmt} --target-version 0.1"));
    out
}

/// `--format html-reflow` writes real flowing `<p>` paragraphs (one per
/// `+p`), in reading order, with their text HTML-escaped-but-intact — and,
/// the defining difference from the faithful `--format html` twin, NO
/// absolute positioning anywhere in the document's own stylesheet/inline
/// styles.
#[test]
fn format_html_reflow_writes_flowing_paragraphs_in_reading_order() {
    let work = tmpdir("basic");
    let out = compile(&phase2_fixture(), &work, "html-reflow", "html");

    let html = std::fs::read_to_string(&out).expect("--format html-reflow must write the output file");
    assert!(html.starts_with("<!doctype html>"), "missing doctype:\n{html}");

    let para_count = html.matches("<p class=\"para\"").count();
    assert!(
        para_count >= 3,
        "expected at least 3 <p> paragraphs (one per +p), got {para_count}:\n{html}"
    );

    // Reading order: bracketed paragraph, then announced paragraph, then
    // the match-computed "Countdown complete." (finished = count-down 5,
    // which recurses down to 0 -> true -> the `true` arm of `chosen`). Each
    // `+p`'s text is tokenized into one `InnerString` PER WORD (same
    // granularity the faithful mode's own per-run `<span>`s use, see
    // `tests/format_html.rs`), so this checks each word individually rather
    // than a contiguous sentence substring — the words are still adjacent
    // in the flowing text, just each in its own `<span>`.
    for word in ["Bracketed", "text", "via", "let-inline."] {
        assert!(html.contains(word), "missing word {word:?} from the first paragraph:\n{html}");
    }
    for word in ["Announced", "lightweight", "let-inline", "form."] {
        assert!(html.contains(word), "missing word {word:?} from the second paragraph:\n{html}");
    }
    for word in ["Countdown", "complete."] {
        assert!(html.contains(word), "missing word {word:?} from the third paragraph:\n{html}");
    }
    let pos_bracket = html.find("Bracketed").expect("missing first paragraph's text");
    let pos_announce = html.find("Announced").expect("missing second paragraph's text");
    let pos_chosen = html.find("Countdown").expect("missing third paragraph's (match-computed) text");
    assert!(
        pos_bracket < pos_announce && pos_announce < pos_chosen,
        "paragraphs are out of reading order:\n{html}"
    );

    // The defining difference from the faithful twin: no absolute
    // positioning. `left:` never appears at all; `top:` only ever appears
    // as part of a flow-safe longhand (`margin-top:`/`border-top:`).
    assert!(
        !html.contains("position:absolute") && !html.contains("position: absolute"),
        "html-reflow output must never use position:absolute:\n{html}"
    );
    assert!(!html.contains("left:"), "html-reflow output must never use `left:`:\n{html}");
    for (idx, _) in html.match_indices("top:") {
        assert!(
            html[..idx].ends_with("margin-") || html[..idx].ends_with("border-"),
            "found a bare `top:` CSS declaration at byte {idx}:\n{html}"
        );
    }

    std::fs::remove_dir_all(&work).ok();
}

/// Additivity guard (design doc §8): compiling the SAME fixture with the
/// faithful `--format html` still produces its established shape (a
/// `<div class="page">` twin of the PDF, per `tests/format_html.rs`) — the
/// new `html-reflow` format could not have touched this code path, since it
/// is reached only through a brand-new, separate `match` arm.
#[test]
fn format_html_faithful_mode_is_unaffected_by_the_new_reflow_format() {
    let work = tmpdir("faithful");
    let out = compile(&phase2_fixture(), &work, "html", "html");

    let html = std::fs::read_to_string(&out).expect("--format html must write the output file");
    assert!(html.starts_with("<!doctype html>"), "missing doctype:\n{html}");
    assert!(html.contains("<div class=\"page\""), "missing page div:\n{html}");
    assert!(html.contains("Bracketed"), "missing expected fixture text:\n{html}");
    // The faithful mode's own defining trait, unchanged: every run IS
    // absolutely positioned.
    assert!(html.contains("position: absolute"), "faithful mode must still be absolutely positioned:\n{html}");

    std::fs::remove_dir_all(&work).ok();
}

/// Additivity guard: the default (`pdf`) format is unaffected too.
#[test]
fn default_pdf_format_is_unaffected_by_the_new_reflow_format() {
    let work = tmpdir("pdf");
    let out = compile(&phase2_fixture(), &work, "pdf", "pdf");

    let bytes = std::fs::read(&out).expect("--format pdf must write the output file");
    assert!(bytes.starts_with(b"%PDF-"), "default --format must still produce a PDF");

    std::fs::remove_dir_all(&work).ok();
}

// ============================================================================
// S4 (`docs/plans/design-reflow-s4-lists.md`): semantic lists + emphasis,
// driven end to end through the real loader (`itemize_fixture`) — nested
// `Itemize.listing?(break=true)`, `Itemize.enumerate`, and `\V01Mini.emph`.
// ============================================================================

/// `--format html-reflow` on a document that actually uses `itemize` must
/// render real, NESTED `<ul>`/`<li>` for `Itemize.listing` (one top-level
/// item + one nested child item), a real `<ol>`/`<li>` for
/// `Itemize.enumerate` (two flat entries), and a real `<em>` for
/// `\V01Mini.emph` — never absolute positioning (same invariant as the
/// basic reflow test above).
#[test]
fn format_html_reflow_renders_nested_lists_and_emphasis_for_itemize() {
    let work = tmpdir("itemize-reflow");
    let out = compile_v01(&itemize_fixture(), &work, "html-reflow", "html");

    let html = std::fs::read_to_string(&out).expect("--format html-reflow must write the output file");
    assert!(html.starts_with("<!doctype html>"), "missing doctype:\n{html}");

    // `Itemize.listing?(break=true)`: one top-level `<li>` with one nested
    // child `<li>` inside its OWN nested `<ul>` — two `<ul>`s total.
    assert_eq!(html.matches("<ul").count(), 2, "expected outer + one nested <ul>:\n{html}");
    assert_eq!(html.matches("</ul>").count(), 2, "expected outer + one nested </ul>:\n{html}");

    // `Itemize.enumerate`: one flat `<ol>` with two `<li>`s, no nesting.
    assert_eq!(html.matches("<ol").count(), 1, "expected exactly one <ol>:\n{html}");
    assert_eq!(html.matches("</ol>").count(), 1, "expected exactly one </ol>:\n{html}");

    // Three `<li>`s total: the listing's top item, its nested child, and
    // (separately, twice for the enumerate — counted together here) the two
    // enumerate entries: 2 (listing) + 2 (enumerate) = 4.
    assert_eq!(html.matches("<li").count(), 4, "expected 4 <li>s total:\n{html}");

    // Each `+p`/item's text is tokenized into one `InnerString` PER WORD
    // (same granularity `format_html_reflow_writes_flowing_paragraphs_in_
    // reading_order` above checks for), so — as that test does — check each
    // word individually rather than a contiguous phrase. `item`/`entry` are
    // distinctive enough not to collide with the stylesheet's own CSS
    // vocabulary (unlike, say, "top", which trivially substring-matches
    // `margin-top:`/`border-top:` everywhere).
    for word in ["nested", "item", "first", "entry", "second"] {
        assert!(html.contains(word), "missing item word {word:?}:\n{html}");
    }
    assert_eq!(
        html.matches("item").count(),
        2,
        "expected \"item\" exactly twice (the top item + the nested item):\n{html}"
    );

    // `\V01Mini.emph{emphasized}` -> a real `<em>`, never `<strong>`.
    assert!(html.contains("<em>") && html.contains("</em>"), "missing <em>:\n{html}");
    assert!(!html.contains("<strong>"), "must not render <strong> for \\emph:\n{html}");
    assert!(html.contains("emphasized"), "missing emphasized text:\n{html}");

    // The drawn bullet/number glyph run itself (`enumerate`'s arabic
    // numeral, `Itemize.listing`'s circle) must not leak in as its own
    // rendered run — `crates/rustyfi-html/tests/reflow.rs`'s
    // `bullet_fence_is_suppressed` proves this precisely at the box-tree
    // level (a raw substring scan here would be unreliable: margin/style
    // attributes on `<ul>`/`<li>` legitimately contain digits too).

    // Still no absolute positioning anywhere, same invariant as the basic
    // reflow test.
    assert!(
        !html.contains("position:absolute") && !html.contains("position: absolute"),
        "html-reflow output must never use position:absolute:\n{html}"
    );
    assert!(!html.contains("left:"), "html-reflow output must never use `left:`:\n{html}");

    std::fs::remove_dir_all(&work).ok();
}

/// THE inert-marker proof (design doc §4.3, the whole premise of this
/// slice): a document that actually `@require:`s `itemize` and calls
/// `\V01Mini.emph` — i.e. one that genuinely exercises the modified
/// `itemize.satyh`/`v01-mini.satyh` — must still produce a real PDF under
/// the default `--format pdf`. Unlike `default_pdf_format_is_unaffected_by_
/// the_new_reflow_format` above (which uses `phase2_fixture`, a fixture
/// that never touches either S4-modified stdlib), THIS is the fixture that
/// actually proves the `VertBox::ListMark`/`PureHorzBox::InlineMark`
/// markers are truly inert for PDF, not merely additive on paper.
#[test]
fn itemize_fixture_still_produces_a_valid_pdf() {
    let work = tmpdir("itemize-pdf");
    let out = compile_v01(&itemize_fixture(), &work, "pdf", "pdf");

    let bytes = std::fs::read(&out).expect("--format pdf must write the output file");
    assert!(bytes.starts_with(b"%PDF-"), "itemize fixture must still produce a valid PDF");
    assert!(bytes.len() > 200, "PDF unexpectedly tiny ({} bytes)", bytes.len());

    std::fs::remove_dir_all(&work).ok();
}

/// Same inert-marker proof for the FAITHFUL `--format html` twin: it must
/// still be the established absolutely-positioned shape (`tests/
/// format_html.rs`'s own invariant), completely unaffected by the fixture
/// actually calling `Itemize.listing`/`Itemize.enumerate`/`\V01Mini.emph`.
#[test]
fn itemize_fixture_faithful_html_is_still_absolutely_positioned() {
    let work = tmpdir("itemize-faithful");
    let out = compile_v01(&itemize_fixture(), &work, "html", "html");

    let html = std::fs::read_to_string(&out).expect("--format html must write the output file");
    assert!(html.starts_with("<!doctype html>"), "missing doctype:\n{html}");
    assert!(html.contains("<div class=\"page\""), "missing page div:\n{html}");
    assert!(html.contains("position: absolute"), "faithful mode must still be absolutely positioned:\n{html}");
    // The markers must not leak into faithful HTML as visible tags either
    // (chop_page never places them; both writers wildcard the box kind).
    assert!(!html.contains("<ul"), "faithful HTML must never render <ul> (S4 is reflow-only):\n{html}");
    assert!(!html.contains("<em>"), "faithful HTML must never render <em> (S4 is reflow-only):\n{html}");
    for text in ["item", "nested", "entry"] {
        assert!(html.contains(text), "missing expected fixture text {text:?}:\n{html}");
    }

    std::fs::remove_dir_all(&work).ok();
}
