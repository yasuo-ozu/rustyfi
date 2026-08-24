//! `--format html`: an INLINE frame that draws something draws it.
//!
//! `railway`'s `\uwave` is an `inline-frame-breakable` whose four deco slots
//! stroke a wave in the frame's own bottom padding
//! (`layout-tests/corpus/railway/src/wavyline.satyh:111`), and the whole visual
//! IS that decoration — the framed content is only the text it sits under. It
//! rendered in PDF and vanished in HTML, with no error anywhere, because
//! nothing recorded it: `fire_hooks` wrote a `FrameDecoration` for a BLOCK
//! frame only (`rustyfi-lang/src/lib.rs`'s `fire_block_frame_fragment`), so
//! `DocumentValue::reflow_frame_decos` came out empty on a document whose only
//! decorations were inline, and `inline.rs`'s wrapper had nothing to look up.
//!
//! Both halves are under test here, end to end through the built binary, and
//! that is deliberate: a test that hand-builds a `FrameDecoration` and checks
//! the CSS would pass on the broken tree, because the broken tree's HTML side
//! was never the problem.
//!
//! **The three placement classes are asserted together**, because they are one
//! decision (`reflow::structure::inline_frame_decoration`) and a change that
//! collapses them is exactly the kind that still looks right on one example: a
//! rule TILES at its own width so its period survives the reader's own line
//! breaking, a panel STRETCHES because it is one shape with two ends.
//!
//! **No bundled face is needed and none is used** — the decorations are pure
//! geometry, and `lib-rustyfi/dist/fonts/` is empty where CI runs `cargo test`
//! (only the fidelity, real-package and release jobs run
//! `download-fonts.sh`). Nothing here asserts a measurement, so the assertions
//! hold whether or not the faces happen to be present; there is deliberately
//! no skip, because there is nothing to skip.

use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_rustyfi"))
}

fn repo_lib_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lib-rustyfi")
}

/// One `inline-frame-breakable` per placement class plus one
/// `inline-frame-outer` — see the fixture's own header.
fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/inline-frame-deco.saty")
}

