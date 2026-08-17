//! PDF image-comparison (visual-regression) test.
//! `docs/plans/design-pdf-image-diff-ci.md`.
//!
//! Rasterizes the PDFs this port produces with poppler's `pdftoppm` and
//! pixel-compares each page against a committed golden PNG. This is the
//! blind spot `tests/layout_snapshot.rs` (geometry-only, `Base14Metrics`
//! only, no glyph shapes/colors) and `tests/e2e.rs`'s `pdftotext`-substring
//! assertions (text content only, no positions/glyph-shapes/colors) both
//! share: neither would notice a wrong fill color, a mis-embedded glyph, or
//! a shifted baseline as long as the same *words* still extract somewhere.
//!
//! ## Why `#[ignore]`d
//!
//! This test shells out to `pdftoppm` (poppler-utils) and, for the
//! embedded-font fixtures, needs `scripts/download-fonts.sh`'s fonts on
//! disk. Neither is guaranteed on a bare dev box, so — mirroring the
//! `find_regular_ttf`/`fc-match` skip discipline `tests/e2e.rs` and
//! `tests/fonts.rs` already use — this test is `#[ignore]`d (so
//! `cargo test --workspace` never runs it) AND self-skips per-prerequisite
//! (`find_pdftoppm`, `fonts_present`) with an `eprintln!`, never a failure,
//! when run with `--ignored` on an underprovisioned box.
//!
//! ## Determinism / the poppler-pinning discipline (design §3/§4/§6)
//!
//! The PDF bytes this port emits are byte-deterministic (no timestamps, no
//! `/ID`, fixed-seed subset tags — see `crates/rustyfi-pdf/tests/ttf.rs`'s
//! `subsetting_is_reproducible_across_reruns`), so the ONLY source of
//! cross-environment drift for a rasterized golden is the `pdftoppm`
//! (poppler+freetype) version itself. Consequently:
//!
//! - Goldens under `tests/golden_images/` are valid ONLY for the poppler
//!   version they were authored under. **This run's goldens were authored
//!   against `pdftoppm version 25.10.0`** (poppler-utils, logged by the CI
//!   job before it runs this test — see `.github/workflows/ci.yml`'s
//!   `image-diff` job). A poppler bump in the CI runner image is a
//!   deliberate golden refresh (`UPDATE_GOLDEN_IMAGES=1`), not a mystery
//!   flake.
//! - Every fixture is rendered through the SAME font path the CLI would use
//!   in production for that configuration — Base14 for fixtures whose
//!   surface is layout/color/graphics (poppler's substitute Helvetica is
//!   then a fixed function too, given a pinned poppler), and the real
//!   embedded/subsetted TrueType(`FontFile2`)/CFF(`FontFile3`) path for the
//!   fixtures whose whole point is glyph-shape/embedding coverage.
//!
//! ## Update mechanism
//!
//! `UPDATE_GOLDEN_IMAGES=1` (re)writes the golden PNGs from the current
//! render — mirrors `tests/layout_snapshot.rs`'s `UPDATE_SNAPSHOTS` idiom
//! exactly, under a distinct env var name so a routine snapshot refresh
//! doesn't silently re-bless pixels too:
//!
//! ```text
//! UPDATE_GOLDEN_IMAGES=1 cargo test -p rustyfi-cli --test pdf_image_diff -- --ignored
//! ```
//!
//! Goldens should only ever be (re)authored on the pinned-poppler CI image
//! (or a container matching it) — never on an arbitrary dev laptop, per the
//! design's §4/§6 poppler-pinning discipline.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;

use rustyfi_backend::FontMetrics;
use rustyfi_lang::value::DocumentValue;
use rustyfi_pdf::{Base14Metrics, FontFlags, FontRegistry, TtfFontStore};

// ---------------------------------------------------------------------
// Tolerance constants (design §4/§5.3) — tune here, nowhere else.
// ---------------------------------------------------------------------

/// A pixel "differs" if any RGBA channel's absolute delta exceeds this, on a
/// 0..=255 scale. Absorbs anti-aliasing edge jitter between reruns of the
/// same pinned poppler.
const PER_PIXEL_CHANNEL_TOL: i16 = 8;

