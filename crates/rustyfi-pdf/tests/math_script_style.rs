//! `ssty` (Math Script Style) — the GSUB feature a math font uses to swap in
//! purpose-drawn exponent/index forms: `TtfFontStore::math_script_variant`
//! (`ttf.rs`) and `push_char_glyph`'s use of it
//! (`rustyfi-lang/src/primitives.rs`), against upstream's
//! `FontFormat.get_math_script_variant` (`fontFormat.ml:2216-2241`) driven by
//! `fontInfo.ml:379-383`'s `if is_in_base_level then gidraw else …`.
//!
//! Why this is a WIDTH test and not only a shape one: a `.st` glyph is not the
//! base glyph scaled. In Latin Modern Math `two.st` advances 569/1000 em where
//! plain `two` advances 500, so a port that ignores the feature sets every
//! script digit ~14% narrow and the error compounds outwards through whatever
//! encloses the script. Nothing here hardcodes 569 — every expectation is read
//! back off the loaded face, so the tests hold for any math font with an
//! `ssty` feature.
//!
//! Font discovery is copied from `math_vertical_variants.rs` (fontconfig,
//! then common distro/nix paths, then a graceful skip), for the same reason:
//! these need a real MATH/GSUB font and CI may not have one.

use std::path::{Path, PathBuf};
use std::process::Command;

use rustyfi_backend::{FontKey, FontMetrics, HorzBox, Length, MathGlyph, PureHorzBox};
use rustyfi_lang::value::Value;
use rustyfi_lang::{elaborate, eval, primitives, typecheck, CompileError};
use rustyfi_pdf::TtfFontStore;
use ttf_parser::gsub::{SingleSubstitution, SubstitutionSubtable};
use ttf_parser::{Face, GlyphId, Tag};

// ----------------------------------------------------------------------
// Font discovery (copied from math_vertical_variants.rs).
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
            "/usr/share/fonts/opentype/noto/NotoSansMath-Regular.otf",
            "/usr/share/fonts/OTF/NotoSansMath-Regular.otf",
            "/usr/share/fonts/noto-fonts/NotoSansMath-Regular.ttf",
            "/run/current-system/sw/share/fonts/truetype/NotoSansMath-Regular.ttf",
        ],
    )
}

