//! Text-rendering plan, Slice 1: CLI font wiring tests. Drives the *built*
//! `rustyfi` binary (like `tests/cache.rs`/`tests/dispatch.rs`), not the
//! library directly, because the thing under test is the CLI-level wiring
//! itself — `--font-dir` discovery, the `cmd_compile` branch between
//! `render_pdf_ttf` and the base-14 `render_pdf`, and the compile-cache fold
//! — none of which exist when calling `rustyfi_lang`/`rustyfi_pdf` directly.
//!
//! Font-dependent tests need a real TrueType file on disk, so they locate
//! one via fontconfig (falling back to a few common paths) and skip
//! gracefully — never failing the build — when none is found, mirroring
//! `crates/rustyfi-pdf/tests/ttf.rs`. They also skip when `pdftotext` is
//! unavailable.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

/// The built `rustyfi` binary (cargo provides this env var to the
/// integration tests of the crate that defines the `[[bin]]`).
fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_rustyfi"))
}

/// This repo's `lib-rustyfi/`, resolved from the crate manifest dir exactly
/// as the other integration tests do — the fixtures `@require: stdja-mini`
/// from there.
fn lib_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lib-rustyfi")
}

fn realfont_fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/realfont.saty")
}

fn minimal_fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/minimal.saty")
}

/// A `--lib-root`-shaped directory with NO `dist/hash/` — i.e. genuinely
/// "nothing configured" — regardless of whether `scripts/download-fonts.sh`
/// has already been run against the real `lib_root()` (which then
/// legitimately DOES carry a `dist/hash/fonts.satysfi-hash` /
/// `default-font.satysfi-hash`, for `crates/rustyfi/tests/cjk_render.rs`
/// and the CJK end-to-end proof). Only `dist/packages` is symlinked in (all
/// the loader needs to resolve `@require: stdja-mini`), so this stays a
/// thin proxy rather than a real copy of the whole package tree.
fn font_free_lib_root(work: &Path) -> PathBuf {
    let root = work.join("no-font-lib-root");
    let dist = root.join("dist");
    std::fs::create_dir_all(&dist).unwrap();
    std::os::unix::fs::symlink(lib_root().join("dist/packages"), dist.join("packages"))
        .expect("symlink dist/packages into the font-free lib root");
    root
}

fn tmpdir(tag: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let p = std::env::temp_dir().join(format!(
        "rustyfi-fonts-{tag}-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        n
    ));
    std::fs::create_dir_all(&p).unwrap();
    p
}

/// Locate a real TrueType file to test against: prefer fontconfig's idea of
/// "DejaVu Sans", then fall back to a few paths common on NixOS/nix-based
/// systems and typical Linux distros. Identical strategy to
/// `crates/rustyfi-pdf/tests/ttf.rs`'s `find_regular_font`.
fn find_regular_font() -> Option<PathBuf> {
    if let Ok(output) = Command::new("fc-match")
        .args(["--format=%{file}", "DejaVuSans"])
        .output()
    {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path.is_empty() && Path::new(&path).is_file() {
                return Some(PathBuf::from(path));
            }
        }
    }

    for candidate in [
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/dejavu/DejaVuSans.ttf",
        "/run/current-system/sw/share/fonts/truetype/DejaVuSans.ttf",
        "/run/current-system/sw/share/X11/fonts/DejaVuSans.ttf",
    ] {
        if Path::new(candidate).is_file() {
            return Some(PathBuf::from(candidate));
        }
    }

    None
}

macro_rules! need_font {
    () => {
        match find_regular_font() {
            Some(path) => path,
            None => {
                eprintln!(
                    "skipping: no DejaVuSans-like TrueType font found on this system \
                     (tried `fc-match DejaVuSans` and common nix/distro paths)"
                );
                return;
            }
        }
    };
}

macro_rules! need_pdftotext {
    () => {
        match Command::new("which").arg("pdftotext").output() {
            Ok(out) if out.status.success() => {}
            _ => {
                eprintln!("skipping: pdftotext not on PATH");
                return;
            }
        }
    };
}

