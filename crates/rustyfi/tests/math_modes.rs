//! The three math renderings, end to end through the built binary.
//!
//! The unit tests in `rustyfi-html` cover each writer against hand-built
//! glyphs. What only an end-to-end run can establish is the part that depends
//! on a real font store and a real argv:
//!
//! - that the DEFAULT actually outlines, which needs a face with a MATH table
//!   to be in play — `Junicode.ttf` has none, and without one every glyph
//!   falls back and the assertion would pass for the wrong reason;
//! - that the two flags reach the renderer at all, and that the wrong
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
    let out = work.join(format!("out.{ext}"));
    let result = Command::new(bin())
        .arg(fixture())
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
    assert!(
        !out.exists(),
        "a refused invocation still wrote {}",
        out.display(),
    );
    let _ = std::fs::remove_dir_all(&work);
    String::from_utf8_lossy(&result.stderr).into_owned()
}

// ---------------------------------------------------------------------------
// What each mode writes
// ---------------------------------------------------------------------------

/// The DEFAULT for `--format markdown` is now a drawing, not characters.
///
/// Three things together, because each is worthless alone: the equation is an
/// `<svg>`, its glyphs are real outlines (not a `<text>` naming a face the
/// reader may not have), and the characters survive behind them as phantom
/// text that a search or a copy can still reach.
#[test]
fn markdown_defaults_to_outlined_svg_with_the_characters_kept_behind_it() {
    if !bundled_faces_present() {
        eprintln!(
            "skipping: the bundled faces are absent, so every glyph falls back \
             to naming a font and there is no outline to assert — run \
             download-fonts.sh"
        );
        return;
    }
    let work = tmpdir("md-default");
    let md = render(&work, "md", &["--format", "markdown"]);

    assert!(md.contains("<svg"), "no drawing in the output:\n{md}");
    assert!(
        md.contains("<path d="),
        "the glyphs are not outlines:\n{md}"
    );
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

    // The characters are still there, exactly once each, and inside the
    // phantom layer rather than painted.
    for ch in ['∀', '∃', '∑'] {
        assert_eq!(
            md.matches(ch).count(),
            1,
            "`{ch}` should survive exactly once, as phantom text:\n{md}"
        );
    }
    // …and the phantom layer carries its own invisibility, because a Markdown
    // file has no stylesheet to put the rule in. Without this the characters
    // are painted in black ON TOP of the outlines.
    assert!(
        md.contains("class=\"mphantom\" style=\"fill:none;"),
        "the phantom text would be drawn twice:\n{md}"
    );
}

/// `--unicode-math` is the old default, and stays reachable because it is the
/// only form that survives a renderer which strips HTML.
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
        html.contains("\\(") || html.contains("\\["),
        "HTML must use the delimiters KaTeX/MathJax enable by default:\n\
         no \\( or \\[ found"
    );
    assert!(
        !html.contains("<path d="),
        "--katex should replace the drawing, not add to it"
    );
    let _ = std::fs::remove_dir_all(&work);
}

// ---------------------------------------------------------------------------
// The argv contract
// ---------------------------------------------------------------------------

/// Both flags at once is a usage error, not a last-one-wins.
#[test]
fn the_two_math_flags_are_mutually_exclusive() {
    let err = expect_usage_error(&["--format", "markdown", "--katex", "--unicode-math"]);
    assert!(
        err.contains("--unicode-math") && err.contains("--katex"),
        "the error should name both flags:\n{err}"
    );
}

/// A math flag on `--format pdf` is refused rather than ignored. This is the
/// case that would otherwise be invisible: the PDF is valid and renders, and
/// nothing anywhere says the flag did nothing.
#[test]
fn a_math_flag_without_a_reflowed_format_is_refused() {
    for flag in ["--katex", "--unicode-math"] {
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
