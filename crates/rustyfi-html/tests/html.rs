//! Integration test for the Slice 1 HTML writer (`render_html_fixed`): a page
//! whose only content is an `InnerString` run serializes to a `<div
//! class="page">` containing an absolutely-positioned `<span>` at the run's
//! placed `(x, y)`, mirroring `tests/graphics.rs`'s content-stream substring
//! style for the PDF writer.

use rustyfi_backend::{
    Closing, Color, DecoId, DocExtras, FontKey, GraphicsElem, HorzStringInfo, ImageId,
    ImageResource, Length, MathGlyph, Page, PageGeometry, Path, PathSeg, PlacedLine, PureHorzBox,
    Subpath, TabularBox, TabularCellBox, VertBox,
};

fn geometry() -> PageGeometry {
    // Round numbers throughout (not `PageGeometry::default`'s mm-converted
    // values), so the expected `width:`/`height:`/`left:`/`top:` strings
    // below are exact integers, not float-formatting guesses.
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

fn page_with_run(bx: PureHorzBox) -> Page {
    Page {
        body_lines: usize::MAX,
        lines: vec![PlacedLine {
            x: Length::pt(50.0),
            baseline_y: Length::pt(100.0),
            contents: vec![(Length::ZERO, bx)],
        }],
    }
}

fn render(page: &Page) -> String {
    rustyfi_html::render_html_fixed(
        &geometry(),
        std::slice::from_ref(page),
        &[],
        &DocExtras::default(),
    )
    .expect("HTML rendering must succeed")
}

#[test]
fn text_run_renders_as_a_positioned_span_with_expected_text() {
    let page = page_with_run(text_run("Hello, world!"));
    let html = render(&page);

    assert!(
        html.starts_with("<!doctype html>"),
        "missing doctype:\n{html}"
    );
    assert!(
        html.contains("<div class=\"page\""),
        "missing page div:\n{html}"
    );
    assert!(html.contains("width:200pt"), "missing page width:\n{html}");
    assert!(
        html.contains("height:300pt"),
        "missing page height:\n{html}"
    );

    // left = line.x + dx = 50 + 0; top = baseline_y - rising - height =
    // 100 - 0 - 9 = 91 (no y-flip — SATySFi's page-down y already matches
    // CSS `top`, see `render_html_fixed`'s doc comment).
    assert!(html.contains("left:50pt"), "missing left offset:\n{html}");
    assert!(html.contains("top:91pt"), "missing top offset:\n{html}");
    assert!(
        html.contains("font-size:12pt"),
        "missing font-size:\n{html}"
    );
    assert!(
        html.contains("<span class=\"run\""),
        "missing run span:\n{html}"
    );
    assert!(html.contains("Hello, world!"), "missing run text:\n{html}");
}

#[test]
fn run_text_is_html_escaped() {
    let page = page_with_run(text_run("<a & \"b\">"));
    let html = render(&page);
    assert!(
        html.contains("&lt;a &amp; &quot;b&quot;&gt;"),
        "text was not HTML-escaped:\n{html}"
    );
    // The raw, unescaped text must not appear anywhere (it would either
    // break the markup or silently disappear as a bogus tag).
    assert!(
        !html.contains("<a & \"b\">"),
        "raw unescaped text leaked:\n{html}"
    );
}

#[test]
fn rising_shifts_the_run_up_the_page() {
    let mut bx = text_run("raised");
    if let PureHorzBox::InnerString { info, .. } = &mut bx {
        info.rising = Length::pt(3.0);
    }
    let page = page_with_run(bx);
    let html = render(&page);
    // top = baseline_y - rising - height = 100 - 3 - 9 = 88: a positive
    // rising moves the run UP the page, i.e. DECREASES the y-down `top`.
    assert!(
        html.contains("top:88pt"),
        "rising did not shift the span up:\n{html}"
    );
}

#[test]
fn glue_boxes_render_no_extra_span() {
    // `OuterEmpty`/`FixedEmpty` carry no visible content of their own —
    // only the one real `InnerString` run should produce a `<span>`.
    let page = Page {
        body_lines: usize::MAX,
        lines: vec![PlacedLine {
            x: Length::pt(50.0),
            baseline_y: Length::pt(100.0),
            contents: vec![
                (
                    Length::ZERO,
                    PureHorzBox::FixedEmpty {
                        width: Length::pt(5.0),
                    },
                ),
                (
                    Length::pt(5.0),
                    PureHorzBox::OuterEmpty {
                        natural: Length::pt(4.0),
                        shrinkable: Length::ZERO,
                        stretchable: Length::ZERO,
                    },
                ),
                (Length::pt(9.0), text_run("word")),
            ],
        }],
    };
    let html = render(&page);
    assert_eq!(
        html.matches("<span").count(),
        1,
        "glue boxes must not emit their own span:\n{html}"
    );
    assert!(
        html.contains("left:59pt"),
        "missing glue-adjusted left offset:\n{html}"
    );
}

#[test]
fn unhandled_box_variants_render_nothing() {
    // `Image`/`Graphics`/etc. are Slice 2/3 — Slice 1's wildcard arm must
    // skip them cleanly rather than panicking or emitting a stray span.
    let page = Page {
        body_lines: usize::MAX,
        lines: vec![PlacedLine {
            x: Length::ZERO,
            baseline_y: Length::pt(100.0),
            contents: vec![(Length::ZERO, PureHorzBox::OuterFil)],
        }],
    };
    let html = render(&page);
    assert!(
        !html.contains("<span"),
        "OuterFil must not render a span:\n{html}"
    );
    assert!(
        html.contains("<div class=\"page\""),
        "page div must still render:\n{html}"
    );
}

// ============================================================================
// Slice 2 (: "graphics (inline SVG)"): `Graphics` -> `<svg>`, plus the
// `Tabular`/`EmbeddedBlock`/`Frame` composite recursions. The graphics
// fixture mirrors `crates/rustyfi-pdf/tests/graphics.rs`'s
// `rectangle_path`/ `page_with_graphics_box` (same 20pt square, fill +
// stroke) so the two backends are tested against the exact same shape.
// ============================================================================

fn rectangle_path() -> Path {
    Path {
        subpaths: vec![Subpath {
            start: (Length::pt(0.0), Length::pt(0.0)),
            segs: vec![
                PathSeg::Line((Length::pt(20.0), Length::pt(0.0))),
                PathSeg::Line((Length::pt(20.0), Length::pt(20.0))),
                PathSeg::Line((Length::pt(0.0), Length::pt(20.0))),
            ],
            closing: Closing::Line,
        }],
    }
}

fn graphics_box() -> PureHorzBox {
    let path = rectangle_path();
    PureHorzBox::Graphics {
        origin_independent: false,
        width: Length::pt(20.0),
        height: Length::pt(20.0),
        depth: Length::pt(0.0),
        elems: vec![
            GraphicsElem::Fill(Color::Rgb(1.0, 0.0, 0.0), path.clone()),
            GraphicsElem::Stroke(Length::pt(1.0), Color::Gray(0.0), path),
        ],
    }
}

#[test]
fn graphics_box_renders_svg_path_with_fill_and_stroke() {
    let page = page_with_run(graphics_box());
    let html = render(&page);

    assert!(html.contains("<svg"), "missing <svg> element:\n{html}");
    // The <svg> is CSS-positioned at the box's top-left corner: left = line.x
    // + dx = 50; top = baseline_y - height = 100 - 20 = 80 (no y-flip at
    // this level — the per-box <g transform> below handles the y-up/y-down
    // reconciliation, see `svg.rs`'s module doc comment).
    assert!(
        html.contains("left:50pt"),
        "missing svg left offset:\n{html}"
    );
    assert!(html.contains("top:80pt"), "missing svg top offset:\n{html}");
    assert!(
        html.contains("viewBox=\"0 0 20 20\""),
        "missing matching viewBox:\n{html}"
    );
    assert!(
        html.contains("<g transform=\"translate(0,20) scale(1,-1)\">"),
        "missing the y-flip <g>:\n{html}"
    );

    // The fill path: even-odd rule, red, no stroke, tracing the rectangle
    // exactly like the PDF writer's `m`/`l`/`h` sequence in
    // `tests/graphics.rs` — the SVG analogue is `M`/`L`/`Z`.
    assert!(
        html.contains("d=\"M0 0 L20 0 L20 20 L0 20 Z\""),
        "missing rectangle path d attribute:\n{html}"
    );
    assert!(
        html.contains("fill=\"rgb(255,0,0)\""),
        "missing red fill:\n{html}"
    );
    assert!(
        html.contains("fill-rule=\"evenodd\""),
        "missing even-odd fill rule:\n{html}"
    );

    // The stroke path: black, 1pt wide, unfilled.
    assert!(
        html.contains("stroke=\"rgb(0,0,0)\""),
        "missing black stroke:\n{html}"
    );
    assert!(
        html.contains("stroke-width=\"1\""),
        "missing stroke width:\n{html}"
    );
}

/// A `draw-text` run's `<span>`s must be emitted AFTER the `</svg>`, never
/// inside it — and everything the drawing has left must still be inside.
///
/// `span` is on the HTML parser's foreign-content breakout list, so a
/// `<span>` written inside an `<svg>` closes the `<svg>` where it stands
/// and every element after it is parsed as ordinary HTML. That is not a
/// well-formedness nicety: 16 of `latexcmds`' 208 `--format html-fixed`
/// `<path>`s were landing outside their drawing and not rendering at all,
/// and the source looked perfectly good — it is valid XML. Nothing moves
/// as a result of the fix: these boxes carry page-absolute coordinates.
#[test]
fn a_draw_text_runs_spans_are_emitted_after_the_svg_not_inside_it() {
    let bx = PureHorzBox::Graphics {
        origin_independent: false,
        width: Length::pt(20.0),
        height: Length::pt(20.0),
        depth: Length::ZERO,
        elems: vec![
            GraphicsElem::Text {
                pt: (Length::pt(2.0), Length::pt(3.0)),
                contents: vec![(Length::ZERO, text_run("NESTED"))],
                width: Length::pt(20.0),
                height: Length::pt(20.0),
                depth: Length::ZERO,
                transform: None,
            },
            // Written AFTER the text, so a breakout would eject it.
            GraphicsElem::Fill(Color::Rgb(0.0, 1.0, 0.0), rectangle_path()),
        ],
    };
    let html = render(&page_with_run(bx));
    let open = html.find("<svg").expect("no <svg> emitted");
    let close = html.find("</svg>").expect("unclosed <svg>");
    let body = &html[open..close];
    assert!(
        !body.contains("NESTED"),
        "a draw-text run's HTML is inside the <svg>; the browser will \
         close the <svg> there and drop the rest of the drawing:\n{body}"
    );
    assert!(
        body.contains("fill=\"rgb(0,255,0)\""),
        "the element written after the draw-text left the <svg>:\n{body}"
    );
    assert!(
        html[close..].contains("NESTED"),
        "the draw-text run's content vanished entirely:\n{html}"
    );
}

#[test]
fn cmyk_fill_converts_to_rgb() {
    // Pure CMYK cyan (C=1, everything else 0) should drop the red channel
    // only, the same naive conversion `svg::css_color` uses (unit-tested
    // directly in `src/html/svg.rs`) — checked here end-to-end through
    // `render_html_fixed` too.
    let bx = PureHorzBox::Graphics {
        origin_independent: false,
        width: Length::pt(20.0),
        height: Length::pt(20.0),
        depth: Length::pt(0.0),
        elems: vec![GraphicsElem::Fill(
            Color::Cmyk(1.0, 0.0, 0.0, 0.0),
            rectangle_path(),
        )],
    };
    let html = render(&page_with_run(bx));
    assert!(
        html.contains("fill=\"rgb(0,255,255)\""),
        "CMYK cyan did not convert to the expected sRGB fill:\n{html}"
    );
}

#[test]
fn frame_recurses_into_its_contents_on_the_frame_baseline() {
    let bx = PureHorzBox::Frame {
        width: Length::pt(50.0),
        height: Length::pt(9.0),
        depth: Length::pt(2.0),
        deco: DecoId(0),
        contents: vec![(Length::pt(3.0), text_run("framed"))],
    };
    let html = render(&page_with_run(bx));
    // left = line.x + frame's own dx(0) + the content's own dx(3) = 53;
    // top = baseline_y(100) - the run's own ascent(9) = 91 — the SAME
    // baseline as the frame itself (a `Frame`'s contents never get a y
    // offset, only x, unlike `Tabular`'s cells below).
    assert!(
        html.contains("left:53pt"),
        "missing frame-content left offset:\n{html}"
    );
    assert!(
        html.contains("top:91pt"),
        "missing frame-content top offset:\n{html}"
    );
    assert!(
        html.contains("framed"),
        "missing frame-content text:\n{html}"
    );
}

#[test]
fn tabular_recurses_into_cell_contents_and_renders_rules() {
    let tab = TabularBox {
        width: Length::pt(100.0),
        height: Length::pt(30.0),
        depth: Length::ZERO,
        cells: vec![TabularCellBox {
            x: Length::pt(5.0),
            baseline_y: Length::pt(10.0),
            contents: vec![(Length::ZERO, text_run("cell"))],
        }],
        rules: vec![GraphicsElem::Stroke(
            Length::pt(0.5),
            Color::Gray(0.0),
            rectangle_path(),
        )],
    };
    let html = render(&page_with_run(PureHorzBox::Tabular(tab)));

    // Cell content: left = line.x(50) + cell.x(5) + cdx(0) = 55; top =
    // (baseline_y(100) - cell.baseline_y(10)) - the run's ascent(9) = 81 —
    // `cell.baseline_y` is box-local y-UP (the `GraphicsElem` convention),
    // so it SUBTRACTS from the page-down anchor (the mirror image of the
    // PDF writer's `ty + cell.baseline_y` in its own y-up space).
    assert!(
        html.contains("left:55pt"),
        "missing cell content left offset:\n{html}"
    );
    assert!(
        html.contains("top:81pt"),
        "missing cell content top offset:\n{html}"
    );
    assert!(html.contains("cell"), "missing cell content text:\n{html}");

    // The grid rules render as their own <svg> path, anchored at the
    // TABULAR box's own placed origin (not any individual cell's).
    assert!(html.contains("<svg"), "missing rules <svg>:\n{html}");
    assert!(
        html.contains("d=\"M0 0 L20 0 L20 20 L0 20 Z\""),
        "missing rule path d attribute:\n{html}"
    );
}

#[test]
fn embedded_block_stacks_lines_from_the_placed_anchor() {
    let line = |text: &str| VertBox::Line {
        height: Length::pt(9.0),
        depth: Length::pt(2.0),
        leading: Length::pt(15.0),
        contents: vec![(Length::ZERO, text_run(text))],
    };
    let bx = PureHorzBox::EmbeddedBlock {
        breakable: false,
        width: Length::pt(80.0),
        height: Length::pt(9.0),
        depth: Length::pt(26.0),
        block: vec![line("first"), line("second")],
        anchor_last: false,
    };
    let html = render(&page_with_run(bx));

    assert_eq!(
        html.matches("<span").count(),
        2,
        "expected exactly two lines:\n{html}"
    );
    assert!(html.contains("first"), "missing first line text:\n{html}");
    assert!(html.contains("second"), "missing second line text:\n{html}");
    // First line's baseline lands exactly at the box's own placed anchor
    // (100), so its top matches an ordinary top-level run: 100 - 9 = 91.
    // Second line falls one `leading` (15pt) further down the page: top =
    // (100 + 15) - 9 = 106.
    assert!(
        html.contains("top:91pt"),
        "missing first line's top offset:\n{html}"
    );
    assert!(
        html.contains("top:106pt"),
        "missing second line's top offset:\n{html}"
    );
}

// ============================================================================
// Slice 3 (: "real fonts + math"): `@font-face` data-URI embedding, `Image`
// -> `<img>` data URIs, and `Math` glyph spans. The font/math tests need a
// real TrueType face on disk — located via fontconfig, falling back to a few
// common distro/nix paths, and SKIPPED gracefully (mirroring `tests/ttf.rs`'s
// own `find_regular_font`/`need_font!`, duplicated here per this workspace's
// per-crate-test-file convention, e.g. `tests/image.rs`'s fixture-duplication
// comment) when none is found — rather than failing the build on a machine
// with no DejaVu installed.
// ============================================================================

use std::path::{Path as FsPath, PathBuf};
use std::process::Command;

use rustyfi_backend::FontMetrics;
use rustyfi_pdf::TtfFontStore;

fn find_regular_font() -> Option<PathBuf> {
    if let Ok(output) = Command::new("fc-match")
        .args(["--format=%{file}", "DejaVuSans"])
        .output()
    {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path.is_empty() && FsPath::new(&path).is_file() {
                return Some(PathBuf::from(path));
            }
        }
    }

    for candidate in [
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/dejavu/DejaVuSans.ttf",
        "/run/current-system/sw/share/fonts/truetype/DejaVuSans.ttf",
        "/run/current-system/sw/share/X11/fonts/DejaVuSans.ttf",
    ] {
        if FsPath::new(candidate).is_file() {
            return Some(PathBuf::from(candidate));
        }
    }

    None
}

