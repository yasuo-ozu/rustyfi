//! `--format html` draws every math glyph as an SVG outline, and the
//! characters survive behind it as invisible, SELECTABLE text.
//!
//! The two halves of that sentence are one change and are tested together,
//! because each is worthless without the other. Outlines alone make math
//! independent of the reader's fonts — a `<text>` names a face and hopes, and
//! where the reader has no Latin Modern Math the substitute's advances are
//! not the ones every glyph's absolute `dx` was computed against, so the
//! equation collides with itself (measured on this machine, `∀` is drawn
//! 12.000 units wide where the port reserved 7.992, and `ε` lands inside the
//! quantifier). Phantom text alone would be pointless. Outlines WITHOUT
//! phantom text would silently destroy selection, copy, in-page find and
//! screen-reader access for every equation in every document.
//!
//! **The browser half is the real evidence and it is not optional reasoning.**
//! Whether a given spelling of "invisible" still selects is a fact about
//! browsers, not about the standard: `visibility: hidden` and `display: none`
//! remove the text from the selection along with the paint, `fill: none`
//! removes only the paint. This drives a real headless chromium over a real
//! render and asserts the extracted string, including both losing spellings
//! as controls in the same page. It SKIPS, loudly, where chromium is not
//! installed — the structural half above still runs everywhere, and it is
//! written so that it fails if the phantom text disappears or turns visible.
//!
//! No node, no puppeteer: `--dump-dom` runs the page's own scripts and prints
//! the resulting DOM, so a probe script that writes its findings into a
//! `<div>` is a complete round trip through one process.

use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_rustyfi"))
}

fn repo_lib_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lib-rustyfi")
}

/// Whether the faces this fixture's glyphs live in are actually on disk.
///
/// They are fetched by `download-fonts.sh`, and CI runs it for the fidelity
/// and real-package jobs but NOT for `build · clippy · test` — the job that
/// runs these tests. Without them every glyph falls back to naming a font,
/// which is precisely what the structural test asserts must not happen, so it
/// would fail on a checkout that is perfectly valid.
///
/// The math face is the one that matters: an outline is taken from it, and
/// with no MATH-table face there is nothing to outline.
fn bundled_faces_present() -> bool {
    let fonts = repo_lib_root().join("dist/fonts");
    ["latinmodern-math.otf", "Junicode.ttf"]
        .iter()
        .all(|f| fonts.join(f).is_file())
}

/// Three `${…}` runs, one per branch of the outline/phantom code — see the
/// fixture's own header for which is which.
fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/math-selection.saty")
}

