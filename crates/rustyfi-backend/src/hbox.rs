use crate::font::FontKey;
use crate::graphics::{Color, GraphicsElem};
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
    /// `set-text-color`'s value at the time this run/glyph was built
    /// (`Context::text_color`, `context.rs`). Both `PureHorzBox::InnerString`
    /// (text runs) and `MathGlyph` (math glyphs) embed this same struct, so
    /// one field threads color to both. `Color::Gray(0.0)` (black) is the
    /// PDF/HTML default; both writers emit NO color op for a black run, so
    /// every pre-existing all-black construction site stays byte-identical.
    pub color: Color,
}

/// An index into a document-wide table of decoded raster images
/// (`DocumentValue::images` in rustyfi-lang), the way `FontKey` indexes a
/// font table. `Value::Image` (rustyfi-lang) carries one of these as its
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
/// deco draws; `fire_hooks` (rustyfi-lang) fires it with the frame's placed
/// `(x, y, w, h, d)` and accumulates the returned graphics onto the page.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DecoId(pub usize);

/// An opaque index into a lang-side table of deferred `inline-graphics-outer`
/// callbacks (`Interp::outer_graphics`) — `HookId`'s exact pattern: the box
/// stays POD-cloneable, and a lang-side post-pass (`resolve_outer_graphics_*`
/// in rustyfi-lang's primitives, run by `line-break`/`tabular`) reads the
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
/// alpha channel is dropped — `samples`/`px_w`/`px_h` are always populated
/// this way (the HTML backend's `<img>` data URI and the PDF writer's
/// non-JPEG path both rely on that). Transparency (`/SMask`) is still
/// roadmap. A JPEG `DCTDecode` passthrough (JPEG DCTDecode passthrough
/// slice, see `write_image_xobjects` in `rustyfi-pdf`) is no longer
/// roadmap: `jpeg_dct` additionally carries the source's original,
/// still-DCT-encoded bytes when `load-image` recognized it as a baseline
/// JPEG, so the PDF writer can embed those bytes directly instead of
/// re-encoding the flattened RGB8 samples.
#[derive(Clone, Debug, PartialEq)]
pub struct ImageResource {
    /// Row-major, top-to-bottom, 3-bytes-per-pixel RGB8 samples with no
    /// padding — exactly what `pdf_writer::Chunk::image_xobject` wants for a
    /// `DeviceRGB` image at `bits_per_component(8)`.
    pub samples: Vec<u8>,
    pub px_w: u32,
    pub px_h: u32,
    /// `Some` when the source file `load-image` decoded was itself a
    /// baseline (or extended-sequential) 8-bit JPEG with 1 or 3 color
    /// components — see `sniff_baseline_jpeg_dct`'s doc comment for exactly
    /// which JPEGs qualify and why the rest fall back to `None` (and thus to
    /// the flattened-RGB8 embedding above). `None` for every non-JPEG
    /// source, and for a JPEG this port doesn't yet know how to map to a PDF
    /// colorspace without guessing (progressive, 12-bit, or 4-component
    /// CMYK/YCCK).
    pub jpeg_dct: Option<JpegDct>,
    /// `Some` when this resource is an imported page of an external PDF
    /// (`load-pdf-image`, docs/plans/design-load-pdf-image.md) rather than a
    /// decoded raster image. `samples`/`px_w`/`px_h` are left at their
    /// default/empty values in that case — every raster consumer keeps
    /// reading those fields unchanged; PDF-page consumers branch on this
    /// field instead. Additive: every pre-existing `ImageResource { .. }`
    /// construction site gets `pdf: None`.
    pub pdf: Option<PdfPageResource>,
}

