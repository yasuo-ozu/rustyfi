//! (MATH-table `MathVariants` — big-operator vertical variants + stretchy
//! delimiters): tests for `TtfFontStore::math_vertical_variant`
//! (`VertVariantPolicy::BigOp`/ `AtLeast`),
//! `push_big_char_glyph`/`push_delimiter_glyph`
//! (`rustyfi-lang/src/primitives.rs`), and the `MathGlyph.gid` raw-gid
//! channel through the real CID pipeline (`render_pdf_ttf`).
//!
//! Font discovery mirrors `tests/math_font.rs`/`tests/math_fraction_radical.rs`
//! (copied, not shared): fontconfig first, then common distro/nix paths, then
//! a graceful skip. Unlike the shared `find_math_font` (whichever family
//! resolves first), this file's unit tests need BOTH fonts independently — DejaVu
//! Math TeX Gyre exercises the glyf path, Noto Sans Math the CFF
//! `glyph_bounding_box` path (`ttf-parser` lib.rs:2172) — so this file adds
//! per-family locators alongside the shared either-font one.

use std::path::{Path, PathBuf};
use std::process::Command;

use rustyfi_backend::{FontKey, FontMetrics, HorzBox, Length, Page, PageGeometry, PlacedLine, PureHorzBox, VertVariantPolicy};
use rustyfi_lang::value::Value;
use rustyfi_lang::{elaborate, eval, primitives, typecheck, CompileError};
use rustyfi_pdf::{render_pdf_ttf, TtfFontStore};
use ttf_parser::Face;

/// Independently replicates `TtfFontStore::math_vertical_variant`'s
/// `VertVariantPolicy::AtLeast` selection (`ttf.rs`): the smallest
/// `vertical_constructions` record whose `advance_measurement` (design
/// units, scaled by `size`/`units_per_em`) covers `target`, else the
/// largest record — checks the SELECTION independently of what
/// `MathVariantGlyph.advance` happens to mean (see the `vertical_variant_unit_*`
/// tests' own comment on why `.advance` isn't the right thing to assert
/// `>= target` against).
fn expected_at_least_gid(face: &Face, c: char, size: Length, target: Length) -> u16 {
    let gid = face.glyph_index(c).expect("cmap has the char");
    let construction = face
        .tables()
        .math
        .expect("MATH table")
        .variants
        .expect("MathVariants subtable")
        .vertical_constructions
        .get(gid)
        .expect("char has a vertical GlyphConstruction");
    let n = construction.variants.len();
    let upem = face.units_per_em() as f64;
    let min_du = (target.0 / size.0) * upem;
    let mut chosen = construction
        .variants
        .get(n - 1)
        .expect("at least one variant record")
        .variant_glyph
        .0;
    for i in 0..n {
        let v = construction.variants.get(i).expect("index < n");
        if v.advance_measurement as f64 >= min_du {
            chosen = v.variant_glyph.0;
            break;
        }
    }
    chosen
}

// ----------------------------------------------------------------------
// Font discovery.
// ----------------------------------------------------------------------

fn find_family(family: &str, fallbacks: &[&str]) -> Option<PathBuf> {
    if let Ok(output) = Command::new("fc-match")
        .args(["--format=%{file}", family])
        .output()
    {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path.is_empty()
                && Path::new(&path).is_file()
                && (path.contains("Math") || path.contains("math"))
            {
                return Some(PathBuf::from(path));
            }
        }
    }
    for candidate in fallbacks {
        if Path::new(candidate).is_file() {
            return Some(PathBuf::from(candidate));
        }
    }
    None
}

fn find_dejavu_math() -> Option<PathBuf> {
    find_family(
        "DejaVu Math TeX Gyre",
        &[
            "/usr/share/texmf/fonts/opentype/public/dejavu-otf/DejaVuMathTeXGyre.ttf",
            "/usr/share/fonts/opentype/dejavu-math-tex-gyre/DejaVuMathTeXGyre.ttf",
            "/usr/share/fonts/truetype/tex-gyre/texgyredejavu-math.otf",
        ],
    )
}

fn find_noto_math() -> Option<PathBuf> {
    find_family(
        "Noto Sans Math",
        &[
            "/usr/share/fonts/noto/NotoSansMath-Regular.ttf",
            "/usr/share/fonts/truetype/noto/NotoSansMath-Regular.ttf",
            "/usr/share/fonts/opentype/noto/NotoSansMath-Regular.ttf",
            "/usr/share/fonts/OTF/NotoSansMath-Regular.otf",
            "/usr/share/fonts/noto-fonts/NotoSansMath-Regular.ttf",
            "/run/current-system/sw/share/fonts/truetype/NotoSansMath-Regular.ttf",
        ],
    )
}

/// A CFF-outline MATH font for the e2e tests, mirroring `math_font.rs`'s
/// `find_math_font`. Prefers the bundled Latin Modern Math
/// (`lib-rustyfi/dist/fonts/latinmodern-math.otf`, `download-fonts.sh`'s
/// default), so these depend on that script having been run rather than on a
/// host-wide font install; only falls through to fontconfig when it hasn't.
///
/// The face must be CFF (`OTTO`), and that is not fussiness. The assertions
/// derive their gids from the loaded font, but the CIDs they expect come from
/// `expected_cid`, which models `cid.rs`'s `write_font_cff` subsetting — a
/// `glyf` face takes the `CIDFontType2`/`FontFile2` path instead and emits
/// its gids unremapped, so every CID expectation here would be wrong for it.
/// A host with only a `glyf` math face (a CI runner with no bundled fonts,
/// which finds Noto Sans Math through fontconfig) therefore SKIPS rather than
/// failing on a difference that is not a defect.
fn find_math_font() -> Option<PathBuf> {
    let is_cff = |p: &Path| {
        std::fs::read(p)
            .map(|b| b.starts_with(b"OTTO"))
            .unwrap_or(false)
    };

    let bundled_lmmath = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../lib-rustyfi/dist/fonts/latinmodern-math.otf");
    if bundled_lmmath.is_file() && is_cff(&bundled_lmmath) {
        return Some(bundled_lmmath);
    }

    for family in ["Noto Sans Math", "DejaVu Math TeX Gyre"] {
        if let Some(p) = find_family(family, &[]) {
            if is_cff(&p) {
                return Some(p);
            }
        }
    }
    find_dejavu_math()
        .or_else(find_noto_math)
        .filter(|p| is_cff(p))
}

