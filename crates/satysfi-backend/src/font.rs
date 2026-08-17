use crate::length::Length;

/// An abstract handle to a loaded font face. Milestone 1 knows three: the
/// base-14 Helvetica family (regular/bold/oblique); later phases hand out
/// keys from a real font registry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FontKey(pub u16);

/// OpenType MATH `MathConstants` table (`docs/plans/math-engine.md` §B),
/// each field stored as a RATIO of the font size (design-units ÷
/// `units_per_em`, or percent ÷ 100 for the two scale-downs) so lang-side
/// callers just multiply by `ctx.font_size`/the current script size.
/// Mirrors upstream `FontFormat.math_constants` (fontFormat.ml:2292-2323).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MathConstants {
    pub axis_height: f64,
    pub superscript_bottom_min: f64,
    pub superscript_shift_up: f64,
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

/// The seam between typesetting and font data: the line breaker and the box
/// builders only measure through this trait. Milestone 1 implements it with
/// hardcoded base-14 AFM tables (satysfi-pdf); phase 5 replaces that with a
/// ttf-parser-backed registry.
pub trait FontMetrics {
    /// Horizontal advance of `c` at `size`, or `None` if the font has no
    /// glyph for it.
    fn advance(&self, font: FontKey, c: char, size: Length) -> Option<Length>;

    /// Height above the baseline at `size`.
    fn ascender(&self, font: FontKey, size: Length) -> Length;

    /// Depth below the baseline at `size` (a positive value).
    fn descender(&self, font: FontKey, size: Length) -> Length;

    fn text_width(&self, font: FontKey, text: &str, size: Length) -> Option<Length> {
        let mut w = Length::ZERO;
        for c in text.chars() {
            w += self.advance(font, c, size)?;
        }
        Some(w)
    }

    /// The font's OpenType MATH `MathConstants` table, or `None` when the
    /// font has no MATH table (every base-14/non-math provider). Lang-side
    /// math layout (`docs/plans/math-engine.md` §B1's `MathC` resolver)
    /// falls back to the pre-MATH-table fixed constants whenever this is
    /// `None`, so a provider that never overrides it (like `Base14Metrics`)
    /// keeps today's fixtures byte-identical.
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
    /// `policy` (OpenType MATH `MathVariants`, `docs/plans/math-engine.md`
    /// §B3 — big operators/stretchy delimiters). `None` when the font has no
    /// MATH table, no vertical construction for `c`, or the construction has
    /// no prepared variant records (assembly-only — out of §B3 scope);
    /// every caller must treat `None` as "use the base glyph unchanged"
    /// (`push_char_glyph`), so a provider that never overrides this (every
    /// base-14 provider) leaves every pre-B3 fixture byte-identical.
    fn math_vertical_variant(
        &self,
        _font: FontKey,
        _c: char,
        _size: Length,
        _policy: VertVariantPolicy,
    ) -> Option<MathVariantGlyph> {
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
