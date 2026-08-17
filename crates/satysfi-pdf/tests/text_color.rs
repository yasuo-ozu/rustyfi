//! `set-text-color` fidelity fix: `Context::text_color` (`satysfi-backend`'s
//! `context.rs`) round-trips through `get-text-color` but, before this fix,
//! was never consumed by either PDF writer's glyph emission — every glyph
//! painted black regardless. `HorzStringInfo` (shared by
//! `PureHorzBox::InnerString` text runs AND `MathGlyph` math glyphs) now
//! carries a `color: Color` field; both writers (`crate::emit_box`/
//! `place_math` in `lib.rs`, `cid::emit_box`) wrap a NON-black run's glyph
//! emission in `q`/a fill-color op/`Q`, and emit NOTHING extra for a black
//! run — the byte-identity guard for every pre-existing all-black document.
//!
//! Mirrors `tests/graphics.rs`/`tests/draw_text.rs`'s technique: render to an
//! uncompressed PDF and scan the content stream for the exact operators.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use satysfi_backend::{
    Color, FontKey, FontMetrics, HorzStringInfo, Length, MathGlyph, Page, PageGeometry,
    PlacedLine, PureHorzBox,
};

fn geometry() -> PageGeometry {
    // Round numbers (not `PageGeometry::default`'s mm-converted values) so
    // there is nothing float-formatting-sensitive to guess at.
    PageGeometry {
        paper_width: Length::pt(200.0),
        paper_height: Length::pt(300.0),
        text_origin: (Length::pt(20.0), Length::pt(20.0)),
        text_width: Length::pt(160.0),
        text_height: Length::pt(260.0),
    }
}

fn text_box(text: &str, color: Color) -> PureHorzBox {
    PureHorzBox::InnerString {
        info: HorzStringInfo {
            font: FontKey(0),
            size: Length::pt(12.0),
            rising: Length::ZERO,
            color,
        },
        text: text.to_string(),
        width: Length::pt(20.0),
        height: Length::pt(9.0),
        depth: Length::pt(2.0),
    }
}

// ============================================================================
// Base-14 writer (`render_pdf`, `crate::emit_box`'s `InnerString` arm).
// ============================================================================

#[test]
fn base14_colored_run_is_scoped_in_q_rg_q_black_run_emits_no_color_op() {
    let page = Page {
        lines: vec![PlacedLine {
            x: Length::pt(50.0),
            baseline_y: Length::pt(100.0),
            contents: vec![
                (Length::ZERO, text_box("Blk", Color::Gray(0.0))),
                (Length::pt(40.0), text_box("Red", Color::Rgb(1.0, 0.0, 0.0))),
            ],
        }],
    };
    let bytes = satysfi_pdf::render_pdf(&geometry(), std::slice::from_ref(&page), &[])
        .expect("PDF rendering must succeed");
    assert!(bytes.starts_with(b"%PDF-"), "not a PDF header");
    let hay = String::from_utf8_lossy(&bytes);

    // The red run's `BT…ET` is immediately preceded by `q`/the RGB fill op
    // and immediately followed by `Q` — the exact sequence
    // `emit_box`'s `InnerString` arm emits when `info.color != Gray(0.0)`.
    assert!(
        hay.contains("q\n1 0 0 rg\nBT"),
        "expected the colored run's BT to be immediately preceded by q/1 0 0 rg:\n{hay}"
    );
    let q_pos = hay.find("q\n1 0 0 rg\nBT").unwrap();
    let et_pos = hay[q_pos..].find("ET").expect("ET after the colored run's BT") + q_pos;
    assert!(
        hay[et_pos..].starts_with("ET\nQ"),
        "expected the colored run's ET to be immediately followed by Q:\n{hay}"
    );
    assert!(
        hay[q_pos..et_pos].contains("(Red) Tj"),
        "expected (Red) Tj inside the colored run's q/Q span:\n{hay}"
    );

    // Byte-identity guard: across the WHOLE page there is exactly one
    // q/Q/color-op triple (the red run's) — the black run contributes none
    // of its own, so a same-shaped all-black page renders with zero of
    // these ops at all.
    assert_eq!(hay.matches("\nq\n").count(), 1, "expected exactly one q op:\n{hay}");
    assert_eq!(hay.matches("\nQ\n").count(), 1, "expected exactly one Q op:\n{hay}");
    assert_eq!(hay.matches(" rg\n").count(), 1, "expected exactly one fill-color op:\n{hay}");

    // The black run's own text still renders, just with no surrounding
    // color scoping.
    assert!(hay.contains("(Blk) Tj"), "expected the black run's own text:\n{hay}");
}