macro_rules! need_font {
    ($finder:expr, $label:expr) => {
        match $finder {
            Some(path) => path,
            None => {
                eprintln!(
                    "skipping: no {} font found on this system (tried fc-match \
                     and common nix/distro paths)",
                    $label
                );
                return;
            }
        }
    };
}

// ----------------------------------------------------------------------
// Unit: `TtfFontStore::math_vertical_variant`.
// ----------------------------------------------------------------------

/// Runs every unit assertion against one font, panicking (with the font
/// path in every message) so a `#[test]` per font gives an attributable
/// failure.
fn assert_vertical_variant_unit(path: &Path) {
    let store = TtfFontStore::load(path, None, None).expect("load math font");
    let face = store.face(FontKey(0)).expect("parse face");
    let size = Length::pt(12.0);
    let upem = face.units_per_em() as f64;

    // -- BigOp('∑'): gid != base cmap gid, IS a member of the enumerated
    // vertical_constructions variant list, height+depth > the base
    // record's (record[0], enumerated — not hardcoded).
    let sum_gid = face
        .glyph_index('∑')
        .unwrap_or_else(|| panic!("{path:?}: cmap has no ∑"));
    let math_table = face
        .tables()
        .math
        .unwrap_or_else(|| panic!("{path:?}: no MATH table"));
    let variants_table = math_table
        .variants
        .unwrap_or_else(|| panic!("{path:?}: no MathVariants subtable"));
    let sum_construction = variants_table
        .vertical_constructions
        .get(sum_gid)
        .unwrap_or_else(|| panic!("{path:?}: ∑ has no vertical GlyphConstruction"));
    let n = sum_construction.variants.len();
    assert!(
        n >= 1,
        "{path:?}: expected at least one prepared variant record for ∑, got {n}"
    );
    let sum_variant_gids: Vec<u16> = sum_construction
        .variants
        .into_iter()
        .map(|v| v.variant_glyph.0)
        .collect();
    let base_rec = sum_construction
        .variants
        .get(0)
        .expect("record[0] must exist since n >= 1");
    let base_bbox = face
        .glyph_bounding_box(base_rec.variant_glyph)
        .unwrap_or_else(|| panic!("{path:?}: base ∑ record has no bbox"));
    let base_h = size.0 * (base_bbox.y_max.max(0) as f64) / upem;
    let base_d = size.0 * ((-(base_bbox.y_min.min(0) as i32)) as f64) / upem;

    let big = store
        .math_vertical_variant(FontKey(0), '∑', size, VertVariantPolicy::BigOp)
        .unwrap_or_else(|| panic!("{path:?}: expected Some for BigOp('∑')"));
    assert_ne!(
        big.gid, sum_gid.0,
        "{path:?}: BigOp variant gid should differ from the plain cmap gid"
    );
    assert!(
        sum_variant_gids.contains(&big.gid),
        "{path:?}: BigOp gid {} not among ∑'s enumerated variant records {sum_variant_gids:?}",
        big.gid
    );
    assert!(
        big.height.0 + big.depth.0 > base_h + base_d,
        "{path:?}: BigOp variant (h+d={}) should exceed record[0]'s (h+d={})",
        big.height.0 + big.depth.0,
        base_h + base_d
    );

    // -- AtLeast(2.0*size) on '(': advance >= 2.0*size.
    let paren_gid = face
        .glyph_index('(')
        .unwrap_or_else(|| panic!("{path:?}: cmap has no '('"));
    let paren_construction = variants_table
        .vertical_constructions
        .get(paren_gid)
        .unwrap_or_else(|| panic!("{path:?}: '(' has no vertical GlyphConstruction"));
    let record0_gid = paren_construction
        .variants
        .get(0)
        .unwrap_or_else(|| panic!("{path:?}: '(' construction has no record[0]"))
        .variant_glyph
        .0;

    // `MathVariantGlyph.advance` is the variant glyph's own horizontal
    // advance width (`face.glyph_hor_advance`, hmtx-based — matching
    // upstream `fontFormat.ml`'s `get_math_glyph_metrics`), not the
    // OpenType `advance_measurement` used to SELECT the record. For a
    // narrow glyph like `(` that only grows taller (not much wider), it
    // does NOT itself grow past `target` (a VERTICAL measurement) — so we
    // check that the SELECTION POLICY picked the right record
    // (independently replicated via `expected_at_least_gid`, directly off
    // `advance_measurement`), not that `.advance` exceeds `target`.
    let target = size * 2.0;
    let expected_gid = expected_at_least_gid(&face, '(', size, target);
    // Sanity: for this to be a meaningful test, `target` must actually
    // exceed record[0]'s own coverage (else the "AtLeast" policy would
    // trivially degenerate to the `AtLeast(tiny)` case below).
    assert_ne!(
        expected_gid, record0_gid,
        "{path:?}: test target {target:?} should force a non-record[0] selection \
         (pick a larger target if this ever fails)"
    );
    let at_least = store
        .math_vertical_variant(FontKey(0), '(', size, VertVariantPolicy::AtLeast(target))
        .unwrap_or_else(|| panic!("{path:?}: expected Some for AtLeast(2*size) on '('"));
    assert_eq!(
        at_least.gid, expected_gid,
        "{path:?}: AtLeast(2*size) should select the smallest record whose \
         advance_measurement covers 2*size (independently computed gid {expected_gid}), got {}",
        at_least.gid
    );
    // `.advance` is still self-consistent: exactly the SELECTED variant
    // glyph's own hmtx horizontal advance, scaled.
    let expected_advance = size
        * (face
            .glyph_hor_advance(ttf_parser::GlyphId(at_least.gid))
            .expect("selected variant has an hmtx advance") as f64
            / upem);
    assert!(
        (at_least.advance.0 - expected_advance.0).abs() < 1e-6,
        "{path:?}: `.advance` should be the selected variant glyph's own hmtx advance \
         ({expected_advance:?}), got {:?}",
        at_least.advance
    );

    // -- AtLeast(tiny) on '(': returns record[0].
    let tiny = Length::pt(1e-6);
    let at_tiny = store
        .math_vertical_variant(FontKey(0), '(', size, VertVariantPolicy::AtLeast(tiny))
        .unwrap_or_else(|| panic!("{path:?}: expected Some for AtLeast(tiny) on '('"));
    assert_eq!(
        at_tiny.gid, record0_gid,
        "{path:?}: AtLeast(tiny) should return record[0]'s gid ({record0_gid}), got {}",
        at_tiny.gid
    );

    // -- no-construction char ('a'): None.
    let none = store.math_vertical_variant(FontKey(0), 'a', size, VertVariantPolicy::BigOp);
    assert!(
        none.is_none(),
        "{path:?}: expected None for 'a' (no vertical construction), got {none:?}"
    );
}

