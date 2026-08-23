//! Integration tests for the reflowable/semantic HTML writer
//! (`render_html_reflow`, `--format html`): hand-built `Vec<VertBox>`
//! fixtures, mirroring `tests/html.rs`'s synthetic box-construction style
//! for the faithful writer.
//!
//! Roughly in the order the file runs: paragraph grouping and splitting,
//! frame and embedded-block nesting, run styling, math/graphics/table/link
//! structure, lists and emphasis — then three groups that carry most of the
//! weight:
//!
//! - **"Text is TEXT"**: what a glue box becomes. The box stream is not a
//!   word stream, and mapping glue to U+0020 rendered Japanese one
//!   `<span>`-per-character with a space between each. See
//!   `reflow/text.rs`.
//! - **Footnotes, images, alignment, headings, margins**: the constructs
//!   that used to be dropped or mis-derived.
//! - **Well-formedness**: run coalescing deliberately leaves a `<span>`
//!   open across boxes, so a missing close at any boundary yields silently
//!   misnested markup rather than a crash. `assert_balanced_tags` is the
//!   guard.
//!
//! Throughout: the "no absolute positioning" invariant that is the defining
//! difference from the faithful mode.

use rustyfi_backend::{
    AnnotAction, Closing, Color, DecoId, DocExtras, FontKey, GraphicsElem, HorzStringInfo,
    InlineMarkKind, Length, ListMarkKind, MathGlyph, OutlineEntry, PageGeometry, Path, PathSeg,
    PureHorzBox, Subpath, TabularBox, TabularCellBox, VertBox,
};

fn geometry() -> PageGeometry {
    PageGeometry {
        paper_width: Length::pt(200.0),
        paper_height: Length::pt(300.0),
        text_origin: (Length::pt(20.0), Length::pt(20.0)),
        text_width: Length::pt(160.0),
        text_height: Length::pt(260.0),
    }
}

fn text_run(text: &str) -> PureHorzBox {
    PureHorzBox::InnerString {
        info: HorzStringInfo {
            font: FontKey(0),
            size: Length::pt(12.0),
            rising: Length::ZERO,
            color: Color::Gray(0.0),
        },
        text: text.to_string(),
        width: Length::pt(80.0),
        height: Length::pt(9.0),
        depth: Length::pt(2.0),
    }
}

fn line(bx: PureHorzBox) -> VertBox {
    VertBox::Line {
        height: Length::pt(9.0),
        depth: Length::pt(2.0),
        leading: Length::pt(12.0),
        contents: vec![(Length::ZERO, bx)],
    }
}

fn text_line(text: &str) -> VertBox {
    line(text_run(text))
}

fn render(vboxes: &[VertBox]) -> String {
    render_with_links(vboxes, &[], &[])
}

fn render_with_links(
    vboxes: &[VertBox],
    links: &[(DecoId, AnnotAction)],
    dests: &[(DecoId, String)],
) -> String {
    render_with_extras(vboxes, &DocExtras::default(), links, dests)
}

/// S3 ("S3"): full-signature helper exercising `extras.outline`
/// alongside `links`/`dests`.
fn render_with_extras(
    vboxes: &[VertBox],
    extras: &DocExtras,
    links: &[(DecoId, AnnotAction)],
    dests: &[(DecoId, String)],
) -> String {
    rustyfi_html::render_html_reflow(Some(vboxes), &geometry(), &[], extras, links, dests)
        .expect("reflow HTML rendering must succeed")
}

#[test]
fn consecutive_lines_coalesce_and_skip_splits_into_two_paragraphs() {
    let vboxes = vec![
        text_line("Hello,"),
        text_line("world!"),
        VertBox::Skip(Length::pt(12.0)),
        text_line("Second paragraph."),
    ];
    let html = render(&vboxes);

    assert!(
        html.starts_with("<!doctype html>"),
        "missing doctype:\n{html}"
    );
    let para_count = html.matches("<p class=\"para\"").count();
    assert_eq!(para_count, 2, "expected exactly two <p>s:\n{html}");
    assert!(
        html.contains("Hello,"),
        "missing first line's text:\n{html}"
    );
    assert!(
        html.contains("world!"),
        "missing second line's text:\n{html}"
    );
    assert!(
        html.contains("Second paragraph."),
        "missing the post-Skip paragraph's text:\n{html}"
    );
    // The Skip's length rides as the SECOND paragraph's margin-top, not the
    // first's — a paragraph boundary belongs to what follows it.
    assert!(
        html.contains("margin-top:12pt"),
        "Skip length did not become a margin-top:\n{html}"
    );
}

#[test]
fn frame_start_end_becomes_a_nested_div() {
    let vboxes = vec![
        VertBox::FrameStart(DecoId(0)),
        text_line("inside the frame"),
        VertBox::FrameEnd(DecoId(0)),
    ];
    let html = render(&vboxes);

    assert!(
        html.contains("<div class=\"frame\""),
        "missing frame div:\n{html}"
    );
    assert!(
        html.contains("inside the frame"),
        "missing frame content:\n{html}"
    );
    // The frame's own paragraph must close BEFORE the frame div closes, so
    // nesting round-trips: <div class="frame">...<p ...>...</p>...</div>.
    let frame_open = html.find("<div class=\"frame\"").unwrap();
    let para_open = html.find("<p class=\"para\"").unwrap();
    let para_close = html[para_open..].find("</p>").unwrap() + para_open;
    // At least one "</div>" must appear after the paragraph closes (the
    // frame's own close, possibly followed by the outer `.doc` wrapper's).
    assert!(
        html[para_close..].contains("</div>"),
        "frame div never closes after its paragraph:\n{html}"
    );
    assert!(
        frame_open < para_open,
        "frame should open before its content:\n{html}"
    );
}

#[test]
fn embedded_block_becomes_a_nested_div_recursively() {
    let inner_vboxes = vec![text_line("nested content")];
    let embed_box = PureHorzBox::EmbeddedBlock {
        breakable: false,
        width: Length::pt(100.0),
        height: Length::pt(20.0),
        depth: Length::ZERO,
        block: inner_vboxes,
        anchor_last: false,
    };
    let vboxes = vec![line(embed_box)];
    let html = render(&vboxes);

    assert!(
        html.contains("<div class=\"embed\""),
        "missing embed div:\n{html}"
    );
    assert!(
        html.contains("nested content"),
        "missing nested block's text:\n{html}"
    );
}

#[test]
fn styled_run_carries_color_and_rising_as_css_not_position() {
    let mut bx = text_run("styled");
    if let PureHorzBox::InnerString { info, .. } = &mut bx {
        info.color = Color::Rgb(1.0, 0.0, 0.0);
        info.rising = Length::pt(3.0);
        info.size = Length::pt(18.0);
    }
    // A second, longer run at the default 12pt, so THAT is the document's
    // body style and the 18pt one is genuinely an exception — otherwise the
    // styled run would itself be the dominant style and correctly carry no
    // size of its own (see `body_styled_runs_need_no_span`).
    let vboxes = vec![line(bx), text_line("plain body text, and more of it")];
    let html = render(&vboxes);

    assert!(
        html.contains("color:rgb(255,0,0)"),
        "missing color CSS:\n{html}"
    );
    assert!(
        html.contains("vertical-align:3pt"),
        "missing rising-as-vertical-align CSS:\n{html}"
    );
    // Sizes are RATIOS of the body size (18/12), not absolute points, so the
    // whole document rescales from the single value on `body`.
    assert!(
        html.contains("font-size:1.5000em"),
        "missing the em-ratio font-size:\n{html}"
    );
    assert!(
        html.contains("font-size: 12pt"),
        "the body rule should carry the document's dominant size:\n{html}"
    );
}

/// The document's dominant `(font, size)` goes on `body`, so the runs that
/// use it — the overwhelming majority in any real document — are written as
/// bare text with no element at all. Before this, the `enumitem` manual
/// emitted 13 592 `<span class="run">`s; it now emits a few hundred.
#[test]
fn body_styled_runs_need_no_span() {
    let vboxes = vec![text_line("ordinary prose")];
    let html = render(&vboxes);
    let body = html.split("<body>").nth(1).expect("a body");
    assert!(
        body.contains("<p class=\"para\">ordinary prose</p>"),
        "body-styled text should be bare, unwrapped text:\n{body}"
    );
    assert!(
        !body.contains("class=\"run\""),
        "no run span should be emitted for body-styled text:\n{body}"
    );
}

#[test]
fn run_text_is_html_escaped() {
    let vboxes = vec![text_line("<a & \"b\">")];
    let html = render(&vboxes);
    assert!(
        html.contains("&lt;a &amp; &quot;b&quot;&gt;"),
        "text was not HTML-escaped:\n{html}"
    );
    assert!(
        !html.contains("<a & \"b\">"),
        "raw unescaped text leaked:\n{html}"
    );
}

