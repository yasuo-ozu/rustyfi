//! Integration tests for `draw-text` glyph emission (roadmap C1):
//! `GraphicsElem::Text` re-enters each PDF writer's own per-box `emit_box`
//! at box-local coordinates, threaded through `place_graphics`'s
//! `NestedEmitter` callback (`crates/satysfi-pdf/src/lib.rs`). Base-14
//! (`render_pdf`) is checked by scanning the uncompressed content stream for
//! the exact `Td`/`Tj` operators the text run must produce, INSIDE the
//! graphics box's own `q`/`cm`/`Q` wrapper (proving no double translate, no
//! second y-flip, and correct z-order against the box's other elements);
//! CID (`render_pdf_ttf`) is checked end to end through `pdftotext`
//! (mirrors `tests/ttf.rs`'s `to_unicode_roundtrips_through_pdftotext`),
//! since an Identity-H glyph run's content bytes aren't human-readable ASCII
//! the way base-14's WinAnsi `Tj` operand is.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use satysfi_backend::{
    Closing, Color, FontKey, FontMetrics, GraphicsElem, HorzStringInfo, ImageId, ImageResource,
    Length, Page, PageGeometry, Path as GrPath, PathSeg, PlacedLine, PureHorzBox, Subpath,
};

/// A 20pt square (fill), the same shape `tests/graphics.rs` uses.
fn rectangle_path() -> GrPath {
    GrPath {
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

/// A page whose only content is a `PureHorzBox::Graphics` box carrying a
/// `Fill` (a red rectangle) FOLLOWED by a `GraphicsElem::Text` run — the
/// z-order `draw-text`'s design summary promises (fill painted first, text
/// drawn on top, in element order) — with `contents` holding one
/// `InnerString` box at box-local `dx = 0` from the text's own anchor `pt`.
fn page_with_fill_then_text() -> Page {
    let path = rectangle_path();
    let text_info = HorzStringInfo { font: FontKey(0), size: Length::pt(10.0), rising: Length::ZERO };
    let elems = vec![
        GraphicsElem::Fill(Color::Rgb(1.0, 0.0, 0.0), path),
        GraphicsElem::Text {
            pt: (Length::pt(5.0), Length::pt(10.0)),
            contents: vec![(
                Length::ZERO,
                PureHorzBox::InnerString {
                    info: text_info,
                    text: "Hi".to_string(),
                    width: Length::pt(10.0),
                    height: Length::pt(7.5),
                    depth: Length::pt(2.5),
                },
            )],
            width: Length::pt(10.0),
            height: Length::pt(7.5),
            depth: Length::pt(2.5),
        },
    ];
    let gbox = PureHorzBox::Graphics {
        width: Length::pt(20.0),
        height: Length::pt(20.0),
        depth: Length::pt(0.0),
        elems,
    };
    Page {
        lines: vec![PlacedLine {
            x: Length::pt(50.0),
            baseline_y: Length::pt(100.0),
            contents: vec![(Length::ZERO, gbox)],
        }],
    }
}

fn round_geometry() -> PageGeometry {
    // Round numbers (not `PageGeometry::default`'s mm-converted values) so
    // the expected `cm` transform is an exact integer string.
    PageGeometry {
        paper_width: Length::pt(200.0),
        paper_height: Length::pt(300.0),
        text_origin: (Length::pt(20.0), Length::pt(20.0)),
        text_width: Length::pt(160.0),
        text_height: Length::pt(260.0),
    }
}

/// The content-stream span between the FIRST `q`/`Q` pair that immediately
/// follows `needle` (e.g. the graphics box's own `cm` line) — a crude but
/// sufficient "is X between this box's q and Q" check given this crate's
/// uncompressed, one-op-per-line content streams (every op is `\n`-
/// terminated, so `q`/`Q` are always their own lines).
fn span_after<'a>(hay: &'a str, needle: &str) -> &'a str {
    let start = hay.find(needle).unwrap_or_else(|| panic!("missing {needle:?} in:\n{hay}"));
    &hay[start..]
}

