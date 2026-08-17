//! Integration tests for the ttf-parser-backed `FontMetrics` provider and
//! the CID-keyed PDF embedder. These need a real TrueType file on disk, so
//! every test locates one via fontconfig (falling back to a few common
//! paths) and skips gracefully — rather than failing the build — when none
//! is found.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use rustyfi_backend::{
    Color, FontKey, FontMetrics, HorzStringInfo, Length, Page, PageGeometry, PlacedLine,
    PureHorzBox,
};
use rustyfi_pdf::{render_pdf_ttf, TtfFontStore};

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
                info: HorzStringInfo { font, size, rising: Length::ZERO, color: Color::Gray(0.0) },
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
            body_lines: usize::MAX,
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
            body_lines: usize::MAX,
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
            body_lines: usize::MAX,
        lines: vec![build_line(&store, &geometry, "Reproducible", FontKey(0))],
    };
    let a = render_pdf_ttf(&geometry, &[page()], &store, &[]).expect("render 1");
    let b = render_pdf_ttf(&geometry, &[page()], &store, &[]).expect("render 2");
    assert_eq!(a, b, "two renders of the same document must be byte-identical");
}

/// A CFF-outline OpenType face (no `glyf` table at all — this host's
/// `NotoSansTagalog-Regular.otf`, a TeX Gyre `.otf`, or any single-face
/// non-collection `.otf` fontconfig turns up) now takes the
/// `CIDFontType0`/`FontFile3` embed path
/// (docs/plans/design-cff-embedding.md) rather than the invalid pre-design
/// `CIDFontType2`/`FontFile2` whole-file embed.
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

/// S1+S2 (docs/plans/design-cff-embedding.md §6.2, §6.3): a CFF-outline
/// OpenType face must embed as `CIDFontType0`/`/FontFile3 /Subtype
/// /OpenType`, with no `/CIDToGIDMap` at all (illegal for `CIDFontType0`) —
/// true whether the writer subsets it (S2) or falls back to a whole-OTF
/// embed (S1, when `subsetter::subset` declines this exact usage set, e.g. a
/// seac composite/CFF2 face). The size relationship below is derived from an
/// INDEPENDENT `subsetter::subset` call against the same single-glyph usage
/// set the writer itself would build, rather than hardcoding "always bigger"
/// (true only under S1) or "always smaller" (true only when S2's subsetting
/// succeeds) — see `subsetter_can_subset_cff_and_the_writer_now_uses_it`
/// below for the underlying primitive this cross-checks.
#[test]
fn cff_face_embeds_as_fontfile3_cidfonttype0() {
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
    // repertoire is arbitrary (Tagalog, Latin, or whatever fc-list found).
    let face = store.face(FontKey(0)).expect("parse face");
    // Excludes space/`.` from the probe set for THIS test specifically (kept
    // in the sibling tests below, which don't pdftotext-check a single
    // char): a lone space is a poor pdftotext round-trip probe (whitespace
    // trimming makes `text.contains(' ')` unreliable).
    let Some(c) = "Aa1".chars().find(|&c| face.glyph_index(c).is_some()) else {
        eprintln!("skipping: face has none of the trivial probe glyphs");
        return;
    };
    let probe_gid = face.glyph_index(c).expect("probe char has a gid").0;
    drop(face);
    let page = Page {
            body_lines: usize::MAX,
        lines: vec![build_line(&store, &geometry, &c.to_string(), FontKey(0))],
    };

    let pdf_bytes = render_pdf_ttf(&geometry, &[page], &store, &[])
        .expect("a CFF face must render via the CIDFontType0/FontFile3 path, not error");
    assert!(pdf_bytes.starts_with(b"%PDF-"));

    assert!(
        contains(&pdf_bytes, b"/FontFile3"),
        "expected a /FontFile3 stream for a CFF-outline face"
    );
    assert!(
        contains(&pdf_bytes, b"/CIDFontType0"),
        "expected a /CIDFontType0 descendant font for a CFF-outline face"
    );
    assert!(
        contains(&pdf_bytes, b"/OpenType"),
        "expected the FontFile3 stream to carry /Subtype /OpenType"
    );
    assert!(
        !contains(&pdf_bytes, b"/CIDToGIDMap"),
        "CIDFontType0 must NOT carry a /CIDToGIDMap (PDF 32000 9.7.4.2)"
    );
    // Whether the PDF should be smaller (S2 subsetting succeeded) or bigger
    // (S1 whole-OTF fallback) than the source face depends on whether THIS
    // font/usage combination is subsettable at all — determined by directly
    // calling the same `subsetter::subset` the writer itself calls, not
    // assumed.
    let font_len = std::fs::metadata(&path).expect("stat font file").len() as usize;
    let font_bytes = std::fs::read(&path).expect("read font file");
    let remapper = subsetter::GlyphRemapper::new_from_glyphs_sorted(&[probe_gid]);
    let subsetting_succeeds = subsetter::subset(&font_bytes, 0, &remapper).is_ok();
    if subsetting_succeeds {
        assert!(
            pdf_bytes.len() < font_len,
            "subsetter can subset this face's single-glyph usage, so the writer's S2 path \
             should have produced a SUBSET embed: PDF ({} bytes) should be smaller than the \
             source face ({} bytes)",
            pdf_bytes.len(),
            font_len,
        );
    } else {
        assert!(
            pdf_bytes.len() > font_len,
            "subsetter declines this face's single-glyph usage, so the writer's S1 fallback \
             should have produced a whole-OTF embed: PDF ({} bytes) should exceed the source \
             face ({} bytes)",
            pdf_bytes.len(),
            font_len,
        );
    }

    let Ok(which) = Command::new("which").arg("pdftotext").output() else {
        return;
    };
    if !which.status.success() {
        return;
    }
    let mut tmp = std::env::temp_dir();
    tmp.push(format!("rustyfi-pdf-cff-fontfile3-test-{}.pdf", std::process::id()));
    std::fs::File::create(&tmp)
        .and_then(|mut f| f.write_all(&pdf_bytes))
        .expect("write temp pdf");
    let output = Command::new("pdftotext").arg(&tmp).arg("-").output().expect("run pdftotext");
    let _ = std::fs::remove_file(&tmp);
    assert!(output.status.success(), "pdftotext failed: {output:?}");
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(
        text.contains(c),
        "pdftotext output missing the CFF-embedded probe char {c:?}, got {text:?}"
    );
}

