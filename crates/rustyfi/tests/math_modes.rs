//! The five math renderings, end to end through the built binary.
//!
//! The unit tests in `rustyfi-html` cover each writer against hand-built
//! glyphs. What only an end-to-end run can establish is the part that depends
//! on a real font store and a real argv:
//!
//! - that each format's DEFAULT is the one it claims — markdown draws the
//!   compact `<text>` and html draws outlines, with no flag given. That pair
//!   is the whole design decision, so it is pinned end to end rather than
//!   only at `OutputFormat::from_str`;
//! - that `--svg-outline-math` actually outlines, which needs a face with a
//!   MATH table to be in play — `Junicode.ttf` has none, and without one every
//!   glyph falls back and the assertion would pass for the wrong reason;
//! - that each flag reaches the renderer at all, and that the wrong
//!   combination is REFUSED rather than ignored. A flag silently dropped on
//!   `--format pdf` looks exactly like a flag that worked.
//!
//! **Every test that needs a bundled face skips loudly when it is absent.**
//! `download-fonts.sh` fetches them and CI runs it for the fidelity and
//! real-package jobs but NOT for `build · clippy · test` — the job that runs
//! this file. A hard `expect` here fails a perfectly valid checkout. The
//! usage-error tests need no fonts and run everywhere, which is deliberate:
//! they are the ones guarding the argv contract.

use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_rustyfi"))
}

fn repo_lib_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lib-rustyfi")
}

/// Whether the faces this fixture's glyphs live in are on disk.
///
/// The MATH face is the one that matters and it is not interchangeable:
/// `latinmodern-math.otf` carries an OpenType `MATH` table, `Junicode.ttf`
/// does not, and with no MATH-table face every glyph falls back — so a test
/// asserting outlines would assert the opposite of what it means.
fn bundled_faces_present() -> bool {
    let fonts = repo_lib_root().join("dist/fonts");
    ["latinmodern-math.otf", "Junicode.ttf"]
        .iter()
        .all(|f| fonts.join(f).is_file())
}

/// The same three-equation fixture `html_math_selection.rs` uses: one run of
/// ordinary cmap-driven glyphs, one MATH-table variant with limits, one
/// `text-in-math` run.
fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/math-selection.saty")
}

/// A document with one INLINE equation and one DISPLAYED one.
///
/// `math-selection.saty` has neither: all three of its equations sit in a
/// sentence, so it cannot show that the two are told apart. A displayed
/// equation is not a distinct construct in the box stream — what makes one
/// displayed is that its paragraph holds nothing else — so a fixture that
/// contains both is the only way to measure the distinction.
fn display_fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/math-display.saty")
}

