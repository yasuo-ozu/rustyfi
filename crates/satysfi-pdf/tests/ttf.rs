//! Integration tests for the ttf-parser-backed `FontMetrics` provider and
//! the CID-keyed PDF embedder. These need a real TrueType file on disk, so
//! every test locates one via fontconfig (falling back to a few common
//! paths) and skips gracefully — rather than failing the build — when none
//! is found.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use satysfi_backend::{
    FontKey, FontMetrics, HorzStringInfo, Length, Page, PageGeometry, PlacedLine, PureHorzBox,
};
use satysfi_pdf::{render_pdf_ttf, TtfFontStore};

/// Locate a real TrueType file to test against: prefer fontconfig's idea of
/// "DejaVu Sans" (present on this dev machine), then fall back to a few
/// paths common on NixOS/nix-based systems and typical Linux distros.
fn find_regular_font() -> Option<PathBuf> {
    if let Ok(output) = Command::new("fc-match")
        .args(["--format=%{file}", "DejaVuSans"])
        .output()
    {
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

#[test]
fn metrics_sanity() {
    let path = need_font!();
    let store = TtfFontStore::load(&path, None, None).expect("load font");
    let size = Length::pt(12.0);

    // DejaVu Sans is proportional: a narrow glyph must advance less than a
    // wide one.
    let narrow = store
        .advance(FontKey(0), 'i', size)
        .expect("'i' has a glyph");
    let wide = store
        .advance(FontKey(0), 'W', size)
        .expect("'W' has a glyph");
    assert!(
        narrow.0 < wide.0,
        "expected advance('i') < advance('W'), got {narrow:?} vs {wide:?}"
    );

    let ascender = store.ascender(FontKey(0), size);
    let descender = store.descender(FontKey(0), size);
    assert!(ascender.0 > 0.0, "ascender should be positive: {ascender:?}");
    assert!(
        descender.0 > 0.0,
        "descender should be a positive depth below the baseline: {descender:?}"
    );
}

#[test]
fn bold_and_oblique_fall_back_to_regular() {
    let path = need_font!();
    let store = TtfFontStore::load(&path, None, None).expect("load font");
    let size = Length::pt(12.0);

    // With no bold/oblique files given, all three keys should measure
    // identically (same underlying file).
    let regular = store.advance(FontKey(0), 'A', size);
    let bold = store.advance(FontKey(1), 'A', size);
    let oblique = store.advance(FontKey(2), 'A', size);
    assert_eq!(regular, bold);
    assert_eq!(regular, oblique);
    assert_eq!(store.num_files(), 1);
}

/// Build a single hand-placed line of text in `font`, using `store` to
/// measure it, mirroring what the (milestone-1) line/page breaker would
/// produce.
fn build_line(store: &TtfFontStore, geometry: &PageGeometry, text: &str, font: FontKey) -> PlacedLine {
    let size = Length::pt(18.0);
    let width = store
        .text_width(font, text, size)
        .expect("all chars in this test have glyphs");
    let ascender = store.ascender(font, size);
    let descender = store.descender(font, size);

    PlacedLine {
        x: geometry.text_origin.0,
        baseline_y: geometry.text_origin.1 + ascender,
        contents: vec![(
            Length::ZERO,
            PureHorzBox::InnerString {
                info: HorzStringInfo { font, size },
                text: text.to_string(),
                width,
                height: ascender,
                depth: descender,
            },
        )],
    }
}

#[test]
fn render_pdf_ttf_produces_a_pdf_with_embedded_font() {
    let path = need_font!();
    let store = TtfFontStore::load(&path, None, None).expect("load font");
    let geometry = PageGeometry::default();

    let ascii_line = build_line(&store, &geometry, "Hello World", FontKey(0));
    let page = Page {
        lines: vec![ascii_line],
    };

    let pdf_bytes = render_pdf_ttf(&geometry, &[page], &store).expect("render");

    assert!(
        pdf_bytes.starts_with(b"%PDF-"),
        "output should start with a PDF header"
    );

    // Full-file embedding heuristic: the raw font bytes appear verbatim in
    // the output (no subsetting yet), so the PDF must be at least as big as
    // the source font file.
    let font_len = std::fs::metadata(&path).expect("stat font file").len() as usize;
    assert!(
        pdf_bytes.len() > font_len,
        "expected the PDF ({} bytes) to be larger than the embedded font file ({} bytes)",
        pdf_bytes.len(),
        font_len
    );
}

#[test]
fn to_unicode_roundtrips_through_pdftotext() {
    let path = need_font!();
    let store = TtfFontStore::load(&path, None, None).expect("load font");

    // Pick a non-WinAnsi character the face can actually render, so this
    // test exercises glyph IDs / ToUnicode beyond plain ASCII rather than
    // asserting on a character that happens not to exist in the face.
    let face = store.face(FontKey(0)).expect("parse face");
    let Some(extra) = ['é', '→', '中'].into_iter().find(|&c| face.glyph_index(c).is_some()) else {
        eprintln!("skipping ToUnicode check: font has none of the candidate non-WinAnsi glyphs");
        return;
    };
    drop(face);

    let geometry = PageGeometry::default();
    let line1 = build_line(&store, &geometry, "Hello World", FontKey(0));
    let extra_text = format!("caf{extra}");
    let mut line2 = build_line(&store, &geometry, &extra_text, FontKey(0));
    // Stack the second line below the first so they don't overlap.
    line2.baseline_y = line1.baseline_y + Length::pt(24.0);
    let page = Page {
        lines: vec![line1, line2],
    };

    let pdf_bytes = render_pdf_ttf(&geometry, &[page], &store).expect("render");

    let Ok(which) = Command::new("which").arg("pdftotext").output() else {
        eprintln!("skipping pdftotext check: `which` not available");
        return;
    };
    if !which.status.success() {
        eprintln!("skipping pdftotext check: pdftotext not on PATH");
        return;
    }

    let mut tmp = std::env::temp_dir();
    tmp.push(format!("satysfi-pdf-ttf-test-{}.pdf", std::process::id()));
    std::fs::File::create(&tmp)
        .and_then(|mut f| f.write_all(&pdf_bytes))
        .expect("write temp pdf");

    let output = Command::new("pdftotext")
        .arg(&tmp)
        .arg("-")
        .output()
        .expect("run pdftotext");
    let _ = std::fs::remove_file(&tmp);

    assert!(output.status.success(), "pdftotext failed: {output:?}");
    let text = String::from_utf8_lossy(&output.stdout);

    assert!(
        text.contains("Hello World"),
        "pdftotext output missing ASCII line, got: {text:?}"
    );
    assert!(
        text.contains(&extra_text),
        "pdftotext output missing non-WinAnsi line {extra_text:?}, got: {text:?}"
    );
}
