//! Content-addressed compile cache for the `cmd_compile` path.
//!
//! The whole lex→parse→elaborate→typecheck→eval→render chain is
//! deterministic: a given set of resolved input bytes, under a given compiler
//! and target language version, always renders the same PDF. This module
//! turns that into a near-instant recompile of an unchanged document by
//! hashing everything that determines the output into a stable key, storing
//! the rendered PDF under that key, and — on a later run whose key matches —
//! copying the stored PDF straight to the requested `--output`, skipping the
//! expensive `compile_document_cst` + `render_pdf` entirely.
//!
//! Caching is transparent: a hit writes byte-for-byte the same PDF a miss
//! would have rendered (it *is* the bytes an earlier miss rendered and
//! stored), so correctness never depends on whether a hit occurred.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use rustyfi_loader::LoadedProgram;
use rustyfi_syntax::RustyfiVersion;
use sha2::{Digest, Sha256};

use crate::format::OutputFormat;

/// A resolved, ready-to-use cache directory. Obtain one with [`Cache::open`];
/// a `None` there means caching is disabled (either `--no-cache`, or the
/// directory could not be created) and the caller compiles uncached.
pub struct Cache {
    dir: PathBuf,
}

/// A cache hit: the stored PDF bytes plus the page/line counts recorded
/// alongside them, so the "output written" status line reads identically to
/// the miss that first produced the document (with a trailing `(cached)`).
pub struct Hit {
    pub pdf: Vec<u8>,
    pub pages: usize,
    pub lines: usize,
}

impl Cache {
    /// Resolve — and create — the cache directory. Precedence:
    ///
    /// 1. the `--cache-dir DIR` override, used verbatim;
    /// 2. `$XDG_CACHE_HOME/rustyfi-rust`;
    /// 3. `$HOME/.cache/rustyfi-rust`;
    /// 4. a `rustyfi-rust-cache` directory under the system temp dir.
    ///
    /// Returns `None` (caching silently disabled) if the directory cannot be
    /// created, so a read-only or missing cache home never fails a compile.
    pub fn open(override_dir: Option<PathBuf>) -> Option<Cache> {
        let dir = resolve_dir(override_dir);
        std::fs::create_dir_all(&dir).ok()?;
        Some(Cache { dir })
    }

    fn pdf_path(&self, key: &str) -> PathBuf {
        self.dir.join(format!("{key}.pdf"))
    }

    fn meta_path(&self, key: &str) -> PathBuf {
        self.dir.join(format!("{key}.meta"))
    }

    /// Look `key` up. A hit requires *both* the `<key>.pdf` payload and a
    /// parseable `<key>.meta` sidecar; a missing or malformed sidecar is
    /// treated as a miss so a hit always carries the counts for its status
    /// line. Any I/O error is a miss (the caller then compiles normally).
    pub fn get(&self, key: &str) -> Option<Hit> {
        let pdf = std::fs::read(self.pdf_path(key)).ok()?;
        let meta = std::fs::read_to_string(self.meta_path(key)).ok()?;
        let (pages, lines) = parse_meta(&meta)?;
        Some(Hit { pdf, pages, lines })
    }

    /// Store `pdf` (with its page/line counts) under `key`. Writes go to a
    /// unique temp file in the same directory and are `rename`d into place, so
    /// a concurrent reader never observes a half-written payload. The payload
    /// is written before the sidecar: a reader that catches the gap simply
    /// misses and recompiles (harmless), never reads a stale pairing (a
    /// different key ⇒ different files). Best-effort — the caller ignores the
    /// returned error, keeping caching invisible to the compile's result.
    pub fn put(&self, key: &str, pdf: &[u8], pages: usize, lines: usize) -> std::io::Result<()> {
        write_atomic(&self.pdf_path(key), pdf)?;
        write_atomic(&self.meta_path(key), format!("{pages}\n{lines}\n").as_bytes())?;
        Ok(())
    }
}

