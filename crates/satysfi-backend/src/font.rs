use crate::length::Length;

/// An abstract handle to a loaded font face. Milestone 1 knows three: the
/// base-14 Helvetica family (regular/bold/oblique); later phases hand out
/// keys from a real font registry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FontKey(pub u16);

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
}
