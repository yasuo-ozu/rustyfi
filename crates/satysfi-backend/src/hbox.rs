use crate::font::FontKey;
use crate::graphics::GraphicsElem;
use crate::length::Length;
use crate::math::MathGlyph;
use crate::tabular::TabularBox;
use crate::vbox::VertBox;

/// Which font/size a string box is set in (`horz_string_info`).
#[derive(Clone, Debug, PartialEq)]
pub struct HorzStringInfo {
    pub font: FontKey,
    pub size: Length,
}

/// An index into a document-wide table of decoded raster images
/// (`DocumentValue::images` in satysfi-lang), the way `FontKey` indexes a
/// font table. `Value::Image` (satysfi-lang) carries one of these as its
/// only payload, and `PureHorzBox::Image` places one on a page; neither
/// carries the decoded bytes directly, so cloning a box (routine during line
/// breaking) never copies image data.
///
/// docs/plans/math-images.md §Slice 1: raster images.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ImageId(pub usize);

/// An opaque index into a lang-side table of deferred page-break-hook
/// closures (`Interp::hooks`), the exact analogue of `ImageId` but pointing
/// at a *computation* instead of a resource. `break_pages` places the box
/// this token lives in like any other content and never learns what the hook
/// computes; a lang-side post-pass (`fire_hooks`) reads the token back once
/// geometry is final. See docs/plans/hooks-annotations-crossref.md.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct HookId(pub usize);

/// A decoded raster image, referenced by `PureHorzBox::Image`/`Value::Image`
/// via its `ImageId` index. Mirrors v0.0.6's `ImageInfo` (`imageInfo.ml`)
/// trimmed to what Slice 1 needs: pixel dimensions (for the
/// `use-image-by-width` aspect-ratio computation) plus enough sample data to
/// emit a PDF Image XObject directly.
///
/// **Slice 1 simplification** (see `docs/plans/math-images.md`'s Risks
/// section): every source format is flattened to 8-bit `DeviceRGB` and any
/// alpha channel is dropped. Transparency (`/SMask`) and a JPEG `DCTDecode`
/// passthrough (rather than a full decode/re-encode) are both roadmap, not
/// Slice 1.
#[derive(Clone, Debug, PartialEq)]
pub struct ImageResource {
    /// Row-major, top-to-bottom, 3-bytes-per-pixel RGB8 samples with no
    /// padding — exactly what `pdf_writer::Chunk::image_xobject` wants for a
    /// `DeviceRGB` image at `bits_per_component(8)`.
    pub samples: Vec<u8>,
    pub px_w: u32,
    pub px_h: u32,
}

/// A milestone-1 subset of `pure_horz_box` from horzBox.ml, keeping its
/// vocabulary so the full port extends rather than replaces it.
#[derive(Clone, Debug, PartialEq)]
pub enum PureHorzBox {
    /// Fixed text with pre-measured dimensions.
    InnerString {
        info: HorzStringInfo,
        text: String,
        width: Length,
        height: Length,
        depth: Length,
    },
    /// Interword glue.
    OuterEmpty {
        natural: Length,
        shrinkable: Length,
        stretchable: Length,
    },
    /// Infinitely stretchable glue (`inline-fil`).
    OuterFil,
    /// A fixed-width empty box with no stretch/shrink (`inline-skip`;
    /// v0.0.6: `PHSFixedEmpty`). Unlike `OuterEmpty` this is never a legal
    /// line-break point (see `is_glue`).
    FixedEmpty { width: Length },
    /// A raster image placed at a fixed on-page size (`use-image-by-width`'s
    /// result). `width`/`height` are the already-computed on-page
    /// dimensions (v0.0.6 `ImageInfo.get_height_from_width`); `image` looks
    /// the decoded bytes up in the document's image table. Like
    /// `FixedEmpty`, this is never a legal line-break point (`is_glue`).
    ///
    /// docs/plans/math-images.md §Slice 1.
    Image {
        width: Length,
        height: Length,
        image: ImageId,
    },
    /// A break point that may or may not be taken (v0.0.6's
    /// `LBDiscretionary(penalty, id, pre, post_nobreak, post_break)`,
    /// `ref:src/backend/lineBreakBox.ml:22-27`). If the paragraph breaker
    /// chooses to break here, `pre_break` renders at the end of the closed
    /// line and `post_break` at the start of the next; otherwise `no_break`
    /// renders in its place. UAX#14 (§3) only needs zero-width inter-chunk
    /// break points with all three slots empty; hyphenation (§4, future)
    /// is expected to fill `pre_break` with a hyphen glyph. Unlike
    /// `OuterEmpty`/`OuterFil` this is not "glue" (see `is_glue`'s doc) —
    /// it is scored separately via `is_break_point`/`break_penalty`.
    Discretionary {
        penalty: i32,
        pre_break: Vec<PureHorzBox>,
        post_break: Vec<PureHorzBox>,
        no_break: Vec<PureHorzBox>,
    },
    /// A box carrying resolved `graphics` elements (`inline-graphics`;
    /// v0.0.6: `PHGFixedGraphics`), coordinates already relative to the
    /// box's baseline-left origin. Unlike `Image`-style boxes this carries a
    /// real depth (a graphics box can extend below the baseline), so both
    /// `height` and `depth` feed line metrics (see `linebreak.rs`'s
    /// `measure`/`layout_line`). Never a legal line-break point (see
    /// `is_glue`).
    Graphics {
        width: Length,
        height: Length,
        depth: Length,
        elems: Vec<GraphicsElem>,
    },
    /// A laid-out inline math run (`${…}`; docs/plans/math-engine.md §Slice
    /// 1): one box carrying its own pre-shifted sub-glyphs, each with a
    /// vertical offset relative to this box's baseline (`MathGlyph::dy`) —
    /// the line model has only a horizontal `dx` per box and a single
    /// `baseline_y` per line, so a superscript can't be a separate box (see
    /// the plan's "structural difference" note). `width`/`height`/`depth` are
    /// the run's outer metrics (computed by `read_math`), so the line
    /// breaker never re-enters the math engine. Never a legal line-break
    /// point (see `is_glue`) — a math run is laid out and flowed atomically.
    Math {
        width: Length,
        height: Length,
        depth: Length,
        glyphs: Vec<MathGlyph>,
    },
    /// A deferred page-break hook (`hook-page-break`; v0.0.6's
    /// `PHGHookPageBreak`). Carries only the opaque token — no closure, no
    /// trait object — so the box stays POD-cloneable; `break_pages` places
    /// it like any other zero-width content, and a lang-side post-pass
    /// (`fire_hooks`) fires the stored closure once placement is final.
    /// Renders nothing (see the PDF writers' wildcard arm).
    ///
    /// docs/plans/hooks-annotations-crossref.md §Slice 1.
    HookPageBreak { id: HookId },
    /// A ruled grid box (`tabular`; v0.0.6's `PHGFixedTabular`) — the first
    /// composite box: it carries other already-laid-out inline boxes (each
    /// cell's own `Vec<(Length, PureHorzBox)>` run, `tabular::TabularCellBox`)
    /// plus the resolved rule graphics from the user's callback. See
    /// docs/plans/table-subsystem.md §4 for how the PDF writers recurse into
    /// it (`emit_box`) and reconcile its three coordinate frames.
    Tabular(TabularBox),
    /// An inline box carrying a whole block (`embed-block-top`;
    /// `docs/plans/context-box-prims.md` §Slice 1 rows 7-8; upstream's
    /// `PHGEmbeddedVert`/`HorzEmbeddedVertBreakable`). `block` is already
    /// broken into `VertBox` lines; the writer stacks them from the box's
    /// placed origin by reentering the same per-`PureHorzBox` emission a
    /// top-level line uses (the `Tabular` cells' recursion, above, is the
    /// same pattern one level up). FIRST CUT is ATOMIC — it does not split
    /// across a page boundary (see that plan's §Risks).
    EmbeddedBlock {
        width: Length,
        height: Length,
        depth: Length,
        block: Vec<VertBox>,
    },
}