// ----------------------------------------------------------------------
// Unit: `TtfFontStore::math_vertical_assembly`
// (GlyphAssembly stretch beyond the largest discrete variant).
// ----------------------------------------------------------------------

/// Independently read `(`'s vertical `GlyphAssembly` off the font (ttf-parser,
/// NO hardcoded gids): the ordered part gids bottom-to-top, which of them are
/// extenders, the min connector overlap, and the largest discrete variant's
/// `advance_measurement` (so the test can pick a `target` guaranteed to lie
/// beyond it).
struct ParenAssembly {
    part_gids: Vec<u16>,
    extender_gids: Vec<u16>,
    non_extender_gids: Vec<u16>,
    largest_variant_advance_du: f64,
    upem: f64,
}

fn read_paren_assembly(face: &Face, c: char) -> ParenAssembly {
    let gid = face.glyph_index(c).expect("cmap has the char");
    let variants = face
        .tables()
        .math
        .expect("MATH table")
        .variants
        .expect("MathVariants subtable");
    let construction = variants
        .vertical_constructions
        .get(gid)
        .expect("char has a vertical GlyphConstruction");
    let assembly = construction
        .assembly
        .expect("this delimiter has a GlyphAssembly (both DejaVu Math & Noto Math do)");
    let mut part_gids = Vec::new();
    let mut extender_gids = Vec::new();
    let mut non_extender_gids = Vec::new();
    for p in assembly.parts {
        part_gids.push(p.glyph_id.0);
        if p.part_flags.extender() {
            extender_gids.push(p.glyph_id.0);
        } else {
            non_extender_gids.push(p.glyph_id.0);
        }
    }
    let n = construction.variants.len();
    let largest_variant_advance_du = if n == 0 {
        0.0
    } else {
        construction
            .variants
            .get(n - 1)
            .expect("largest variant")
            .advance_measurement as f64
    };
    ParenAssembly {
        part_gids,
        extender_gids,
        non_extender_gids,
        largest_variant_advance_du,
        upem: face.units_per_em() as f64,
    }
}

fn assert_vertical_assembly_unit(path: &Path) {
    let store = TtfFontStore::load(path, None, None).expect("load math font");
    let face = store.face(FontKey(0)).expect("parse face");
    let size = Length::pt(12.0);

    let asm = read_paren_assembly(&face, '(');
    assert!(
        !asm.extender_gids.is_empty(),
        "{path:?}: '(' assembly should have at least one extender part"
    );
    assert!(
        !asm.non_extender_gids.is_empty(),
        "{path:?}: '(' assembly should have at least one non-extender (hook) part"
    );

    // A target well beyond the largest DISCRETE variant record: at 12pt the
    // largest '(' variant is `largest_variant_advance_du * size/upem`; ask for
    // ~6x that so the assembly must repeat the extender many times.
    let largest_pt = size.0 * asm.largest_variant_advance_du / asm.upem;
    let target = Length::pt(largest_pt * 6.0);

    let parts = store
        .math_vertical_assembly(FontKey(0), '(', size, target)
        .unwrap_or_else(|| panic!("{path:?}: expected Some assembly for a tall '('"));

    // Every placed part gid must belong to `(`'s enumerated assembly parts.
    for (gid, _dy, _adv) in &parts {
        assert!(
            asm.part_gids.contains(gid),
            "{path:?}: placed part gid {gid} is not among '('s assembly parts {:?}",
            asm.part_gids
        );
    }

    // MULTIPLE part gids: top hook + repeated extenders + bottom hook.
    assert!(
        parts.len() > asm.part_gids.len(),
        "{path:?}: a very tall '(' must emit MORE placed parts ({}) than the {} distinct \
         assembly parts — i.e. the extender is repeated",
        parts.len(),
        asm.part_gids.len()
    );
    let placed_gids: Vec<u16> = parts.iter().map(|(g, _, _)| *g).collect();
    // Each non-extender (hook) part appears exactly once.
    for hook in &asm.non_extender_gids {
        let count = placed_gids.iter().filter(|g| *g == hook).count();
        assert_eq!(
            count, 1,
            "{path:?}: non-extender hook gid {hook} should be placed exactly once, got {count}"
        );
    }
    // At least one extender part is repeated (appears more than once).
    let repeated_extender = asm
        .extender_gids
        .iter()
        .any(|ext| placed_gids.iter().filter(|g| *g == ext).count() > 1);
    assert!(
        repeated_extender,
        "{path:?}: a very tall '(' must repeat an extender part; placed gids = {placed_gids:?}"
    );

    // The stacked assembly covers `target` (bottom baseline 0 .. top part's
    // baseline+advance), and parts stack monotonically upward.
    let total = parts
        .last()
        .map(|(_, dy, adv)| dy.0 + adv.0)
        .expect("at least one part");
    assert!(
        total >= target.0 - 1e-6,
        "{path:?}: stacked assembly extent ({total}) should cover target ({})",
        target.0
    );
    let mut prev_dy = f64::NEG_INFINITY;
    for (_, dy, _) in &parts {
        assert!(
            dy.0 >= prev_dy - 1e-6,
            "{path:?}: parts must be placed bottom-to-top (non-decreasing dy)"
        );
        prev_dy = dy.0;
    }

    // A non-stretchy char with no vertical construction/assembly ('a'): None.
    assert!(
        store
            .math_vertical_assembly(FontKey(0), 'a', size, target)
            .is_none(),
        "{path:?}: 'a' has no assembly -> None"
    );
}

