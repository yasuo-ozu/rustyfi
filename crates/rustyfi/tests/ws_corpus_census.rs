//! SCRATCH HARNESS: how many bytes of the REAL corpus each normalisable
//! position actually covers, so `docs/plans/formatter-cst/`'s ranking is a
//! measurement rather than a guess.
//!
//! Classifies every byte of every corpus file into:
//!   - a token span (untouchable by span arithmetic), and within the
//!     whitespace-bearing token kinds, into "load-bearing first char of an
//!     inline whitespace run" vs "free interior";
//!   - an inter-token GAP, tagged with the lexical area it sits in.
//!
//! The area fold is `rustyfi-lsp`'s `area::AreaStack::advance` (that module is
//! `pub(crate)`, so the four push/pop lines are repeated here rather than
//! imported — they are checked against it by inspection, see the doc there).
//!
//!     RUSTFLAGS="-C linker-features=-lld" cargo test -p rustyfi \
//!         --test ws_corpus_census -- --ignored --nocapture

use std::path::{Path, PathBuf};

use rustyfi_syntax::{RustyfiVersion, Token};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Area {
    Program,
    Inline,
    Block,
    Math,
}

fn advance(stack: &mut Vec<Area>, tok: &Token) {
    match tok {
        Token::BHorzGrp => stack.push(Area::Inline),
        Token::BVertGrp => stack.push(Area::Block),
        Token::BMathGrp => stack.push(Area::Math),
        Token::LParen | Token::BList | Token::BRecord | Token::OpenModule(_) => {
            stack.push(Area::Program)
        }
        Token::EHorzGrp
        | Token::EVertGrp
        | Token::EMathGrp
        | Token::RParen
        | Token::EList
        | Token::ERecord => {
            if stack.len() > 1 {
                stack.pop();
            }
        }
        _ => {}
    }
}

#[derive(Default, Debug)]
struct Census {
    files: usize,
    bytes: usize,
    lines: usize,
    /// Inter-token gap bytes, by the area the gap sits in.
    gap_program: usize,
    gap_inline: usize,
    gap_block: usize,
    gap_math: usize,
    /// Whitespace bytes swallowed INTO a token's span, by token kind.
    ws_in_space_break: usize,
    /// … of which the load-bearing first character of the run.
    ws_space_break_first: usize,
    ws_in_bhorzgrp: usize,
    ws_in_ehorzgrp: usize,
    ws_in_sep_item: usize,
    /// Bytes inside a string/verbatim literal token (never touchable).
    literal_bytes: usize,
    /// Bytes inside a header token (never touchable past `@name:`).
    header_bytes: usize,
    /// Program gaps that stay on one line and are 2+ columns wide (alignment).
    lines_with_alignment: usize,
    /// Comment bytes, by area.
    comment_program: usize,
    comment_textish: usize,
}

fn is_ws(b: u8) -> bool {
    b == b' ' || b == b'\t' || b == b'\n' || b == b'\r'
}

fn census(src: &str, version: RustyfiVersion, c: &mut Census) -> bool {
    let Ok(atoms) = rustyfi_syntax::lex_with_version(src, version) else {
        return false;
    };
    c.files += 1;
    c.bytes += src.len();
    c.lines += src.lines().count();

    let bytes = src.as_bytes();
    let mut stack = vec![Area::Program];
    let mut cursor = 0usize;
    for a in &atoms {
        let start = a.span.start.byte.max(cursor);
        let area = *stack.last().unwrap();
        if start > cursor {
            let gap = &src[cursor..start];
            let n = gap.len();
            match area {
                Area::Program => c.gap_program += n,
                Area::Inline => c.gap_inline += n,
                Area::Block => c.gap_block += n,
                Area::Math => c.gap_math += n,
            }
            let cmt: usize = gap
                .lines()
                .map(|l| l.find('%').map(|i| l.len() - i).unwrap_or(0))
                .sum();
            if area == Area::Program {
                c.comment_program += cmt;
                // An INTERIOR alignment run: a program gap that stays on one
                // line (no break) and is 2+ columns wide — `val x   : t`,
                // `| f init []        = init`, and the gap before a trailing
                // comment. This is the population `format.rs`'s module comment
                // refuses to collapse.
                let no_break = !gap.contains('\n') && !gap.contains('\r');
                if no_break && gap.chars().filter(|ch| *ch == ' ' || *ch == '\t').count() >= 2 {
                    c.lines_with_alignment += 1;
                }
            } else {
                c.comment_textish += cmt;
            }
        }
        let span = &src[a.span.start.byte.min(src.len())..a.span.end.byte.min(src.len())];
        match &a.slot {
            Token::Space | Token::Break => {
                let n = span.bytes().filter(|b| is_ws(*b)).count();
                c.ws_in_space_break += n;
                if !span.is_empty() && is_ws(bytes[a.span.start.byte]) {
                    c.ws_space_break_first += 1;
                }
                let cmt: usize = span
                    .lines()
                    .map(|l| l.find('%').map(|i| l.len() - i).unwrap_or(0))
                    .sum();
                c.comment_textish += cmt;
            }
            Token::BHorzGrp => c.ws_in_bhorzgrp += span.bytes().filter(|b| is_ws(*b)).count(),
            Token::EHorzGrp | Token::EVertGrp | Token::BVertGrp => {
                c.ws_in_ehorzgrp += span.bytes().filter(|b| is_ws(*b)).count()
            }
            Token::Sep | Token::Item(_) => {
                c.ws_in_sep_item += span.bytes().filter(|b| is_ws(*b)).count()
            }
            Token::Literal { .. } | Token::CodeText(_) => c.literal_bytes += span.len(),
            Token::HeaderRequire(_)
            | Token::HeaderImport(_)
            | Token::HeaderStage0
            | Token::HeaderStage1
            | Token::HeaderPersistent0 => c.header_bytes += span.len(),
            _ => {}
        }
        advance(&mut stack, &a.slot);
        cursor = a.span.end.byte.max(cursor);
    }
    true
}

fn collect(root: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(root) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            if p.file_name().is_some_and(|n| n == "target" || n == ".git") {
                continue;
            }
            collect(&p, out);
        } else if p
            .extension()
            .is_some_and(|x| x == "saty" || x == "satyh" || x == "satyg")
        {
            out.push(p);
        }
    }
}

#[test]
#[ignore = "scratch census; run with --ignored --nocapture"]
fn corpus_whitespace_census() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    collect(&repo.join("lib-rustyfi/dist/packages"), &mut files);
    collect(&repo.join("layout-tests/corpus"), &mut files);
    files.sort();

    let mut c = Census::default();
    let mut declined = 0usize;
    for f in &files {
        let Ok(src) = std::fs::read_to_string(f) else {
            continue;
        };
        if !census(&src, RustyfiVersion::V0_0, &mut c) {
            declined += 1;
        }
    }
    println!("{} files found, {declined} did not lex under V0_0", files.len());
    println!("{c:#?}");
    let norm_today = c.gap_program;
    let norm_new = c.gap_block + c.gap_math + c.gap_inline;
    let free_inside = c.ws_in_space_break - c.ws_space_break_first
        + c.ws_in_bhorzgrp
        + c.ws_in_ehorzgrp
        + c.ws_in_sep_item;
    println!(
        "\nreachable today (program gaps): {norm_today} bytes\n\
         newly reachable as GAPS (block+math+inline-area gaps): {norm_new}\n\
         insignificant but INSIDE a token span: {free_inside}\n\
         untouchable literal bytes: {}\n\
         untouchable header bytes: {}",
        c.literal_bytes, c.header_bytes
    );
}
