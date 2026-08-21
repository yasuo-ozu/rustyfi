use crate::context::Script;
use crate::length::Length;

/// An abstract handle to a loaded font face. Milestone 1 knows three: the
/// base-14 Helvetica family (regular/bold/oblique); later phases hand out
/// keys from a real font registry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FontKey(pub u16);

/// OpenType MATH `MathConstants` table, each field stored as a RATIO of
/// the font size (design-units ÷ `units_per_em`, or percent ÷ 100 for the
/// two scale-downs) so lang-side callers just multiply by
/// `ctx.font_size`/the current script size. Mirrors upstream
/// `FontFormat.math_constants` (fontFormat.ml:2292-2323).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MathConstants {
    pub axis_height: f64,
    pub superscript_bottom_min: f64,
    pub superscript_shift_up: f64,
    /// `superscript_shift_up_cramped` — OpenType `SuperscriptShiftUpCramped`,
    /// the lowered shift-up used when the enclosing sub-formula is "cramped"
    /// (TeXbook Appendix G rule 18a).
    pub superscript_shift_up_cramped: f64,
    pub superscript_baseline_drop_max: f64,
    pub subscript_top_max: f64,
    pub subscript_shift_down: f64,
    pub subscript_baseline_drop_min: f64,
    /// `script_percent_scale_down / 100`.
    pub script_scale_down: f64,
    /// `script_script_percent_scale_down / 100`.
    pub script_script_scale_down: f64,
    pub space_after_script: f64,
    pub sub_superscript_gap_min: f64,
    pub fraction_rule_thickness: f64,
    /// `fraction_numerator_display_style_shift_up`.
    pub fraction_numer_shift_up: f64,
    /// `fraction_num_display_style_gap_min`.
    pub fraction_numer_gap_min: f64,
    /// `fraction_denominator_display_style_shift_down`.
    pub fraction_denom_shift_down: f64,
    /// `fraction_denom_display_style_gap_min`.
    pub fraction_denom_gap_min: f64,
    pub radical_extra_ascender: f64,
    pub radical_rule_thickness: f64,
    /// `radical_display_style_vertical_gap`.
    pub radical_vertical_gap: f64,
    pub upper_limit_gap_min: f64,
    pub upper_limit_baseline_rise_min: f64,
    pub lower_limit_gap_min: f64,
    pub lower_limit_baseline_drop_min: f64,
}

/// Which corner of a math-kerned glyph a `MathKernInfo` entry describes
/// (OpenType MATH `MathKernInfoRecord`: top-right/top-left/bottom-right/
/// bottom-left).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MathCorner {
    TopRight,
    TopLeft,
    BottomRight,
    BottomLeft,
}

/// Classify one character into the four-way script bucket a font scheme is
/// indexed by (D1b, 2). Deviation from the spec's originally-proposed
/// standalone `CharScript` enum: this port already has `context::Script`
/// (Group E, `set-dominant-*-script`) with the exact same four
/// constructors in the exact same order (`HanIdeographic=0, Kana=1,
/// Latin=2, OtherScript=3`) — introducing a second, structurally-identical
/// enum just to keep "per-char classifier" and "context-stored dominant
/// script" conceptually separate would add a conversion at every call site
/// for no behavioral gain, so this reuses `Script` directly as the
/// per-char classification result too.
///
/// Upstream classifies via `Scripts.txt` + East-Asian-width
/// (`scriptDataMap.ml:74-167`, itself labelled "temporary" by its own
/// comment); this range classifier has no unidata file to ship and matches
/// upstream's *observable* output for the stdja corpus, not the full
/// Unicode script property. CJK punctuation/fullwidth forms classify as
/// `HanIdeographic` (not `OtherScript`) so `「」。、` render in the CJK
/// (mincho) face, matching upstream's Kana/Han default-font assignment.
pub fn char_script(c: char) -> Script {
    match c as u32 {
        // Hiragana, Katakana (+ phonetic extensions).
        0x3040..=0x30FF | 0x31F0..=0x31FF => Script::Kana,
        // CJK Unified Ideographs (+ Ext-A), CJK symbols/punctuation,
        // compatibility ideographs, halfwidth/fullwidth forms.
        0x3400..=0x4DBF
        | 0x4E00..=0x9FFF
        | 0xF900..=0xFAFF
        | 0x3000..=0x303F
        | 0xFF00..=0xFFEF
        | 0x20000..=0x2FA1F => Script::HanIdeographic,
        // Basic Latin .. Latin Extended-B.
        0x0000..=0x024F => Script::Latin,
        _ => Script::OtherScript,
    }
}