/// `realfont.saty`'s body needs both of these glyphs (café, →); skip rather
/// than fail if the discovered font happens to lack either, since the
/// fixture's source text is static (unlike `rustyfi-pdf/tests/ttf.rs`, which
/// builds its text from a probed character and can dodge an unsupported
/// one).
fn font_supports_fixture_glyphs(font_path: &Path) -> bool {
    use rustyfi_backend::{FontKey, FontMetrics, Length};
    let Ok(store) = rustyfi_pdf::TtfFontStore::load(font_path, None, None) else {
        return false;
    };
    let size = Length::pt(12.0);
    store.advance(FontKey(0), 'é', size).is_some() && store.advance(FontKey(0), '→', size).is_some()
}

/// Write a temporary font root: `<root>/dist/hash/fonts.satysfi-hash` +
/// `default-font.satysfi-hash`, both in this port's plain-JSON schema (see
/// `rustyfi_pdf::fonts`'s module docs), naming `font_path` as the sole
/// (regular) face. Returns `root`, suitable for `--font-dir`.
fn write_font_config(work: &Path, font_path: &Path) -> PathBuf {
    let root = work.join("fontroot");
    let hash_dir = root.join("dist/hash");
    std::fs::create_dir_all(&hash_dir).unwrap();

    let fonts_hash = serde_json::json!({
        "testface": { "src": font_path.to_str().expect("font path is valid UTF-8") }
    });
    std::fs::write(
        hash_dir.join("fonts.satysfi-hash"),
        serde_json::to_vec_pretty(&fonts_hash).unwrap(),
    )
    .unwrap();

    let default_font_hash = serde_json::json!({ "regular": "testface" });
    std::fs::write(
        hash_dir.join("default-font.satysfi-hash"),
        serde_json::to_vec_pretty(&default_font_hash).unwrap(),
    )
    .unwrap();

    root
}

fn as_v006(cst: rustyfi_loader::LoadedCst) -> rustyfi_syntax::cst::File {
    match cst {
        rustyfi_loader::LoadedCst::V0_0(f) => f,
        rustyfi_loader::LoadedCst::V0_1(_) => {
            unreachable!("this test's ground-truth path is V0_0-only")
        }
    }
}