macro_rules! need_font {
    () => {
        match find_regular_font() {
            Some(path) => path,
            None => {
                eprintln!(
                    "skipping: no DejaVuSans-like TrueType font found on this system \
                     (tried `fc-match DejaVuSans` and common nix/distro paths)"
                );
                return;
            }
        }
    };
}

/// (a) `render_html_fixed_ttf_with` under a real `TtfFontStore` emits one
/// `@font-face` rule with a `data:font/ttf;base64,` src, and the run's own
/// `<span>` names that SAME `font-family` — the metric-fidelity mechanism
/// the design doc's §Risks flags as Option A's core risk (Slice 3's whole
/// point).
#[test]
fn ttf_render_emits_font_face_with_embedded_ttf_and_spans_reference_it() {
    let path = need_font!();
    let store = TtfFontStore::load(&path, None, None).expect("load font");

    let page = page_with_run(text_run("Hello"));
    let html = rustyfi_html::render_html_fixed_ttf_with(
        &geometry(),
        std::slice::from_ref(&page),
        &store,
        &[],
        &DocExtras::default(),
    )
    .expect("HTML rendering must succeed");

    assert!(
        html.contains("@font-face"),
        "missing @font-face rule:\n{html}"
    );
    assert!(
        html.contains("data:font/ttf;base64,"),
        "missing embedded TTF data URI:\n{html}"
    );

    // Pull the family name the @font-face rule just declared, then confirm
    // the run's <span> style names that EXACT family (not just "some"
    // font-family, or a mismatched one).
    let needle = "font-family: \"";
    let start = html
        .find(needle)
        .expect("missing font-family in @font-face")
        + needle.len();
    let end = start
        + html[start..]
            .find('"')
            .expect("unterminated font-family value");
    let family = &html[start..end];
    assert!(
        html.contains(&format!("font-family:\"{family}\"")),
        "span did not reference the @font-face family {family:?}:\n{html}"
    );
}

