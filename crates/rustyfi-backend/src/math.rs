//! Math value / box model: trimmed analog of `math.ml`'s `math_kind`
//! (`horzBox.ml:134`) and `low_math_atom` (`math.ml:9`).

use crate::hbox::HorzStringInfo;
use crate::length::Length;
use std::collections::BTreeMap;

/// v0.0.6 `math_kind` (horzBox.ml:134-145). `Prefix` is a SATySFi-specific
/// class (differential `d`, `\partial`); `End` is a synthetic list-boundary
/// sentinel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MathKind {
    Ord,
    Bin,
    Rel,
    Op,
    Punct,
    Open,
    Close,
    Prefix,
    Inner,
    End,
}

/// One already-positioned glyph inside a laid-out math run: a string set in
/// `info` (font + size — a superscript carries a *smaller* size here), placed
/// at `dx` right of the math box's origin and `dy` **above** its baseline
/// (dy < 0 = below, for subscripts). The analog of `math.ml`'s `LowMathGlyph`
/// after `horz_of_low_math` has resolved its `PHGRising` shift into an
/// offset.
#[derive(Clone, Debug, PartialEq)]
pub struct MathGlyph {
    pub info: HorzStringInfo,
    pub text: String,
    /// `Some(gid)`: a raw MATH-table variant glyph id
    /// (`push_big_char_glyph`/`push_delimiter_glyph`), NOT necessarily
    /// cmap-reachable from `text`; the CID writer emits it directly rather
    /// than re-deriving a gid, keeping `text` as the ToUnicode source. `None`
    /// for every ordinary cmap-driven glyph, and on every base-14 path (that
    /// writer never reads this field).
    pub gid: Option<u16>,
    pub dx: Length,
    pub dy: Length,
    pub width: Length,
    pub height: Length,
    pub depth: Length,
}

/// v0.0.6 `math_char_class` (`primitives.cppo.ml`'s `MathItalic`/…): which
/// Mathematical-Alphanumeric style block a plain `${…}` letter resolves to
/// (`\mathrm`/`\mathbf`/… — `math.satyh`'s `\math-style`). `Ord`/`Hash` so
/// it can key `Context::math_variant_char_map`'s override table.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MathCharClass {
    Italic,
    BoldItalic,
    Roman,
    BoldRoman,
    Script,
    BoldScript,
    Fraktur,
    BoldFraktur,
    DoubleStruck,
    /// Upstream dev-0-1-0 widens `math_char_class` 9 → 14
    /// (`b836d512:src/backend/horzBox.ml:98-113`); v0.0.6 has exactly the 9
    /// above (`v0.0.6:src/backend/horzBox.ml:147-158`). These 5 are
    /// unreachable under V0_0: registration
    /// (`prim_types.rs::math_char_class_decl`) is V0_1-gated and typecheck
    /// rejects unregistered constructor names — version-blind at this
    /// enum/backend layer, version-gated at the registration layer.
    SansSerif,
    BoldSansSerif,
    ItalicSansSerif,
    BoldItalicSansSerif,
    Typewriter,
}

/// Upstream `default_math_class_map` (`primitives.cppo.ml:465-480`):
/// whole-TOKEN entries consulted BEFORE the per-char variant lookup below;
/// value = (replacement codepoints, math class). Only `-` changes codepoint
/// (`-` -> U+2212 MINUS SIGN); every other entry is an identity remap that
/// exists purely to attach the right `MathKind` to the whole run.
pub(crate) fn default_math_class_map() -> BTreeMap<String, (String, MathKind)> {
    [
        ("=", "=", MathKind::Rel),
        ("<", "<", MathKind::Rel),
        (">", ">", MathKind::Rel),
        (":", ":", MathKind::Rel),
        ("+", "+", MathKind::Bin),
        ("-", "\u{2212}", MathKind::Bin),
        ("|", "|", MathKind::Bin),
        ("/", "/", MathKind::Ord),
        (",", ",", MathKind::Punct),
    ]
    .into_iter()
    .map(|(k, v, mk)| (k.to_string(), (v.to_string(), mk)))
    .collect()
}