/// The test fails if the fraction of differing pixels (by the per-pixel
/// rule above) exceeds this. 0.1%: small enough that a real regression (a
/// recolored fill, a swapped glyph, a shifted line) trips it, large enough
/// to absorb residual AA jitter along glyph/path edges.
const MAX_DIFF_FRACTION: f64 = 0.001;

/// `pdftoppm -r <DPI>` — moderate: small PNGs, still resolves glyph shapes.
const DPI: &str = "96";

// ---------------------------------------------------------------------
// Paths.
// ---------------------------------------------------------------------

fn repo_lib_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lib-rustyfi")
}

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden_images")
}

/// Failure artifacts (actual + diff PNGs), NOT committed — lives under the
/// workspace `target/` dir, which the repo root already `.gitignore`s
/// wholesale (`/target`).
fn artifacts_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/golden_images_out")
}

fn dist_fonts_dir() -> PathBuf {
    repo_lib_root().join("dist/fonts")
}

// ---------------------------------------------------------------------
// Gating: poppler + fonts.
// ---------------------------------------------------------------------

/// Probe for `pdftoppm` exactly like `tests/e2e.rs`'s inlined `pdftotext`
/// probe (`Command::new(...).output()`, check `status.success()`).
fn find_pdftoppm() -> Option<PathBuf> {
    match Command::new("pdftoppm").arg("-v").output() {
        Ok(out) if out.status.success() => Some(PathBuf::from("pdftoppm")),
        _ => None,
    }
}

/// The `download-fonts.sh` outputs the embedded-font fixtures need,
/// checked by hardcoded relative path from `CARGO_MANIFEST_DIR` — same
/// idiom as `tests/cjk_render.rs`'s `ipaexm_path()`.
fn fonts_present() -> bool {
    dist_fonts_dir().join("lmsans10-regular.otf").is_file()
        && dist_fonts_dir().join("Junicode.ttf").is_file()
        && dist_fonts_dir().join("ipaexm.ttf").is_file()
}

// ---------------------------------------------------------------------
// Compiling a fixture (mirrors tests/e2e.rs's / tests/layout_snapshot.rs's
// load_and_merge / compile_v006_fixture — duplicated here per this test
// suite's own convention of a self-contained file).
// ---------------------------------------------------------------------

fn as_v006(cst: rustyfi_loader::LoadedCst) -> rustyfi_syntax::cst::File {
    match cst {
        rustyfi_loader::LoadedCst::V0_0_6(f) => f,
        rustyfi_loader::LoadedCst::V0_1(_) => {
            unreachable!("this test's v006 load path is V0_0_6-only")
        }
    }
}

fn load_and_merge_v006(name: &str) -> rustyfi_syntax::cst::File {
    let entry = fixtures_dir().join(name);
    let program = rustyfi_loader::load(
        &entry,
        &rustyfi_loader::LoadOptions {
            lib_root: Some(repo_lib_root()),
            ..Default::default()
        },
    )
    .unwrap_or_else(|e| panic!("failed to load {}: {e}", entry.display()));

    let mut files = program.files;
    let entry_file = files.pop().expect("loader always yields the entry last");
    let entry_cst = as_v006(entry_file.cst);
    let mut prelude = Vec::new();
    for lib in files {
        prelude.extend(as_v006(lib.cst).prelude);
    }
    prelude.extend(entry_cst.prelude);
    rustyfi_syntax::cst::File {
        headers: Vec::new(),
        prelude,
        in_kw: entry_cst.in_kw,
        body: entry_cst.body,
        eoi: entry_cst.eoi,
    }
}

fn compile_v006(name: &str, metrics: &dyn FontMetrics) -> Rc<DocumentValue> {
    let merged = load_and_merge_v006(name);
    rustyfi_lang::compile_document_cst(&merged, metrics)
        .unwrap_or_else(|e| panic!("{name} must compile: {e}"))
}