/// A `render_html_fixed_ttf_with` document that never emits any run marks no font
/// file as used, so no `@font-face` rule should appear — the store being
/// configured at all must not, on its own, force an embed (mirrors the CID
/// PDF writer only writing fonts for `usage.keys()`, `cid.rs`).
#[test]
fn ttf_render_with_no_runs_emits_no_font_face() {
    let path = need_font!();
    let store = TtfFontStore::load(&path, None, None).expect("load font");
    let html =
        rustyfi_html::render_html_fixed_ttf_with(&geometry(), &[], &store, &[], &DocExtras::default())
            .expect("HTML rendering must succeed");
    assert!(
        !html.contains("@font-face"),
        "an empty document must not emit @font-face:\n{html}"
    );
}

/// Base-14 mode (`render_html_fixed`, no store) must stay Slice 1/2's behavior
/// exactly: no `@font-face` block, no per-run `font-family` override.
#[test]
fn base14_render_still_emits_no_font_face() {
    let html = render(&page_with_run(text_run("plain")));
    assert!(
        !html.contains("@font-face"),
        "base-14 mode must not emit @font-face:\n{html}"
    );
    assert!(
        !html.contains("font-family:\"rustyfi-html-font"),
        "base-14 mode must not override font-family per run:\n{html}"
    );
}

