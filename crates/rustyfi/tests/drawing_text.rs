//! Text INSIDE a drawing survives, in both text backends.
//!
//! `figbox` composes a figure out of `draw-text`: `textbox {caption} |>
//! hvmargin 6pt |> frame 1pt Color.black` is an `inline-graphics` holding a
//! stroked rectangle and a `draw-text` of an `embed-block-top`, and that
//! composition is the whole of `\fig-inline` and `+fig-center`. Both backends
//! lost the words in it, for unrelated reasons:
//!
//! - **Markdown** dropped them outright. `svg::graphics_block` emitted the
//!   paths and skipped every `GraphicsElem::Text`, on the correct grounds that
//!   an HTML child inside `<svg>` ends the parser's foreign-content mode and
//!   ejects the rest of the drawing — but the conclusion should have been
//!   `<text>`, which is SVG and composes. Its own doc comment said "the caller
//!   is responsible for their text"; the caller had nowhere to put it, and
//!   never did.
//! - **HTML** kept them and broke them out of the frame. The box is an
//!   inline-block sized to the port's own measurement, and the reader's
//!   metrics are wider often enough that the browser re-wrapped text the port
//!   had fitted on one line: `leftward` rendered as `left-` inside the
//!   rectangle and `ward` on the line below it. A one-line box is now
//!   `white-space: nowrap`, and with nothing able to break inside it the
//!   soft hyphen is not written either, so the word is one string in the file
//!   (`Ctx::nobreak`; the suppression itself is pinned unit-level in
//!   `rustyfi-html/tests/reflow.rs`, where a `Discretionary` can be placed
//!   inside an embedded block directly).
//!
//! The multi-line box in the fixture is the control for that: there the
//! declared width IS the document's wrapping instruction and the browser is
//! meant to re-break.
//!
//! **No bundled face is needed**, and that is a CONSTRAINT on what may be
//! asserted here rather than a remark. Hyphenation is dictionary-driven and
//! every assertion is about which characters appear in which element, so this
//! holds against the built-in base-14 fonts exactly as against
//! `download-fonts.sh`'s — but only for as long as nothing writes a measured
//! number down. `.github/workflows/ci.yml` runs `download-fonts.sh` in the
//! fidelity, real-packages and release jobs; `build · clippy · test`, the one
//! that runs `cargo test`, does NOT. A width literal therefore passes on any
//! developer's tree and fails CI on a perfectly valid checkout. Two of them
//! did. Identify a box by its CONTENT; the fixture's own `30pt` is the sole
//! number that may be matched, because the fixture writes it.
//!
//! Skipping when the faces are absent — the idiom the rest of this suite uses,
//! and the right one for a test that genuinely needs a face — would be the
//! wrong fix HERE, and not a mild one: line 37 is the only `cargo test` in the
//! whole workflow (the fidelity job runs one `--ignored` target, the corpus job
//! one syntax target), so a skip would take this test out of CI altogether.
//! Nothing in it needs a face; it only needed to stop saying so in numbers.

use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_rustyfi"))
}

fn repo_lib_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lib-rustyfi")
}

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/drawing-text.saty")
}

fn render(format: &str, ext: &str) -> String {
    let work = std::env::temp_dir().join(format!(
        "rustyfi-drawing-text-{}-{}-{}",
        format,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
    ));
    std::fs::create_dir_all(&work).unwrap();
    let out = work.join(format!("out.{ext}"));
    let result = Command::new(bin())
        .arg(fixture())
        .args(["-o".as_ref(), out.as_os_str()])
        .args(["--lib-root".as_ref(), repo_lib_root().as_os_str()])
        .args(["--cache-dir".as_ref(), work.join("cache").as_os_str()])
        .args(["--format", format, "--no-cache", "--no-aux"])
        .output()
        .expect("failed to spawn rustyfi");
    assert!(
        result.status.success(),
        "rustyfi --format {format} failed: {}",
        String::from_utf8_lossy(&result.stderr),
    );
    let text = std::fs::read_to_string(&out).expect("no output written");
    let _ = std::fs::remove_dir_all(&work);
    text
}

/// Every `embed-inline` span in the body, as (style, content).
///
/// Content, because a style's width is the port's measurement of whichever
/// faces the run found and the module header forbids matching one. An
/// embedded box in this fixture holds a bare string, so scanning to the next
/// `</span>` is exact; were a nested element ever to appear the content would
/// come back truncated and the equality below would fail rather than pass for
/// the wrong reason.
fn embed_inline_spans(body: &str) -> Vec<(String, String)> {
    const OPEN: &str = "class=\"embed-inline\" style=\"";
    let mut out = Vec::new();
    let mut rest = body;
    while let Some(at) = rest.find(OPEN) {
        rest = &rest[at + OPEN.len()..];
        let Some(style_end) = rest.find('"') else { break };
        let style = rest[..style_end].to_string();
        rest = &rest[style_end..];
        let Some(gt) = rest.find('>') else { break };
        rest = &rest[gt + 1..];
        let Some(close) = rest.find("</span>") else { break };
        out.push((style, rest[..close].to_string()));
        rest = &rest[close..];
    }
    out
}

