use crate::font::FontKey;
use crate::length::Length;

/// The typesetting context (a milestone-1 subset of `context_main` in
/// horzBox.ml). Grows field by field as primitives need them.
#[derive(Clone, Debug, PartialEq)]
pub struct Context {
    pub font: FontKey,
    pub font_size: Length,
    /// Baseline-to-baseline distance.
    pub leading: Length,
    /// Wrap width for paragraphs.
    pub paragraph_width: Length,
    /// Extra vertical skip inserted above a paragraph
    /// (`set-paragraph-margin`'s first argument; v0.0.6
    /// `context_main.paragraph_top`, horzBox.ml:227). Not wired into any
    /// box-producing primitive yet — a future `+p` is expected to turn this
    /// into a leading `VertBox::Skip`.
    pub paragraph_top: Length,
    /// Extra vertical skip inserted below a paragraph
    /// (`set-paragraph-margin`'s second argument; v0.0.6
    /// `context_main.paragraph_bottom`, horzBox.ml:228). Same "not wired in
    /// yet" status as `paragraph_top`.
    pub paragraph_bottom: Length,
}

impl Context {
    /// The default context `get-initial-context` hands to `document`.
    pub fn initial(paragraph_width: Length) -> Context {
        Context {
            font: FontKey(0),
            font_size: Length::pt(12.0),
            leading: Length::pt(18.0),
            paragraph_width,
            // v0.0.6's `get_pdf_mode_initial_context`
            // (primitives.cppo.ml:514-515) defaults both to 18pt.
            paragraph_top: Length::pt(18.0),
            paragraph_bottom: Length::pt(18.0),
        }
    }
}

/// Page geometry (A4 with even margins by default).
#[derive(Clone, Debug, PartialEq)]
pub struct PageGeometry {
    pub paper_width: Length,
    pub paper_height: Length,
    /// Top-left corner of the text area.
    pub text_origin: (Length, Length),
    pub text_width: Length,
    pub text_height: Length,
}

impl Default for PageGeometry {
    fn default() -> Self {
        let paper_width = Length::from_unit(210.0, "mm").unwrap();
        let paper_height = Length::from_unit(297.0, "mm").unwrap();
        let margin = Length::from_unit(25.0, "mm").unwrap();
        PageGeometry {
            paper_width,
            paper_height,
            text_origin: (margin, margin),
            text_width: paper_width - margin - margin,
            text_height: paper_height - margin - margin,
        }
    }
}