fn small_image() -> ImageResource {
    // A 2x2 RGB8 fixture (mirrors the design doc's Slice-2 "tiny 2x2
    // ImageResource" note, reused here since Image itself lands in Slice 3):
    // top row red/green, bottom row blue/white, top-to-bottom per
    // `ImageResource`'s own doc comment.
    ImageResource {
        samples: vec![
            255, 0, 0, 0, 255, 0, //
            0, 0, 255, 255, 255, 255,
        ],
        px_w: 2,
        px_h: 2,
        jpeg_dct: None,
        pdf: None,
    }
}

/// (b) `PureHorzBox::Image` -> an `<img>` positioned at the box's origin
/// with the `ImageResource`'s bytes as a base64 `data:` URI.
#[test]
fn image_box_renders_as_an_img_data_uri() {
    let bx = PureHorzBox::Image {
        width: Length::pt(40.0),
        height: Length::pt(20.0),
        image: ImageId(0),
    };
    let page = page_with_run(bx);
    let html = rustyfi_html::render_html_fixed(
        &geometry(),
        std::slice::from_ref(&page),
        &[small_image()],
        &DocExtras::default(),
    )
    .expect("HTML rendering must succeed");

    assert!(html.contains("<img"), "missing <img> element:\n{html}");
    assert!(
        html.contains("src=\"data:image/bmp;base64,"),
        "missing base64 image data URI with the expected MIME:\n{html}"
    );
    // left = line.x + dx = 50; top = baseline_y - height = 100 - 20 = 80 —
    // an Image box is all height/zero depth, so its baseline is its bottom
    // edge (the same arithmetic `place_image`, `lib.rs:165`, relies on).
    assert!(
        html.contains("left:50pt"),
        "missing image left offset:\n{html}"
    );
    assert!(
        html.contains("top:80pt"),
        "missing image top offset:\n{html}"
    );
    assert!(html.contains("width:40pt"), "missing image width:\n{html}");
    assert!(
        html.contains("height:20pt"),
        "missing image height:\n{html}"
    );
}

