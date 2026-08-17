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
                info: HorzStringInfo { font, size, rising: Length::ZERO },
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

    let pdf_bytes = render_pdf_ttf(&geometry, &[page], &store, &[]).expect("render");

    assert!(
        pdf_bytes.starts_with(b"%PDF-"),
        "output should start with a PDF header"
    );

    // D5 (docs/plans/text-rendering.md §2): the embedded `FontFile2` is now
    // SUBSET to the ~10 glyphs "Hello World" actually uses, so the whole PDF
    // is much SMALLER than the source face (inverted from the pre-D5
    // whole-file-embed assertion this test used to make).
    let font_len = std::fs::metadata(&path).expect("stat font file").len() as usize;
    assert!(
        pdf_bytes.len() < font_len,
        "expected the subsetted PDF ({} bytes) to be smaller than the source font file ({} bytes)",
        pdf_bytes.len(),
        font_len
    );
}

/// A subsetted font's `/BaseFont` carries the PDF-spec subset tag
/// (`XXXXXX+FontName`, six uppercase letters then `+`) — the signal a real
/// PDF consumer / `qpdf --check` uses to know this copy of "FontName" is a
/// partial glyph set, not the whole face.
#[test]
fn subsetted_base_font_carries_a_subset_tag() {
    let path = need_font!();
    let store = TtfFontStore::load(&path, None, None).expect("load font");
    let geometry = PageGeometry::default();
    let page = Page {
        lines: vec![build_line(&store, &geometry, "Hi", FontKey(0))],
    };
    let pdf_bytes = render_pdf_ttf(&geometry, &[page], &store, &[]).expect("render");
    let text = String::from_utf8_lossy(&pdf_bytes);
    let base_font = text
        .split("/BaseFont")
        .nth(1)
        .and_then(|rest| rest.split('/').nth(1))
        .map(|s| s.split_whitespace().next().unwrap_or(""))
        .unwrap_or("");
    assert!(
        base_font.len() >= 8
            && base_font.as_bytes()[6] == b'+'
            && base_font[..6].bytes().all(|b| b.is_ascii_uppercase()),
        "expected a `XXXXXX+Name` subset tag in /BaseFont, got {base_font:?} (raw: {text})"
    );
}

/// D5 reruns are reproducible: subsetting the same document twice yields
/// byte-identical output (the subset tag is a deterministic hash of the
/// used-gid set, not e.g. a random UUID or a timestamp).
#[test]
fn subsetting_is_reproducible_across_reruns() {
    let path = need_font!();
    let store = TtfFontStore::load(&path, None, None).expect("load font");
    let geometry = PageGeometry::default();
    let page = || Page {
        lines: vec![build_line(&store, &geometry, "Reproducible", FontKey(0))],
    };
    let a = render_pdf_ttf(&geometry, &[page()], &store, &[]).expect("render 1");
    let b = render_pdf_ttf(&geometry, &[page()], &store, &[]).expect("render 2");
    assert_eq!(a, b, "two renders of the same document must be byte-identical");
}

/// D5's `glyf`-presence gate: a CFF-outline OpenType face (no `glyf` table
/// at all — this host's `NotoSansTagalog-Regular.otf`, or any single-face
/// non-collection `.otf` fontconfig turns up) must NOT be subsetted (the
/// `subsetter` crate's TrueType path doesn't apply) and must still degrade
/// gracefully to the pre-D5 whole-file embed rather than erroring — the
/// module doc's "first honest gate" for CFF/`CIDFontType0` being out of
/// scope.
fn find_cff_otf() -> Option<PathBuf> {
    let output = Command::new("fc-list").output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines() {
        let Some(path_str) = line.split(':').next() else {
            continue;
        };
        if !path_str.ends_with(".otf") {
            continue; // skip .ttc (collection) and .ttf entries
        }
        let path = PathBuf::from(path_str);
        if !path.is_file() {
            continue;
        }
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let Ok(face) = ttf_parser::Face::parse(&bytes, 0) else {
            continue;
        };
        if face.tables().glyf.is_none() && face.tables().cff.is_some() {
            return Some(path);
        }
    }
    None
}

#[test]
fn cff_face_falls_back_to_whole_file_embed_without_erroring() {
    let Some(path) = find_cff_otf() else {
        eprintln!("skipping: no CFF-outline .otf found via fc-list on this system");
        return;
    };
    let store = match TtfFontStore::load(&path, None, None) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("skipping: {path:?} failed to load as a face at all: {e}");
            return;
        }
    };
    let geometry = PageGeometry::default();
    // Use whatever char this face actually has a glyph for — its own
    // repertoire is arbitrary (Tagalog, or whatever fc-list found).
    let face = store.face(FontKey(0)).expect("parse face");
    let Some(c) = "Aa1 .".chars().find(|&c| face.glyph_index(c).is_some()) else {
        eprintln!("skipping: face has none of the trivial probe glyphs");
        return;
    };
    drop(face);
    let page = Page {
        lines: vec![build_line(&store, &geometry, &c.to_string(), FontKey(0))],
    };

    let pdf_bytes = render_pdf_ttf(&geometry, &[page], &store, &[]).expect(
        "a CFF face must degrade to whole-file embed, not error",
    );
    assert!(pdf_bytes.starts_with(b"%PDF-"));
    // No `glyf` table => `write_font`'s subsetter gate is skipped entirely
    // => the whole input file is embedded verbatim (pre-D5 behavior for
    // this face), so the PDF is at least as large as the source file.
    let font_len = std::fs::metadata(&path).expect("stat font file").len() as usize;
    assert!(
        pdf_bytes.len() > font_len,
        "expected whole-file embed: PDF ({} bytes) should exceed the source face ({} bytes)",
        pdf_bytes.len(),
        font_len,
    );
}

