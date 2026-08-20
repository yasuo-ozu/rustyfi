//! `--format` output-format selector (surface).
//!
//! Deliberately NOT a clap `ValueEnum`: this crate uses *builder*-style clap
//! throughout (`dispatch::compile_command`), and `--target-version` sets the
//! precedent (`dispatch.rs:75-82`, parsed at `main.rs:296-299`) — a plain
//! `Arg` with `.value_parser([...])` for clap-level validation, parsed to a
//! real Rust type at the use site via `str::parse`. `--format` follows the
//! exact same shape.

/// The compile target's output medium. `Pdf` is the default, so omitting
/// `--format` keeps every existing invocation's behavior byte-identical to
/// before this flag existed.
///
/// `HtmlReflow` ("CLI"): a THIRD, independent format alongside `Pdf`/`Html` —
/// the reflowable/semantic HTML twin (`rustyfi_html::render_html_reflow`),
/// not a variant of `Html`. Adding it changes no existing match arm's
/// behavior (every prior exhaustive `match format { ... }` gains one new arm;
/// `Pdf`/`Html` are untouched).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum OutputFormat {
    #[default]
    Pdf,
    Html,
    HtmlReflow,
}

impl std::str::FromStr for OutputFormat {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "pdf" => Ok(OutputFormat::Pdf),
            "html" => Ok(OutputFormat::Html),
            "html-reflow" => Ok(OutputFormat::HtmlReflow),
            other => Err(format!(
                "unknown --format {other:?} (expected pdf|html|html-reflow)"
            )),
        }
    }
}

impl OutputFormat {
    /// The default output file extension for this format — used to derive
    /// `-o`'s default (`input.with_extension(..)`, `main.rs`'s `cmd_compile`)
    /// when `-o` is omitted. `HtmlReflow` shares `html` (design doc §5): it
    /// is still an `.html` file, just a different serialization of the same
    /// compiled document — `cache_tag` below, not the extension, is what
    /// keeps its cache entries from colliding with faithful HTML's.
    pub fn extension(self) -> &'static str {
        match self {
            OutputFormat::Pdf => "pdf",
            OutputFormat::Html => "html",
            OutputFormat::HtmlReflow => "html",
        }
    }

    /// Domain-separator byte-string folded into the compile-cache key
    /// (`cache.rs::hash_inputs`) so a PDF render and an HTML render of the
    /// identical input never collide under the same cache key (and so a
    /// hit's stored bytes are never written to the wrong-format output).
    /// `HtmlReflow` gets its OWN distinct tag (not `"html"`) so a cached
    /// faithful-HTML render can never be served back for a `html-reflow`
    /// request or vice versa.
    pub(crate) fn cache_tag(self) -> &'static str {
        match self {
            OutputFormat::Pdf => "pdf",
            OutputFormat::Html => "html",
            OutputFormat::HtmlReflow => "html-reflow",
        }
    }
}