/// An `Image` box whose `ImageId` has no matching entry in the document's
/// image table renders nothing (mirrors `write_image_xobjects`'s own
/// graceful skip, `lib.rs:136-142`) rather than panicking.
#[test]
fn image_with_out_of_range_id_renders_nothing() {
    let bx = PureHorzBox::Image {
        width: Length::pt(40.0),
        height: Length::pt(20.0),
        image: ImageId(5),
    };
    let html = render(&page_with_run(bx)); // `render`'s helper always passes `images = &[]`
    assert!(
        !html.contains("<img"),
        "out-of-range ImageId must render nothing:\n{html}"
    );
}

/// (c) A `Math` box's glyphs render as ordinary positioned `<span>`s
/// (reusing the `InnerString` run path — the design doc's math row: the
/// semantic tree is already flattened by `read_math` by the time a box
/// exists) and `rules` (the fraction bar) render through the Slice-2 SVG
/// path; under a real font store, the glyph spans also register the SAME
/// `@font-face` mechanism a plain text run does — the design's requirement
/// that the math font gets registered too, not a separate math-specific
/// code path.
#[test]
fn math_box_renders_glyph_spans_and_fraction_rule() {
    let path = need_font!();
    let store = TtfFontStore::load(&path, None, None).expect("load font");
    let size = Length::pt(18.0);
    let font = FontKey(0);

    let make_glyph = |c: char, dx: f64, dy: f64| {
        let advance = store
            .advance(font, c, size)
            .expect("DejaVu must measure ASCII");
        MathGlyph {
            info: HorzStringInfo {
                font,
                size,
                rising: Length::ZERO,
                color: Color::Gray(0.0),
            },
            text: c.to_string(),
            gid: None,
            dx: Length::pt(dx),
            dy: Length::pt(dy),
            width: advance,
            height: store.ascender(font, size),
            depth: store.descender(font, size),
        }
    };

    // A minimal `\frac{1}{x}`-shaped box: numerator glyph raised, denominator
    // glyph lowered, a filled fraction bar in between — `rules` in the SAME
    // box-local convention `Tabular.rules` uses (checked by the existing
    // `tabular_recurses_into_cell_contents_and_renders_rules` test above).
    let numerator = make_glyph('1', 0.0, 10.0);
    let denominator = make_glyph('x', 0.0, -8.0);
    let bar = GraphicsElem::Fill(Color::Gray(0.0), rectangle_path());

    let math = PureHorzBox::Math {
        width: Length::pt(20.0),
        height: Length::pt(20.0),
        depth: Length::pt(5.0),
        glyphs: vec![numerator, denominator],
        rules: vec![bar],
    };

    let page = page_with_run(math);
    let html = rustyfi_html::render_html_fixed_ttf_with(
        &geometry(),
        std::slice::from_ref(&page),
        &store,
        &[],
        &DocExtras::default(),
    )
    .expect("HTML rendering must succeed");

    assert_eq!(
        html.matches("<span").count(),
        2,
        "expected exactly two glyph spans (numerator, denominator):\n{html}"
    );
    assert!(
        html.contains(">1<"),
        "missing numerator glyph text:\n{html}"
    );
    assert!(
        html.contains(">x<"),
        "missing denominator glyph text:\n{html}"
    );
    assert!(html.contains("<svg"), "missing fraction-bar <svg>:\n{html}");
    assert!(
        html.contains("@font-face"),
        "math glyphs should register the SAME @font-face mechanism as text:\n{html}"
    );
}

