//! Markdown output — `--format markdown`, a SUBSET of `--format html`.
//!
//! Both backends read the same input, `DocumentValue::reflow_source`: the
//! flat `Vec<VertBox>` as it stood BEFORE page breaking. There are no pages
//! in it, so nothing is cut at a page boundary and the page furniture —
//! running heads, folios — never exists at all. And both recover the same
//! document structure out of it, through the same code
//! ([`crate::recover`]): headings correlated to `extras.outline` by
//! `dest_name`, lists from the inert `VertBox::ListMark` markers, tables from
//! a `TabularBox`'s cell positions, the CJK glue rule, the line-breaker's own
//! hyphen.
//!
//! **What differs is only what is written at the end**, and the difference is
//! that Markdown says less. HTML can carry a frame's decoration, a
//! paragraph's alignment, a run's colour, a drawing, a page's geometry;
//! Markdown has words, emphasis, lists, tables, links, code and headings, and
//! nothing else. So this backend is not a lesser HTML writer — it is the same
//! recovery with a smaller vocabulary, and each thing it cannot say is
//! dropped deliberately rather than approximated:
//!
//! | dropped | why |
//! |--|--|
//! | frames, decorations, borders | Markdown has no box model; a blockquote is not a frame |
//! | alignment (`\align-center`) | no alignment syntax; nothing about the text depends on it |
//! | page breaks, running heads | already absent from the pre-page-break stream, and meaningless once reflowed |
//! | colour, font, size | no styling syntax outside emphasis and code |
//! | a paragraph's recovered indent | four leading spaces is an indented CODE BLOCK, which would be a lie |
//! | table rules | GFM has one table style |
//! | in-document anchors (`\ref` targets) | no anchor scheme; a renderer invents its own from heading text |
//!
//! And two things it says BETTER than the HTML backend, both because a fence
//! is a stronger container than a `<p>`:
//!
//! - **A code block keeps its indentation.** In HTML it is lost, because the
//!   `inline-skip` that carries it collapses like any other glue. Here it is
//!   divided back into columns by the measured fixed-pitch advance — see
//!   `para.rs`'s `Piece::Gap`.
//! - **A footnote is a real footnote.** GFM has `[^1]`, so the note goes
//!   where a reader's own renderer puts notes, rather than into an `<aside>`
//!   wedged after the paragraph because there is no page foot any more.
//!
//! The three decisions with no good answer — math, drawings, images — are
//! argued where they are implemented: `math.rs`'s module comment, and
//! `inline.rs`'s `graphic_placeholder` and `Image` arm.

mod block;
mod escape;
mod inline;
mod math;
mod para;
mod table;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::fmt::Write as _;

use rustyfi_backend::{AnnotAction, DecoId, DocExtras, ImageResource, VertBox};
use rustyfi_pdf::TtfFontStore;

use crate::HtmlError;
use para::Para;

