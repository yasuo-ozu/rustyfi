//! `--format html`: the three `latexcmds` box shapes, each of which the
//! reflow backend got wrong in a different way — and only one of them was a
//! decoration problem.
//!
//! Measured against the PDF of the same document, rasterised at 130dpi, on
//! `layout-tests/corpus/latexcmds`:
//!
//! | shape | PDF | HTML before |
//! |--|--|--|
//! | `\fbox` | a rectangle with 3.4pt of padding all round | the rectangle squashed into the FONT's content area, 39.2 x 16.1pt drawn into 43.8 x 12.75 |
//! | `\framebox(4cm)` | a 4cm box, inline, text centred inside | the text ejected onto a centred line of its own, the box collapsed to an empty square |
//! | `\rotatebox(0.25)` | the word set at about 14° | upright |
//!
//! Three causes, none of them the missing-decoration bug the rest of this
//! branch fixes:
//!
//! - `\framebox` is a LAYOUT failure. It is `\fbox{\makebox(wid){…}}`, and
//!   `\makebox` is an `embed-block-top` — which `block.rs` treated as
//!   block-level wherever it appeared, flushing the paragraph the box was one
//!   word of. See `block::lone_embedded_block`.
//! - `\rotatebox` is a DROPPED TRANSFORM. `GraphicsElem::Text::transform` is
//!   read by the PDF writer (`rustyfi-pdf/src/lib.rs:941`, as a `cm`) and was
//!   ignored by `svg.rs`, so the one thing that command does did not happen.
//! - `\fbox`/`\shadowbox` are GEOMETRY. CSS measures an inline background
//!   against the font's content area, and the frame's own vertical padding —
//!   folded into the fragment's height by `append_vert_padding` and invisible
//!   to CSS — has to be put back as real padding or the drawing is compressed
//!   into the text's own extent.
//!
//! The assertions are on the EMITTED MARKUP rather than on a rendering: what
//! each element is and what declarations it carries is the whole of what
//! changed, and it is checkable without a browser.
//!
//! **No bundled face is needed and none is used.** Nothing here asserts a
//! measurement — the numbers that appear are the DOCUMENT's own (a 120pt
//! `embed-block-top`, a 3pt pad, a rotation of 0.25 rad), which are the same
//! whatever face renders them. There is deliberately no skip, for the same
//! reason `html_inline_frame_deco.rs` has none.

use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_rustyfi"))
}

fn repo_lib_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lib-rustyfi")
}

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/inline-box-commands.saty")
}

