//! Integration tests for the reflowable/semantic HTML writer
//! (`render_html_reflow`, Slice 1): hand-built `Vec<VertBox>` fixtures
//! (mirroring `tests/html.rs`'s synthetic box-construction style for the
//! faithful writer) exercising paragraph grouping/splitting,
//! frame/embedded-block nesting, inline run styling, and the "no absolute
//! positioning" invariant that is the defining difference from the faithful
//! mode.

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

/// Rejoining two lines of one paragraph puts an ordinary space between
/// them — that is what a line break between two Latin words is.
#[test]
fn rejoined_lines_take_one_space_between_words() {
    let html = render(&[text_line("Hello,"), text_line("world!")]);
    assert!(
        html.contains("<p class=\"para\">Hello, world!</p>"),
        "two lines of one paragraph should rejoin with one space:\n{html}"
    );
}

/// …but NOT after a hyphen the line breaker itself inserted. A taken
/// hyphenation leaves the discretionary's `pre_break` — a lone one-character
/// run — spliced at the line's end (`linebreak.rs`'s `line_content`); the
/// document says `figbox`, and rejoining naively said `fig- box`. The hyphen
/// goes back to being the soft hyphen it stands for.
#[test]
fn a_line_breaker_hyphen_becomes_a_soft_hyphen_not_a_space() {
    let html = render(&[
        line_of(vec![text_run("fig"), text_run("-")]),
        text_line("box"),
    ]);
    assert!(
        html.contains("<p class=\"para\">fig&shy;box</p>"),
        "the breaker's hyphen should rejoin as a soft hyphen:\n{html}"
    );
}

/// …and not after a hyphen that was in the TEXT either. UAX#14 breaks AFTER
/// an explicit hyphen, so `align-right` splits as `align-` / `right` with
/// the hyphen still part of its run — it must be kept, and still not gain a
/// space.
#[test]
fn an_explicit_hyphen_at_a_line_end_keeps_its_hyphen_and_gains_no_space() {
    let html = render(&[text_line("align-"), text_line("right")]);
    assert!(
        html.contains("<p class=\"para\">align-right</p>"),
        "an explicit hyphen must survive the rejoin without a space:\n{html}"
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
    }
    let vboxes = vec![line(bx)];
    let html = render(&vboxes);

    assert!(
        html.contains("color:rgb(255,0,0)"),
        "missing color CSS:\n{html}"
    );
    assert!(
        html.contains("vertical-align:3pt"),
        "missing rising-as-vertical-align CSS:\n{html}"
    );
}

/// The document's own dominant `(font, size)` goes on `body`
/// (`text::BodyStyle::dominant`, `css.rs`), so a run set in it needs no
/// element at all: body prose serializes as bare escaped text. This is what
/// makes the output readable markup rather than one `<span>` per syllable.
#[test]
fn a_body_styled_run_is_written_as_bare_text_with_no_span() {
    let html = render(&[text_line("ordinary prose")]);
    assert!(
        html.contains("<p class=\"para\">ordinary prose</p>"),
        "a body-styled run should carry no <span> at all:\n{html}"
    );
    assert!(
        html.contains("font-size: 12pt"),
        "the dominant size should be stated once, on `body`:\n{html}"
    );
}