/// An embedded page of an external PDF (`load-pdf-image`), carrying just
/// enough of the source page's object graph to re-emit it as a PDF **Form
/// XObject** (docs/plans/design-load-pdf-image.md §2-3). Parsed by
/// `rustyfi-lang` (via `lopdf`) and consumed by `rustyfi-pdf`'s writer; this
/// struct itself is `lopdf`-free plain data so `rustyfi-backend` need not
/// depend on `lopdf`.
#[derive(Clone, Debug, PartialEq)]
pub struct PdfPageResource {
    /// The source page's `/MediaBox`, `(x0, y0, x1, y1)` in raw PDF points
    /// (upstream `loadPdf.ml`: MediaBox only, no `/CropBox` fallback).
    pub media_box: (f64, f64, f64, f64),
    /// The page's content stream(s), already inflated (`/FlateDecode`
    /// resolved) and concatenated (with a separating space per PDF rules
    /// when a page has more than one content stream) — ready to wrap
    /// verbatim in a Form XObject's stream body.
    pub content: Vec<u8>,
    /// The imported object subtree reachable from the page's `/Resources`,
    /// self-contained and keyed by source object number so the writer can
    /// remap references to freshly allocated output `Ref`s. Local id `0` is
    /// reserved (PDF object number 0 is always free/unused in a well-formed
    /// file) and holds the page's own `/Resources` dictionary itself
    /// (whether it was a direct or an indirect object in the source); every
    /// other entry is a real source object number.
    pub resources: ImportedObjects,
}

/// A serialized subtree of a *foreign* PDF's object graph, self-contained
/// and neutral (no `lopdf` types) so it can cross the `rustyfi-backend`
/// boundary as plain data. See `PdfPageResource::resources`'s doc comment
/// for the local-id convention.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ImportedObjects(pub Vec<(u32, ObjRepr)>);

/// A minimal sum type mirroring the PDF object grammar, just enough to
/// re-emit an imported object verbatim (`docs/plans/design-load-pdf-image.md`
/// §2). `Ref(u32)` refers to another entry's local id in the same
/// `ImportedObjects` table (or, in principle, to an object outside the
/// imported subtree — the writer should treat an unresolved `Ref` as a bug
/// in the importer, not attempt to fetch it).
#[derive(Clone, Debug, PartialEq)]
pub enum ObjRepr {
    Null,
    Bool(bool),
    Int(i64),
    Real(f64),
    /// A PDF name's raw bytes, without the leading `/` and with `#xx`
    /// escapes already decoded (mirrors `lopdf::Object::Name`).
    Name(Vec<u8>),
    /// A PDF string's raw bytes (mirrors `lopdf::Object::String`, literal or
    /// hex — both collapse to bytes here since we only ever re-emit them).
    String(Vec<u8>),
    Ref(u32),
    Array(Vec<ObjRepr>),
    Dict(Vec<(Vec<u8>, ObjRepr)>),
    /// A stream object: its dictionary entries (excluding `/Length`, which
    /// the writer derives) plus already-decompressed content bytes. The
    /// writer decides filtering/compression at write time; the imported
    /// content is kept in cleartext form here so no `lopdf`-specific codec
    /// state needs to cross the boundary.
    Stream(Vec<(Vec<u8>, ObjRepr)>, Vec<u8>),
}

/// The original, still-DCT-encoded bytes of a source JPEG file, plus just
/// enough metadata (`components`) for the PDF writer to pick the matching
/// `/ColorSpace` — never re-derived from the flattened `samples` (those have
/// already lost whatever the JPEG's own YCbCr subsampling/quantization
/// looked like; the whole point of a passthrough is to hand the PDF viewer
/// the exact bytes the encoder produced).
#[derive(Clone, Debug, PartialEq)]
pub struct JpegDct {
    /// The complete original file contents, `FFD8` (SOI) to `FFD9` (EOI)
    /// and everything in between, byte-for-byte as `load-image` read it —
    /// this is exactly the stream a PDF `/Filter /DCTDecode` XObject wants.
    pub bytes: Vec<u8>,
    /// Color components declared by the JPEG's own SOF marker: `1`
    /// (grayscale, maps to `/DeviceGray`) or `3` (YCbCr/RGB, maps to
    /// `/DeviceRGB`). `sniff_baseline_jpeg_dct` never returns any other
    /// value here (4-component CMYK/YCCK is rejected, not represented).
    pub components: u8,
}