/// The bundled Latin Modern Math if `download-fonts.sh` has been run, else
/// whatever math face the host has — the e2e tests only need SOME face with
/// an `ssty` feature, and unlike `math_vertical_variants.rs`'s CID
/// assertions nothing here depends on the outline format.
fn find_math_font() -> Option<PathBuf> {
    let bundled = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../lib-rustyfi/dist/fonts/latinmodern-math.otf");
    if bundled.is_file() {
        return Some(bundled);
    }
    find_dejavu_math().or_else(find_noto_math)
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
// An independent `ssty` reader, so the assertions do not derive their
// expectations from the implementation under test.
// ----------------------------------------------------------------------

/// Walk GSUB for `ssty` and return the substitute glyph for `c`, reading the
/// feature list, lookup list and coverage tables directly. Deliberately a
/// SECOND implementation: `TtfFontStore::math_script_variant` short-circuits
/// on the first hit and this one collects every hit, so a font where the two
/// disagree would fail here rather than silently agree with a bug.
fn ssty_substitutes(face: &Face, c: char) -> Vec<GlyphId> {
    let gid = face.glyph_index(c).expect("cmap has the char");
    let Some(gsub) = face.tables().gsub else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for fi in 0..gsub.features.len() {
        let feature = gsub.features.get(fi).expect("index < len");
        if feature.tag != Tag::from_bytes(b"ssty") {
            continue;
        }
        for li in 0..feature.lookup_indices.len() {
            let idx = feature.lookup_indices.get(li).expect("index < len");
            let Some(lookup) = gsub.lookups.get(idx) else {
                continue;
            };
            for st in lookup.subtables.into_iter::<SubstitutionSubtable>() {
                match st {
                    SubstitutionSubtable::Single(s) => {
                        let Some(i) = s.coverage().get(gid) else {
                            continue;
                        };
                        match s {
                            SingleSubstitution::Format1 { delta, .. } => {
                                out.push(GlyphId((gid.0 as i32 + delta as i32) as u16));
                            }
                            SingleSubstitution::Format2 { substitutes, .. } => {
                                out.extend(substitutes.get(i));
                            }
                        }
                    }
                    SubstitutionSubtable::Alternate(a) => {
                        let Some(i) = a.coverage.get(gid) else {
                            continue;
                        };
                        out.extend(a.alternate_sets.get(i).and_then(|s| s.alternates.get(0)));
                    }
                    _ => {}
                }
            }
        }
    }
    out
}

// ----------------------------------------------------------------------
// Unit: `TtfFontStore::math_script_variant`.
// ----------------------------------------------------------------------

fn assert_script_variant_unit(path: &Path) {
    let store = TtfFontStore::load(path, None, None).expect("load math font");
    let face = store.face(FontKey(0)).expect("parse face");
    let size = Length::pt(8.4);
    let upem = face.units_per_em() as f64;

    // A digit is the case the corpus exercises hardest (every `x^2`); assert
    // it substitutes at all, so a font without `ssty` coverage for digits
    // would fail loudly here rather than make the rest vacuous.
    let base_gid = face.glyph_index('2').expect("cmap has '2'");
    let expected = ssty_substitutes(&face, '2');
    assert!(
        !expected.is_empty(),
        "{path:?}: expected an `ssty` substitution for '2'; this font has none, \
         so every assertion below would be vacuous"
    );

    let got = store
        .math_script_variant(FontKey(0), '2', size)
        .unwrap_or_else(|| panic!("{path:?}: expected Some for '2' at script size"));
    assert!(
        expected.contains(&GlyphId(got.gid)),
        "{path:?}: gid {} is not among the `ssty` substitutes {expected:?} for '2'",
        got.gid
    );
    assert_ne!(
        got.gid, base_gid.0,
        "{path:?}: the script variant must differ from the base glyph"
    );

    // The advance is the VARIANT glyph's own `hmtx` entry, scaled — this is
    // the whole point of the feature, and it is not the base glyph's.
    let want = size * (face.glyph_hor_advance(GlyphId(got.gid)).unwrap() as f64 / upem);
    assert!(
        (got.advance.0 - want.0).abs() < 1e-9,
        "{path:?}: advance {} pt, expected the variant's own {} pt",
        got.advance.0,
        want.0
    );
    let base_advance = size * (face.glyph_hor_advance(base_gid).unwrap() as f64 / upem);
    assert_ne!(
        got.advance, base_advance,
        "{path:?}: a font whose `two.st` advances exactly like `two` would make \
         the width half of this feature untestable"
    );

    // Height/depth truncate towards the baseline, like `math_glyph_vextent`.
    let bbox = face.glyph_bounding_box(GlyphId(got.gid)).unwrap();
    assert!(
        (got.height.0 - (size * (bbox.y_max.max(0) as f64 / upem)).0).abs() < 1e-9,
        "{path:?}: height should be the variant's truncated ink ymax"
    );
    assert!(got.depth.0 >= 0.0, "{path:?}: depth is never negative");

    // A glyph with no `ssty` coverage declines rather than returning the base
    // glyph dressed up as a variant — `push_char_glyph` keys "use the base
    // unchanged" on the `None`.
    for c in [' ', '\u{221A}'] {
        if face.glyph_index(c).is_some() && ssty_substitutes(&face, c).is_empty() {
            assert!(
                store.math_script_variant(FontKey(0), c, size).is_none(),
                "{path:?}: expected None for {c:?}, which has no `ssty` substitution"
            );
        }
    }
}

#[test]
fn script_variant_unit_bundled_or_host() {
    let path = need_font!(find_math_font(), "math (Latin Modern / DejaVu / Noto)");
    assert_script_variant_unit(&path);
}

#[test]
fn script_variant_unit_dejavu() {
    let path = need_font!(find_dejavu_math(), "DejaVu Math TeX Gyre");
    assert_script_variant_unit(&path);
}

#[test]
fn script_variant_unit_noto() {
    let path = need_font!(find_noto_math(), "Noto Sans Math");
    assert_script_variant_unit(&path);
}

// ----------------------------------------------------------------------
// End to end through the layout engine.
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

fn math_glyphs(src: &str, store: &TtfFontStore) -> (Length, Vec<MathGlyph>) {
    let v = run_math(&with_ctx(src), store).expect("compile and evaluate");
    match v {
        Value::InlineBoxes(boxes) => {
            assert_eq!(boxes.len(), 1, "expected exactly one box, got {boxes:?}");
            match boxes.into_iter().next().unwrap() {
                HorzBox::Pure(PureHorzBox::Math { width, glyphs, .. }) => (width, glyphs),
                other => panic!("expected a PureHorzBox::Math, got {other:?}"),
            }
        }
        other => panic!("expected inline-boxes, got {other:?}"),
    }
}

/// `${x^2}`'s `2` is set in the `ssty` variant, carried to the writer through
/// the raw-`gid` channel, and measured with the VARIANT's advance — while the
/// same `2` at base level (`${x2}`) keeps the plain glyph and no gid
/// override. The control is what makes this a test of the LEVEL and not
/// merely of the substitution.
fn assert_script_variant_e2e(path: &Path) {
    let store = TtfFontStore::load(path, None, None).expect("load math font");
    let face = store.face(FontKey(0)).expect("parse face");
    let base_gid = face.glyph_index('2').expect("cmap has '2'");
    let variants = ssty_substitutes(&face, '2');
    if variants.is_empty() {
        eprintln!("skipping e2e: {path:?} has no `ssty` substitution for '2'");
        return;
    }

    let (_, scripted) = math_glyphs("embed-math ctx ${x^2}", &store);
    let two = scripted
        .iter()
        .find(|g| g.text == "2")
        .unwrap_or_else(|| panic!("{path:?}: no `2` glyph in ${{x^2}}: {scripted:?}"));
    let vgid = two
        .gid
        .unwrap_or_else(|| panic!("{path:?}: a scripted `2` must carry a raw gid, got {two:?}"));
    assert!(
        variants.contains(&GlyphId(vgid)),
        "{path:?}: scripted `2` emitted gid {vgid}, not one of {variants:?}"
    );
    let want = store
        .math_script_variant(FontKey(0), '2', two.info.size)
        .expect("the variant this glyph came from");
    assert!(
        (two.width.0 - want.advance.0).abs() < 1e-9,
        "{path:?}: scripted `2` measured {} pt, expected the variant's {} pt",
        two.width.0,
        want.advance.0
    );

    // Control: the same character at BASE level is the plain glyph.
    let (_, flat) = math_glyphs("embed-math ctx ${x2}", &store);
    let base_two = flat
        .iter()
        .find(|g| g.text == "2")
        .unwrap_or_else(|| panic!("{path:?}: no `2` glyph in ${{x2}}"));
    assert_eq!(
        base_two.gid, None,
        "{path:?}: a base-level `2` must NOT be substituted"
    );
    let upem = face.units_per_em() as f64;
    let base_advance =
        base_two.info.size * (face.glyph_hor_advance(base_gid).unwrap() as f64 / upem);
    assert!(
        (base_two.width.0 - base_advance.0).abs() < 1e-9,
        "{path:?}: a base-level `2` keeps the plain advance"
    );
    assert_ne!(
        two.width * (1.0 / two.info.size.0),
        base_two.width * (1.0 / base_two.info.size.0),
        "{path:?}: per-em, the scripted `2` must not measure like the plain one — \
         if it did, this test could not tell the feature from a pure scale"
    );
}

#[test]
fn script_variant_e2e() {
    let path = need_font!(find_math_font(), "math (Latin Modern / DejaVu / Noto)");
    assert_script_variant_e2e(&path);
}
