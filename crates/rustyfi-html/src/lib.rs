//! HTML and Markdown output, plus the structure recovery a THIRD backend
//! reads out of this crate.
//!
//! `--format html` ([`reflow`]) is one continuous, self-contained, semantic
//! web document. `--format markdown` ([`markdown`]) is a SUBSET of it —
//! the same document structure, written in a smaller vocabulary.
//! `--format latex` is the same structure again, handed to another
//! typesetter: real `\frac`s, real `tikzpicture`s and real cross-references,
//! because LaTeX can say most of what SATySFi can. It is a crate of its own,
//! `rustyfi-latex`, and it reads its half of the recovery from here.
//!
//! **The recovery itself is shared** ([`recover`], and `mathrec` for the
//! inside of a formula): which paragraph is a heading, where the lists and
//! their nesting are, how a table's rows are regrouped out of a flat cell
//! list, which of its grid lines it actually draws, when a glue box is a
//! space and when it would ruin a Japanese sentence, and whose hyphen sits at
//! the end of a line. Every one of those was got wrong at least once before
//! it was got right, so there is one implementation and three callers. What
//! each backend decides for itself is only what to WRITE.
//!
//! This root module holds what all of them share: the crate's error type,
//! HTML escaping, and the `base64`/`fonts`/`image`/`svg` helper modules.
//!
//! **[`recover`], [`latex`] and [`collapse_whitespace`] are `pub`, and
//! nothing else new is.** They are exactly what `rustyfi-latex` reads: the
//! structure recovery above, the math-run writer it shares with `--katex`,
//! and the whitespace fold a GFM row and a LaTeX alignment cell both need.
//! Publishing them is what lets the third backend be its own crate without a
//! second `wants_space` — a second copy of the CJK glue rule is the one
//! outcome that would be definitely wrong, because the two would diverge and
//! the symptom (a space between every pair of Japanese characters, in one
//! format only) is not one anybody would come looking for here. Everything
//! else in this crate is still private.
//!
//! That is achieved for the rules [`recover`] enumerates, and NOT for eight
//! box-stream helpers this crate still defines twice or three times over —
//! see "Still forked" at the end of [`recover`]'s module doc, which names
//! each one and where its copies are. The debt is `rustyfi-html`-internal
//! (`rustyfi-latex` calls the hoisted copy); it is written down there rather
//! than paid off here because all four sites have other work in flight.
//!
//! **The crate is still misnamed, and it is a smaller lie than it was.** It
//! was the HTML backend; it is the HTML and Markdown backends plus the
//! recovery all three share. Lifting [`recover`]/[`latex`]/`mathrec` into a
//! fourth crate that this one and `rustyfi-latex` both depend on is the
//! honest end state, and is deliberately NOT done here: those modules are
//! named from roughly a hundred sites across `reflow/`, `markdown/` and
//! `mathsvg`, so the lift is a rename sweep through every file the other two
//! backends live in, for no behavioural gain. It belongs in a commit that
//! does only that.
//!
//! The document is built from the flat block stream as it stood BEFORE page
//! breaking (`DocumentValue::reflow_source` in `rustyfi-lang`), so there are
//! no pages in it and nothing is cut at a page boundary; the browser does the
//! line breaking. Nothing is fetched and nothing is executed — graphics and
//! math are inline `<svg>` (`svg.rs`), images are data URIs (`image.rs`), and
//! fonts are NAMED rather than embedded (`fonts.rs`'s `reflow_font_stack`).
//!
//! **Location.** This is its own `rustyfi-html` crate, a peer of
//! `rustyfi-pdf`. It depends on `rustyfi-backend` for every box/geometry type
//! it walks, plus `rustyfi-pdf` for [`rustyfi_pdf::TtfFontStore`] (the one
//! type this crate reuses rather than re-implements — only its `pub`
//! `file_index`/`file_family_name` accessors are used, so this is a plain
//! one-way dependency, not a cycle: `rustyfi-pdf` does not depend on
//! `rustyfi-html`). Nothing here touches `pdf_writer` or any other
//! PDF-specific type, only `rustyfi_backend`/`rustyfi_pdf::TtfFontStore`
//! types and `String` building.

