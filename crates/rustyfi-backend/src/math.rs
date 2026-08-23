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

/// The *unstyled* letter a Mathematical Alphanumeric Symbol stands for:
/// Unicode's own `<font>` compatibility decomposition, for the whole
/// U+1D400..=U+1D7FF block plus the Letterlike Symbols that fill that
/// block's holes. `None` for anything that is not one of those.
///
/// **Why this exists.** [`default_math_variant_char`] is the FORWARD
/// direction — plain letter to styled codepoint — and it is what makes a
/// `${x}` come out as `𝑥` U+1D465. Fonts, however, cover this block very
/// unevenly, and a codepoint the chosen face has no `cmap` entry for is
/// emitted as gid 0 (`.notdef`). What that LOOKS like depends on the face and
/// is never what the author asked for: a TrueType face draws a tofu box, and
/// a CFF/OTF face — including `latinmodern-math.otf`, this port's own default
/// math font — usually has an EMPTY `.notdef`, so the character occupies its
/// advance and paints nothing at all. That is the whole of "some glyphs are
/// not drawn in PDF mode": no error, no warning, just absent letters.
///
/// So this is the INVERSE direction, used only as a last resort by the math
/// layout path (`primitives::push_char_glyph`) when neither the math font nor
/// the text font covers the styled codepoint. Falling back to `π` for a
/// `𝜋` no font in the document can draw loses the italic styling and keeps
/// the mathematics; falling back to `.notdef` loses both. This continues the
/// port's existing metrics-probe policy — `primitives::resolve_variant_char`
/// already declines the forward remap when the target is uncoverable — rather
/// than introducing a new one; it just reaches the cases that policy cannot,
/// because `math.satyh` hands those codepoints over ALREADY styled
/// (`greek-lowercase 0x1D70B 0x1D745` for `\pi`) and there is no plain letter
/// left to decline back to.
///
/// Upstream has no counterpart: `fontInfo.ml:180-187`'s `get_glyph_id` warns
/// (`Logging.warn_no_glyph`) and uses `notdef`. The warning is worth having
/// and is ported separately; the substitution is this port's own, and it can
/// only ever fire where the alternative was a glyph that does not render.
pub fn math_alphanumeric_base(c: char) -> Option<char> {
    let u = c as u32;

    // The Letterlike Symbols that Unicode uses INSTEAD of a codepoint in the
    // block, because the block position is reserved-unused. Every one of
    // these is a hole in the Script/Fraktur/Double-struck/Italic runs, and
    // `default_math_variant_char`'s own exception arms above produce exactly
    // this set — so the two functions stay inverse to each other.
    let letterlike = |u: u32| -> Option<char> {
        Some(match u {
            0x210E => 'h',                                     // italic small h
            0x212C => 'B',                                     // script capitals
            0x2130 => 'E',
            0x2131 => 'F',
            0x210B => 'H',
            0x2110 => 'I',
            0x2112 => 'L',
            0x2133 => 'M',
            0x211B => 'R',
            0x212F => 'e',                                     // script smalls
            0x210A => 'g',
            0x2134 => 'o',
            0x212D => 'C',                                     // fraktur
            0x210C => 'H',
            0x2111 => 'I',
            0x211C => 'R',
            0x2128 => 'Z',
            0x2102 => 'C',                                     // double-struck
            0x210D => 'H',
            0x2115 => 'N',
            0x2119 => 'P',
            0x211A => 'Q',
            0x211D => 'R',
            0x2124 => 'Z',
            0x213C => '\u{3C0}',                               // double-struck Greek
            0x213D => '\u{3B3}',
            0x213E => '\u{393}',
            0x213F => '\u{3A0}',
            0x2140 => '\u{2211}',
            0x2145 => 'D',                                     // double-struck italic
            0x2146 => 'd',
            0x2147 => 'e',
            0x2148 => 'i',
            0x2149 => 'j',
            _ => return None,
        })
    };
    if let Some(base) = letterlike(u) {
        return Some(base);
    }

    if !(0x1D400..=0x1D7FF).contains(&u) {
        return None;
    }

    // The block is laid out as runs of fixed stride, and the strides tile it
    // exactly — which is the check worth keeping in mind when reading the
    // constants below: 0x1D400 + 13*52 == 0x1D6A4 (the two dotless letters),
    // 0x1D6A8 + 5*58 == 0x1D7CA (the two digammas), and 0x1D7CE + 5*10 ==
    // 0x1D800 (one past the block). `alphanumeric_block_strides_tile_the_block`
    // pins all three.

    // 13 alphabetic runs of 52: A..Z then a..z.
    if u < 0x1D6A4 {
        let off = (u - 0x1D400) % 52;
        return Some(if off < 26 {
            (b'A' + off as u8) as char
        } else {
            (b'a' + (off - 26) as u8) as char
        });
    }
    // The two dotless letters, decomposing to Latin Extended-A (NOT to
    // 'i'/'j' — Unicode's `<font>` targets are U+0131/U+0237, and a font that
    // draws a dotted 'i' where the author wrote a dotless one would be
    // silently wrong rather than merely unstyled).
    if u == 0x1D6A4 {
        return Some('\u{131}');
    }
    if u == 0x1D6A5 {
        return Some('\u{237}');
    }
    if u < 0x1D6A8 {
        return None; // 0x1D6A6/0x1D6A7 are unassigned
    }
    // 5 Greek runs of 58.
    if u < 0x1D7CA {
        let off = (u - 0x1D6A8) % 58;
        return Some(match off {
            // Α..Ρ, then the CAPITAL THETA SYMBOL that sits where the
            // ordinary capital theta already was, then Σ..Ω.
            0..=16 => char::from_u32(0x391 + off)?,
            17 => '\u{3F4}',
            18..=24 => char::from_u32(0x3A3 + (off - 18))?,
            25 => '\u{2207}', // nabla
            // α..ω, final sigma ς included at its Unicode position.
            26..=50 => char::from_u32(0x3B1 + (off - 26))?,
            51 => '\u{2202}', // partial differential
            52 => '\u{3F5}',  // lunate epsilon
            53 => '\u{3D1}',  // theta symbol
            54 => '\u{3F0}',  // kappa symbol
            55 => '\u{3D5}',  // phi symbol
            56 => '\u{3F1}',  // rho symbol
            57 => '\u{3D6}',  // pi symbol
            _ => unreachable!("offset is `% 58`"),
        });
    }
    // The two digammas, which are their own one-off pair.
    if u == 0x1D7CA {
        return Some('\u{3DC}');
    }
    if u == 0x1D7CB {
        return Some('\u{3DD}');
    }
    if u < 0x1D7CE {
        return None; // 0x1D7CC/0x1D7CD are unassigned
    }
    // 5 digit runs of 10.
    let off = (u - 0x1D7CE) % 10;
    Some((b'0' + off as u8) as char)
}