/// Render-time state shared by the whole walk.
///
/// Interior-mutable throughout, for the same bargain the HTML backend's `Ctx`
/// makes: the emitters keep a `&Ctx`-only signature and no caller has to
/// thread a `&mut` through a recursion that already carries three other
/// buffers.
pub(crate) struct Ctx<'a> {
    fonts: Option<&'a TtfFontStore>,
    /// Which of the store's FILES are fixed-pitch, computed once — see
    /// [`crate::recover::MonoFiles`].
    mono_files: crate::recover::MonoFiles,
    /// How an equation is written — see [`crate::MathMode`]. All three modes
    /// are reachable here; this is the backend the choice was invented for.
    math: crate::MathMode,
    /// `DecoId -> action` for every `register-link-to-uri`/`-to-location`
    /// call the compile driver observed firing. The `DecoId` a `Frame` or an
    /// `InlineFrameMarker` carries is the SAME one, so this is an exact match
    /// rather than a geometry guess.
    links: HashMap<DecoId, &'a AnnotAction>,
    /// `DecoId -> destination name`, the other half of the heading
    /// correlation (`crate::recover::find_heading_level`).
    dests: HashMap<DecoId, &'a str>,
    /// `dest_name -> outline level`, from `extras.outline`.
    outline_by_dest: HashMap<String, i64>,
    images: &'a [ImageResource],
    /// Every `ImageId` mapped to the LOWEST one holding identical pixels.
    ///
    /// Content, not identity, is what has to be deduplicated: each
    /// `include-image` call mints a fresh `ImageResource` even for a file
    /// already loaded, so `figbox`'s manual holds seventeen distinct
    /// `ImageId`s covering two actual pictures — and each one would otherwise
    /// get its own copy of the same base64 payload at the foot of the file.
    image_canon: HashMap<usize, usize>,
    /// Canonical `ImageId`s in first-use order; the index is the reference
    /// label's number. See [`Ctx::image_ref`].
    image_refs: RefCell<Vec<usize>>,
    /// The natural width (pt) of glue seen since the last thing written,
    /// awaiting the character that follows it before
    /// `crate::recover::wants_space` can judge whether it is a space, a kern,
    /// or a bare break opportunity. Consecutive glues merge by taking the
    /// widest — two adjacent glues are still at most one space.
    pending_glue: Cell<Option<f64>>,
    /// The last character actually written, the `prev` half of the glue
    /// decision.
    last_char: Cell<Option<char>>,
    /// Whether the last text run was set in a fixed-pitch face.
    mono_run: Cell<bool>,
    /// The width of one fixed-pitch character in the paragraph being built,
    /// measured from a run rather than assumed — the divisor that turns a
    /// code block's `inline-skip` indentation back into a count of spaces.
    /// Reset per paragraph, since a document may set code at more than one
    /// size.
    mono_advance: Cell<Option<f64>>,
    /// The line just built ends with a hyphen the LINE BREAKER inserted, so
    /// rejoining must drop it. Distinguishing this from an authored hyphen is
    /// the whole reason `InlineMarkKind::BreakHyphen` exists.
    break_hyphen: Cell<bool>,
    /// Open emphasis delimiters. `EmphEnd` carries no payload, so which of
    /// `*`/`**` closes it has to be remembered.
    emph_stack: RefCell<Vec<&'static str>>,
    /// Non-zero while inside a `BulletStart`/`BulletEnd` fence, during which
    /// every other box renders nothing — the drawn bullet is replaced by the
    /// list's own marker. A counter rather than a flag so a stray unmatched
    /// marker cannot leave it wrong.
    bullet_suppress: Cell<u32>,
    /// Whether each open `inline-frame-breakable` region pushed a link, so
    /// its end marker knows whether there is one to close.
    iframe_stack: RefCell<Vec<bool>>,
    /// Footnote bodies whose reference has been written, drained into the
    /// document's foot at the end.
    footnotes: RefCell<Vec<(usize, String)>>,
    footnote_seq: Cell<usize>,
    /// The next run is the reference marker the DOCUMENT typeset for a
    /// footnote, and is dropped because `[^n]` has already been written. See
    /// `inline.rs`'s `Footnote` arm.
    drop_fn_marker: Cell<bool>,
    /// Blocks a paragraph produced that cannot live inside it — a table,
    /// which is block-level in Markdown — released by the paragraph flush
    /// immediately after it.
    pending_blocks: RefCell<Vec<String>>,
}

impl Ctx<'_> {
    /// Record that a glue box of `natural_pt` natural width stands here.
    /// Nothing is written yet: whether it becomes a space depends on the
    /// character that follows.
    fn note_glue(&self, natural_pt: f64) {
        let merged = match self.pending_glue.get() {
            Some(prev) if prev >= natural_pt => prev,
            _ => natural_pt,
        };
        self.pending_glue.set(Some(merged));
    }

    /// Resolve the pending glue against the character about to be written.
    ///
    /// The space carries the monospace-ness of the run BEFORE it, which is
    /// what keeps `` `point list` `` one code span rather than two: the box
    /// stream splits it at the glue, and a space tagged as prose between two
    /// fixed-pitch chunks would close the first span and open a second. The
    /// tag is read before `emit_run` overwrites `mono_run` with the run it is
    /// about to write, so it is genuinely the left neighbour's.
    fn resolve_glue(&self, para: &mut Para, next: Option<char>) {
        if let Some(width) = self.pending_glue.take() {
            if crate::recover::wants_space(self.last_char.get(), next, width) {
                para.push_text(" ", self.mono_run.get());
            }
        }
    }

    /// Drop any pending glue and forget the last character — used at a hard
    /// boundary (a new paragraph, a table cell, a footnote body) where a
    /// space carried over from the previous context would be wrong.
    fn reset_flow(&self) {
        self.pending_glue.set(None);
        self.last_char.set(None);
    }

    /// Settle any pending glue against "no following character" before
    /// writing something that is not text at all — an image, a footnote
    /// reference, a table — and forget the last character, since the next
    /// glue has nothing textual on its left to be judged against.
    fn open_opaque(&self, para: &mut Para) {
        self.resolve_glue(para, None);
        self.last_char.set(None);
        self.mono_run.set(false);
    }

    /// The reference LABEL and display number for an image, registering it
    /// for the definition list at the foot of the document if this is its
    /// first placement.
    ///
    /// A reference-style image, `![image 1][md-img-1]`, rather than an inline
    /// `![](data:image/jpeg;base64,…)`. The bytes are identical either way;
    /// what differs is that the prose stays readable. A single figure is
    /// commonly a hundred kilobytes of base64, and the whole claim of this
    /// format is that the raw file is legible — a paragraph interrupted by
    /// two screens of base64 is not.
    fn image_ref(&self, id: usize) -> (String, usize) {
        let canon = self.image_canon.get(&id).copied().unwrap_or(id);
        let mut refs = self.image_refs.borrow_mut();
        let idx = match refs.iter().position(|c| *c == canon) {
            Some(i) => i,
            None => {
                refs.push(canon);
                refs.len() - 1
            }
        };
        (format!("md-img-{}", idx + 1), idx + 1)
    }
}