/// Slice 2 ("Math"): a `Math` box (glyphs only, no `rules`) must render as
/// a real inline `<svg>` — not the Slice 1 `math-placeholder` `<span>` —
/// with the glyph's literal text inside an SVG `<text>` element.
#[test]
fn math_renders_as_an_inline_svg_with_glyph_text() {
    let math_box = PureHorzBox::Math {
        width: Length::pt(10.0),
        height: Length::pt(8.0),
        depth: Length::ZERO,
        glyphs: vec![MathGlyph {
            text: "x".to_string(),
            gid: None,
            dx: Length::ZERO,
            dy: Length::ZERO,
            info: HorzStringInfo {
                font: FontKey(0),
                size: Length::pt(12.0),
                rising: Length::ZERO,
                color: Color::Gray(0.0),
            },
            width: Length::pt(10.0),
            height: Length::pt(8.0),
            depth: Length::ZERO,
        }],
        rules: vec![],
    };
    let vboxes = vec![line(math_box)];
    let html = render(&vboxes);
    assert!(
        !html.contains("class=\"math-placeholder\""),
        "S1 placeholder leaked:\n{html}"
    );
    assert!(
        html.contains("<svg"),
        "missing inline <svg> for math:\n{html}"
    );
    assert!(
        html.contains("<text"),
        "missing SVG <text> glyph element:\n{html}"
    );
    assert!(
        html.contains('x'),
        "missing the glyph's literal text:\n{html}"
    );
}

/// A `Math` box's `rules` (fraction bar / radical) must ALSO render — via
/// the same `svg::emit_graphics` path `Graphics` boxes use — as an SVG
/// `<path>` inside the math `<svg>`.
#[test]
fn math_rules_render_as_svg_paths() {
    let rule_path = Path {
        subpaths: vec![Subpath {
            start: (Length::pt(0.0), Length::pt(0.0)),
            segs: vec![PathSeg::Line((Length::pt(10.0), Length::pt(0.0)))],
            closing: Closing::Open,
        }],
    };
    let math_box = PureHorzBox::Math {
        width: Length::pt(10.0),
        height: Length::pt(8.0),
        depth: Length::pt(2.0),
        glyphs: vec![],
        rules: vec![GraphicsElem::Fill(Color::Gray(0.0), rule_path)],
    };
    let vboxes = vec![line(math_box)];
    let html = render(&vboxes);
    assert!(
        html.contains("<svg"),
        "missing inline <svg> for math rules:\n{html}"
    );
    assert!(
        html.contains("<path"),
        "missing SVG <path> for the fraction-bar rule:\n{html}"
    );
}

/// Slice 2 (§4 "Graphics — inline SVG, reuse `svg::emit_graphics`
/// verbatim"): a `Graphics` box renders as a real inline `<svg>` containing
/// the path, not the Slice 1 `gfx-placeholder` `<span>`.
#[test]
fn graphics_renders_as_an_inline_svg() {
    let path = Path {
        subpaths: vec![Subpath {
            start: (Length::pt(0.0), Length::pt(0.0)),
            segs: vec![
                PathSeg::Line((Length::pt(10.0), Length::pt(0.0))),
                PathSeg::Line((Length::pt(10.0), Length::pt(10.0))),
            ],
            closing: Closing::Line,
        }],
    };
    let gfx_box = PureHorzBox::Graphics {
        origin_independent: false,
        width: Length::pt(10.0),
        height: Length::pt(10.0),
        depth: Length::ZERO,
        elems: vec![GraphicsElem::Fill(Color::Rgb(1.0, 0.0, 0.0), path)],
    };
    let vboxes = vec![line(gfx_box)];
    let html = render(&vboxes);
    assert!(
        !html.contains("class=\"gfx-placeholder\""),
        "S1 placeholder leaked:\n{html}"
    );
    assert!(
        html.contains("<svg"),
        "missing inline <svg> for graphics:\n{html}"
    );
    assert!(
        html.contains("<path"),
        "missing SVG <path> for the fill:\n{html}"
    );
    assert!(
        html.contains("rgb(255,0,0)"),
        "missing the fill color:\n{html}"
    );
}

/// Slice 2 (§4 "Links/metadata"): a `PureHorzBox::Frame` whose `DecoId`
/// matches an observed `register-link-to-uri` call (`DocumentValue::
/// reflow_links`, here passed straight in as the test's own side-channel)
/// renders as a real `<a href="…">`, not the Slice 1 `<span class="iframe">`.
#[test]
fn href_frame_renders_as_an_anchor_link() {
    let link_frame = PureHorzBox::Frame {
        width: Length::pt(40.0),
        height: Length::pt(9.0),
        depth: Length::pt(2.0),
        deco: DecoId(7),
        contents: vec![(Length::ZERO, text_run("click me"))],
    };
    let vboxes = vec![line(link_frame)];
    let links = vec![(
        DecoId(7),
        AnnotAction::Uri("https://example.com".to_string()),
    )];
    let html = render_with_links(&vboxes, &links, &[]);

    assert!(
        html.contains("<a class=\"link\" href=\"https://example.com\">"),
        "missing <a href> for the link frame:\n{html}"
    );
    assert!(
        html.contains("click me"),
        "missing the link's inline text:\n{html}"
    );
    assert!(html.contains("</a>"), "missing the closing </a>:\n{html}");
}

/// A `GotoName` (in-document) link becomes an `<a href="#name">`; the
/// matching `register-destination` (fired from a BLOCK frame with the SAME
/// `DecoId`) becomes that frame div's `id="name"` — so the two actually
/// wire up to a real, clickable in-page jump.
#[test]
fn goto_name_link_and_matching_destination_frame_wire_together() {
    let link_frame = PureHorzBox::Frame {
        width: Length::pt(40.0),
        height: Length::pt(9.0),
        depth: Length::pt(2.0),
        deco: DecoId(1),
        contents: vec![(Length::ZERO, text_run("see below"))],
    };
    let vboxes = vec![
        line(link_frame),
        VertBox::FrameStart(DecoId(2)),
        text_line("the target section"),
        VertBox::FrameEnd(DecoId(2)),
    ];
    let links = vec![(DecoId(1), AnnotAction::GotoName("sec1".to_string()))];
    let dests = vec![(DecoId(2), "sec1".to_string())];
    let html = render_with_links(&vboxes, &links, &dests);

    assert!(
        html.contains("<a class=\"link\" href=\"#sec1\">"),
        "missing the GotoName <a href=\"#…\">:\n{html}"
    );
    assert!(
        html.contains("id=\"sec1\""),
        "missing the destination frame's id=\"…\" anchor:\n{html}"
    );
}

/// The defining difference from the faithful (`render_html_fixed`) mode: NOTHING
/// in reflow output is absolutely positioned. `left:` is never emitted at
/// all; every occurrence of the substring `top:` must be part of
/// `margin-top:` (a legitimate flow property), never a bare CSS `top`
/// positioning declaration.
#[test]
fn reflow_output_never_uses_absolute_positioning() {
    let vboxes = vec![
        text_line("first"),
        VertBox::Skip(Length::pt(6.0)),
        VertBox::FrameStart(DecoId(1)),
        text_line("second"),
        VertBox::FrameEnd(DecoId(1)),
    ];
    let html = render(&vboxes);

    // The CONTENT never positions anything. This is the invariant that
    // matters: no run, paragraph, frame or table is placed at a coordinate.
    let body = body_of(&html);
    assert!(
        !body.contains("position:absolute") && !body.contains("position: absolute"),
        "reflow content must never use position:absolute:\n{body}"
    );
    // The STYLESHEET has exactly one absolute rule, and it is a DRAWING
    // layer, not page positioning: a framed block's decoration is stretched
    // over its own relatively-positioned box (`css.rs`'s `svg.frame-deco`,
    // the same licence the inline `svg`/math wrappers already have — see
    // this module's doc comment). Pinned by count so a second one cannot
    // arrive unnoticed.
    let sheet = html.split("<style>").nth(1).expect("a stylesheet");
    let sheet = sheet.split("</style>").next().unwrap();
    assert_eq!(
        sheet.matches("position: absolute").count()
            + sheet.matches("position:absolute").count(),
        1,
        "unexpected absolute positioning in the stylesheet:\n{sheet}"
    );
    assert!(
        sheet.contains("svg.frame-deco"),
        "the one absolute rule should be the decoration layer:\n{sheet}"
    );
    // `top:`/`left:` are allowed only as the tail of a flow-safe longhand
    // (`margin-top`, `border-top`, `padding-left`, … — used by the static
    // `.clearpage`/`aside.footnote`/`nav.toc` stylesheet rules, `css.rs`),
    // never as the bare positioned `top`/`left` property — except inside
    // that one decoration rule.
    for prop in ["top:", "left:"] {
        for (idx, _) in body.match_indices(prop) {
            let before = &body[..idx];
            assert!(
                ["margin-", "border-", "padding-", "-"]
                    .iter()
                    .any(|p| before.ends_with(p)),
                "found a bare `{prop}` CSS declaration at byte {idx}:\n{body}"
            );
        }
    }
}

#[test]
fn missing_reflow_source_renders_a_placeholder_instead_of_panicking() {
    let html =
        rustyfi_html::render_html_reflow(None, &geometry(), &[], &DocExtras::default(), &[], &[])
            .expect("must not panic/error when reflow_source is None");
    assert!(
        html.starts_with("<!doctype html>"),
        "missing doctype:\n{html}"
    );
    assert!(
        html.contains("reflow-empty"),
        "missing the no-source placeholder:\n{html}"
    );
}

// ---------------------------------------------------------------------
// Slice 3 ("S3"): outline-driven headings + navigable TOC, and real
// `<table>` from `Tabular`.
// ---------------------------------------------------------------------