/// S2 (docs/plans/design-cff-embedding.md §6.3): the pinned `subsetter`
/// crate subsets CFF fonts cleanly on its own — normalising its output to
/// CID-keyed with `CID == new_gid` — and `write_font_cff` now uses exactly
/// this (`cid.rs`'s module doc / `write_font_cff`'s doc comment describe the
/// two-pass content-remap this required). This test is decoupled from
/// `render_pdf_ttf`/`write_font_cff` entirely — it calls `subsetter::subset`
/// directly against the discovered CFF file — so it stays meaningful
/// regardless of which embed path the writer takes for a given face/usage
/// (S1's whole-OTF fallback still exists for the seac-composite/CFF2 faces
/// `subsetter` legitimately declines, exercised by the `Err` arm below).
#[test]
fn subsetter_can_subset_cff_and_the_writer_now_uses_it() {
    let Some(path) = find_cff_otf() else {
        eprintln!("skipping: no CFF-outline .otf found via fc-list on this system");
        return;
    };
    let bytes = std::fs::read(&path).expect("read font file");
    let Ok(face) = ttf_parser::Face::parse(&bytes, 0) else {
        eprintln!("skipping: {path:?} failed to parse as a face at all");
        return;
    };
    // Prefer a short Latin run (exercises several glyphs, more
    // representative of real subsetting) and fall back to a single probe
    // glyph, exactly like the sibling S1 test above.
    let text = ["Hello", "Aa1 ."]
        .into_iter()
        .find(|s| s.chars().all(|c| face.glyph_index(c).is_some()))
        .map(str::to_string)
        .or_else(|| {
            "Aa1 .".chars().find(|&c| face.glyph_index(c).is_some()).map(|c| c.to_string())
        });
    let Some(text) = text else {
        eprintln!("skipping: face has none of the trivial probe glyphs");
        return;
    };
    let glyphs: Vec<u16> = text
        .chars()
        .filter_map(|c| face.glyph_index(c).map(|g| g.0))
        .collect();
    drop(face);

    let remapper = subsetter::GlyphRemapper::new_from_glyphs_sorted(&glyphs);
    match subsetter::subset(&bytes, 0, &remapper) {
        Ok(subset) => {
            assert!(
                subset.len() < bytes.len(),
                "expected the subsetter's CFF output ({} bytes) to be smaller than the whole \
                 source OTF ({} bytes) for {path:?} (glyphs {glyphs:?})",
                subset.len(),
                bytes.len(),
            );
            assert_eq!(
                &subset[0..4],
                b"OTTO",
                "subsetter's CFF output should itself be a valid OTTO-flavoured sfnt"
            );
        }
        Err(e) => {
            // Not a test failure: `subsetter` legitimately declines some
            // glyph sets (seac composites, CFF2) — that's exactly the case
            // S1's whole-OTF fallback exists for.
            eprintln!(
                "subsetter declined {path:?} (glyphs {glyphs:?}): {e:?} — expected for e.g. \
                 seac composites/CFF2, S1's whole-OTF path covers this case"
            );
        }
    }
}