// ============================================================================
// Slice 4 (: "multi-page + print pagination"): a `>= 2`-page document
// renders one `<div class="page">` per `Page` plus the print
// `@page`/`page-break-after` CSS, and `DocExtras::page_graphics` (the
// per-page deco-graphics underlay, absolute PDF y-up page coordinates)
// renders as a page-anchored `<svg>` flipped into HTML's y-down page space.
// ============================================================================

/// Build a small multi-page fixture directly (no `.saty` compile needed,
/// house style for this test file, `page_with_run`'s own precedent above): a
/// `Vec<Page>` with `n` pages, each holding one distinct, page-number-tagged
/// `InnerString` run so each page's own markup is independently
/// identifiable in assertions.
fn pages_with_runs(n: usize) -> Vec<Page> {
    (0..n)
        .map(|i| page_with_run(text_run(&format!("page{i}"))))
        .collect()
}

#[test]
fn two_page_document_renders_two_page_divs() {
    let pages = pages_with_runs(2);
    let html = rustyfi_html::render_html_fixed(&geometry(), &pages, &[], &DocExtras::default())
        .expect("HTML rendering must succeed");

    assert_eq!(
        html.matches("<div class=\"page\"").count(),
        2,
        "expected exactly two page divs:\n{html}"
    );
    assert!(
        html.contains("page0"),
        "missing first page's run text:\n{html}"
    );
    assert!(
        html.contains("page1"),
        "missing second page's run text:\n{html}"
    );
}

