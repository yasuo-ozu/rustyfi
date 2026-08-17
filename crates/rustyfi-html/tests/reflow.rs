//! Integration tests for the reflowable/semantic HTML writer
//! (`render_html_reflow`, `docs/plans/design-reflowable-html.md` Slice 1):
//! hand-built `Vec<VertBox>` fixtures (mirroring `tests/html.rs`'s synthetic
//! box-construction style for the faithful writer) exercising paragraph
//! grouping/splitting, frame/embedded-block nesting, inline run styling, and
//! the "no absolute positioning" invariant that is the defining difference
//! from the faithful mode.

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

/// S3 (`docs/plans/design-reflowable-html.md` §6 "S3"): full-signature
/// helper exercising `extras.outline` alongside `links`/`dests`.
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

    assert!(html.starts_with("<!doctype html>"), "missing doctype:\n{html}");
    let para_count = html.matches("<p class=\"para\"").count();
    assert_eq!(para_count, 2, "expected exactly two <p>s:\n{html}");
    assert!(html.contains("Hello,"), "missing first line's text:\n{html}");
    assert!(html.contains("world!"), "missing second line's text:\n{html}");
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

    assert!(html.contains("<div class=\"frame\""), "missing frame div:\n{html}");
    assert!(html.contains("inside the frame"), "missing frame content:\n{html}");
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
    assert!(frame_open < para_open, "frame should open before its content:\n{html}");
}

#[test]
fn embedded_block_becomes_a_nested_div_recursively() {
    let inner_vboxes = vec![text_line("nested content")];
    let embed_box = PureHorzBox::EmbeddedBlock {
        width: Length::pt(100.0),
        height: Length::pt(20.0),
        depth: Length::ZERO,
        block: inner_vboxes,
        anchor_last: false,
    };
    let vboxes = vec![line(embed_box)];
    let html = render(&vboxes);

    assert!(html.contains("<div class=\"embed\""), "missing embed div:\n{html}");
    assert!(html.contains("nested content"), "missing nested block's text:\n{html}");
}

#[test]
fn styled_run_carries_color_and_rising_as_css_not_position() {
    let mut bx = text_run("styled");
    if let PureHorzBox::InnerString { info, .. } = &mut bx {
        info.color = Color::Rgb(1.0, 0.0, 0.0);
        info.rising = Length::pt(3.0);
    }
    let vboxes = vec![line(bx)];
    let html = render(&vboxes);

    assert!(html.contains("color:rgb(255,0,0)"), "missing color CSS:\n{html}");
    assert!(
        html.contains("vertical-align:3pt"),
        "missing rising-as-vertical-align CSS:\n{html}"
    );
    assert!(html.contains("font-size:12pt"), "missing font-size CSS:\n{html}");
}

#[test]
fn run_text_is_html_escaped() {
    let vboxes = vec![text_line("<a & \"b\">")];
    let html = render(&vboxes);
    assert!(
        html.contains("&lt;a &amp; &quot;b&quot;&gt;"),
        "text was not HTML-escaped:\n{html}"
    );
    assert!(!html.contains("<a & \"b\">"), "raw unescaped text leaked:\n{html}");
}

/// Slice 2 (`docs/plans/design-reflowable-html.md` §4 "Math"): a `Math` box
/// (glyphs only, no `rules`) must render as a real inline `<svg>` — not the
/// Slice 1 `math-placeholder` `<span>` — with the glyph's literal text
/// inside an SVG `<text>` element.
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
    assert!(html.contains("<svg"), "missing inline <svg> for math:\n{html}");
    assert!(html.contains("<text"), "missing SVG <text> glyph element:\n{html}");
    assert!(html.contains('x'), "missing the glyph's literal text:\n{html}");
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
    assert!(html.contains("<svg"), "missing inline <svg> for math rules:\n{html}");
    assert!(html.contains("<path"), "missing SVG <path> for the fraction-bar rule:\n{html}");
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
    assert!(html.contains("<svg"), "missing inline <svg> for graphics:\n{html}");
    assert!(html.contains("<path"), "missing SVG <path> for the fill:\n{html}");
    assert!(html.contains("rgb(255,0,0)"), "missing the fill color:\n{html}");
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
    let links = vec![(DecoId(7), AnnotAction::Uri("https://example.com".to_string()))];
    let html = render_with_links(&vboxes, &links, &[]);

    assert!(
        html.contains("<a class=\"link\" href=\"https://example.com\">"),
        "missing <a href> for the link frame:\n{html}"
    );
    assert!(html.contains("click me"), "missing the link's inline text:\n{html}");
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