/// Group `images` by CONTENT, producing [`Ctx::image_canon`]. Two resources
/// are the same picture when their pixel dimensions and their bytes agree —
/// the original JPEG stream when there is one (which is also what
/// `image::data_uri` will emit), the decoded samples otherwise.
fn canonical_images(images: &[ImageResource]) -> HashMap<usize, usize> {
    let mut first_by_content: HashMap<(&[u8], u32, u32), usize> = HashMap::new();
    let mut canon_of = HashMap::new();
    for (idx, res) in images.iter().enumerate() {
        let bytes: &[u8] = match &res.jpeg_dct {
            Some(j) => &j.bytes,
            None => &res.samples,
        };
        // An imported PDF page has neither, so every one of them would hash
        // alike; they render as a labelled placeholder anyway.
        if bytes.is_empty() {
            canon_of.insert(idx, idx);
            continue;
        }
        let canon = *first_by_content
            .entry((bytes, res.px_w, res.px_h))
            .or_insert(idx);
        canon_of.insert(idx, canon);
    }
    canon_of
}

/// Serialize the pre-page-break `Vec<VertBox>` (`source` —
/// `DocumentValue::reflow_source`, `None` when unavailable, e.g. a
/// hand-built `DocumentValue` in a test) to one self-contained Markdown
/// document, with no font store: the base-14 twin of
/// [`render_markdown_ttf_with`], exactly mirroring
/// `rustyfi_pdf::render_pdf_with`'s relationship to
/// `rustyfi_pdf::render_pdf_ttf_with`.
///
/// Without a font store no face can be asked whether it is fixed-pitch
/// (`crate::recover::is_monospace` has no file to read a family name from),
/// so a code block comes out as ordinary prose. That is the same degradation
/// the HTML backend takes, for the same reason, and the CLI always has a
/// store.
/// `math` chooses how equations are written ([`crate::MathMode`]) and is a
/// required argument rather than an option with a default, unlike the HTML
/// backend's. That is deliberate: this backend's default CHANGED — it drew
/// Unicode characters and now draws outlines — so a caller that did not
/// restate its choice would have silently got different output, and there is
/// no reading of "unspecified" that is right for every caller.
#[allow(clippy::too_many_arguments)]
pub fn render_markdown(
    source: Option<&[VertBox]>,
    images: &[ImageResource],
    extras: &DocExtras,
    links: &[(DecoId, AnnotAction)],
    dests: &[(DecoId, String)],
    math: crate::MathMode,
) -> Result<String, HtmlError> {
    render_markdown_impl(source, images, extras, links, dests, None, math)
}

/// [`render_markdown`] under a real [`TtfFontStore`] — the full-fidelity
/// entry point the CLI uses. The store is read for two things: whether a
/// run's face is fixed-pitch, which is what tells a code block from a wrapped
/// paragraph, and — under [`crate::MathMode::SvgOutline`] — the glyph outlines
/// an equation is drawn from. No font FILE is embedded; there is nowhere in
/// Markdown to embed one, which is also why the outlines are drawn as paths
/// rather than named as a face.
#[allow(clippy::too_many_arguments)]
pub fn render_markdown_ttf_with(
    source: Option<&[VertBox]>,
    store: &TtfFontStore,
    images: &[ImageResource],
    extras: &DocExtras,
    links: &[(DecoId, AnnotAction)],
    dests: &[(DecoId, String)],
    math: crate::MathMode,
) -> Result<String, HtmlError> {
    render_markdown_impl(source, images, extras, links, dests, Some(store), math)
}