#[test]
fn print_css_sizes_the_page_and_breaks_between_pages() {
    let pages = pages_with_runs(2);
    let html = rustyfi_html::render_html_fixed(&geometry(), &pages, &[], &DocExtras::default())
        .expect("HTML rendering must succeed");

    // `@page` pins the printed sheet to the document's own paper size
    // (`geometry`'s 200pt x 300pt fixture above), not the browser default.
    assert!(
        html.contains("@page { size: 200pt 300pt; margin: 0; }"),
        "missing @page print rule:\n{html}"
    );
    // A hard break after every non-last page — both the legacy and
    // Fragmentation-Level-3 property names, for browser compatibility.
    assert!(
        html.contains(".page:not(:last-child)")
            && html.contains("page-break-after: always")
            && html.contains("break-after: page"),
        "missing page-break CSS:\n{html}"
    );
}

#[test]
fn single_page_document_has_no_page_break_after_rule_match() {
    // The design's "keep single-page docs looking identical" requirement:
    // `.page:not(:last-child)` matches nothing when there is only one
    // `.page` div, so — while the (inert) rule TEXT is always present in the
    // `<style>` block — a single page never actually receives a
    // `page-break-after`. This is checked structurally (page div count),
    // since the rule text itself is unconditionally emitted (see the test
    // above) regardless of page count.
    let pages = pages_with_runs(1);
    let html = rustyfi_html::render_html_fixed(&geometry(), &pages, &[], &DocExtras::default())
        .expect("HTML rendering must succeed");
    assert_eq!(
        html.matches("<div class=\"page\"").count(),
        1,
        "expected exactly one page div:\n{html}"
    );
}