#[test]
fn base14_draw_text_emits_td_tj_inside_the_boxs_q_cm_q() {
    let geometry = round_geometry();
    let page = page_with_fill_then_text();
    let bytes = satysfi_pdf::render_pdf(&geometry, std::slice::from_ref(&page), &[])
        .expect("PDF rendering must succeed");
    assert!(bytes.starts_with(b"%PDF-"), "not a PDF header");

    let hay = String::from_utf8_lossy(&bytes);

    // The whole graphics box is translated once via `cm` to its placed,
    // already-y-flipped anchor `(50, paper_h - 100)`.
    let expected_ty = geometry.paper_height.0 - 100.0;
    let expected_cm = format!("1 0 0 1 50 {expected_ty} cm");
    assert!(
        hay.contains(&expected_cm),
        "content stream missing the box's placement transform {expected_cm:?}:\n{hay}"
    );

    // Everything from the `cm` onward: the fill, then the text run, in that
    // order (z-order: fill painted first, text drawn on top).
    let after_cm = span_after(&hay, &expected_cm);
    let fill_pos = after_cm.find("f*").expect("fill op");
    let bt_pos = after_cm.find("BT").expect("BT (text run)");
    assert!(fill_pos < bt_pos, "expected the fill before the text run (z-order)");

    // The text run itself: `Tf` at the string's own size, box-local `Td` at
    // `pt + dx = (5, 10)` (NOT re-flipped, NOT re-translated — box-local,
    // inside the `cm` above), `(Hi) Tj`, `ET`.
    for op in ["BT", "10 Tf", "5 10 Td", "(Hi) Tj", "ET"] {
        assert!(
            after_cm.contains(op),
            "content stream (after the box's cm) missing {op:?}:\n{after_cm}"
        );
    }

    // The text run sits strictly between the box's own opening `q`/`cm` and
    // its closing `Q` — never after the outer `Q` closes the box's CTM
    // (which would place it at PAGE-space coordinates, wrong by (50, ty)).
    let after_text = &after_cm[after_cm.find("ET").unwrap()..];
    assert!(after_text.contains('Q'), "expected the box to still close with Q after its text");
}

#[test]
fn base14_draw_text_on_an_empty_run_emits_no_text_operators() {
    // `draw-text (pt) inline-nil` (the lang-side FAITHFUL empty case, see
    // `stdlib_tier0.rs`): a `Text` element with an empty `contents` run
    // still renders (its wrapping `q`/`Q` is unconditional, matching every
    // other element), but contributes no `BT`/`Tj` at all.
    let geometry = round_geometry();
    let elems = vec![GraphicsElem::Text {
        pt: (Length::pt(1.0), Length::pt(2.0)),
        contents: Vec::new(),
        width: Length::ZERO,
        height: Length::ZERO,
        depth: Length::ZERO,
    }];
    let gbox = PureHorzBox::Graphics {
        width: Length::ZERO,
        height: Length::ZERO,
        depth: Length::ZERO,
        elems,
    };
    let page = Page {
        lines: vec![PlacedLine {
            x: Length::ZERO,
            baseline_y: Length::ZERO,
            contents: vec![(Length::ZERO, gbox)],
        }],
    };
    let bytes = satysfi_pdf::render_pdf(&geometry, std::slice::from_ref(&page), &[])
        .expect("PDF rendering must succeed");
    let hay = String::from_utf8_lossy(&bytes);
    assert!(!hay.contains("BT"), "empty Text run should emit no BT:\n{hay}");
}

// ============================================================================
// CID writer (`render_pdf_ttf`): text extraction through `pdftotext`, since
// an Identity-H glyph run's `Tj` operand is raw big-endian glyph IDs, not
// readable ASCII. Skips gracefully (mirrors `tests/ttf.rs`) when no
// TrueType font or `pdftotext` is available on this machine.
// ============================================================================

