//! Integration tests for the reflowable/semantic HTML writer
//! (`render_html_reflow`, `--format html`): hand-built `Vec<VertBox>`
//! fixtures built by hand rather than compiled from a document.
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
//! Throughout: the "no absolute positioning" invariant that defines this
//! backend.

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

/// A math glyph's `font-size` must be in the math `<svg>`'s own USER UNITS,
/// never an absolute `pt` length.
///
/// The `<svg>` is `width="{w}pt" … viewBox="0 0 {w} {h}"`, so one user unit
/// renders as one `pt` — which is exactly why `dx`/`dy` and the `rules` path
/// coordinates are emitted as bare `Length` numbers. An absolute CSS length
/// does NOT get that treatment: SVG converts `pt` to user units at
/// 1px = 1 user unit, so `font-size:12pt` inside this viewport resolves to
/// 16 user units and paints at 16pt. Every glyph came out 4/3 too big at a
/// position computed for 12pt, so glyphs overlapped each other, overflowed
/// the fraction bars and radical overbars beside them (`rules` paths, which
/// were correctly scaled all along), and spilled outside the wrapper
/// `<span>`'s reserved height into the lines above and below. Inline and
/// displayed math alike, since both are this one `PureHorzBox::Math` arm.
///
/// Asserting on the ABSENCE of `pt` here rather than on an exact string, so
/// the test keeps its meaning if the size is ever reformatted.
#[test]
fn math_glyph_font_size_is_in_svg_user_units_not_points() {
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
    let html = render(&[line(math_box)]);
    let text_elem = html
        .lines()
        .find(|l| l.contains("<text"))
        .unwrap_or_else(|| panic!("no SVG <text> glyph emitted:\n{html}"));
    assert!(
        !text_elem.contains("font-size:12pt"),
        "math glyph font-size is an absolute `pt` length inside a \
         1-user-unit-per-pt viewBox — it will paint 4/3 too big:\n{text_elem}"
    );
    // 12 user units is what "12pt of document font size" means in here;
    // `px` is the standards-safe spelling of one user unit.
    assert!(
        text_elem.contains("font-size:12px"),
        "math glyph font-size is not the box's own 12 user units:\n{text_elem}"
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

/// A math glyph the document placed by GLYPH ID (`MathGlyph::gid`) must be
/// drawn from that glyph's own OUTLINE, because no `<text>` can address it.
///
/// **The bug.** `gid` is `Some` exactly when the drawn form is not the glyph
/// its `text` cmaps to — a display-size big operator, a stretchy delimiter,
/// an `ssty` script form. The PDF writer emits the id directly; this backend
/// wrote `<text>∑</text>`, which can only ever produce the BASE `∑`. So the
/// operator came out base-size, and — because `layout_math_list` had centred
/// the limits on the VARIANT's width — `n` and `k = 1` were centred on a
/// glyph that was not there. In Latin Modern Math at 12pt: `summation`
/// advances 1.056 em, `summation.v1` 1.444 em, so every limit sat 2.334pt
/// right of its operator, and `\int`'s scripts began 4.008pt past the end of
/// the integral sign.
///
/// So the assertion that matters is not "a `<path>` appeared" but "the ink
/// that appeared is the VARIANT's, not the base's" — measured off the
/// emitted `d` and its `transform`, which is the whole of what the browser
/// will draw.
#[test]
fn a_math_variant_glyph_is_drawn_from_its_own_outline_not_as_a_character() {
    use rustyfi_backend::{FontMetrics, VertVariantPolicy};

    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../lib-rustyfi/dist/fonts/latinmodern-math.otf");
    let Ok(store) = rustyfi_pdf::TtfFontStore::load(&path, None, None) else {
        eprintln!("skipping: {} did not load", path.display());
        return;
    };
    let font = FontKey(0);
    let size = Length::pt(12.0);
    // The same call `primitives::push_big_char_glyph` makes, so the fixture
    // is the real variant record rather than a hand-picked gid.
    let Some(variant) = store.math_vertical_variant(font, '\u{2211}', size, VertVariantPolicy::BigOp)
    else {
        eprintln!("skipping: no MATH BigOp variant for U+2211 in {}", path.display());
        return;
    };
    let base_advance = store
        .advance(font, '\u{2211}', size)
        .expect("the face cmaps U+2211");
    assert!(
        variant.advance.0 > base_advance.0 + 1.0,
        "fixture is not exercising anything: the variant must be visibly \
         wider than the base ({} vs {})",
        variant.advance.0,
        base_advance.0
    );

    let math_box = PureHorzBox::Math {
        width: variant.advance,
        height: variant.height,
        depth: variant.depth,
        glyphs: vec![MathGlyph {
            text: "\u{2211}".to_string(),
            gid: Some(variant.gid),
            dx: Length::ZERO,
            dy: Length::ZERO,
            info: HorzStringInfo {
                font,
                size,
                rising: Length::ZERO,
                color: Color::Gray(0.0),
            },
            width: variant.advance,
            height: variant.height,
            depth: variant.depth,
        }],
        rules: vec![],
    };
    let html = rustyfi_html::render_html_reflow_ttf_with(
        Some(&[line(math_box)]),
        &geometry(),
        &store,
        &[],
        &DocExtras::default(),
        &[],
        &[],
    )
    .expect("reflow HTML rendering must succeed");

    // The character must not be DRAWN — a `<text>` can only ever address the
    // base glyph — but it must still be PRESENT, in the invisible phantom
    // layer, or the operator becomes uncopyable and unsearchable. Both halves
    // are asserted, because dropping either is a real regression: this test
    // originally required the character to be absent altogether, and that is
    // exactly the accessibility hole the phantom layer closes.
    let sigma = html.match_indices('\u{2211}').collect::<Vec<_>>();
    assert_eq!(
        sigma.len(),
        1,
        "the variant's character should appear exactly once:\n{html}"
    );
    let before = &html[..sigma[0].0];
    let open = before.rfind("<text ").expect("inside a <text> element");
    assert!(
        before[open..].contains("class=\"mphantom\""),
        "the character is in DRAWN text, so the browser paints the BASE \
         glyph over the variant's outline:\n{}",
        &before[open..]
    );
    assert!(
        html.contains(".math-glyphs .mphantom { fill: none; }"),
        "the phantom layer is not made invisible:\n{html}"
    );
    let (d, transform) = math_path_and_transform(&html).expect("a <path> for the variant glyph");

    // The transform must be `translate(pen) scale(s, -s)` with
    // `s = size / units_per_em` — the y-flip a filled path needs and a
    // `<text>` must never get (it would mirror the letters).
    let s = parse_uniform_flip_scale(&transform).unwrap_or_else(|| {
        panic!("transform is not a `scale(s -s)` uniform flip: {transform:?}")
    });
    assert!(
        (s - 12.0 / 1000.0).abs() < 1e-9,
        "scale should be size/units_per_em, got {s}"
    );

    // The load-bearing measurement: the drawn ink fills the VARIANT's
    // advance, which is what the surrounding layout was computed against —
    // not the base glyph's, which is what a `<text>` would have given.
    //
    // Ink is compared against ADVANCE, so the two bounds are asymmetric on
    // purpose. Ink is always the narrower of the two by the glyph's side
    // bearings (here 0.056 + 0.057 em), hence the 0.85 floor rather than an
    // equality; and it can only EXCEED the base advance if the glyph drawn is
    // not the base one, since a glyph's own ink never reaches past its own
    // advance. That second bound is the one that would have caught the bug.
    let (x_min, x_max) = path_x_extent(&d).expect("the path has coordinates");
    let ink = (x_max - x_min) * s;
    assert!(
        ink > base_advance.0,
        "the drawn ink ({ink}pt) is no wider than the BASE advance \
         ({}pt) — this is still the small glyph",
        base_advance.0
    );
    assert!(
        ink <= variant.advance.0 && ink >= 0.85 * variant.advance.0,
        "the drawn ink ({ink}pt) does not fill the variant advance \
         ({}pt) the limits were centred on",
        variant.advance.0
    );

    // Glyph outlines are NONZERO-winding (SVG's default). `evenodd`, which
    // every other path this backend writes carries because it is reproducing
    // PDF's `f*`, would punch holes in a CFF face's overlapping contours.
    let tag = math_path_tag(&html).expect("a <path> for the variant glyph");
    assert!(
        !tag.contains("fill-rule"),
        "a glyph outline must not carry a fill-rule — SVG's nonzero default \
         is the one fonts are drawn under:\n{tag}"
    );
}

/// An ORDINARY math glyph — one whose `text` really does cmap to the glyph
/// the document laid out — is drawn from its outline too, not as a `<text>`.
///
/// **The bug this closes is not the variant one above; it is bigger.** A
/// `<text>` names a face and hopes the reader has it. Where they do not, the
/// substitute's advances are not the ones the equation was measured against,
/// and math is the one place where that is fatal rather than untidy: every
/// glyph carries an absolute `dx`, so there is no flow to absorb the
/// difference and the glyphs simply overlap. Measured on this repo's own
/// fonts, `∀` at 12pt in Latin Modern Math advances 7.992pt; a reader
/// substituting DejaVu draws it 12.000 wide, which puts the next glyph inside
/// it. Only the variant glyphs were being outlined, so every ordinary one —
/// which is nearly all of them — was exposed.
///
/// The assertion is on the drawn INK, not on the presence of a `<path>`: the
/// ink must fall inside the advance the document reserved, which is the
/// property a substituted face violates and a `<path>` cannot.
#[test]
fn an_ordinary_math_glyph_is_drawn_from_its_outline_not_left_to_the_readers_font() {
    use rustyfi_backend::FontMetrics;

    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../lib-rustyfi/dist/fonts/latinmodern-math.otf");
    let Ok(store) = rustyfi_pdf::TtfFontStore::load(&path, None, None) else {
        eprintln!("skipping: {} did not load", path.display());
        return;
    };
    let font = FontKey(0);
    let size = Length::pt(12.0);
    let advance = store
        .advance(font, '\u{2200}', size)
        .expect("the face cmaps U+2200 FOR ALL");

    let g = MathGlyph {
        text: "\u{2200}".to_string(),
        // The whole point: NO gid. Before this change that meant `<text>`.
        gid: None,
        dx: Length::ZERO,
        dy: Length::ZERO,
        info: HorzStringInfo {
            font,
            size,
            rising: Length::ZERO,
            color: Color::Gray(0.0),
        },
        width: advance,
        height: Length::pt(9.0),
        depth: Length::ZERO,
    };
    let math_box = PureHorzBox::Math {
        width: advance,
        height: Length::pt(9.0),
        depth: Length::ZERO,
        glyphs: vec![g],
        rules: vec![],
    };
    let html = rustyfi_html::render_html_reflow_ttf_with(
        Some(&[line(math_box)]),
        &geometry(),
        &store,
        &[],
        &DocExtras::default(),
        &[],
        &[],
    )
    .expect("reflow HTML rendering must succeed");

    let (d, transform) = math_path_and_transform(&html).expect("a <path> for the glyph");
    let s = parse_uniform_flip_scale(&transform)
        .unwrap_or_else(|| panic!("transform is not a `scale(s -s)` uniform flip: {transform:?}"));
    let (x_min, x_max) = path_x_extent(&d).expect("the path has coordinates");
    let ink = (x_max - x_min) * s;
    assert!(
        ink > 0.5 * advance.0 && ink <= advance.0,
        "the drawn ink ({ink}pt) is not this glyph's own, measured against \
         the {}pt advance the document reserved for it",
        advance.0
    );
    // And the character is still there to be copied.
    assert!(
        html.contains("class=\"mphantom\"") && html.contains('\u{2200}'),
        "the character was dropped along with the <text>:\n{html}"
    );
}

/// A stretchy delimiter grown from a `GlyphAssembly` is several glyph records
/// stacked in ONE column — `push_delimiter_glyph` gives every part the same
/// `text` and the same `dx` — and its character must be copied ONCE.
///
/// Without the guard the clipboard gets `(((` for a bracket the page draws
/// once, which is worse than the `<text>` behaviour it replaced. The parts
/// are hand-built here because the port only reaches
/// `push_delimiter_glyph` on the paren-closure fallback path, so no fixture
/// document produces the shape reliably — and the point of the guard is that
/// it holds whenever the shape arrives, not that today's corpus makes one.
#[test]
fn a_stacked_delimiter_column_copies_its_character_once_not_once_per_part() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../lib-rustyfi/dist/fonts/latinmodern-math.otf");
    let Ok(store) = rustyfi_pdf::TtfFontStore::load(&path, None, None) else {
        eprintln!("skipping: {} did not load", path.display());
        return;
    };
    let part = |dy: f64, width: f64| MathGlyph {
        text: "(".to_string(),
        gid: None,
        dx: Length::ZERO,
        dy: Length::pt(dy),
        info: HorzStringInfo {
            font: FontKey(0),
            size: Length::pt(12.0),
            rising: Length::ZERO,
            color: Color::Gray(0.0),
        },
        // Only the first part carries the column's width, exactly as
        // `push_delimiter_glyph` builds it.
        width: Length::pt(width),
        height: Length::pt(6.0),
        depth: Length::ZERO,
    };
    let math_box = PureHorzBox::Math {
        width: Length::pt(4.0),
        height: Length::pt(14.0),
        depth: Length::pt(4.0),
        glyphs: vec![part(0.0, 4.0), part(6.0, 0.0), part(12.0, 0.0)],
        rules: vec![],
    };
    let html = rustyfi_html::render_html_reflow_ttf_with(
        Some(&[line(math_box)]),
        &geometry(),
        &store,
        &[],
        &DocExtras::default(),
        &[],
        &[],
    )
    .expect("reflow HTML rendering must succeed");

    // Three parts are DRAWN…
    assert_eq!(
        html.matches("<path ").count(),
        3,
        "each part of the column should still be painted:\n{html}"
    );
    // …and one character is copied. Counted inside the phantom layer, since
    // the stylesheet is full of unrelated `(`s.
    assert_eq!(
        html.matches("<tspan").count(),
        1,
        "the delimiter's character is repeated once per assembly part, so a \
         reader copying one bracket gets several:\n{html}"
    );
    assert!(html.contains(">(</tspan>"), "the bracket was not copied at all");
}

/// A `MathGlyph` whose `text` is several characters — what
/// `primitives::math_boxes_of_inline_boxes` produces for a `text-in-math`
/// body, folding a whole run into one record — is outlined character by
/// character, with the pen advancing by each glyph's own `hmtx` advance.
///
/// That reproduces the port's own measurement rather than approximating it:
/// `measure_run` is purely additive per character with no kerning or
/// ligatures, and `FontMetrics::advance` is the same `hmtx / units_per_em`
/// ratio. The assertion is on the SECOND character's translate, which is
/// exactly the first character's advance and nothing else.
#[test]
fn a_multi_character_math_run_advances_the_pen_by_each_glyphs_own_advance() {
    use rustyfi_backend::FontMetrics;

    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../lib-rustyfi/dist/fonts/latinmodern-math.otf");
    let Ok(store) = rustyfi_pdf::TtfFontStore::load(&path, None, None) else {
        eprintln!("skipping: {} did not load", path.display());
        return;
    };
    let font = FontKey(0);
    let size = Length::pt(12.0);
    let (a, b) = ('A', 'V');
    let wa = store.advance(font, a, size).expect("cmaps A");
    let total = wa + store.advance(font, b, size).expect("cmaps V");

    let math_box = PureHorzBox::Math {
        width: total,
        height: Length::pt(9.0),
        depth: Length::ZERO,
        glyphs: vec![MathGlyph {
            text: format!("{a}{b}"),
            gid: None,
            dx: Length::pt(3.0),
            dy: Length::ZERO,
            info: HorzStringInfo {
                font,
                size,
                rising: Length::ZERO,
                color: Color::Gray(0.0),
            },
            width: total,
            height: Length::pt(9.0),
            depth: Length::ZERO,
        }],
        rules: vec![],
    };
    let html = rustyfi_html::render_html_reflow_ttf_with(
        Some(&[line(math_box)]),
        &geometry(),
        &store,
        &[],
        &DocExtras::default(),
        &[],
        &[],
    )
    .expect("reflow HTML rendering must succeed");

    let xs: Vec<f64> = html
        .match_indices("transform=\"translate(")
        .filter_map(|(i, _)| {
            let rest = &html[i + "transform=\"translate(".len()..];
            rest.split_whitespace().next()?.parse().ok()
        })
        .collect();
    assert_eq!(xs.len(), 2, "expected one <path> per character:\n{html}");
    assert!(
        (xs[0] - 3.0).abs() < 1e-9,
        "the first character should sit at the record's own dx, got {}",
        xs[0]
    );
    assert!(
        (xs[1] - (3.0 + wa.0)).abs() < 1e-9,
        "the second character should sit one `A`-advance ({}pt) further on, \
         got {}",
        wa.0,
        xs[1] - 3.0,
    );
    // Both characters are copied, from ONE phantom text element.
    assert_eq!(html.matches("class=\"mphantom\"").count(), 1);
    assert!(html.contains(">AV</tspan>"), "the run's text is not intact");
}

/// The first `<path …/>` tag inside a `math-glyphs` `<svg>`.
fn math_path_tag(html: &str) -> Option<String> {
    let svg = html.split("class=\"math-glyphs\"").nth(1)?;
    let svg = svg.split("</svg>").next()?;
    Some(svg.split("<path ").nth(1)?.split("/>").next()?.to_string())
}

/// That tag's `d` and `transform`.
fn math_path_and_transform(html: &str) -> Option<(String, String)> {
    let tag = math_path_tag(html)?;
    let field = |name: &str| -> Option<String> {
        Some(
            tag.split(&format!("{name}=\""))
                .nth(1)?
                .split('"')
                .next()?
                .to_string(),
        )
    };
    Some((field("d")?, field("transform")?))
}

/// `translate(a b) scale(s -s)` -> `Some(s)`, and `None` if the scale is not
/// a uniform y-flip.
fn parse_uniform_flip_scale(transform: &str) -> Option<f64> {
    let inside = transform.split("scale(").nth(1)?.split(')').next()?;
    let mut parts = inside.split_whitespace();
    let sx: f64 = parts.next()?.parse().ok()?;
    let sy: f64 = parts.next()?.parse().ok()?;
    ((sx + sy).abs() < 1e-12 && sx > 0.0).then_some(sx)
}

/// The x-extent of an SVG `d`, read off every coordinate PAIR in it. Good
/// enough for a bound: a Bezier stays inside its control hull, so the
/// control points can only over-state the extent, never under-state it.
fn path_x_extent(d: &str) -> Option<(f64, f64)> {
    let nums: Vec<f64> = d
        .split_whitespace()
        .map(|t| t.trim_start_matches(['M', 'L', 'Q', 'C', 'Z']))
        .filter_map(|t| t.parse::<f64>().ok())
        .collect();
    let xs: Vec<f64> = nums.iter().step_by(2).copied().collect();
    let lo = xs.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = xs.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    (lo.is_finite() && hi.is_finite()).then_some((lo, hi))
}

/// Every length written INSIDE an `<svg>` must be in the viewport's own
/// user units, i.e. carry no unit at all — or, where CSS forces one
/// (`font-size`, which is a declaration rather than an attribute), the `px`
/// that SVG defines as exactly one user unit.
///
/// The generalisation of `math_glyph_font_size_is_in_svg_user_units_
/// not_points`. That test pins the one emitter that got it wrong; this one
/// pins the RULE, over every emitter at once — `<text>`'s `x`/`y` and
/// `font-size`, and `svg.rs`'s `d`, `stroke-width`, `stroke-dasharray` and
/// `stroke-dashoffset` — so a new length written into SVG content with a
/// `pt` on it fails here even if nobody thinks to extend the other test.
/// The `<svg>` OPENING tag is deliberately exempt: its `width`/`height` and
/// its CSS `left`/`top` live in the PARENT's coordinate space, where `pt`
/// is exactly right and is what makes one user unit come out as one point.
#[test]
fn no_absolute_length_unit_appears_inside_an_svg_body() {
    let dashed = GraphicsElem::DashedStroke(
        Length::pt(0.8),
        (Length::pt(3.0), Length::pt(2.0), Length::pt(1.0)),
        Color::Rgb(0.0, 0.0, 1.0),
        Path {
            subpaths: vec![Subpath {
                start: (Length::pt(0.0), Length::pt(10.0)),
                segs: vec![PathSeg::Line((Length::pt(20.0), Length::pt(0.0)))],
                closing: Closing::Open,
            }],
        },
    );
    let clipped = GraphicsElem::Clip(
        Path {
            subpaths: vec![Subpath {
                start: (Length::pt(0.0), Length::pt(0.0)),
                segs: vec![PathSeg::Line((Length::pt(20.0), Length::pt(20.0)))],
                closing: Closing::Line,
            }],
        },
        vec![GraphicsElem::Group(vec![rule_line(0.0, 0.0, 20.0, 10.0, 1.5)])],
    );
    let math_box = PureHorzBox::Math {
        width: Length::pt(20.0),
        height: Length::pt(14.0),
        depth: Length::pt(4.0),
        glyphs: vec![glyph("x", 12.0, 0.0, 0.0)],
        rules: vec![dashed.clone(), clipped.clone()],
    };
    let gfx_box = PureHorzBox::Graphics {
        origin_independent: false,
        width: Length::pt(20.0),
        height: Length::pt(20.0),
        depth: Length::ZERO,
        elems: vec![dashed, clipped],
    };
    let html = render(&[line(math_box), line(gfx_box)]);
    let bodies = svg_bodies(&html);
    assert!(!bodies.is_empty(), "no <svg> emitted at all:\n{html}");
    for body in bodies {
        if let Some((num, unit)) = first_absolute_length(body) {
            panic!(
                "`{num}{unit}` inside an <svg> body: the viewBox makes one \
                 user unit one point, so an absolute unit is resolved \
                 against the CSS reference pixel FIRST and comes out {}/1 \
                 too big:\n{body}",
                if unit == "pt" { "4/3" } else { "some other ratio" },
            );
        }
    }
}

/// One positioned math glyph: `text` at box-local `(dx, dy)`, set at
/// `size` pt.
fn glyph(text: &str, size: f64, dx: f64, dy: f64) -> MathGlyph {
    MathGlyph {
        text: text.to_string(),
        gid: None,
        dx: Length::pt(dx),
        dy: Length::pt(dy),
        info: HorzStringInfo {
            font: FontKey(0),
            size: Length::pt(size),
            rising: Length::ZERO,
            color: Color::Gray(0.0),
        },
        width: Length::pt(size * 0.6),
        height: Length::pt(size * 0.7),
        depth: Length::ZERO,
    }
}

/// The `<svg>` bodies of `html`, i.e. what lies between each `<svg …>` and
/// its `</svg>`. These never nest (`emit_graphics` is the only emitter and
/// it never re-enters itself), so a flat scan is exact.
fn svg_bodies(html: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = html;
    while let Some(open) = rest.find("<svg") {
        let after_tag = rest[open..]
            .find('>')
            .map(|i| open + i + 1)
            .expect("unterminated <svg> tag");
        let close = rest[after_tag..]
            .find("</svg>")
            .map(|i| after_tag + i)
            .expect("unclosed <svg>");
        out.push(&rest[after_tag..close]);
        rest = &rest[close + "</svg>".len()..];
    }
    out
}

/// The first `<digit><unit>` in `s` for any absolute or font-relative CSS
/// unit other than `px`, or `None`. Scans for the UNIT and looks left, so a
/// unit inside a word (`Modern`, `points`) is rejected by the digit test.
fn first_absolute_length(s: &str) -> Option<(char, &'static str)> {
    const UNITS: [&str; 11] = [
        "pt", "pc", "in", "mm", "cm", "rem", "em", "ex", "ch", "vw", "vh",
    ];
    let bytes = s.as_bytes();
    for (i, _) in s.char_indices() {
        for unit in UNITS {
            if !s[i..].starts_with(unit) {
                continue;
            }
            let before = bytes[..i].last().copied();
            let after = bytes.get(i + unit.len()).copied();
            let digit_before = before.is_some_and(|b| b.is_ascii_digit());
            let word_after = after.is_some_and(|b| b.is_ascii_alphanumeric());
            if digit_before && !word_after {
                return Some((before.unwrap() as char, unit));
            }
        }
    }
    None
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

/// The invariant that defines this backend: no RUN, PARAGRAPH, FRAME or
/// TABLE is placed at a coordinate — the reader's browser lays the document
/// out, so `left:` is never emitted for flowing content and every occurrence
/// of the substring `top:` is part of `margin-top:` or another flow-safe
/// longhand, never a bare CSS `top`.
///
/// The two exceptions are both DRAWING-scoped and both live in the
/// stylesheet, where the assertion below counts them: see the comment on
/// that count. This fixture is deliberately made of plain lines and a frame,
/// with no math or graphics box in it, so its BODY exercises the strict form
/// of the rule.
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
    // The STYLESHEET has exactly TWO absolute rules, and NEITHER is page
    // positioning — both are scoped to one relatively-positioned inline or
    // block box, the same licence the inline `svg`/math wrappers already
    // have (see this module's doc comment):
    //
    // - `svg.frame-deco`, a framed block's decoration stretched over its own
    //   box;
    // - `.dtx`, one row of a `draw-text` construction placed inside its own
    //   math/graphics wrapper. It was ONE until `\overset`/`\underset` and
    //   every big operator carrying limits were found rendering their rows
    //   side by side in source order — `\underset{m}{Y}` as `Y m` — because
    //   flow has no way to say "above" and the wrapper-local coordinates the
    //   SVG walker computes for each row were being discarded. See
    //   `inline.rs`'s `emit_placed_text`, and `all_nested_text_at_anchor`
    //   for the (unchanged) case where flow IS the right answer.
    //
    // Pinned by count so a THIRD one cannot arrive unnoticed.
    let sheet = html.split("<style>").nth(1).expect("a stylesheet");
    let sheet = sheet.split("</style>").next().unwrap();
    assert_eq!(
        sheet.matches("position: absolute").count() + sheet.matches("position:absolute").count(),
        2,
        "unexpected absolute positioning in the stylesheet:\n{sheet}"
    );
    for rule in ["svg.frame-deco", ".dtx {"] {
        assert!(
            sheet.contains(rule),
            "the absolute rules should be `{rule}` and the other one:\n{sheet}"
        );
    }
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
    // ONE `<a>`, spanning the whole region — the embedded block is a word of
    // this sentence (it sits between `linked ` and `tail`, inside a frame
    // marker pair) and so is emitted inline, which leaves the link intact.
    // It used to be block-level, and the link was therefore closed before it
    // and re-opened after: two `<a>`s, correct but worse. What the depth
    // floor still guards is the FOOTNOTE's body, which really is a nested
    // `walk_vboxes` run while this link is on the wrapper stack — without it
    // that body's own paragraph flush emitted a spurious `</a>` inside
    // itself.
    assert_eq!(
        body.matches("<a class=\"link\"").count(),
        1,
        "an inline embedded block must not split the link around it:\n{body}"
    );
    let aside = body.split("<aside").nth(1).expect("the footnote body");
    assert_eq!(
        (aside.matches("<a ").count(), aside.matches("</a>").count()),
        (1, 1),
        "the footnote body should hold exactly its own back-link — an extra \
         `</a>` there is the enclosing paragraph's wrapper being closed a \
         second time:\n{body}"
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

/// A wrapper that carries a `draw-text` run's HTML states its reserved
/// size as a MINIMUM, not as a fixed `width`/`height`.
///
/// The wrapper is an `inline-block`. A fixed height is right while its only
/// children are the absolutely-positioned `<svg>`s, but a `draw-text` run's
/// boxes cannot go inside the `<svg>` (they eject the rest of the drawing
/// from it, see `svg.rs`), so they land in the wrapper as FLOW content —
/// and flow content does not make a fixed-size inline-block grow, it
/// overflows, painting over the lines above and below. Measured on
/// `latexcmds`, where the `∑` of a `\sum` arrives as a nested run rather
/// than as a `MathGlyph`: 6.1pt of ink below a 10.4pt box, into the next
/// line. Both wrappers, since both had it.
///
/// The control below is the point: a wrapper with NO nested content keeps
/// the fixed size, because there the reservation is exact and letting the
/// box grow would let a stray line box change the layout.
#[test]
fn a_wrapper_carrying_draw_text_reserves_a_minimum_size_not_a_fixed_one() {
    let math_box = PureHorzBox::Math {
        width: Length::pt(20.0),
        height: Length::pt(8.0),
        depth: Length::pt(2.0),
        glyphs: vec![glyph("\u{22ef}", 12.0, 0.0, 0.0)],
        rules: vec![draw_text(text_run("BIGSIGMA"))],
    };
    let gfx_box = PureHorzBox::Graphics {
        origin_independent: false,
        width: Length::pt(20.0),
        height: Length::pt(20.0),
        depth: Length::ZERO,
        elems: vec![
            rule_line(0.0, 0.0, 20.0, 10.0, 1.0),
            draw_text(text_run("DRAWN")),
        ],
    };
    for (bx, cls, marker) in [(math_box, "math", "BIGSIGMA"), (gfx_box, "gfx", "DRAWN")] {
        let out = render(&[line(bx)]);
        let html = body_of(&out);
        assert!(html.contains(marker), "lost the nested content:\n{html}");
        let open = format!("<span class=\"{cls}\"");
        let wrapper = span_body(html, &open)
            .unwrap_or_else(|| panic!("no `{open}` wrapper emitted:\n{html}"));
        assert!(
            wrapper.contains(marker),
            "the nested `draw-text` run left its wrapper:\n{wrapper}"
        );
        let tag = &html[html.find(&open).unwrap()..][..html[html.find(&open).unwrap()..]
            .find('>')
            .unwrap()];
        assert!(
            tag.contains("min-height:") && tag.contains("min-width:"),
            "a `{cls}` wrapper carrying flow content must reserve a MINIMUM \
             size, or the content overflows it onto the neighbouring \
             lines:\n{tag}"
        );
        // And nowhere inside an <svg> either — an HTML child there makes the
        // parser close the <svg> and drops the rest of the drawing.
        for body in svg_bodies(html) {
            assert!(
                !body.contains(marker),
                "nested HTML inside an <svg>:\n{body}"
            );
        }
    }
    // The control: no nested content, so the reservation stays exact.
    let plain = PureHorzBox::Math {
        width: Length::pt(20.0),
        height: Length::pt(8.0),
        depth: Length::pt(2.0),
        glyphs: vec![glyph("x", 12.0, 0.0, 0.0)],
        rules: vec![],
    };
    let out = render(&[line(plain)]);
    assert!(
        out.contains("display:inline-block; width:20pt; height:10pt;"),
        "a wrapper with no flow content must keep its exact size:\n{out}"
    );
}

/// A math box that draws nothing of its own — no glyphs, and every rule a
/// `draw-text` — must emit no wrapper at all, exactly as
/// `a_text_only_graphics_box_emits_no_sized_wrapper` requires of a graphics
/// box. `latexcmds` builds two `\paren`-style decorations this way, and
/// each one reserved a blank rectangle the full size of the equation and
/// then painted the equation's real content on top of the following lines.
#[test]
fn a_text_only_math_box_emits_no_sized_wrapper() {
    let math_box = PureHorzBox::Math {
        width: Length::pt(60.0),
        height: Length::pt(30.0),
        depth: Length::pt(8.0),
        glyphs: vec![],
        rules: vec![draw_text(text_run("only drawn text"))],
    };
    let out = render(&[line(math_box)]);
    let html = body_of(&out);
    assert!(
        html.contains("only drawn text"),
        "lost the content:\n{html}"
    );
    assert!(
        !html.contains("class=\"math\""),
        "emitted a wrapper for a math box that draws nothing:\n{html}"
    );
    // The control: one real glyph and the wrapper comes back.
    let drawn = PureHorzBox::Math {
        width: Length::pt(60.0),
        height: Length::pt(30.0),
        depth: Length::pt(8.0),
        glyphs: vec![glyph("x", 12.0, 0.0, 0.0)],
        rules: vec![draw_text(text_run("only drawn text"))],
    };
    assert!(
        render(&[line(drawn)]).contains("class=\"math\""),
        "a math box with a real glyph lost its wrapper"
    );
}

/// A `draw-text` run anchored anywhere BUT the box's own origin is the
/// document PLACING content, and the placement is honoured
/// (`inline.rs`'s `emit_placed_text`).
///
/// This is how every stacked math construction in the corpus is built:
/// `latexcmds`' `\overset`/`\underset`/`\normal-overset` and each big
/// operator carrying limits is one `inline-graphics` holding two or three
/// `draw-text`s, one per row, differing only in their point. Rendered in
/// flow they came out side by side in source order — `\underset{m}{Y}` as
/// `Y m`, `\normal-overset{TOP}{BASE}` as `BASETOP` — while the SVG walker
/// was computing the right wrapper-local coordinates for each row and the
/// backend discarded them.
///
/// The three things that have to hold, and each of which was wrong:
/// 1. every row is placed, not appended;
/// 2. the ACCENT row lands above the BASE row (the bug was that "above" had
///    no expression in flow at all, so it became "after");
/// 3. `top` is the row's BASELINE minus that row's OWN ascent — the strut
///    carries the ascent, which is what makes `top` independent of whatever
///    font the reader's browser resolves the text to.
#[test]
fn an_off_anchor_draw_text_row_is_placed_above_the_row_it_accents() {
    // `\normal-overset`-shaped: a base row on the box's baseline and an
    // accent row 13pt above it, in a 24pt-tall box. `text_run` is 9pt tall.
    let gfx = PureHorzBox::Graphics {
        origin_independent: false,
        width: Length::pt(80.0),
        height: Length::pt(24.0),
        depth: Length::pt(2.0),
        elems: vec![
            draw_text_at(0.0, 0.0, text_run("BASE")),
            draw_text_at(4.0, 13.0, text_run("ACCENT")),
        ],
    };
    let out = render(&[line(gfx)]);
    let html = body_of(&out);

    // (1) Both rows placed, inside the wrapper that scopes the placement.
    let wrapper = span_body(html, "<span class=\"gfx\"")
        .unwrap_or_else(|| panic!("no graphics wrapper emitted:\n{html}"));
    assert_eq!(
        wrapper.matches("class=\"dtx\"").count(),
        2,
        "both `draw-text` rows must be placed:\n{wrapper}"
    );

    // (3) `top = baseline - ascent`, per row. The base row's baseline is the
    // box's own (24pt down from the wrapper's top), the accent row's is 13pt
    // above that; both runs are 9pt tall.
    assert!(
        wrapper.contains("<span class=\"dtx\" style=\"left:0pt; top:15pt;\">"),
        "the base row is at (0, 24-9):\n{wrapper}"
    );
    assert!(
        wrapper.contains("<span class=\"dtx\" style=\"left:4pt; top:2pt;\">"),
        "the accent row is at (4, 24-13-9):\n{wrapper}"
    );
    assert!(
        wrapper.contains("<span class=\"dtx-strut\" style=\"height:9pt;\"></span>"),
        "each placed row needs its own ascent as a strut, or `top` depends \
         on the reader's font metrics:\n{wrapper}"
    );

    // (2) And so the accent really is ABOVE, not after: source order still
    // puts BASE first, which is exactly why flow got this wrong.
    let base = wrapper.find("BASE").expect("lost the base row");
    let accent = wrapper.find("ACCENT").expect("lost the accent row");
    assert!(
        base < accent,
        "source order should be unchanged:\n{wrapper}"
    );
    assert!(
        wrapper.find("top:2pt").unwrap() > wrapper.find("top:15pt").unwrap(),
        "the LATER row in source order is the one placed higher:\n{wrapper}"
    );

    // Still nowhere inside the `<svg>` — an HTML child there closes it and
    // ejects the rest of the drawing (`svg.rs`).
    for body in svg_bodies(html) {
        assert!(
            !body.contains("class=\"dtx\""),
            "placed row inside an <svg>:\n{body}"
        );
    }
}

/// The control for the test above, and the reason it is keyed on the POINT
/// rather than on "is this a `draw-text`": a run at the box's OWN origin is
/// a package WRAPPING content it has already laid out, not positioning it —
/// `easytable` overlays a table and its rules with two `draw-text (x, y)` at
/// the callback's own point, `figbox` and `enumitem` wrap a single one. For
/// those, in flow is both where the content belongs and the only rendering
/// that still reflows, so nothing about them may change.
#[test]
fn a_draw_text_run_at_the_boxs_own_origin_stays_in_flow() {
    let gfx = PureHorzBox::Graphics {
        origin_independent: false,
        width: Length::pt(80.0),
        height: Length::pt(24.0),
        depth: Length::pt(2.0),
        elems: vec![
            draw_text_at(0.0, 0.0, text_run("RULES")),
            draw_text_at(0.0, 0.0, text_run("TABLE")),
        ],
    };
    let out = render(&[line(gfx)]);
    let html = body_of(&out);
    assert!(
        !html.contains("class=\"dtx\""),
        "an at-origin run must not be placed — it would stop reflowing:\n{html}"
    );
    for marker in ["RULES", "TABLE"] {
        assert!(html.contains(marker), "lost {marker}:\n{html}");
    }
    // …and, this being a box that draws nothing else, with no sized wrapper
    // around it at all (`a_text_only_graphics_box_emits_no_sized_wrapper`).
    assert!(
        !html.contains("class=\"gfx\""),
        "an all-at-origin text-only box must emit no wrapper:\n{html}"
    );
}

/// The contents of the first `<span …>` in `html` whose opening tag starts
/// with `open`, matching nested `<span>`s so an inner wrapper does not end
/// the outer one early.
fn span_body<'a>(html: &'a str, open: &str) -> Option<&'a str> {
    let start = html.find(open)?;
    let body_at = start + html[start..].find('>')? + 1;
    let mut depth = 1usize;
    let mut i = body_at;
    while depth > 0 {
        let next_open = html[i..].find("<span").map(|d| i + d);
        let next_close = html[i..].find("</span>").map(|d| i + d)?;
        match next_open {
            Some(o) if o < next_close => {
                depth += 1;
                i = o + "<span".len();
            }
            _ => {
                depth -= 1;
                if depth == 0 {
                    return Some(&html[body_at..next_close]);
                }
                i = next_close + "</span>".len();
            }
        }
    }
    None
}

/// A block composed into a DRAWING keeps its text.
///
/// **The shape.** `figbox`'s `frame`, `bgcolor`, `shift`, `rotate`, `scale`
/// and `graffiti` each wrap their argument in
/// `inline-graphics (fun (x, y) -> [draw-text (x, y) ib; …])`, so
/// `textbox-with-width 100pt {…} |> frame 1pt Color.black` — a paragraph
/// broken to a stated measure with a rule round it, the most ordinary thing
/// that package does — arrives as a `PureHorzBox::EmbeddedBlock` inside a
/// `draw-text`, i.e. at `inline::emit_inline` rather than at `block.rs`'s own
/// per-`Line` loop.
///
/// **The bug.** That arm was empty, with a comment asserting it was
/// "unreachable in practice". It was reachable, and the whole paragraph
/// silently disappeared: the page kept the frame, correctly sized, with
/// nothing inside it. Nothing failed and nothing warned.
///
/// The assertion is that the text is THERE and inside the drawing's own
/// wrapper — not merely somewhere on the page, which a fix that deferred the
/// block to after the paragraph (the way a footnote is deferred) would also
/// satisfy while moving the caption out of its own figure.
#[test]
fn an_embedded_block_inside_a_draw_text_keeps_its_text() {
    let inner = PureHorzBox::EmbeddedBlock {
        width: Length::pt(100.0),
        height: Length::pt(20.0),
        depth: Length::pt(3.0),
        block: vec![text_line("INSIDE THE FRAME")],
        anchor_last: true,
        breakable: false,
    };
    let gfx = PureHorzBox::Graphics {
        origin_independent: false,
        width: Length::pt(100.0),
        height: Length::pt(20.0),
        depth: Length::pt(3.0),
        elems: vec![
            draw_text(inner),
            rule_line(0.0, 0.0, 100.0, 0.0, 1.0),
        ],
    };
    let html = render(&[line(gfx)]);
    assert!(
        html.contains("INSIDETHEFRAME") || html.contains("INSIDE THE FRAME"),
        "the embedded block's text was dropped — the frame is drawn empty:\n{html}"
    );
    let wrapper = span_body(&html, "<span class=\"gfx\"").expect("a graphics wrapper");
    assert!(
        wrapper.contains("INSIDE"),
        "the text landed outside the drawing it belongs to:\n{wrapper}"
    );
    assert!(
        wrapper.contains("class=\"embed-inline\" style=\"width:100pt; white-space:nowrap;\""),
        "the block's own measure is not kept, so the paragraph reflows to \
         the full column instead of the 100pt it was built for:\n{wrapper}"
    );
    // Inline markup only. Everything here is inside the enclosing
    // `<p class="para">`, and an HTML parser closes an open `<p>` at the
    // first block-level start tag — so a `<p>`/`<div>` in here would not
    // nest, it would terminate the paragraph and eject the rest of it.
    for tag in ["<p ", "<p>", "<div ", "<div>"] {
        assert!(
            !wrapper.contains(tag),
            "`{tag}` inside inline content closes the surrounding \
             paragraph:\n{wrapper}"
        );
    }
    // And it stays out of the `<svg>`, like every other nested run.
    for body in svg_bodies(&html) {
        assert!(
            !body.contains("INSIDE"),
            "HTML inside an <svg> ends the drawing at the first tag:\n{body}"
        );
    }
}

/// Two paragraphs inside one embedded block stay two, and two LINES of one
/// paragraph rejoin — the same distinction `block.rs` draws, because the
/// browser is going to re-break the text and the port's own line breaks must
/// not survive as hard ones.
#[test]
fn an_embedded_block_in_a_drawing_rejoins_lines_but_keeps_paragraphs() {
    let inner = PureHorzBox::EmbeddedBlock {
        width: Length::pt(100.0),
        height: Length::pt(40.0),
        depth: Length::ZERO,
        block: vec![
            text_line("alpha"),
            text_line("beta"),
            VertBox::Skip(Length::pt(6.0)),
            text_line("gamma"),
        ],
        anchor_last: true,
        breakable: false,
    };
    let gfx = PureHorzBox::Graphics {
        origin_independent: false,
        width: Length::pt(100.0),
        height: Length::pt(40.0),
        depth: Length::ZERO,
        elems: vec![draw_text(inner)],
    };
    // A text-only box at its own anchor emits no sized wrapper of its own
    // (`a_text_only_graphics_box_emits_no_sized_wrapper`), so the embedded
    // block is the paragraph's own content here.
    let html = render(&[line(gfx)]);
    let embed = html
        .split("class=\"embed-inline\"")
        .nth(1)
        .expect("the inline-block wrapper");
    assert!(
        embed.contains("alpha beta"),
        "two lines of one paragraph must rejoin for the browser to \
         re-break:\n{embed}"
    );
    assert_eq!(
        embed.matches("<br>").count(),
        1,
        "exactly one paragraph boundary, and it is the `Skip`:\n{embed}"
    );
    assert!(
        embed.find("<br>") < embed.find("gamma"),
        "the break belongs before the second paragraph:\n{embed}"
    );
}

fn draw_text(bx: PureHorzBox) -> GraphicsElem {
    draw_text_at(0.0, 0.0, bx)
}

/// [`draw_text`] anchored at a box-local, y-UP point — the frame
/// `GraphicsElem`'s own coordinates use (`graphics.rs`).
fn draw_text_at(x: f64, y: f64, bx: PureHorzBox) -> GraphicsElem {
    GraphicsElem::Text {
        pt: (Length::pt(x), Length::pt(y)),
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
            // A BLOCK frame: no baseline to be measured against, which is also
            // what keeps it out of the inline arm — see
            // `an_inline_frames_decoration_is_painted_on_its_own_wrapper`.
            depth: None,
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
        rustyfi_html::MathMode::SvgOutline,
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

/// An INLINE frame's decoration lands on the wrapper `<span>` as a background,
/// not on a `<div>` as a stretched `<svg>` — and the two are told apart by
/// `FrameDecoration::depth` alone, which is the only thing in the recording
/// that says a baseline was involved.
///
/// The end-to-end statement of this is `rustyfi/tests/html_inline_frame_deco.rs`
/// (it has to be end to end: the recording side is where the bug was). This is
/// the unit-level pin on the discriminator, which that test cannot isolate —
/// it drives the same table with the block entry the block arm expects.
#[test]
fn an_inline_frames_decoration_is_painted_on_its_own_wrapper() {
    let deco = DecoId(21);
    // 11pt tall, baseline 2pt up from the bottom (the marker's own metrics),
    // with a rule stroked 1pt BELOW that baseline — `\uwave`'s shape.
    let decos = vec![(
        deco,
        rustyfi_backend::FrameDecoration {
            width: Length::pt(60.0),
            height: Length::pt(11.0),
            pads: (Length::ZERO, Length::ZERO, Length::ZERO, Length::ZERO),
            depth: Some(Length::pt(2.0)),
            elems: vec![rule_line(0.0, 1.0, 60.0, 1.0, 0.5)],
        },
    )];
    let vboxes = vec![line_of(vec![
        iframe_marker(21, false),
        text_run("wavy"),
        iframe_marker(21, true),
    ])];
    let out = rustyfi_html::render_html_reflow_with_decos(
        Some(&vboxes),
        &geometry(),
        &[],
        &DocExtras::default(),
        &[],
        &[],
        &decos,
        rustyfi_html::MathMode::SvgOutline,
    )
    .expect("reflow HTML rendering must succeed");

    assert!(
        out.contains(".ideco-0 { background-image:url(\"data:image/svg+xml,"),
        "the inline decoration reached no stylesheet rule:\n{out}"
    );
    assert!(
        body_of(&out).contains("<span class=\"iframe ideco ideco-0\">wavy</span>"),
        "the wrapper does not wear its decoration:\n{}",
        body_of(&out)
    );
    // Ink below the baseline: tiled, bottom-anchored. NOT the block arm's
    // `<svg class="frame-deco">`, which would need a `<div>` to stretch over.
    assert!(
        out.contains("background-repeat:repeat-x;")
            && out.contains("background-position:left bottom;"),
        "wrong placement class for a rule below the baseline:\n{out}"
    );
    // `.frame.framed > svg.frame-deco` is static stylesheet furniture; what
    // must not exist is an ELEMENT wearing it.
    assert!(
        !body_of(&out).contains("frame-deco"),
        "an inline decoration must not go through the block arm:\n{}",
        body_of(&out)
    );
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
            depth: None,
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
        rustyfi_html::MathMode::SvgOutline,
    )
    .expect("reflow HTML rendering must succeed");
    let html = body_of(&out);
    assert!(html.contains("background:"), "no background panel:\n{html}");
    assert!(
        !html.contains("frame-deco"),
        "a flat panel should need no SVG:\n{html}"
    );
}

/// An embedded block used as a WORD stays in its sentence.
///
/// `block.rs` treated every `EmbeddedBlock` as block-level: it flushed the
/// paragraph, opened a `<div class="embed">` and resumed afterwards. That is
/// right for the lone-box line a centred figure arrives as
/// (`embedded_block_becomes_a_nested_div_recursively`, above) and wrong for
/// `latexcmds`' `\framebox`, which is a FIXED-WIDTH box in the middle of
/// running text (`\fbox{\makebox(wid){…}}`) — there it took the rest of the
/// sentence out of the line with it.
///
/// The two cases are one line apart in the box stream, so they are asserted
/// against each other here: same box, once alone on its line and once between
/// two words.
#[test]
fn an_embedded_block_between_words_is_inline_not_a_div() {
    let embed = PureHorzBox::EmbeddedBlock {
        breakable: false,
        width: Length::pt(100.0),
        height: Length::pt(20.0),
        depth: Length::ZERO,
        block: vec![text_line("in the box")],
        anchor_last: false,
    };
    let vboxes = vec![VertBox::Line {
        height: Length::pt(9.0),
        depth: Length::pt(2.0),
        leading: Length::pt(12.0),
        contents: vec![
            (Length::ZERO, text_run("before")),
            (Length::pt(40.0), embed),
            (Length::pt(150.0), text_run("after")),
        ],
    }];
    let out = render(&vboxes);
    let html = body_of(&out);

    assert!(
        !html.contains("<div class=\"embed\""),
        "an embedded block between two words must not become block-level:\n{html}"
    );
    assert_eq!(
        html.matches("<p class=\"para\"").count(),
        1,
        "the sentence was split into several paragraphs:\n{html}"
    );
    let para = html.lines().find(|l| l.contains("before")).unwrap();
    assert!(
        para.contains("after") && para.contains("class=\"embed-inline\""),
        "`before`, the box and `after` belong to one paragraph:\n{para}"
    );
}

/// A ONE-LINE embedded block is `white-space: nowrap`, and inside it a
/// `Discretionary` writes no soft hyphen.
///
/// Both halves are the same fact: the port already fitted this text at this
/// width, so the browser must not re-break it — and a break opportunity in a
/// region that cannot break is not merely inert, it splits the word in the
/// SOURCE. Measured on `figbox`, where a framed caption came out as
/// `cap&shy;tion` and no search for `caption` found it, while the visible
/// rendering put `cap-` inside the frame and `tion` below it.
#[test]
fn a_one_line_embedded_block_neither_wraps_nor_offers_a_hyphen() {
    let embed = PureHorzBox::EmbeddedBlock {
        breakable: false,
        width: Length::pt(60.0),
        height: Length::pt(9.0),
        depth: Length::pt(2.0),
        block: vec![VertBox::Line {
            height: Length::pt(9.0),
            depth: Length::pt(2.0),
            leading: Length::pt(12.0),
            contents: vec![
                (Length::ZERO, text_run("left")),
                (
                    Length::pt(20.0),
                    PureHorzBox::Discretionary {
                        pre_break: vec![text_run("-")],
                        post_break: vec![],
                        no_break: vec![],
                        penalty: 0,
                    },
                ),
                (Length::pt(20.0), text_run("ward")),
            ],
        }],
        anchor_last: false,
    };
    // Between two words, so it is the INLINE embedded block under test rather
    // than the lone-box line that is legitimately a `<div>`.
    let out = render(&[
        line_of(vec![text_run("before"), embed, text_run("after")]),
        text_line("body text, and more of it, so the box is not the whole flow"),
    ]);
    let html = body_of(&out);

    assert!(
        html.contains("white-space:nowrap;"),
        "a one-line box must not be re-broken at the reader's metrics:\n{html}"
    );
    assert!(
        html.contains("leftward") && !html.contains("left&shy;ward"),
        "the word is split in the source by a hyphen nothing can use:\n{html}"
    );
}

/// The control: a MULTI-line embedded block keeps both.
///
/// There the declared width is the document's own wrapping instruction
/// (`textbox-with-width 100pt`), the browser re-breaking it is the point of
/// this backend, and the dictionary's break opportunity is real information
/// that only `&shy;` carries.
#[test]
fn a_multi_line_embedded_block_keeps_its_soft_hyphen() {
    let hyphen_line = VertBox::Line {
        height: Length::pt(9.0),
        depth: Length::pt(2.0),
        leading: Length::pt(12.0),
        contents: vec![
            (Length::ZERO, text_run("left")),
            (
                Length::pt(20.0),
                PureHorzBox::Discretionary {
                    pre_break: vec![text_run("-")],
                    post_break: vec![],
                    no_break: vec![],
                    penalty: 0,
                },
            ),
            (Length::pt(20.0), text_run("ward")),
        ],
    };
    let embed = PureHorzBox::EmbeddedBlock {
        breakable: false,
        width: Length::pt(60.0),
        height: Length::pt(9.0),
        depth: Length::pt(2.0),
        block: vec![hyphen_line, text_line("second line")],
        anchor_last: false,
    };
    let out = render(&[
        line_of(vec![text_run("before"), embed, text_run("after")]),
        text_line("body text, and more of it, so the box is not the whole flow"),
    ]);
    let html = body_of(&out);

    assert!(
        !html.contains("white-space:nowrap;"),
        "a box the document asked to be wrapped must stay breakable:\n{html}"
    );
    assert!(
        html.contains("left&shy;ward"),
        "the dictionary's break opportunity is the only thing that tells the \
         browser where it may hyphenate:\n{html}"
    );
}

// ---------------------------------------------------------------------------
// `--mathml`
// ---------------------------------------------------------------------------

/// One math glyph, with the metrics `mathrec`'s geometry is measured against.
fn ml_glyph(text: &str, dx: f64, dy: f64, size: f64) -> MathGlyph {
    MathGlyph {
        text: text.to_string(),
        gid: None,
        dx: Length::pt(dx),
        dy: Length::pt(dy),
        info: HorzStringInfo {
            font: FontKey(0),
            size: Length::pt(size),
            rising: Length::ZERO,
            color: Color::Gray(0.0),
        },
        width: Length::pt(size * 0.5),
        height: Length::pt(size * 0.7),
        depth: Length::ZERO,
    }
}

fn ml_math(glyphs: Vec<MathGlyph>) -> PureHorzBox {
    ml_math_with_rules(glyphs, Vec::new())
}

fn ml_math_with_rules(glyphs: Vec<MathGlyph>, rules: Vec<GraphicsElem>) -> PureHorzBox {
    PureHorzBox::Math {
        width: Length::pt(30.0),
        height: Length::pt(10.0),
        depth: Length::pt(2.0),
        glyphs,
        rules,
    }
}

/// A filled rectangle — ink the recovery cannot name.
fn ml_inked_rule() -> GraphicsElem {
    GraphicsElem::Fill(
        Color::Gray(0.0),
        Path {
            subpaths: vec![Subpath {
                start: (Length::pt(0.0), Length::pt(0.0)),
                segs: vec![
                    PathSeg::Line((Length::pt(10.0), Length::pt(0.0))),
                    PathSeg::Line((Length::pt(10.0), Length::pt(1.0))),
                    PathSeg::Line((Length::pt(0.0), Length::pt(1.0))),
                ],
                closing: Closing::Line,
            }],
        },
    )
}

/// The EXTENT MARKER a `Math::Radical` emits beside its two real fills: a
/// single point with no segments, which `primitives.rs` adds purely so the
/// headroom above the bar reaches the outer box through `graphics_bbox`. It
/// paints nothing — PDF's `f` on a zero-length path is a no-op.
fn ml_extent_marker() -> GraphicsElem {
    GraphicsElem::Fill(
        Color::Gray(0.0),
        Path {
            subpaths: vec![Subpath {
                start: (Length::pt(0.0), Length::pt(12.0)),
                segs: Vec::new(),
                closing: Closing::Open,
            }],
        },
    )
}

fn render_mathml(vboxes: &[VertBox]) -> String {
    rustyfi_html::render_html_reflow_with_decos(
        Some(vboxes),
        &geometry(),
        &[],
        &DocExtras::default(),
        &[],
        &[],
        &[],
        rustyfi_html::MathMode::MathMl,
    )
    .expect("reflow HTML rendering must succeed")
}

/// `--mathml` writes MathML Core into the page's own tree instead of drawing —
/// no `<svg>`, no phantom layer, no LaTeX for someone else's typesetter.
///
/// The equation that is the whole of its paragraph is `display="block"` and the
/// one with prose beside it is not. **Asserted as a contrast**, because each
/// half alone is satisfied by a renderer that never emits the other — the
/// mistake `sole_math_tex` shipped with and that `html_katex_uses_display_
/// delimiters_for_a_displayed_equation` was rewritten to catch.
#[test]
fn mathml_writes_core_elements_and_tells_display_from_inline() {
    let displayed = ml_math(vec![
        ml_glyph("x", 0.0, 0.0, 10.0),
        ml_glyph("2", 6.0, 5.0, 7.0),
    ]);
    let inline = ml_math(vec![ml_glyph("y", 0.0, 0.0, 10.0)]);
    // A `Skip` between them, or consecutive lines coalesce into ONE paragraph
    // and the displayed equation is no longer alone in its block.
    let doc = render_mathml(&[
        line(displayed),
        VertBox::Skip(Length::pt(12.0)),
        line_of(vec![text_run("see"), inline, text_run("then")]),
    ]);
    // Nesting is checked over the WHOLE document, which is what
    // `assert_balanced_tags` counts `<html>`/`<body>` in.
    assert_balanced_tags(&doc);
    let html = body_of(&doc).to_string();

    assert!(html.contains("<math "), "no MathML at all:\n{html}");
    assert!(
        html.contains("<msup><mi mathvariant=\"normal\">x</mi><mn>2</mn></msup>"),
        "the script did not become an <msup>:\n{html}"
    );
    // Not a drawing and not LaTeX: this mode replaces both.
    assert!(!html.contains("math-glyphs"), "{html}");
    assert!(!html.contains("mphantom"), "{html}");
    assert!(!html.contains("math-tex"), "{html}");

    assert!(
        html.contains("display=\"block\""),
        "the display upgrade never fired:\n{html}"
    );
    assert!(
        html.contains("display=\"inline\""),
        "no inline equation: the contrast is not being measured:\n{html}"
    );
    assert!(html.contains("class=\"para math-display\""), "{html}");
    assert_eq!(
        html.matches("<math ").count(),
        html.matches("</math>").count(),
        "unbalanced <math> elements:\n{html}"
    );
}

/// The `--mathml` stylesheet is emitted in that mode and in NO other, so
/// `--format html` without the flag is byte-identical to what it was.
///
/// The two centring declarations are the load-bearing part and are checked by
/// name: `math[display="block"]` computes to `display: block math`, a
/// block-level box, so the enclosing paragraph's `text-align: center` — which
/// is all `--katex`'s inline `\[…\]` needs — moves it not at all. Measured in
/// headless Chromium: without them every displayed equation sits flush left.
#[test]
fn the_mathml_stylesheet_is_scoped_to_the_mode() {
    let vboxes = vec![line(ml_math(vec![ml_glyph("x", 0.0, 0.0, 10.0)]))];
    let with_flag = render_mathml(&vboxes);
    assert!(with_flag.contains("margin-inline: auto"), "{with_flag}");
    assert!(with_flag.contains("width: fit-content"), "{with_flag}");
    assert!(with_flag.contains(".para.math-display"), "{with_flag}");

    let without = render(&vboxes);
    for rule in [
        "margin-inline: auto",
        "width: fit-content",
        ".para.math-display",
        "rustyfi-approx",
    ] {
        assert!(
            !without.contains(rule),
            "`{rule}` leaked into a render that did not ask for --mathml"
        );
    }
}

/// `rustyfi-approx` marks a run whose drawing the recovery could not account
/// for — and an EXTENT MARKER is not such a drawing.
///
/// Both directions, because each alone is worthless: a marker that fires on
/// everything says nothing, and one that never fires says nothing either. The
/// existing end-to-end coverage in `rustyfi/tests/math_modes.rs` asserts only
/// the negative, against a fixture that draws no rules at all — which cannot
/// distinguish "accounted for" from "never counted".
///
/// The extent-marker half is the one with history. A `Math::Radical` emits
/// THREE `Fill`s — the checkmark sign, the overbar, and a single-point subpath
/// carrying the extra ascender above the bar — and counting that third one as
/// ink marked every `\sqrt` equation approximate the moment the recovery
/// learned to read radicals. `mathrec::inked_paths` is named for what it
/// counts, so the fix belongs there rather than in an off-by-one at the call
/// site; this pins it.
#[test]
fn the_approx_mark_counts_ink_and_not_an_extent_marker() {
    let glyphs = vec![ml_glyph("x", 0.0, 0.0, 10.0)];

    let unaccounted = render_mathml(&[line(ml_math_with_rules(
        glyphs.clone(),
        vec![ml_inked_rule()],
    ))]);
    // The CLASS ATTRIBUTE, not the bare name: the stylesheet carries a
    // `.rustyfi-approx {}` rule in every `--mathml` render, so a substring
    // search for the name alone passes whatever the marker did.
    assert!(
        unaccounted.contains("class=\"math-ml rustyfi-approx\""),
        "a filled rectangle the recovery cannot name is exactly what the mark \
         is for, and it did not fire:\n{unaccounted}"
    );

    let marker_only =
        render_mathml(&[line(ml_math_with_rules(glyphs, vec![ml_extent_marker()]))]);
    assert!(
        !marker_only.contains("class=\"math-ml rustyfi-approx\""),
        "an extent marker paints nothing, so there is no unrecovered drawing \
         to warn about:\n{marker_only}"
    );
}
