//! LaTeX output — `--format latex`, a complete `.tex` document.
//!
//! The third serialization of the same recovered document. It reads what the
//! other two read, `DocumentValue::reflow_source` — the flat `Vec<VertBox>`
//! as it stood BEFORE page breaking — and recovers the same structure out of
//! it through the same code ([`rustyfi_html::recover`]): headings correlated to
//! `extras.outline` by `dest_name`, lists from the inert `VertBox::ListMark`
//! markers, tables and their rules from a `TabularBox`, the CJK glue rule,
//! the line breaker's own hyphen.
//!
//! **What is different is that the target is another typesetter.** HTML and
//! Markdown hand the document to a renderer that will lay it out; so does
//! this, but the renderer is TeX, which can say most of what SATySFi can. So
//! where the other two backends drop or approximate, this one usually does
//! not:
//!
//! | | Markdown | HTML | here |
//! |--|--|--|--|
//! | math | characters in reading order | an `<svg>` of outlines | `$\frac{a+b}{c}$` |
//! | drawings | an `<svg>` a sanitizing renderer strips | an `<svg>` | a `tikzpicture` of the same paths |
//! | a `\ref` | plain text | `<a href="#…">` | `\hyperlink` |
//! | a `draw-text` label | flowed after its drawing | absolutely positioned | a `\node` at its own point |
//! | table rules | none (GFM has one style) | per-cell CSS borders | `|` and `\hline` where drawn |
//! | a code block | a fence | a `<pre>` | `fancyvrb`'s `Verbatim` |
//! | CMYK colour | — | converted, lossily | `xcolor`'s own `cmyk` |
//!
//! and where it does, it says so ([`crate::tikz::placeholder`] for
//! raster images, which cannot travel inside a single output file).
//!
//! ## The engine
//!
//! The preamble names the engine it needs, in a comment at the top of the
//! file and again as a `\Require…TeX` that fails cleanly rather than
//! producing garbage:
//!
//! - **A document with no CJK compiles under any of pdfLaTeX, XeLaTeX and
//!   LuaLaTeX.** The math is written as `amsmath`/`amssymb` command names
//!   rather than as Unicode ([`rustyfi_html::latex`]), and the only
//!   engine-conditional line is `fontenc`/`inputenc`, guarded by `iftex`.
//! - **A document with any CJK requires LuaLaTeX**, and gets
//!   `luatexja-fontspec` with the Harano Aji faces. pdfLaTeX cannot set CJK
//!   at all — there is no encoding for it and no font it can load — so this
//!   is not a preference. XeLaTeX could, through `xeCJK`, but only one of the
//!   two can be named in a generated file and `luatexja` is the one that also
//!   gets the JLreq inter-script spacing right, which is most of what a
//!   Japanese document's typography IS.
//!
//! ## The preamble declares only what the body uses
//!
//! `tikz`, `hyperref` and `fancyvrb` are each emitted only if the walk
//! actually reached a drawing, a link or a code block ([`Ctx`]'s `uses_*`
//! flags, set at the point of emission rather than predicted). That keeps a
//! plain document's preamble to five lines, and it means a reader can tell
//! from the top of the file what is in the rest of it.
//!
//! ## Known wrong, as opposed to known absent
//!
//! Everything in the table above is a deliberate simplification. These are
//! not: they are cases where the output is silently WRONG or does not
//! compile, found by an adversarial sweep and written down because a reader
//! comparing the `.tex` to the PDF deserves to find them here rather than
//! discover them.
//!
//! - **A footnote is numbered twice** under `stdjabook`/`stdjareport`. The
//!   note's BODY already begins with the numeral the document typeset
//!   (`stdjabook.satyh:628`) and `\footnote` adds its own. The reference
//!   MARKER is already handled — `Ctx::drop_fn_marker` drops it, keyed on
//!   `set-manual-rising` — but the body's leading numeral has no such tell,
//!   and stripping a leading digit from arbitrary note text would take more
//!   than it gave.
//! - **A footnote inside a table cell loses its text.** `\footnote` inside
//!   `tabular` needs `\footnotemark`/`\footnotetext` split across the table
//!   boundary, which this walk has no place to put.
//! - **A table nested inside a table cell is hoisted out and emitted BEFORE
//!   its parent**, inverting the reading order. Same queue
//!   (`Ctx::pending_blocks`) that correctly lifts a top-level table out of a
//!   paragraph, one level too deep.
//! - **A list nested five deep** is `Too deeply nested`. Four is LaTeX's
//!   limit for `itemize`/`enumerate` and is not raised here.
//! - **A coordinate or paper size past `\maxdimen`** (about 5.76 m) fails.
//!   [`tikz::fit_scale`] does not help: TikZ evaluates a coordinate before
//!   applying `scale`.
//! - **Above Latin-1, a Unicode engine may still lack the glyph.** The
//!   preamble says which engine is required and says this out loud, because
//!   TeX reports it as one `Missing character` line and then exits 0. See
//!   [`needs_unicode_engine`].
//!
//! ## Location: its own crate, depending on `rustyfi-html`
//!
//! This backend is a crate of its own, a peer of `rustyfi-pdf` and
//! `rustyfi-html`, and it **depends on `rustyfi-html`** for the structure
//! recovery it shares with the HTML and Markdown backends
//! ([`rustyfi_html::recover`]) and for the math-run writer `--katex` also
//! uses ([`rustyfi_html::latex`]). **This crate copies nothing**: every rule
//! it needs, it calls. There is one `wants_space` in the tree, one
//! `line_join`, one `table_rows`, one `Borders`, one `collapse_whitespace`
//! and one `math_latex`. A second copy of the CJK glue rule would be the one
//! outcome worth refusing outright — the two would diverge, and the symptom
//! (a space between every pair of Japanese characters, in one format only) is
//! not one anybody would look for here.
//!
//! **Not every rule in [`rustyfi_html::recover`] is singular yet, though, and
//! that is not this crate's doing.** Eight of the box-stream helpers there —
//! `is_pure_text`, `glue_width`, the `HSKIP_MIN_PT`/`GRAPHIC_MIN_PT`
//! thresholds and five more — still have a second or third definition inside
//! `rustyfi-html`'s own backends, because the commit that hoisted them wrote
//! a new copy and left the old ones. This crate calls the hoisted one; the
//! duplicates are `rustyfi-html`-internal, are listed row by row under
//! "Still forked" in [`rustyfi_html::recover`]'s module doc, and are waiting
//! on the files they live in to be quiet.
//!
//! **The dependency points the wrong way if you read the crate names as a
//! layering, and it is deliberate.** The alternative — lifting
//! `recover`/`latex`/`mathrec` into a fourth crate that `rustyfi-html` and
//! this one both depend on — is the better end state and a bigger, riskier
//! change than this one: those three modules are named from roughly a hundred
//! sites across `reflow/`, `markdown/` and `mathsvg`, so the lift is a rename
//! sweep through every file the other two backends live in, for no
//! behavioural gain and a guaranteed conflict with anything in flight there.
//! `rustyfi-html`'s own module doc already records that its name outgrew it.
//! Splitting the recovery out belongs in a commit that does only that.
//!
//! What this crate takes from `rustyfi-html` is exactly two things, and
//! neither of them is HTML: [`rustyfi_html::recover`] and
//! [`rustyfi_html::latex::math_latex`]. No escaper, no `<svg>` writer, no CSS
//! and no colour conversion crosses — `tikz.rs` writes `xcolor`'s own `cmyk`,
//! so it does not even want the SVG writer's lossy one.

