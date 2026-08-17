use crate::font::FontKey;
use crate::graphics::Color;
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
    /// A manual vertical shift applied to text set under this context
    /// (`set-manual-rising`'s argument; v0.0.6 `context_main.manual_rising`,
    /// horzBox.ml:232). Like `paragraph_top`/`paragraph_bottom`, not yet
    /// wired into any box-producing primitive (upstream's `PHGRising` box
    /// has no analogue here yet) — `pervasives.satyh`'s `\SATySFi`/`\LaTeX`/
    /// `\TeX` set it, but this port has nothing downstream that reads it.
    pub manual_rising: Length,
    /// `set-text-color`/`get-text-color` (docs/plans/context-box-prims.md
    /// §Slice 1 row 1-2; v0.0.6 `context_main.text_color`). FAITHFUL storage
    /// — round-trips exactly (`itemize.satyh`'s bullet `fill` color depends
    /// on it) — but not yet *consumed* by either PDF writer's glyph
    /// emission, which still always draws in black.
    pub text_color: Color,
    /// `set-hyphen-penalty` (row 3; v0.0.6 `context_main.hyphen_badness`).
    /// Stored faithfully; no consumer yet (this port has no hyphenation).
    pub hyphen_badness: i64,
    /// `set-space-ratio`'s three fields (row 4; v0.0.6
    /// `context_main.space_natural`/`space_shrink`/`space_stretch`). Stored
    /// faithfully; interword-glue sizing still uses the line breaker's own
    /// fixed ratios until `docs/plans/text-rendering.md` wires these in.
    pub space_natural: f64,
    pub space_shrink: f64,
    pub space_stretch: f64,
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
            manual_rising: Length::ZERO,
            // v0.0.6's `get_pdf_mode_initial_context` (primitives.cppo.ml):
            // `text_color = DeviceGray 0.`, `hyphen_badness = 100`,
            // `space_natural = 0.33`, `space_shrink = 0.08`,
            // `space_stretch = 0.16`.
            text_color: Color::Gray(0.0),
            hyphen_badness: 100,
            space_natural: 0.33,
            space_shrink: 0.08,
            space_stretch: 0.16,
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

impl PageGeometry {
    /// Build a geometry from `page`'s paper dimensions
    /// (docs/plans/document-page-model.md §"`page` -> dimensions"). Only
    /// `paper_width`/`paper_height` are read by the PDF writer; the
    /// `text_*` fields are vestigial here — each page's real text area
    /// lives in its `PlacedLine` coordinates, set per page by the
    /// content scheme (`chop_page`'s caller), not by this geometry.
    pub fn for_paper(paper_width: Length, paper_height: Length) -> PageGeometry {
        PageGeometry {
            paper_width,
            paper_height,
            text_origin: (Length::ZERO, Length::ZERO),
            text_width: paper_width,
            text_height: paper_height,
        }
    }
}

/// v0.0.6's `page_size` (`primitives.cppo.ml:203-212`) — the paper-size
/// constant set `page-break`'s first argument selects from, the port of
/// `get_pdf_paper` (`handlePdf.ml:406`, `Pdfpaper.t` dims from
/// `pdfpaper.ml`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PaperSize {
    A0,
    A1,
    A2,
    A3,
    A4,
    A5,
    USLetter,
    USLegal,
    UserDefined(Length, Length),
}

impl PaperSize {
    /// `(width, height)` in points.
    pub fn dims(&self) -> (Length, Length) {
        fn mm(w: f64, h: f64) -> (Length, Length) {
            (
                Length::from_unit(w, "mm").unwrap(),
                Length::from_unit(h, "mm").unwrap(),
            )
        }
        fn inch(w: f64, h: f64) -> (Length, Length) {
            (
                Length::from_unit(w, "inch").unwrap(),
                Length::from_unit(h, "inch").unwrap(),
            )
        }
        match *self {
            PaperSize::A0 => mm(841.0, 1189.0),
            PaperSize::A1 => mm(594.0, 841.0),
            PaperSize::A2 => mm(420.0, 594.0),
            PaperSize::A3 => mm(297.0, 420.0),
            PaperSize::A4 => mm(210.0, 297.0),
            PaperSize::A5 => mm(148.0, 210.0),
            PaperSize::USLetter => inch(8.5, 11.0),
            PaperSize::USLegal => inch(8.5, 14.0),
            PaperSize::UserDefined(w, h) => (w, h),
        }
    }
}