#[test]
fn vertical_assembly_unit_dejavu() {
    let path = need_font!(find_dejavu_math(), "DejaVu Math TeX Gyre");
    assert_vertical_assembly_unit(&path);
}

#[test]
fn vertical_assembly_unit_noto() {
    let path = need_font!(find_noto_math(), "Noto Sans Math");
    assert_vertical_assembly_unit(&path);
}

#[test]
fn vertical_variant_unit_dejavu() {
    let path = need_font!(find_dejavu_math(), "DejaVu Math TeX Gyre");
    assert_vertical_variant_unit(&path);
}

#[test]
fn vertical_variant_unit_noto() {
    let path = need_font!(find_noto_math(), "Noto Sans Math");
    assert_vertical_variant_unit(&path);
}

// ----------------------------------------------------------------------
// Pipeline helpers (mirrors `math_font.rs`/`math_fraction_radical.rs`'s
// `run_math`/`with_ctx`/`math_box`).
// ----------------------------------------------------------------------

fn run_math(src: &str, metrics: &dyn FontMetrics) -> Result<Value, CompileError> {
    let file = rustyfi_syntax::parse_file(src)?;
    let env = primitives::base_env();
    let store = rustyfi_lang::symbol::SymbolStore::new();
    let scope = elaborate::Scope::new(&store, env.names());
    let program = elaborate::elaborate_program(&file, &scope)?;
    typecheck::typecheck(&program)?;
    let mut interp = eval::Interp::new(metrics);
    Ok(interp.eval(&env, &rustyfi_lang::ast::debrand(&program.body, &store))?)
}

fn with_ctx(body: &str) -> String {
    format!(
        "let-inline ctx \\dummy m = inline-nil\n\
         in\n\
         let ctx = get-initial-context 200pt (command \\dummy) in\n\
         {body}"
    )
}

fn math_box(v: Value) -> PureHorzBox {
    match v {
        Value::InlineBoxes(boxes) => {
            assert_eq!(boxes.len(), 1, "expected exactly one box, got {boxes:?}");
            match boxes.into_iter().next().unwrap() {
                HorzBox::Pure(m @ PureHorzBox::Math { .. }) => m,
                other => panic!("expected a PureHorzBox::Math, got {other:?}"),
            }
        }
        other => panic!("expected inline-boxes, got {other:?}"),
    }
}

fn as_math_parts(bx: PureHorzBox) -> (Length, Length, Length, Vec<rustyfi_backend::MathGlyph>) {
    match bx {
        PureHorzBox::Math {
            width,
            height,
            depth,
            glyphs,
            ..
        } => (width, height, depth, glyphs),
        other => panic!("expected PureHorzBox::Math, got {other:?}"),
    }
}

fn page_for(bx: PureHorzBox, geometry: &PageGeometry) -> Page {
    Page {
            body_lines: usize::MAX,
        lines: vec![PlacedLine {
            x: geometry.text_origin.0,
            baseline_y: geometry.text_origin.1 + Length::pt(60.0),
            contents: vec![(Length::ZERO, bx)],
        }],
    }
}

// ----------------------------------------------------------------------
// PDF content-stream introspection: byte-exact reproduction of
// `pdf-writer` 0.13's `Str::write` escaping (verified against
// `pdf-writer-0.13.0/src/object.rs`) so we can search the REAL emitted
// bytes for a specific 2-byte-BE glyph id's `Tj` operand — a lossy-UTF8
// string search would corrupt non-ASCII glyph-id bytes, and a naive raw
// search risks a false hit inside the also-embedded (whole, unsubsetted)
// font file. We narrow to just the content stream: `render_pdf_ttf` embeds
// exactly two `stream`/`endstream` objects for a single-page, no-image
// document (content stream + font file), and the content stream is always
// the smaller of the two.
// ----------------------------------------------------------------------

fn pdf_str_repr(bytes: &[u8]) -> Vec<u8> {
    if bytes.iter().all(|b| b.is_ascii()) {
        let is_balanced = {
            let mut depth = 0i32;
            let mut ok = true;
            for &b in bytes {
                match b {
                    b'(' => depth += 1,
                    b')' => {
                        if depth > 0 {
                            depth -= 1;
                        } else {
                            ok = false;
                        }
                    }
                    _ => {}
                }
            }
            ok && depth == 0
        };
        let mut out = vec![b'('];
        let mut balanced_flag: Option<bool> = None;
        for &byte in bytes {
            match byte {
                b'(' | b')' => {
                    let bal =
                        *balanced_flag.get_or_insert_with(|| byte != b')' && is_balanced);
                    if !bal {
                        out.push(b'\\');
                    }
                    out.push(byte);
                }
                b'\\' => out.extend(b"\\\\"),
                b' '..=b'~' => out.push(byte),
                b'\n' => out.extend(b"\\n"),
                b'\r' => out.extend(b"\\r"),
                b'\t' => out.extend(b"\\t"),
                0x08 => out.extend(b"\\b"),
                0x0c => out.extend(b"\\f"),
                _ => {
                    out.push(b'\\');
                    out.push(b'0' + (byte >> 6));
                    out.push(b'0' + ((byte >> 3) & 7));
                    out.push(b'0' + (byte & 7));
                }
            }
        }
        out.push(b')');
        out
    } else {
        let mut out = vec![b'<'];
        let hex = |b: u8| -> u8 {
            if b < 10 {
                b'0' + b
            } else {
                b'A' + (b - 10)
            }
        };
        for &byte in bytes {
            out.push(hex(byte >> 4));
            out.push(hex(byte & 0xF));
        }
        out.push(b'>');
        out
    }
}