mod base64;
mod fonts;
mod image;
// `latex.rs` documents itself against a dozen of its own helpers and against
// `mathrec`, all of which stayed private when the module went `pub` so that
// `rustyfi-latex` could reach `math_latex`. Those links are for the reader of
// the SOURCE and are worth keeping; rustdoc's objection to them is not.
#[allow(rustdoc::private_intra_doc_links)]
pub mod latex;
mod markdown;
mod mathml;
mod mathrec;
mod mathsvg;
pub mod recover;
mod reflow;
mod svg;

pub use markdown::{render_markdown, render_markdown_ttf_with};
pub use reflow::{
    render_html_reflow, render_html_reflow_ttf_with, render_html_reflow_ttf_with_decos,
    render_html_reflow_with_decos,
};

/// How an equation is written into the output.
///
/// Math is the one part of a document that no reflowed format can simply
/// carry over: `${\frac{a}{b}}` is laid out during compilation, so what
/// reaches a backend is positioned glyphs and a couple of filled paths, with
/// no `\frac` node left anywhere ([`crate::mathrec`]'s doc comment has the
/// detail). There is therefore no single right answer, only several answers
/// that are right for different readers — and which one a document wants is a
/// property of where it is going to be READ, not of the document. So it is a
/// flag rather than a heuristic.
///
/// **There is deliberately no [`Default`] impl**, and its absence is the
/// point: the same argument that makes this a flag makes a GLOBAL default
/// wrong. Where a document will be read is exactly what the output FORMAT
/// already says, so the default follows the format — HTML draws the faithful
/// outline, Markdown writes the compact `<text>`. Which mode each format
/// picks is decided once, in the CLI's `OutputFormat: FromStr`, and every
/// entry point here takes the mode explicitly so that no caller can
/// accidentally inherit someone else's answer.
///
/// The five, and the axis each trades on:
///
/// | mode | flag | ink | needs of the reader |
/// |--|--|--|--|
/// | [`MathMode::SvgOutline`] | `--svg-outline-math` | outline paths | nothing |
/// | [`MathMode::SvgText`] | `--svg-math` | `<text>` + `<rect>` | the document's faces |
/// | [`MathMode::Katex`] | `--katex` | LaTeX source | a math typesetter |
/// | [`MathMode::MathMl`] | `--mathml` | MathML Core elements | a 2023-or-later browser |
/// | [`MathMode::Unicode`] | `--unicode-math` | characters | nothing |
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum MathMode {
    /// Draw each glyph as an SVG outline path taken from the document's own
    /// face, with the characters kept behind it as invisible, selectable text
    /// ([`crate::mathsvg::emit_outline_layer`]).
    ///
    /// Faithful, self-contained and font-independent: it draws exactly what
    /// the PDF draws, and it needs nothing of the reader — no math typesetter,
    /// no font, no network. **HTML's default**, where all of that is free and
    /// where it is what the backend has always done.
    ///
    /// The cost is size: an outline is hundreds of coordinates per glyph.
    /// [`MathMode::SvgText`] is the same drawing at a fraction of the bytes,
    /// for a reader who has the faces.
    SvgOutline,
    /// Draw the equation with SVG's own `<text>` for the glyphs and
    /// `<rect>`/`<line>` for the rules, positioned exactly where the layout
    /// put them ([`crate::mathsvg::emit_text_layer`]).
    ///
    /// **Markdown's default.** Compact, and the source reads as what it is —
    /// a fraction bar is a `<rect>` rather than four path coordinates. The
    /// text is real text, so it selects, copies and searches with no phantom
    /// layer behind it.
    ///
    /// Two costs, both stated where they are implemented. It depends on the
    /// reader HAVING the document's faces: a substitute's advances are not the
    /// ones each glyph's absolute position was computed against. And a
    /// MATH-table variant — a display-size `∑`, a stretched delimiter — has no
    /// character that names it, so those glyphs keep an outline `<path>`; see
    /// `crate::mathsvg::emit_text_layer` for why that hybrid is not optional.
    SvgText,
    /// Write the equation's characters in reading order, using Unicode's own
    /// super/subscript characters where they exist (`x²`, `∑ₐᵇ`) and splitting
    /// a fraction at its bar (`(a+b)/(c+d)`).
    ///
    /// Markdown only. It is the only form that survives a sanitizing renderer,
    /// the only one that reads as text in a terminal, and the only one that is
    /// searchable in the raw file — and it loses radicals, matrices and nested
    /// fractions to get there. Meaningless for the HTML backend, which can
    /// always draw the real thing.
    Unicode,
    /// Write LaTeX in math delimiters, for a reader whose renderer runs KaTeX
    /// or MathJax ([`crate::latex`]).
    ///
    /// A RE-DERIVATION from the laid-out glyphs rather than a round trip, so
    /// several constructs cannot come back — `crate::latex`'s doc comment
    /// lists exactly which, and why each is unavailable rather than merely
    /// unimplemented. Opt-in in both formats for that reason: it typesets as
    /// real mathematics and survives a sanitizer, but it is the only mode that
    /// can render something the PDF does not show.
    Katex,
    /// Write the equation as **MathML Core** elements the browser lays out
    /// itself ([`crate::mathml`]).
    ///
    /// The one mode whose output is neither a picture nor a foreign language:
    /// `<mfrac>`, `<msubsup>` and `<munderover>` are real structure in the
    /// document's own tree, so a screen reader reads the equation as
    /// mathematics rather than as a row of characters, the browser's find
    /// works, and no script runs — where [`MathMode::Katex`] needs the reader
    /// to have loaded KaTeX or MathJax, and both SVG modes hand over a
    /// drawing that assistive technology can only read out of a phantom layer.
    ///
    /// Opt-in in both formats, for two reasons that do not cancel out. The
    /// support floor is a browser from 2023 — Firefox has always had it and
    /// Safari since 2013, but Chromium only since 109 — and, more to the
    /// point, it is the SAME re-derivation `--katex` is, from the same
    /// [`crate::mathrec`] recovery, so it carries every one of that mode's
    /// losses while rendering them as confidently as a hand-written equation.
    /// [`crate::mathml`]'s doc comment says what is done about that.
    MathMl,
}

