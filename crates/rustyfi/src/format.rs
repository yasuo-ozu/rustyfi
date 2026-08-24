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
/// page geometry) is dropped rather than approximated. Readability
/// is the goal; layout fidelity is explicitly not.
///
/// ## Why the two non-PDF variants carry a [`MathMode`]
///
/// Math is the one thing neither format can simply carry over — it is laid
/// out during compilation, so what reaches a backend is positioned glyphs and
/// no `\frac` node — and the right rendering depends on where the file will be
/// READ rather than on what the document says. So it is a pair of flags
/// (`--unicode-math`, `--katex`) rather than a heuristic.
///
/// It lives INSIDE the format rather than beside it because
/// [`OutputFormat::cache_tag`] has to see it. The compile cache stores every
/// format's payload as a bare `<key>.pdf`, so the tag is the only thing
/// keeping two renders of one document apart, and three math modes are three
/// different renders of it.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum OutputFormat {
    #[default]
    Pdf,
    /// Reflowable, semantic HTML — see this type's doc comment.
    Html(MathMode),
    /// GitHub-flavoured Markdown — see this type's doc comment.
    Markdown(MathMode),
}

/// Re-exported so this module and `main.rs` name one type. The renderer owns
/// the definition, since it is what has to act on it.
pub use rustyfi_html::MathMode;

impl std::str::FromStr for OutputFormat {
    type Err = String;
    /// Parses the `--format` VALUE only. The math mode is a separate pair of
    /// flags, so this always yields the default ([`MathMode::Outline`]) and
    /// `main.rs` replaces it through [`OutputFormat::with_math`] once it has
    /// read them — which is also where a mode that makes no sense for the
    /// format is refused.
    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "pdf" => Ok(OutputFormat::Pdf),
            // `html-reflow` was the name while a second, layout-faithful
            // backend held `html`; that backend is gone, but the alias is
            // kept so the rename breaks nobody.
            "html" | "html-reflow" => Ok(OutputFormat::Html(MathMode::Outline)),
            "markdown" | "md" => Ok(OutputFormat::Markdown(MathMode::Outline)),
            other => Err(format!(
                "unknown --format {other:?} (expected pdf|html|markdown)"
            )),
        }
    }
}

impl OutputFormat {
    /// This format rendering its math as `math` instead.
    ///
    /// `Pdf` comes back unchanged: it typesets the equation itself and has no
    /// such choice, and the CLI has already rejected the flag by the time this
    /// could be reached with one.
    pub fn with_math(self, math: MathMode) -> Self {
        match self {
            OutputFormat::Pdf => OutputFormat::Pdf,
            OutputFormat::Html(_) => OutputFormat::Html(math),
            OutputFormat::Markdown(_) => OutputFormat::Markdown(math),
        }
    }