fn contains_subslice(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || needle.len() > haystack.len() {
        return false;
    }
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// All `stream`/`endstream` payloads in a raw PDF byte buffer.
fn extract_streams(pdf: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(rel) = find_subslice(&pdf[i..], b"stream") {
        let mut start = i + rel + b"stream".len();
        if pdf.get(start) == Some(&b'\r') {
            start += 1;
        }
        if pdf.get(start) == Some(&b'\n') {
            start += 1;
        }
        match find_subslice(&pdf[start..], b"endstream") {
            Some(end_rel) => {
                let end = start + end_rel;
                out.push(&pdf[start..end]);
                i = end + b"endstream".len();
            }
            None => break,
        }
    }
    out
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// The content stream out of `extract_streams`' output: for a single-page,
/// no-image document the ONLY other stream is the (much larger)
/// unsubsetted embedded font file.
fn content_stream(pdf: &[u8]) -> Vec<u8> {
    let streams = extract_streams(pdf);
    let content = streams
        .iter()
        .min_by_key(|s| s.len())
        .expect("expected at least one PDF stream object");
    assert!(
        streams.len() >= 2,
        "expected >= 2 stream objects (content + embedded font), got {}",
        streams.len()
    );
    // Sanity: the picked stream really does look like a content stream, not
    // (e.g.) a pathologically small font table.
    assert!(
        contains_subslice(content, b"BT") && contains_subslice(content, b"Tf"),
        "the shortest stream object doesn't look like a content stream \
         (missing BT/Tf) -- got {} bytes: {:?}",
        content.len(),
        String::from_utf8_lossy(content)
    );
    content.to_vec()
}

fn approx(a: Length, b: Length, tol: f64) -> bool {
    (a.0 - b.0).abs() < tol
}

// ----------------------------------------------------------------------
// Gid-remap-robust CID prediction: a CFF-outline math font (this file's
// "Noto Sans Math" case) takes the writer's `write_font_cff` subsetting path, which
// emits `subsetter`'s REMAPPED gid (CID == new gid) as the content-stream
// CID rather than the raw face gid — CFF has no `/CIDToGIDMap` to hide the
// renumbering behind (`cid.rs`'s module doc). `original_gids_used`/
// `expected_cid` below independently replicate EXACTLY the two calls
// `render_pdf_ttf_with`/`write_font_cff` make
// (`subsetter::GlyphRemapper::new_from_glyphs_sorted` + `subsetter::subset`)
// against the same per-font usage set a test's page actually contains, so
// assertions are never against a hardcoded CID, which would silently rot
// when fontconfig resolves a different "Noto Sans Math" build.

/// The ORIGINAL face gid every `MathGlyph` in `glyphs` resolves to — a raw
/// MATH-table variant (`gid: Some(_)`) is used directly, otherwise each
/// char of `.text` goes through `face.glyph_index`, mirroring `emit_box`'s
/// `Math` arm / `encode_glyph_run` (`cid.rs`). Since every test below
/// renders a single-box page against a single physical font file
/// (`FontKey(0)`), this is exactly the `usage.glyphs` key set
/// `render_pdf_ttf_with`'s Pass 1a would build for that file.
fn original_gids_used(face: &Face, glyphs: &[rustyfi_backend::MathGlyph]) -> Vec<u16> {
    glyphs
        .iter()
        .flat_map(|g| match g.gid {
            Some(gid) => vec![gid],
            None => g
                .text
                .chars()
                .map(|c| face.glyph_index(c).expect("MathGlyph.text char has a gid").0)
                .collect(),
        })
        .collect()
}

/// The exact CID `write_font_cff`'s subsetting will emit for
/// `original_gid`, given the full usage set (`used_gids`, from
/// `original_gids_used` above) — computed via the same
/// `subsetter::GlyphRemapper::new_from_glyphs_sorted` + `subsetter::subset`
/// calls `render_pdf_ttf_with`/`write_font_cff` make (`cid.rs`). Falls back
/// to `original_gid` unchanged when subsetting fails (mirrors
/// `write_font_cff`'s whole-OTF fallback for a seac composite/CFF2 face).
fn expected_cid(font_bytes: &[u8], used_gids: &[u16], original_gid: u16) -> u16 {
    let remapper = subsetter::GlyphRemapper::new_from_glyphs_sorted(used_gids);
    match subsetter::subset(font_bytes, 0, &remapper) {
        Ok(_) => remapper.get(original_gid).unwrap_or(original_gid),
        Err(_) => original_gid,
    }
}

// ----------------------------------------------------------------------
// e2e: `∑` grows via `math-big-char`.
// ----------------------------------------------------------------------

#[test]
fn big_char_sum_variant_grows_and_emits_variant_gid() {
    let path = need_font!(find_math_font(), "MATH");
    let store = TtfFontStore::load(&path, None, None).expect("load math font");
    let face = store.face(FontKey(0)).expect("parse face");
    // `with_ctx`'s `get-initial-context` defaults `font_size` to 12pt
    // (`Context::initial`) -- matches `math_fraction_radical.rs`'s own
    // `size` convention for the same reason.
    let size = Length::pt(12.0);

    // Independently (ttf-parser, no hardcoded gids) compute the BigOp
    // record[1] variant gid the port's own policy is supposed to select,
    // and the plain base gid it must NOT select.
    let base_gid = face.glyph_index('∑').expect("cmap has ∑");
    let construction = face
        .tables()
        .math
        .expect("MATH table")
        .variants
        .expect("MathVariants")
        .vertical_constructions
        .get(base_gid)
        .expect("∑ has a vertical GlyphConstruction");
    let n = construction.variants.len();
    let expected_variant = construction
        .variants
        .get(if n >= 2 { 1 } else { 0 })
        .expect("record[1] (or [0])")
        .variant_glyph;

    // -- Non-big baseline: plain `math-char MathOp` ∑, no growth.
    let base_src = with_ctx("embed-math ctx (math-char MathOp `∑`)");
    let base_v = run_math(&base_src, &store).expect("plain ∑ should compile");
    // `_base_h`/`_base_d` deliberately unused for the growth comparison
    // below (see that assertion's comment for why the plain run's box
    // height/depth is the wrong baseline) — this run only proves `gid: None`.
    let (_, _base_h, _base_d, base_glyphs) = as_math_parts(math_box(base_v));
    assert_eq!(base_glyphs.len(), 1);
    assert_eq!(
        base_glyphs[0].gid, None,
        "plain (non-big) ∑ should stay on the cmap path (gid: None)"
    );

    // -- Big: `math-big-char MathOp` ∑.
    let big_src = with_ctx("embed-math ctx (math-big-char MathOp `∑`)");
    let big_v = run_math(&big_src, &store).expect("math-big-char MathOp `∑` should compile");
    let (_, big_h, big_d, big_glyphs) = as_math_parts(math_box(big_v));
    assert_eq!(big_glyphs.len(), 1, "expected exactly 1 glyph, got {big_glyphs:?}");
    assert_eq!(
        big_glyphs[0].gid,
        Some(expected_variant.0),
        "expected the BigOp policy's record[1] (or [0] if only one) variant gid"
    );
    assert_ne!(
        big_glyphs[0].gid,
        Some(base_gid.0),
        "the big-char glyph must NOT be the plain base gid"
    );

    // -- Growth: >20% more height+depth than the base (non-variant) glyph's
    // own ink bbox. NOT the plain run's `PureHorzBox::Math` height/depth
    // above: those come from `push_char_glyph`, which reports the FONT-WIDE
    // ascender/descender for every plain glyph (see `math_fraction_radical.rs`'s
    // "ascender/descender ARE h_cont/d_cont" comment) — an inflated quantity
    // that already exceeds even a stretched variant's real ink on one test
    // font. `MathVariantGlyph.height`/`.depth` ARE real per-glyph ink bboxes
    // (`ttf.rs`'s doc comment on the struct), so the fair baseline is the
    // unstretched base glyph's own bbox, computed independently via
    // `ttf-parser`.
    let base_bbox = face
        .glyph_bounding_box(base_gid)
        .expect("base ∑ glyph has a bbox");
    let base_ink_h = size.0 * (base_bbox.y_max.max(0) as f64) / (face.units_per_em() as f64);
    let base_ink_d =
        size.0 * ((-(base_bbox.y_min.min(0) as i32)) as f64) / (face.units_per_em() as f64);
    let base_ink_extent = base_ink_h + base_ink_d;
    let big_extent = big_h.0 + big_d.0;
    assert!(
        big_extent > base_ink_extent * 1.2,
        "expected the big ∑ variant's height+depth ({big_extent}) to exceed the base \
         (non-variant) glyph's own ink height+depth ({base_ink_extent}) by more than 20%"
    );

    // e2e through the real CID pipeline: the content stream's Tj operand for
    // this run is the exact 2-byte-BE CID the writer assigns the variant
    // gid, not the base gid. `expected_cid` derives that CID (remapped,
    // for a CFF font) from this page's actual usage set — a single glyph,
    // the variant itself.
    let geometry = PageGeometry::default();
    let e2e_box = math_box(run_math(&big_src, &store).unwrap());
    let (_, _, _, e2e_glyphs) = as_math_parts(e2e_box.clone());
    let used_gids = original_gids_used(&face, &e2e_glyphs);
    let page = page_for(e2e_box, &geometry);
    let pdf_bytes = render_pdf_ttf(&geometry, &[page], &store, &[]).expect("render");
    assert!(pdf_bytes.starts_with(b"%PDF-"));
    let content = content_stream(&pdf_bytes);

    let font_bytes = std::fs::read(&path).expect("read font file for subsetter cross-check");
    let variant_cid = expected_cid(&font_bytes, &used_gids, expected_variant.0);
    let variant_bytes = variant_cid.to_be_bytes();
    let variant_repr = pdf_str_repr(&variant_bytes);
    assert!(
        contains_subslice(&content, &variant_repr),
        "expected the content stream to contain the variant gid's (remapped) Tj operand \
         {variant_repr:02x?} ({:?}); content stream = {:?}",
        String::from_utf8_lossy(&variant_repr),
        String::from_utf8_lossy(&content)
    );

    // The base (non-variant) gid was never used anywhere in this page, so it
    // never gets a CID assignment at all (`GlyphRemapper` only maps glyphs it
    // was asked to remap) — its RAW gid bytes should not appear as a Tj
    // operand.
    let base_bytes = base_gid.0.to_be_bytes();
    let base_repr = pdf_str_repr(&base_bytes);
    if base_repr != variant_repr {
        assert!(
            !contains_subslice(&content, &base_repr),
            "the content stream should NOT contain the plain base gid's Tj \
             operand {base_repr:02x?}; content stream = {:?}",
            String::from_utf8_lossy(&content)
        );
    }
}

// ----------------------------------------------------------------------
// e2e: `(`/`)` stretch around a tall inner, with a
// short-inner control that must fall back to record[0].
// ----------------------------------------------------------------------

/// A well-typed but POISONED `paren` closure (`length -> length -> length ->
/// length -> color -> (inline-boxes, length -> length)`, `prim_types::
/// t_paren`): the closure route is PRIMARY (`make_paren_run`
/// invokes `_l`/`_r`), so a closure that merely returned a valid tuple would
/// be invoked and used verbatim, breaking every assertion below that
/// expects the MATH-native variant-gid fallback path. Poisoned with
/// a runtime (not typecheck) error — `string-sub \`x\` 9 9` is well-typed
/// but out-of-bounds on the 1-char string `` `x` `` (`prim_string_sub`
/// rejects `pos+wid > len`) — inside a discarded application, so it only
/// fires once the closure is CALLED with all 5 args (curried, call-by-value).
/// `make_paren_run` forwards that `EvalError` through `interp.apply`'s `?`,
/// and `Math::Paren`'s `Err(_) => paren_variant_fallback(...)` arm catches
/// it — exercising exactly the fallback this file's assertions need.
const DUMMY_PAREN: &str = "(fun hgt dpt hgtaxis fontsize color -> \
     (fun s -> (inline-nil, (fun x -> x))) (string-sub `x` 9 9))";

#[test]
fn paren_stretches_around_tall_inner_and_short_inner_stays_record0() {
    let path = need_font!(find_math_font(), "MATH");
    let store = TtfFontStore::load(&path, None, None).expect("load math font");
    let face = store.face(FontKey(0)).expect("parse face");
    let size = Length::pt(12.0);

    // `(` and `)` are DIFFERENT glyphs with their own, independent
    // `GlyphConstruction`s -- record[0] gids must be computed separately for
    // each; open/close parens are never the same glyph.
    let record0_gid_of = |c: char| -> u16 {
        let gid = face.glyph_index(c).expect("cmap has the char");
        face.tables()
            .math
            .expect("MATH table")
            .variants
            .expect("MathVariants")
            .vertical_constructions
            .get(gid)
            .expect("char has a vertical GlyphConstruction")
            .variants
            .get(0)
            .expect("record[0]")
            .variant_glyph
            .0
    };
    let open_record0_gid = record0_gid_of('(');
    let close_record0_gid = record0_gid_of(')');

    let mc = store
        .math_constants(FontKey(0))
        .expect("MATH font should expose MathConstants");
    let axis = size * mc.axis_height;

    // -- Control: a short (empty) inner must emit record[0]'s gid (no
    // stretch needed). NOT a single ordinary char (`x`): `push_char_glyph`
    // reports a plain glyph's height/depth as the font-wide ascender/
    // descender, which for both test fonts is comfortably TALLER than
    // record[0]'s own coverage (empirically: DejaVu's target from one
    // plain char is ~12.4pt vs. record[0]'s own ~10.8pt; Noto's is ~19.0pt
    // vs. ~11.3pt) — so a single ordinary character is NOT "short" enough
    // to stay at record[0] on either font. An EMPTY inner (`${}`,
    // `inner_ink_extent` folds to `(ZERO, ZERO)`) robustly IS: its target
    // reduces to `2*axis` (~6.6-6.7pt on both fonts), well under
    // record[0]'s own coverage on both.
    let short_src = with_ctx(&format!(
        "embed-math ctx (math-paren {DUMMY_PAREN} {DUMMY_PAREN} ${{}})"
    ));
    let short_v =
        run_math(&short_src, &store).expect("math-paren over an empty inner should compile");
    let (_, _, _, short_glyphs) = as_math_parts(math_box(short_v));
    assert_eq!(
        short_glyphs.len(),
        2,
        "expected '(', ')' -- 2 glyphs (empty inner), got {short_glyphs:?}"
    );
    assert_eq!(
        short_glyphs[0].gid,
        Some(open_record0_gid),
        "short inner: the '(' should stay at record[0] (gid {open_record0_gid})"
    );
    assert_eq!(
        short_glyphs[1].gid,
        Some(close_record0_gid),
        "short inner: the ')' should stay at record[0] (gid {close_record0_gid})"
    );

    // -- Tall inner: `math-big-char MathOp` ∑ (already proven, above test,
    // to have a REAL, substantially larger ink extent than a plain char) --
    // its own extent forces the parens to stretch to a later, non-record[0]
    // variant.
    let tall_inner_src = "(math-big-char MathOp `∑`)";
    let tall_src = with_ctx(&format!(
        "embed-math ctx (math-paren {DUMMY_PAREN} {DUMMY_PAREN} {tall_inner_src})"
    ));
    let tall_v =
        run_math(&tall_src, &store).expect("math-paren over a tall inner should compile");
    let (_, paren_h, paren_d, tall_glyphs) = as_math_parts(math_box(tall_v));
    assert_eq!(
        tall_glyphs.len(),
        3,
        "expected '(', ∑, ')' -- 3 glyphs, got {tall_glyphs:?}"
    );

    let open = &tall_glyphs[0];
    let close = &tall_glyphs[2];
    assert_ne!(
        open.gid,
        Some(open_record0_gid),
        "tall inner: the '(' should have stretched past record[0] (gid {open_record0_gid}), got {:?}",
        open.gid
    );
    assert_ne!(
        close.gid,
        Some(close_record0_gid),
        "tall inner: the ')' should have stretched past record[0] (gid {close_record0_gid}), got {:?}",
        close.gid
    );

    // -- Sized to cover the inner extent: the whole paren box's height+depth
    // must be at least the standalone tall-inner run's own height+depth
    // (the big ∑ alone, measured independently above).
    let inner_alone_src = with_ctx(&format!("embed-math ctx {tall_inner_src}"));
    let inner_alone_v =
        run_math(&inner_alone_src, &store).expect("the tall inner alone should compile");
    let (_, inner_h, inner_d, _) = as_math_parts(math_box(inner_alone_v));
    assert!(
        paren_h.0 + paren_d.0 >= inner_h.0 + inner_d.0 - 1e-6,
        "expected the paren box's height+depth ({}) to cover the tall inner's own \
         ({}) ",
        paren_h.0 + paren_d.0,
        inner_h.0 + inner_d.0
    );

    // -- Centering: dy = axis - (height - depth) / 2 (y-up), NOT mirrored
    // below the baseline -- the placed ink [dy - depth, dy + height]
    // straddles the axis, and matches the exact formula.
    for (label, g) in [("'('", open), ("')'", close)] {
        let expected_dy = axis - (g.height - g.depth) * 0.5;
        assert!(
            approx(g.dy, expected_dy, 1e-6),
            "{label}: expected dy = axis - (h-d)/2 = {expected_dy:?}, got {:?}",
            g.dy
        );
        let placed_bottom = g.dy - g.depth;
        let placed_top = g.dy + g.height;
        assert!(
            placed_bottom.0 <= axis.0 + 1e-6 && placed_top.0 >= axis.0 - 1e-6,
            "{label}: placed ink [{:?}, {:?}] should straddle the axis ({axis:?}) -- \
             NOT mirrored entirely below the baseline",
            placed_bottom,
            placed_top
        );
    }

    // e2e through the real CID pipeline: the content stream contains the
    // '(' variant gid's Tj operand, at the CID `expected_cid` derives from
    // this page's actual 3-glyph usage set ('(', ∑-variant, ')').
    let geometry = PageGeometry::default();
    let e2e_box = math_box(run_math(&tall_src, &store).unwrap());
    let (_, _, _, e2e_glyphs) = as_math_parts(e2e_box.clone());
    let used_gids = original_gids_used(&face, &e2e_glyphs);
    let page = page_for(e2e_box, &geometry);
    let pdf_bytes = render_pdf_ttf(&geometry, &[page], &store, &[]).expect("render");
    assert!(pdf_bytes.starts_with(b"%PDF-"));
    let content = content_stream(&pdf_bytes);
    let open_gid = open.gid.expect("open paren has a variant gid");
    let font_bytes = std::fs::read(&path).expect("read font file for subsetter cross-check");
    let open_cid = expected_cid(&font_bytes, &used_gids, open_gid);
    let open_repr = pdf_str_repr(&open_cid.to_be_bytes());
    assert!(
        contains_subslice(&content, &open_repr),
        "expected the content stream to contain the '(' variant gid's (remapped) Tj operand \
         {open_repr:02x?}; content stream = {:?}",
        String::from_utf8_lossy(&content)
    );
}

// ----------------------------------------------------------------------
// e2e wiring: a delimiter taller than any discrete
// variant record is built from `GlyphAssembly` parts by
// `push_delimiter_glyph` — driven through the real lang layout.
// ----------------------------------------------------------------------

/// The set of `(`'s enumerated `GlyphAssembly` part gids (ttf-parser, no
/// hardcoded gids), plus the set of its DISCRETE variant record gids — used
/// to distinguish "the paren was built from assembly parts" from "the paren
/// picked a single discrete variant".
fn paren_assembly_and_variant_gids(face: &Face, c: char) -> (Vec<u16>, Vec<u16>) {
    let gid = face.glyph_index(c).expect("cmap has the char");
    let construction = face
        .tables()
        .math
        .expect("MATH table")
        .variants
        .expect("MathVariants")
        .vertical_constructions
        .get(gid)
        .expect("char has a vertical GlyphConstruction");
    let assembly_gids: Vec<u16> = construction
        .assembly
        .expect("delimiter has a GlyphAssembly")
        .parts
        .into_iter()
        .map(|p| p.glyph_id.0)
        .collect();
    let variant_gids: Vec<u16> = construction
        .variants
        .into_iter()
        .map(|v| v.variant_glyph.0)
        .collect();
    (assembly_gids, variant_gids)
}

#[test]
fn very_tall_paren_is_built_from_assembly_parts() {
    let path = need_font!(find_math_font(), "MATH");
    let store = TtfFontStore::load(&path, None, None).expect("load math font");
    let face = store.face(FontKey(0)).expect("parse face");

    let (open_asm_gids, open_variant_gids) = paren_assembly_and_variant_gids(&face, '(');

    // A deeply-nested fraction of big operators: four stacked rows of ∑, far
    // taller than the largest discrete '(' variant record — so the paren must
    // stretch via `GlyphAssembly` (top hook + repeated extenders + bottom
    // hook), not a single discrete variant glyph. `DUMMY_PAREN` forces the
    // MATH-native fallback path (`paren_variant_fallback` ->
    // `push_delimiter_glyph`) that carries the assembly wiring.
    let big = "(math-big-char MathOp `∑`)";
    let row = format!("(math-frac {big} {big})");
    let tall_inner = format!("(math-frac {row} {row})");
    let src = with_ctx(&format!(
        "embed-math ctx (math-paren {DUMMY_PAREN} {DUMMY_PAREN} {tall_inner})"
    ));
    let v = run_math(&src, &store).expect("tall nested-fraction paren should compile");
    let (_, _, _, glyphs) = as_math_parts(math_box(v));

    // Among the laid-out glyphs, the ones belonging to `(`'s assembly part
    // set: there must be MULTIPLE (top + repeated extenders + bottom), and
    // more than the number of DISTINCT assembly parts (extender repeated).
    let placed_open_parts: Vec<u16> = glyphs
        .iter()
        .filter_map(|g| g.gid)
        .filter(|gid| open_asm_gids.contains(gid))
        .collect();
    assert!(
        placed_open_parts.len() > open_asm_gids.len(),
        "expected the tall '(' to be built from MULTIPLE assembly parts (extender repeated): \
         placed {placed_open_parts:?} vs {} distinct assembly parts {open_asm_gids:?}",
        open_asm_gids.len()
    );
    // Sanity: the placed parts are assembly parts, NOT a single discrete
    // variant glyph (the two gid sets are disjoint for these fonts).
    let placed_open_variants: Vec<u16> = glyphs
        .iter()
        .filter_map(|g| g.gid)
        .filter(|gid| open_variant_gids.contains(gid) && !open_asm_gids.contains(gid))
        .collect();
    assert!(
        placed_open_variants.is_empty(),
        "the tall '(' should have used assembly parts, not a discrete variant glyph, but found \
         discrete-variant gids {placed_open_variants:?}"
    );

    // e2e: the assembly part gids flow through the real CID pipeline. This
    // tall run embeds the (unsubsetted, ~half-MB) font whose raw CFF/glyf
    // bytes contain stray `BT`/`Tf`/`stream` sequences, so the
    // "smallest stream"/"stream with BT" heuristics can't reliably isolate
    // the content stream here. Instead verify the part gid reached the
    // ToUnicode CMap (`beginbfchar`) — a distinctive stream that
    // render_pdf_ttf only populates with gids it actually emitted. The
    // CMap's source CID comes from `expected_cid` against this page's own
    // full usage set (`glyphs`, above) — not the raw face gid.
    let geometry = PageGeometry::default();
    let page = page_for(math_box(run_math(&src, &store).unwrap()), &geometry);
    let pdf_bytes = render_pdf_ttf(&geometry, &[page], &store, &[]).expect("render");
    assert!(pdf_bytes.starts_with(b"%PDF-"));
    let cmap = extract_streams(&pdf_bytes)
        .into_iter()
        .find(|s| contains_subslice(s, b"beginbfchar"))
        .expect("a ToUnicode CMap stream (beginbfchar)")
        .to_vec();
    let some_part = placed_open_parts[0];
    let font_bytes = std::fs::read(&path).expect("read font file for subsetter cross-check");
    let used_gids = original_gids_used(&face, &glyphs);
    let some_part_cid = expected_cid(&font_bytes, &used_gids, some_part);
    // CMap entries are `<GGGG> <UUUU>` uppercase-hex; the emitted part gid's
    // (remapped) CID must appear as a source CID.
    let gid_hex = format!("<{:04X}>", some_part_cid).into_bytes();
    assert!(
        contains_subslice(&cmap, &gid_hex),
        "expected the ToUnicode CMap to map the assembly part gid {some_part} (CID {some_part_cid}, \
         {}); CMap = {:?}",
        String::from_utf8_lossy(&gid_hex),
        String::from_utf8_lossy(&cmap)
    );
}

