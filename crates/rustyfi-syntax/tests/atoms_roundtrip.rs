//! Does `Unparse` replay a parsed file's atoms EXACTLY — spans and all?
//!
//! `roundtrip.rs` next door answers a weaker question. It compares
//! `.map(|a| a.slot)` on both sides and says so in its own doc comment
//! ("spans aside"), so it would not notice a reordering that preserves the
//! token multiset, and it would not notice a span that came back wrong.
//!
//! That gap matters now. The CST-based formatter designed in
//! `docs/plans/formatter-cst/trivia-representation.md` gets a node's token list
//! — and with it every interior whitespace slot — by unparsing the node into a
//! collecting `Emitter`, the way `rustyfi_lsp::symbols::node_span` already gets
//! a node's extent. The trivia between two tokens is then addressed as
//! `src[prev.end.byte .. next.start.byte]`. So the formatter is correct only if
//! **every node's `Unparse` emits all of its atoms, in source order, carrying
//! the spans the lexer gave them**. Six leaves and three nodes hand-write
//! `Unparse` rather than deriving it, so this can drift, and if it drifts the
//! formatter moves comments to the wrong place or drops them.
//!
//! This test is that contract. It is deliberately in `rustyfi-syntax`, next to
//! the grammar it constrains, rather than in the formatter's own crate: the
//! property belongs to the parser, and a future grammar change should fail here
//! and not two crates away.
//!
//! The comparison is `Vec<Atom>` — token AND span — against `lex`, over every
//! file both corpora ship, for both grammars.

use std::path::{Path, PathBuf};

use rustyfi_syntax::cst::parse_file;
use rustyfi_syntax::cst_v1::parse_file_v1;
use rustyfi_syntax::token::Atom;
use rustyfi_syntax::{lex_with_version, RustyfiVersion};
use syan::parse::Unparse;

/// Replay a node's atoms in source order. This is the ~15-line bridge the
/// formatter design rests on, written here so the test exercises the real
/// thing rather than a lookalike.
fn atoms_of<T: Unparse<Atom> + ?Sized>(node: &T) -> Vec<Atom> {
    let mut out = Vec::<Atom>::new();
    node.unparse(&mut (&mut out))
        .expect("writing into a Vec cannot fail");
    out
}

/// How two atom streams differ, as a short human-readable report, or `None`
/// when they agree exactly.
///
/// Reports the FIRST disagreement rather than a count: a single dropped atom
/// shifts everything after it, so a count would read as catastrophic when one
/// token is wrong.
fn first_difference(want: &[Atom], got: &[Atom]) -> Option<String> {
    for (i, (w, g)) in want.iter().zip(got.iter()).enumerate() {
        if w.slot != g.slot {
            return Some(format!(
                "atom {i}: token differs — lexed {:?}, unparsed {:?}",
                w.slot, g.slot
            ));
        }
        if w.span != g.span {
            return Some(format!(
                "atom {i} ({:?}): span differs — lexed {}..{}, unparsed {}..{}",
                w.slot, w.span.start.byte, w.span.end.byte, g.span.start.byte, g.span.end.byte
            ));
        }
    }
    if want.len() != got.len() {
        return Some(format!(
            "length differs — lexed {} atoms, unparsed {}",
            want.len(),
            got.len()
        ));
    }
    None
}

fn corpus(dirs: &[&str]) -> Vec<PathBuf> {
    let root = repo_root();
    let mut out = Vec::new();
    for d in dirs {
        collect(&root.join(d), &mut out);
    }
    out.sort();
    out
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect(&p, out);
        } else if matches!(
            p.extension().and_then(|s| s.to_str()),
            Some("saty" | "satyh" | "satyg")
        ) {
            out.push(p);
        }
    }
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the crate sits two levels below the workspace root")
        .to_path_buf()
}

/// The sweep. Returns `(compared, unparseable)`.
///
/// A file that does not PARSE is counted and skipped, not failed: this test is
/// about `Unparse`, and a grammar gap is a different subject. But the count is
/// asserted against a ceiling, because "everything was skipped" is how a corpus
/// test goes vacuously green.
fn sweep(files: &[PathBuf], version: RustyfiVersion) -> (usize, usize) {
    assert!(
        files.len() > 20,
        "expected the bundled corpus, found {} files — is the checkout complete?",
        files.len()
    );
    let (mut compared, mut unparseable) = (0, 0);
    for path in files {
        let Ok(src) = std::fs::read_to_string(path) else {
            continue;
        };
        let Ok(lexed) = lex_with_version(&src, version) else {
            unparseable += 1;
            continue;
        };
        let replayed = match version {
            RustyfiVersion::V0_1 => match parse_file_v1(&src) {
                Ok(f) => atoms_of(&f),
                Err(_) => {
                    unparseable += 1;
                    continue;
                }
            },
            _ => match parse_file(&src) {
                Ok(f) => atoms_of(&f),
                Err(_) => {
                    unparseable += 1;
                    continue;
                }
            },
        };
        if let Some(diff) = first_difference(&lexed, &replayed) {
            panic!(
                "{}: Unparse does not replay the lexed atoms — {diff}\n\
                 This breaks the CST formatter's trivia addressing: it reads the \n\
                 whitespace between two tokens as src[prev.end..next.start], so a \n\
                 wrong span moves or drops a comment. See \n\
                 docs/plans/formatter-cst/trivia-representation.md, risk R5.",
                path.display()
            );
        }
        compared += 1;
    }
    (compared, unparseable)
}

#[test]
fn unparse_replays_every_atom_and_span_v006() {
    let files = corpus(&["lib-rustyfi/dist/packages", "layout-tests/corpus"]);
    let (compared, unparseable) = sweep(&files, RustyfiVersion::V0_0);
    eprintln!("0.0.6: {compared} files compared, {unparseable} unparseable");
    assert!(
        compared > 20,
        "only {compared} of {} files actually reached the comparison — this \
         sweep has gone vacuous",
        files.len()
    );
}

#[test]
fn unparse_replays_every_atom_and_span_v01() {
    let files = corpus(&["lib-rustyfi/dist-v01/packages"]);
    let (compared, unparseable) = sweep(&files, RustyfiVersion::V0_1);
    eprintln!("0.1: {compared} files compared, {unparseable} unparseable");
    assert!(
        compared > 20,
        "only {compared} of {} files actually reached the comparison — this \
         sweep has gone vacuous",
        files.len()
    );
}

/// A hand-written control, so the sweep cannot be the only thing keeping this
/// honest: the shapes whose `Unparse` is hand-written rather than derived.
#[test]
fn unparse_replays_the_hand_written_leaves() {
    for src in [
        "let x = 0x1F in x",
        "let x = 1.50 in x",
        "let s = ``a`` in s",
        "let s = #`a`# in s",
        "let l = 12pt in l",
        "let f = (+) in f",
        "@require: foo\nlet x = 1 in x",
        "let x = 1 in {a \\& b}",
        "let x = 1 in ${x + y}",
        "let x = 1 in '<+p{hi}>",
    ] {
        let lexed = lex_with_version(src, RustyfiVersion::V0_0)
            .unwrap_or_else(|e| panic!("{src:?} does not lex: {e:?}"));
        let file =
            parse_file(src).unwrap_or_else(|e| panic!("{src:?} does not parse: {e}"));
        let replayed = atoms_of(&file);
        if let Some(diff) = first_difference(&lexed, &replayed) {
            panic!("{src:?}: {diff}");
        }
    }
}