/// The seam between typesetting and font data: the line breaker and the box
/// builders only measure through this trait. Milestone 1 implements it with
/// hardcoded base-14 AFM tables (rustyfi-pdf); phase 5 replaces that with a
/// ttf-parser-backed registry.
pub trait FontMetrics {
    /// Horizontal advance of `c` at `size`, or `None` if the font has no
    /// glyph for it.
    fn advance(&self, font: FontKey, c: char, size: Length) -> Option<Length>;

    /// Height above the baseline at `size`.
    fn ascender(&self, font: FontKey, size: Length) -> Length;

    /// Depth below the baseline at `size` (a positive value).
    fn descender(&self, font: FontKey, size: Length) -> Length;

    /// One glyph's vertical extent from its ACTUAL bounding box —
    /// `(height above baseline = ymax, depth below baseline = -ymin)`, both in
    /// `size` units. `None` when the provider has no per-glyph bbox (base-14 /
    /// test stubs), in which case `run_vextent` falls back to
    /// `ascender`/`descender`. This is how SATySFi measures glyphs
    /// (`fontFormat.ml`'s `get_glyph_metrics`: `hgt = ymax`, `dpt = ymin`).
    fn glyph_vextent(&self, _font: FontKey, _c: char, _size: Length) -> Option<(Length, Length)> {
        None
    }

    /// A text run's `(height, depth)` the way SATySFi's `get_metrics_of_word`
    /// (`fontInfo.ml:192`) computes it: the MAX glyph `ymax` and MAX `-ymin`
    /// over the run's actual glyph bounding boxes — NOT the font-level
    /// ascender/descender. Starting the folds at zero clamps a run with no
    /// descenders (Japanese, digits, TOC leader dots) to depth 0, matching
    /// SATySFi's much tighter inter-line advance for such content. Falls back
    /// to `ascender`/`descender` when no glyph exposes a bbox.
    fn run_vextent(&self, font: FontKey, text: &str, size: Length) -> (Length, Length) {
        let mut hgt = Length::ZERO;
        let mut dpt = Length::ZERO;
        let mut any = false;
        for c in text.chars() {
            if let Some((h, d)) = self.glyph_vextent(font, c, size) {
                hgt = hgt.max(h);
                dpt = dpt.max(d);
                any = true;
            }
        }
        if any {
            (hgt, dpt)
        } else {
            (self.ascender(font, size), self.descender(font, size))
        }
    }

    fn text_width(&self, font: FontKey, text: &str, size: Length) -> Option<Length> {
        let mut w = Length::ZERO;
        for c in text.chars() {
            w += self.advance(font, c, size)?;
        }
        Some(w)
    }

    /// The font's OpenType MATH `MathConstants` table, or `None` when the
    /// font has no MATH table (every base-14/non-math provider). Lang-side
    /// math layout (`MathC` resolver) falls back to the pre-MATH-table
    /// fixed constants whenever this is `None`, so a provider that never
    /// overrides it (like `Base14Metrics`) keeps today's fixtures
    /// byte-identical.
    fn math_constants(&self, _font: FontKey) -> Option<MathConstants> {
        None
    }

