//! Math value / box model — Slice 1 of `docs/plans/math-engine.md`. Trimmed
//! analog of `math.ml`'s `math_kind` (`horzBox.ml:134`) and `low_math_atom`
//! (`math.ml:9`), holding only what a fixed-constant super/subscript layout
//! needs (no MATH-table metrics — see the plan's roadmap §B).

use crate::hbox::HorzStringInfo;
use crate::length::Length;
use std::collections::BTreeMap;

/// v0.0.6 `math_kind` (horzBox.ml:134-145). `Prefix` is a SATySFi-specific
/// class (differential `d`, `\partial`); `End` is a synthetic list-boundary
/// sentinel. Full set kept so the pairwise spacing table (roadmap A) extends
/// rather than replaces it; Slice 1 only ever produces Ord/Bin/Rel/Punct.
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
    /// `Some(gid)`: a raw MATH-table variant glyph id (§B3 —
    /// `push_big_char_glyph`/`push_delimiter_glyph`), NOT necessarily
    /// cmap-reachable from `text`; the CID writer (`satysfi-pdf`) emits it
    /// directly rather than re-deriving a gid from `text`, keeping `text` as
    /// the ToUnicode source. `None` for every ordinary cmap-driven glyph and
    /// (structurally) for every base-14 glyph — the base-14 writer never
    /// reads this field, so it is `None` on every base-14 code path.
    pub gid: Option<u16>,
    pub dx: Length,
    pub dy: Length,
    pub width: Length,
    pub height: Length,
    pub depth: Length,
}

/// v0.0.6 `math_char_class` (`primitives.cppo.ml`'s `MathItalic`/…): which
/// Mathematical-Alphanumeric style block a plain `${…}` letter resolves to
/// (`\mathrm`/`\mathbf`/… — `math.satyh`'s `\math-style`, `docs/plans/
/// math-engine.md` §F). `Ord`/`Hash` so it can key
/// `Context::math_variant_char_map`'s override table.
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
}

/// Upstream `default_math_class_map` (`primitives.cppo.ml:465-480`):
/// whole-TOKEN entries consulted BEFORE the per-char variant lookup below;
/// value = (replacement codepoints, math class). Only `-` changes codepoint
/// (`-` -> U+2212 MINUS SIGN); every other entry is an identity remap that
/// exists purely to attach the right `MathKind` to the whole run.
pub fn default_math_class_map() -> BTreeMap<String, (String, MathKind)> {
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
/// as a pure function (base-offset + exception lists) rather than a big
/// table cloned into every `Context` — `Context` only stores the runtime
/// override map (`set-math-variant-char`'s target of gap 7); this supplies
/// the DEFAULT remap consulted when no override exists. `None` means "no
/// remap" (keep the source char) — either because `class`/`c` has no
/// Unicode Mathematical-Alphanumeric counterpart at all (e.g. a digit) or
/// because the specific letter has no assigned codepoint in that style
/// block (a handful of Script/Fraktur/Double-Struck letters, per Unicode's
/// own gaps in that block, use a distinct legacy symbol instead — the
/// hardcoded exceptions below).
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
    }
}
