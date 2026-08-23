//! HTML output backend (`--format html`): one continuous, self-contained,
//! semantic web document.
//!
//! The whole backend lives in the [`reflow`] submodule — see its doc comment
//! for what the box stream becomes and why. This root module holds only what
//! that submodule and its helpers share: the crate's error type, HTML
//! escaping, and the `base64`/`fonts`/`image`/`svg` helper modules.
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
//! used below, plus `rustyfi-pdf` for [`rustyfi_pdf::TtfFontStore`] (the one
//! type this crate reuses rather than re-implements — only its `pub`
//! `file_index`/`file_family_name` accessors are used, so this is a plain
//! one-way dependency, not a cycle: `rustyfi-pdf` does not depend on
//! `rustyfi-html`). Nothing here touches `pdf_writer` or any other
//! PDF-specific type, only `rustyfi_backend`/`rustyfi_pdf::TtfFontStore`
//! types and `String` building.

mod base64;
mod fonts;
mod image;
mod reflow;
mod svg;

pub use reflow::{
    render_html_reflow, render_html_reflow_ttf_with, render_html_reflow_ttf_with_decos,
    render_html_reflow_with_decos,
};

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
