//! Corpus regression test: run our lexer and parser over real-world SATySFi
//! packages (the author's `github.com/yasuo-ozu/satysfi-*` repos) and hold the
//! line against regressions.
//!
//! This port implements a **v0.0.x subset** with no stdlib loading, so real
//! packages do not compile end-to-end — most do not even fully parse (they use
//! grammar our subset hasn't reached yet). So this harness does NOT assert
//! "must compile"; it asserts what is actually meaningful for a growing
//! front-end:
//!
//! 1. **Robustness (hard gate):** the lexer and parser must never *panic* on
//!    real input — a crash is always a bug, regardless of feature coverage.
//! 2. **Lex coverage floor:** our lexer is a faithful port of `lexer.mll`, so
//!    it handles nearly all real v0.0.x source; the success *rate* must stay
//!    above [`LEX_RATE_FLOOR`]. A drop means a lexer regression.
//! 3. **Parse ratchet:** the number of files that fully parse is tracked and
//!    printed; as the parser grows (the phase-2 grammar remainder), bump
//!    [`PARSE_RATE_FLOOR`] to lock in the gain. It starts at 0 — full packages
//!    exceed the subset today — so this tier only *ratchets up*, never blocks.
//!
//! The corpus is external, so the harness is driven by `$SATYSFI_CORPUS_DIR`
//! (a `:`-separated list of repo roots). With the variable unset the test
//! skips, so a plain `cargo test` (no corpus checked out) stays green; CI sets
//! it after cloning the packages. Rates (not absolute counts) are used so the
//! gate survives the external repos gaining or losing files.

use std::path::{Path, PathBuf};

/// Minimum fraction of corpus files that must lex without error. Current rate
/// is ~0.89 (24/27; the failures are all the unsupported `@`-positioned string
/// literal). The floor leaves headroom for the corpus shifting under us.
const LEX_RATE_FLOOR: f64 = 0.80;

/// Minimum fraction that must fully parse. Real packages exceed the subset
/// grammar today, so this is 0.0 — raise it as the parser grows to lock in
/// coverage (a ratchet, never a false-positive block).
const PARSE_RATE_FLOOR: f64 = 0.0;

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for entry in rd.flatten() {
        let p = entry.path();
        let s = p.to_string_lossy();
        // Skip opam build trees, VCS, and cargo output.
        if s.contains("/_opam/") || s.contains("/.git/") || s.contains("/target/") {
            continue;
        }
        if p.is_dir() {
            collect(&p, out);
        } else if matches!(
            p.extension().and_then(|e| e.to_str()),
            Some("saty") | Some("satyh") | Some("satyg")
        ) {
            out.push(p);
        }
    }
}

/// Run one closure under `catch_unwind`, mapping a panic to an `Err` marker so
/// a crash is a recorded failure rather than aborting the whole harness.
fn guarded<T>(f: impl FnOnce() -> Result<T, String> + std::panic::UnwindSafe) -> Result<T, String> {
    // Silence the default panic hook for the duration so a caught panic does
    // not spam the log; the location is still reported via the Err below.
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let r = std::panic::catch_unwind(f);
    std::panic::set_hook(prev);
    match r {
        Ok(inner) => inner,
        Err(_) => Err("PANIC".to_string()),
    }
}

#[test]
fn corpus_lex_and_parse() {
    let Ok(roots) = std::env::var("SATYSFI_CORPUS_DIR") else {
        eprintln!(
            "corpus: SATYSFI_CORPUS_DIR unset — skipping (set it to a ':'-separated \
             list of satysfi-* repo roots to run the corpus regression)"
        );
        return;
    };

    let mut files = Vec::new();
    for root in roots.split(':').filter(|s| !s.is_empty()) {
        collect(Path::new(root), &mut files);
    }
    files.sort();
    assert!(
        !files.is_empty(),
        "SATYSFI_CORPUS_DIR was set but no .saty/.satyh/.satyg files were found under {roots:?}"
    );

    let mut lex_ok = 0usize;
    let mut parse_ok = 0usize;
    let mut panics: Vec<String> = Vec::new();
    let mut lex_fail: Vec<String> = Vec::new();
    let mut parse_fail: Vec<String> = Vec::new();

    for f in &files {
        let src = match std::fs::read_to_string(f) {
            Ok(s) => s,
            Err(_) => continue, // non-UTF-8 or unreadable: not our concern here
        };
        let rel = f.display().to_string();

        match guarded(|| satysfi_syntax::lex(&src).map_err(|e| e.to_string())) {
            Ok(_) => {
                lex_ok += 1;
                match guarded(|| satysfi_syntax::parse_file(&src).map(|_| ()).map_err(|e| e.to_string())) {
                    Ok(()) => parse_ok += 1,
                    Err(ref m) if m == "PANIC" => panics.push(format!("parse {rel}")),
                    Err(m) => parse_fail.push(format!("{rel}: {}", first_line(&m))),
                }
            }
            Err(ref m) if m == "PANIC" => panics.push(format!("lex {rel}")),
            Err(m) => lex_fail.push(format!("{rel}: {}", first_line(&m))),
        }
    }

    let total = files.len();
    let lex_rate = lex_ok as f64 / total as f64;
    let parse_rate = parse_ok as f64 / total as f64;

    eprintln!("\n==== SATySFi corpus regression ====");
    eprintln!("files: {total}");
    eprintln!("lex:   {lex_ok}/{total} ({:.1}%)", lex_rate * 100.0);
    eprintln!("parse: {parse_ok}/{total} ({:.1}%)", parse_rate * 100.0);
    if !lex_fail.is_empty() {
        eprintln!("\n-- lex failures ({}) --", lex_fail.len());
        for l in &lex_fail {
            eprintln!("  {l}");
        }
    }
    if !parse_fail.is_empty() {
        eprintln!("\n-- parse failures ({}, first 20) --", parse_fail.len());
        for l in parse_fail.iter().take(20) {
            eprintln!("  {l}");
        }
    }
    eprintln!("===================================\n");

    // 1. Robustness: never panic on real input.
    assert!(
        panics.is_empty(),
        "the frontend PANICKED on real SATySFi source (always a bug):\n  {}",
        panics.join("\n  ")
    );

    // 2. Lex coverage floor (regression guard).
    assert!(
        lex_rate >= LEX_RATE_FLOOR,
        "lex success rate {:.1}% fell below the floor {:.0}% — a lexer regression \
         (was ~89% at baseline). See the failures above.",
        lex_rate * 100.0,
        LEX_RATE_FLOOR * 100.0,
    );

    // 3. Parse ratchet (raise the floor as the grammar grows).
    assert!(
        parse_rate >= PARSE_RATE_FLOOR,
        "parse success rate {:.1}% fell below the floor {:.0}%.",
        parse_rate * 100.0,
        PARSE_RATE_FLOOR * 100.0,
    );
}

fn first_line(s: &str) -> &str {
    s.lines().next().unwrap_or(s)
}