/// The defining difference from the faithful (`render_html`) mode: NOTHING
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

    assert!(
        !html.contains("position:absolute") && !html.contains("position: absolute"),
        "reflow output must never use position:absolute:\n{html}"
    );
    assert!(!html.contains("left:"), "reflow output must never use `left:`:\n{html}");
    // `top:` is only allowed as a suffix of a flow-safe longhand
    // (`margin-top:`, `border-top:` — used by the static `.clearpage`/
    // `.frame` stylesheet rules, `css.rs`), never as the bare positioned
    // `top` property.
    for (idx, _) in html.match_indices("top:") {
        assert!(
            html[..idx].ends_with("margin-") || html[..idx].ends_with("border-"),
            "found a bare `top:` CSS declaration (not margin-top/border-top) at byte {idx}:\n{html}"
        );
    }
}

#[test]
fn missing_reflow_source_renders_a_placeholder_instead_of_panicking() {
    let html = rustyfi_html::render_html_reflow(
        None,
        &geometry(),
        &[],
        &DocExtras::default(),
        &[],
        &[],
    )
    .expect("must not panic/error when reflow_source is None");
    assert!(html.starts_with("<!doctype html>"), "missing doctype:\n{html}");
    assert!(
        html.contains("reflow-empty"),
        "missing the no-source placeholder:\n{html}"
    );
}

// ---------------------------------------------------------------------
// Slice 3 (`docs/plans/design-reflowable-html.md` §6 "S3"): outline-driven
// headings + navigable TOC, and real `<table>` from `Tabular`.
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
    assert!(html.contains("Introduction"), "missing heading text:\n{html}");
    assert!(html.contains("</h1>"), "missing closing </h1>:\n{html}");
    // The destination id= anchor (S2's Frame/dests wiring) must still be
    // present INSIDE the promoted heading, unaffected by S3's tag swap.
    assert!(html.contains("id=\"sec1\""), "missing id anchor inside heading:\n{html}");
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
    assert!(!html.contains("<nav"), "unexpected <nav> with no outline:\n{html}");
    assert!(!html.contains("<h1"), "unexpected heading with no outline:\n{html}");
    assert!(html.contains("<p class=\"para\">"), "missing plain <p>:\n{html}");
}

/// `extras.outline` (even without any matching in-flow destination frame)
/// must render a navigable `<nav class="toc">` nested list with a real
/// `<a href="#dest_name">` — design doc §3's "Navigation (always safe)".
#[test]
fn outline_renders_a_navigable_toc_nav() {
    let extras = DocExtras {
        outline: vec![
            OutlineEntry {
                level: 0,
                text: "Chapter One".to_string(),
                dest_name: "ch1".to_string(),
                is_open: false,
            },
            OutlineEntry {
                level: 1,
                text: "Section 1.1".to_string(),
                dest_name: "ch1sec1".to_string(),
                is_open: false,
            },
        ],
        ..DocExtras::default()
    };
    let html = render_with_extras(&[text_line("body")], &extras, &[], &[]);

    assert!(html.contains("<nav class=\"toc\">"), "missing TOC nav:\n{html}");
    assert!(
        html.contains("<a href=\"#ch1\">Chapter One</a>"),
        "missing top-level TOC entry:\n{html}"
    );
    assert!(
        html.contains("<a href=\"#ch1sec1\">Section 1.1</a>"),
        "missing nested TOC entry:\n{html}"
    );
    // The level-1 entry must be nested inside a SECOND <ol>, not a sibling
    // of the level-0 <li> at the same depth.
    assert_eq!(
        html.matches("<ol>").count(),
        2,
        "expected one nested <ol> for the level-1 entry:\n{html}"
    );
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
    assert!(html.contains("<table class=\"tabular\">"), "missing <table>:\n{html}");
    assert_eq!(html.matches("<tr>").count(), 2, "expected two rows:\n{html}");
    assert_eq!(html.matches("<td>").count(), 4, "expected four cells:\n{html}");
    for text in ["R0C0", "R0C1", "R1C0", "R1C1"] {
        assert!(html.contains(text), "missing cell text {text}:\n{html}");
    }
}

