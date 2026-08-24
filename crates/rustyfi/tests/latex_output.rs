//! `--format latex`, end to end and THROUGH A REAL TEX.
//!
//! "It looks like valid LaTeX" is not a test. A `.tex` file has exactly one
//! property worth asserting — that a TeX engine turns it into the document it
//! claims to be — and the ways it can fail to are not ones inspection finds:
//!
//! - a bare `%` truncates the line and STILL COMPILES;
//! - a `Verbatim` or an `itemize` in a `tabular` cell is `Not allowed in LR
//!   mode`, which no amount of reading the cell tells you;
//! - a `tikzpicture` one point too tall for the measure does not overflow, it
//!   makes LaTeX ship an empty page and try again — forever. The first
//!   version of this backend produced 131072 pages from `slydifi` before
//!   `dest_names_size` ran out.
//!
//! Every one of those was found by compiling, and none of them by looking.
//!
//! **Both halves skip loudly.** The engine comes from the flake's own TeX
//! Live (`nix develop`, or `nix build .#tex`), which CI's
//! `build · clippy · test` job does not have — and the CJK fixture needs the
//! bundled faces, which the same job does not fetch either. A skip prints why
//! and returns; it does not pass vacuously, and it does not fail a checkout
//! that is perfectly valid. The structural half of the same behaviour lives
//! in `rustyfi-html/tests/latex.rs` and runs unconditionally.

use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_rustyfi"))
}

fn repo_lib_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lib-rustyfi")
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

/// Whether the faces a CJK document needs are on disk.
///
/// They are fetched by `download-fonts.sh`, and CI runs it for the fidelity
/// and real-package jobs but NOT for `build · clippy · test`. Without them a
/// Japanese document sets nothing, so a test that asserts what came out of
/// the PDF would fail on a checkout that is perfectly valid.
fn bundled_faces_present() -> bool {
    let fonts = repo_lib_root().join("dist/fonts");
    ["ipaexm.ttf", "Junicode.ttf"]
        .iter()
        .all(|f| fonts.join(f).is_file())
}

/// A LaTeX engine, if this machine has one.
///
/// `PATH` first, which is what `nix develop` gives; then the flake's `tex`
/// output built directly, which is faster if only the binaries are wanted.
/// No engine is a SKIP, not a failure: the flake carries one so the check can
/// be made, but a bare `cargo test` on a machine without nix is still a valid
/// thing to run.
fn engine(name: &str) -> Option<PathBuf> {
    if Command::new(name)
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
    {
        return Some(PathBuf::from(name));
    }
    let out = Command::new("nix")
        .args(["build", ".#tex", "--no-link", "--print-out-paths"])
        .current_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|p| Path::new(p.trim()).join("bin").join(name))
        .find(|p| p.is_file())
}