/// TeX's forced-break convention: a discretionary penalty this low or
/// lower means the paragraph breaker must end a line there. `text_to_boxes`
/// uses this to turn a UAX#14 `Mandatory` boundary (e.g. a literal newline)
/// into a break the DP cannot skip over (see `linebreak.rs`'s `floor`).
pub const FORCED_BREAK_PENALTY: i32 = -10_000;

impl PureHorzBox {
    pub fn natural_width(&self) -> Length {
        match self {
            PureHorzBox::InnerString { width, .. } => *width,
            PureHorzBox::OuterEmpty { natural, .. } => *natural,
            PureHorzBox::OuterFil => Length::ZERO,
            PureHorzBox::FixedEmpty { width } => *width,
            PureHorzBox::Image { width, .. } => *width,
            // §3 never fills the slots, so there's nothing to sum yet;
            // §4 (hyphenation) will need to measure `no_break`/`pre_break`.
            PureHorzBox::Discretionary { .. } => Length::ZERO,
            PureHorzBox::Graphics { width, .. } => *width,
            PureHorzBox::Math { width, .. } => *width,
            // `EvHorzHookPageBreak` has width `Length.zero` (pageInfo.ml:42).
            PureHorzBox::HookPageBreak { .. } => Length::ZERO,
            PureHorzBox::Tabular(tab) => tab.width,
            PureHorzBox::EmbeddedBlock { width, .. } => *width,
        }
    }

    /// `false` for every variant except the two glue kinds — including the
    /// new `Image` (an image is exactly as "fixed" a box as `FixedEmpty`),
    /// which falls out of this `matches!` without needing its own arm.
    pub fn is_glue(&self) -> bool {
        matches!(
            self,
            PureHorzBox::OuterEmpty { .. } | PureHorzBox::OuterFil
        )
    }

    /// A legal paragraph-break candidate: glue (today's only breakpoints)
    /// or a discretionary (UAX#14/hyphenation break points). CJK text has
    /// no glue at all, so discretionaries are its *only* break candidates.
    pub fn is_break_point(&self) -> bool {
        self.is_glue() || matches!(self, PureHorzBox::Discretionary { .. })
    }

    /// The break's own penalty (TeX's discretionary/`\penalty`
    /// convention): 0 for glue (no preference either way), a
    /// discretionary's own `penalty` otherwise.
    pub fn break_penalty(&self) -> i32 {
        match self {
            PureHorzBox::Discretionary { penalty, .. } => *penalty,
            _ => 0,
        }
    }

    /// Whether breaking here is not just legal but mandatory
    /// (`penalty <= FORCED_BREAK_PENALTY`).
    pub fn is_forced_break(&self) -> bool {
        self.break_penalty() <= FORCED_BREAK_PENALTY
    }
}

/// `horz_box`: the wrapper stays even though `Pure` is its only variant so
/// far, keeping line-break input the shape lineBreak.ml expects.
#[derive(Clone, Debug, PartialEq)]
pub enum HorzBox {
    Pure(PureHorzBox),
}