    /// Derives `-o`'s default when omitted.
    pub fn extension(self) -> &'static str {
        match self {
            OutputFormat::Pdf => "pdf",
            OutputFormat::Html(_) => "html",
            OutputFormat::Markdown(_) => "md",
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
    ///
    /// ## The math modes, and the rule the whole table follows
    ///
    /// **A tag changes exactly when the BYTES change**, and every entry below
    /// is that rule applied rather than a naming preference:
    ///
    /// - `Html(Outline)` keeps the bare historical `html-reflow`. Its output
    ///   is byte-identical to what every previous binary wrote, so an entry an
    ///   older one left on disk is still a correct answer and re-tagging it
    ///   would only throw away valid cache.
    /// - `Html(Katex)` is a different document, so it is a different tag.
    /// - **`Markdown(Outline)` does NOT keep the bare `markdown` tag, and that
    ///   is the one entry here that would be a live bug if it did.** Markdown's
    ///   DEFAULT changed with this flag: `--format markdown` used to write
    ///   Unicode characters and now draws outlined SVG. Nothing else in the
    ///   key would notice — the input files, the fonts and the language
    ///   version are all unchanged, and `CARGO_PKG_VERSION` does not move on
    ///   every commit — so a stale `markdown` entry from an older binary would
    ///   be served, silently, for a request the new binary answers
    ///   differently. Renaming it invalidates exactly those entries and
    ///   nothing else.
    /// - `Markdown(Unicode)` is what that old tag MEANT, but it does not
    ///   inherit the name either: an entry written under `markdown` by an
    ///   older binary is only PROBABLY this, and "probably" is not a cache
    ///   invariant.
    ///
    /// Pinned by `every_format_and_math_mode_has_its_own_cache_tag` and
    /// `the_markdown_default_tag_changed_with_its_default` below.
    pub(crate) fn cache_tag(self) -> &'static str {
        match self {
            OutputFormat::Pdf => "pdf",
            OutputFormat::Html(MathMode::Outline) => "html-reflow",
            // `Unicode` is refused for HTML by the CLI, so this arm is
            // unreachable through the binary; it is tagged distinctly anyway
            // rather than folded into another, since a shared tag is a cache
            // collision waiting for the day the restriction is lifted.
            OutputFormat::Html(MathMode::Unicode) => "html-reflow-unicode",
            OutputFormat::Html(MathMode::Katex) => "html-reflow-katex",
            OutputFormat::Markdown(MathMode::Outline) => "markdown-outline",
            OutputFormat::Markdown(MathMode::Unicode) => "markdown-unicode",
            OutputFormat::Markdown(MathMode::Katex) => "markdown-katex",
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
        let html = OutputFormat::Html(MathMode::Outline);
        assert_eq!(html.cache_tag(), "html-reflow");
        assert_ne!(html.cache_tag(), "html");
        assert_ne!(OutputFormat::Pdf.cache_tag(), html.cache_tag());
    }

    /// `html-fixed` is retired as a FLAG VALUE too, not merely reassigned:
    /// it must no longer parse at all, so the removal cannot be papered over
    /// by silently accepting the old spelling.
    #[test]
    fn html_fixed_no_longer_parses() {
        assert!("html-fixed".parse::<OutputFormat>().is_err());
        assert_eq!(
            "html".parse::<OutputFormat>(),
            Ok(OutputFormat::Html(MathMode::Outline))
        );
        assert_eq!(
            "html-reflow".parse::<OutputFormat>(),
            Ok(OutputFormat::Html(MathMode::Outline))
        );
    }

    /// The cache stores every format's payload as a bare `<key>.pdf`, so the
    /// tag is the ONLY thing standing between a `--format markdown` request
    /// and the HTML document an earlier run of the same source cached. The
    /// hazard is not hypothetical for these two in particular: Markdown's
    /// output is a subset of HTML's, recovered from the same input by the
    /// same code, so every other field in the key is identical — and the same
    /// argument applies again, one level down, to the three math modes, which
    /// differ from each other in nothing but this field.
    #[test]
    fn every_format_and_math_mode_has_its_own_cache_tag() {
        let mut tags = vec![OutputFormat::Pdf.cache_tag()];
        for math in [MathMode::Outline, MathMode::Unicode, MathMode::Katex] {
            tags.push(OutputFormat::Html(math).cache_tag());
            tags.push(OutputFormat::Markdown(math).cache_tag());
        }
        let unique: std::collections::HashSet<_> = tags.iter().collect();
        assert_eq!(unique.len(), tags.len(), "cache tags collide: {tags:?}");
    }

    /// The one entry that would be a live bug if it were left alone.
    ///
    /// `--format markdown` with no math flag used to write Unicode characters
    /// and now draws outlined SVG. Nothing else in the cache key moved — same
    /// sources, same fonts, same language version, and `CARGO_PKG_VERSION`
    /// does not change on every commit — so keeping the bare `markdown` tag
    /// would serve an older binary's Unicode render for a request this one
    /// answers with a drawing. Neither new mode may claim the old name.
    #[test]
    fn the_markdown_default_tag_changed_with_its_default() {
        for math in [MathMode::Outline, MathMode::Unicode, MathMode::Katex] {
            assert_ne!(
                OutputFormat::Markdown(math).cache_tag(),
                "markdown",
                "the retired tag names output no current mode produces",
            );
        }
        // The default is the drawing, and it says so.
        assert_eq!(
            OutputFormat::Markdown(MathMode::default()).cache_tag(),
            "markdown-outline"
        );
    }

    /// The flags are additive: the format decides the vocabulary, the math
    /// mode decides only how equations are written, and neither reads the
    /// other.
    #[test]
    fn with_math_replaces_only_the_math_mode() {
        let md = OutputFormat::Markdown(MathMode::Outline).with_math(MathMode::Katex);
        assert_eq!(md, OutputFormat::Markdown(MathMode::Katex));
        assert_eq!(md.extension(), "md");
        // PDF has no math mode to set, and asking for one does not make it
        // into some other format.
        assert_eq!(
            OutputFormat::Pdf.with_math(MathMode::Katex),
            OutputFormat::Pdf
        );
    }

    #[test]
    fn markdown_parses_and_names_its_own_extension() {
        assert_eq!(
            "markdown".parse::<OutputFormat>(),
            Ok(OutputFormat::Markdown(MathMode::Outline))
        );
        assert_eq!(
            "md".parse::<OutputFormat>(),
            Ok(OutputFormat::Markdown(MathMode::Outline))
        );
        assert_eq!(OutputFormat::Markdown(MathMode::Outline).extension(), "md");
        // Every format's default output name differs, or `-o` omitted would
        // make two formats fight over one path. The math mode must NOT enter
        // into it: `--katex` writes a `.md`, not a `.md-katex`.
        let exts = [
            OutputFormat::Pdf.extension(),
            OutputFormat::Html(MathMode::Outline).extension(),
            OutputFormat::Markdown(MathMode::Outline).extension(),
        ];
        let unique: std::collections::HashSet<_> = exts.iter().collect();
        assert_eq!(unique.len(), exts.len(), "extensions collide: {exts:?}");
        for math in [MathMode::Unicode, MathMode::Katex] {
            assert_eq!(OutputFormat::Markdown(math).extension(), "md");
            assert_eq!(OutputFormat::Html(math).extension(), "html");
        }
    }
}