/// A `Frame` whose `DecoId` resolves (via `dests`) to a destination name
/// that also appears in `extras.outline` must be promoted from a plain
/// `<p class="para">` to `<h{level+1}>` — the structural `dest_name` match
/// `structure::find_heading_level` implements (§2 "the one real lever").
#[test]
fn outline_matched_frame_promotes_its_paragraph_to_a_heading() {
    let heading_frame = PureHorzBox::Frame {
        width: Length::pt(80.0),
        height: Length::pt(9.0),
        depth: Length::pt(2.0),
        deco: DecoId(9),
        contents: vec![(Length::ZERO, text_run("Introduction"))],
    };
    let vboxes = vec![line(heading_frame)];
    let extras = DocExtras {
        outline: vec![OutlineEntry {
            level: 0,
            text: "Introduction".to_string(),
            dest_name: "sec1".to_string(),
            is_open: false,
        }],
        ..DocExtras::default()
    };
    let dests = vec![(DecoId(9), "sec1".to_string())];
    let html = render_with_extras(&vboxes, &extras, &[], &dests);

    assert!(
        html.contains("<h1 class=\"heading\" data-outline-level=\"0\""),
        "missing promoted <h1>:\n{html}"
    );
    assert!(
        !html.contains("<p class=\"para\">"),
        "the heading's own text must not ALSO render as a plain <p>:\n{html}"
    );
    assert!(
        html.contains("Introduction"),
        "missing heading text:\n{html}"
    );
    assert!(html.contains("</h1>"), "missing closing </h1>:\n{html}");
    // The destination id= anchor (S2's Frame/dests wiring) must still be
    // present INSIDE the promoted heading, unaffected by S3's tag swap.
    assert!(
        html.contains("id=\"sec1\""),
        "missing id anchor inside heading:\n{html}"
    );
}

/// A `register-outline` entry at level 1 (`+subsection`'s convention)
/// becomes `<h2>`; deeper levels clamp at `<h6>` rather than emitting an
/// invalid `<h7>`.
#[test]
fn outline_level_maps_to_the_matching_heading_tag_and_clamps_at_h6() {
    for (level, expected_tag) in [(0i64, "h1"), (1, "h2"), (5, "h6"), (9, "h6")] {
        let frame = PureHorzBox::Frame {
            width: Length::pt(40.0),
            height: Length::pt(9.0),
            depth: Length::pt(2.0),
            deco: DecoId(3),
            contents: vec![(Length::ZERO, text_run("T"))],
        };
        let vboxes = vec![line(frame)];
        let extras = DocExtras {
            outline: vec![OutlineEntry {
                level,
                text: "T".to_string(),
                dest_name: "d".to_string(),
                is_open: false,
            }],
            ..DocExtras::default()
        };
        let dests = vec![(DecoId(3), "d".to_string())];
        let html = render_with_extras(&vboxes, &extras, &[], &dests);
        assert!(
            html.contains(&format!("<{expected_tag} class=\"heading\"")),
            "level {level} should map to <{expected_tag}>:\n{html}"
        );
    }
}

/// An empty `extras.outline` (the common case: a doc class that never calls
/// `register-outline`) must emit no `<nav>` at all, and paragraphs stay
/// plain `<p>` — S3 is purely additive when its one lever is unused.
#[test]
fn no_outline_means_no_nav_and_no_heading_promotion() {
    let html = render(&[text_line("just a paragraph")]);
    assert!(
        !html.contains("<nav"),
        "unexpected <nav> with no outline:\n{html}"
    );
    assert!(
        !html.contains("<h1"),
        "unexpected heading with no outline:\n{html}"
    );
    assert!(
        html.contains("<p class=\"para\">"),
        "missing plain <p>:\n{html}"
    );
}

/// `extras.outline` drives heading promotion and the `id=` anchors, but must
/// NOT generate a table of contents of its own: a document that wants one
/// typesets it (`stdjabook`'s `\table-of-contents`), and the generated copy
/// duplicated it above the title in every real manual.
#[test]
fn an_outline_generates_no_table_of_contents() {
    let extras = DocExtras {
        outline: vec![OutlineEntry {
            level: 0,
            text: "Chapter One".to_string(),
            dest_name: "ch1".to_string(),
            is_open: false,
        }],
        ..DocExtras::default()
    };
    let html = render_with_extras(&[text_line("body")], &extras, &[], &[]);

    assert!(!html.contains("<nav"), "generated a TOC nav:\n{html}");
    assert!(
        !html.contains("Chapter One"),
        "the outline's own text must not be emitted as content:\n{html}"
    );
    assert!(html.contains("body"), "lost the document:\n{html}");
}

/// `PureHorzBox::Tabular` (design doc §3's `Tabular` row: "genuinely
/// recoverable") must render a real `<table>`/`<tr>`/`<td>` grid, not the
/// Slice 1/2 `table-placeholder` `<span>` — row grouping recovered from
/// `TabularCellBox::x` per `structure::render_table`'s doc comment.
#[test]
fn tabular_renders_as_a_real_table_with_rows_and_cells() {
    let cell = |x: f64, text: &str| TabularCellBox {
        x: Length::pt(x),
        baseline_y: Length::ZERO,
        contents: vec![(Length::ZERO, text_run(text))],
    };
    let tab = TabularBox {
        width: Length::pt(40.0),
        height: Length::pt(20.0),
        depth: Length::ZERO,
        cells: vec![
            // Row 0: x strictly increasing (0, 20).
            cell(0.0, "R0C0"),
            cell(20.0, "R0C1"),
            // Row 1: x resets to 0 — a new row begins.
            cell(0.0, "R1C0"),
            cell(20.0, "R1C1"),
        ],
        rules: vec![],
    };
    let vboxes = vec![line(PureHorzBox::Tabular(tab))];
    let html = render(&vboxes);

    assert!(
        !html.contains("class=\"table-placeholder\""),
        "S1/S2 placeholder leaked:\n{html}"
    );
    assert!(
        html.contains("<table class=\"tabular\">"),
        "missing <table>:\n{html}"
    );
    assert_eq!(
        html.matches("<tr>").count(),
        2,
        "expected two rows:\n{html}"
    );
    assert_eq!(
        html.matches("<td>").count(),
        4,
        "expected four cells:\n{html}"
    );
    for text in ["R0C0", "R0C1", "R1C0", "R1C1"] {
        assert!(html.contains(text), "missing cell text {text}:\n{html}");
    }
}

// ============================================================================
// S4: semantic lists + emphasis.
// ============================================================================

/// `VertBox::ListMark(ListStart{ordered:false})`/`ItemStart`/`ItemEnd`/
/// `ListEnd` (design doc §6.3's `list_marks_become_a_ul_with_li`): a flat
/// one-item list becomes exactly one `<ul>` with exactly one `<li>`.
#[test]
fn list_marks_become_a_ul_with_li() {
    let vboxes = vec![
        VertBox::ListMark(ListMarkKind::ListStart { ordered: false }),
        VertBox::ListMark(ListMarkKind::ItemStart),
        text_line("one item"),
        VertBox::ListMark(ListMarkKind::ItemEnd),
        VertBox::ListMark(ListMarkKind::ListEnd),
    ];
    let html = render(&vboxes);

    assert_eq!(
        html.matches("<ul").count(),
        1,
        "expected exactly one <ul>:\n{html}"
    );
    assert_eq!(
        html.matches("</ul>").count(),
        1,
        "expected exactly one </ul>:\n{html}"
    );
    assert_eq!(
        html.matches("<li").count(),
        1,
        "expected exactly one <li>:\n{html}"
    );
    assert_eq!(
        html.matches("</li>").count(),
        1,
        "expected exactly one </li>:\n{html}"
    );
    assert!(
        !html.contains("<ol"),
        "unordered list must not render <ol>:\n{html}"
    );
    assert!(html.contains("one item"), "missing item text:\n{html}");
}

/// Design doc §6.3's `nested_list_marks_nest_li`: a `ListStart` reached
/// while an `<li>` is still open must produce a GENUINELY nested `<ul>`
/// inside that `<li>` (not two sibling top-level lists) — no depth payload
/// is carried by the markers (§4.1), so this is purely `block.rs`'s stack
/// discipline.
#[test]
fn nested_list_marks_nest_li() {
    let vboxes = vec![
        VertBox::ListMark(ListMarkKind::ListStart { ordered: false }),
        VertBox::ListMark(ListMarkKind::ItemStart),
        text_line("parent"),
        VertBox::ListMark(ListMarkKind::ListStart { ordered: false }),
        VertBox::ListMark(ListMarkKind::ItemStart),
        text_line("child"),
        VertBox::ListMark(ListMarkKind::ItemEnd),
        VertBox::ListMark(ListMarkKind::ListEnd),
        VertBox::ListMark(ListMarkKind::ItemEnd),
        VertBox::ListMark(ListMarkKind::ListEnd),
    ];
    let html = render(&vboxes);

    assert_eq!(
        html.matches("<ul").count(),
        2,
        "expected two <ul>s (outer + nested):\n{html}"
    );
    assert_eq!(
        html.matches("</ul>").count(),
        2,
        "expected two </ul>s:\n{html}"
    );
    assert_eq!(
        html.matches("<li").count(),
        2,
        "expected two <li>s:\n{html}"
    );
    assert!(html.contains("parent"), "missing parent item text:\n{html}");
    assert!(html.contains("child"), "missing child item text:\n{html}");

    // Structural nesting check: the nested `<ul>` must open BEFORE the
    // outer/first `<li>`'s `</li>` closes — i.e. it sits INSIDE that `<li>`,
    // not as a sibling after it.
    let first_li = html.find("<li").expect("missing first <li>");
    let first_li_close = html[first_li..].find("</li>").expect("missing first </li>") + first_li;
    let parent_text = html.find("parent").expect("missing parent text");
    let nested_ul = html[parent_text..]
        .find("<ul")
        .expect("missing nested <ul>")
        + parent_text;
    assert!(
        nested_ul < first_li_close,
        "nested <ul> must open before the parent's </li> closes (real nesting, not flattening):\n{html}"
    );
}

