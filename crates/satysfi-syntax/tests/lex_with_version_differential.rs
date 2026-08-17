//! Differential guardrail for the S4/S5 lexer seam (`lex_with_version`):
//! `lex(src)` and `lex_with_version(src, V0_0_6)` must be **byte-identical**
//! for every 0.0.6 source, since `lex` is now defined as a thin wrapper over
//! `lex_with_version(_, V0_0_6)` (see `lexer.rs`). This is the proof that
//! threading a `version` field through `Lexer` and adding the Slice-1
//! `rec`/`inline`/`block` keyword gate never perturbs existing 0.0.6 lexing.
//!
//! Two corpora are exercised:
//! 1. Every real vendored 0.0.6 package (`lib-satysfi/dist/packages/`) and
//!    CLI fixture (`crates/satysfi-cli/tests/fixtures/`) — real-world 0.0.6
//!    source, walked recursively.
//! 2. A hardcoded set of small snippets covering every lexer mode (program,
//!    vertical, horizontal, active, math) and every new Slice-1 keyword
//!    spelling used as a plain identifier — self-contained, so the guardrail
//!    still holds even if the fixture directories are ever reorganized.

use satysfi_syntax::version::SatysfiVersion;
use satysfi_syntax::{lex, lex_with_version};
use std::path::{Path, PathBuf};

fn assert_same(src: &str, label: &str) {
    let via_lex = lex(src);
    let via_versioned = lex_with_version(src, SatysfiVersion::V0_0_6);
    match (via_lex, via_versioned) {
        (Ok(a), Ok(b)) => {
            assert_eq!(a, b, "token stream mismatch for {label}");
        }
        (Err(a), Err(b)) => {
            // Both must fail, and at the same position/message.
            assert_eq!(
                a.to_string(),
                b.to_string(),
                "lex error mismatch for {label}"
            );
        }
        (a, b) => panic!(
            "lex(..) and lex_with_version(.., V0_0_6) disagree on success/failure for \
             {label}: lex={a:?} lex_with_version={b:?}"
        ),
    }
}

fn collect_saty_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for entry in rd.flatten() {
        let p = entry.path();
        if p.is_dir() {
            collect_saty_files(&p, out);
        } else if matches!(
            p.extension().and_then(|e| e.to_str()),
            Some("saty") | Some("satyh") | Some("satyg")
        ) {
            out.push(p);
        }
    }
}

#[test]
fn lex_and_lex_with_version_v0_0_6_agree_on_real_fixtures() {
    let roots = [
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../lib-satysfi/dist/packages"),
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../crates/satysfi-cli/tests/fixtures"),
    ];
    let mut files = Vec::new();
    for root in roots {
        collect_saty_files(Path::new(root), &mut files);
    }
    files.sort();
    assert!(
        files.len() >= 5,
        "expected to find at least 5 .saty/.satyh fixtures under {roots:?}, found {}",
        files.len()
    );

    let mut checked = 0usize;
    for f in &files {
        let src = std::fs::read_to_string(f)
            .unwrap_or_else(|e| panic!("{}: {e}", f.display()));
        assert_same(&src, &f.display().to_string());
        checked += 1;
    }
    assert!(checked >= 5, "expected to check at least 5 files, got {checked}");
}

#[test]
fn lex_and_lex_with_version_v0_0_6_agree_on_mode_coverage_snippets() {
    let snippets = [
        "let x = 3 in x",
        "let-rec f n = f n in f",
        "let-inline ctx \\emph it = read-inline ctx it",
        "let-block ctx +p it = line-break true true ctx it",
        "let-mutable c <- 0 in c",
        "module M = struct\nlet x = 1\nend\nopen M in M.x",
        "type t = | A | B of int\nin 0",
        "@require: stdjabook\n@import: local\nlet x = 1 in x",
        "@stage: 1\n3",
        "{ Hello, \\emph{world}! #name; }",
        "'< +p { hi } +q { there } >",
        "${x^2 + \\frac{a}{b} + #y}",
        "match x with | 0 -> `a` | n when n -> `b` | _ -> `c`",
        "while !c < 3 do c <- !c + 1",
        "a before b",
        "(| title = {T}; size = 3pt |)",
        "[1; 2; 3]",
        // Every Slice-1 keyword spelling, used as a plain identifier under
        // 0.0.6 — the crux of the version-gating change: these three words
        // must lex identically to before (as `Var`) when no version (or
        // V0_0_6) is requested.
        "let rec = 1 in rec",
        "let inline = 1 in inline",
        "let block = 1 in block",
        "rec + inline + block",
    ];
    for src in snippets {
        assert_same(src, src);
    }
}