// ============================================================================
// S4 (`docs/plans/design-reflow-s4-lists.md`): semantic lists + emphasis.
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

    assert_eq!(html.matches("<ul").count(), 1, "expected exactly one <ul>:\n{html}");
    assert_eq!(html.matches("</ul>").count(), 1, "expected exactly one </ul>:\n{html}");
    assert_eq!(html.matches("<li").count(), 1, "expected exactly one <li>:\n{html}");
    assert_eq!(html.matches("</li>").count(), 1, "expected exactly one </li>:\n{html}");
    assert!(!html.contains("<ol"), "unordered list must not render <ol>:\n{html}");
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

    assert_eq!(html.matches("<ul").count(), 2, "expected two <ul>s (outer + nested):\n{html}");
    assert_eq!(html.matches("</ul>").count(), 2, "expected two </ul>s:\n{html}");
    assert_eq!(html.matches("<li").count(), 2, "expected two <li>s:\n{html}");
    assert!(html.contains("parent"), "missing parent item text:\n{html}");
    assert!(html.contains("child"), "missing child item text:\n{html}");

    // Structural nesting check: the nested `<ul>` must open BEFORE the
    // outer/first `<li>`'s `</li>` closes — i.e. it sits INSIDE that `<li>`,
    // not as a sibling after it.
    let first_li = html.find("<li").expect("missing first <li>");
    let first_li_close = html[first_li..].find("</li>").expect("missing first </li>") + first_li;
    let parent_text = html.find("parent").expect("missing parent text");
    let nested_ul = html[parent_text..].find("<ul").expect("missing nested <ul>") + parent_text;
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

    assert_eq!(html.matches("<ol").count(), 1, "expected exactly one <ol>:\n{html}");
    assert_eq!(html.matches("</ol>").count(), 1, "expected exactly one </ol>:\n{html}");
    assert_eq!(html.matches("<li").count(), 2, "expected two <li>s:\n{html}");
    assert!(!html.contains("<ul"), "ordered list must not render <ul>:\n{html}");
    assert!(html.contains("first") && html.contains("second"), "missing item text:\n{html}");
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
            (Length::ZERO, PureHorzBox::InlineMark(InlineMarkKind::BulletStart)),
            (Length::ZERO, text_run("BULLETGLYPH")),
            (Length::ZERO, PureHorzBox::InlineMark(InlineMarkKind::BulletEnd)),
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
            (Length::ZERO, PureHorzBox::InlineMark(InlineMarkKind::EmphEnd)),
        ],
    }];
    let html = render(&vboxes);

    assert!(html.contains("<em>"), "missing <em>:\n{html}");
    assert!(html.contains("</em>"), "missing </em>:\n{html}");
    assert!(!html.contains("<strong>"), "\\emph must not render <strong>:\n{html}");
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
            (Length::ZERO, PureHorzBox::InlineMark(InlineMarkKind::EmphEnd)),
        ],
    }];
    let html = render(&vboxes);

    assert!(html.contains("<strong>"), "missing <strong>:\n{html}");
    assert!(html.contains("</strong>"), "missing </strong>:\n{html}");
    assert!(!html.contains("<em>"), "\\bold must not render <em>:\n{html}");
}