#[test]
fn base14_all_black_page_emits_no_color_ops_at_all() {
    // The non-regression control: a page shaped just like the one above but
    // with BOTH runs black must emit no `q`/`Q`/color op whatsoever —
    // exactly today's pre-fix output for an all-black document.
    let page = Page {
        lines: vec![PlacedLine {
            x: Length::pt(50.0),
            baseline_y: Length::pt(100.0),
            contents: vec![
                (Length::ZERO, text_box("Blk", Color::Gray(0.0))),
                (Length::pt(40.0), text_box("Two", Color::Gray(0.0))),
            ],
        }],
    };
    let bytes = satysfi_pdf::render_pdf(&geometry(), std::slice::from_ref(&page), &[])
        .expect("PDF rendering must succeed");
    let hay = String::from_utf8_lossy(&bytes);
    assert!(!hay.contains("\nq\n"), "an all-black page must emit no q:\n{hay}");
    assert!(!hay.contains("\nQ\n"), "an all-black page must emit no Q:\n{hay}");
    assert!(!hay.contains("rg"), "an all-black page must emit no fill-color op:\n{hay}");
    assert!(hay.contains("(Blk) Tj") && hay.contains("(Two) Tj"));
}

// ============================================================================
// Math glyphs (`place_math`, shared by both writers): the `HorzStringInfo`
// win — a colored `MathGlyph` needs no writer-specific code at all, since
// `MathGlyph::info` is the exact same struct as `InnerString`'s.
// ============================================================================

fn math_glyph(text: &str, color: Color) -> MathGlyph {
    MathGlyph {
        info: HorzStringInfo {
            font: FontKey(0),
            size: Length::pt(12.0),
            rising: Length::ZERO,
            color,
        },
        text: text.to_string(),
        gid: None,
        dx: Length::ZERO,
        dy: Length::ZERO,
        width: Length::pt(8.0),
        height: Length::pt(9.0),
        depth: Length::pt(2.0),
    }
}

#[test]
fn base14_colored_math_glyph_is_scoped_in_q_rg_q_black_sibling_glyph_is_not() {
    // One `Math` box holding two glyphs: a black "x" then a blue "y" — the
    // same box, same `place_math` call, so this also proves the guard is
    // per-GLYPH, not just per-box.
    let math_box = PureHorzBox::Math {
        width: Length::pt(16.0),
        height: Length::pt(9.0),
        depth: Length::pt(2.0),
        glyphs: vec![
            math_glyph("x", Color::Gray(0.0)),
            math_glyph("y", Color::Rgb(0.0, 0.0, 1.0)),
        ],
        rules: Vec::new(),
    };
    let page = Page {
        lines: vec![PlacedLine {
            x: Length::pt(50.0),
            baseline_y: Length::pt(100.0),
            contents: vec![(Length::ZERO, math_box)],
        }],
    };
    let bytes = satysfi_pdf::render_pdf(&geometry(), std::slice::from_ref(&page), &[])
        .expect("PDF rendering must succeed");
    let hay = String::from_utf8_lossy(&bytes);

    assert!(
        hay.contains("q\n0 0 1 rg\nBT"),
        "expected the blue glyph's BT to be immediately preceded by q/0 0 1 rg:\n{hay}"
    );
    assert!(hay.contains("(y) Tj"), "expected the blue glyph's own text:\n{hay}");
    assert!(hay.contains("(x) Tj"), "expected the black glyph's own text:\n{hay}");
    // Exactly one fill-color op in the whole stream (the blue glyph's) — the
    // black sibling glyph, in the SAME `Math` box/`place_math` call,
    // contributes none of its own. (Not asserting a total `q`/`Q` count here:
    // a `Math` box's own `rules` graphics — empty in this test — already go
    // through `place_graphics`, which wraps its own unconditional q/cm/Q
    // regardless of color, pre-existing and unrelated to this fix.)
    assert_eq!(hay.matches(" rg\n").count(), 1, "expected exactly one fill-color op:\n{hay}");
    // And the black glyph's own `BT` is untouched — no `rg` immediately
    // precedes it the way it does the blue glyph's.
    let x_pos = hay.find("(x) Tj").expect("black glyph's own text");
    let bt_before_x = hay[..x_pos].rfind("BT").expect("a BT before the black glyph's Tj");
    let prefix = &hay[bt_before_x.saturating_sub(20)..bt_before_x];
    assert!(
        !prefix.contains("rg"),
        "the black glyph's BT must not be preceded by a color op, found in {prefix:?}:\n{hay}"
    );
}