    /// The italic correction of `c` at `size` (OpenType MATH
    /// `MathItalicsCorrectionInfo`), or `None` when the font has no MATH
    /// table or no entry for this glyph.
    fn italic_correction(&self, _font: FontKey, _c: char, _size: Length) -> Option<Length> {
        None
    }

    /// The OpenType MATH per-glyph corner kern of `c` at `size`, sampled at
    /// correction height `corr` (`MathKernInfo`/`MathKern`), or `None` when
    /// the font has no MATH table or no kern data for this glyph/corner.
    fn math_kern(
        &self,
        _font: FontKey,
        _c: char,
        _size: Length,
        _corner: MathCorner,
        _corr: Length,
    ) -> Option<Length> {
        None
    }

    /// A vertically-grown MATH variant of `c` at `size`, selected per
    /// `policy` (OpenType MATH `MathVariants`, — big operators/stretchy
    /// delimiters). `None` when the font has no MATH table, no vertical
    /// construction for `c`, or the construction has no prepared variant
    /// records (assembly-only — out of §B3 scope); every caller must treat
    /// `None` as "use the base glyph unchanged" (`push_char_glyph`), so a
    /// provider that never overrides this (every base-14 provider) leaves
    /// every pre-B3 fixture byte-identical.
    fn math_vertical_variant(
        &self,
        _font: FontKey,
        _c: char,
        _size: Length,
        _policy: VertVariantPolicy,
    ) -> Option<MathVariantGlyph> {
        None
    }

    /// Build a vertically-stretched delimiter/big-op from the OpenType MATH
    /// `GlyphAssembly` of `c` (the stretch-beyond-the-largest-discrete-variant
    /// path). Returns the placed parts as `(gid, dy, advance)`, bottom-to-top,
    /// where `dy` is the **y-up, box-local** vertical offset of the part's own
    /// baseline (the bottom part sits at `dy = 0`, each subsequent part is
    /// raised by the previous part's advance minus their connector overlap),
    /// and `advance` is the part glyph's design-unit `full_advance` scaled to
    /// `size` (the vertical extent it contributes). The parts stack with
    /// overlaps `>= min_connector_overlap`, repeating `extender` parts as many
    /// times as needed to reach `target`. `None` when the font has no MATH
    /// table, no vertical construction for `c`, or that construction has no
    /// `GlyphAssembly` — every caller must treat `None` as "fall back to the
    /// largest discrete variant" (`push_delimiter_glyph`), so a provider that
    /// never overrides this (every base-14 provider) is unaffected.
    fn math_vertical_assembly(
        &self,
        _font: FontKey,
        _c: char,
        _size: Length,
        _target: Length,
    ) -> Option<Vec<(u16, Length, Length)>> {
        None
    }

    /// Resolve a registry abbrev (`"ipaexm"`, `"Junicode-b"`, ...) to its
    /// `FontKey` (D1a). `None` means either "no such abbrev in this
    /// provider's registry" or "this provider has no registry at all" (every
    /// pre-D1 provider, `Base14Metrics`) — the caller then falls back to the
    /// milestone-1 3-face name heuristic (`resolve_font_abbrev` free fn,
    /// rustyfi-lang), keeping every pre-D1 `set-font` call byte-identical.
    fn resolve_font_abbrev(&self, _abbrev: &str) -> Option<FontKey> {
        None
    }

