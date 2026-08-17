//! Compile-cache behaviour, driven through the *built* `rustyfi` binary
//! (plan: content-addressed cache so an unchanged recompile is near-instant).
//!
//! Every test points `--cache-dir` at its own unique temp directory, so the
//! cache never touches the developer's real `~/.cache/rustyfi` and no two
//! tests can interfere. (Key correctness — a SHA-256 over the resolved input
//! bytes — already rules out false hits between distinct documents; the
//! per-test dir is belt-and-braces plus a clean slate for the miss/hit
//! assertions.)

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

/// The built `rustyfi` binary (cargo hands this to the integration tests
/// of the crate that defines the `[[bin]]`).
fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_rustyfi"))
}

/// This repo's `lib-rustyfi/`, resolved from the crate manifest dir exactly as
/// `tests/e2e.rs` does — the fixtures `@require: stdja-mini` from there.
fn repo_lib_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lib-rustyfi")
}

fn minimal_fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/minimal.saty")
}

/// A fresh, unique temp directory for one test (created).
fn tmpdir(tag: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let p = std::env::temp_dir().join(format!(
        "rustyfi-cli-cache-{tag}-{}-{}-{}",
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

/// Compile `input` to `output`, resolving packages against this repo's
/// `lib-rustyfi/` and using `cache_dir` for the compile cache. `extra` carries
/// flags like `--no-cache`.
fn compile(input: &Path, output: &Path, cache_dir: &Path, extra: &[&str]) -> Output {
    Command::new(bin())
        .arg(input)
        .args(["-o".as_ref(), output.as_os_str()])
        .args(["--lib-root".as_ref(), repo_lib_root().as_os_str()])
        .args(["--cache-dir".as_ref(), cache_dir.as_os_str()])
        .args(extra)
        .output()
        .expect("spawn rustyfi")
}

/// Whether a compile run reported a cache hit (the `(cached)` marker on the
/// "output written" status line, which goes to stderr).
fn was_cached(out: &Output) -> bool {
    String::from_utf8_lossy(&out.stderr).contains("(cached)")
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

/// How many `<hex>.pdf` cache payloads exist under `dir` (0 if the dir does
/// not exist).
fn cached_pdf_count(dir: &Path) -> usize {
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(Result::ok)
                .filter(|e| e.path().extension().is_some_and(|x| x == "pdf"))
                .count()
        })
        .unwrap_or(0)
}

/// First run misses and renders; the second run hits and writes byte-for-byte
/// the same PDF, skipping compile+render.
#[test]
fn cache_miss_then_hit_is_byte_identical() {
    let work = tmpdir("miss-hit");
    let cache_dir = work.join("cache");
    let out = work.join("out.pdf");

    let first = compile(&minimal_fixture(), &out, &cache_dir, &[]);
    assert_ok(&first, "first run");
    assert!(!was_cached(&first), "first run must be a miss (rendered)");
    let bytes_first = std::fs::read(&out).expect("first run wrote a PDF");
    assert!(bytes_first.starts_with(b"%PDF-"), "not a PDF");
    assert_eq!(
        cached_pdf_count(&cache_dir),
        1,
        "miss must populate the cache"
    );

    let second = compile(&minimal_fixture(), &out, &cache_dir, &[]);
    assert_ok(&second, "second run");
    assert!(was_cached(&second), "second run must be a cache hit");
    let bytes_second = std::fs::read(&out).expect("second run wrote a PDF");

    assert_eq!(
        bytes_first, bytes_second,
        "a cache hit must reproduce the exact bytes of the miss"
    );

    std::fs::remove_dir_all(&work).ok();
}

/// `--no-cache` never reads nor writes: no `(cached)` marker on a repeat run,
/// and the cache directory is never even created.
#[test]
fn no_cache_flag_never_touches_the_cache() {
    let work = tmpdir("no-cache");
    // A path that does not exist yet: if the run writes the cache, it appears.
    let cache_dir = work.join("never-created");
    let out = work.join("out.pdf");

    let first = compile(&minimal_fixture(), &out, &cache_dir, &["--no-cache"]);
    assert_ok(&first, "first --no-cache run");
    assert!(!was_cached(&first), "first --no-cache run cannot be cached");

    let second = compile(&minimal_fixture(), &out, &cache_dir, &["--no-cache"]);
    assert_ok(&second, "second --no-cache run");
    assert!(
        !was_cached(&second),
        "a second --no-cache run must still re-render, never hit"
    );

    assert!(
        !cache_dir.exists(),
        "--no-cache must not create or write the cache directory"
    );

    std::fs::remove_dir_all(&work).ok();
}

/// Changing one input byte changes the key: the next run misses again, and a
/// further unchanged run hits under the *new* key.
#[test]
fn editing_the_input_invalidates_the_cache() {
    let work = tmpdir("invalidate");
    let cache_dir = work.join("cache");
    let out = work.join("out.pdf");

    // A writable copy of the fixture (still `@require: stdja-mini`).
    let src = work.join("doc.saty");
    std::fs::copy(minimal_fixture(), &src).unwrap();

    let first = compile(&src, &out, &cache_dir, &[]);
    assert_ok(&first, "first run");
    assert!(!was_cached(&first), "first run is a miss");

    let warm = compile(&src, &out, &cache_dir, &[]);
    assert_ok(&warm, "warm run");
    assert!(was_cached(&warm), "unchanged rerun hits");

    // Mutate one paragraph's text — still a valid document, different bytes.
    let text = std::fs::read_to_string(&src).unwrap();
    let edited = text.replace("Hello, world!", "Hello, cache!");
    assert_ne!(edited, text, "the edit must actually change the source");
    std::fs::write(&src, edited).unwrap();

    let after_edit = compile(&src, &out, &cache_dir, &[]);
    assert_ok(&after_edit, "post-edit run");
    assert!(
        !was_cached(&after_edit),
        "a changed input byte must invalidate the key (miss again)"
    );
    assert_eq!(
        cached_pdf_count(&cache_dir),
        2,
        "the edit stores a second payload under the new key"
    );

    // The new content now caches under its own key.
    let rewarm = compile(&src, &out, &cache_dir, &[]);
    assert_ok(&rewarm, "post-edit rerun");
    assert!(was_cached(&rewarm), "the edited document now hits too");

    std::fs::remove_dir_all(&work).ok();
}

/// Timing demonstration (run with `cargo test -p rustyfi-cli --ignored`): a
/// warm hit is dramatically faster than a cold miss, since it skips
/// elaborate/typecheck/eval/render. Prints both wall-clock times.
#[test]
#[ignore = "timing demonstration; run explicitly with --ignored"]
fn cache_hit_is_much_faster_than_miss() {
    use std::time::Instant;

    let work = tmpdir("timing");
    let cache_dir = work.join("cache");
    let out = work.join("out.pdf");

    let t0 = Instant::now();
    let miss = compile(&minimal_fixture(), &out, &cache_dir, &[]);
    let miss_dur = t0.elapsed();
    assert_ok(&miss, "cold run");
    assert!(!was_cached(&miss));

    let t1 = Instant::now();
    let hit = compile(&minimal_fixture(), &out, &cache_dir, &[]);
    let hit_dur = t1.elapsed();
    assert_ok(&hit, "warm run");
    assert!(was_cached(&hit));

    eprintln!("cold miss: {miss_dur:?}   warm hit: {hit_dur:?}");
    assert!(
        hit_dur < miss_dur,
        "a cache hit ({hit_dur:?}) should beat a full compile ({miss_dur:?})"
    );

    std::fs::remove_dir_all(&work).ok();
}