/// The actual writer output for a CFF face round-trips through `pdftotext`
/// regardless of whether the probe text is a single char or a short run —
/// whether this run's CID is the original face gid (S1 fallback) or
/// `subsetter`'s remapped gid (S2, `write_font_cff`'s doc comment) is an
/// internal detail: `/W`, ToUnicode, and content all key off the SAME CID
/// either way, so pdftotext extraction is unaffected. This is really
/// re-confirming `cff_face_embeds_as_fontfile3_cidfonttype0`'s round trip
/// with a richer (multi-glyph) probe text.
#[test]
fn cff_face_multi_glyph_text_roundtrips_through_pdftotext() {
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
    let face = store.face(FontKey(0)).expect("parse face");
    let text = ["Hello", "Aa1 ."]
        .into_iter()
        .find(|s| s.chars().all(|c| face.glyph_index(c).is_some()))
        .map(str::to_string)
        .or_else(|| {
            "Aa1 .".chars().find(|&c| face.glyph_index(c).is_some()).map(|c| c.to_string())
        });
    let Some(text) = text else {
        eprintln!("skipping: face has none of the trivial probe glyphs");
        return;
    };
    drop(face);

    let page = Page {
            body_lines: usize::MAX,
        lines: vec![build_line(&store, &geometry, &text, FontKey(0))],
    };
    let pdf_bytes = render_pdf_ttf(&geometry, &[page], &store, &[])
        .expect("a CFF face must render via the CIDFontType0/FontFile3 path");
    assert!(pdf_bytes.starts_with(b"%PDF-"));

    let Ok(which) = Command::new("which").arg("pdftotext").output() else {
        return;
    };
    if !which.status.success() {
        return;
    }
    let mut tmp = std::env::temp_dir();
    tmp.push(format!("rustyfi-pdf-cff-multiglyph-test-{}.pdf", std::process::id()));
    std::fs::File::create(&tmp)
        .and_then(|mut f| f.write_all(&pdf_bytes))
        .expect("write temp pdf");
    let output = Command::new("pdftotext").arg(&tmp).arg("-").output().expect("run pdftotext");
    let _ = std::fs::remove_file(&tmp);
    assert!(output.status.success(), "pdftotext failed: {output:?}");
    let extracted = String::from_utf8_lossy(&output.stdout);
    assert!(
        extracted.contains(text.trim()),
        "pdftotext output missing {text:?}, got {extracted:?}"
    );
}