impl ImageResource {
    /// Scan raw file bytes for a JPEG **SOF0** (baseline DCT) or **SOF1**
    /// (extended sequential DCT) marker — the two JPEG variants a PDF
    /// `/Filter /DCTDecode` XObject can safely wrap verbatim, matching
    /// upstream SATySFi's own JPEG special-case (`imageInfo.ml`'s bypass of
    /// full decode/re-encode for `Jpeg`). Returns `None` — meaning "fall
    /// back to the flattened RGB8 embedding" — for:
    ///
    /// - anything that isn't a JPEG at all (no `FFD8` SOI marker);
    /// - a malformed/truncated JPEG (a marker's declared segment length runs
    ///   past the end of the buffer, or entropy-coded scan data is reached
    ///   before any SOF marker was seen);
    /// - progressive, lossless, arithmetic-coded, or hierarchical JPEGs
    ///   (any SOF marker other than `0xC0`/`0xC1`) — a real PDF viewer's
    ///   `DCTDecode` support for these is inconsistent, so this port only
    ///   trusts the two most common, universally-supported variants;
    /// - non-8-bit sample precision;
    /// - anything other than 1 (grayscale) or 3 (YCbCr/RGB) color
    ///   components — in particular 4-component CMYK/YCCK JPEGs, whose
    ///   correct PDF embedding depends on an Adobe APP14 marker's transform
    ///   flag (some CMYK JPEGs store inverted samples) that this port does
    ///   not attempt to interpret.
    ///
    /// `bytes` is consumed (not just borrowed) so the `Some` case can hand
    /// the original file contents straight into `JpegDct` with no copy.
    pub fn sniff_baseline_jpeg_dct(bytes: Vec<u8>) -> Option<JpegDct> {
        if bytes.len() < 4 || bytes[0] != 0xFF || bytes[1] != 0xD8 {
            return None; // no SOI marker: not a JPEG.
        }
        let mut i = 2usize;
        while i < bytes.len() {
            if bytes[i] != 0xFF {
                return None; // not aligned on a marker; bail rather than guess.
            }
            // Marker codes may be preceded by any number of 0xFF fill bytes.
            let mut j = i + 1;
            while j < bytes.len() && bytes[j] == 0xFF {
                j += 1;
            }
            if j >= bytes.len() {
                return None;
            }
            let marker = bytes[j];
            i = j + 1;
            // Standalone markers carry no length-prefixed segment: SOI
            // (stray, shouldn't recur but is harmless), EOI, TEM, RSTn.
            if marker == 0xD8 || marker == 0xD9 || marker == 0x01 || (0xD0..=0xD7).contains(&marker)
            {
                continue;
            }
            if i + 2 > bytes.len() {
                return None;
            }
            let seg_len = u16::from_be_bytes([bytes[i], bytes[i + 1]]) as usize;
            if seg_len < 2 || i + seg_len > bytes.len() {
                return None;
            }
            if marker == 0xDA {
                return None; // start-of-scan reached before any SOF: bail.
            }
            // SOF0..SOF15 (0xC0..0xCF) except 0xC4 (DHT), 0xC8 (JPG
            // extension, reserved), 0xCC (DAC) — those codes overlap the
            // SOF range but aren't frame headers.
            let is_sof = (0xC0..=0xCF).contains(&marker) && ![0xC4, 0xC8, 0xCC].contains(&marker);
            if is_sof {
                if marker != 0xC0 && marker != 0xC1 {
                    return None; // progressive/lossless/hierarchical: bail.
                }
                // Segment payload after the 2-byte length: precision(1),
                // height(2 BE), width(2 BE), num_components(1).
                let payload = &bytes[i + 2..i + seg_len];
                if payload.len() < 6 {
                    return None;
                }
                let precision = payload[0];
                let components = payload[5];
                return if precision == 8 && (components == 1 || components == 3) {
                    Some(JpegDct { bytes, components })
                } else {
                    None
                };
            }
            i += seg_len;
        }
        None
    }