/// `ListStart{ordered:true}` (`\enumerate`'s marker, design doc §5's exact
/// "ordered-vs-unordered is not a heuristic" bit): renders `<ol>`, never
/// `<ul>`.
#[test]
fn enumerate_marks_become_ol() {
    let vboxes = vec![
        VertBox::ListMark(ListMarkKind::ListStart { ordered: true }),
        VertBox::ListMark(ListMarkKind::ItemStart),
        text_line("first"),
        VertBox::ListMark(ListMarkKind::ItemEnd),
        VertBox::ListMark(ListMarkKind::ItemStart),
        text_line("second"),
        VertBox::ListMark(ListMarkKind::ItemEnd),
        VertBox::ListMark(ListMarkKind::ListEnd),
    ];
    let html = render(&vboxes);

    assert_eq!(
        html.matches("<ol").count(),
        1,
        "expected exactly one <ol>:\n{html}"
    );
    assert_eq!(
        html.matches("</ol>").count(),
        1,
        "expected exactly one </ol>:\n{html}"
    );
    assert_eq!(
        html.matches("<li").count(),
        2,
        "expected two <li>s:\n{html}"
    );
    assert!(
        !html.contains("<ul"),
        "ordered list must not render <ul>:\n{html}"
    );
    assert!(
        html.contains("first") && html.contains("second"),
        "missing item text:\n{html}"
    );
}

/// Design doc §6.3's `bullet_fence_is_suppressed` / R2's mitigation: content
/// between `InlineMark(BulletStart)`/`InlineMark(BulletEnd)` renders
/// NOTHING (the real `<ul>`/`<ol>` marker replaces it) — content outside
/// the fence, on the SAME line, still renders normally.
#[test]
fn bullet_fence_is_suppressed() {
    let vboxes = vec![VertBox::Line {
        height: Length::pt(9.0),
        depth: Length::pt(2.0),
        leading: Length::pt(12.0),
        contents: vec![
            (
                Length::ZERO,
                PureHorzBox::InlineMark(InlineMarkKind::BulletStart),
            ),
            (Length::ZERO, text_run("BULLETGLYPH")),
            (
                Length::ZERO,
                PureHorzBox::InlineMark(InlineMarkKind::BulletEnd),
            ),
            (Length::ZERO, text_run("realtext")),
        ],
    }];
    let html = render(&vboxes);

    assert!(
        !html.contains("BULLETGLYPH"),
        "fenced bullet glyph run must be dropped entirely:\n{html}"
    );
    assert!(
        html.contains("realtext"),
        "unfenced content after the bullet fence must still render:\n{html}"
    );
}

/// `InlineMark(EmphStart{strong:false})`/`EmphEnd` (`\emph`'s marker):
/// wraps its content in a real `<em>`, never `<strong>`.
#[test]
fn emph_marks_wrap_em() {
    let vboxes = vec![VertBox::Line {
        height: Length::pt(9.0),
        depth: Length::pt(2.0),
        leading: Length::pt(12.0),
        contents: vec![
            (
                Length::ZERO,
                PureHorzBox::InlineMark(InlineMarkKind::EmphStart { strong: false }),
            ),
            (Length::ZERO, text_run("emphasized")),
            (
                Length::ZERO,
                PureHorzBox::InlineMark(InlineMarkKind::EmphEnd),
            ),
        ],
    }];
    let html = render(&vboxes);

    assert!(html.contains("<em>"), "missing <em>:\n{html}");
    assert!(html.contains("</em>"), "missing </em>:\n{html}");
    assert!(
        !html.contains("<strong>"),
        "\\emph must not render <strong>:\n{html}"
    );
    let open = html.find("<em>").unwrap();
    let close = html.find("</em>").unwrap();
    let text = html.find("emphasized").unwrap();
    assert!(
        open < text && text < close,
        "emphasized text must sit between <em> and </em>:\n{html}"
    );
}

/// `InlineMark(EmphStart{strong:true})`/`EmphEnd` (`\bold`'s marker): wraps
/// its content in a real `<strong>`, never `<em>`.
#[test]
fn bold_marks_wrap_strong() {
    let vboxes = vec![VertBox::Line {
        height: Length::pt(9.0),
        depth: Length::pt(2.0),
        leading: Length::pt(12.0),
        contents: vec![
            (
                Length::ZERO,
                PureHorzBox::InlineMark(InlineMarkKind::EmphStart { strong: true }),
            ),
            (Length::ZERO, text_run("bolded")),
            (
                Length::ZERO,
                PureHorzBox::InlineMark(InlineMarkKind::EmphEnd),
            ),
        ],
    }];
    let html = render(&vboxes);

    assert!(html.contains("<strong>"), "missing <strong>:\n{html}");
    assert!(html.contains("</strong>"), "missing </strong>:\n{html}");
    assert!(
        !html.contains("<em>"),
        "\\bold must not render <em>:\n{html}"
    );
}

fn iframe_marker(id: usize, end: bool) -> PureHorzBox {
    PureHorzBox::InlineFrameMarker {
        id: DecoId(id),
        end,
        height: Length::pt(9.0),
        depth: Length::pt(2.0),
    }
}

fn line_of(boxes: Vec<PureHorzBox>) -> VertBox {
    VertBox::Line {
        height: Length::pt(9.0),
        depth: Length::pt(2.0),
        leading: Length::pt(12.0),
        contents: boxes.into_iter().map(|b| (Length::ZERO, b)).collect(),
    }
}

/// `\href` does NOT build a `PureHorzBox::Frame`: `inline-frame-breakable`
/// splices its contents between an `InlineFrameMarker` PAIR so the frame can
/// split across lines. The link lookup therefore has to key on the MARKER's
/// `DecoId`. Every other link test in this file uses a `Frame`, which is the
/// shape real documents stopped producing — so without this test the suite
/// stays green while every actual `\href` renders as bare text.
#[test]
fn href_through_a_breakable_inline_frame_renders_as_an_anchor_link() {
    let vboxes = vec![line_of(vec![
        iframe_marker(11, false),
        text_run("click me"),
        iframe_marker(11, true),
    ])];
    let links = vec![(
        DecoId(11),
        AnnotAction::Uri("https://example.com".to_string()),
    )];
    let html = render_with_links(&vboxes, &links, &[]);

    let open = html
        .find("<a class=\"link\" href=\"https://example.com\">")
        .unwrap_or_else(|| panic!("missing <a href> for the marker pair:\n{html}"));
    let text = html
        .find("click me")
        .unwrap_or_else(|| panic!("missing the link text:\n{html}"));
    let close = html
        .find("</a>")
        .unwrap_or_else(|| panic!("missing the closing </a>:\n{html}"));
    assert!(
        open < text && text < close,
        "the anchor must WRAP the text, not merely appear near it:\n{html}"
    );
}

/// The end marker carries no payload saying which tag it closes, so the
/// walker keeps a stack. Nested pairs must therefore close innermost-first:
/// a plain frame inside a link frame closes its `</span>` before the `</a>`.
/// Getting this wrong produces overlapping tags that browsers silently
/// reinterpret, so it would not show up as a crash.
#[test]
fn nested_breakable_inline_frames_close_innermost_first() {
    let vboxes = vec![line_of(vec![
        iframe_marker(21, false),
        text_run("outer "),
        iframe_marker(22, false),
        text_run("inner"),
        iframe_marker(22, true),
        iframe_marker(21, true),
    ])];
    let links = vec![(
        DecoId(21),
        AnnotAction::Uri("https://example.org".to_string()),
    )];
    let html = render_with_links(&vboxes, &links, &[]);

    let span_close = html
        .find("</span>")
        .expect("inner frame must close a <span>");
    let a_close = html.find("</a>").expect("outer link must close an </a>");
    assert!(
        span_close < a_close,
        "inner </span> must close before the outer </a>:\n{html}"
    );
}

// ============================================================================
// Text is TEXT: what a glue box becomes.
//
// The box stream is not a word stream — `convertText.ml`'s port splits a run
// at every UAX#14 chunk boundary and puts glue between the pieces — so "glue
// means U+0020", which is what this backend used to do, rendered Japanese as
// one `<span>` per character with a space between each, and the `\LaTeX`
// logo's negative kerns as `L AT EX`. See `reflow/text.rs`.
// ============================================================================

/// A `size`-pt run of `text`, for the glue tests.
fn sized_run(text: &str, size: f64) -> PureHorzBox {
    PureHorzBox::InnerString {
        info: HorzStringInfo {
            font: FontKey(0),
            size: Length::pt(size),
            rising: Length::ZERO,
            color: Color::Gray(0.0),
        },
        text: text.to_string(),
        width: Length::pt(8.0),
        height: Length::pt(9.0),
        depth: Length::pt(2.0),
    }
}

fn glue(natural: f64) -> PureHorzBox {
    PureHorzBox::OuterEmpty {
        natural: Length::pt(natural),
        shrinkable: Length::pt(1.0),
        stretchable: Length::pt(1.0),
    }
}

fn body_of(html: &str) -> &str {
    html.split("<body>").nth(1).expect("a <body>")
}