// The public surface here is two functions; everything that implements them —
// `Ctx`, `tikz`, `escape`, `para` — is private, and the doc comments name
// those constantly because they are written for the reader of the SOURCE.
// They read the same either way; rustdoc's objection to them does not.
#![allow(rustdoc::private_intra_doc_links)]

mod block;
mod escape;
mod inline;
mod para;
mod table;
mod tikz;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::fmt::Write as _;

use rustyfi_backend::{
    AnnotAction, DecoId, DocExtras, GraphicsElem, ImageResource, PageGeometry, VertBox,
};
use rustyfi_html::recover;
use rustyfi_pdf::TtfFontStore;

use para::Para;

/// Rendering is in practice infallible — every run of the document's own
/// text goes out through `escape::text`, and the image, link and outline
/// handling reads tables the compile step already validated. The `Result`
/// return shape is kept anyway so the entry points are argument-for-argument
/// with `rustyfi_pdf::render_pdf_with` and `rustyfi_html::render_html_reflow`
/// (module signature, not module fallibility), and so a step that has to
/// touch the filesystem — the sidecar files an `\includegraphics` would need,
/// see [`tikz::placeholder`] for why there are none yet — could surface a
/// real error without a breaking signature change.
///
/// A twin of `rustyfi_html::HtmlError` rather than a reuse of it. The two
/// crates are peers; a `rustyfi-latex` entry point handing back an
/// `HtmlError` would be the crate split's one visible lie.
#[derive(Debug, thiserror::Error)]
pub enum LatexError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Render-time state shared by the whole walk.
///
/// Interior-mutable throughout, for the same bargain the other two backends'
/// `Ctx` makes: the emitters keep a `&Ctx`-only signature and no caller has
/// to thread a `&mut` through a recursion that already carries three other
/// buffers.
pub(crate) struct Ctx<'a> {
    fonts: Option<&'a TtfFontStore>,
    /// Which of the store's FILES are fixed-pitch, computed once — see
    /// [`rustyfi_html::recover::MonoFiles`], where the 145ms this saves is measured.
    mono_files: recover::MonoFiles,
    /// `DecoId -> action` for every `register-link-to-uri`/`-to-location`
    /// call the compile driver observed firing. The `DecoId` a `Frame` or an
    /// `InlineFrameMarker` carries is the SAME one, so this is an exact match
    /// rather than a geometry guess.
    links: HashMap<DecoId, &'a AnnotAction>,
    /// `DecoId -> destination name`, the other half of the heading
    /// correlation (`rustyfi_html::recover::find_heading_level`) and the source of
    /// every `\hypertarget`.
    dests: HashMap<DecoId, &'a str>,
    /// `dest_name -> outline level`, from `extras.outline`.
    outline_by_dest: HashMap<String, i64>,
    images: &'a [ImageResource],
    /// `ImageId -> the placement number` in its label. First use wins, so a
    /// figure repeated on two pages carries two numbers — which is right for
    /// a PLACEHOLDER, whose job is to say "something was here", rather than
    /// for an embedded picture where deduplicating the bytes would matter.
    image_numbers: RefCell<HashMap<usize, usize>>,
    /// Rules collected from a table's invisible rules-only twin, awaiting the
    /// real one — see [`rustyfi_html::recover::overlaid_table_rules`].
    tabular_rules: RefCell<Vec<(f64, f64, Vec<GraphicsElem>)>>,
    /// The natural width (pt) of glue seen since the last thing written,
    /// awaiting the character that follows it before
    /// `rustyfi_html::recover::wants_space` can judge whether it is a space, a kern,
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
    /// code block's `inline-skip` indentation back into a column count.
    mono_advance: Cell<Option<f64>>,
    /// The line just built ends with a hyphen the LINE BREAKER inserted, so
    /// rejoining must drop it.
    break_hyphen: Cell<bool>,
    /// Non-zero while inside a `BulletStart`/`BulletEnd` fence, during which
    /// every other box renders nothing — the drawn bullet is replaced by the
    /// list's own `\item`. A counter rather than a flag so a stray unmatched
    /// marker cannot leave it wrong.
    bullet_suppress: Cell<u32>,
    /// Whether each open `inline-frame-breakable` region pushed a link, so
    /// its end marker knows whether there is one to close.
    iframe_stack: RefCell<Vec<bool>>,
    /// Blocks a paragraph produced that cannot live inside it — a table —
    /// released by the paragraph flush immediately after it.
    pending_blocks: RefCell<Vec<String>>,
    /// The next run is the reference marker the DOCUMENT typeset for a
    /// footnote, and is dropped because `\footnote` has already written one.
    drop_fn_marker: Cell<bool>,
    /// The text area the generated `geometry` declares, in points — what a
    /// drawing has to fit inside. See `inline.rs`'s `emit_drawing`, where a
    /// figure too big for it is scaled rather than left to hang the page
    /// builder.
    text_area: (f64, f64),
    /// Set while rendering a nested block somewhere no LaTeX ENVIRONMENT may
    /// go — inside a `tabular` cell that also holds inline content, where a
    /// `Verbatim` or an `itemize` is `Not allowed in LR mode` and stops the
    /// compile. Code degrades to `\texttt` and a list to its bare items; the
    /// alternative, dropping the content, would lose a whole table column.
    inline_only: Cell<bool>,
    /// Which packages the body turned out to need — see this module's doc
    /// comment. Set where the construct is emitted, never predicted.
    uses_cjk: Cell<bool>,
    /// The finished body holds a character outside Latin-1 that is not CJK —
    /// see [`needs_unicode_engine`]. Set from the body in one pass, not
    /// during the walk.
    uses_wide: Cell<bool>,
    uses_tikz: Cell<bool>,
    uses_hyperref: Cell<bool>,
    uses_verbatim: Cell<bool>,
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
    /// what keeps `\texttt{point list}` one box rather than two: the box
    /// stream splits it at the glue, and a space tagged as prose between two
    /// fixed-pitch chunks would close the first and open a second.
    fn resolve_glue(&self, para: &mut Para, next: Option<char>) {
        if let Some(width) = self.pending_glue.take() {
            if recover::wants_space(self.last_char.get(), next, width) {
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

    /// Run `f` with the inline-flow state saved and restored around it.
    ///
    /// A nested render — a footnote body, a `\node` label — must not let its
    /// own last character decide the spacing of the word after the marker
    /// that carried it, and must not inherit the glue pending outside. Both
    /// call sites saved the same triple by hand before; the risk of that is
    /// that the triple grows a fourth member and only one of them gets it.
    fn isolated<T>(&self, f: impl FnOnce() -> T) -> T {
        let saved = (
            self.pending_glue.take(),
            self.last_char.take(),
            self.mono_run.get(),
        );
        let out = f();
        self.pending_glue.set(saved.0);
        self.last_char.set(saved.1);
        self.mono_run.set(saved.2);
        out
    }

    /// Settle the inline flow ACROSS something that is not text — a formula,
    /// a drawing, an image, a table, a footnote — carrying it from the
    /// reading-order text that thing stands for.
    ///
    /// **The obvious version of this clears `last_char`, and doing so drops
    /// the word space after every one of them.**
    /// [`rustyfi_html::recover::wants_space`] opens with
    /// `let Some(p) = prev else { return false }`, so a `None` predecessor
    /// discards the glue that follows and the next word runs straight into
    /// the box: `ALPHA $x$ BRAVO` came out as `ALPHA $x$BRAVO`, on 22 of the
    /// 26 formulas in `latexcmds`' manual, and identically for every drawing,
    /// `\node` label, image placeholder and table.
    ///
    /// Two things hid it. The space BEFORE is fine — `resolve_glue` is called
    /// with the box's own first character and answers correctly — so half the
    /// spacing looked right. And the HTML and Markdown backends have the same
    /// hazard and escape it, because their markup contributes a newline where
    /// the space was; LaTeX emits no whitespace token at all, so nothing
    /// covers for it. `markdown/inline.rs`'s `math_flow` is where this was
    /// diagnosed the first time, for equations only. This is that fix applied
    /// to every opaque construct, in the backend that cannot survive it.
    ///
    /// `plain` is whatever the call site passes to `Para::push_markup` as its
    /// verbatim side, so there is one notion per site of "what this box says
    /// in text". Where a site has none — a table, a footnote — the flow still
    /// has to carry, so the box counts as U+FFFC OBJECT REPLACEMENT
    /// CHARACTER: present, and not CJK. That is exactly the contract
    /// [`rustyfi_html::recover::wants_space`]'s own doc comment states for an
    /// opaque box ("a formula or figure set into Japanese prose takes the
    /// same space its inter-script glue was asking for") — the trailing half
    /// of it had simply never been implemented here.
    ///
    /// `mono_run` is cleared because none of these is fixed-pitch, whatever
    /// the run before it was.
    fn flow_across(&self, para: &mut Para, plain: &str) {
        self.resolve_glue(para, plain.chars().next());
        self.last_char
            .set(Some(plain.chars().next_back().unwrap_or('\u{fffc}')));
        self.mono_run.set(false);
    }

    /// This image placement's number, assigned on first sight.
    fn image_number(&self, id: usize) -> usize {
        let mut numbers = self.image_numbers.borrow_mut();
        let next = numbers.len() + 1;
        *numbers.entry(id).or_insert(next)
    }

    /// Record a text-only graphics overlay's rules-only tables, returning the
    /// stack depth to truncate back to once the overlay is done — so a
    /// pairing can never reach an unrelated table later in the document.
    fn push_overlaid_rules(&self, elems: &[GraphicsElem]) -> usize {
        let base = self.tabular_rules.borrow().len();
        self.tabular_rules
            .borrow_mut()
            .extend(recover::overlaid_table_rules(elems));
        base
    }

    fn pop_overlaid_rules(&self, depth: usize) {
        self.tabular_rules.borrow_mut().truncate(depth);
    }

    fn mark_cjk(&self) {
        self.uses_cjk.set(true);
    }
    fn mark_tikz(&self) {
        self.uses_tikz.set(true);
    }
    fn mark_hyperref(&self) {
        self.uses_hyperref.set(true);
    }
    fn mark_verbatim(&self) {
        self.uses_verbatim.set(true);
    }
}

/// Serialize the pre-page-break `Vec<VertBox>` (`source` —
/// `DocumentValue::reflow_source`, `None` when unavailable, e.g. a
/// hand-built `DocumentValue` in a test) to one self-contained `.tex`
/// document, with no font store: the base-14 twin of
/// [`render_latex_ttf_with`].
///
/// Without a font store no face can be asked whether it is fixed-pitch
/// (`rustyfi_html::recover::is_monospace` has no file to read a family name from),
/// so a code block comes out as ordinary prose. That is the same degradation
/// the other two backends take, for the same reason, and the CLI always has
/// a store.
pub fn render_latex(
    source: Option<&[VertBox]>,
    geometry: &PageGeometry,
    images: &[ImageResource],
    extras: &DocExtras,
    links: &[(DecoId, AnnotAction)],
    dests: &[(DecoId, String)],
) -> Result<String, LatexError> {
    render_latex_impl(source, geometry, images, extras, links, dests, None)
}

/// [`render_latex`] under a real [`TtfFontStore`] — the full-fidelity entry
/// point the CLI uses. The store is read for exactly one thing: whether a
/// run's face is fixed-pitch, which is what tells a code block from a wrapped
/// paragraph. No face is EMBEDDED; a `.tex` names its fonts and lets the
/// engine find them.
pub fn render_latex_ttf_with(
    source: Option<&[VertBox]>,
    geometry: &PageGeometry,
    store: &TtfFontStore,
    images: &[ImageResource],
    extras: &DocExtras,
    links: &[(DecoId, AnnotAction)],
    dests: &[(DecoId, String)],
) -> Result<String, LatexError> {
    render_latex_impl(source, geometry, images, extras, links, dests, Some(store))
}

#[allow(clippy::too_many_arguments)]
fn render_latex_impl(
    source: Option<&[VertBox]>,
    geometry: &PageGeometry,
    images: &[ImageResource],
    extras: &DocExtras,
    links: &[(DecoId, AnnotAction)],
    dests: &[(DecoId, String)],
    font_store: Option<&TtfFontStore>,
) -> Result<String, LatexError> {
    let (paper_w, paper_h) = (
        geometry.paper_width.0.max(1.0),
        geometry.paper_height.0.max(1.0),
    );
    let area = text_area(source, paper_w, paper_h);
    let ctx = Ctx {
        fonts: font_store,
        mono_files: recover::MonoFiles::new(font_store),
        links: links.iter().map(|(id, action)| (*id, action)).collect(),
        dests: dests
            .iter()
            .map(|(id, name)| (*id, name.as_str()))
            .collect(),
        outline_by_dest: recover::outline_levels(&extras.outline),
        images,
        image_numbers: RefCell::new(HashMap::new()),
        tabular_rules: RefCell::new(Vec::new()),
        pending_glue: Cell::new(None),
        last_char: Cell::new(None),
        mono_run: Cell::new(false),
        mono_advance: Cell::new(None),
        break_hyphen: Cell::new(false),
        bullet_suppress: Cell::new(0),
        iframe_stack: RefCell::new(Vec::new()),
        pending_blocks: RefCell::new(Vec::new()),
        drop_fn_marker: Cell::new(false),
        text_area: area,
        inline_only: Cell::new(false),
        uses_cjk: Cell::new(false),
        uses_wide: Cell::new(false),
        uses_tikz: Cell::new(false),
        uses_hyperref: Cell::new(false),
        uses_verbatim: Cell::new(false),
    };

    // The BODY is built first and the preamble assembled from what it turned
    // out to need. That ordering is the whole reason the preamble can be
    // honest: predicting the packages from the box stream would mean a second
    // walk that has to agree with the first, and the two would drift.
    let body = match source {
        // No captured pre-page-break flow (e.g. a hand-built `DocumentValue`
        // in a unit test that never populated `reflow_source`) — an empty
        // document rather than a panic. Still a COMPILABLE empty document:
        // the point of this format is that the file works.
        None => String::new(),
        Some(vboxes) => block::render_block(vboxes, &ctx),
    };

    // Which ENGINE the file needs is a question about the finished body, so
    // it is asked of the finished body — once, here, rather than at each of
    // the several places a character can be written. `uses_cjk` is set during
    // the walk because it also selects PACKAGES; this only selects a claim
    // and a guard.
    ctx.uses_wide.set(body.chars().any(needs_unicode_engine));

    let mut out = preamble(&ctx, paper_w, paper_h, area, extras);
    out.push_str("\\begin{document}\n\n");
    out.push_str(&body);
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str("\n\\end{document}\n");
    Ok(out)
}

/// Does this character require a Unicode engine (XeTeX or LuaTeX) to be set
/// at all?
///
/// **This is deliberately NOT
/// [`rustyfi_html::recover::is_cjk`], and the difference is what the engine
/// claim got wrong.** `is_cjk` answers a SPACING question — "is this set
/// solid, with no inter-character glue" — and its doc comment says so; it
/// knows Han, kana and Hangul because those are the scripts `wants_space`
/// has to suppress a space between. Using it to choose an engine silently
/// promised pdfLaTeX, XeLaTeX and LuaLaTeX for every document whose only
/// non-Latin content was Greek, Cyrillic, Hebrew, Arabic, an emoji or a bare
/// `≤`. What that actually produced was a hard error under pdfLaTeX and, far
/// worse, a clean exit 0 under the other two with dozens of `Missing
/// character` lines in the log and the glyphs simply absent from the page —
/// a loss this backend introduced, since the port's own PDF sets them
/// correctly.
///
/// The line is drawn at U+00FF rather than at a font's real coverage because
/// that is the boundary of what pdfTeX's 8-bit `T1`/`inputenc` route can
/// address at all. Above it, only a Unicode engine has a chance; whether it
/// has the GLYPH is a font question this backend cannot answer and says so
/// in the preamble instead.
fn needs_unicode_engine(c: char) -> bool {
    c as u32 > 0xFF
}

/// Everything before `\begin{document}`.
fn preamble(
    ctx: &Ctx,
    paper_w: f64,
    paper_h: f64,
    area: (f64, f64),
    extras: &DocExtras,
) -> String {
    let cjk = ctx.uses_cjk.get();
    let wide = !cjk && ctx.uses_wide.get();
    let mut out = String::new();
    out.push_str("% Generated by rustyfi --format latex.\n%\n");
    if cjk {
        out.push_str(
            "% ENGINE: lualatex.  This document contains CJK text, which is set\n\
             % through luatexja; pdflatex cannot set CJK at all, and the xelatex\n\
             % route would need a different package (xeCJK) than the one named\n\
             % below.  The \\RequireLuaTeX under \\usepackage{iftex} makes a wrong\n\
             % engine fail immediately rather than silently dropping the glyphs.\n",
        );
    } else if wide {
        out.push_str(
            "% ENGINE: xelatex or lualatex, NOT pdflatex.  This document has no CJK\n\
             % in it, but it does contain characters outside Latin-1 -- Greek,\n\
             % Cyrillic, an arrow, a mathematical relation set as prose -- and\n\
             % pdflatex's 8-bit fonts cannot address them at all.\n\
             %\n\
             % A KNOWN LIMITATION, stated because the alternative is finding it in\n\
             % the output: even under a Unicode engine the DEFAULT font may not\n\
             % have every one of those glyphs, and TeX reports that as a\n\
             % `Missing character' line in the log and then exits 0.  If a\n\
             % character is absent from the PDF, that is where it went; give the\n\
             % preamble a `fontspec' main font that covers the script.  This\n\
             % backend does not choose one, because choosing wrong is worse than\n\
             % saying so.\n",
        );
    } else {
        out.push_str(
            "% ENGINE: any of pdflatex, xelatex or lualatex.  This document has no\n\
             % CJK in it, nothing outside Latin-1, and the mathematics is written\n\
             % with amsmath/amssymb command names rather than Unicode characters,\n\
             % so nothing here is engine-specific except the fontenc guard below.\n",
        );
    }
    out.push_str(
        "%\n% The layout is NOT the source document's. Lines, pages and hyphenation\n\
         % are all LaTeX's own, made afresh at the measure declared below — this\n\
         % is a document, not a picture of one.\n",
    );
    out.push_str("\\documentclass{article}\n");
    out.push_str("\\usepackage{iftex}\n");
    if cjk {
        out.push_str("\\RequireLuaTeX\n");
        out.push_str("\\usepackage{luatexja-fontspec}\n");
        out.push_str(
            "% Harano Aji is the open Japanese family TeX Live ships. Substitute\n\
             % any installed face here if this document is not Japanese.\n\
             \\setmainjfont{HaranoAjiMincho}\n\
             \\setsansjfont{HaranoAjiGothic}\n\
             \\setmonojfont{HaranoAjiGothic}\n",
        );
    } else if wide {
        // Refused rather than attempted. pdfTeX's 8-bit fonts have no slot
        // for these characters, so what it actually does is emit an error per
        // character and set nothing — dozens of them, with the real cause at
        // the top of a log nobody reads to the top. One stated error beats
        // that. XeTeX and LuaTeX are both fine, so the test is on the engine
        // that cannot rather than on the one that must.
        out.push_str(
            "\\ifPDFTeX\n  \\PackageError{rustyfi}{This document needs xelatex or \
             lualatex}{It contains characters outside Latin-1, which pdflatex \
             cannot set.}\n\\fi\n",
        );
    } else {
        // Both are no-ops outside pdfTeX — modern LaTeX is UTF-8 by default
        // and `fontspec` chooses its own encoding — and T1 under a Unicode
        // engine is actively wrong, so the guard is not decoration.
        out.push_str("\\ifPDFTeX\n  \\usepackage[T1]{fontenc}\n  \\usepackage[utf8]{inputenc}\n\\fi\n");
    }
    out.push_str(&geometry_line(paper_w, paper_h, area));
    out.push_str("\\usepackage{amsmath}\n\\usepackage{amssymb}\n");
    if ctx.uses_tikz.get() {
        // `tikz` loads `xcolor` itself, so declaring it separately would be
        // the noise this preamble is trying not to have.
        out.push_str("\\usepackage{tikz}\n");
    }
    if ctx.uses_verbatim.get() {
        out.push_str(
            "% fvextra, not fancyvrb alone: `breaklines` does not exist in the\n\
             % fancyvrb 2.8 line at all (it is a fatal xkeyval error, not a\n\
             % silent no-op), and `breakanywhere` is what a listing whose long\n\
             % lines contain no spaces actually needs.\n\
             \\usepackage{fvextra}\n\
             \\fvset{fontsize=\\small,breaklines=true,breakanywhere=true}\n",
        );
    }
    if ctx.uses_hyperref.get() {
        // Last, and it has to be: hyperref redefines a great deal of what is
        // loaded before it, and expects to be able to.
        let _ = writeln!(
            out,
            "\\usepackage[hidelinks{}]{{hyperref}}",
            pdf_metadata(extras)
        );
    }
    out.push('\n');
    out
}

/// The `geometry` invocation, taken from the document's own page.
///
/// **Exact, and in `bp`.** A `Length` in this port is 1/72 inch (PDF user
/// space, which is what the PDF writer emits into unchanged); TeX's `pt` is
/// 1/72.27 inch and its `bp` is 1/72. Writing `pt` here would make every page
/// 0.37% too small — invisible on one page and a whole line by the bottom of
/// a long one.
///
/// The MARGINS are the document's too, derived from where it put its text
/// area. That matters more than it sounds: a slide deck (`slydifi`) declares
/// a landscape page with almost no margin, and defaulting to `article`'s A4
/// portrait would reflow every slide into something unrecognisable.
/// **`head=0`/`headsep=0`/`footskip=0` are load-bearing, not tidiness.** In
/// `geometry`'s default mode `top` and `bottom` are the margins OUTSIDE the
/// running head and foot, so `\textheight` comes out as
/// `paperheight - top - bottom - headheight - headsep - footskip` — some 67bp
/// less than asked for. `slydifi` declares a 720x405bp slide and its first
/// slide is a full-bleed 405bp drawing: 67bp short, that drawing can never
/// fit on a page, so LaTeX ships an empty one and tries again. It got to
/// 131072 pages before `dest_names_size` ran out. There is no running head to
/// reserve room for in the first place — the input is the pre-page-break
/// flow, from which page furniture is already absent — hence
/// `\pagestyle{empty}` alongside.
fn geometry_line(paper_w: f64, paper_h: f64, area: (f64, f64)) -> String {
    let left = ((paper_w - area.0) / 2.0).max(0.0);
    let top = ((paper_h - area.1) / 2.0).max(0.0);
    format!(
        "\\usepackage[paperwidth={:.3}bp,paperheight={:.3}bp,left={:.3}bp,right={:.3}bp,\
         top={:.3}bp,bottom={:.3}bp,head=0bp,headsep=0bp,footskip=0bp]{{geometry}}\n\
         \\pagestyle{{empty}}\n",
        paper_w, paper_h, left, left, top, top,
    )
}

/// The text area to set the document in, in points.
///
/// **The paper size is the document's; the MARGINS are measured, because the
/// document does not record them.** `PageGeometry` looks like it carries a
/// text area — it has `text_origin`, `text_width` and `text_height` — but
/// every real compile builds it through `PageGeometry::for_paper`, which sets
/// the origin to `(0, 0)` and the area to the whole sheet. The margins a
/// document actually uses live in its `page-break` callback, lang-side, and
/// never reach a backend. Taking those fields at face value put every
/// generated document's text edge to edge on the paper, which no reader would
/// accept and which also made [`crate::tikz::fit_scale`] a no-op.
///
/// So the WIDTH is measured from the flow: the paragraph breaker set every
/// justified line to the measure, so the widest line in the document IS the
/// measure. That is exact for any document with one full line in it, which is
/// every prose document; a document of nothing but short centred lines
/// measures narrow, and gets the fallback below instead.
///
/// The HEIGHT is not recoverable at all — the input is the pre-page-break
/// flow, in which there are no pages to have had a top margin. It takes the
/// same margin as the width, which at least makes the page look deliberate.
///
/// [`MIN_MARGIN_RATIO`] floors both: a measured width equal to the paper is
/// what a document with a full-bleed drawing in it reports, and a page with
/// no margin at all is not a document.
fn text_area(source: Option<&[VertBox]>, paper_w: f64, paper_h: f64) -> (f64, f64) {
    let measured = source.map_or(0.0, widest_line);
    let max_w = paper_w * (1.0 - 2.0 * MIN_MARGIN_RATIO);
    let width = if measured > 0.0 {
        measured.min(max_w)
    } else {
        max_w
    };
    let margin = (paper_w - width) / 2.0;
    (width, (paper_h - 2.0 * margin).max(paper_h * 0.5))
}

/// The narrowest margin a generated page gets, as a fraction of the paper's
/// width — see [`text_area`].
const MIN_MARGIN_RATIO: f64 = 0.06;

/// The widest line in the flow, which is the measure the document was broken
/// at. Recursive through `EmbeddedBlock`, since a figure's caption is set at
/// its own narrower measure and would otherwise be the only thing measured in
/// a document that is mostly figures.
fn widest_line(vboxes: &[VertBox]) -> f64 {
    let mut widest: f64 = 0.0;
    for vb in vboxes {
        let VertBox::Line { contents, .. } = vb else {
            continue;
        };
        let mut extent: f64 = 0.0;
        for (x, bx) in contents {
            extent = extent.max(x.0 + bx.natural_width().0);
            if let rustyfi_backend::PureHorzBox::EmbeddedBlock { block, .. } = bx {
                widest = widest.max(widest_line(block));
            }
        }
        widest = widest.max(extent);
    }
    widest
}

/// `pdftitle`/`pdfauthor` from the document's own metadata, when it set any.
///
/// The same fields the PDF writer puts in `/Info`, so the two outputs agree
/// about what the document is called. Emitted as `hyperref` options rather
/// than a separate `\hypersetup`, which keeps the preamble one line shorter
/// and is where `hyperref`'s own documentation puts them.
fn pdf_metadata(extras: &DocExtras) -> String {
    let mut out = String::new();
    let Some(info) = &extras.doc_info else {
        return out;
    };
    if let Some(title) = info.title.as_deref().filter(|t| !t.trim().is_empty()) {
        let _ = write!(out, ",pdftitle={{{}}}", escape::text(title));
    }
    if let Some(author) = info.author.as_deref().filter(|a| !a.trim().is_empty()) {
        let _ = write!(out, ",pdfauthor={{{}}}", escape::text(author));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustyfi_backend::PureHorzBox;

    /// The engine is named where a reader will see it, and enforced where TeX
    /// will. A document with no CJK says neither — it compiles anywhere.
    #[test]
    fn the_cjk_engine_requirement_is_stated_and_enforced() {
        let cjk = [VertBox::Line {
            height: rustyfi_backend::Length::pt(9.0),
            depth: rustyfi_backend::Length::pt(2.0),
            leading: rustyfi_backend::Length::pt(12.0),
            contents: vec![(
                rustyfi_backend::Length::ZERO,
                PureHorzBox::InnerString {
                    info: rustyfi_backend::HorzStringInfo {
                        font: rustyfi_backend::FontKey(0),
                        size: rustyfi_backend::Length::pt(12.0),
                        rising: rustyfi_backend::Length::ZERO,
                        color: rustyfi_backend::Color::Gray(0.0),
                    },
                    text: "研究計画".into(),
                    width: rustyfi_backend::Length::pt(48.0),
                    height: rustyfi_backend::Length::pt(9.0),
                    depth: rustyfi_backend::Length::pt(2.0),
                },
            )],
        }];
        let tex = render_latex(
            Some(&cjk),
            &PageGeometry::default(),
            &[],
            &DocExtras::default(),
            &[],
            &[],
        )
        .unwrap();
        assert!(tex.contains("% ENGINE: lualatex."), "{tex}");
        assert!(tex.contains("\\RequireLuaTeX"), "{tex}");
        assert!(tex.contains("\\usepackage{luatexja-fontspec}"), "{tex}");
        // …and the CJK is not spaced out, which is the one rule every one of
        // these backends has to get right.
        assert!(tex.contains("研究計画"), "{tex}");
    }

}