/// A run that DIFFERS from the body style states its size as an `em` RATIO
/// of the body size, not an absolute point size — so the whole document
/// rescales from the single value on `body`. Here the 12pt runs win the
/// count and become the body, and the one 18pt run comes out at 1.5em.
#[test]
fn an_off_body_size_becomes_an_em_ratio_of_the_body_size() {
    let mut big = text_run("BIG");
    if let PureHorzBox::InnerString { info, .. } = &mut big {
        info.size = Length::pt(18.0);
    }
    let html = render(&[
        text_line("plenty of ordinary body text here"),
        text_line("and more of it, to win the count"),
        line(big),
    ]);
    assert!(
        html.contains("font-size:1.5000em"),
        "an off-body size should be an em ratio:\n{html}"
    );
    assert!(
        !html.contains("font-size:18pt"),
        "an off-body size should not be frozen at an absolute point size:\n{html}"
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

/// `draw-text` inside a graphic must come out as SVG `<text>`, not as an
/// HTML `<span>`. `span` is on the HTML parser's foreign-content BREAKOUT
/// list, so a `<span>` between `<g>` and `</g>` makes a browser close the
/// `<svg>` early and reparse the rest of the drawing as HTML — the text is
/// not merely mispositioned, the whole graphic is mangled. See `svg.rs`'s
/// doc comment.
#[test]
fn draw_text_inside_a_graphic_renders_as_svg_text_not_an_html_span() {
    let gfx_box = PureHorzBox::Graphics {
        origin_independent: false,
        width: Length::pt(40.0),
        height: Length::pt(10.0),
        depth: Length::ZERO,
        elems: vec![GraphicsElem::Text {
            pt: (Length::pt(2.0), Length::pt(3.0)),
            contents: vec![(Length::ZERO, text_run("drawn"))],
            width: Length::pt(40.0),
            height: Length::pt(9.0),
            depth: Length::pt(2.0),
            transform: None,
        }],
    };
    let html = render(&[line(gfx_box)]);

    let svg_start = html.find("<svg").expect("missing the graphic's <svg>");
    let svg_end = html[svg_start..].find("</svg>").expect("unclosed <svg>") + svg_start;
    let inside = &html[svg_start..svg_end];
    assert!(
        inside.contains("<text"),
        "the drawn run should be an SVG <text>:\n{html}"
    );
    assert!(
        inside.contains(">drawn</text>"),
        "the drawn run should keep its text:\n{html}"
    );
    assert!(
        !inside.contains("<span"),
        "no HTML element may appear inside an <svg>:\n{html}"
    );
    // The enclosing `<g>` flips the y axis for paths; a glyph must be
    // counter-flipped or it renders mirrored.
    assert!(
        inside.contains("scale(1,-1)\" style="),
        "the drawn text must undo the box flip:\n{html}"
    );
}

/// A `draw-text` carrying something SVG cannot host — `figbox`'s figures put
/// images, tables and embedded blocks there — must still reach the document.
/// It comes back through `svg::Deferred` and is emitted after the wrapper
/// closes, as ordinary flowing content: a `<table>` OUTSIDE the `<svg>`,
/// never inside it.
#[test]
fn a_table_drawn_into_a_graphic_is_emitted_after_the_svg_not_inside_it() {
    let cell = TabularCellBox {
        x: Length::ZERO,
        baseline_y: Length::ZERO,
        contents: vec![(Length::ZERO, text_run("cell"))],
    };
    let table = PureHorzBox::Tabular(TabularBox {
        width: Length::pt(20.0),
        height: Length::pt(9.0),
        depth: Length::pt(2.0),
        cells: vec![cell],
        rules: vec![],
    });
    let gfx_box = PureHorzBox::Graphics {
        origin_independent: false,
        width: Length::pt(40.0),
        height: Length::pt(12.0),
        depth: Length::ZERO,
        elems: vec![GraphicsElem::Text {
            pt: (Length::ZERO, Length::ZERO),
            contents: vec![(Length::ZERO, table)],
            width: Length::pt(20.0),
            height: Length::pt(9.0),
            depth: Length::pt(2.0),
            transform: None,
        }],
    };
    let html = render(&[line(gfx_box)]);

    let svg_end = html.find("</svg>").expect("missing the graphic's <svg>");
    let table_at = html.find("<table").expect("the drawn table went missing");
    assert!(
        table_at > svg_end,
        "the table must be emitted after the </svg>, not inside it:\n{html}"
    );
    assert!(
        html.contains("cell"),
        "the cell's text went missing:\n{html}"
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

/// The two CSS box-offset properties, and the flow-safe longhands that end
/// in the same word. `top`/`left` as bare declarations are page positioning;
/// `margin-top`/`border-left`/`padding-left`/… are ordinary flow properties
/// that merely share the suffix, and the stylesheet uses several of them
/// (the `.clearpage` rule, the footnote `<aside>`'s rule).
pub(crate) fn assert_no_box_offsets(html: &str) {
    assert!(
        !html.contains("position:absolute") && !html.contains("position: absolute"),
        "reflow output must never use position:absolute:\n{html}"
    );
    for prop in ["top:", "left:", "right:", "bottom:"] {
        for (idx, _) in html.match_indices(prop) {
            let before = &html[..idx];
            assert!(
                ["margin-", "border-", "padding-", "inset-", "scroll-margin-"]
                    .iter()
                    .any(|p| before.ends_with(p)),
                "found a bare `{prop}` CSS declaration at byte {idx}:\n{html}"
            );
        }
    }
}

/// The defining difference from the faithful (`render_html_fixed`) mode:
/// NOTHING in reflow output is absolutely positioned — no
/// `position:absolute`, and no bare `top`/`left`/`right`/`bottom`
/// declaration. See [`assert_no_box_offsets`].
#[test]
fn reflow_output_never_uses_absolute_positioning() {
    let vboxes = vec![
        text_line("first"),
        VertBox::Skip(Length::pt(6.0)),
        VertBox::FrameStart(DecoId(1)),
        text_line("second"),
        VertBox::FrameEnd(DecoId(1)),
    ];
    assert_no_box_offsets(&render(&vboxes));
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

    assert!(
        html.contains("<nav class=\"toc\">"),
        "missing TOC nav:\n{html}"
    );
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

/// A `Tabular` reached through inline content — inside a `Frame`, or drawn
/// into a graphic — cannot be written where it stands: `<table>` inside
/// `<p>` is not valid HTML, and a parser closes the paragraph at the
/// `<table>` and leaves the `</p>` stray (the easytable manual had 18 of
/// them). It is queued and emitted just after the paragraph instead.
#[test]
fn a_table_inside_inline_content_is_emitted_after_the_paragraph_not_within_it() {
    let table = PureHorzBox::Tabular(TabularBox {
        width: Length::pt(20.0),
        height: Length::pt(9.0),
        depth: Length::ZERO,
        cells: vec![TabularCellBox {
            x: Length::ZERO,
            baseline_y: Length::ZERO,
            contents: vec![(Length::ZERO, text_run("cell"))],
        }],
        rules: vec![],
    });
    let framed = PureHorzBox::Frame {
        width: Length::pt(20.0),
        height: Length::pt(9.0),
        depth: Length::ZERO,
        deco: DecoId(0),
        contents: vec![(Length::ZERO, table)],
    };
    let html = render(&[line_of(vec![text_run("before"), framed])]);

    let para_close = html.find("</p>").expect("missing the paragraph");
    let table_at = html.find("<table").expect("the nested table went missing");
    assert!(
        table_at > para_close,
        "a <table> must never open inside a <p>:\n{html}"
    );
    assert!(
        html.contains("before"),
        "the paragraph's own text went missing:\n{html}"
    );
}

/// A table package pads its own cells with `inline-skip` struts on both
/// sides — `easytable`'s manual emits two per cell, 2136 in all. `<td>`'s CSS
/// padding is HTML's way to say that, so the struts at a cell's edges go;
/// one BETWEEN two words is spacing the author wrote and stays.
#[test]
fn a_cells_own_padding_struts_are_dropped_but_inner_spacing_is_kept() {
    let pad = || PureHorzBox::FixedEmpty {
        width: Length::pt(6.0),
    };
    let tab = TabularBox {
        width: Length::pt(40.0),
        height: Length::pt(20.0),
        depth: Length::ZERO,
        cells: vec![TabularCellBox {
            x: Length::ZERO,
            baseline_y: Length::ZERO,
            contents: vec![
                (Length::ZERO, pad()),
                (Length::ZERO, text_run("a")),
                (Length::ZERO, pad()),
                (Length::ZERO, text_run("b")),
                (Length::ZERO, pad()),
            ],
        }],
        rules: vec![],
    };
    let html = render(&[line(PureHorzBox::Tabular(tab))]);

    assert_eq!(
        html.matches("class=\"hskip\"").count(),
        1,
        "only the strut between the two words should survive:\n{html}"
    );
    assert!(
        html.contains("<td>a<span class=\"hskip\" style=\"width:6pt;\"></span>b</td>"),
        "the cell should start and end on its own text:\n{html}"
    );
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
/// an anchor frame inside a link frame closes its `</span>` before the
/// `</a>`. Getting this wrong produces overlapping tags that browsers
/// silently reinterpret, so it would not show up as a crash.
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
    let dests = vec![(DecoId(22), "inner-anchor".to_string())];
    let html = render_with_links(&vboxes, &links, &dests);

    let span_close = html
        .find("</span>")
        .expect("inner frame must close a <span>");
    let a_close = html.find("</a>").expect("outer link must close an </a>");
    assert!(
        span_close < a_close,
        "inner </span> must close before the outer </a>:\n{html}"
    );
}

/// An `inline-frame-breakable` that is NEITHER a link NOR a named
/// destination has nothing to say — every `\code`-style command in the
/// corpus goes through one — so it emits no element at all. An empty
/// wrapper is not merely noise: it would split two adjacent identical runs
/// that [`a_body_styled_run_is_written_as_bare_text_with_no_span`]'s sibling
/// coalescing would otherwise join.
#[test]
fn a_breakable_inline_frame_with_no_link_or_anchor_emits_no_wrapper() {
    let vboxes = vec![line_of(vec![
        iframe_marker(31, false),
        text_run("plain"),
        iframe_marker(31, true),
    ])];
    let html = render(&vboxes);

    assert!(
        html.contains("<p class=\"para\">plain</p>"),
        "a decoration-less inline frame should leave its text bare:\n{html}"
    );
    assert!(
        !html.contains("class=\"iframe\""),
        "no wrapper should be emitted for a frame with nothing to say:\n{html}"
    );
}

/// Two runs that carry the SAME style are one `<span>`, not two — including
/// across the word space between them. The box stream splits text at every
/// UAX#14 chunk boundary and between every pair of CJK characters, so
/// without this a Japanese title serialises as one element per character.
#[test]
fn adjacent_runs_with_the_same_style_become_one_span() {
    let styled = |text: &str| {
        let mut bx = text_run(text);
        if let PureHorzBox::InnerString { info, .. } = &mut bx {
            info.color = Color::Rgb(1.0, 0.0, 0.0);
        }
        bx
    };
    let html = render(&[line_of(vec![
        styled("Latex"),
        styled("Cmds"),
        PureHorzBox::OuterEmpty {
            natural: Length::pt(3.5),
            shrinkable: Length::ZERO,
            stretchable: Length::ZERO,
        },
        styled("パ"),
        styled("ッ"),
    ])]);

    assert_eq!(
        html.matches("<span class=\"run\"").count(),
        1,
        "four same-styled runs should be one span:\n{html}"
    );
    assert!(
        html.contains(">LatexCmds パッ</span>"),
        "the merged span should hold all four runs' text, space included:\n{html}"
    );
}

/// …but only when the style genuinely matches. A run that differs in any
/// property opens its own span, and the one after it does not silently join
/// the WRONG neighbour.
#[test]
fn a_differently_styled_run_still_opens_its_own_span() {
    let mut red = text_run("red");
    if let PureHorzBox::InnerString { info, .. } = &mut red {
        info.color = Color::Rgb(1.0, 0.0, 0.0);
    }
    let mut blue = text_run("blue");
    if let PureHorzBox::InnerString { info, .. } = &mut blue {
        info.color = Color::Rgb(0.0, 0.0, 1.0);
    }
    let html = render(&[line_of(vec![red, blue])]);

    assert_eq!(
        html.matches("<span class=\"run\"").count(),
        2,
        "two differently-coloured runs must stay two spans:\n{html}"
    );
    assert!(
        html.contains(">red</span>") && html.contains(">blue</span>"),
        "each run keeps its own text:\n{html}"
    );
}
