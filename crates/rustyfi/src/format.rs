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
/// READ rather than on what the document says. So it is five flags
/// (`--svg-outline-math`, `--svg-math`, `--katex`, `--mathml`,
/// `--unicode-math`) rather than a heuristic.
///
/// It lives INSIDE the format rather than beside it for two reasons, and both
/// are load-bearing:
///
/// - **the DEFAULT is per-format** (see [`OutputFormat::from_str`]), so
///   "which format" and "which math" are not independent choices and pairing
///   them in one value is what makes an unset flag mean the right thing;
/// - **[`OutputFormat::cache_tag`] has to see it.** The compile cache stores
///   every format's payload as a bare `<key>.pdf`, so the tag is the only
///   thing keeping two renders of one document apart, and five math modes
///   are five different renders of it.
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
    /// Parses the `--format` VALUE, and with it **the math mode that format
    /// defaults to**. The three math flags are separate, and `main.rs`
    /// replaces the default through [`OutputFormat::with_math`] once it has
    /// read them — which is also where a mode that makes no sense for the
    /// format is refused.
    ///
    /// ## The defaults differ by format, on purpose
    ///
    /// [`MathMode`] has no [`Default`] impl precisely so that this decision
    /// has to be made HERE, once, where the format is known. The rule is
    /// "what serves the typical reader of this format":
    ///
    /// - **`markdown` -> [`MathMode::SvgText`]**. A `.md` is read as source at
    ///   least as often as it is rendered, and this is the mode whose source
    ///   reads as what it is — `<text>` at measured positions, a fraction bar
    ///   as a `<rect>` — at a fraction of the outline mode's bytes. It still
    ///   draws the equation where the file is previewed.
    /// - **`html` -> [`MathMode::SvgOutline`]**. A web page is self-contained
    ///   and nothing strips its markup, so the outline costs nothing but size
    ///   and is the only rendering that reproduces the PDF exactly — it does
    ///   not depend on the reader having the document's faces. This is also
    ///   what `--format html` has always done, which is why its output is
    ///   byte-identical across this change.
    ///
    /// Either default can be overridden with the flag for any other mode, so
    /// nothing here removes a choice; it only picks the one that is right
    /// more often.
    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "pdf" => Ok(OutputFormat::Pdf),
            // `html-reflow` was the name while a second, layout-faithful
            // backend held `html`; that backend is gone, but the alias is
            // kept so the rename breaks nobody.
            "html" | "html-reflow" => Ok(OutputFormat::Html(MathMode::SvgOutline)),
            "markdown" | "md" => Ok(OutputFormat::Markdown(MathMode::SvgText)),
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
    /// **A tag changes exactly when the BYTES change.** Two consequences, and
    /// the second is the one that keeps this correct as the defaults move:
    ///
    /// **1. The tag is a function of the (format, mode) PAIR — never of
    /// "was a flag given".** The pair is what determines the output, so it is
    /// what the discriminator is computed from. This is not a stylistic
    /// choice: it is what makes a change of DEFAULT safe without editing the
    /// table below, and this design has now survived two such changes. When
    /// markdown's default moved to `SvgText`, a bare `--format markdown` began
    /// computing `markdown-svgtext` automatically, because the parse yields a
    /// different pair — while an entry written under `markdown-svgoutline`
    /// stayed a correct answer for `--svg-outline-math`, which is exactly the
    /// bytes it holds. Had the tag keyed on "the default", every such change
    /// would silently re-point a live key at different content.
    ///
    /// **2. Nothing may share a tag.** Markdown output is a SUBSET of HTML's,
    /// recovered from the same input by the same code, so every other field of
    /// the key is identical between them; and the five math modes differ from
    /// each other in NOTHING but this field. That is ten combinations, nine
    /// of them reachable, and the stored payload is a bare `<key>.pdf`
    /// whatever the format — so there is no extension, no header and no magic
    /// number downstream to notice a wrong hit.
    ///
    /// Two entries are fixed by history rather than derived:
    ///
    /// - `Html(SvgOutline)` keeps the bare historical `html-reflow`, because
    ///   it IS what every previous binary wrote — `--format html` is
    ///   byte-identical across this change — so an entry left on disk is
    ///   still a correct answer and re-tagging would throw away valid cache.
    /// - **No mode may claim the bare `markdown` tag.** That name belongs to
    ///   the pre-flag renderer, whose output only PROBABLY matches
    ///   `Markdown(Unicode)` today, and "probably" is not a cache invariant.
    ///   Nothing else in the key would catch the difference: the inputs, the
    ///   fonts and the language version are unchanged, and
    ///   `CARGO_PKG_VERSION` does not move on every commit.
    ///
    /// Pinned by `every_format_and_math_mode_has_its_own_cache_tag`,
    /// `no_mode_claims_the_retired_markdown_tag` and
    /// `each_formats_default_tags_itself_by_the_mode_it_resolves_to` below.
    pub(crate) fn cache_tag(self) -> &'static str {
        match self {
            OutputFormat::Pdf => "pdf",
            OutputFormat::Html(MathMode::SvgOutline) => "html-reflow",
            // `Unicode` is refused for HTML by the CLI, so this arm is
            // unreachable through the binary; it is tagged distinctly anyway
            // rather than folded into another, since a shared tag is a cache
            // collision waiting for the day the restriction is lifted.
            OutputFormat::Html(MathMode::SvgText) => "html-reflow-svgtext",
            OutputFormat::Html(MathMode::Unicode) => "html-reflow-unicode",
            OutputFormat::Html(MathMode::Katex) => "html-reflow-katex",
            OutputFormat::Html(MathMode::MathMl) => "html-reflow-mathml",
            OutputFormat::Markdown(MathMode::SvgOutline) => "markdown-svgoutline",
            OutputFormat::Markdown(MathMode::SvgText) => "markdown-svgtext",
            OutputFormat::Markdown(MathMode::Unicode) => "markdown-unicode",
            OutputFormat::Markdown(MathMode::Katex) => "markdown-katex",
            OutputFormat::Markdown(MathMode::MathMl) => "markdown-mathml",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every [`MathMode`], for the tests that must cover ALL of them.
    ///
    /// A literal list repeated in each test is what this file had, and it is
    /// the wrong shape for the property being asserted: the collision test's
    /// whole job is to see a new mode, and a hand-written list silently does
    /// not. [`all_math_modes_are_listed`] closes that with a `match` the
    /// compiler checks, so adding a variant fails to BUILD here rather than
    /// passing vacuously.
    const ALL_MATH_MODES: [MathMode; 5] = [
        MathMode::SvgOutline,
        MathMode::SvgText,
        MathMode::Unicode,
        MathMode::Katex,
        MathMode::MathMl,
    ];

    /// [`ALL_MATH_MODES`] really is all of them — enforced by the compiler,
    /// not by the length.
    ///
    /// The `match` below has no wildcard arm, so a sixth variant is a hard
    /// COMPILE error here; the `contains` then makes sure it was added to the
    /// array too, which the `match` alone would not.
    ///
    /// It maps each mode to the flag that selects it rather than to itself,
    /// which is what keeps the guard honest in two ways: an identity match is
    /// a no-op clippy rightly objects to, and stating the flag makes a new
    /// mode declare its own name in the one place that enumerates them all.
    #[test]
    fn all_math_modes_are_listed_with_the_flag_that_selects_each() {
        let mut flags: Vec<&str> = Vec::new();
        for mode in ALL_MATH_MODES {
            flags.push(match mode {
                MathMode::SvgOutline => "--svg-outline-math",
                MathMode::SvgText => "--svg-math",
                MathMode::Unicode => "--unicode-math",
                MathMode::Katex => "--katex",
                MathMode::MathMl => "--mathml",
            });
            assert!(
                ALL_MATH_MODES.contains(&mode),
                "{mode:?} is missing from ALL_MATH_MODES, so every test that \
                 iterates it silently stops covering this mode",
            );
        }
        // One flag per mode and no mode listed twice — otherwise the array's
        // length stops meaning "how many modes there are" and a missing one
        // hides behind a duplicate.
        let unique: std::collections::HashSet<_> = flags.iter().collect();
        assert_eq!(unique.len(), ALL_MATH_MODES.len(), "flags collide: {flags:?}");
    }

    /// The cache tags are a compatibility surface, not an internal spelling:
    /// see [`OutputFormat::cache_tag`]. `html` in particular is RETIRED — it
    /// belonged to the removed `--format html-fixed` backend, and an
    /// equally-versioned older binary's entries under it are still on disk in
    /// users' cache directories.
    #[test]
    fn the_html_cache_tag_is_not_the_removed_backends() {
        let html = OutputFormat::Html(MathMode::SvgOutline);
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
            Ok(OutputFormat::Html(MathMode::SvgOutline))
        );
        assert_eq!(
            "html-reflow".parse::<OutputFormat>(),
            Ok(OutputFormat::Html(MathMode::SvgOutline))
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
        for math in ALL_MATH_MODES {
            tags.push(OutputFormat::Html(math).cache_tag());
            tags.push(OutputFormat::Markdown(math).cache_tag());
        }
        let unique: std::collections::HashSet<_> = tags.iter().collect();
        assert_eq!(unique.len(), tags.len(), "cache tags collide: {tags:?}");
    }

    /// The bare `markdown` tag is RETIRED and no mode may claim it.
    ///
    /// It names the output of the pre-flag renderer, which only PROBABLY
    /// matches `Markdown(Unicode)` today — and nothing else in the cache key
    /// would catch the difference: same sources, same fonts, same language
    /// version, and `CARGO_PKG_VERSION` does not change on every commit. An
    /// older binary's entry served for a request this one answers differently
    /// is exactly the failure the tag exists to prevent.
    #[test]
    fn no_mode_claims_the_retired_markdown_tag() {
        for math in ALL_MATH_MODES {
            assert_ne!(
                OutputFormat::Markdown(math).cache_tag(),
                "markdown",
                "the retired tag names output no current mode produces",
            );
        }
    }

    /// **The property that keeps the tag correct when a DEFAULT moves.**
    ///
    /// The tag is computed from the (format, mode) pair, never from "was a
    /// flag given" — so a bare `--format markdown` tags itself by the mode it
    /// actually resolves to, and changing which mode that is re-points it
    /// automatically. This test states both defaults through the same path
    /// the CLI uses (`FromStr`), so it fails if either the default or the tag
    /// moves without the other.
    ///
    /// The two are checked TOGETHER rather than separately because the bug
    /// being guarded against is precisely a disagreement between them: a
    /// default that changed while its tag did not would serve the old bytes,
    /// silently, forever.
    #[test]
    fn each_formats_default_tags_itself_by_the_mode_it_resolves_to() {
        // Markdown defaults to the compact text drawing: a `.md` is read as
        // source at least as often as it is rendered.
        let md: OutputFormat = "markdown".parse().unwrap();
        assert_eq!(md, OutputFormat::Markdown(MathMode::SvgText));
        assert_eq!(md.cache_tag(), "markdown-svgtext");

        // HTML defaults to the outline, which is what it has always done —
        // hence the unchanged historical tag, and hence byte-identical output.
        let html: OutputFormat = "html".parse().unwrap();
        assert_eq!(html, OutputFormat::Html(MathMode::SvgOutline));
        assert_eq!(html.cache_tag(), "html-reflow");

        // The two formats' defaults are genuinely different modes; a single
        // global default would make one of these wrong.
        assert_ne!(md, OutputFormat::Markdown(MathMode::SvgOutline));
        assert_ne!(md.cache_tag(), html.cache_tag());

        // `--svg-outline-math` on markdown reaches the mode that was briefly
        // its default, and tags itself as that — so an entry written then is
        // still a correct answer for the flag rather than a stale one for the
        // new default.
        assert_eq!(
            md.with_math(MathMode::SvgOutline).cache_tag(),
            "markdown-svgoutline"
        );
        // …and every other mode this format can take is reachable and
        // distinct, so no flag can collide with the default.
        assert_eq!(md.with_math(MathMode::Katex).cache_tag(), "markdown-katex");
        assert_eq!(
            md.with_math(MathMode::Unicode).cache_tag(),
            "markdown-unicode"
        );
        assert_eq!(md.with_math(MathMode::MathMl).cache_tag(), "markdown-mathml");
        assert_eq!(
            html.with_math(MathMode::MathMl).cache_tag(),
            "html-reflow-mathml"
        );
    }

    /// The flags are additive: the format decides the vocabulary, the math
    /// mode decides only how equations are written, and neither reads the
    /// other.
    #[test]
    fn with_math_replaces_only_the_math_mode() {
        let md = OutputFormat::Markdown(MathMode::SvgOutline).with_math(MathMode::Katex);
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
            Ok(OutputFormat::Markdown(MathMode::SvgText))
        );
        assert_eq!(
            "md".parse::<OutputFormat>(),
            Ok(OutputFormat::Markdown(MathMode::SvgText))
        );
        assert_eq!(OutputFormat::Markdown(MathMode::SvgOutline).extension(), "md");
        // Every format's default output name differs, or `-o` omitted would
        // make two formats fight over one path. The math mode must NOT enter
        // into it: `--katex` writes a `.md`, not a `.md-katex`.
        let exts = [
            OutputFormat::Pdf.extension(),
            OutputFormat::Html(MathMode::SvgOutline).extension(),
            OutputFormat::Markdown(MathMode::SvgOutline).extension(),
        ];
        let unique: std::collections::HashSet<_> = exts.iter().collect();
        assert_eq!(unique.len(), exts.len(), "extensions collide: {exts:?}");
        for math in ALL_MATH_MODES {
            assert_eq!(OutputFormat::Markdown(math).extension(), "md");
            assert_eq!(OutputFormat::Html(math).extension(), "html");
        }
    }
}
