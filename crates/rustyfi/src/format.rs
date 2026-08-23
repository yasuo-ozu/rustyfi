//! `--format` output-format selector (surface).
//!
//! Deliberately NOT a clap `ValueEnum`: matches `--lang`'s precedent of a
//! plain `Arg` with `.value_parser([...])` for clap-level validation, parsed
//! to a real Rust type at the use site via `str::parse`.

/// The compile target's output medium. `Pdf` is the default, so omitting
/// `--format` keeps every existing invocation's behavior byte-identical to
/// before this flag existed.
///
/// ## What `--format html` means
///
/// `Html` is the REFLOWABLE backend (`rustyfi_html::render_html_reflow`): one
/// continuous document, real `<p>`s the browser re-breaks and justifies,
/// headings, lists, links, footnotes in the flow. That is what someone asking
/// a typesetter for HTML wants.
///
/// `--format html-reflow` still parses, as an alias of `html`, so any
/// existing script keeps working.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum OutputFormat {
    #[default]
    Pdf,
    /// Reflowable, semantic HTML — see this type's doc comment.
    Html,
}

impl std::str::FromStr for OutputFormat {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "pdf" => Ok(OutputFormat::Pdf),
            // `html-reflow` was the name while a second, layout-faithful
            // backend held `html`; that backend is gone, but the alias is
            // kept so the rename breaks nobody.
            "html" | "html-reflow" => Ok(OutputFormat::Html),
            other => Err(format!("unknown --format {other:?} (expected pdf|html)")),
        }
    }
}

impl OutputFormat {
    /// Derives `-o`'s default when omitted.
    pub fn extension(self) -> &'static str {
        match self {
            OutputFormat::Pdf => "pdf",
            OutputFormat::Html => "html",
        }
    }

    /// Domain-separator byte-string folded into the compile-cache key
    /// (`cache.rs::hash_inputs`) so a PDF render and an HTML render of the
    /// identical input never collide under the same cache key (and so a
    /// hit's stored bytes are never written to the wrong-format output).
    ///
    /// `Html`'s tag stays the historical `html-reflow` rather than becoming
    /// plain `html`: the tag is a cache key, and renaming it would silently
    /// invalidate every existing entry for no behavioural gain.
    pub(crate) fn cache_tag(self) -> &'static str {
        match self {
            OutputFormat::Pdf => "pdf",
            OutputFormat::Html => "html-reflow",
        }
    }
}