    /// The inverse of [`FontMetrics::resolve_font_abbrev`]: which registry
    /// abbrev minted `key`. This exists for 0.0.6's `get-font`, whose result
    /// type `tFONT = string * float * float` leads with an ABBREV — upstream
    /// keeps the abbrev in `context_main.font_scheme` and only resolves it to
    /// a file at render time, whereas this port resolves eagerly at `set-font`
    /// and stores a `FontKey`, so the name has to be recovered from whoever
    /// minted the key.
    ///
    /// It is a genuine inverse where it answers at all: `build_store`
    /// allocates one `FontKey` per CONFIGURED abbrev even when two abbrevs
    /// name the same font file (they share a `files` index, not a key), so no
    /// key is reachable from two abbrevs. `None` means the key was never named
    /// by the registry — the three seeded default faces (regular/bold/oblique,
    /// `FontKey(0..3)`), anything a bare `TtfFontStore::load` produced, and
    /// every `Base14Metrics` key. `get-font` reports those as `""` rather than
    /// inventing a name; nothing in the corpus reads the slot (every caller,
    /// and upstream's own `convertText.ml:78`, writes `let (_, ratio, _) =`),
    /// so the ratio and rising — which are exact either way — are what
    /// actually matters.
    fn font_abbrev(&self, _key: FontKey) -> Option<String> {
        None
    }

    /// The configured default `(font, ratio, rising)` for `script`, from
    /// `default-font.satysfi-hash`'s `scripts` block (D1a). `None` means "no
    /// scheme configured for this script" — the caller then falls back to
    /// `(ctx.font, 1.0, 0.0)`, i.e. today's single-font behavior.
    fn default_script_font(&self, _script: Script) -> Option<(FontKey, f64, f64)> {
        None
    }

    /// The configured default math font, from `default-font.satysfi-hash`'s
    /// optional `"math"` abbrev (Slice B). `None` means "no math default
    /// configured" — the caller (`get-initial-context`) then leaves
    /// `Context::math_font` at its `Context::initial` seed (`FontKey(0)`, the
    /// regular text face), i.e. today's behavior. Every pre-Slice-B provider
    /// (`Base14Metrics`, a bare `TtfFontStore::load`, a registry with no
    /// `"math"` entry) returns `None` here, so this is purely additive.
    fn default_math_font(&self) -> Option<FontKey> {
        None
    }
}

/// How to pick a vertically-grown MATH variant (`MathVariants`, §B3).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum VertVariantPolicy {
    /// v0.0.6 big-operator policy (`fontInfo.ml:386-401`): the 2nd record if
    /// present, else the 1st ("somewhat ad-hoc; uses the second smallest" —
    /// upstream's own comment). Upstream's `is_in_display && is_big` guard
    /// reduces to just `is_big` here since `convert_math_char` hardcodes
    /// `is_in_display = true`; the port tracks no display/inline distinction
    /// and needs none — see the caller (`push_big_char_glyph`) for detail.
    BigOp,
    /// Stretchy-delimiter policy: the smallest record whose
    /// `advance_measurement` covers `Length`, else the largest record.
    AtLeast(Length),
}

/// One selected vertical variant with real per-glyph ink metrics at size
/// (`fontFormat.ml:2257` `get_math_glyph_metrics`: `hgt = max(0, ymax)`,
/// `dpt = -min(0, ymin)`, both from the variant glyph's own outline bbox).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MathVariantGlyph {
    /// Raw font glyph id — NOT necessarily cmap-reachable (variant glyphs
    /// like `summation.v1` typically have no cmap entry at all); the CID
    /// writer emits this directly as an Identity-H content byte pair rather
    /// than re-deriving it from a character.
    pub gid: u16,
    pub advance: Length,
    pub height: Length,
    pub depth: Length,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn char_script_classifies_the_stdja_corpus() {
        assert_eq!(char_script('あ'), Script::Kana); // Hiragana
        assert_eq!(char_script('ア'), Script::Kana); // Katakana
        assert_eq!(char_script('漢'), Script::HanIdeographic);
        assert_eq!(char_script('A'), Script::Latin);
        assert_eq!(char_script('z'), Script::Latin);
        assert_eq!(char_script('é'), Script::Latin); // Latin-1 Supplement
        assert_eq!(char_script('→'), Script::OtherScript); // U+2192 arrow
        assert_eq!(char_script('。'), Script::HanIdeographic); // CJK punctuation
        assert_eq!(char_script('「'), Script::HanIdeographic);
        assert_eq!(char_script('、'), Script::HanIdeographic);
    }
}
