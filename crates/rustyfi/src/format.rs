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
///
/// ## What `--format markdown` means
///
/// `Markdown` is a SUBSET of `Html`: the same structure recovered out of the
/// same pre-page-break box stream — headings, lists, tables, links,
/// emphasis, code blocks, footnotes — written in Markdown's much smaller
/// vocabulary. Everything Markdown cannot say (frames, alignment, colour,
/// drawings, page geometry) is dropped rather than approximated. Readability
/// is the goal; layout fidelity is explicitly not.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum OutputFormat {
    #[default]
    Pdf,
    /// Reflowable, semantic HTML — see this type's doc comment.
    Html,
    /// GitHub-flavoured Markdown — see this type's doc comment.
    Markdown,
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
            "markdown" | "md" => Ok(OutputFormat::Markdown),
            other => Err(format!(
                "unknown --format {other:?} (expected pdf|html|markdown)"
            )),
        }
    }
}

impl OutputFormat {
    /// Derives `-o`'s default when omitted.
    pub fn extension(self) -> &'static str {
        match self {
            OutputFormat::Pdf => "pdf",
            OutputFormat::Html => "html",
            OutputFormat::Markdown => "md",
        }
    }

    /// Domain-separator byte-string folded into the compile-cache key
    /// (`cache.rs::hash_inputs`) so a PDF render and an HTML render of the
    /// identical input never collide under the same cache key (and so a
    /// hit's stored bytes are never written to the wrong-format output).
    ///
    /// `Html`'s tag stays the historical `html-reflow`, and tidying it up to
    /// plain `html` would be a BUG, not a cosmetic change. `html` was the tag
    /// of the layout-faithful `--format html-fixed` backend that used to sit
    /// beside this one, and nothing else in the key tells the two apart: the
    /// cache stores every format's payload as a bare `<key>.pdf`
    /// (`cache.rs`'s `pdf_path`), and removing that backend did not bump
    /// `CARGO_PKG_VERSION`, which is the only other version-ish field in the
    /// key. So a binary spelling this `html` would HIT, and serve, entries an
    /// equally-versioned older binary wrote for `html-fixed` — a page of
    /// absolutely-positioned `<div class="page">`s handed back where a
    /// reflowed document was asked for, under the same `.html` name, with
    /// nothing downstream to notice. Keeping the historical spelling also
    /// keeps existing reflow entries valid, but that is the smaller half of
    /// the reason. Pinned by `the_html_cache_tag_is_not_the_removed_backends`
    /// below.
    ///
    /// `Markdown` gets its own tag for the same reason and with the same
    /// consequence if it does not: Markdown output is a SUBSET of HTML's, so
    /// a cache that could not tell them apart would happily hand a `.md`
    /// request the HTML document it stored earlier — and the stored payload
    /// is a bare `<key>.pdf` whatever the format, so there is no extension,
    /// no header and no magic number downstream to notice.
    pub(crate) fn cache_tag(self) -> &'static str {
        match self {
            OutputFormat::Pdf => "pdf",
            OutputFormat::Html => "html-reflow",
            OutputFormat::Markdown => "markdown",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The cache tags are a compatibility surface, not an internal spelling:
    /// see [`OutputFormat::cache_tag`]. `html` in particular is RETIRED — it
    /// belonged to the removed `--format html-fixed` backend, and an
    /// equally-versioned older binary's entries under it are still on disk in
    /// users' cache directories.
    #[test]
    fn the_html_cache_tag_is_not_the_removed_backends() {
        assert_eq!(OutputFormat::Html.cache_tag(), "html-reflow");
        assert_ne!(OutputFormat::Html.cache_tag(), "html");
        assert_ne!(
            OutputFormat::Pdf.cache_tag(),
            OutputFormat::Html.cache_tag()
        );
    }

    /// `html-fixed` is retired as a FLAG VALUE too, not merely reassigned:
    /// it must no longer parse at all, so the removal cannot be papered over
    /// by silently accepting the old spelling.
    #[test]
    fn html_fixed_no_longer_parses() {
        assert!("html-fixed".parse::<OutputFormat>().is_err());
        assert_eq!("html".parse::<OutputFormat>(), Ok(OutputFormat::Html));
        assert_eq!(
            "html-reflow".parse::<OutputFormat>(),
            Ok(OutputFormat::Html)
        );
    }

    /// The cache stores every format's payload as a bare `<key>.pdf`, so the
    /// tag is the ONLY thing standing between a `--format markdown` request
    /// and the HTML document an earlier run of the same source cached. The
    /// hazard is not hypothetical for these two in particular: Markdown's
    /// output is a subset of HTML's, recovered from the same input by the
    /// same code, so every other field in the key is identical.
    #[test]
    fn every_format_has_its_own_cache_tag() {
        let tags = [
            OutputFormat::Pdf.cache_tag(),
            OutputFormat::Html.cache_tag(),
            OutputFormat::Markdown.cache_tag(),
        ];
        let unique: std::collections::HashSet<_> = tags.iter().collect();
        assert_eq!(unique.len(), tags.len(), "cache tags collide: {tags:?}");
    }

    #[test]
    fn markdown_parses_and_names_its_own_extension() {
        assert_eq!(
            "markdown".parse::<OutputFormat>(),
            Ok(OutputFormat::Markdown)
        );
        assert_eq!("md".parse::<OutputFormat>(), Ok(OutputFormat::Markdown));
        assert_eq!(OutputFormat::Markdown.extension(), "md");
        // Every format's default output name differs, or `-o` omitted would
        // make two formats fight over one path.
        let exts = [
            OutputFormat::Pdf.extension(),
            OutputFormat::Html.extension(),
            OutputFormat::Markdown.extension(),
        ];
        let unique: std::collections::HashSet<_> = exts.iter().collect();
        assert_eq!(unique.len(), exts.len(), "extensions collide: {exts:?}");
    }
}