#[allow(clippy::too_many_arguments)]
fn render_markdown_impl(
    source: Option<&[VertBox]>,
    images: &[ImageResource],
    extras: &DocExtras,
    links: &[(DecoId, AnnotAction)],
    dests: &[(DecoId, String)],
    font_store: Option<&TtfFontStore>,
    math: crate::MathMode,
) -> Result<String, HtmlError> {
    let ctx = Ctx {
        fonts: font_store,
        mono_files: crate::recover::MonoFiles::new(font_store),
        math,
        links: links.iter().map(|(id, action)| (*id, action)).collect(),
        dests: dests
            .iter()
            .map(|(id, name)| (*id, name.as_str()))
            .collect(),
        outline_by_dest: crate::recover::outline_levels(&extras.outline),
        images,
        image_canon: canonical_images(images),
        image_refs: RefCell::new(Vec::new()),
        pending_glue: Cell::new(None),
        last_char: Cell::new(None),
        mono_run: Cell::new(false),
        mono_advance: Cell::new(None),
        break_hyphen: Cell::new(false),
        emph_stack: RefCell::new(Vec::new()),
        bullet_suppress: Cell::new(0),
        iframe_stack: RefCell::new(Vec::new()),
        footnotes: RefCell::new(Vec::new()),
        footnote_seq: Cell::new(0),
        drop_fn_marker: Cell::new(false),
        pending_blocks: RefCell::new(Vec::new()),
    };

    let Some(vboxes) = source else {
        // No captured pre-page-break flow (e.g. a hand-built `DocumentValue`
        // in a unit test that never populated `reflow_source`) — an empty
        // document rather than a panic.
        return Ok(String::new());
    };
    let mut out = block::render_block(vboxes, &ctx);

    // No generated title, table of contents or metadata header. A document
    // that wants a contents page TYPESETS one, and it is already in the flow
    // above; emitting a second, differently-shaped copy duplicated it in
    // every real manual the HTML backend was tried on.
    append_footnotes(&mut out, &ctx);
    append_image_defs(&mut out, &ctx);
    Ok(out)
}

/// The GFM footnote definitions, at the foot of the document where a reader's
/// renderer expects to find them.
///
/// A note's body may be several blocks; continuation lines are indented by
/// four spaces, which is what keeps them part of the definition rather than
/// starting a new top-level block.
fn append_footnotes(out: &mut String, ctx: &Ctx) {
    let notes = ctx.footnotes.borrow();
    if notes.is_empty() {
        return;
    }
    ensure_blank_line(out);
    for (n, body) in notes.iter() {
        let body = strip_typeset_number(body.trim(), *n);
        let mut lines = body.lines();
        let first = lines.next().unwrap_or("");
        let _ = writeln!(out, "[^{n}]: {first}");
        for line in lines {
            if line.is_empty() {
                out.push('\n');
            } else {
                let _ = writeln!(out, "    {line}");
            }
        }
    }
}

/// The reference-style image definitions, likewise at the foot — see
/// [`Ctx::image_ref`] for why the payloads are not inline.
///
/// A `data:` URI, not a sidecar file. The output of a compile is ONE path
/// (`-o out.md`), and inventing a directory beside it would break the moment
/// the file is moved, mailed or pasted — which is most of what a `.md` is
/// for. Data URIs render in VS Code, Typora, pandoc and any local previewer;
/// GitHub's image proxy refuses them, so there the image degrades to its alt
/// text rather than to a broken relative path. The bytes themselves are
/// `crate::image::data_uri`'s, the same ones the HTML backend embeds: a
/// baseline JPEG passes through unchanged, anything else becomes an
/// uncompressed BMP.
fn append_image_defs(out: &mut String, ctx: &Ctx) {
    let refs = ctx.image_refs.borrow();
    if refs.is_empty() {
        return;
    }
    ensure_blank_line(out);
    for (idx, canon) in refs.iter().enumerate() {
        let Some(res) = ctx.images.get(*canon) else {
            continue;
        };
        let _ = writeln!(
            out,
            "[md-img-{}]: {}",
            idx + 1,
            crate::image::data_uri(res)
        );
    }
}

/// Drop the note's own number from the start of its body, when the document
/// typeset one there.
///
/// `stdjabook`'s footnote body is `bbf num`, which opens with the numeral —
/// so a GFM definition, which its renderer numbers for itself, would read
/// "1. 1 These commands…". Only an EXACT match for the number this backend
/// assigned is removed, so a note whose body genuinely begins with a figure
/// ("1 in 5 documents…") keeps it unless that figure happens to be the note's
/// own number, and a class that writes no number in the body loses nothing.
fn strip_typeset_number(body: &str, n: usize) -> &str {
    let digits = n.to_string();
    let Some(rest) = body.strip_prefix(&digits) else {
        return body;
    };
    match rest.strip_prefix(' ') {
        Some(rest) => rest,
        None => body,
    }
}

fn ensure_blank_line(out: &mut String) {
    if out.is_empty() {
        return;
    }
    if !out.ends_with('\n') {
        out.push('\n');
    }
    if !out.ends_with("\n\n") {
        out.push('\n');
    }
}