#[cfg(test)]
mod alphanumeric_base_tests {
    use super::*;

    /// The three stride runs must tile U+1D400..=U+1D7FF exactly. Getting one
    /// stride wrong would silently shift a whole run's decomposition (a
    /// `𝜋` coming back as `ο`, say), which no spot check on one letter would
    /// catch — so assert the arithmetic itself.
    #[test]
    fn alphanumeric_block_strides_tile_the_block() {
        assert_eq!(0x1D400 + 13 * 52, 0x1D6A4, "13 alphabetic runs of 52");
        assert_eq!(0x1D6A8 + 5 * 58, 0x1D7CA, "5 Greek runs of 58");
        assert_eq!(0x1D7CE + 5 * 10, 0x1D800, "5 digit runs of 10");
    }

    /// [`math_alphanumeric_base`] is the inverse of
    /// [`default_math_variant_char`] wherever the latter produces anything:
    /// for every class and every ASCII letter, styling then un-styling is the
    /// identity. This is the property that makes the fallback safe — it can
    /// only ever hand back the letter the author actually wrote.
    #[test]
    fn it_inverts_default_math_variant_char_for_every_class_and_letter() {
        let classes = [
            MathCharClass::Italic,
            MathCharClass::BoldItalic,
            MathCharClass::Roman,
            MathCharClass::BoldRoman,
            MathCharClass::Script,
            MathCharClass::BoldScript,
            MathCharClass::Fraktur,
            MathCharClass::BoldFraktur,
            MathCharClass::DoubleStruck,
            MathCharClass::SansSerif,
            MathCharClass::BoldSansSerif,
            MathCharClass::ItalicSansSerif,
            MathCharClass::BoldItalicSansSerif,
            MathCharClass::Typewriter,
        ];
        for class in classes {
            for c in ('A'..='Z').chain('a'..='z') {
                let Some(styled) = default_math_variant_char(class, c) else {
                    continue;
                };
                if styled == c {
                    continue; // `Roman` is the identity; nothing to invert.
                }
                assert_eq!(
                    math_alphanumeric_base(styled),
                    Some(c),
                    "{class:?} {c:?} -> U+{:04X} did not invert",
                    styled as u32
                );
            }
        }
    }