// ============================================================================
// CID/TrueType writer (`render_pdf_ttf`, `cid::emit_box`'s `InnerString`
// arm) — the std-ja capstone's real font-embedding path. Skips gracefully
// (mirrors `tests/ttf.rs`) when no TrueType font is available on this
// machine, since it needs real glyph metrics/ids.
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
fn cid_colored_run_is_scoped_in_q_rg_q_black_run_emits_no_color_op() {
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
    let geo = geometry();
    let font = FontKey(0);
    let size = Length::pt(18.0);

    let mk = |text: &str, color: Color| {
        let width = store.text_width(font, text, size).expect("glyphs exist");
        PureHorzBox::InnerString {
            info: HorzStringInfo { font, size, rising: Length::ZERO, color },
            text: text.to_string(),
            width,
            height: store.ascender(font, size),
            depth: store.descender(font, size),
        }
    };

    let page = Page {
        lines: vec![PlacedLine {
            x: geo.text_origin.0,
            baseline_y: geo.text_origin.1 + store.ascender(font, size),
            contents: vec![
                (Length::ZERO, mk("Hi", Color::Gray(0.0))),
                (Length::pt(40.0), mk("Yo", Color::Rgb(1.0, 0.0, 0.0))),
            ],
        }],
    };
    let bytes = satysfi_pdf::render_pdf_ttf(&geo, &[page], &store, &[]).expect("render");
    assert!(bytes.starts_with(b"%PDF-"));
    let hay = String::from_utf8_lossy(&bytes);

    // Identity-H `Tj` operands are raw glyph-index bytes, not readable
    // ASCII, so (unlike the base-14 test) this can't assert on the text
    // itself — but the color-op scoping is plain ASCII either way.
    assert!(
        hay.contains("q\n1 0 0 rg\nBT"),
        "expected the colored run's BT to be immediately preceded by q/1 0 0 rg:\n{hay}"
    );
    assert_eq!(hay.matches("\nq\n").count(), 1, "only the colored run should wrap q/Q:\n{hay}");
    assert_eq!(hay.matches("\nQ\n").count(), 1);
    assert_eq!(hay.matches(" rg\n").count(), 1);

    // Sanity: the PDF is still well-formed enough for `pdftotext` to pull
    // both runs' text back out (mirrors `tests/ttf.rs`'s round-trip check).
    if let Ok(which) = Command::new("which").arg("pdftotext").output() {
        if which.status.success() {
            let mut tmp = std::env::temp_dir();
            tmp.push(format!("satysfi-pdf-text-color-cid-{}.pdf", std::process::id()));
            std::fs::File::create(&tmp)
                .and_then(|mut f| f.write_all(&bytes))
                .expect("write temp pdf");
            if let Ok(out) = Command::new("pdftotext").args(["-layout", tmp.to_str().unwrap(), "-"]).output() {
                if out.status.success() {
                    let text = String::from_utf8_lossy(&out.stdout);
                    assert!(text.contains("Hi"), "pdftotext should recover the black run:\n{text}");
                    assert!(text.contains("Yo"), "pdftotext should recover the colored run:\n{text}");
                }
            }
            let _ = std::fs::remove_file(&tmp);
        }
    }
}
