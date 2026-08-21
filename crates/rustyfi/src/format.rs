//! `--format` output-format selector (surface).
//!
//! Deliberately NOT a clap `ValueEnum`: this crate uses *builder*-style clap
//! throughout (`dispatch::compile_command`), and `--lang` sets the
//! precedent (`dispatch.rs:75-82`, parsed at `main.rs:296-299`) — a plain
//! `Arg` with `.value_parser([...])` for clap-level validation, parsed to a
//! real Rust type at the use site via `str::parse`. `--format` follows the
//! exact same shape.

/// The compile target's output medium. `Pdf` is the default, so omitting
/// `--format` keeps every existing invocation's behavior byte-identical to
/// before this flag existed.
///
/// Only `Pdf` is built on this branch. The HTML backends (a
/// layout-faithful serialization and a reflowable/semantic one) live on the
/// `html-support` branch, which re-adds the `rustyfi-html` crate and the
/// `--format html`/`html-reflow` arms on top of this enum.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum OutputFormat {
    #[default]
    Pdf,
}

impl std::str::FromStr for OutputFormat {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "pdf" => Ok(OutputFormat::Pdf),
            other => Err(format!(
                "unknown --format {other:?} (expected pdf)"
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
        }
    }

    /// Domain-separator byte-string folded into the compile-cache key
    /// (`cache.rs::hash_inputs`) so a PDF render and an HTML render of the
    /// identical input never collide under the same cache key (and so a
    /// hit's stored bytes are never written to the wrong-format output).
    pub(crate) fn cache_tag(self) -> &'static str {
        match self {
            OutputFormat::Pdf => "pdf",
        }
    }
}