fn assert_ok(out: &Output, ctx: &str) {
    assert!(
        out.status.success(),
        "{ctx}: compile failed (code {:?})\nstdout:\n{}\nstderr:\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
}

/// Deliverable 1e, case 1: a fixture whose body needs a real font (a
/// non-WinAnsi café/→) compiles through `--font-dir`, embeds the real font
/// file, and round-trips through `pdftotext` (proving the `ToUnicode` CMap
/// built in `cid.rs` works end to end, not just in `rustyfi-pdf`'s own unit
/// tests).
#[test]
fn real_font_fixture_renders_through_ttf_path_and_roundtrips() {
    let font_path = need_font!();
    if !font_supports_fixture_glyphs(&font_path) {
        eprintln!(
            "skipping: discovered font {} lacks café/→ glyphs used by realfont.saty",
            font_path.display()
        );
        return;
    }
    need_pdftotext!();

    let work = tmpdir("realfont");
    let font_root = write_font_config(&work, &font_path);
    let out = work.join("out.pdf");

    let output = Command::new(bin())
        .arg(realfont_fixture())
        .args(["-o".as_ref(), out.as_os_str()])
        .args(["--lib-root".as_ref(), lib_root().as_os_str()])
        .args(["--font-dir".as_ref(), font_root.as_os_str()])
        .args(["--no-cache"])
        .output()
        .expect("spawn rustyfi");
    assert_ok(&output, "real-font compile");

    let pdf_bytes = std::fs::read(&out).expect("read output pdf");
    assert!(
        pdf_bytes.starts_with(b"%PDF-"),
        "not a PDF header: {:?}",
        &pdf_bytes[..pdf_bytes.len().min(16)]
    );

    // D5: the embedded `FontFile2` is now SUBSET to the glyphs
    // `realfont.saty`'s body actually uses, so the whole output PDF is much
    // SMALLER than the source font file (inverted from the pre-D5
    // whole-file-embed assertion this test used to make — see
    // `rustyfi-pdf/tests/ttf.rs`'s matching update).
    let font_bytes = std::fs::read(&font_path).expect("read font file");
    assert!(
        pdf_bytes.len() < font_bytes.len(),
        "expected the subsetted PDF ({} bytes) to be smaller than the source font file ({} bytes)",
        pdf_bytes.len(),
        font_bytes.len()
    );

    let pdftotext = Command::new("pdftotext")
        .arg(&out)
        .arg("-")
        .output()
        .expect("run pdftotext");
    assert!(
        pdftotext.status.success(),
        "pdftotext failed: {pdftotext:?}"
    );
    let text = String::from_utf8_lossy(&pdftotext.stdout);
    assert!(
        text.contains("Hello, world!"),
        "missing ASCII text: {text:?}"
    );
    assert!(
        text.contains("café") && text.contains('→'),
        "missing non-WinAnsi text (ToUnicode round-trip failed): {text:?}"
    );

    std::fs::remove_dir_all(&work).ok();
}

/// Deliverable 1e, case 2 (the fallback guard): with no font configured
/// anywhere (no flags, no `$RUSTYFI_FONT_DIR`, no `dist/hash/` under
/// `--lib-root`), `cmd_compile` must take the *exact* pre-Slice-1 path —
/// same `Base14Metrics` instance, same `render_pdf` call — so the output is
/// byte-for-byte identical to calling the library directly the way
/// `tests/e2e.rs` does. This is the core invariant: adding font support must
/// not change a single byte of output for documents/setups that don't use
/// it.
#[test]
fn no_font_config_matches_base14_byte_for_byte() {
    let work = tmpdir("no-font-config");
    let out = work.join("out.pdf");
    // A `dist/hash`-free lib root — NOT `lib_root()` directly, which (once
    // `scripts/download-fonts.sh` has been run for the CJK proof,
    // `tests/cjk_render.rs`) legitimately carries a real font config and so
    // would no longer exercise the "nothing configured" invariant this test
    // is about.
    let font_free_root = font_free_lib_root(&work);

    let output = Command::new(bin())
        .arg(minimal_fixture())
        .args(["-o".as_ref(), out.as_os_str()])
        .args(["--lib-root".as_ref(), font_free_root.as_os_str()])
        .args(["--no-cache"])
        .output()
        .expect("spawn rustyfi");
    assert_ok(&output, "no-font-config compile");
    let via_cli = std::fs::read(&out).expect("read cli output");
    assert!(via_cli.starts_with(b"%PDF-"));

    // Ground truth: the base-14 path, called directly (mirrors
    // `tests/e2e.rs`'s `compile_fixture`).
    let program = rustyfi_loader::load(
        &minimal_fixture(),
        &rustyfi_loader::LoadOptions {
            lib_root: Some(font_free_root.clone()),
            ..Default::default()
        },
    )
    .expect("load fixture");
    let mut files = program.files;
    let entry = files.pop().expect("loader always yields the entry last");
    let entry_cst = as_v006(entry.cst);
    let mut prelude = Vec::new();
    for lib in files {
        prelude.extend(as_v006(lib.cst).prelude);
    }
    prelude.extend(entry_cst.prelude);
    let merged = rustyfi_syntax::cst::File {
        headers: Vec::new(),
        prelude,
        in_kw: entry_cst.in_kw,
        body: entry_cst.body,
        eoi: entry_cst.eoi,
    };
    let metrics = rustyfi_pdf::Base14Metrics;
    let doc = rustyfi_lang::compile_document_cst(&merged, &metrics).expect("compile fixture");
    let via_lib =
        rustyfi_pdf::render_pdf(&doc.geometry, &doc.pages, &doc.images).expect("render fixture");

    assert_eq!(
        via_cli, via_lib,
        "with no font configured, CLI output must be byte-identical to the direct base-14 path"
    );

    std::fs::remove_dir_all(&work).ok();
}

/// Deliverable 1d: the compile cache folds in the resolved font identity, so
/// switching a document from the base-14 path to a real font (with the
/// *same* cache directory) must be a fresh miss, never a stale hit that
/// would otherwise serve back the earlier base-14 PDF under a font-enabled
/// request.
#[test]
fn changing_font_config_invalidates_the_compile_cache() {
    let font_path = need_font!();

    let work = tmpdir("cache-invalidate");
    let cache_dir = work.join("cache");
    let out = work.join("out.pdf");
    // `dist/hash`-free (see `font_free_lib_root`'s doc): keeps this test's
    // "first run" genuinely base-14, independent of whether the real
    // `lib_root()` has since gained a font config via
    // `scripts/download-fonts.sh` (`tests/cjk_render.rs`).
    let font_free_root = font_free_lib_root(&work);

    let was_cached = |out: &Output| String::from_utf8_lossy(&out.stderr).contains("(cached)");

    // First: no font config (base-14 path) populates the cache.
    let first = Command::new(bin())
        .arg(minimal_fixture())
        .args(["-o".as_ref(), out.as_os_str()])
        .args(["--lib-root".as_ref(), font_free_root.as_os_str()])
        .args(["--cache-dir".as_ref(), cache_dir.as_os_str()])
        .output()
        .expect("spawn rustyfi (first)");
    assert_ok(&first, "first (base-14) run");
    assert!(!was_cached(&first), "first run must be a miss");
    let base14_bytes = std::fs::read(&out).expect("read first output");

    // Same document, same cache dir, but now with a real font configured:
    // must be a fresh miss (not a stale hit under the base-14 entry), and
    // must actually render through the TTF path (different bytes).
    let font_root = write_font_config(&work, &font_path);
    let second = Command::new(bin())
        .arg(minimal_fixture())
        .args(["-o".as_ref(), out.as_os_str()])
        .args(["--lib-root".as_ref(), font_free_root.as_os_str()])
        .args(["--cache-dir".as_ref(), cache_dir.as_os_str()])
        .args(["--font-dir".as_ref(), font_root.as_os_str()])
        .output()
        .expect("spawn rustyfi (second)");
    assert_ok(&second, "second (font-configured) run");
    assert!(
        !was_cached(&second),
        "a newly-configured font must not hit the base-14 cache entry"
    );
    let ttf_bytes = std::fs::read(&out).expect("read second output");
    assert_ne!(
        base14_bytes, ttf_bytes,
        "the font-configured run must actually render through the TTF path"
    );

    // Running the font-configured request again now hits its own entry.
    let third = Command::new(bin())
        .arg(minimal_fixture())
        .args(["-o".as_ref(), out.as_os_str()])
        .args(["--lib-root".as_ref(), font_free_root.as_os_str()])
        .args(["--cache-dir".as_ref(), cache_dir.as_os_str()])
        .args(["--font-dir".as_ref(), font_root.as_os_str()])
        .output()
        .expect("spawn rustyfi (third)");
    assert_ok(&third, "third (font-configured, warm) run");
    assert!(
        was_cached(&third),
        "the font-configured request should now hit its own cache entry"
    );

    std::fs::remove_dir_all(&work).ok();
}

/// `--font` (a config-less one-off) takes precedence over `--font-dir` and
/// needs no `fonts.satysfi-hash` at all — the CLI-flags path through
/// `rustyfi_pdf::fonts::FontRegistry::discover`.
#[test]
fn font_flag_is_a_config_less_one_off() {
    let font_path = need_font!();
    if !font_supports_fixture_glyphs(&font_path) {
        eprintln!("skipping: discovered font lacks café/→ glyphs");
        return;
    }
    need_pdftotext!();

    let work = tmpdir("font-flag");
    let out = work.join("out.pdf");

    let output = Command::new(bin())
        .arg(realfont_fixture())
        .args(["-o".as_ref(), out.as_os_str()])
        .args(["--lib-root".as_ref(), lib_root().as_os_str()])
        .args(["--font".as_ref(), font_path.as_os_str()])
        .args(["--no-cache"])
        .output()
        .expect("spawn rustyfi");
    assert_ok(&output, "--font one-off compile");

    let pdf_bytes = std::fs::read(&out).expect("read output");
    assert!(pdf_bytes.starts_with(b"%PDF-"));

    let pdftotext = Command::new("pdftotext")
        .arg(&out)
        .arg("-")
        .output()
        .expect("pdftotext");
    assert!(pdftotext.status.success());
    let text = String::from_utf8_lossy(&pdftotext.stdout);
    assert!(text.contains("café"), "missing café: {text:?}");

    std::fs::remove_dir_all(&work).ok();
}

/// `--font-bold`/`--font-oblique` without `--font` is a clap usage error
/// (exit 2), not a silent no-op or a panic — enforced via `.requires("font")`
/// in `dispatch.rs`.
#[test]
fn font_bold_without_font_is_a_usage_error() {
    let work = tmpdir("font-bold-without-font");
    let out = work.join("out.pdf");
    let output = Command::new(bin())
        .arg(minimal_fixture())
        .args(["-o".as_ref(), out.as_os_str()])
        .args(["--lib-root".as_ref(), lib_root().as_os_str()])
        .args(["--font-bold".as_ref(), std::ffi::OsStr::new("whatever.ttf")])
        .output()
        .expect("spawn rustyfi");
    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2), "clap usage error is exit 2");
    std::fs::remove_dir_all(&work).ok();
}