/// The whole point of `text.rs`. Between two CJK characters the port emits
/// `adjacent_space` glue — natural 0, stretch only — at EVERY character
/// boundary, plus a JLreq class space (nonzero) after punctuation. Neither
/// may become a space: `研 究 計 画` is not Japanese, is not selectable as a
/// word, and gives the browser nothing to line-break.
#[test]
fn cjk_characters_join_into_one_run_with_no_spaces_between() {
    let vboxes = vec![line_of(vec![
        sized_run("研", 12.0),
        glue(0.0),
        sized_run("究", 12.0),
        glue(0.0),
        sized_run("計", 12.0),
        glue(5.28),
        sized_run("画", 12.0),
    ])];
    let html = render(&vboxes);
    let body = body_of(&html);
    assert!(
        body.contains("研究計画"),
        "CJK characters must join into one uninterrupted string:\n{body}"
    );
    assert!(
        !body.contains("研 究"),
        "no space may be inserted between two CJK characters:\n{body}"
    );
}

/// A zero-width glue inside a Latin word (`Con|trib|utors`) is a break
/// OPPORTUNITY, not spacing — and the pieces must rejoin into one word, or
/// the text is neither readable nor searchable.
#[test]
fn zero_width_glue_inside_a_word_rejoins_it() {
    let vboxes = vec![line_of(vec![
        sized_run("Con", 12.0),
        glue(0.0),
        sized_run("trib", 12.0),
        glue(0.0),
        sized_run("utors", 12.0),
    ])];
    let html = render(&vboxes);
    assert!(
        body_of(&html).contains("Contributors"),
        "chunks of one word must rejoin:\n{html}"
    );
}

/// A real inter-word glue still becomes a space, and so does a
/// Japanese/Latin boundary, where the port emits inter-SCRIPT glue — a
/// space is what HTML wants there.
#[test]
fn word_spaces_and_script_boundaries_still_take_a_space() {
    let vboxes = vec![line_of(vec![
        sized_run("hello", 12.0),
        glue(3.5),
        sized_run("world", 12.0),
        glue(2.6),
        sized_run("を", 12.0),
    ])];
    let html = render(&vboxes);
    let body = body_of(&html);
    assert!(body.contains("hello world"), "{body}");
    assert!(body.contains("world を"), "{body}");
}

/// A negative `inline-skip` is a kern (the `\LaTeX` logo has four), never a
/// space and never a strut.
#[test]
fn a_negative_inline_skip_is_a_kern_not_a_space() {
    let vboxes = vec![line_of(vec![
        sized_run("L", 12.0),
        PureHorzBox::FixedEmpty {
            width: Length::pt(-4.0),
        },
        sized_run("A", 12.0),
    ])];
    let html = render(&vboxes);
    let body = body_of(&html);
    assert!(
        body.contains("LA"),
        "kerned letters must not split:\n{body}"
    );
    assert!(!body.contains("L A"), "{body}");
    assert!(
        !body.contains("hskip"),
        "a negative skip is not a strut:\n{body}"
    );
}

/// A deliberate, visible `inline-skip` (a paragraph indent, a cell pad)
/// keeps its width as an inline-block strut — intrinsic sizing, not
/// positioning.
#[test]
fn a_wide_inline_skip_survives_as_a_sized_strut() {
    let vboxes = vec![line_of(vec![
        PureHorzBox::FixedEmpty {
            width: Length::pt(10.5),
        },
        sized_run("indented", 12.0),
    ])];
    let html = render(&vboxes);
    assert!(
        html.contains("<span class=\"hskip\" style=\"width:10.5pt;\">"),
        "{html}"
    );
}

/// A `Discretionary` carrying a hyphen in `pre_break` is a real dictionary
/// hyphenation point and becomes a soft hyphen. One carrying only glue is a
/// UAX#14 chunk boundary and must NOT, or the browser is invited to
/// hyphenate at a point no dictionary sanctioned.
#[test]
fn only_a_hyphen_bearing_discretionary_becomes_a_soft_hyphen() {
    let with_hyphen = PureHorzBox::Discretionary {
        penalty: 0,
        pre_break: vec![sized_run("-", 12.0)],
        post_break: vec![],
        no_break: vec![],
    };
    let bare = PureHorzBox::Discretionary {
        penalty: 0,
        pre_break: vec![glue(0.0)],
        post_break: vec![],
        no_break: vec![],
    };
    let a = render(&[line_of(vec![
        sized_run("La", 12.0),
        with_hyphen,
        sized_run("tex", 12.0),
    ])]);
    assert!(a.contains("&shy;"), "{a}");

    let b = render(&[line_of(vec![
        sized_run("Con", 12.0),
        bare,
        sized_run("trib", 12.0),
    ])]);
    let b = body_of(&b);
    assert!(
        !b.contains("&shy;"),
        "a bare chunk boundary is not a hyphen:\n{b}"
    );
    assert!(b.contains("Contrib"), "{b}");
}

// ============================================================================
// Footnotes, images, alignment, headings, margins.
// ============================================================================

/// A continuous document has no page foot, so a footnote goes where it is
/// referenced: an `<aside>` immediately after the referencing paragraph,
/// carrying the `id` its in-text anchor links back from. Dropping it — the
/// inert `footnote-placeholder` this used to emit — is not an option.
#[test]
fn a_footnote_lands_as_an_aside_right_after_its_paragraph() {
    let note = PureHorzBox::Footnote {
        block: vec![text_line("the note body")],
    };
    let vboxes = vec![
        line_of(vec![text_run("body text"), note]),
        VertBox::Skip(Length::pt(6.0)),
        text_line("the next paragraph"),
    ];
    let html = render(&vboxes);
    let body = body_of(&html);

    let para = body.find("body text").expect("the referencing paragraph");
    let aside = body
        .find("<aside class=\"footnote\" id=\"fn-1\"")
        .unwrap_or_else(|| panic!("missing the footnote aside:\n{body}"));
    let next = body
        .find("the next paragraph")
        .expect("the following paragraph");
    assert!(
        para < aside && aside < next,
        "the aside must sit between its own paragraph and the next:\n{body}"
    );
    assert!(body.contains("the note body"), "{body}");
    // A zero-width anchor, not a second visible marker: the document
    // typesets its own reference mark right beside the box.
    assert!(
        body.contains("<span class=\"fnref\" id=\"fnref-1\"></span>"),
        "{body}"
    );
    assert!(
        body.contains("href=\"#fnref-1\""),
        "missing the back-link:\n{body}"
    );
    assert!(
        !body.contains("footnote-placeholder"),
        "footnotes must not be dropped:\n{body}"
    );
}

fn tiny_image(fill: u8) -> rustyfi_backend::ImageResource {
    rustyfi_backend::ImageResource {
        samples: vec![fill; 3 * 4],
        px_w: 2,
        px_h: 2,
        jpeg_dct: None,
        pdf: None,
    }
}

fn image_line(id: usize) -> VertBox {
    line(PureHorzBox::Image {
        width: Length::pt(40.0),
        height: Length::pt(30.0),
        image: rustyfi_backend::ImageId(id),
    })
}

fn render_with_images(vboxes: &[VertBox], images: &[rustyfi_backend::ImageResource]) -> String {
    rustyfi_html::render_html_reflow(
        Some(vboxes),
        &geometry(),
        images,
        &DocExtras::default(),
        &[],
        &[],
    )
    .expect("reflow HTML rendering must succeed")
}

/// An `Image` becomes a real, self-contained `<img>` data URI — not the
/// inert placeholder this used to emit, which for `figbox`'s manual meant
/// losing all 39 figures.
#[test]
fn an_image_renders_as_a_self_contained_img() {
    let html = render_with_images(&[image_line(0)], &[tiny_image(7)]);
    assert!(
        html.contains("<img class=\"img\" src=\"data:image/"),
        "{html}"
    );
    assert!(html.contains("width:40pt; height:30pt;"), "{html}");
    assert!(!html.contains("image-placeholder"), "{html}");
}

/// The SAME picture placed many times must have its bytes emitted ONCE, as
/// a shared CSS rule. `figbox`'s manual places two figures seventeen times
/// between them, and inlining a data URI per placement made a 13 MB file.
/// Each placement is a DISTINCT `ImageId` — `include-image` mints a fresh
/// resource every call — so the dedup has to key on content, not identity.
#[test]
fn a_repeated_image_is_emitted_once_as_a_shared_rule() {
    let images = vec![tiny_image(9), tiny_image(9)];
    let vboxes = vec![image_line(0), VertBox::Skip(Length::pt(4.0)), image_line(1)];
    let html = render_with_images(&vboxes, &images);
    assert_eq!(
        html.matches("data:image/").count(),
        1,
        "the picture's bytes must appear exactly once:\n{html}"
    );
    assert_eq!(
        html.matches("shared-img-0").count(),
        3,
        "one CSS rule plus one class per placement:\n{html}"
    );
}

/// Two DIFFERENT pictures are not conflated by the content dedup.
#[test]
fn distinct_images_are_not_shared_with_each_other() {
    let images = vec![tiny_image(1), tiny_image(2)];
    let vboxes = vec![image_line(0), VertBox::Skip(Length::pt(4.0)), image_line(1)];
    let html = render_with_images(&vboxes, &images);
    assert_eq!(
        html.matches("data:image/").count(),
        2,
        "each distinct picture keeps its own bytes:\n{html}"
    );
    assert!(!html.contains("shared-img-"), "{html}");
}

