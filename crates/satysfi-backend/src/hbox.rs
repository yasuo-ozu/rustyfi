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
    /// A manual baseline raise (D1b, `ScriptFont::rising` scaled by the
    /// run's font size — `fontInfo.ml`'s `get_font_with_ratio`). `ZERO` for
    /// every pre-D1 construction site and every stdja default, so this
    /// field being carried instead of dropped changes no existing output —
    /// both PDF writers add it to the placed `ty` before `Tj`.
    pub rising: Length,
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

/// An opaque index into a lang-side table of deferred *decoration* closures
/// (`Interp::decos`) — `HookId`'s twin for §D frames
/// (docs/plans/hooks-annotations-crossref.md §D: the resolved struct layout).
/// The backend carries it through line breaking and never learns what the
/// deco draws; `fire_hooks` (satysfi-lang) fires it with the frame's placed
/// `(x, y, w, h, d)` and accumulates the returned graphics onto the page.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DecoId(pub usize);

/// An opaque index into a lang-side table of deferred `inline-graphics-outer`
/// callbacks (`Interp::outer_graphics`) — `HookId`'s exact pattern: the box
/// stays POD-cloneable, and a lang-side post-pass (`resolve_outer_graphics_*`
/// in satysfi-lang's primitives, run by `line-break`/`tabular`) reads the
/// token back once line layout has resolved the box's width.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GraphicsFnId(pub usize);

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
    /// `inline-graphics-outer` (v0.0.6 `PHGOuterFilGraphics`,
    /// vminst.ml:1891): a graphics box whose WIDTH stretches like
    /// `inline-fil` (upstream widinfo `{natural = 0; stretchable = Fils(1)}`,
    /// lineBreak.ml:40-48). `width` starts at ZERO and is written by
    /// `justify_line` with the box's per-fil slack share; the box is then
    /// replaced by a resolved `Graphics` in a lang-side post-pass that fires
    /// `fn_id`'s callback with that width (see `GraphicsFnId`). NOT glue
    /// (`is_glue` = false — upstream's box is pure content, never a break
    /// point), but counted as a fil by `measure`/`justify_line`.
    GraphicsOuter {
        height: Length,
        depth: Length,
        width: Length,
        fn_id: GraphicsFnId,
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
    ///
    /// `rules` (§B2, `docs/plans/math-engine.md`): filled paths the run
    /// needs alongside its glyphs — the fraction bar and radical sign/overbar
    /// are `Fill`s, not glyphs, since neither is drawable through a font's
    /// `Tj` at all. Box-local, y-**up** coordinates relative to this box's
    /// own baseline-left origin — exactly `PureHorzBox::Graphics::elems`'
    /// convention (see that variant's doc comment), NOT `MathGlyph::dy`'s
    /// sign (which happens to agree: up is positive either way, just via a
    /// different type). `natural_width` is unaffected (rules never extend the
    /// run's own advance — a bar/radical-sign always sits within `glyphs`'
    /// already-measured span). Empty for the Slice-1 `read_math` path (its
    /// `MathElem` tree has no `Fraction`/`Radical` production at all) and for
    /// every atom the faithful `layout_math_atom` path doesn't specially
    /// handle.
    Math {
        width: Length,
        height: Length,
        depth: Length,
        glyphs: Vec<MathGlyph>,
        rules: Vec<GraphicsElem>,
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
    /// An inline frame (`inline-frame-outer`/`-inner`/`-breakable`;
    /// upstream `PHGOuterFrame`/`PHGInnerFrame`/`HorzFrameBreakable`) —
    /// ATOMIC in this port: contents are pre-fit at their natural width
    /// (`fit_cell` — the same no-Context fit tabular cells use), and the
    /// frame never splits across a line break (the breakable variant fires
    /// only its whole-frame deco, see `prim_inline_frame_breakable`).
    /// `width`/`height`/`depth` are the OUTER dims (padding included;
    /// baseline unshifted — padding grows the box, upstream lineBreak.ml's
    /// frame metrics). `contents` carry x-offsets from the frame's left edge
    /// (pad-L already applied), all on the frame's own baseline — the
    /// writers recurse exactly like `Tabular` cells. `deco` is fired
    /// lang-side after placement; the writers draw nothing for it here.
    Frame {
        width: Length,
        height: Length,
        depth: Length,
        deco: DecoId,
        contents: Vec<(Length, PureHorzBox)>,
    },
    /// A placed block-frame marker (`VertBox::FrameStart`/`FrameEnd` after
    /// page breaking) — zero-width, renders nothing (writers' wildcard arm),
    /// read back by `fire_hooks` only.
    FrameMarker { id: DecoId, end: bool },
    /// `add-footnote`'s marker (v0.0.6 `PHGFootnote(imvblst)`,
    /// `horzBox.ml:283` → `ImHorzFootnote`, `:306`): a zero-width/height/
    /// depth inline box carrying the footnote's already-assembled block.
    /// Rides the paragraph like `HookPageBreak` (writers skip it via their
    /// wildcard arm); `chop_page` (pagebreak.rs) extracts it when the line
    /// carrying it is COMMITTED to a page, reserves the block's stacked
    /// height at the page bottom, and bottom-places the block in the same
    /// column (upstream `pageBreak.ml:131-142` + `handlePdf.ml:400-403`).
    /// The marker itself stays in the placed line's contents (render-inert)
    /// — extraction is a read-only scan, unlike upstream's removing
    /// `embed_page_info` (`pageInfo.ml:47`). Consequence: the block payload
    /// appears both (inert) inside its referencing line and (rendered) as
    /// bottom-placed lines — any future exhaustive consumer of a placed
    /// line's contents must treat this variant as inert or it will
    /// double-count the body.
    Footnote { block: Vec<VertBox> },
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
            // Un-taken discretionary: renders as `no_break` (§4, hyphenation
            // — `linebreak.rs`'s `line_content` handles the taken case,
            // which never reaches this generic accessor). Empty for §3
            // (UAX#14-only discretionaries), hence zero then.
            PureHorzBox::Discretionary { no_break, .. } => no_break
                .iter()
                .map(PureHorzBox::natural_width)
                .fold(Length::ZERO, |acc, w| acc + w),
            PureHorzBox::Graphics { width, .. } => *width,
            // Fil semantics: zero natural width, like `OuterFil` — a
            // resolved box is a `Graphics`, never re-measured as this
            // variant (see the variant's own doc comment).
            PureHorzBox::GraphicsOuter { .. } => Length::ZERO,
            PureHorzBox::Math { width, .. } => *width,
            // `EvHorzHookPageBreak` has width `Length.zero` (pageInfo.ml:42).
            PureHorzBox::HookPageBreak { .. } => Length::ZERO,
            PureHorzBox::Tabular(tab) => tab.width,
            PureHorzBox::EmbeddedBlock { width, .. } => *width,
            PureHorzBox::Frame { width, .. } => *width,
            PureHorzBox::FrameMarker { .. } => Length::ZERO,
            // Zero width, like `HookPageBreak` (`ImHorzFootnote` is skipped
            // by every width scan upstream, lineBreak.ml:1200/1254).
            PureHorzBox::Footnote { .. } => Length::ZERO,
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