/// Compute the content-addressed cache key for a loaded program, or `None` if
/// any resolved input file cannot be re-read (the caller then compiles
/// uncached rather than risk a wrong key).
///
/// The SHA-256 stream is domain-separated and covers exactly what determines
/// the rendered output:
///
/// 1. a format tag — bumped to `v2` (from `v1`) by the HTML output backend's
///    Slice 1 (`docs/plans/design-html-output.md` §CLI surface, "Cache key"),
///    which also folds in field 7 below; bump again to invalidate every
///    entry on a further layout/cache-shape change;
/// 2. the compiler version (`CARGO_PKG_VERSION`), so an upgrade re-renders;
/// 3. the target language version;
/// 4. the entry file's basename — defensive: cheap, and future-proofs against
///    output ever coming to depend on the document's own name (it does not
///    today, so identical content under a different entry name is a *safe*
///    miss, never a wrong hit);
/// 5. every resolved input file's bytes, length-prefixed, in the loader's
///    dependency-first order (entry last). The paths themselves are *not*
///    hashed — only content matters for rendering — but the set of files and
///    their order come straight from the loader's resolution, so a changed
///    `@require:`/`@import:` graph changes the key.
/// 6. (text-rendering plan, Slice 1) the resolved font identity: `None` when
///    compiling through the base-14 path, or each backing font file's bytes
///    (length-prefixed, in `TtfFontStore` slot order) when a real
///    `TtfFontStore` is in play. Folding this in is required, not cosmetic —
///    without it, switching `--font-dir` (or toggling fonts on/off) would
///    hit a stale entry cached under a different font (or none) but the same
///    document bytes, silently serving the wrong PDF. Bytes rather than
///    paths, so an in-place font-file edit (same path, new content) also
///    invalidates, matching this function's "only content matters" stance
///    for the resolved input files above.
/// 7. (HTML output backend, Slice 1) `format.cache_tag()` — without this, a
///    `--format pdf` compile and a `--format html` compile of the identical
///    document/version/font would collide on the same key, so a hit from one
///    format would write the OTHER format's bytes to the requested output
///    (wrong extension, unparseable content).
/// 8. (phase-7c saphe solver, C3 — `docs/plans/design-saphe-solver.md` §5.3)
///    `deps_lock` — the resolved `Satyrfile.lock`'s
///    [`rustyfi_satyrographos::Lockfile::digest`], when the compile is
///    driven by a solved package-manager lock (Envelopes/manifest mode);
///    `None` when no lock is in play. Without this, a `saphe update` that
///    changes a locked package's version — but leaves every already-resolved
///    `@require:`/`use` *input file* byte-identical — would silently keep
///    serving the OLD lock's cached render, since nothing else in this key
///    would have changed.
///
/// The loader does not retain raw file bytes, so each file is re-read from its
/// canonical path here. Length-prefixing makes the concatenation unambiguous
/// (no two distinct file splits hash alike), and the NUL separators between
/// the header fields are safe because none of those fields can contain a NUL.
pub fn compute_key(
    program: &LoadedProgram,
    compiler_version: &str,
    target: RustyfiVersion,
    entry: &Path,
    font_store: Option<&rustyfi_pdf::TtfFontStore>,
    format: OutputFormat,
    deps_lock: Option<&str>,
) -> Option<String> {
    hash_inputs(
        program.files.iter().map(|f| f.path.as_path()),
        compiler_version,
        target,
        entry,
        font_store,
        format,
        deps_lock,
    )
}

/// The key computation over a bare sequence of input paths (see
/// [`compute_key`]). Split out so it can be unit-tested without constructing a
/// [`LoadedProgram`], whose `cst` field is impractical to build by hand.
fn hash_inputs<'a>(
    paths: impl Iterator<Item = &'a Path>,
    compiler_version: &str,
    target: RustyfiVersion,
    entry: &Path,
    font_store: Option<&rustyfi_pdf::TtfFontStore>,
    format: OutputFormat,
    deps_lock: Option<&str>,
) -> Option<String> {
    let mut h = Sha256::new();
    h.update(b"rustyfi-rust-compile-cache\x00v2\x00");
    h.update(compiler_version.as_bytes());
    h.update(b"\x00");
    h.update(target.to_string().as_bytes());
    h.update(b"\x00");
    let entry_name = entry
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    h.update(entry_name.as_bytes());
    h.update(b"\x00");
    for path in paths {
        let bytes = std::fs::read(path).ok()?;
        h.update((bytes.len() as u64).to_le_bytes());
        h.update(&bytes);
    }
    h.update(b"\x00fonts\x00");
    match font_store {
        Some(store) => {
            h.update(b"\x01");
            h.update((store.num_files() as u64).to_le_bytes());
            for i in 0..store.num_files() {
                let bytes = store.file_bytes(i);
                h.update((bytes.len() as u64).to_le_bytes());
                h.update(bytes);
            }
        }
        None => h.update(b"\x00"),
    }
    h.update(b"\x00format\x00");
    h.update(format.cache_tag().as_bytes());
    h.update(b"\x00deps_lock\x00");
    match deps_lock {
        Some(digest) => {
            h.update(b"\x01");
            h.update(digest.as_bytes());
        }
        None => h.update(b"\x00"),
    }
    Some(hex(&h.finalize()))
}