/// Markdown: a drawing's words are in it, as SVG `<text>`.
#[test]
fn a_markdown_drawing_carries_its_text() {
    let md = render("markdown", "md");
    assert!(
        md.contains("<svg "),
        "the drawing itself is missing, so this test is measuring nothing: {md}",
    );
    for word in ["leftward", "boxedword"] {
        assert!(
            md.contains(&format!(">{word}</text>")),
            "`{word}` is not in the Markdown drawing — every `draw-text` in a \
             figure was being dropped: {md}",
        );
    }
}

/// The `<text>` is INSIDE the `<svg>`, and the `<svg>` is still one line.
///
/// Both halves matter and neither implies the other. Outside the `<svg>` the
/// words would be loose markup beside the picture rather than in it; and a
/// `<svg>` broken across lines stops being a Markdown HTML block, so the
/// reader's own parser folds `<br>`s into the middle of the path data.
#[test]
fn the_markdown_text_is_inside_the_one_line_svg() {
    let md = render("markdown", "md");
    for line in md.lines().filter(|l| l.contains("<svg ")) {
        assert_eq!(
            line.matches("<svg ").count(),
            line.matches("</svg>").count(),
            "a drawing is split across lines: {line}",
        );
        let svg_start = line.find("<svg ").unwrap();
        let svg_end = line.find("</svg>").unwrap();
        for (i, _) in line.match_indices("<text ") {
            assert!(
                i > svg_start && i < svg_end,
                "a `<text>` escaped its drawing: {line}",
            );
        }
    }
}

/// The `<text>` is not inside the y-flipping `<g>`, which would mirror it.
///
/// SVG text has no orientation-independence: a glyph inside `scale(1,-1)`
/// renders upside-down. The paths need that flip (box-local coordinates are
/// y-up) and the text must therefore be placed by hand outside it — the same
/// reason `reflow::inline`'s math glyphs are.
#[test]
fn the_markdown_text_is_not_inside_the_flip() {
    let md = render("markdown", "md");
    let line = md.lines().find(|l| l.contains("<text ")).expect("no text");
    let flip_end = line.find("</g>").expect("no flipped group");
    let text_at = line.find("<text ").unwrap();
    assert!(
        text_at > flip_end,
        "the `<text>` is inside `scale(1,-1)` and will render mirrored: {line}",
    );
}

/// HTML: a ONE-LINE box does not re-break, so its text stays inside it.
///
/// The declared `width` is the port's measurement of ITS faces. The reader's
/// are wider often enough, and the browser then wraps text the port fitted on
/// one line — pushing the tail out from under the rectangle the document drew
/// round it. Measured: `leftward` rendered as `left-` inside the frame and
/// `ward` on the line below.
///
/// The MULTI-line box is the control on the same line: there the width IS the
/// document's wrapping instruction, and re-breaking at the reader's metrics is
/// the whole premise of this backend.
///
/// The one-line boxes are found by the word in them, not by their width, for
/// the reason in the module header — and finding them that way asserts the
/// other half at the same time: the word is one whole string, which is what a
/// reader searching the file for it actually needs.
#[test]
fn a_one_line_box_does_not_rebreak_but_a_multi_line_one_does() {
    let html = render("html", "html");
    let body = html.split_once("<body>").map(|(_, b)| b).unwrap_or(&html);
    let spans = embed_inline_spans(body);
    assert!(
        !spans.is_empty(),
        "no embedded box reached the page at all, so this test is measuring \
         nothing: {body}",
    );
    for word in ["leftward", "boxedword"] {
        let (style, _) = spans
            .iter()
            .find(|(_, content)| content.as_str() == word)
            .unwrap_or_else(|| {
                panic!(
                    "`{word}` is not one whole string in a box of its own — a \
                     soft hyphen split it, or it never reached the page: {body}"
                )
            });
        assert!(
            style.contains("white-space:nowrap"),
            "the one-line box holding `{word}` must not re-break at the \
             reader's own metrics, but its style is `{style}`: {body}",
        );
    }
    let (style, _) = spans
        .iter()
        .find(|(style, _)| style.contains("width:30pt"))
        .unwrap_or_else(|| {
            panic!("the MULTI-line box is gone, so its control is vacuous: {body}")
        });
    assert!(
        !style.contains("nowrap"),
        "the MULTI-line box's width is the document's own wrapping instruction \
         and must stay breakable, but its style is `{style}`: {body}",
    );
}