    /// The Greek run's internal layout is the irregular part: two symbol
    /// letters interrupt the capitals and smalls, and seven variant forms
    /// trail each run. `\pi` is the case the playground actually hit.
    #[test]
    fn greek_runs_decompose_including_their_irregular_slots() {
        // `math.satyh`'s `\pi = greek-lowercase 0x1D70B 0x1D745`.
        assert_eq!(math_alphanumeric_base('\u{1D70B}'), Some('\u{3C0}'));
        assert_eq!(math_alphanumeric_base('\u{1D745}'), Some('\u{3C0}'));
        // First and last capital of the bold run, either side of the
        // capital-theta-symbol slot.
        assert_eq!(math_alphanumeric_base('\u{1D6A8}'), Some('\u{391}'));
        assert_eq!(math_alphanumeric_base('\u{1D6C0}'), Some('\u{3A9}'));
        assert_eq!(math_alphanumeric_base('\u{1D6B9}'), Some('\u{3F4}'));
        // Nabla and partial, which sit inside the run rather than beside it.
        assert_eq!(math_alphanumeric_base('\u{1D6C1}'), Some('\u{2207}'));
        assert_eq!(math_alphanumeric_base('\u{1D6DB}'), Some('\u{2202}'));
        // The trailing variant forms of the last (sans-serif bold italic) run.
        assert_eq!(math_alphanumeric_base('\u{1D7C9}'), Some('\u{3D6}'));
    }

    /// Digits, the dotless pair and the digammas — the runs that are not
    /// letters — plus the guarantee that ordinary text is left alone, since
    /// this function gates a substitution.
    #[test]
    fn non_letter_runs_and_the_none_cases() {
        assert_eq!(math_alphanumeric_base('\u{1D7CE}'), Some('0'));
        assert_eq!(math_alphanumeric_base('\u{1D7FF}'), Some('9'));
        assert_eq!(math_alphanumeric_base('\u{1D6A4}'), Some('\u{131}'));
        assert_eq!(math_alphanumeric_base('\u{1D6A5}'), Some('\u{237}'));
        assert_eq!(math_alphanumeric_base('\u{1D7CA}'), Some('\u{3DC}'));
        // Unassigned holes between runs.
        assert_eq!(math_alphanumeric_base('\u{1D6A6}'), None);
        assert_eq!(math_alphanumeric_base('\u{1D7CC}'), None);
        // Everything outside the block, including the operators and
        // delimiters math documents are otherwise full of.
        for c in ['a', 'Z', '0', 'π', '∑', '∫', '√', '±', '∞', '→', '(', ' '] {
            assert_eq!(math_alphanumeric_base(c), None, "{c:?} is not a remap");
        }
    }

    /// The Letterlike Symbols are the block's holes, so they have to invert
    /// too or `\mathcal{B}` would keep degrading to `.notdef` while
    /// `\mathcal{A}` recovered.
    #[test]
    fn letterlike_holes_decompose_to_their_plain_letter() {
        assert_eq!(math_alphanumeric_base('\u{210E}'), Some('h'));
        assert_eq!(math_alphanumeric_base('\u{212C}'), Some('B'));
        assert_eq!(math_alphanumeric_base('\u{2112}'), Some('L'));
        assert_eq!(math_alphanumeric_base('\u{211C}'), Some('R'));
        assert_eq!(math_alphanumeric_base('\u{2115}'), Some('N'));
        assert_eq!(math_alphanumeric_base('\u{2147}'), Some('e'));
    }
}