/// `DocExtras::page_graphics` — a per-page deco-graphics underlay in
/// ABSOLUTE PDF y-up page coordinates (`doc.rs:76`, confirmed by the PDF
/// writer feeding it to `place_graphics` at anchor `(0.0, 0.0)`,
/// `lib.rs:576`) — renders as a page-anchored `<svg>` UNDER the page's own
/// text, flipped into HTML's y-down page space: a point at PDF-absolute
/// `(px, py)` (measured from the paper's BOTTOM-left) must land at HTML page
/// coordinate `(px, paper_h - py)` (measured from the paper's TOP-left).
#[test]
fn page_graphics_underlay_renders_as_a_flipped_svg_underneath_the_text() {
    // A single filled rectangle anchored at the path's own local origin
    // (0,0)-(20,20); since `page_graphics` coordinates are already
    // page-absolute (not box-local), this path's points ARE page
    // coordinates directly.
    let overlay_path = rectangle_path();
    let mut extras = DocExtras::default();
    extras.page_graphics = vec![vec![GraphicsElem::Fill(
        Color::Rgb(0.0, 1.0, 0.0),
        overlay_path,
    )]];

    let page = page_with_run(text_run("on top"));
    let html = rustyfi_html::render_html_fixed(&geometry(), std::slice::from_ref(&page), &[], &extras)
        .expect("HTML rendering must succeed");

    // The underlay <svg> covers the whole page (paper_w x paper_h from the
    // `geometry` fixture: 200pt x 300pt), anchored at the page's own
    // top-left corner (left:0pt; top:0pt).
    assert!(html.contains("<svg"), "missing underlay <svg>:\n{html}");
    assert!(
        html.contains("left:0pt; top:0pt;"),
        "underlay svg must be anchored at the page origin:\n{html}"
    );
    assert!(
        html.contains("width=\"200pt\" height=\"300pt\""),
        "underlay svg must cover the full page:\n{html}"
    );
    assert!(
        html.contains("viewBox=\"0 0 200 300\""),
        "underlay svg viewBox must match the full page:\n{html}"
    );
    // The fill path's own local coordinates are unchanged (the y-flip is the
    // svg's `<g transform="translate(0,300) scale(1,-1)">` wrapper, not a
    // per-coordinate rewrite — same convention every other `emit_graphics`
    // call in this module uses).
    assert!(
        html.contains("<g transform=\"translate(0,300) scale(1,-1)\">"),
        "missing the full-page y-flip <g>:\n{html}"
    );
    assert!(
        html.contains("d=\"M0 0 L20 0 L20 20 L0 20 Z\""),
        "missing underlay rectangle path:\n{html}"
    );
    assert!(
        html.contains("fill=\"rgb(0,255,0)\""),
        "missing green underlay fill:\n{html}"
    );

    // Underneath: the underlay's <svg> markup must precede the page's own
    // text <span> in document order (painted first = drawn under, per CSS
    // paint order for normal-flow siblings).
    let svg_pos = html.find("<svg").expect("svg present");
    let span_pos = html.find("<span class=\"run\"").expect("span present");
    assert!(
        svg_pos < span_pos,
        "underlay svg must come before the page text in document order:\n{html}"
    );
}

/// A page with no `page_graphics` entry (or an empty one) renders no
/// underlay `<svg>` at all — mirrors `page_content`'s own guard
/// (`lib.rs:575`, "so an extras-free page's content stream stays
/// byte-identical").
#[test]
fn no_page_graphics_renders_no_underlay_svg() {
    let page = page_with_run(text_run("plain"));
    let html = render(&page); // `render`'s helper always passes `&DocExtras::default()`
    assert!(
        !html.contains("<svg"),
        "a page with no page_graphics must not render any <svg>:\n{html}"
    );
}