/// Upstream `default_math_variant_char_map` (`primitives.cppo.ml:358-460`)
/// as a pure function (base-offset + exception lists) rather than a big table
/// cloned into every `Context`, which stores only the runtime override map
/// (`set-math-variant-char`). `None` means "no remap" — either `class`/`c` has
/// no Unicode Mathematical-Alphanumeric counterpart (e.g. a digit), or the
/// letter has no assigned codepoint in that style block (a handful of
/// Script/Fraktur/Double-Struck letters use a distinct legacy symbol instead,
/// per Unicode's own gaps — the hardcoded exceptions below).
pub fn default_math_variant_char(class: MathCharClass, c: char) -> Option<char> {
    let cap = c.is_ascii_uppercase().then(|| c as u32 - 'A' as u32);
    let small = c.is_ascii_lowercase().then(|| c as u32 - 'a' as u32);
    fn cp(base: u32, i: u32) -> Option<char> {
        char::from_u32(base + i)
    }
    match class {
        MathCharClass::Italic => match (cap, small) {
            (Some(i), _) => cp(0x1D434, i),
            (_, Some(7)) => Some('\u{210E}'),
            (_, Some(i)) => cp(0x1D44E, i),
            _ => None,
        },
        MathCharClass::BoldItalic => match (cap, small) {
            (Some(i), _) => cp(0x1D468, i),
            (_, Some(i)) => cp(0x1D482, i),
            _ => None,
        },
        MathCharClass::Roman => (cap.is_some() || small.is_some()).then_some(c),
        MathCharClass::BoldRoman => match (cap, small) {
            (Some(i), _) => cp(0x1D400, i),
            (_, Some(i)) => cp(0x1D41A, i),
            _ => None,
        },
        MathCharClass::Script => match c {
            'B' => Some('\u{212C}'),
            'E' => Some('\u{2130}'),
            'F' => Some('\u{2131}'),
            'H' => Some('\u{210B}'),
            'I' => Some('\u{2110}'),
            'L' => Some('\u{2112}'),
            'M' => Some('\u{2133}'),
            'R' => Some('\u{211B}'),
            'e' => Some('\u{212F}'),
            'g' => Some('\u{210A}'),
            'o' => Some('\u{2134}'),
            _ => match (cap, small) {
                (Some(i), _) => cp(0x1D49C, i),
                (_, Some(i)) => cp(0x1D4B6, i),
                _ => None,
            },
        },
        MathCharClass::BoldScript => match (cap, small) {
            (Some(i), _) => cp(0x1D4D0, i),
            (_, Some(i)) => cp(0x1D4EA, i),
            _ => None,
        },
        MathCharClass::Fraktur => match c {
            'C' => Some('\u{212D}'),
            'H' => Some('\u{210C}'),
            'I' => Some('\u{2111}'),
            'R' => Some('\u{211C}'),
            'Z' => Some('\u{2128}'),
            _ => match (cap, small) {
                (Some(i), _) => cp(0x1D504, i),
                (_, Some(i)) => cp(0x1D51E, i),
                _ => None,
            },
        },
        MathCharClass::BoldFraktur => match (cap, small) {
            (Some(i), _) => cp(0x1D56C, i),
            (_, Some(i)) => cp(0x1D586, i),
            _ => None,
        },
        MathCharClass::DoubleStruck => match c {
            'C' => Some('\u{2102}'),
            'H' => Some('\u{210D}'),
            'N' => Some('\u{2115}'),
            'P' => Some('\u{2119}'),
            'Q' => Some('\u{211A}'),
            'R' => Some('\u{211D}'),
            'Z' => Some('\u{2124}'),
            _ => match (cap, small) {
                (Some(i), _) => cp(0x1D538, i),
                (_, Some(i)) => cp(0x1D552, i),
                _ => None,
            },
        },
        // These 5 Unicode blocks (`primitives.cppo.ml`'s capitals/smalls
        // folds) are gap-free — no exception chars, unlike
        // Script/Fraktur/DoubleStruck above.
        // KNOWN GAP: upstream also remaps DIGITS in every class (sans
        // `0x1D7E2`, bold-sans `0x1D7EC`, typewriter `0x1D7F6`, …); this
        // returns `None` for digits under every class.
        MathCharClass::SansSerif => match (cap, small) {
            (Some(i), _) => cp(0x1D5A0, i),
            (_, Some(i)) => cp(0x1D5BA, i),
            _ => None,
        },
        MathCharClass::BoldSansSerif => match (cap, small) {
            (Some(i), _) => cp(0x1D5D4, i),
            (_, Some(i)) => cp(0x1D5EE, i),
            _ => None,
        },
        MathCharClass::ItalicSansSerif => match (cap, small) {
            (Some(i), _) => cp(0x1D608, i),
            (_, Some(i)) => cp(0x1D622, i),
            _ => None,
        },
        MathCharClass::BoldItalicSansSerif => match (cap, small) {
            (Some(i), _) => cp(0x1D63C, i),
            (_, Some(i)) => cp(0x1D656, i),
            _ => None,
        },
        MathCharClass::Typewriter => match (cap, small) {
            (Some(i), _) => cp(0x1D670, i),
            (_, Some(i)) => cp(0x1D68A, i),
            _ => None,
        },
    }
}