/// Some fixtures' parse trees (via `@require: list`, or the full v0.1
/// spine) overflow the default 8 MiB test-thread stack through syan's
/// recursive-descent parser — the exact same reason `tests/e2e.rs`'s
/// `compile_table_fixture`/`v01_math_document_renders_to_extractable_pdf`
/// spawn a bigger-stack thread for these same fixtures. `Vec<u8>` (the
/// final PDF bytes) is `Send`, so the whole compile-and-render runs on that
/// thread and only the rendered bytes join back out — `Rc<DocumentValue>`
/// itself never crosses the thread boundary.
fn run_with_big_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(f)
        .expect("spawn big-stack thread")
        .join()
        .expect("big-stack thread panicked (see assertion above)")
}

fn compile_v01(name: &str, metrics: &dyn FontMetrics) -> Rc<DocumentValue> {
    let entry = fixtures_dir().join(name);
    let program = rustyfi_loader::load(
        &entry,
        &rustyfi_loader::LoadOptions {
            lib_root: Some(repo_lib_root()),
            version: rustyfi_syntax::RustyfiVersion::V0_1,
            ..Default::default()
        },
    )
    .unwrap_or_else(|e| panic!("failed to load {}: {e}", entry.display()));
    rustyfi_lang::compile_document_v1(&program.files, metrics)
        .unwrap_or_else(|e| panic!("{name} must compile: {e}"))
}

/// Render through the Base14 path — same call the CLI makes with no font
/// configured (`main.rs`'s `None` arm).
fn render_base14(doc: &DocumentValue) -> Vec<u8> {
    rustyfi_pdf::render_pdf_with(&doc.geometry, &doc.pages, &doc.images, &doc.extras)
        .expect("PDF rendering (base14) must succeed")
}

/// Render through the embedded/subsetted TrueType-or-CFF path — same call
/// the CLI makes once a font is configured (`main.rs`'s `Some(store)` arm;
/// design §3's "Critical caveat" — this is the ONLY self-contained,
/// environment-independent rasterization path for glyph-shape coverage).
fn render_ttf(doc: &DocumentValue, store: &TtfFontStore) -> Vec<u8> {
    rustyfi_pdf::render_pdf_ttf_with(&doc.geometry, &doc.pages, store, &doc.images, &doc.extras)
        .expect("PDF rendering (ttf) must succeed")
}

// ---------------------------------------------------------------------
// Rasterization.
// ---------------------------------------------------------------------