    /// The image's intrinsic dimensions for aspect-ratio math
    /// (`use-image-by-width`, docs/plans/design-load-pdf-image.md §4): pixel
    /// extents for a raster resource, MediaBox point extents for an
    /// imported PDF page. Both ratios are dimensionless (px/px or pt/pt), so
    /// callers can apply the exact same `height = width * ih/iw` formula
    /// regardless of kind — only the placement CTM (rustyfi-pdf) needs to
    /// know which units these actually are.
    pub fn intrinsic_dims_pt(&self) -> (f64, f64) {
        if let Some(pdf) = &self.pdf {
            let (x0, y0, x1, y1) = pdf.media_box;
            (x1 - x0, y1 - y0)
        } else {
            (self.px_w as f64, self.px_h as f64)
        }
    }
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
        /// True when the callback ignored its placed-point argument, so its
        /// `elems` are PAGE-ABSOLUTE (e.g. a slydifi frame background /
        /// full-page decoration built with `fun _ -> …`). Such graphics must
        /// be emitted with an IDENTITY `cm` — NOT translated by the box's
        /// placed position — otherwise the whole decoration shifts off the
        /// page (the box is often placed at a negative text-origin). For an
        /// ordinary position-relative callback this is false and the writer's
        /// per-box `cm` translate applies as usual.
        origin_independent: bool,
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
        /// Which of the block's lines sits on the surrounding text baseline:
        /// `false` = the FIRST line (`embed-block-top`, `adjust_to_first_line`),
        /// `true` = the LAST line (`embed-block-bottom`, `adjust_to_last_line`).
        /// Governs both this box's height/depth split and where the writers
        /// anchor the block's inner lines (`place_embedded_block`).
        anchor_last: bool,
        /// Built by `embed-block-BREAKABLE` (upstream
        /// `HorzEmbeddedVertBreakable`) rather than `embed-block-top`/`-bottom`
        /// (`HorzEmbeddedVert`). Upstream keeps these two apart because the
        /// breakable one is not laid out as inline content at all: the line
        /// breaker flushes the current line, splices the block's own vertical
        /// boxes straight into the vertical list (`AlreadyVert`,
        /// `lineBreak.ml:809-818`), and starts a fresh line. See
        /// `break_into_lines`, which does exactly that for this flag.
        breakable: bool,
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
    /// `docs/plans/design-reflow-s4-lists.md` §4.1: an INERT reflow marker
    /// for emphasis runs (`\emph`/`\bold` in the repo-controlled stdlibs
    /// that opt in, §5) and list-bullet fencing, emitted by the
    /// `inline-mark` primitive. Zero width/height/depth, exactly like
    /// `FrameMarker` above (renders nothing — see the PDF/faithful-HTML
    /// writers' wildcard arms, and `natural_width`/`is_glue` below) — it
    /// contributes zero advance wherever it rides inside a placed line's
    /// `contents`, so PDF/faithful HTML are byte-identical whether or not a
    /// document's stdlib emits these. Read only by the reflow HTML walker
    /// (`crates/rustyfi-html/src/reflow/inline.rs`'s `emit_inline`), which
    /// uses `EmphStart`/`EmphEnd` to wrap `<em>`/`<strong>` and
    /// `BulletStart`/`BulletEnd` to suppress the drawn bullet/number glyph
    /// run (the real marker comes from the `<ul>`/`<ol>` itself).
    InlineMark(InlineMarkKind),
}

/// The marker kind a `PureHorzBox::InlineMark` carries (`docs/plans/
/// design-reflow-s4-lists.md` §4.1). `strong` on `EmphStart` is a naming
/// choice made AT THE WRAP SITE (which stdlib command calls `inline-mark`
/// with which tag) — not recovered from the box tree, honest per §5: we map
/// `\emph` -> `EmphStart { strong: false }` (`<em>`), `\bold`/`\strong` ->
/// `EmphStart { strong: true }` (`<strong>`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InlineMarkKind {
    /// Opens `<em>` (`strong = false`) or `<strong>` (`strong = true`).
    EmphStart { strong: bool },
    /// Closes the innermost open emphasis span.
    EmphEnd,
    /// Opens a fence around a drawn bullet/number glyph run (`itemize.satyh`'s
    /// `make-bullet`/`enumerate-item`'s numeral) — the reflow walker drops
    /// everything between this and the matching `BulletEnd`, since the real
    /// `<ul>`/`<ol>` marker replaces it.
    BulletStart,
    /// Closes the bullet fence.
    BulletEnd,
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
            // Zero width — see the variant's own doc comment.
            PureHorzBox::InlineMark(_) => Length::ZERO,
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