/// D5 x §B3 interaction (module doc, "Interactions verified"): a raw
/// MATH-table variant gid (`MathGlyph::gid: Some(_)`, not necessarily
/// cmap-reachable from `text`) inserted into `usage.glyphs` by `emit_box`'s
/// `Math` arm must survive subsetting — the `subsetter` crate's `glyf`
/// composite closure is generic over "whatever gids you asked to keep",
/// cmap-reachability is irrelevant to it. Forces a `glyf` (TrueType) face
/// (DejaVu, not a CFF math font) so subsetting actually fires, and picks an
/// arbitrary but definitely-valid gid (glyph 3, comfortably below any
/// reasonable `maxp.numGlyphs`) as the "variant" — this test only cares
/// that the CID pipeline round-trips a non-cmap-driven gid post-subsetting,
/// not that gid 3 is any particular real MATH construction.
#[test]
fn math_variant_gid_survives_subsetting() {
    let path = need_font!();
    let store = TtfFontStore::load(&path, None, None).expect("load font");
    let face = store.face(FontKey(0)).expect("parse face");
    let num_glyphs = face.number_of_glyphs();
    if num_glyphs < 4 {
        eprintln!("skipping: face has too few glyphs to pick an arbitrary gid");
        return;
    }
    let variant_gid: u16 = 3;
    let advance = face.glyph_hor_advance(ttf_parser::GlyphId(variant_gid)).unwrap_or(0) as f64;
    let units_per_em = face.units_per_em() as f64;
    drop(face);

    let geometry = PageGeometry::default();
    let size = Length::pt(18.0);
    let glyph = satysfi_backend::MathGlyph {
        info: HorzStringInfo { font: FontKey(0), size, rising: Length::ZERO },
        // ToUnicode source char for this synthetic "variant" — arbitrary,
        // just needs to be searchable in `pdftotext` output.
        text: "+".to_string(),
        gid: Some(variant_gid),
        dx: Length::ZERO,
        dy: Length::ZERO,
        width: size * (advance / units_per_em),
        height: size * 0.7,
        depth: Length::ZERO,
    };
    let line = PlacedLine {
        x: geometry.text_origin.0,
        baseline_y: geometry.text_origin.1 + size,
        contents: vec![(
            Length::ZERO,
            PureHorzBox::Math {
                width: glyph.width,
                height: glyph.height,
                depth: glyph.depth,
                glyphs: vec![glyph],
                rules: vec![],
            },
        )],
    };
    let page = Page { lines: vec![line] };

    let pdf_bytes = render_pdf_ttf(&geometry, &[page], &store, &[]).expect(
        "a raw MATH-variant gid must render through the (now subsetting) CID pipeline",
    );
    assert!(pdf_bytes.starts_with(b"%PDF-"));
    // Subsetting fired (this is a `glyf` face): much smaller than the
    // source, exactly like the plain-text `render_pdf_ttf_produces_a_pdf_
    // with_embedded_font` case.
    let font_len = std::fs::metadata(&path).expect("stat font file").len() as usize;
    assert!(
        pdf_bytes.len() < font_len,
        "expected subsetting to fire for a `glyf` face even with a raw-gid Math box"
    );

    let Ok(which) = Command::new("which").arg("pdftotext").output() else {
        return;
    };
    if !which.status.success() {
        return;
    }
    let mut tmp = std::env::temp_dir();
    tmp.push(format!("satysfi-pdf-ttf-mathgid-test-{}.pdf", std::process::id()));
    std::fs::File::create(&tmp)
        .and_then(|mut f| f.write_all(&pdf_bytes))
        .expect("write temp pdf");
    let output = Command::new("pdftotext").arg(&tmp).arg("-").output().expect("run pdftotext");
    let _ = std::fs::remove_file(&tmp);
    assert!(output.status.success(), "pdftotext failed: {output:?}");
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(
        text.contains('+'),
        "pdftotext output missing the ToUnicode-mapped '+' for the raw variant gid, got {text:?}"
    );
}

/// `qpdf --check` (when the binary happens to be on `PATH` — not installed
/// on this dev machine, so this test is skip-gated exactly like the
/// `pdftotext` checks) validates the subsetted PDF's object/xref structure
/// end to end: a real, independent PDF parser, not just "our own writer
/// didn't panic".
#[test]
fn qpdf_check_accepts_a_subsetted_pdf() {
    let Ok(which) = Command::new("which").arg("qpdf").output() else {
        eprintln!("skipping: `which` not available");
        return;
    };
    if !which.status.success() {
        eprintln!("skipping: qpdf not on PATH");
        return;
    }
    let path = need_font!();
    let store = TtfFontStore::load(&path, None, None).expect("load font");
    let geometry = PageGeometry::default();
    let page = Page {
        lines: vec![build_line(&store, &geometry, "Hello World", FontKey(0))],
    };
    let pdf_bytes = render_pdf_ttf(&geometry, &[page], &store, &[]).expect("render");

    let mut tmp = std::env::temp_dir();
    tmp.push(format!("satysfi-pdf-ttf-qpdf-test-{}.pdf", std::process::id()));
    std::fs::File::create(&tmp)
        .and_then(|mut f| f.write_all(&pdf_bytes))
        .expect("write temp pdf");
    let output = Command::new("qpdf").arg("--check").arg(&tmp).output().expect("run qpdf");
    let _ = std::fs::remove_file(&tmp);
    assert!(
        output.status.success(),
        "qpdf --check reported problems: {}",
        String::from_utf8_lossy(&output.stderr)
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

    let pdf_bytes = render_pdf_ttf(&geometry, &[page], &store, &[]).expect("render");

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