/// Resolve the cache directory from the override / env chain (see
/// [`Cache::open`]). Never touches the filesystem; the caller creates the dir.
fn resolve_dir(override_dir: Option<PathBuf>) -> PathBuf {
    if let Some(dir) = override_dir {
        return dir;
    }
    if let Some(xdg) = non_empty_env("XDG_CACHE_HOME") {
        return PathBuf::from(xdg).join("rustyfi-rust");
    }
    if let Some(home) = non_empty_env("HOME") {
        return PathBuf::from(home).join(".cache").join("rustyfi-rust");
    }
    std::env::temp_dir().join("rustyfi-rust-cache")
}

/// `$VAR` if it is set and non-empty, else `None` (an empty `XDG_CACHE_HOME`
/// must not resolve to a bare `/rustyfi-rust`).
fn non_empty_env(var: &str) -> Option<std::ffi::OsString> {
    std::env::var_os(var).filter(|v| !v.is_empty())
}

/// Parse a `<key>.meta` sidecar: the first two whitespace-separated tokens are
/// the page and line counts. Tolerant of the trailing newline.
fn parse_meta(s: &str) -> Option<(usize, usize)> {
    let mut it = s.split_whitespace();
    let pages = it.next()?.parse().ok()?;
    let lines = it.next()?.parse().ok()?;
    Some((pages, lines))
}