fn tmpdir() -> PathBuf {
    let p = std::env::temp_dir().join(format!(
        "rustyfi-inline-frame-deco-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn render_html() -> String {
    let work = tmpdir();
    let out = work.join("out.html");
    let result = Command::new(bin())
        .arg(fixture())
        .args(["-o".as_ref(), out.as_os_str()])
        .args(["--lib-root".as_ref(), repo_lib_root().as_os_str()])
        .args(["--cache-dir".as_ref(), work.join("cache").as_os_str()])
        .args(["--format", "html", "--no-cache", "--no-aux"])
        .output()
        .expect("failed to spawn rustyfi");
    assert!(
        result.status.success(),
        "rustyfi --format html failed: {}",
        String::from_utf8_lossy(&result.stderr),
    );
    let html = std::fs::read_to_string(&out).expect("no HTML written");
    let _ = std::fs::remove_dir_all(&work);
    html
}

/// The declarations of one `.ideco-N` rule, panicking with the whole
/// stylesheet's `.ideco` lines when it is absent — which is the failure the
/// original bug produces, and the message a reader of it needs.
fn ideco_rule(html: &str, n: usize) -> String {
    let head = format!(".ideco-{n} {{ ");
    let Some(start) = html.find(&head) else {
        let found: Vec<&str> = html.lines().filter(|l| l.contains(".ideco")).collect();
        panic!(
            "no `.ideco-{n}` rule in the stylesheet — an inline frame's \
             decoration was not recorded or not replayed. `.ideco` lines \
             present: {found:?}"
        );
    };
    let rest = &html[start + head.len()..];
    let end = rest.find('}').expect("unterminated CSS rule");
    rest[..end].to_string()
}

/// The decoded SVG document behind one `.ideco-N` rule's `background-image`.
fn ideco_svg(html: &str, n: usize) -> String {
    let rule = ideco_rule(html, n);
    let marker = "background-image:url(\"data:image/svg+xml,";
    let start = rule.find(marker).unwrap_or_else(|| {
        panic!("`.ideco-{n}` carries no `data:image/svg+xml` background: {rule}")
    }) + marker.len();
    let rest = &rule[start..];
    let end = rest.find('"').expect("unterminated data URI");
    percent_decode(&rest[..end])
}

/// The inverse of `svg::svg_data_uri` — enough of one to read the five
/// escapes it writes back.
fn percent_decode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(b as char);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

/// The wavy-underline shape: an inline frame whose deco strokes a path below
/// the baseline reaches the HTML as a real drawing, in the colour the document
/// asked for, on a wrapper around the text it decorates.
///
/// This is the assertion the bug fails: before the fix the wrapper is a bare
/// `<span class="iframe">` and the stylesheet has no `.ideco` line at all.
#[test]
fn an_inline_frame_that_strokes_below_the_baseline_draws_in_html() {
    let html = render_html();

    assert!(
        // `Before ` and not `Before` + a space INSIDE the wrapper: the word
        // space in front of a decorated region belongs outside it, or the
        // decoration's background box starts one space too far left (see
        // `Ctx::resolve_glue_before_wrapper`).
        html.contains("Before <span class=\"iframe ideco ideco-0\">underlined phrase</span>"),
        "the decorated region's own wrapper is missing or undecorated; body: {}",
        body_of(&html),
    );

    let svg = ideco_svg(&html, 0);
    assert!(
        svg.contains("<path ") && svg.contains("stroke=\"rgb(217,26,26)\""),
        "the recorded decoration is not the document's own stroked path: {svg}",
    );
    // A drawing sized to the box it came out of, not to nothing: the crop is
    // the ink's bounding box, and a degenerate one would render as a blank.
    assert!(
        svg.contains("preserveAspectRatio=\"none\""),
        "without this the SVG is centred at natural size inside the background \
         box instead of filling it: {svg}",
    );
}

/// The three placement classes, and which of them tiles.
///
/// A rule is a pattern along x, so it repeats at its own recorded width and
/// keeps its period whatever width the reader's line turns out to be; a panel
/// is one shape and is stretched. Getting this backwards is invisible on a
/// short single-line example and ruins both on a long one.
#[test]
fn a_rule_tiles_at_its_own_width_and_a_panel_stretches() {
    let html = render_html();

    let ruled = ideco_rule(&html, 0);
    assert!(
        ruled.contains("background-repeat:repeat-x;"),
        "a rule must tile, or its period is scaled by the reader's line width: {ruled}",
    );
    assert!(
        ruled.contains("background-position:left bottom;"),
        "ink below the baseline is an underline and anchors to the box's bottom: {ruled}",
    );
    assert!(
        !ruled.contains("background-size:100%"),
        "a tiled rule is sized in points, not to the box: {ruled}",
    );

    for n in [1, 2] {
        let panel = ideco_rule(&html, n);
        assert!(
            panel.contains("background-size:100% 100%;")
                && panel.contains("background-repeat:no-repeat;"),
            "ink straddling the baseline is a panel and is stretched over the box: {panel}",
        );
    }
}

/// Every inline frame that DREW is replayed — registered against fired, the
/// count being the part a walker that skips one still passes without.
///
/// Three commands draw in the fixture and the third goes through
/// `inline-frame-outer`, the NON-breakable primitive, which is a different
/// firing site in `fire_hooks` (`fire_inline_frame`, not
/// `fire_inline_frame_fragment`) and was equally blank before.
#[test]
fn every_drawing_inline_frame_is_replayed_including_the_non_breakable_one() {
    let html = render_html();

    // REGISTERED against FIRED: one stylesheet rule per decoration, and one
    // wrapper naming it. Either count alone passes a walk that reaches an
    // inline frame's marker but never its contents, or one that records a
    // decoration nothing then wears.
    let rules = html.matches(".ideco-").count();
    let wrappers = html.matches("class=\"iframe ideco ideco-").count();
    assert_eq!(
        (rules, wrappers),
        (3, 3),
        "expected 3 inline decorations, each with one rule and one wrapper; \
         mentions: {:?}",
        html.lines()
            .filter(|l| l.contains("ideco-"))
            .map(|l| l.chars().take(120).collect::<String>())
            .collect::<Vec<_>>(),
    );

    assert!(
        html.contains("Before <span class=\"iframe ideco ideco-2\">boxed phrase</span>"),
        "the non-breakable `inline-frame-outer` lost its decoration; body: {}",
        body_of(&html),
    );
    let svg = ideco_svg(&html, 2);
    assert!(
        svg.contains("fill=\"rgb(242,217,102)\""),
        "the non-breakable frame's recorded drawing is not the document's: {svg}",
    );
}

/// An inline frame that draws NOTHING still draws nothing. `\href` is the
/// overwhelmingly common inline frame and its deco is empty; a decoration
/// mechanism that gave every one of them a background would put a box round
/// every link in every document.
#[test]
fn an_inline_frame_with_an_empty_decoration_stays_bare() {
    let href = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/href.saty");
    let work = tmpdir();
    let out = work.join("out.html");
    let result = Command::new(bin())
        .arg(&href)
        .args(["-o".as_ref(), out.as_os_str()])
        .args(["--lib-root".as_ref(), repo_lib_root().as_os_str()])
        .args(["--cache-dir".as_ref(), work.join("cache").as_os_str()])
        .args(["--format", "html", "--no-cache", "--no-aux"])
        .output()
        .expect("failed to spawn rustyfi");
    assert!(
        result.status.success(),
        "rustyfi --format html failed on href.saty: {}",
        String::from_utf8_lossy(&result.stderr),
    );
    let html = std::fs::read_to_string(&out).expect("no HTML written");
    let _ = std::fs::remove_dir_all(&work);

    assert!(
        html.contains("<a class=\"link\" href=\"https://example.com/\">"),
        "the link itself regressed; body: {}",
        body_of(&html),
    );
    // The `.ideco` rule itself is static stylesheet furniture and is always
    // there; what must not exist is a per-decoration `.ideco-N` rule or a
    // wrapper wearing one.
    assert!(
        !html.contains(".ideco-") && !html.contains("iframe ideco"),
        "an empty decoration must register no rule and no class; body: {}",
        body_of(&html),
    );
}

/// The `<body>` of `html`, for a failure message that is readable — the
/// stylesheet is several kilobytes of data URI.
fn body_of(html: &str) -> String {
    match html.find("<body>") {
        Some(i) => html[i..].to_string(),
        None => html.to_string(),
    }
}