/// Byte-search helper for the structural CFF assertions above — the tests
/// only need "does this marker appear anywhere in the serialized PDF", not a
/// real PDF parse (mirrors how `subsetted_base_font_carries_a_subset_tag`
/// above already string-searches the raw bytes).
fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// The bundled `lmsans` face (real Latin Modern Sans, `scripts/download-fonts.sh`
/// — replaces the old Noto glyf stand-in now that CFF embedding exists,
/// docs/plans/design-cff-embedding.md) is itself a CFF-outline OpenType face,
/// so it must take the same `CIDFontType0`/`/FontFile3` path as the
/// fontconfig-discovered CFF faces above — this is the concrete, named
/// abbrev a `set-font` call actually resolves to, not just "some CFF found on
/// the host". Skips gracefully if `scripts/download-fonts.sh` hasn't been run
/// in this checkout (the font is gitignored, not committed).
///
/// Also the S2 SIZE-WIN check (docs/plans/design-cff-embedding.md §8): the
/// probe text "Latin Modern Sans" uses only a small fraction of lmsans's
/// full glyph repertoire (Latin Modern ships hundreds of glyphs — accents,
/// ligatures, extended Latin, etc.), a genuine "multi-glyph-but-not-all-
/// glyphs" document, so a real subset embed must be substantially SMALLER
/// than the whole source OTF — unlike `cff_face_embeds_as_fontfile3_cidfonttype0`'s
/// single-glyph probe (which can't rule out subsetting having declined this
/// particular face), this is deterministic and host-independent (a bundled,
/// checked-in-by-download font, not fontconfig's arbitrary pick).
#[test]
fn lmsans_bundled_font_embeds_as_fontfile3_cidfonttype0() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../lib-rustyfi/dist/fonts/lmsans10-regular.otf");
    if !path.is_file() {
        eprintln!(
            "skipping: {path:?} not present — run `scripts/download-fonts.sh` first \
             (font binaries are gitignored, not committed)"
        );
        return;
    }
    let store = TtfFontStore::load(&path, None, None).expect("load bundled lmsans10-regular.otf");
    let face = store.face(FontKey(0)).expect("parse bundled lmsans face");
    assert!(
        face.tables().glyf.is_none() && face.tables().cff.is_some(),
        "lmsans10-regular.otf is expected to be CFF-outline (no glyf table)"
    );
    drop(face);

    let geometry = PageGeometry::default();
    let text = "Latin Modern Sans";
    let page = Page {
            body_lines: usize::MAX,
        lines: vec![build_line(&store, &geometry, text, FontKey(0))],
    };
    let pdf_bytes = render_pdf_ttf(&geometry, &[page], &store, &[])
        .expect("lmsans (a CFF face) must render via the CIDFontType0/FontFile3 path");
    assert!(pdf_bytes.starts_with(b"%PDF-"));

    assert!(
        contains(&pdf_bytes, b"/FontFile3"),
        "expected a /FontFile3 stream for the lmsans CFF-outline face"
    );
    assert!(
        contains(&pdf_bytes, b"/CIDFontType0"),
        "expected a /CIDFontType0 descendant font for the lmsans CFF-outline face"
    );
    assert!(
        contains(&pdf_bytes, b"/OpenType"),
        "expected the FontFile3 stream to carry /Subtype /OpenType"
    );
    assert!(
        !contains(&pdf_bytes, b"/CIDToGIDMap"),
        "CIDFontType0 must NOT carry a /CIDToGIDMap (PDF 32000 9.7.4.2)"
    );

    // S2 size win: a real subset embed of a handful of glyphs must be
    // substantially smaller than the whole (hundreds-of-glyphs) source OTF —
    // see this test's doc comment.
    let font_len = std::fs::metadata(&path).expect("stat lmsans font file").len() as usize;
    assert!(
        pdf_bytes.len() < font_len,
        "expected S2 real CFF subsetting: the PDF ({} bytes, embedding only the glyphs \
         {text:?} uses) should be SMALLER than the whole lmsans OTF ({} bytes) — this is the \
         size win S2 has over S1's always-whole-OTF embed",
        pdf_bytes.len(),
        font_len,
    );

    let Ok(which) = Command::new("which").arg("pdftotext").output() else {
        return;
    };
    if !which.status.success() {
        return;
    }
    let mut tmp = std::env::temp_dir();
    tmp.push(format!("rustyfi-pdf-lmsans-fontfile3-test-{}.pdf", std::process::id()));
    std::fs::File::create(&tmp)
        .and_then(|mut f| f.write_all(&pdf_bytes))
        .expect("write temp pdf");
    let output = Command::new("pdftotext").arg(&tmp).arg("-").output().expect("run pdftotext");
    let _ = std::fs::remove_file(&tmp);
    assert!(output.status.success(), "pdftotext failed: {output:?}");
    let extracted = String::from_utf8_lossy(&output.stdout);
    assert!(
        extracted.contains(text),
        "pdftotext output missing {text:?}, got {extracted:?}"
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
    let glyph = rustyfi_backend::MathGlyph {
        info: HorzStringInfo { font: FontKey(0), size, rising: Length::ZERO, color: Color::Gray(0.0) },
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
    let page = Page {
            body_lines: usize::MAX, lines: vec![line] };

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
    tmp.push(format!("rustyfi-pdf-ttf-mathgid-test-{}.pdf", std::process::id()));
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
            body_lines: usize::MAX,
        lines: vec![build_line(&store, &geometry, "Hello World", FontKey(0))],
    };
    let pdf_bytes = render_pdf_ttf(&geometry, &[page], &store, &[]).expect("render");

    let mut tmp = std::env::temp_dir();
    tmp.push(format!("rustyfi-pdf-ttf-qpdf-test-{}.pdf", std::process::id()));
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
            body_lines: usize::MAX,
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
    tmp.push(format!("rustyfi-pdf-ttf-test-{}.pdf", std::process::id()));
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