/// Write `bytes` to `path` atomically: a uniquely-named temp file in the same
/// directory (so the `rename` stays on one filesystem), renamed into place. On
/// a rename failure the temp file is cleaned up.
fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let name = path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = path.with_file_name(format!(".{name}.tmp.{}.{seq}", std::process::id()));
    std::fs::write(&tmp, bytes)?;
    match std::fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "rustyfi-cache-unit-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn key_is_stable_for_identical_inputs() {
        let dir = scratch();
        let a = dir.join("a.saty");
        std::fs::write(&a, b"@require: foo\ndocument (||) '<>\n").unwrap();
        let k1 = hash_inputs([a.as_path()].into_iter(), "0.1.0", RustyfiVersion::DEFAULT, &a, None, OutputFormat::Pdf, None).unwrap();
        let k2 = hash_inputs([a.as_path()].into_iter(), "0.1.0", RustyfiVersion::DEFAULT, &a, None, OutputFormat::Pdf, None).unwrap();
        assert_eq!(k1, k2, "same inputs must hash identically");
        assert_eq!(k1.len(), 64, "sha-256 hex is 64 chars");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn key_changes_when_a_byte_changes() {
        let dir = scratch();
        let a = dir.join("a.saty");
        std::fs::write(&a, b"document (||) '<>\n").unwrap();
        let before = hash_inputs([a.as_path()].into_iter(), "0.1.0", RustyfiVersion::DEFAULT, &a, None, OutputFormat::Pdf, None).unwrap();
        std::fs::write(&a, b"document (||) '< >\n").unwrap();
        let after = hash_inputs([a.as_path()].into_iter(), "0.1.0", RustyfiVersion::DEFAULT, &a, None, OutputFormat::Pdf, None).unwrap();
        assert_ne!(before, after, "a one-byte edit must change the key");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn key_changes_when_compiler_version_bumps() {
        let dir = scratch();
        let a = dir.join("a.saty");
        std::fs::write(&a, b"document (||) '<>\n").unwrap();
        let v1 = hash_inputs([a.as_path()].into_iter(), "0.1.0", RustyfiVersion::DEFAULT, &a, None, OutputFormat::Pdf, None).unwrap();
        let v2 = hash_inputs([a.as_path()].into_iter(), "0.2.0", RustyfiVersion::DEFAULT, &a, None, OutputFormat::Pdf, None).unwrap();
        assert_ne!(v1, v2, "a compiler-version bump must invalidate the key");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn key_changes_with_the_input_set() {
        let dir = scratch();
        let a = dir.join("a.saty");
        let b = dir.join("b.satyh");
        std::fs::write(&a, b"document (||) '<>\n").unwrap();
        std::fs::write(&b, b"let x = 1\n").unwrap();
        let one = hash_inputs([a.as_path()].into_iter(), "0.1.0", RustyfiVersion::DEFAULT, &a, None, OutputFormat::Pdf, None).unwrap();
        let two = hash_inputs(
            [b.as_path(), a.as_path()].into_iter(),
            "0.1.0",
            RustyfiVersion::DEFAULT,
            &a,
            None,
            OutputFormat::Pdf,
            None,
        )
        .unwrap();
        assert_ne!(one, two, "adding a resolved dependency must change the key");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Text-rendering plan, Slice 1: a `None` font store (base-14 path) must
    /// hash differently from a `Some` store (the TTF path), for the same
    /// document/version/entry — otherwise turning font support on would
    /// silently reuse a cached base-14 PDF. Uses a real system font when one
    /// is discoverable (skips gracefully otherwise, matching
    /// `crates/rustyfi-pdf/tests/ttf.rs`'s convention), since building a
    /// `TtfFontStore` validates the font file by actually parsing it.
    #[test]
    fn key_changes_when_a_font_store_is_configured() {
        let Some(font_path) = find_test_font() else {
            eprintln!("skipping: no DejaVuSans-like TrueType font found on this system");
            return;
        };
        let dir = scratch();
        let a = dir.join("a.saty");
        std::fs::write(&a, b"document (||) '<>\n").unwrap();

        let store = rustyfi_pdf::TtfFontStore::load(&font_path, None, None).expect("load font");

        let without_font =
            hash_inputs([a.as_path()].into_iter(), "0.1.0", RustyfiVersion::DEFAULT, &a, None, OutputFormat::Pdf, None)
                .unwrap();
        let with_font = hash_inputs(
            [a.as_path()].into_iter(),
            "0.1.0",
            RustyfiVersion::DEFAULT,
            &a,
            Some(&store),
            OutputFormat::Pdf,
            None,
        )
        .unwrap();
        assert_ne!(
            without_font, with_font,
            "configuring a font must change the cache key vs. the base-14 path"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// HTML output backend, Slice 1 (docs/plans/design-html-output.md §CLI
    /// surface, "Cache key"): a PDF compile and an HTML compile of the exact
    /// same document/version/font must hash to DIFFERENT keys — otherwise a
    /// `--format html` run could hit a key a prior `--format pdf` run
    /// populated (or vice versa) and write the wrong-format bytes to the
    /// requested output.
    #[test]
    fn key_changes_with_output_format() {
        let dir = scratch();
        let a = dir.join("a.saty");
        std::fs::write(&a, b"document (||) '<>\n").unwrap();
        let pdf = hash_inputs([a.as_path()].into_iter(), "0.1.0", RustyfiVersion::DEFAULT, &a, None, OutputFormat::Pdf, None).unwrap();
        let html = hash_inputs([a.as_path()].into_iter(), "0.1.0", RustyfiVersion::DEFAULT, &a, None, OutputFormat::Html, None).unwrap();
        assert_ne!(pdf, html, "--format pdf and --format html must hash to different keys");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Phase-7c saphe solver, C3: the resolved dependency lock's digest must
    /// be part of the key — otherwise re-solving to a different locked
    /// version, with the document's own bytes unchanged, would silently keep
    /// serving a stale cached render.
    #[test]
    fn key_changes_with_deps_lock_digest() {
        let dir = scratch();
        let a = dir.join("a.saty");
        std::fs::write(&a, b"document (||) '<>\n").unwrap();
        let no_lock =
            hash_inputs([a.as_path()].into_iter(), "0.1.0", RustyfiVersion::DEFAULT, &a, None, OutputFormat::Pdf, None)
                .unwrap();
        let lock_a = hash_inputs(
            [a.as_path()].into_iter(),
            "0.1.0",
            RustyfiVersion::DEFAULT,
            &a,
            None,
            OutputFormat::Pdf,
            Some("digest-aaaa"),
        )
        .unwrap();
        let lock_b = hash_inputs(
            [a.as_path()].into_iter(),
            "0.1.0",
            RustyfiVersion::DEFAULT,
            &a,
            None,
            OutputFormat::Pdf,
            Some("digest-bbbb"),
        )
        .unwrap();
        assert_ne!(no_lock, lock_a, "an absent lock vs. a present one must differ");
        assert_ne!(lock_a, lock_b, "two different lock digests must hash to different keys");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Locate a real TrueType file for the font-identity test above, exactly
    /// like `crates/rustyfi-pdf/tests/ttf.rs`'s `find_regular_font`.
    fn find_test_font() -> Option<PathBuf> {
        if let Ok(output) = std::process::Command::new("fc-match")
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

    #[test]
    fn store_round_trips_pdf_and_counts() {
        let dir = scratch();
        let cache = Cache { dir: dir.clone() };
        let key = "deadbeef";
        assert!(cache.get(key).is_none(), "empty cache is a miss");
        cache.put(key, b"%PDF-1.7 fake", 3, 12).unwrap();
        let hit = cache.get(key).expect("stored key must hit");
        assert_eq!(hit.pdf, b"%PDF-1.7 fake");
        assert_eq!((hit.pages, hit.lines), (3, 12));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_sidecar_is_a_miss() {
        let dir = scratch();
        let cache = Cache { dir: dir.clone() };
        // Payload present but no `.meta`: must not hit (no counts to report).
        std::fs::write(cache.pdf_path("orphan"), b"%PDF-").unwrap();
        assert!(cache.get("orphan").is_none());
        std::fs::remove_dir_all(&dir).ok();
    }
}