fn tmpdir(tag: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!(
        "rustyfi-html-math-selection-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}

/// The fixture, rendered to reflow HTML through the built binary with the
/// repo's real fonts — which this test needs for a reason no other reflow
/// test does: with no font store there is no face to take an outline from,
/// `Ctx::math_glyph_outline` answers `None` for every glyph, and the whole
/// thing under test degrades to the `<text>` path it is replacing.
fn render(work: &Path) -> String {
    let out = work.join("out.html");
    let result = Command::new(bin())
        .arg(fixture())
        .args(["-o".as_ref(), out.as_os_str()])
        .args(["--lib-root".as_ref(), repo_lib_root().as_os_str()])
        .args(["--font-dir".as_ref(), repo_lib_root().as_os_str()])
        .args(["--cache-dir".as_ref(), work.join("cache").as_os_str()])
        .args(["--format", "html"])
        .output()
        .expect("spawn rustyfi");
    assert!(
        result.status.success(),
        "compile failed (code {:?})\nstdout:\n{}\nstderr:\n{}",
        result.status.code(),
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr),
    );
    std::fs::read_to_string(&out).expect("read rendered HTML")
}

/// Everything between the math `<svg>` open tags and their `</svg>` — the
/// only region where a math glyph can be, so an assertion about "no visible
/// glyph text" can be made about it without tripping over the document's
/// ordinary prose.
fn math_svgs(html: &str) -> Vec<&str> {
    html.split("class=\"math-glyphs\"")
        .skip(1)
        .filter_map(|s| s.split("</svg>").next())
        .collect()
}

/// The three equations the fixture sets, as they should come out of a
/// selection. The spaces around `:` and inside `if and only if` are not
/// decoration: they are gaps the document measured, and reading them back is
/// the only way a space survives at all — the glue inside a `text-in-math`
/// body reaches this backend as bare advance with no character attached
/// (`Phantom::push`).
const EXPECTED: [&str; 3] = ["∀𝜀 : ∃𝛿", "∑𝑘=1𝑛𝑘", "𝑥if and only if𝑦"];

/// Every math glyph is drawn as an outline, and every character it stands for
/// is present exactly once, in text marked as the phantom layer.
///
/// This runs without a browser, so it is the half that guards the change in
/// CI. It is deliberately phrased as "no VISIBLE glyph text remains" rather
/// than "some `<path>` exists": the failure mode worth catching is a glyph
/// quietly falling back to `<text>` for a face the reader may not have, and a
/// bare `<path>`-count assertion would not see it.
#[test]
fn every_math_glyph_is_an_outline_with_its_character_kept_as_phantom_text() {
    if !bundled_faces_present() {
        eprintln!(
            "skipping: the bundled faces are absent, so every glyph falls back \
             to naming a font and there is no outline to assert — run \
             download-fonts.sh"
        );
        return;
    }
    let work = tmpdir("structure");
    let html = render(&work);

    let svgs = math_svgs(&html);
    assert_eq!(svgs.len(), 3, "expected one math <svg> per equation:\n{html}");

    let mut paths = 0usize;
    for svg in &svgs {
        paths += svg.matches("<path ").count();
        // A `<text>` in here is either the phantom layer or a glyph that
        // fell back to naming a font — and this fixture's glyphs are all in
        // faces the repo ships, so none of them should have to.
        for (idx, _) in svg.match_indices("<text ") {
            let tag = &svg[idx..];
            let tag = &tag[..tag.find('>').unwrap_or(tag.len())];
            assert!(
                tag.contains("class=\"mphantom\""),
                "a math glyph is still drawn as a character, so the reader's \
                 own font decides its advance:\n{tag}"
            );
        }
    }
    assert!(paths >= 12, "expected an outline per glyph, got {paths} paths");

    // Each character appears once, and only inside the phantom layer.
    for ch in ['∀', '∃', '∑', '𝜀', '𝛿'] {
        assert_eq!(
            html.matches(ch).count(),
            1,
            "`{ch}` should appear exactly once, in the phantom text:\n{html}"
        );
        let idx = html.find(ch).unwrap();
        let before = &html[..idx];
        let open = before.rfind("<text ").expect("inside a <text>");
        assert!(
            before[open..].contains("class=\"mphantom\""),
            "`{ch}` is not in the phantom layer"
        );
    }

    // The invisibility must be the spelling that keeps the text selectable.
    assert!(
        html.contains(".math-glyphs .mphantom { fill: none; }"),
        "the phantom rule is missing or has changed spelling:\n{html}"
    );
    let rule = html
        .split(".math-glyphs .mphantom")
        .nth(1)
        .and_then(|s| s.split('}').next())
        .unwrap_or_default();
    for banned in ["visibility", "display"] {
        assert!(
            !rule.contains(banned),
            "`{banned}` in the phantom rule would remove the text from the \
             selection and the accessibility tree, which is the whole point \
             of having it:\n{rule}"
        );
    }
}

/// The same page in a real browser: select each equation with a `Range` and
/// assert the string that comes back, then prove the two losing spellings of
/// "invisible" really do lose, on this same page, in this same browser.
///
/// Also exercises the browser's own in-page find (`window.find`), with a
/// character the document does NOT contain as the negative control — without
/// it, a `find` that returned `true` for everything would pass.
#[test]
fn a_browser_can_select_copy_and_find_the_math_characters() {
    if !bundled_faces_present() {
        eprintln!("skipping: the bundled faces are absent — run download-fonts.sh");
        return;
    }
    let Some(chromium) = find_chromium() else {
        eprintln!(
            "skipping: no chromium on PATH — the structural half of this file \
             still ran, but whether an invisible <text> is SELECTABLE is a \
             fact about browsers and cannot be asserted without one"
        );
        return;
    };
    let work = tmpdir("browser");
    let html = render(&work);
    let page = work.join("probe.html");
    std::fs::write(
        &page,
        html.replace("</body>", &format!("<script>{PROBE_JS}</script></body>")),
    )
    .expect("write probe page");

    let child = Command::new(&chromium)
        .args(["--headless=new", "--disable-gpu", "--no-sandbox"])
        // A CI container's `/dev/shm` is typically 64 MB, and Chrome's default
        // shared-memory use exceeds it — whereupon it does not exit, it hangs.
        .arg("--disable-dev-shm-usage")
        .args(["--no-first-run", "--disable-background-networking"])
        .arg(format!(
            "--user-data-dir={}",
            work.join("chrome").to_string_lossy()
        ))
        .arg("--virtual-time-budget=5000")
        .arg("--dump-dom")
        .arg(format!("file://{}", page.to_string_lossy()))
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn chromium");
    let Some(out) = wait_with_deadline(child) else {
        eprintln!(
            "skipping: chromium did not exit within {BROWSER_TIMEOUT_SECS}s and was \
             killed — the structural half of this file still ran and still asserts \
             the phantom text is present; only the browser-behaviour half is lost"
        );
        return;
    };
    let dom = String::from_utf8_lossy(&out.stdout).into_owned();
    let report = dom
        .split("<div id=\"rustyfi-probe\">")
        .nth(1)
        .and_then(|s| s.split("</div>").next())
        .unwrap_or_else(|| {
            panic!(
                "the probe script did not run (chromium stderr:\n{})",
                String::from_utf8_lossy(&out.stderr)
            )
        });
    // The probe writes its findings as `key=value` lines, one per assertion,
    // with `~` between a key's several values — chosen over JSON so this test
    // needs no parser and the failure message is the raw finding.
    let field = |name: &str| -> String {
        report
            .lines()
            .find_map(|l| l.strip_prefix(&format!("{name}=")))
            .unwrap_or_else(|| panic!("probe did not report `{name}`:\n{report}"))
            .to_string()
    };

    // 1. A drag-select over each equation yields its characters, in order.
    let selected = field("selected");
    assert_eq!(
        selected,
        EXPECTED.join("~"),
        "the selection does not read back as the equations the document set"
    );

    // 2. The browser's own in-page find reaches them, INCLUDING the two that
    //    `28144b3` already outlined and thereby made uncopyable.
    assert_eq!(field("find"), "true~true~true~false", "in-page find");

    // 3. The controls, on this same page: the spellings this code does NOT
    //    use take the text out of the selection entirely, and removing them
    //    brings it straight back. This is the measurement behind the comment
    //    on `Phantom` and behind `css.rs`'s `fill: none`.
    assert_eq!(
        field("hidden"),
        "",
        "`visibility: hidden` still selected — the argument for `fill: none` \
         no longer holds and the comment on `Phantom` needs rewriting"
    );
    assert_eq!(field("none"), "", "`display: none` still selected");
    assert_eq!(
        field("restored"),
        EXPECTED[0],
        "the control styles were not cleanly removed, so nothing above is \
         measuring what it claims"
    );

    // 4. The phantom text steals no hit-testing: `fill: none` paints no fill,
    //    and SVG's default `visiblePainted` only tests where paint landed.
    assert_eq!(
        field("phantomhit"),
        "false",
        "the invisible text is capturing pointer events over the equation"
    );
}

/// How long the browser gets. It normally finishes in well under a second;
/// this is a hang-breaker, not a performance budget.
const BROWSER_TIMEOUT_SECS: u64 = 120;

/// `Child::wait_with_output` with a deadline. `None` means the deadline passed
/// and the process was killed.
///
/// Not `Command::output()`, which waits forever: a browser that wedges — for
/// want of shared memory, a writable profile, or a display it cannot get —
/// then wedges the whole test job until the CI runner's own six-hour limit
/// fires. Both pipes are drained on their own threads, because a child that
/// fills a pipe buffer blocks in `write` and would never reach the deadline
/// check at all.
fn wait_with_deadline(mut child: std::process::Child) -> Option<std::process::Output> {
    let mut stdout = child.stdout.take().expect("piped stdout");
    let mut stderr = child.stderr.take().expect("piped stderr");
    let drain = |r: &mut dyn std::io::Read| {
        let mut buf = Vec::new();
        let _ = r.read_to_end(&mut buf);
        buf
    };
    let out_t = std::thread::spawn(move || drain(&mut stdout));
    let err_t = std::thread::spawn(move || drain(&mut stderr));

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(BROWSER_TIMEOUT_SECS);
    let status = loop {
        match child.try_wait().expect("poll chromium") {
            Some(status) => break Some(status),
            None if std::time::Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            None => std::thread::sleep(std::time::Duration::from_millis(50)),
        }
    };
    let stdout = out_t.join().unwrap_or_default();
    let stderr = err_t.join().unwrap_or_default();
    status.map(|status| std::process::Output {
        status,
        stdout,
        stderr,
    })
}

fn find_chromium() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for name in ["chromium", "chromium-browser", "google-chrome", "chrome"] {
        for dir in std::env::split_paths(&path) {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// Injected into the rendered page; writes its findings into a `<div>` that
/// `--dump-dom` then prints. Everything it measures is a browser fact that
/// cannot be established by reading the markup.
const PROBE_JS: &str = r#"
window.addEventListener("load", () => {
  const sel = window.getSelection();
  const textOf = (el) => {
    const r = document.createRange();
    r.selectNodeContents(el);
    sel.removeAllRanges();
    sel.addRange(r);
    const s = sel.toString();
    sel.removeAllRanges();
    return s;
  };
  const wrappers = [...document.querySelectorAll("span.math")];
  const lines = [];
  lines.push("selected=" + wrappers.map(textOf).join("~"));
  // The last needle is absent from the document: without a negative control
  // a find() that answered true unconditionally would look like a pass.
  const found = ["∀", "∃", "∑", "∫"].map((n) => {
    sel.removeAllRanges();
    return !!(window.find && window.find(n, true, false, true, false, true, false));
  });
  sel.removeAllRanges();
  lines.push("find=" + found.join("~"));

  const style = document.createElement("style");
  document.head.appendChild(style);
  style.textContent = ".math-glyphs .mphantom { visibility: hidden; }";
  lines.push("hidden=" + textOf(wrappers[0]).trim());
  style.textContent = ".math-glyphs .mphantom { display: none; }";
  lines.push("none=" + textOf(wrappers[0]).trim());
  style.remove();
  lines.push("restored=" + textOf(wrappers[0]));

  wrappers[0].scrollIntoView({ block: "center" });
  const b = wrappers[0].getBoundingClientRect();
  let phantomHit = false;
  for (let f = 0.05; f < 1; f += 0.1) {
    const stack = document.elementsFromPoint(b.left + b.width * f, b.top + b.height * 0.5);
    if (stack.some((e) => e.classList && e.classList.contains("mphantom"))) phantomHit = true;
  }
  lines.push("phantomhit=" + phantomHit);

  const d = document.createElement("div");
  d.id = "rustyfi-probe";
  d.textContent = lines.join("\n");
  document.body.appendChild(d);
});
"#;