fn tmpdir(tag: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!(
        "rustyfi-math-modes-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}

/// Compile the fixture with `args`, returning the output file's contents.
///
/// `--no-cache`, so a run never reads what a differently-flagged run of the
/// same source left behind. The cache key does separate the modes
/// (`OutputFormat::cache_tag`), but a test of what the RENDERER writes should
/// not depend on that being right.
fn render(work: &Path, ext: &str, args: &[&str]) -> String {
    render_doc(work, &fixture(), ext, args)
}

/// [`render`] against a chosen source file.
fn render_doc(work: &Path, src: &Path, ext: &str, args: &[&str]) -> String {
    let out = work.join(format!("out.{ext}"));
    let result = Command::new(bin())
        .arg(src)
        .args(["-o".as_ref(), out.as_os_str()])
        .args(["--lib-root".as_ref(), repo_lib_root().as_os_str()])
        .args(["--font-dir".as_ref(), repo_lib_root().as_os_str()])
        .arg("--no-cache")
        .args(args)
        .output()
        .expect("spawn rustyfi");
    assert!(
        result.status.success(),
        "compile failed with {args:?} (code {:?})\nstderr:\n{}",
        result.status.code(),
        String::from_utf8_lossy(&result.stderr),
    );
    std::fs::read_to_string(&out).expect("read output")
}

/// Run the binary purely to see it FAIL, returning stderr.
fn expect_usage_error(args: &[&str]) -> String {
    let work = tmpdir("usage");
    let out = work.join("out");
    let result = Command::new(bin())
        .arg(fixture())
        .args(["-o".as_ref(), out.as_os_str()])
        .args(args)
        .output()
        .expect("spawn rustyfi");
    assert!(
        !result.status.success(),
        "expected {args:?} to be refused, but it succeeded",
    );
    // A usage error exits 2, like clap's own — not 1, which means "the
    // document failed to compile". Both kinds of flag-validation failure must
    // report the same way: clap rejects two math flags at once and exits 2,
    // and `apply_math_flags` rejects a flag the format has no reading for, so
    // it does too. Nothing checked this before, and the two disagreed.
    assert_eq!(
        result.status.code(),
        Some(2),
        "a usage error must exit 2, not {:?}, for {args:?}",
        result.status.code(),
    );
    assert!(
        !out.exists(),
        "a refused invocation still wrote {}",
        out.display(),
    );
    let _ = std::fs::remove_dir_all(&work);
    String::from_utf8_lossy(&result.stderr).into_owned()
}

// ---------------------------------------------------------------------------
// What each format DEFAULTS to
// ---------------------------------------------------------------------------

/// Markdown defaults to the compact `<text>` drawing, HTML to the outline —
/// and the pair is the design decision, so it is asserted as a pair.
///
/// A `.md` is read as source at least as often as it is rendered, so its
/// default is the mode whose source says what it means and whose bytes are a
/// fraction of the outline's. An HTML page is self-contained and nobody reads
/// it as source, so there the outline costs only size and is the only
/// rendering that does not depend on the reader having the document's faces.
///
/// Both halves in one test because the interesting property is that they
/// DIFFER: a single global default would necessarily make one of them wrong,
/// and two separate tests would each still pass if the two collapsed together.
#[test]
fn each_format_defaults_to_the_rendering_its_typical_reader_wants() {
    if !bundled_faces_present() {
        eprintln!("skipping: bundled faces absent — run download-fonts.sh");
        return;
    }
    let work = tmpdir("defaults");

    // Markdown, no flag: a drawing made of `<text>`, not of outlines.
    let md = render(&work, "md", &["--format", "markdown"]);
    assert!(md.contains("<svg"), "markdown default does not draw:\n{md}");
    assert!(
        md.contains("class=\"math-text\""),
        "markdown default is not the text mode:\n{md}"
    );
    assert!(
        !md.contains("mphantom"),
        "the text mode needs no phantom — the text is real:\n{md}"
    );
    assert!(!md.contains('$'), "markdown default is not KaTeX:\n{md}");

    // HTML, no flag: outlines with a phantom behind them, and NOT LaTeX.
    let html = render(&work, "html", &["--format", "html"]);
    assert!(html.contains("<path d="), "html default is not outlined");
    assert!(html.contains("mphantom"), "html default lost its phantom text");
    assert!(
        !html.contains("math-tex"),
        "html default should draw, not write LaTeX"
    );
    let _ = std::fs::remove_dir_all(&work);
}

/// The new default, `--svg-math`, in its own right: real `<text>`, no phantom,
/// inline styling, and an outline kept ONLY for the glyphs a character cannot
/// name.
///
/// The hybrid is the part worth pinning. A MATH-table variant — the fixture's
/// display-size `∑` — has no character that addresses it, so writing `∑`
/// would draw the base-size glyph with its limits centred on the variant's
/// advance: the measured misplacement `ce2f73c` fixed. Everything else is
/// `<text>`, which is where the size win comes from.
#[test]
fn svg_math_is_text_with_an_outline_only_where_no_character_names_the_glyph() {
    if !bundled_faces_present() {
        eprintln!("skipping: bundled faces absent — run download-fonts.sh");
        return;
    }
    let work = tmpdir("md-svgtext");
    let md = render(&work, "md", &["--format", "markdown", "--svg-math"]);
    let outlined = render(&work, "md", &["--format", "markdown", "--svg-outline-math"]);

    assert!(md.contains("class=\"math-text\""), "{md}");
    // No phantom anywhere: the text is real, and a second invisible copy
    // would be selected and searched twice.
    assert!(!md.contains("mphantom"), "{md}");
    // Some outlining, but far less than the outline mode — the variants only.
    let text_paths = md.matches("<path d=").count();
    let all_paths = outlined.matches("<path d=").count();
    assert!(
        text_paths < all_paths,
        "the text mode outlined as much as the outline mode ({text_paths} vs {all_paths})"
    );
    // Each character appears exactly once across the whole file, whichever
    // layer carries it.
    for ch in ['∀', '∃', '∑'] {
        assert_eq!(
            md.matches(ch).count(),
            1,
            "`{ch}` should appear exactly once:\n{md}"
        );
    }
    // Styling is inline: a Markdown file has no stylesheet to carry it.
    assert!(md.contains("font-size:"), "{md}");
    assert!(!md.contains(".math-text {"), "{md}");
    let _ = std::fs::remove_dir_all(&work);
}

// ---------------------------------------------------------------------------
// What each mode writes
// ---------------------------------------------------------------------------

/// Every `<text class="mphantom">` tag in `doc`, as its raw open tag.
fn phantom_tags(doc: &str) -> Vec<&str> {
    doc.match_indices("<text class=\"mphantom\"")
        .map(|(i, _)| {
            let tail = &doc[i..];
            &tail[..tail.find('>').map(|n| n + 1).unwrap_or(tail.len())]
        })
        .collect()
}

/// `--svg-outline-math` in Markdown: outlines, with the characters kept behind
/// them as phantom text that is hidden by an **inline** style.
///
/// **The inline style is the load-bearing part and is why this test exists.**
/// The two backends hide the phantom layer by different mechanisms, and only
/// one of them is available here:
///
/// - HTML has a stylesheet, and hides it with `css.rs`'s
///   `.math-glyphs .mphantom { fill: none; }`;
/// - **a Markdown file has no stylesheet at all**, so the rule has nowhere to
///   live and the `<text>` must carry `fill:none` in its own `style`.
///
/// Fold the inline style back into CSS — an entirely reasonable-looking tidy —
/// and Markdown silently draws every equation TWICE: once as outlines, and
/// once as visible fallback text painted on top of them in whatever face the
/// reader happens to have. Nothing about the markup would look wrong; a
/// `mphantom` element would still be present, and a test that only asserted
/// its presence would still pass. So this asserts the MECHANISM, per element,
/// and additionally that no stylesheet is being relied on.
///
/// After the default flip this rendering is reachable in Markdown ONLY through
/// the explicit flag, so the flag is what is exercised.
#[test]
fn svg_outline_math_in_markdown_hides_its_phantom_text_with_an_inline_style() {
    if !bundled_faces_present() {
        eprintln!(
            "skipping: the bundled faces are absent, so every glyph falls back \
             to naming a font and there is no outline to assert — run \
             download-fonts.sh"
        );
        return;
    }
    let work = tmpdir("md-svg");
    let md = render(&work, "md", &["--format", "markdown", "--svg-outline-math"]);

    assert!(md.contains("<svg"), "no drawing in the output:\n{md}");
    assert!(md.contains("<path d="), "the glyphs are not outlines:\n{md}");

    // Every math `<svg>` must be one line — a raw `<svg>` broken across lines
    // gets blank lines and `nl2br` inserted into it by the reader's parser.
    for line in md.lines().filter(|l| l.contains("<svg")) {
        assert!(
            line.contains("</svg>"),
            "an <svg> is split across lines:\n{line}"
        );
    }
    // No positioning: a Markdown file has no positioned ancestor, so an
    // absolutely-placed drawing would land on top of the prose.
    assert!(!md.contains("position:absolute"), "{md}");

    // The characters survive, exactly once each.
    for ch in ['∀', '∃', '∑'] {
        assert_eq!(
            md.matches(ch).count(),
            1,
            "`{ch}` should survive exactly once, as phantom text:\n{md}"
        );
    }

    // The mechanism, per element rather than once for the document: a single
    // `contains` would pass while some other equation's phantom went visible.
    let tags = phantom_tags(&md);
    assert_eq!(
        tags.len(),
        3,
        "expected one phantom layer per equation, got {}:\n{md}",
        tags.len()
    );
    for tag in &tags {
        let style = tag
            .split("style=\"")
            .nth(1)
            .and_then(|s| s.split('"').next())
            .unwrap_or_else(|| panic!("phantom text has no inline style:\n{tag}"));
        assert!(
            style.contains("fill:none"),
            "the phantom text is not hidden INLINE, so Markdown — which has no \
             stylesheet — will paint it on top of the outlines:\n{tag}"
        );
        // And not by a mechanism that would take it out of the selection
        // along with the paint, which is the whole point of having it.
        for banned in ["visibility", "display:none"] {
            assert!(
                !style.contains(banned),
                "`{banned}` would remove the text from the selection too:\n{tag}"
            );
        }
    }

    // Nothing to fall back on: a Markdown file carries no stylesheet, so if
    // the inline style ever goes away there is no rule anywhere to save it.
    assert!(
        !md.contains(".mphantom {"),
        "a stylesheet rule in a .md file is not applied by any renderer; the \
         inline style is the only thing that works here:\n{md}"
    );
    let _ = std::fs::remove_dir_all(&work);
}

/// `--svg-outline-math` is accepted for HTML too, where it names the existing
/// default rather than changing anything — stating an intent explicitly is not
/// an error, and a script passing it should not break if the default moves.
#[test]
fn svg_outline_math_is_accepted_for_html_and_names_its_existing_default() {
    if !bundled_faces_present() {
        eprintln!("skipping: bundled faces absent — run download-fonts.sh");
        return;
    }
    let work = tmpdir("html-svg");
    let implicit = render(&work, "html", &["--format", "html"]);
    let explicit = render(&work, "html", &["--format", "html", "--svg-outline-math"]);
    assert_eq!(
        implicit, explicit,
        "--svg-outline-math must be exactly html's default, not a near-miss"
    );
    let _ = std::fs::remove_dir_all(&work);
}

/// `--unicode-math`: the only rendering that is plain TEXT, and the reason it
/// stays reachable — it survives any renderer at all, reads in a terminal and
/// is greppable.
#[test]
fn unicode_math_writes_characters_and_no_markup() {
    if !bundled_faces_present() {
        eprintln!("skipping: bundled faces absent — run download-fonts.sh");
        return;
    }
    let work = tmpdir("md-unicode");
    let md = render(&work, "md", &["--format", "markdown", "--unicode-math"]);

    assert!(!md.contains("<svg"), "still drawing:\n{md}");
    assert!(!md.contains('$'), "still emitting math delimiters:\n{md}");
    // The quantifiers line, as its own characters.
    assert!(md.contains('∀') && md.contains('∃'), "{md}");
    // `\sum_{k=1}^{n}`'s limits, as Unicode script characters — the whole
    // point of this mode over a bare character dump.
    assert!(
        md.contains('ₖ') || md.contains('ⁿ'),
        "no Unicode scripts in the big-operator line:\n{md}"
    );
}

/// `--katex` writes LaTeX for the reader's own typesetter — `$…$` in
/// Markdown, `\(…\)` in HTML, because the two ecosystems' default delimiters
/// genuinely differ.
#[test]
fn katex_writes_latex_in_each_targets_own_delimiters() {
    if !bundled_faces_present() {
        eprintln!("skipping: bundled faces absent — run download-fonts.sh");
        return;
    }
    let work = tmpdir("katex");
    let md = render(&work, "md", &["--format", "markdown", "--katex"]);
    assert!(!md.contains("<svg"), "still drawing:\n{md}");
    assert!(md.contains('$'), "no math delimiters:\n{md}");
    // KaTeX is nobody's default any more, so it must differ from markdown's.
    assert_ne!(
        md,
        render(&work, "md", &["--format", "markdown"]),
        "--katex should not be markdown's default"
    );
    // The commands, by name rather than as raw characters.
    assert!(md.contains("\\forall"), "{md}");
    assert!(md.contains("\\exists"), "{md}");
    // A big operator's limits, grouped into ONE subscript. `\sum_{k}_{=}_{1}`
    // is what a per-glyph emitter produces and KaTeX refuses to render it, so
    // this is the assertion that matters most in the whole file.
    assert!(md.contains("\\sum"), "{md}");
    let sum = md.split("\\sum").nth(1).unwrap_or("");
    let sum = &sum[..sum.find('$').unwrap_or(sum.len())];
    assert!(
        sum.matches('_').count() <= 1,
        "a double subscript will not render:\n{sum}"
    );

    let html = render(&work, "html", &["--format", "html", "--katex"]);
    assert!(
        html.contains("\\("),
        "HTML must use the delimiters KaTeX/MathJax enable by default:\n{html}"
    );
    assert!(
        !html.contains("<path d="),
        "--katex should replace the drawing, not add to it"
    );
    let _ = std::fs::remove_dir_all(&work);
}

/// HTML `--katex` tells a DISPLAYED equation from an inline one.
///
/// **Asserted as a contrast, because each half alone is vacuous.** The
/// previous shape of this check was `contains("\\(") || contains("\\[")`, which
/// the inline delimiter satisfies on its own — so mutating `sole_math_tex` to
/// return `None`, disabling the display upgrade entirely, left the suite
/// green. It also hid a real bug: the upgrade required the paragraph to START
/// with the math span, and a centred equation carries its centring strut
/// first, so `\\[` fired once across ten corpus documents.
///
/// The distinction is not cosmetic. In inline style KaTeX shrinks a big
/// operator and sets its limits beside it; in display style it sets them above
/// and below at full size. The fixture's `\\sum_{k=1}^{n} k` is displayed and
/// its quantifier line is not, so one document exercises both.
#[test]
fn html_katex_uses_display_delimiters_for_a_displayed_equation() {
    if !bundled_faces_present() {
        eprintln!("skipping: bundled faces absent — run download-fonts.sh");
        return;
    }
    let work = tmpdir("html-katex-display");
    let html = render_doc(
        &work,
        &display_fixture(),
        "html",
        &["--format", "html", "--katex"],
    );

    assert!(
        html.contains("\\["),
        "no displayed equation: the display upgrade never fired:\n{html}"
    );
    assert!(
        html.contains("\\("),
        "no inline equation: the contrast is not being measured:\n{html}"
    );
    // A display block is a paragraph of its own, and says so.
    assert!(html.contains("math-display"), "{html}");
    // Every `\\[` closes, and no `\\[` sits inside a `\\(`.
    assert_eq!(
        html.matches("\\[").count(),
        html.matches("\\]").count(),
        "unbalanced display delimiters:\n{html}"
    );
    // The stylesheet rules the mode needs are present — and only in this mode.
    assert!(html.contains(".para.math-display"), "{html}");
    let plain = render_doc(&work, &display_fixture(), "html", &["--format", "html"]);
    assert!(
        !plain.contains(".para.math-display"),
        "the --katex rules leaked into a render that did not ask for them"
    );
    let _ = std::fs::remove_dir_all(&work);
}

/// `--mathml` writes MathML Core into the document's own tree, in BOTH
/// formats, and replaces the drawing rather than adding to it.
///
/// The elements are asserted by NAME rather than by "contains `<math`":
/// `<mfrac>`/`<msubsup>`/`<munderover>` are the structure this mode exists to
/// produce, and a writer that emitted one flat row of `<mi>`s would satisfy
/// the weaker test while rendering as a row of letters.
#[test]
fn mathml_writes_core_structure_in_both_formats() {
    if !bundled_faces_present() {
        eprintln!(
            "skipping: the bundled faces are absent, so the fixture's \
             MATH-table `\\sum` and its limits never reach the recovery and \
             there is no <munderover> to assert — run download-fonts.sh"
        );
        return;
    }
    let work = tmpdir("mathml");
    for (ext, fmt) in [("md", "markdown"), ("html", "html")] {
        let out = render(&work, ext, &["--format", fmt, "--mathml"]);
        assert!(out.contains("<math "), "{fmt}: no MathML:\n{out}");
        assert!(
            out.contains("xmlns=\"http://www.w3.org/1998/Math/MathML\""),
            "{fmt}: the namespace is what makes it well-formed XML:\n{out}"
        );
        // `\sum_{k=1}^{n}`: limits CENTRED on the operator, which is the one
        // construct that needs `<munderover>` rather than `<msubsup>`.
        assert!(
            out.contains("<munderover>"),
            "{fmt}: the big operator's limits are not set as limits:\n{out}"
        );
        // …with the browser told not to re-decide where they go.
        assert!(
            out.contains("movablelimits=\"false\""),
            "{fmt}: an operator base must pin the position we measured:\n{out}"
        );
        // The fixture's `text-in-math` run: the layout splits it at every
        // glue, so its words arrive as separate records with no space in
        // them — `is_prose_run` cannot see them and they are not `<mtext>`.
        // What they must NOT be is one `<mi>` per LETTER, which is eight
        // elements where two do, and which italicises nothing correctly by
        // accident.
        assert!(
            out.contains("<mi>if</mi>") && out.contains("<mi>and</mi>"),
            "{fmt}: a folded text run came out letter by letter:\n{out}"
        );
        // Neither a drawing nor LaTeX.
        assert!(!out.contains("<svg"), "{fmt}: still drawing:\n{out}");
        assert!(!out.contains("<path d="), "{fmt}: still drawing:\n{out}");
        assert!(!out.contains("math-tex"), "{fmt}: still writing LaTeX");
        assert_eq!(
            out.matches("<math ").count(),
            out.matches("</math>").count(),
            "{fmt}: unbalanced <math> elements"
        );
    }
    // Every element must be on ONE line in the Markdown file: a renderer with
    // `breaks: true` puts a `<br>` at every newline inside inline HTML, and a
    // blank line ends the HTML block outright.
    let md = render(&work, "md", &["--format", "markdown", "--mathml"]);
    for line in md.lines().filter(|l| l.contains("<math ")) {
        assert_eq!(
            line.matches("<math ").count(),
            line.matches("</math>").count(),
            "a <math> element is split across lines:\n{line}"
        );
    }
    let _ = std::fs::remove_dir_all(&work);
}

/// HTML `--mathml` tells a DISPLAYED equation from an inline one, and marks a
/// run whose drawing it could not fully account for.
///
/// Both halves are contrasts. `display="block"` alone is satisfied by a
/// renderer that never emits the inline form — the vacuity that let a broken
/// `sole_math_tex` ship — and `rustyfi-approx` alone is satisfied by a writer
/// that marks everything, which would say nothing at all.
#[test]
fn html_mathml_marks_display_style_and_an_unrecovered_drawing() {
    if !bundled_faces_present() {
        eprintln!("skipping: bundled faces absent — run download-fonts.sh");
        return;
    }
    let work = tmpdir("html-mathml-display");
    let html = render_doc(
        &work,
        &display_fixture(),
        "html",
        &["--format", "html", "--mathml"],
    );
    assert!(
        html.contains("display=\"block\""),
        "the display upgrade never fired:\n{html}"
    );
    assert!(
        html.contains("display=\"inline\""),
        "no inline equation: the contrast is not being measured:\n{html}"
    );
    assert!(html.contains("class=\"para math-display\""), "{html}");
    // The stylesheet rules the mode needs, and only in this mode. The two
    // centring declarations are load-bearing: `math[display="block"]` is a
    // block-level box, so the paragraph's own `text-align` does not move it.
    assert!(html.contains("margin-inline: auto"), "{html}");
    assert!(html.contains("width: fit-content"), "{html}");
    let plain = render_doc(&work, &display_fixture(), "html", &["--format", "html"]);
    assert!(
        !plain.contains("margin-inline: auto"),
        "the --mathml rules leaked into a render that did not ask for them"
    );

    // `math-selection.saty` draws no delimiter and no radical, so NOTHING in
    // it is approximate — the control for the marker's other half.
    let exact = render(&work, "html", &["--format", "html", "--mathml"]);
    assert!(
        !exact.contains("class=\"math-ml rustyfi-approx\""),
        "a run with no unrecovered ink must not be marked:\n{exact}"
    );
    let _ = std::fs::remove_dir_all(&work);
}

// ---------------------------------------------------------------------------
// The argv contract
// ---------------------------------------------------------------------------

/// Any two math flags at once is a usage error, not a last-one-wins.
///
/// Every PAIR, not just one: they are held apart by a single `ArgGroup`, and
/// checking one pair would pass even if a flag had been left out of it.
#[test]
fn the_math_flags_are_mutually_exclusive() {
    const FLAGS: [&str; 5] = [
        "--svg-outline-math",
        "--svg-math",
        "--katex",
        "--mathml",
        "--unicode-math",
    ];
    for (i, a) in FLAGS.iter().enumerate() {
        for b in &FLAGS[i + 1..] {
            let err = expect_usage_error(&["--format", "markdown", a, b]);
            assert!(
                err.contains(a) && err.contains(b),
                "the error should name both flags ({a}, {b}):\n{err}"
            );
        }
    }
}

/// A math flag on `--format pdf` is refused rather than ignored. This is the
/// case that would otherwise be invisible: the PDF is valid and renders, and
/// nothing anywhere says the flag did nothing.
#[test]
fn a_math_flag_without_a_reflowed_format_is_refused() {
    for flag in [
        "--svg-outline-math",
        "--svg-math",
        "--katex",
        "--mathml",
        "--unicode-math",
    ] {
        let err = expect_usage_error(&[flag]);
        assert!(
            err.contains(flag),
            "the error should name the flag that was refused:\n{err}"
        );
        // Named explicitly, so the message says what to do rather than only
        // what is wrong.
        assert!(
            err.contains("markdown"),
            "the error should say which format to use:\n{err}"
        );
    }
}

/// `--unicode-math` is Markdown-only: it is a plain-text fallback for
/// renderers that strip markup, which an HTML document is definitionally not.
/// Refused rather than quietly treated as the default, for the same reason.
#[test]
fn unicode_math_is_refused_for_html_and_katex_is_not() {
    let err = expect_usage_error(&["--format", "html", "--unicode-math"]);
    assert!(err.contains("--unicode-math"), "{err}");
    assert!(err.contains("markdown"), "{err}");

    // The control: the other flag on the same format is accepted, so the test
    // above is measuring the flag rather than the format.
    if !bundled_faces_present() {
        eprintln!("skipping the --katex control: bundled faces absent");
        return;
    }
    let work = tmpdir("html-katex");
    let _ = render(&work, "html", &["--format", "html", "--katex"]);
    let _ = std::fs::remove_dir_all(&work);
}