fn tmpdir() -> PathBuf {
    let p = std::env::temp_dir().join(format!(
        "rustyfi-box-commands-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn render_html() -> String {
    let work = tmpdir();
    let out = work.join("out.html");
    let result = Command::new(bin())
        .arg(fixture())
        .args(["-o".as_ref(), out.as_os_str()])
        .args(["--lib-root".as_ref(), repo_lib_root().as_os_str()])
        .args(["--cache-dir".as_ref(), work.join("cache").as_os_str()])
        .args(["--format", "html", "--no-cache", "--no-aux"])
        .output()
        .expect("failed to spawn rustyfi");
    assert!(
        result.status.success(),
        "rustyfi --format html failed: {}",
        String::from_utf8_lossy(&result.stderr),
    );
    let html = std::fs::read_to_string(&out).expect("no HTML written");
    let _ = std::fs::remove_dir_all(&work);
    html
}

fn body_of(html: &str) -> String {
    html.split_once("<body>")
        .map(|(_, b)| format!("<body>{b}"))
        .unwrap_or_else(|| html.to_string())
}

/// The declarations of one `.ideco-N` rule.
fn ideco_rule(html: &str, n: usize) -> String {
    let head = format!(".ideco-{n} {{ ");
    let start = html.find(&head).unwrap_or_else(|| {
        let found: Vec<&str> = html.lines().filter(|l| l.contains(".ideco")).collect();
        panic!("no `.ideco-{n}` rule; `.ideco` lines present: {found:?}")
    }) + head.len();
    let rest = &html[start..];
    rest[..rest.find('}').expect("unterminated CSS rule")].to_string()
}

/// `\framebox`: a fixed-width embedded block in the MIDDLE of a sentence stays
/// in the sentence.
///
/// This is the one that destroyed content rather than merely misdrawing it.
/// `block.rs`'s `EmbeddedBlock` arm flushed the open paragraph and opened a
/// `<div class="embed">` for every embedded block it met, so `A \framebox(4cm)
/// {…} B` came out as three block elements: `A` and the frame's empty opening
/// half, then the box's text alone on a centred line, then `B`. The frame's
/// own wrapper was left around nothing, which is why the decoration
/// (correctly recorded, correctly replayed) painted a tiny empty square.
#[test]
fn a_fixed_width_embedded_block_stays_inline_in_its_sentence() {
    let html = render_html();
    let body = body_of(&html);

    // The whole sentence in ONE paragraph, the box a span inside it.
    let para = body
        .lines()
        .find(|l| l.contains("fixed width box"))
        .unwrap_or_else(|| panic!("the embedded block's text is gone: {body}"));
    assert!(
        para.contains("A <span class=\"iframe ideco")
            && para.trim_end().ends_with("B</p>"),
        "the box left the line it was a word of — `A` and `B` must be in the \
         same paragraph as it: {para}",
    );
    assert!(
        !body.contains("<div class=\"embed\""),
        "an embedded block used as a WORD must not become a block-level \
         `<div>`: {body}",
    );

    // Sized to the document's own measure, centred as `\makebox`'s
    // `inline-fil ++ … ++ inline-fil` asks, and not re-broken (the port fitted
    // it on one line; the reader's own metrics must not split it out of the
    // frame).
    assert!(
        para.contains(
            "<span class=\"embed-inline\" style=\"width:120pt; text-align:center; \
             white-space:nowrap;\">fixed width box</span>"
        ),
        "the embedded block is not the document's own 120pt centred box: {para}",
    );
}

/// A LONE embedded block is still block-level — the control for the test
/// above, and the behaviour every centred figure in the corpus depends on.
///
/// `single-centering-line`/`+fig-block` put an embedded block on a line of its
/// own between two `inline-fil`s, and there the `<div>` is right: it is a real
/// block-level thing the box stream had no other way to say. A fix for
/// `\framebox` that made every embedded block inline would take those out of
/// the flow, and `crates/rustyfi-html/tests/reflow.rs`'s
/// `embedded_block_becomes_a_nested_div_recursively` is the unit-level
/// statement of the same rule.
#[test]
fn the_paragraph_boxes_are_the_only_inline_ones() {
    let html = render_html();
    // Every embedded block in THIS fixture is a word in a sentence, so none of
    // them may be a `<div>`; the lone-block case is pinned in `reflow.rs`,
    // where a one-box line can be built directly.
    assert_eq!(
        html.matches("class=\"embed-inline\"").count(),
        1,
        "expected exactly the one `\\framebox` to be an inline embedded \
         block: {}",
        body_of(&html),
    );
}

/// `\rotatebox`: the run's own 2x2 matrix reaches the HTML.
///
/// `cos 0.25 = 0.968912…`, `sin 0.25 = 0.247403…`, and the CSS matrix is
/// `(a, -c, -b, d)` of the port's row-major y-UP one — a flip conjugation,
/// which for a rotation is the sign of the angle (see `svg::CssMatrix`). So a
/// counter-clockwise quarter-radian in the document is
/// `matrix(cos, -sin, sin, cos)` here, and writing it unconjugated would
/// silently mirror the run.
#[test]
fn a_rotated_draw_text_run_carries_its_matrix() {
    let html = render_html();
    let body = body_of(&html);
    let dtx = body
        .lines()
        .find(|l| l.contains("class=\"dtx\""))
        .unwrap_or_else(|| panic!("no placed `draw-text` run at all: {body}"));

    let cos = 0.25_f64.cos();
    let sin = 0.25_f64.sin();
    assert!(
        dtx.contains(&format!("transform:matrix({cos},{},{sin},{cos},0,0)", -sin)),
        "the rotation was dropped, or written in the port's own y-up \
         convention (which would mirror it): {dtx}",
    );
    // About the run's LEFT-BASELINE point, which is what the matrix was
    // composed about — and which only the strut makes nameable in CSS.
    assert!(
        dtx.contains("transform-origin:0 ") && dtx.contains("class=\"dtx-strut\""),
        "a transform with no origin turns the run about its top-left corner: {dtx}",
    );
}

/// `\fbox`: the frame's own vertical padding comes back as CSS padding, so the
/// drawing is not compressed into the font's content area.
///
/// The fixture pads by 3pt on all four sides. Horizontally those are already
/// in the flow — `append_horz_padding` splices them into the box stream and
/// `inline.rs` renders them as `hskip` struts INSIDE this very wrapper — so
/// only the vertical pair may be written, or they would be applied twice.
#[test]
fn a_stretched_frame_restores_its_vertical_padding_only() {
    let html = render_html();
    let rule = ideco_rule(&html, 0);
    assert!(
        rule.contains("background-size:100% 100%"),
        "a frame straddling the baseline is stretched to its box, not tiled: {rule}",
    );
    assert!(
        rule.contains("padding-top:3pt;") && rule.contains("padding-bottom:3pt;"),
        "without the frame's own vertical pads the drawing is squashed into \
         the font's content area: {rule}",
    );
    assert!(
        !rule.contains("padding-left") && !rule.contains("padding-right"),
        "the horizontal pads are already in the flow as `hskip` struts; \
         writing them here doubles them: {rule}",
    );
    // The struts that make that true, in the wrapper this rule paints.
    let body = body_of(&html);
    let para = body.lines().find(|l| l.contains(">framed<")).unwrap();
    assert_eq!(
        para.matches("class=\"hskip\" style=\"width:3pt;\"").count(),
        2,
        "the frame's left and right pads should be the flow's own struts: {para}",
    );
}

/// The word space in front of a decorated region is OUTSIDE it.
///
/// A wrapper opens positionally while glue resolves lazily at the next run, so
/// the space between `A` and `\fbox{…}` was landing one tag deeper — inside
/// the region whose background the decoration paints. Measured: it made the
/// drawn box 43.8pt wide where the frame is 39.2pt, and started the rectangle
/// at the `A`.
#[test]
fn the_space_before_a_decorated_region_is_outside_it() {
    let html = render_html();
    let body = body_of(&html);
    assert!(
        body.contains("A <span class=\"iframe ideco ideco-0\"><span class=\"hskip\""),
        "the space between `A` and the frame belongs before the wrapper, not \
         inside it: {body}",
    );
}