fn tmpdir(tag: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!(
        "rustyfi-rust-image-diff-{tag}-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}

/// Write `pdf_bytes` to a temp file, rasterize with `pdftoppm -png -r 96
/// -aa yes -aaVector yes -freetype yes` (design §5.2), and return every
/// resulting page PNG's bytes, ordered by page number (1-based, parsed out
/// of the `<prefix>-<n>.png` filename poppler writes — not just a
/// lexicographic sort, which would misorder a double-digit page count
/// against single digits).
fn rasterize(pdf_bytes: &[u8], work: &Path, stem: &str) -> Vec<(u32, Vec<u8>)> {
    let pdf_path = work.join(format!("{stem}.pdf"));
    std::fs::write(&pdf_path, pdf_bytes).expect("write temp pdf");
    let out_prefix = work.join(stem);

    let output = Command::new("pdftoppm")
        .args(["-png", "-r", DPI, "-aa", "yes", "-aaVector", "yes", "-freetype", "yes"])
        .arg(&pdf_path)
        .arg(&out_prefix)
        .output()
        .expect("spawn pdftoppm");
    assert!(
        output.status.success(),
        "pdftoppm failed for {stem}: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let prefix_name = format!("{stem}-");
    let mut pages: Vec<(u32, PathBuf)> = std::fs::read_dir(work)
        .expect("read rasterize work dir")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter_map(|p| {
            let file_name = p.file_name()?.to_str()?.to_string();
            let rest = file_name.strip_prefix(&prefix_name)?;
            let num_str = rest.strip_suffix(".png")?;
            let n: u32 = num_str.parse().ok()?;
            Some((n, p))
        })
        .collect();
    assert!(!pages.is_empty(), "pdftoppm produced no page PNGs for {stem}");
    pages.sort_by_key(|(n, _)| *n);
    pages
        .into_iter()
        .map(|(n, p)| (n, std::fs::read(&p).expect("read rasterized page png")))
        .collect()
}

// ---------------------------------------------------------------------
// Image diff (design §5.3): ~40 lines, no new crate — `image` (already
// workspace-pinned) decode + a hand-written dual-tolerance compare.
// ---------------------------------------------------------------------

struct DiffResult {
    differing_pixels: u64,
    total_pixels: u64,
    /// `Some` only when dimensions matched (a diff-highlight image only
    /// makes sense pixel-for-pixel).
    diff_image: Option<image::RgbaImage>,
}

/// Compare two same-format PNGs (already decoded to RGBA8): a pixel
/// "differs" when any channel's absolute delta exceeds
/// `PER_PIXEL_CHANNEL_TOL`. Returns the differing-pixel count/total plus a
/// magenta-highlighted diff image (differing pixels painted magenta over a
/// dimmed copy of `actual`) for failure artifacts.
fn diff_images(golden: &image::RgbaImage, actual: &image::RgbaImage) -> Option<DiffResult> {
    if golden.dimensions() != actual.dimensions() {
        return None;
    }
    let (w, h) = golden.dimensions();
    let mut diff_img = image::RgbaImage::new(w, h);
    let mut differing_pixels: u64 = 0;
    for y in 0..h {
        for x in 0..w {
            let gp = golden.get_pixel(x, y);
            let ap = actual.get_pixel(x, y);
            let differs = gp
                .0
                .iter()
                .zip(ap.0.iter())
                .any(|(g, a)| (*g as i16 - *a as i16).abs() > PER_PIXEL_CHANNEL_TOL);
            if differs {
                differing_pixels += 1;
                diff_img.put_pixel(x, y, image::Rgba([255, 0, 255, 255]));
            } else {
                // Dimmed copy of the actual image so a reviewer can still
                // see roughly where on the page the mismatch sits.
                let [r, g, b, a] = ap.0;
                diff_img.put_pixel(
                    x,
                    y,
                    image::Rgba([r / 3, g / 3, b / 3, a.max(64)]),
                );
            }
        }
    }
    Some(DiffResult {
        differing_pixels,
        total_pixels: (w as u64) * (h as u64),
        diff_image: Some(diff_img),
    })
}

/// Compare one rasterized page PNG against its committed golden (or write
/// the golden, under `UPDATE_GOLDEN_IMAGES=1`). `key` names the golden file
/// (`tests/golden_images/<key>.png`) and the failure-artifact stem.
fn check_golden_page(key: &str, actual_png_bytes: &[u8]) {
    let golden_path = golden_dir().join(format!("{key}.png"));

    if std::env::var("UPDATE_GOLDEN_IMAGES").as_deref() == Ok("1") {
        std::fs::create_dir_all(golden_dir()).expect("create tests/golden_images/");
        std::fs::write(&golden_path, actual_png_bytes)
            .unwrap_or_else(|e| panic!("write golden {}: {e}", golden_path.display()));
        eprintln!("updated golden {}", golden_path.display());
        return;
    }

    let golden_bytes = std::fs::read(&golden_path).unwrap_or_else(|e| {
        panic!(
            "missing golden image {} ({e}) — run with UPDATE_GOLDEN_IMAGES=1 to create it",
            golden_path.display()
        )
    });

    let golden_img = image::load_from_memory(&golden_bytes)
        .unwrap_or_else(|e| panic!("decode golden {}: {e}", golden_path.display()))
        .to_rgba8();
    let actual_img = image::load_from_memory(actual_png_bytes)
        .unwrap_or_else(|e| panic!("decode rasterized page for {key}: {e}"))
        .to_rgba8();

    if golden_img.dimensions() != actual_img.dimensions() {
        write_failure_artifacts(key, actual_png_bytes, None);
        panic!(
            "image diff mismatch for {key}: dimension mismatch, expected {:?} got {:?} \
             (page size / DPI regression) — see {} for the actual PNG",
            golden_img.dimensions(),
            actual_img.dimensions(),
            artifacts_dir().display()
        );
    }

    let diff = diff_images(&golden_img, &actual_img).expect("dimensions already checked equal");
    let frac = diff.differing_pixels as f64 / diff.total_pixels as f64;
    if frac > MAX_DIFF_FRACTION {
        write_failure_artifacts(key, actual_png_bytes, diff.diff_image.as_ref());
        panic!(
            "image diff mismatch for {key}: {}/{} pixels differ ({:.4}% > {:.4}% threshold) \
             (baseline: {}); if this is an INTENTIONAL rendering change, rerun with \
             UPDATE_GOLDEN_IMAGES=1 to accept it, otherwise this is a rendering regression. \
             Actual + diff PNGs written under {}",
            diff.differing_pixels,
            diff.total_pixels,
            frac * 100.0,
            MAX_DIFF_FRACTION * 100.0,
            golden_path.display(),
            artifacts_dir().display()
        );
    }
}

fn write_failure_artifacts(key: &str, actual_png_bytes: &[u8], diff_image: Option<&image::RgbaImage>) {
    let dir = artifacts_dir();
    std::fs::create_dir_all(&dir).ok();
    let actual_path = dir.join(format!("{key}.actual.png"));
    std::fs::write(&actual_path, actual_png_bytes).ok();
    if let Some(diff_img) = diff_image {
        let diff_path = dir.join(format!("{key}.diff.png"));
        let _ = diff_img.save(&diff_path);
    }
}

/// Rasterize `pdf_bytes` and check every page against its golden
/// (`<fixture_key>-<page>`), 1-based page numbering to match `pdftoppm`'s
/// own `-<n>` suffix.
fn check_golden_pdf(fixture_key: &str, pdf_bytes: &[u8]) {
    let work = tmpdir(fixture_key);
    let pages = rasterize(pdf_bytes, &work, fixture_key);
    for (page_num, png_bytes) in &pages {
        check_golden_page(&format!("{fixture_key}-{page_num}"), png_bytes);
    }
    std::fs::remove_dir_all(&work).ok();
}

// ---------------------------------------------------------------------
// The fixtures (design §5.1). Every helper below panics on a genuine
// compile/render failure (a real bug), and only the top-level test below
// self-skips on a missing prerequisite.
// ---------------------------------------------------------------------

/// #1 minimal.saty — Latin text, line-wrap, `\emph`; the layout anchor.
fn check_minimal() {
    let doc = compile_v006("minimal.saty", &Base14Metrics);
    check_golden_pdf("minimal", &render_base14(&doc));
}

/// #4 graphics.saty — path fill (RGB red) / stroke (Gray) / inline
/// `draw-text`: colors + vector graphics, invisible to both the
/// geometry-only box snapshot and `pdftotext`.
fn check_graphics() {
    let doc = compile_v006("graphics.saty", &Base14Metrics);
    check_golden_pdf("graphics", &render_base14(&doc));
}

/// #5 table.saty — 2x2 ruled tabular: cell text + rule strokes. Needs the
/// big-stack thread (`@require: list` deepens the parse tree past the
/// default budget — see `run_with_big_stack`'s doc comment).
fn check_table() {
    let pdf_bytes = run_with_big_stack(|| {
        let doc = compile_v006("table.saty", &Base14Metrics);
        render_base14(&doc)
    });
    check_golden_pdf("table", &pdf_bytes);
}

/// #6 v01-math.saty — math layout (`a^2+b^2=c^2`, `\frac`) AND 0.1 syntax
/// in one fixture; renders via `Base14Metrics` (mirrors
/// `tests/e2e.rs`'s v0.1 math capstone, which also needs the big-stack
/// thread for this fixture).
fn check_v01_math() {
    let pdf_bytes = run_with_big_stack(|| {
        let doc = compile_v01("v01-math.saty", &Base14Metrics);
        render_base14(&doc)
    });
    check_golden_pdf("v01-math", &pdf_bytes);
}

/// #7 multicolumn.saty — multi-page page-break geometry
/// (`page-break-multicolumn`); validates per-page PNG pairing across
/// several pages, not just a single-page fixture.
fn check_multicolumn() {
    let doc = compile_v006("multicolumn.saty", &Base14Metrics);
    check_golden_pdf("multicolumn", &render_base14(&doc));
}

/// #2 realfont.saty via `--font`-equivalent direct `TtfFontStore::load`,
/// once per real downloaded face: `lmsans10-regular.otf` (CFF/OTF —
/// `/FontFile3` embed) and `Junicode.ttf` (glyf — `/FontFile2` embed).
/// `realfont.saty`'s body needs café/→; per the design's note, a face that
/// lacks either glyph self-skips (not fails) for that one sub-fixture, same
/// spirit as `tests/fonts.rs`'s `font_supports_fixture_glyphs` guard.
fn check_realfont() {
    for (tag, font_file) in [("lmsans", "lmsans10-regular.otf"), ("junicode", "Junicode.ttf")] {
        let font_path = dist_fonts_dir().join(font_file);
        let store = TtfFontStore::load(&font_path, None, None)
            .unwrap_or_else(|e| panic!("load {}: {e}", font_path.display()));
        if !font_supports_fixture_glyphs(&store) {
            eprintln!(
                "skipping realfont/{tag}: {} lacks café/→ glyphs used by realfont.saty",
                font_path.display()
            );
            continue;
        }
        let doc = compile_v006("realfont.saty", &store);
        check_golden_pdf(&format!("realfont-{tag}"), &render_ttf(&doc, &store));
    }
}

/// Same glyph-coverage probe `tests/fonts.rs` uses (café + →), against a
/// bare (registry-less) `TtfFontStore`.
fn font_supports_fixture_glyphs(store: &TtfFontStore) -> bool {
    use rustyfi_backend::{FontKey, Length};
    let size = Length::pt(12.0);
    store.advance(FontKey(0), 'é', size).is_some() && store.advance(FontKey(0), '→', size).is_some()
}

/// #3 cjk.saty — Japanese kanji/kana, CID-keyed `glyf` `/FontFile2`, via
/// the real `FontRegistry::discover` production path (same as
/// `tests/cjk_render.rs`), so the font-scheme routing (`han-ideographic`/
/// `kana` -> `ipaexm`) is exercised too, not just a bare `--font` one-off.
fn check_cjk() {
    let registry = FontRegistry::discover(Some(&repo_lib_root()), None, &FontFlags::default())
        .expect("font config discovery must succeed")
        .unwrap_or_else(|| {
            panic!(
                "lib-rustyfi/dist/hash/fonts.rustyfi-hash must be present after \
                 download-fonts.sh (checked {})",
                repo_lib_root().join("dist/hash/fonts.rustyfi-hash").display()
            )
        });
    let store = registry.build_store().expect("build_store must succeed");
    let doc = compile_v006("cjk.saty", &store);
    check_golden_pdf("cjk", &render_ttf(&doc, &store));
}

// ---------------------------------------------------------------------
// The test.
// ---------------------------------------------------------------------

#[test]
#[ignore = "needs poppler pdftoppm + scripts/download-fonts.sh's fonts; run in CI's image-diff job \
            (or locally with poppler-utils + `sh scripts/download-fonts.sh`, then --ignored)"]
fn pdf_image_diff_regression() {
    if find_pdftoppm().is_none() {
        eprintln!("skipping pdf_image_diff_regression: pdftoppm (poppler-utils) not found on PATH");
        return;
    }

    // Base14 fixtures: no font prerequisite (design §5.1's rationale — a
    // pinned poppler makes its substitute Helvetica a fixed function too).
    check_minimal();
    check_graphics();
    check_table();
    check_v01_math();
    check_multicolumn();

    // Embedded-font fixtures: the real point of this test (glyph-shape /
    // font-embedding coverage) — gated on scripts/download-fonts.sh.
    if fonts_present() {
        check_realfont();
        check_cjk();
    } else {
        eprintln!(
            "skipping realfont/cjk embedded-font fixtures: run `sh scripts/download-fonts.sh` \
             first (checked {})",
            dist_fonts_dir().display()
        );
    }
}