fn tmpdir(tag: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!(
        "rustyfi-latex-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}

/// `fixture` rendered to a `.tex` in `work`, through the built binary.
fn render(fixture_name: &str, work: &Path) -> String {
    let out = work.join("doc.tex");
    let result = Command::new(bin())
        .args(["--format", "latex", "--no-cache", "--lib-root"])
        .arg(repo_lib_root())
        .arg("--font-dir")
        .arg(repo_lib_root())
        .arg(fixture(fixture_name))
        .arg("-o")
        .arg(&out)
        .output()
        .expect("run rustyfi");
    assert!(
        result.status.success(),
        "rustyfi --format latex failed ({:?}):\n{}\n{}",
        result.status.code(),
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr),
    );
    std::fs::read_to_string(&out).expect("read the generated .tex")
}

/// Compile `work/doc.tex` with `engine`, returning the `.log` on failure.
fn compile(engine: &Path, work: &Path) -> Result<PathBuf, String> {
    let out = Command::new(engine)
        .args([
            "-interaction=nonstopmode",
            "-halt-on-error",
            "-file-line-error",
            "doc.tex",
        ])
        .current_dir(work)
        .output()
        .map_err(|e| format!("cannot run {}: {e}", engine.display()))?;
    let pdf = work.join("doc.pdf");
    if out.status.success() && pdf.is_file() {
        return Ok(pdf);
    }
    let log = std::fs::read_to_string(work.join("doc.log")).unwrap_or_default();
    // The first `!` line is TeX's own error; everything before it is package
    // loading and everything after is a memory dump.
    let first = log
        .lines()
        .find(|l| l.starts_with('!') || l.contains(".tex:"))
        .unwrap_or("(no error line in the log)");
    Err(format!(
        "{} failed: {first}\n--- last 40 log lines ---\n{}",
        engine.display(),
        log.lines()
            .rev()
            .take(40)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n"),
    ))
}

/// The PDF's text, with every run of whitespace folded to one space.
///
/// The folding is not tidiness: LaTeX re-broke the paragraph at its own
/// measure, so where a line ends is its decision and an assertion about a
/// phrase would otherwise be an assertion about the line breaker.
fn pdf_text(pdf: &Path) -> String {
    let raw = Command::new("pdftotext")
        .args(["-enc", "UTF-8"])
        .arg(pdf)
        .arg("-")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A document with no CJK in it must compile under ALL THREE engines, which
/// is exactly what its own preamble claims in a comment — so the claim is
/// checked rather than asserted.
///
/// It is also where the escaping is verified against a real typesetter rather
/// than against a string comparison: every character LaTeX reserves is in the
/// fixture, and every one of them has to come back out of the PDF as itself.
/// `100%` is the one that matters most, because getting it wrong compiles
/// cleanly and silently drops the rest of the line.
#[test]
fn a_document_with_no_cjk_compiles_under_every_engine_with_its_specials_intact() {
    let engines: Vec<(&str, PathBuf)> = ["pdflatex", "xelatex", "lualatex"]
        .iter()
        .filter_map(|n| engine(n).map(|p| (*n, p)))
        .collect();
    if engines.is_empty() {
        eprintln!(
            "skipping: no LaTeX engine on PATH and `nix build .#tex` did not \
             produce one — run inside `nix develop`"
        );
        return;
    }
    for (name, path) in &engines {
        let work = tmpdir(name);
        let tex = render("latex-plain.saty", &work);
        assert!(
            tex.contains("% ENGINE: any of pdflatex"),
            "the fixture has no CJK, so it must not demand one engine:\n{tex}"
        );
        let pdf = compile(path, &work).unwrap_or_else(|e| panic!("{e}"));
        let text = pdf_text(&pdf);
        // Every reserved character, back out of the PDF as itself. `100%`
        // first: unescaped it takes the rest of the line with it and the
        // document still compiles, so nothing else here would notice.
        for expected in [
            "100% of the budget",
            "#3 in the series",
            "a & b",
            "x_1",
            "{braces}",
            "~tilde",
            "^caret",
            "back\\slash",
        ] {
            assert!(
                text.contains(expected),
                "{name} lost {expected:?} from the PDF:\n{text}"
            );
        }
        // Both list items are there. Matched on `item` alone rather than on
        // `first item`: under pdfLaTeX's T1 encoding the `fi` in `first` is
        // one LIGATURE glyph with no `ToUnicode` behind it, so `pdftotext`
        // extracts ` rst` — a property of the extractor and the font, not of
        // anything this backend controls.
        assert_eq!(
            text.matches("item").count(),
            2,
            "{name} did not keep both list items:\n{text}"
        );
        assert!(text.contains("After the list."), "{name}:\n{text}");
    }
    eprintln!(
        "compiled under: {}",
        engines
            .iter()
            .map(|(n, _)| *n)
            .collect::<Vec<_>>()
            .join(", ")
    );
}

/// A Japanese document is the case pdfLaTeX genuinely cannot do, so the
/// generated preamble asks for `lualatex` and `luatexja` — and the check that
/// matters is that the characters come back out, not that the file parses.
///
/// The CJK spacing rule is checked at the same time: the box stream puts glue
/// between every pair of CJK characters, and a backend that reads it as a
/// space renders Japanese as `研 究 計 画`.
#[test]
fn a_cjk_document_compiles_under_lualatex_and_keeps_its_characters_together() {
    if !bundled_faces_present() {
        eprintln!(
            "skipping: the bundled CJK faces are absent, so the source document \
             typesets nothing to compare against — run download-fonts.sh"
        );
        return;
    }
    let Some(lualatex) = engine("lualatex") else {
        eprintln!(
            "skipping: no lualatex on PATH and `nix build .#tex` did not produce \
             one — run inside `nix develop`"
        );
        return;
    };
    let work = tmpdir("cjk");
    let tex = render("cjk.saty", &work);
    assert!(tex.contains("\\RequireLuaTeX"), "{tex}");
    let pdf = compile(&lualatex, &work).unwrap_or_else(|e| panic!("{e}"));
    let text = pdf_text(&pdf);
    let cjk: String = text.chars().filter(|c| (*c as u32) >= 0x3000).collect();
    assert!(
        cjk.chars().count() >= 4,
        "no CJK reached the PDF at all:\n{text}"
    );
    // The rule: no space between two CJK characters. Written as "there is a
    // run of at least three of them with nothing between" rather than "no
    // space anywhere", since a Japanese sentence may legitimately contain
    // Latin words with spaces around them.
    let has_run = text
        .split(|c: char| c.is_whitespace())
        .any(|w| w.chars().filter(|c| (*c as u32) >= 0x3000).count() >= 3);
    assert!(
        has_run,
        "every CJK character came out separated — the glue rule is inverted:\n{text}"
    );
}

/// The math modes belong to the reflowed formats. A `.tex` reaches a math
/// typesetter by definition, so it always writes the LaTeX `--katex` asks
/// for; accepting the flag and ignoring it would look exactly like a flag
/// that worked.
#[test]
fn the_math_mode_flags_are_refused_for_latex_rather_than_ignored() {
    for flag in ["--katex", "--unicode-math"] {
        let out = Command::new(bin())
            .args(["--format", "latex", flag, "--no-cache", "--lib-root"])
            .arg(repo_lib_root())
            .arg(fixture("latex-plain.saty"))
            .arg("-o")
            .arg(tmpdir("refuse").join("doc.tex"))
            .output()
            .expect("run rustyfi");
        assert!(
            !out.status.success(),
            "{flag} was accepted for --format latex"
        );
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(
            err.contains("--format latex always writes LaTeX math"),
            "{flag} was refused, but not for the stated reason:\n{err}"
        );
    }
}