/// Rendering is in practice infallible — every text run is valid
/// UTF-8/HTML-escapable, and image/font handling reads from tables the
/// compile step already validated. The `Result` return shape is kept anyway
/// so the entry points are argument-for-argument (module signature, not
/// module fallibility) with `rustyfi_pdf::render_pdf_with`, and so a future
/// embedding step can surface a real error without a breaking signature
/// change.
#[derive(Debug, thiserror::Error)]
pub enum HtmlError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Fold every run of whitespace to one space and drop it at the edges.
///
/// Not a formatting nicety in either caller: it is what makes a recovered
/// paragraph safe to put somewhere that has a LINE structure of its own. A
/// GFM pipe table's row grammar ends at the newline, and a LaTeX alignment
/// cell treats a blank line as a `\par` — `Paragraph ended before \\ was
/// complete`, a hard error. The backends had a copy each.
pub fn collapse_whitespace(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_space = true;
    for c in s.chars() {
        if c.is_whitespace() {
            if !last_space {
                out.push(' ');
            }
            last_space = true;
        } else {
            out.push(c);
            last_space = false;
        }
    }
    out.trim_end().to_string()
}

/// Escape the five HTML/attribute-hostile characters. Emitted text is never
/// re-parsed as markup, so this is the standard minimal set (no need for a
/// full entity table).
fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}
