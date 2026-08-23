//! `--format` output-format selector (surface).
//!
//! Deliberately NOT a clap `ValueEnum`: matches `--lang`'s precedent of a
//! plain `Arg` with `.value_parser([...])` for clap-level validation, parsed
//! to a real Rust type at the use site via `str::parse`.

/// The compile target's output medium. `Pdf` is the default, so omitting
/// `--format` keeps every existing invocation's behavior byte-identical to
/// before this flag existed.
///
/// ## Which HTML `--format html` means
///
/// `Html` is the REFLOWABLE backend (`rustyfi_html::render_html_reflow`): one
/// continuous document, real `<p>`s the browser re-breaks and justifies,
/// headings, lists, links, footnotes in the flow. That is what someone asking
/// a typesetter for HTML wants, and it is the only one of the two whose
/// output is worth reading in a browser.
///
/// `HtmlFixed` (`--format html-fixed`,
/// `rustyfi_html::render_html_fixed`) is the layout-faithful serialization of
/// exactly the placed boxes the PDF writer consumes — one `.page` div per
/// page, every glyph run at its own `position:absolute`. It is kept, under a
/// name that says what it is, for the one job it is genuinely good at:
/// diffing this port's layout against the PDF in a browser, where you can
/// inspect a run's coordinates instead of eyeballing two renderings. It is
/// not a web page and was never meant to be read as one.
///
/// `--format html-reflow` still parses, as an alias of `html`, so any
/// existing script keeps working.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum OutputFormat {
    #[default]
    Pdf,
    /// Reflowable, semantic HTML — see this type's doc comment.
    Html,
    /// Layout-faithful, absolutely-positioned HTML — see this type's doc
    /// comment.
    HtmlFixed,
}

impl std::str::FromStr for OutputFormat {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "pdf" => Ok(OutputFormat::Pdf),
            // `html-reflow` was the name while the faithful backend held
            // `html`; kept as an alias so the rename breaks nobody.
            "html" | "html-reflow" => Ok(OutputFormat::Html),
            "html-fixed" => Ok(OutputFormat::HtmlFixed),
            other => Err(format!(
                "unknown --format {other:?} (expected pdf|html|html-fixed)"
            )),
        }
    }
}

impl OutputFormat {
    /// Derives `-o`'s default when omitted. `HtmlFixed` shares this same
    /// `.html` extension; `cache_tag` below, not the extension, is what
    /// keeps its cache entries from colliding with reflowed HTML's.
    pub fn extension(self) -> &'static str {
        match self {
            OutputFormat::Pdf => "pdf",
            OutputFormat::Html => "html",
            OutputFormat::HtmlFixed => "html",
        }
    }

    /// Domain-separator byte-string folded into the compile-cache key
    /// (`cache.rs::hash_inputs`) so a PDF render and an HTML render of the
    /// identical input never collide under the same cache key (and so a
    /// hit's stored bytes are never written to the wrong-format output).
    /// The two HTML modes get DISTINCT tags so a cached render of one can
    /// never be served back for the other.
    pub(crate) fn cache_tag(self) -> &'static str {
        match self {
            OutputFormat::Pdf => "pdf",
            OutputFormat::Html => "html-reflow",
            OutputFormat::HtmlFixed => "html",
        }
    }
}