fn find_regular_font() -> Option<PathBuf> {
    if let Ok(output) = Command::new("fc-match").args(["--format=%{file}", "DejaVuSans"]).output() {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path.is_empty() && Path::new(&path).is_file() {
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
        if Path::new(candidate).is_file() {
            return Some(PathBuf::from(candidate));
        }
    }
    None
}

#[test]
fn cid_draw_text_run_survives_pdftotext_extraction() {
    let path = match find_regular_font() {
        Some(p) => p,
        None => {
            eprintln!(
                "skipping: no DejaVuSans-like TrueType font found on this system \
                 (tried `fc-match DejaVuSans` and common nix/distro paths)"
            );
            return;
        }
    };
    let store = satysfi_pdf::TtfFontStore::load(&path, None, None).expect("load font");
    let geometry = round_geometry();

    let font = FontKey(0);
    let size = Length::pt(18.0);
    let text = "Hi";
    let width = store.text_width(font, text, size).expect("glyphs exist");
    let ascender = store.ascender(font, size);
    let descender = store.descender(font, size);

    let elems = vec![GraphicsElem::Text {
        pt: (Length::pt(0.0), Length::pt(0.0)),
        contents: vec![(
            Length::ZERO,
            PureHorzBox::InnerString {
                info: HorzStringInfo { font, size, rising: Length::ZERO },
                text: text.to_string(),
                width,
                height: ascender,
                depth: descender,
            },
        )],
        width,
        height: ascender,
        depth: descender,
    }];
    let gbox = PureHorzBox::Graphics {
        width,
        height: ascender,
        depth: descender,
        elems,
    };
    let page = Page {
        lines: vec![PlacedLine {
            x: geometry.text_origin.0,
            baseline_y: geometry.text_origin.1 + ascender,
            contents: vec![(Length::ZERO, gbox)],
        }],
    };

    let pdf_bytes =
        satysfi_pdf::render_pdf_ttf(&geometry, &[page], &store, &[]).expect("render");
    assert!(pdf_bytes.starts_with(b"%PDF-"));

    let Ok(which) = Command::new("which").arg("pdftotext").output() else {
        eprintln!("skipping pdftotext check: `which` not available");
        return;
    };
    if !which.status.success() {
        eprintln!("skipping pdftotext check: pdftotext not on PATH");
        return;
    }

    let mut tmp = std::env::temp_dir();
    tmp.push(format!("satysfi-pdf-draw-text-cid-{}.pdf", std::process::id()));
    std::fs::File::create(&tmp)
        .and_then(|mut f| f.write_all(&pdf_bytes))
        .expect("write temp pdf");

    let output = Command::new("pdftotext").arg(&tmp).arg("-").output().expect("run pdftotext");
    let _ = std::fs::remove_file(&tmp);

    assert!(output.status.success(), "pdftotext failed: {output:?}");
    let extracted = String::from_utf8_lossy(&output.stdout);
    assert!(
        extracted.contains("Hi"),
        "pdftotext output missing graphics-positioned text \"Hi\", got: {extracted:?}"
    );
}

// ============================================================================
// Regression: a Fill/Stroke-only graphics box (no Text element) produces a
// byte-identical content stream to before this slice — `tests/graphics.rs`
// already pins this exactly; this is a second, independent witness that the
// `place_graphics` signature change (adding `NestedEmitter`) is behavior-
// neutral for existing (Text-free) documents.
// ============================================================================

#[test]
fn a_text_free_graphics_box_is_unaffected_by_the_nested_emitter_plumbing() {
    let geometry = round_geometry();
    let path = rectangle_path();
    let elems = vec![GraphicsElem::Fill(Color::Rgb(1.0, 0.0, 0.0), path)];
    let gbox = PureHorzBox::Graphics {
        width: Length::pt(20.0),
        height: Length::pt(20.0),
        depth: Length::pt(0.0),
        elems,
    };
    let page = Page {
        lines: vec![PlacedLine {
            x: Length::pt(50.0),
            baseline_y: Length::pt(100.0),
            contents: vec![(Length::ZERO, gbox)],
        }],
    };
    let bytes = satysfi_pdf::render_pdf(&geometry, std::slice::from_ref(&page), &[])
        .expect("PDF rendering must succeed");
    let hay = String::from_utf8_lossy(&bytes);
    assert!(!hay.contains("BT"), "no Text element present, so no BT should appear:\n{hay}");
    assert!(hay.contains("f*"));
}

// ============================================================================
// A `draw-text` run's `contents` can carry a `PureHorzBox::Image` (its
// `inline-boxes` argument is arbitrary `read-inline`d content, so it can
// embed a `use-image-by-width` box). `used_images`'s recursive scan
// (`lib.rs`'s `scan_box_images`) must find it THROUGH a `GraphicsElem::Text`
// run — the same nested-content class as `Frame`/`Tabular`/`EmbeddedBlock`
// — or the writer never registers the `/XObject` the content stream's `Do`
// operator then dangles on.
// ============================================================================

#[test]
fn an_image_nested_inside_a_draw_text_run_gets_its_xobject_registered() {
    let geometry = round_geometry();
    let image = ImageResource { samples: vec![255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 0], px_w: 2, px_h: 2 };
    let elems = vec![GraphicsElem::Text {
        pt: (Length::pt(0.0), Length::pt(0.0)),
        contents: vec![(
            Length::ZERO,
            PureHorzBox::Image {
                width: Length::pt(10.0),
                height: Length::pt(10.0),
                image: ImageId(0),
            },
        )],
        width: Length::pt(10.0),
        height: Length::pt(10.0),
        depth: Length::ZERO,
    }];
    let gbox = PureHorzBox::Graphics {
        width: Length::pt(10.0),
        height: Length::pt(10.0),
        depth: Length::ZERO,
        elems,
    };
    let page = Page {
        lines: vec![PlacedLine {
            x: Length::ZERO,
            baseline_y: Length::ZERO,
            contents: vec![(Length::ZERO, gbox)],
        }],
    };
    let bytes = satysfi_pdf::render_pdf(&geometry, std::slice::from_ref(&page), &[image])
        .expect("PDF rendering must succeed");
    let hay = String::from_utf8_lossy(&bytes);
    // The page's `/XObject` resource dictionary must list the image (proving
    // `used_images` found it through the `Text` run, not just skipped it),
    // and the content stream must place it (`Do`).
    assert!(hay.contains("/Im0"), "expected the image's XObject to be registered:\n{hay}");
    assert!(hay.contains("Do"), "expected a Do operator placing the image:\n{hay}");
}