/// `inline-fil` is not content, it is the alignment signal — leading AND
/// trailing means centred, leading only means flush right. An ORDINARY
/// paragraph ends with `inline-fil` (that is how the last line fills), so a
/// trailing one alone must mean nothing.
#[test]
fn leading_and_trailing_fil_recover_centring_and_right_alignment() {
    let centred = render(&[line_of(vec![
        PureHorzBox::OuterFil,
        text_run("centred"),
        PureHorzBox::OuterFil,
    ])]);
    assert!(centred.contains("data-align=\"center\""), "{centred}");

    let right = render(&[line_of(vec![
        PureHorzBox::OuterFil,
        text_run("flush right"),
    ])]);
    assert!(right.contains("data-align=\"right\""), "{right}");

    let ordinary = render(&[line_of(vec![text_run("ordinary"), PureHorzBox::OuterFil])]);
    let ordinary = body_of(&ordinary);
    assert!(
        !ordinary.contains("data-align"),
        "a trailing fil alone is just the last line filling:\n{ordinary}"
    );
}

/// Every bundled doc class writes a section title with
/// `inline-frame-breakable` (`stdjabook.satyh:551`), which splices an
/// `InlineFrameMarker` PAIR rather than building a `Frame`. Matching only
/// `Frame`, as the heading promotion did, meant no heading in any
/// `stdjabook`/`stdjareport` document was ever promoted — the `latexcmds`
/// manual's seven `+section`s all came out as `<p>`.
#[test]
fn a_heading_behind_a_breakable_inline_frame_is_promoted() {
    let extras = DocExtras {
        outline: vec![OutlineEntry {
            level: 0,
            text: "1. Introduction".to_string(),
            dest_name: "sec1".to_string(),
            is_open: false,
        }],
        ..DocExtras::default()
    };
    let vboxes = vec![line_of(vec![
        iframe_marker(7, false),
        text_run("1. Introduction"),
        iframe_marker(7, true),
    ])];
    let dests = vec![(DecoId(7), "sec1".to_string())];
    let html = render_with_extras(&vboxes, &extras, &[], &dests);
    assert!(
        html.contains("<h1 class=\"heading\" data-outline-level=\"0\""),
        "the outline-registered title must become a heading:\n{html}"
    );
}

/// Adjacent vertical margins COLLAPSE, they do not add up — SATySFi's own
/// `squash_margins` rule, and CSS's. Summing them put roughly two blank
/// lines between every pair of paragraphs in a real document.
#[test]
fn consecutive_skips_collapse_to_the_largest() {
    let vboxes = vec![
        text_line("first"),
        VertBox::Skip(Length::pt(6.0)),
        VertBox::ParagTop(Length::pt(9.0)),
        VertBox::Skip(Length::pt(4.0)),
        text_line("second"),
    ];
    let html = render(&vboxes);
    assert!(
        html.contains("margin-top:9pt;"),
        "three adjacent skips must collapse to the largest, not sum to 19pt:\n{html}"
    );
}

/// A paragraph whose whole content is whitespace emits nothing and keeps
/// its margin for what follows. The box stream is full of lines holding
/// only the `inline-fil` that `single-centering-line` wraps a table in, and
/// each one used to become an empty `<p>` worth a blank line of leading.
#[test]
fn a_whitespace_only_paragraph_emits_nothing() {
    let vboxes = vec![
        VertBox::Skip(Length::pt(7.0)),
        line(PureHorzBox::OuterFil),
        text_line("real content"),
    ];
    let html = render(&vboxes);
    let body = body_of(&html);
    assert_eq!(
        body.matches("<p class=\"para\"").count(),
        1,
        "only the paragraph with content should be emitted:\n{body}"
    );
    assert!(
        body.contains("margin-top:7pt;"),
        "the dropped paragraph's margin must carry forward:\n{body}"
    );
}

/// A `Tabular` whose every cell is empty renders nothing. `easytable`
/// builds every table TWICE (`table-builder.satyh`'s `build`) — once for
/// the content and once as a PHANTOM grid of empty cells carrying only the
/// rule callbacks — so rendering the carrier literally put an empty
/// bordered grid above every real table in its manual, forty of them.
#[test]
fn an_all_empty_phantom_table_renders_nothing() {
    let pad = || {
        (
            Length::ZERO,
            PureHorzBox::FixedEmpty {
                width: Length::pt(6.0),
            },
        )
    };
    let empty = TabularBox {
        width: Length::pt(60.0),
        height: Length::pt(20.0),
        depth: Length::ZERO,
        cells: vec![
            TabularCellBox {
                x: Length::ZERO,
                baseline_y: Length::ZERO,
                contents: vec![pad(), pad()],
            },
            TabularCellBox {
                x: Length::pt(30.0),
                baseline_y: Length::ZERO,
                contents: vec![pad(), pad()],
            },
        ],
        rules: vec![],
    };
    let html = render(&[line(PureHorzBox::Tabular(empty))]);
    let body = body_of(&html);
    assert!(
        !body.contains("<table"),
        "a phantom rule-carrier table must not be rendered:\n{body}"
    );
}

// ============================================================================
// Well-formedness.
// ============================================================================

/// Every element opened must be closed, in order. Run coalescing leaves a
/// `<span>` open ACROSS boxes on purpose, so a missing `close_run` at any of
/// the boundaries that need one produces silently misnested markup rather
/// than a crash — a browser reinterprets it and the document merely looks
/// wrong somewhere else.
fn assert_balanced_tags(html: &str) {
    const VOID: [&str; 4] = ["img", "br", "hr", "meta"];
    let mut stack: Vec<&str> = Vec::new();
    let mut i = 0;
    while let Some(rel) = html[i..].find('<') {
        let open = i + rel;
        let Some(rel_end) = html[open..].find('>') else {
            break;
        };
        let close = open + rel_end;
        let inner = &html[open + 1..close];
        i = close + 1;
        if inner.starts_with('!') || inner.starts_with('?') || inner.ends_with('/') {
            continue;
        }
        let (is_end, rest) = match inner.strip_prefix('/') {
            Some(r) => (true, r),
            None => (false, inner),
        };
        let name = rest.split([' ', '\n', '\t']).next().unwrap_or("").trim();
        if name.is_empty() || VOID.contains(&name) {
            continue;
        }
        if is_end {
            assert_eq!(
                stack.pop(),
                Some(name),
                "unbalanced </{name}> — open stack {stack:?}"
            );
        } else {
            stack.push(name);
        }
    }
    assert!(stack.is_empty(), "unclosed elements: {stack:?}");
}

/// An inline region may not straddle a block boundary. An
/// `inline-frame-breakable` whose start and end markers land in different
/// paragraphs — which is what `\ref` does whenever a `Skip` falls between
/// them — has to be closed at the flush and re-opened after it. The
/// re-opened copy must NOT repeat the `id`, which belongs to the first
/// fragment alone.
#[test]
fn an_inline_frame_split_by_a_paragraph_break_stays_balanced() {
    let vboxes = vec![
        line_of(vec![iframe_marker(9, false), text_run("before")]),
        VertBox::Skip(Length::pt(6.0)),
        line_of(vec![text_run("after"), iframe_marker(9, true)]),
    ];
    let dests = vec![(DecoId(9), "anchor".to_string())];
    let html = render_with_links(&vboxes, &[], &dests);
    assert_eq!(
        body_of(&html).matches("id=\"anchor\"").count(),
        1,
        "the anchor id must appear exactly once:\n{html}"
    );
    assert_balanced_tags(&html);
}

/// The invariant run coalescing rests on, over a fixture touching every
/// boundary that has to close an open run: a wrapper, a strut, an `<svg>`,
/// a footnote body, a heading and a frame.
#[test]
fn a_document_touching_every_boundary_stays_balanced() {
    let extras = DocExtras {
        outline: vec![OutlineEntry {
            level: 0,
            text: "T".to_string(),
            dest_name: "d".to_string(),
            is_open: false,
        }],
        ..DocExtras::default()
    };
    let vboxes = vec![
        line_of(vec![
            iframe_marker(1, false),
            text_run("title"),
            iframe_marker(1, true),
        ]),
        VertBox::Skip(Length::pt(5.0)),
        line_of(vec![
            PureHorzBox::FixedEmpty {
                width: Length::pt(10.0),
            },
            sized_run("prose", 18.0),
            glue(3.0),
            sized_run("more", 18.0),
            PureHorzBox::Footnote {
                block: vec![text_line("note")],
            },
            PureHorzBox::Graphics {
                width: Length::pt(10.0),
                height: Length::pt(10.0),
                depth: Length::ZERO,
                elems: vec![GraphicsElem::Fill(
                    Color::Gray(0.0),
                    Path {
                        subpaths: vec![Subpath {
                            start: (Length::ZERO, Length::ZERO),
                            segs: vec![PathSeg::Line((Length::pt(5.0), Length::ZERO))],
                            closing: Closing::Line,
                        }],
                    },
                )],
                origin_independent: false,
            },
            text_run("tail"),
        ]),
        VertBox::FrameStart(DecoId(3)),
        text_line("framed"),
        VertBox::FrameEnd(DecoId(3)),
    ];
    let html = render_with_extras(&vboxes, &extras, &[], &[(DecoId(1), "d".to_string())]);
    assert_balanced_tags(&html);
}

/// A nested block — a footnote body, an `embed-block-top` — runs while the
/// ENCLOSING paragraph's inline wrappers are still on the stack, deliberately
/// (they are waiting to be re-opened after the flush). Its own paragraph
/// flushes must not close them a second time inside the nested block.
#[test]
fn a_nested_block_inside_an_open_inline_frame_stays_balanced() {
    let vboxes = vec![line_of(vec![
        iframe_marker(4, false),
        text_run("linked "),
        PureHorzBox::Footnote {
            block: vec![text_line("note inside a link")],
        },
        PureHorzBox::EmbeddedBlock {
            width: Length::pt(50.0),
            height: Length::pt(10.0),
            depth: Length::ZERO,
            block: vec![text_line("embedded inside a link")],
            anchor_last: false,
            breakable: false,
        },
        text_run("tail"),
        iframe_marker(4, true),
    ])];
    let links = vec![(
        DecoId(4),
        AnnotAction::Uri("https://example.net".to_string()),
    )];
    let html = render_with_links(&vboxes, &links, &[]);
    let body = body_of(&html);
    // The link is closed before the nested blocks and re-opened after them,
    // so it appears twice — and, crucially, is not left dangling around
    // them. Without the depth floor the nested blocks' own paragraph
    // flushes emitted a third, spurious `</a>` inside themselves.
    assert_eq!(
        body.matches("<a class=\"link\"").count(),
        2,
        "the link should be closed around the nested blocks and re-opened:\n{body}"
    );
    assert!(
        !body
            .split("<div class=\"embed\">")
            .nth(1)
            .unwrap()
            .starts_with("</a>"),
        "a nested block must not close its enclosing paragraph's wrapper:\n{body}"
    );
    assert_balanced_tags(&html);
}

/// A word the LINE BREAKER split rejoins, and an AUTHORED hyphen does not
/// disappear when the breaker happens to break after it.
///
/// The two are told apart by `InlineMarkKind::BreakHyphen`, which
/// `linebreak::line_content` emits immediately before the `pre_break` slot it
/// splices onto a closed line. Without it the only test available was the
/// shape of the text, and that guess deleted real hyphens: a paragraph
/// wrapping at `code-printer` rendered as `codeprinter`.
#[test]
fn a_breaker_hyphen_is_undone_and_an_authored_one_is_not() {
    // What the breaker actually emits: the marker, then the hyphen.
    let hyphenated = VertBox::Line {
        height: Length::pt(9.0),
        depth: Length::pt(2.0),
        leading: Length::pt(12.0),
        contents: vec![
            (Length::ZERO, sized_run("vestibu", 12.0)),
            (
                Length::pt(40.0),
                PureHorzBox::InlineMark(InlineMarkKind::BreakHyphen),
            ),
            (Length::pt(40.0), sized_run("-", 12.0)),
        ],
    };
    let out = render(&[hyphenated, line(sized_run("lum sed", 12.0))]);
    let body = body_of(&out);
    assert!(body.contains("vestibulum"), "not rejoined:\n{body}");
    assert!(!body.contains("vestibu-"), "the breaker's hyphen survived:\n{body}");

    // No marker: the hyphen is the AUTHOR's. It stays, and the two halves
    // still rejoin without a space.
    let authored = render(&[
        line(sized_run("code-", 12.0)),
        line(sized_run("printer runs", 12.0)),
    ]);
    let authored = body_of(&authored);
    assert!(
        authored.contains("code-printer"),
        "an authored hyphen was mangled:\n{authored}"
    );
    assert!(
        !authored.contains("code- printer") && !authored.contains("codeprinter"),
        "{authored}"
    );

    // A line ending in a hyphen before a CAPITAL is still one word, and the
    // hyphen is still the author's.
    let kept = render(&[
        line(sized_run("well-", 12.0)),
        line(sized_run("Known name", 12.0)),
    ]);
    let kept = body_of(&kept);
    assert!(kept.contains("well-Known"), "{kept}");
}

/// Self-containment: nothing fetched, nothing executed. The reflow backend
/// NAMES fonts rather than embedding them (`fonts::reflow_font_stack`) —
/// that is a size decision, not a self-containment one, since a named
/// family the reader lacks falls back to a generic and is never requested.
#[test]
fn output_fetches_nothing_and_runs_nothing() {
    let html = render(&[text_line("hello")]);
    for forbidden in ["<script", "@import", "http://", "https://", "url(/"] {
        assert!(
            !html.contains(forbidden),
            "output must be self-contained, found {forbidden}:\n{html}"
        );
    }
}

// ============================================================================
// Constructs that reached the browser looking nothing like the PDF: code
// blocks, indentation, and table rules. Each of the four was reported against
// a real document, and each has a single cause named in the test below.
// ============================================================================

/// A straight horizontal or vertical line, for a table rule.
fn rule_line(x0: f64, y0: f64, x1: f64, y1: f64, width: f64) -> GraphicsElem {
    GraphicsElem::Stroke(
        Length::pt(width),
        Color::Gray(0.0),
        Path {
            subpaths: vec![Subpath {
                start: (Length::pt(x0), Length::pt(y0)),
                segs: vec![PathSeg::Line((Length::pt(x1), Length::pt(y1)))],
                closing: Closing::Open,
            }],
        },
    )
}

fn grid_cell(x: f64, y: f64, text: &str) -> TabularCellBox {
    TabularCellBox {
        x: Length::pt(x),
        baseline_y: Length::pt(y),
        contents: vec![(Length::ZERO, text_run(text))],
    }
}

/// A two-row grid with cells at x = 0/20 and baselines DESCENDING (row 0
/// above row 1), matching `Solved::ys`.
fn two_by_two(rules: Vec<GraphicsElem>) -> TabularBox {
    TabularBox {
        width: Length::pt(40.0),
        height: Length::pt(20.0),
        depth: Length::ZERO,
        cells: vec![
            grid_cell(0.0, 14.0, "R0C0"),
            grid_cell(20.0, 14.0, "R0C1"),
            grid_cell(0.0, 4.0, "R1C0"),
            grid_cell(20.0, 4.0, "R1C1"),
        ],
        rules,
    }
}

/// A line whose boxes carry an x offset — what `primitives.rs`'s
/// `indent_left` leaves behind for a `block-frame-breakable`'s `pad_l`.
fn indented_line(x: f64, text: &str) -> VertBox {
    VertBox::Line {
        height: Length::pt(9.0),
        depth: Length::pt(2.0),
        leading: Length::pt(12.0),
        contents: vec![(Length::pt(x), text_run(text))],
    }
}

/// A `block-frame-breakable`'s left padding exists ONLY as the x offset
/// `indent_left` folded into each contained line — no marker carries it. The
/// walker discards x everywhere else, so an `enumitem` list, which indents
/// purely this way, came out with every level flush left.
#[test]
fn frame_left_padding_survives_as_a_margin_left() {
    let out = render(&[indented_line(36.0, "nested item")]);
    let html = body_of(&out);
    assert!(
        html.contains("margin-left:36pt;"),
        "indentation lost:\n{html}"
    );
}

/// The control: an unindented paragraph must gain no margin at all, and a
/// sub-point offset is a kern, not an indent.
#[test]
fn an_unindented_paragraph_gets_no_margin_left() {
    let flush = render(&[text_line("flush left")]);
    let kerned = render(&[indented_line(0.4, "still flush left")]);
    for html in [body_of(&flush), body_of(&kerned)] {
        assert!(
            !html.contains("margin-left"),
            "spurious indent:\n{html}"
        );
    }
}

/// A CENTRED line's content sits at a large x — that is the alignment
/// offset, not an indent, and `data-align` already carries it. Reading it as
/// an indent pushed every centred table 163pt to the right AND kept it
/// centred within what remained.
#[test]
fn a_centred_line_reports_no_indent() {
    let vboxes = vec![VertBox::Line {
        height: Length::pt(9.0),
        depth: Length::pt(2.0),
        leading: Length::pt(12.0),
        contents: vec![
            (Length::ZERO, PureHorzBox::OuterFil),
            (Length::pt(60.0), text_run("centred")),
            (Length::pt(100.0), PureHorzBox::OuterFil),
        ],
    }];
    let out = render(&vboxes);
    let html = body_of(&out);
    assert!(
        html.contains("data-align=\"center\""),
        "lost the centring:\n{html}"
    );
    assert!(
        !html.contains("margin-left"),
        "alignment offset misread as an indent:\n{html}"
    );
}

/// Which grid lines a table draws is the DOCUMENT's business, carried by
/// `TabularBox::rules`. A blanket `td { border }` rendered `easytable`'s
/// three-rule booktabs look as a full grid. Here: one rule above row 0 and
/// nothing else.
#[test]
fn table_rules_become_per_cell_borders_and_nothing_else_does() {
    let tab = two_by_two(vec![rule_line(0.0, 20.0, 40.0, 20.0, 1.0)]);
    let out = render(&[line(PureHorzBox::Tabular(tab))]);
    let html = body_of(&out);

    assert_eq!(
        html.matches("border-top:1pt solid").count(),
        2,
        "expected the rule on both cells of row 0 only:\n{html}"
    );
    assert!(
        !html.contains("border-left") && !html.contains("border-right"),
        "invented a vertical rule the document never drew:\n{html}"
    );
    assert!(
        !html.contains("border-bottom"),
        "invented a bottom rule:\n{html}"
    );
}

/// A vertical rule lands on a column boundary, as `border-left`.
#[test]
fn a_vertical_rule_becomes_a_column_border() {
    let tab = two_by_two(vec![rule_line(20.0, 0.0, 20.0, 20.0, 0.5)]);
    let out = render(&[line(PureHorzBox::Tabular(tab))]);
    let html = body_of(&out);
    assert_eq!(
        html.matches("border-left:0.5pt solid").count(),
        2,
        "expected the rule left of column 1, on both rows:\n{html}"
    );
    assert!(!html.contains("border-top"), "misread as horizontal:\n{html}");
}

/// A table with no rules gets no borders — the stylesheet must not supply
/// one of its own.
#[test]
fn a_ruleless_table_draws_no_borders() {
    let out = render(&[line(PureHorzBox::Tabular(two_by_two(vec![])))]);
    let html = body_of(&out);
    assert!(
        !html.contains("border-"),
        "a table the document left unruled gained borders:\n{html}"
    );
}

/// `easytable` draws a table as TWO overlaid `tabular`s in one
/// `inline-graphics`: rules over phantom cells, then content with no rules.
/// Rendered independently the rules were dropped with the empty table and the
/// visible one came out bare. See `Ctx::tabular_rules`.
#[test]
fn overlaid_rule_and_content_tabulars_are_paired() {
    let phantom = TabularBox {
        cells: two_by_two(vec![])
            .cells
            .into_iter()
            .map(|c| TabularCellBox {
                contents: vec![],
                ..c
            })
            .collect(),
        ..two_by_two(vec![rule_line(0.0, 20.0, 40.0, 20.0, 1.0)])
    };
    let overlay = PureHorzBox::Graphics {
        width: Length::pt(40.0),
        height: Length::pt(20.0),
        depth: Length::ZERO,
        origin_independent: false,
        elems: vec![
            draw_text(PureHorzBox::Tabular(phantom)),
            draw_text(PureHorzBox::Tabular(two_by_two(vec![]))),
        ],
    };
    let out = render(&[line(overlay)]);
    let html = body_of(&out);

    assert_eq!(
        html.matches("<table").count(),
        1,
        "the phantom half must not render as a table of its own:\n{html}"
    );
    assert!(html.contains("R0C0"), "lost the content half:\n{html}");
    assert_eq!(
        html.matches("border-top:1pt solid").count(),
        2,
        "the rules did not reach the visible table:\n{html}"
    );
}

/// A graphics box whose every element is a `draw-text` draws NOTHING: its
/// `<svg>` comes out with an empty `<g>`. Emitting the sized wrapper anyway
/// reserved the box's full extent a second time, on top of the content's own
/// — every `easytable` table sat under a table-sized rectangle of blank space.
#[test]
fn a_text_only_graphics_box_emits_no_sized_wrapper() {
    let overlay = PureHorzBox::Graphics {
        width: Length::pt(40.0),
        height: Length::pt(20.0),
        depth: Length::ZERO,
        origin_independent: false,
        elems: vec![draw_text(text_run("drawn text"))],
    };
    let out = render(&[line(overlay)]);
    let html = body_of(&out);
    assert!(html.contains("drawn text"), "lost the content:\n{html}");
    assert!(
        !html.contains("class=\"gfx\""),
        "emitted a wrapper for a box that draws nothing:\n{html}"
    );
    // The control: a box that DOES draw keeps its wrapper.
    let drawn = PureHorzBox::Graphics {
        width: Length::pt(40.0),
        height: Length::pt(20.0),
        depth: Length::ZERO,
        origin_independent: false,
        elems: vec![rule_line(0.0, 0.0, 40.0, 0.0, 1.0)],
    };
    assert!(
        render(&[line(drawn)]).contains("class=\"gfx\""),
        "a real drawing lost its wrapper"
    );
}

fn draw_text(bx: PureHorzBox) -> GraphicsElem {
    GraphicsElem::Text {
        pt: (Length::ZERO, Length::ZERO),
        contents: vec![(Length::ZERO, bx)],
        width: Length::pt(40.0),
        height: Length::pt(20.0),
        depth: Length::ZERO,
        transform: None,
    }
}

/// A `+code` block and a wrapped paragraph are structurally IDENTICAL in the
/// box stream — both are consecutive `Line`s with nothing between them,
/// because `code.satyh` calls `line-break` once per source line exactly as
/// the line breaker does per wrapped line. The face is the only signal that
/// separates them, so a code block arrived as one long line of proportional
/// serif: `let rec map f xs = match xs with | [] -> [] | x :: rest -> …`.
#[test]
fn a_monospace_paragraph_keeps_its_line_breaks_and_says_so() {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lib-rustyfi/dist/fonts/lmmono10-regular.otf");
    let Ok(store) = rustyfi_pdf::TtfFontStore::load(&path, None, None) else {
        eprintln!("skipping: {} did not load", path.display());
        return;
    };
    let vboxes = vec![text_line("line one"), text_line("line two")];
    let html = rustyfi_html::render_html_reflow_ttf_with(
        Some(&vboxes),
        &geometry(),
        &store,
        &[],
        &DocExtras::default(),
        &[],
        &[],
    )
    .expect("reflow HTML rendering must succeed");
    let body = body_of(&html);

    assert!(body.contains("<br>"), "line break collapsed:\n{body}");
    assert!(
        body.contains("class=\"para code\""),
        "not marked as code, so the stylesheet still justifies it:\n{body}"
    );
    assert!(
        html.contains("monospace"),
        "no monospace fallback in the stack:\n{html}"
    );
    // The control: the same two lines with NO font store rejoin as prose.
    let prose = render(&vboxes);
    assert!(
        !body_of(&prose).contains("<br>"),
        "a proportional paragraph must still rejoin its lines:\n{prose}"
    );
}

/// A block frame's decoration is a lang-side callback, and this backend has
/// no page grid to run it on — so `.frame` drew NOTHING, and a `stdjabook`
/// title block, a `+code` panel and every framed figure arrived as bare text.
/// `fire_hooks` already runs the callback for the PDF; its graphics now reach
/// here box-local (`DocumentValue::reflow_frame_decos`) and get stretched
/// over the div.
#[test]
fn a_frames_own_decoration_is_drawn_over_it() {
    let deco = DecoId(7);
    let decos = vec![(
        deco,
        rustyfi_backend::FrameDecoration {
            width: Length::pt(100.0),
            height: Length::pt(40.0),
            pads: (
                Length::pt(6.0),
                Length::pt(8.0),
                Length::pt(4.0),
                Length::pt(4.0),
            ),
            // A stroked outline, so NOT the plain-panel shortcut.
            elems: vec![rule_line(0.0, 0.0, 100.0, 0.0, 1.0)],
        },
    )];
    let vboxes = vec![
        VertBox::FrameStart(deco),
        text_line("inside"),
        VertBox::FrameEnd(deco),
    ];
    let out = rustyfi_html::render_html_reflow_with_decos(
        Some(&vboxes),
        &geometry(),
        &[],
        &DocExtras::default(),
        &[],
        &[],
        &decos,
    )
    .expect("reflow HTML rendering must succeed");
    let html = body_of(&out);

    assert!(
        html.contains("class=\"frame framed\""),
        "the frame is not marked as decorated:\n{html}"
    );
    assert!(
        html.contains("<svg class=\"frame-deco\""),
        "the decoration was not drawn:\n{html}"
    );
    assert!(
        html.contains("preserveAspectRatio=\"none\""),
        "the decoration must stretch with the box:\n{html}"
    );
    // Only `padding-right` — the other three are already in the flow, as the
    // contained lines' own x offsets and as `FramePad` skips.
    assert!(html.contains("padding-right:8pt;"), "{html}");
    assert!(!html.contains("padding-left"), "{html}");
    assert!(html.contains("inside"), "lost the frame's content:\n{html}");
    assert_balanced_tags(&out);
}

/// A frame that draws NOTHING must still draw nothing — `block-frame-
/// breakable` is how packages group content, and `enumitem`'s manual alone
/// opens 336 of them.
#[test]
fn an_undecorated_frame_is_left_alone() {
    let vboxes = vec![
        VertBox::FrameStart(DecoId(1)),
        text_line("plain grouping"),
        VertBox::FrameEnd(DecoId(1)),
    ];
    let out = render(&vboxes);
    let html = body_of(&out);
    assert!(html.contains("class=\"frame\""), "{html}");
    assert!(!html.contains("framed"), "invented a decoration:\n{html}");
    assert!(!html.contains("frame-deco"), "{html}");
}

/// A decoration that is nothing but a filled rectangle covering the frame —
/// `+code`'s grey panel, and most highlight boxes — becomes a real
/// `background`, which is exact at every width and costs no markup.
#[test]
fn a_plain_filled_panel_becomes_a_background_not_an_svg() {
    let deco = DecoId(3);
    let panel = Path {
        subpaths: vec![Subpath {
            start: (Length::ZERO, Length::ZERO),
            segs: vec![
                PathSeg::Line((Length::pt(100.0), Length::ZERO)),
                PathSeg::Line((Length::pt(100.0), Length::pt(40.0))),
                PathSeg::Line((Length::ZERO, Length::pt(40.0))),
            ],
            closing: Closing::Line,
        }],
    };
    let decos = vec![(
        deco,
        rustyfi_backend::FrameDecoration {
            width: Length::pt(100.0),
            height: Length::pt(40.0),
            pads: (Length::ZERO, Length::ZERO, Length::ZERO, Length::ZERO),
            elems: vec![GraphicsElem::Fill(Color::Gray(0.9), panel)],
        },
    )];
    let vboxes = vec![
        VertBox::FrameStart(deco),
        text_line("code"),
        VertBox::FrameEnd(deco),
    ];
    let out = rustyfi_html::render_html_reflow_with_decos(
        Some(&vboxes),
        &geometry(),
        &[],
        &DocExtras::default(),
        &[],
        &[],
        &decos,
    )
    .expect("reflow HTML rendering must succeed");
    let html = body_of(&out);
    assert!(html.contains("background:"), "no background panel:\n{html}");
    assert!(
        !html.contains("frame-deco"),
        "a flat panel should need no SVG:\n{html}"
    );
}
