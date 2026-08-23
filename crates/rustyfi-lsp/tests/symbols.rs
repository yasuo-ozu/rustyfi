//! `document_symbols` over both generations.
//!
//! Most assertions here are made against a **rendered outline** — one line
//! per symbol, indented by depth, `name [kind] detail` — rather than against
//! a hand-built `Vec<Symbol>`. Two reasons: the expected value reads as the
//! thing an editor would show, and the shape of the tree (which is the whole
//! point of `DocumentSymbol` over `SymbolInformation`) is visible in the
//! test source rather than buried in nested constructors.
//!
//! Ranges are checked separately, because they are the part a reader cannot
//! eyeball: [`ranges_are_utf16_and_well_formed`] runs over every file in the
//! corpus, and the targeted tests pin the two failure modes that a
//! byte-offset implementation passes every ASCII test with.

use rustyfi_lsp::{document_symbols, document_symbols_auto, RustyfiVersion, Symbol, SymbolKind};

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// `name [Kind] detail`, one symbol per line, two spaces per level of
/// nesting, with a trailing newline. Empty for an empty outline.
fn outline(syms: &[Symbol]) -> String {
    let mut out = String::new();
    render(syms, 0, &mut out);
    out
}

fn render(syms: &[Symbol], depth: usize, out: &mut String) {
    for s in syms {
        out.push_str(&"  ".repeat(depth));
        out.push_str(&s.name);
        out.push_str(&format!(" [{:?}]", s.kind));
        if let Some(d) = &s.detail {
            out.push(' ');
            out.push_str(d);
        }
        out.push('\n');
        render(&s.children, depth + 1, out);
    }
}

/// The first symbol anywhere in the tree with this name.
fn find<'a>(syms: &'a [Symbol], name: &str) -> &'a Symbol {
    fn go<'a>(syms: &'a [Symbol], name: &str) -> Option<&'a Symbol> {
        for s in syms {
            if s.name == name {
                return Some(s);
            }
            if let Some(hit) = go(&s.children, name) {
                return Some(hit);
            }
        }
        None
    }
    go(syms, name).unwrap_or_else(|| panic!("no symbol named `{name}` in:\n{}", outline(syms)))
}

fn count(syms: &[Symbol]) -> usize {
    syms.iter().map(|s| 1 + count(&s.children)).sum()
}

// ---------------------------------------------------------------------------
// 0.0.6
// ---------------------------------------------------------------------------

/// Every declaration form 0.0.6 has, in one library, as one outline.
///
/// Read the expected value as the symbol pane: the two headers, then the
/// prelude, then `Mod`'s signature and body folded under `Mod`.
#[test]
fn every_0_0_6_declaration_form_shows_up() {
    let src = r#"@require: stdjabook
@import: ./local

let plain = 1
let curried a b = a
let-rec even n = odd n
  and odd n = even n
let-mutable counter <- 0
type colour = Red | Green | Blue
type point = length * length
let-inline ctx \emph it = read-inline ctx it
let-block ctx +para it = read-block ctx it
let-math \frac a b = a

module Mod : sig
  val helper : int -> int
  direct \cmd : [inline-text] inline-cmd
  direct +blk : [block-text] block-cmd
  type opaque
end = struct
  let helper x = x
  let-inline ctx \cmd it = read-inline ctx it
  let-block ctx +blk it = read-block ctx it
  type opaque = int
end
"#;
    assert_eq!(
        outline(&document_symbols(src, RustyfiVersion::V0_0)),
        "\
stdjabook [Package] @require:
./local [File] @import:
plain [Variable] let
curried [Function] let
even [Function] let-rec
odd [Function] let-rec
counter [Variable] let-mutable
colour [Enum] type
  Red [EnumMember]
  Green [EnumMember]
  Blue [EnumMember]
point [TypeParameter] type
\\emph [Function] let-inline
+para [Function] let-block
\\frac [Function] let-math
Mod [Module] module
  sig [Interface]
    helper [Function] val
    \\cmd [Function] direct
    +blk [Function] direct
    opaque [TypeParameter] type
  helper [Function] let
  \\cmd [Function] let-inline
  +blk [Function] let-block
  opaque [TypeParameter] type
"
    );
}

/// A module's members are its *children*. This is the difference between an
/// outline and a wall of names, so it gets its own assertion rather than
/// riding on the big one above.
#[test]
fn a_0_0_6_module_owns_its_members() {
    let src = "module A = struct\n  let x = 1\n  module B = struct\n    let y = 2\n  end\nend\n";
    let syms = document_symbols(src, RustyfiVersion::V0_0);
    assert_eq!(syms.len(), 1, "one top-level symbol, the module");
    assert_eq!(
        outline(&syms),
        "A [Module] module\n  x [Variable] let\n  B [Module] module\n    y [Variable] let\n"
    );
}

/// A 0.0.6 *document* keeps its declarations in the prelude, before `in` —
/// but `let … in` after it is legal and the corpus writes it (an
/// `xpath-doc.saty` binds its context that way).
#[test]
fn a_0_0_6_document_body_spine_is_walked() {
    let src = "@require: stdja\n\nlet-inline ctx \\c it = it\n\nlet ctx = get-initial-context 16pt in\nopen Foo in\nlet-mutable n <- 0 in\ndocument (||) '<>\n";
    assert_eq!(
        outline(&document_symbols(src, RustyfiVersion::V0_0)),
        "\
stdja [Package] @require:
\\c [Function] let-inline
ctx [Variable] let
n [Variable] let-mutable
"
    );
}

// ---------------------------------------------------------------------------
// 0.1
// ---------------------------------------------------------------------------

/// Every declaration form 0.1 has, in one library.
///
/// Note the stage qualifiers: `val ~x` and `val persistent ~x` are the one
/// thing about a 0.1 binding that its name does not tell you, so they are
/// spelled out in the detail.
#[test]
fn every_0_1_declaration_form_shows_up() {
    let src = r#"use package Stdlib
use Local of `./local`

module Lib :> sig
  val helper : int -> int
  val ~staged : int
  val \cmd : inline [inline-text]
  val +blk : block [block-text]
  type opaque :: o
  type visible = int
  module Inner : sig
    val deep : int
  end
  signature Nested = sig
    val nested : int
  end
end = struct
  val helper x = x
  val ~staged = 1
  val persistent ~kept = 2
  val rec ping x = pong x
    and pong x = ping x
  val mutable counter <- 0
  val inline ctx \cmd it = read-inline ctx it
  val block ctx +blk it = read-block ctx it
  val math ctx \frac a b = a
  type tree = Leaf | Node of int
  type alias = int
  signature S = sig
    val member : int
  end
  module Inner = struct
    val deep = 1
  end
  include Other
end
"#;
    assert_eq!(
        outline(&document_symbols(src, RustyfiVersion::V0_1)),
        "\
Stdlib [Package] use package
Local [File] use … of
Lib [Module] module
  sig [Interface]
    helper [Function] val
    staged [Variable] val (stage 0)
    \\cmd [Function] val
    +blk [Function] val
    opaque [TypeParameter] type
    visible [TypeParameter] type
    Inner [Module] module
      deep [Variable] val
    Nested [Interface] signature
      nested [Variable] val
  helper [Function] val
  staged [Variable] val (stage 0)
  kept [Variable] val (persistent)
  ping [Function] val rec
  pong [Function] val rec
  counter [Variable] val mutable
  \\cmd [Function] val inline
  +blk [Function] val block
  \\frac [Function] val math
  tree [Enum] type
    Leaf [EnumMember]
    Node [EnumMember]
  alias [TypeParameter] type
  S [Interface] signature
    member [Variable] val
  Inner [Module] module
    deep [Variable] val
  Other [Module] include
"
    );
}

/// A 0.1 *document* has no top-level binding sequence at all — every `let`
/// chains its own `in` — so the whole outline is the body's spine. Getting
/// this wrong means a 0.1 document shows nothing but its headers.
#[test]
fn a_0_1_document_declares_everything_in_its_let_spine() {
    let src = r#"use package Stdlib

let answer = 41 in
let rec even n = odd n and odd n = even n in
let mutable count <- 0 in
let open Stdlib in
let (a, b) = (1, 2) in
let show x = x in
document (||) '<>
"#;
    assert_eq!(
        outline(&document_symbols(src, RustyfiVersion::V0_1)),
        "\
Stdlib [Package] use package
answer [Variable] let
even [Function] let rec
odd [Function] let rec
count [Variable] let mutable
show [Function] let
"
    );
}

/// One declaration can name several things. A mutually recursive `type … and
/// …` chain — `satysfi-base`'s `stream.satyg` writes one — must put its
/// clauses beside each other rather than nesting the second inside the first,
/// both in a `struct` and in a `sig`.
#[test]
fn an_and_chained_type_names_every_clause_as_a_sibling() {
    let src = "module M :> sig\n  type a = int\n  and b = bool\nend = struct\n  type c = int\n  and d = bool\nend\n";
    assert_eq!(
        outline(&document_symbols(src, RustyfiVersion::V0_1)),
        "\
M [Module] module
  sig [Interface]
    a [TypeParameter] type
    b [TypeParameter] type
  c [TypeParameter] type
  d [TypeParameter] type
"
    );

    // And the 0.0.6 spelling of the same thing.
    let src = "type a = int\n  and b = bool\n";
    assert_eq!(
        outline(&document_symbols(src, RustyfiVersion::V0_0)),
        "a [TypeParameter] type\nb [TypeParameter] type\n"
    );
}

/// A functor's members are its codomain's — `set.satyg` and `map.satyg`
/// declare theirs as `module Make : (Ord : Ord) -> sig … end`, and declining
/// to look through the arrow would leave both packages declaring nothing.
#[test]
fn a_functor_contributes_its_bodys_members() {
    let src = "module Set :> sig\n  module Make : (Ord : Ord) -> sig\n    val empty : int\n  end\nend = struct\n  module Make = fun (Ord : Ord) -> struct\n    val empty = 0\n  end\nend\n";
    assert_eq!(
        outline(&document_symbols(src, RustyfiVersion::V0_1)),
        "\
Set [Module] module
  sig [Interface]
    Make [Module] module
      empty [Variable] val
  Make [Module] module
    empty [Variable] val
"
    );
}

// ---------------------------------------------------------------------------
// Version detection
// ---------------------------------------------------------------------------

/// A `module M = struct` head is deliberately no version signal, so the
/// automatic entry point has to reach the same conclusion `analyze_detected`
/// does — via the same code. Under the 0.0.6 grammar this file's `val`
/// bindings are not declarations at all, so the wrong answer is not a
/// slightly different outline, it is an empty one.
#[test]
fn a_signal_free_0_1_library_is_read_as_0_1() {
    let src = "module Lib = struct\n  val f x = x\n  type t = int\nend\n";
    assert_eq!(
        outline(&document_symbols_auto(src)),
        "Lib [Module] module\n  f [Function] val\n  t [TypeParameter] type\n"
    );
    // And the 0.0.6 reading of the same text really is empty, so the test
    // above is not passing by coincidence.
    assert!(document_symbols(src, RustyfiVersion::V0_0).is_empty());
}

/// An explicit generation is taken literally, exactly as `analyze` does —
/// no fallback to the reading that happens to work better.
///
/// `let-inline` is one token in 0.0.6 and no token at all in 0.1 (which
/// spells it `val inline`), so the two readings of this file genuinely
/// differ: 0.0.6 sees a command binding, 0.1 sees only the header it also
/// accepts.
#[test]
fn an_explicit_generation_is_obeyed() {
    let src = "@require: list\n\nlet-inline ctx \\emph it = it\n";
    assert_eq!(
        outline(&document_symbols(src, RustyfiVersion::V0_0)),
        "list [Package] @require:\n\\emph [Function] let-inline\n"
    );
    assert_eq!(
        outline(&document_symbols(src, RustyfiVersion::V0_1)),
        "list [Package] @require:\n"
    );
}

/// A `let` whose `in` has not been typed yet still declares its name.
///
/// Deliberate, and the thing that separates a useful outline from a correct
/// one: `let total = ` at the bottom of a document is *the* state a buffer is
/// in while its author is working, and the name is already unambiguous. The
/// spine stops there rather than guessing at what follows.
#[test]
fn a_final_let_without_its_in_still_declares_its_name() {
    for (src, lang) in [
        ("let x = 1\n", RustyfiVersion::V0_0),
        ("let x = 1\n", RustyfiVersion::V0_1),
    ] {
        assert_eq!(
            outline(&document_symbols(src, lang)),
            "x [Variable] let\n",
            "under {lang}"
        );
    }
}

// ---------------------------------------------------------------------------
// Partial buffers
// ---------------------------------------------------------------------------

/// The state an editor buffer is in on most keystrokes. Everything above the
/// unfinished declaration must survive — a symbol pane that empties itself
/// while you type is worse than one that lags.
#[test]
fn a_half_typed_0_0_6_library_keeps_what_is_above_the_error() {
    let src = "@require: list\n\nlet done-one x = x\n\nlet-inline ctx \\emph it = it\n\nlet half y = if y then\n";
    assert_eq!(
        outline(&document_symbols(src, RustyfiVersion::V0_0)),
        "list [Package] @require:\ndone-one [Function] let\n\\emph [Function] let-inline\n"
    );
}

/// The same for a 0.1 document, where it matters much more: the spine is the
/// *only* place a document declares anything, so parsing the body as one
/// expression would cost every symbol above the typo.
#[test]
fn a_half_typed_0_1_document_keeps_the_spine_above_the_error() {
    let src =
        "use package Stdlib\n\nlet good = 1 in\nlet rec f x = x and g y = y in\nlet half = match\n";
    assert_eq!(
        outline(&document_symbols(src, RustyfiVersion::V0_1)),
        "\
Stdlib [Package] use package
good [Variable] let
f [Function] let rec
g [Function] let rec
"
    );
}

/// A buffer with nothing in it, and one with nothing but a comment, have
/// nothing to declare — under either grammar, and without a panic.
#[test]
fn an_empty_buffer_has_no_symbols() {
    for src in ["", "   \n\n", "% just a comment\n"] {
        assert!(document_symbols_auto(src).is_empty(), "for {src:?}");
        assert!(document_symbols(src, RustyfiVersion::V0_0).is_empty());
        assert!(document_symbols(src, RustyfiVersion::V0_1).is_empty());
    }
}

/// A file that does not even lex yields no symbols rather than a panic. (The
/// diagnostic path is what reports it; there is no prefix of a token stream
/// to walk when lexing is what failed.)
#[test]
fn an_unlexable_buffer_yields_nothing_rather_than_panicking() {
    let src = "let x = 1\nlet y = \u{0}\u{1}\u{2}\n";
    let _ = document_symbols_auto(src);
}

// ---------------------------------------------------------------------------
// Ranges
// ---------------------------------------------------------------------------

/// The failure a byte-offset implementation passes every ASCII test with.
///
/// Two declarations on one line, with Japanese in the first one's value, so
/// the second's columns are only right if they are counted in UTF-16 code
/// units: `let title = ` is 12 characters, the backtick-quoted `日本語のタイトル`
/// is 8 kanji/kana plus two backticks — 22 UTF-16 units, but **42 bytes**.
#[test]
fn a_column_after_japanese_is_counted_in_utf16_units() {
    let src = "let title = `日本語のタイトル` let sub = 2\n";
    let syms = document_symbols(src, RustyfiVersion::V0_0);
    let sub = find(&syms, "sub");

    // The whole `let sub = 2` declaration.
    assert_eq!(sub.range.start.line, 0);
    assert_eq!(sub.range.start.character, 23);
    assert_eq!(sub.range.end.character, 34);
    // Just the name.
    assert_eq!(sub.selection_range.start.character, 27);
    assert_eq!(sub.selection_range.end.character, 30);

    // The byte offsets those columns are emphatically not.
    assert_eq!(src.find("let sub"), Some(39));
    assert_eq!(src.find("sub ="), Some(43));
}

/// An astral-plane character counts as **two** UTF-16 units, which is where
/// `rustyfi_syntax::Loc::col` (a `char` column) and LSP part company.
#[test]
fn an_astral_character_before_a_symbol_counts_as_two() {
    let src = "let a = `🎉🎉` let b = 2\n";
    let syms = document_symbols(src, RustyfiVersion::V0_0);
    let b = find(&syms, "b");
    // `let a = ` (8) + backtick + 2 emoji (4 units) + backtick + space = 15.
    assert_eq!(b.range.start.character, 15);
}

/// `range` must contain `selection_range`, and must not run past the end of
/// the declaration into the following line — a `@require:` header token spans
/// its own line terminator, so an untrimmed range would.
#[test]
fn a_range_stops_at_the_end_of_the_declaration() {
    let src = "@require: list\n\nlet x = 1\n";
    let syms = document_symbols(src, RustyfiVersion::V0_0);
    let req = find(&syms, "list");
    assert_eq!(req.range.start.line, 0);
    assert_eq!(req.range.end.line, 0, "must not spill onto line 1");
    assert_eq!(req.range.end.character, 14);

    let x = find(&syms, "x");
    assert_eq!(x.range.start.line, 2);
    assert_eq!(x.range.end.line, 2);
    assert_eq!(x.range.end.character, 9);
}

/// A multi-line declaration's range covers all of it — the whole point of
/// having a `range` distinct from the `selectionRange`.
#[test]
fn a_multi_line_declarations_range_covers_all_of_it() {
    let src = "let f x =\n  let y = x in\n  y\n\nlet g = 1\n";
    let syms = document_symbols(src, RustyfiVersion::V0_0);
    let f = find(&syms, "f");
    assert_eq!(f.range.start.line, 0);
    assert_eq!(f.range.start.character, 0);
    assert_eq!(f.range.end.line, 2);
    assert_eq!(f.range.end.character, 3);
    assert_eq!(f.selection_range.start.line, 0);
    assert_eq!(f.selection_range.start.character, 4);
}

// ---------------------------------------------------------------------------
// The whole corpus
// ---------------------------------------------------------------------------

/// Every SATySFi file this repository ships, through both entry points.
///
/// Three things are being ruled out, in order of how expensive they are to
/// find later:
///
/// 1. **A panic.** A language server that crashes takes the editor's whole
///    outline pane down with it, and the walk runs on whatever tree the parse
///    produced — including a truncated one.
/// 2. **A malformed range.** A `selectionRange` outside its `range`, or a
///    range running past the end of the file, makes a client discard the
///    *entire* response — so one bad symbol silently costs the file's whole
///    outline, with nothing in any log.
/// 3. **A file that declares nothing.** Every one of these compiles, and a
///    library that compiles declares something. The only exception is
///    admitted explicitly below rather than absorbed into a threshold.
#[test]
fn the_whole_corpus_yields_plausible_outlines() {
    // Genuinely zero bytes long, so genuinely nothing to declare. Named
    // rather than tolerated by a count, so that a *second* empty outline
    // fails the test instead of hiding behind it.
    const KNOWN_EMPTY: &[&str] = &["counter.satyh"];
    // Floors, so a mistyped path cannot make this pass vacuously. The
    // corpus holds 247 files and yields a little under ten thousand symbols
    // at the time of writing; both numbers only grow.
    const FILE_FLOOR: usize = 240;
    const SYMBOL_FLOOR: usize = 9_000;

    let mut files = 0usize;
    let mut symbols = 0usize;
    let mut complaints = Vec::new();

    for rel in [
        "/../../layout-tests",
        "/../../lib-rustyfi",
        "/../../crates/rustyfi/tests/fixtures",
        "/../../manual",
    ] {
        let root = format!("{}{rel}", env!("CARGO_MANIFEST_DIR"));
        visit(std::path::Path::new(&root), &mut |path, src| {
            files += 1;
            let syms = document_symbols_auto(src);
            symbols += count(&syms);

            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if syms.is_empty() && !KNOWN_EMPTY.contains(&name) {
                complaints.push(format!("{}: no symbols at all", path.display()));
            }
            let lines = src.lines().count() as u32;
            check_ranges(&syms, lines, &path.display().to_string(), &mut complaints);
        });
    }

    assert!(
        complaints.is_empty(),
        "{} problem(s) across {files} files:\n{}",
        complaints.len(),
        complaints.join("\n")
    );
    assert!(files >= FILE_FLOOR, "only {files} files were swept");
    assert!(
        symbols >= SYMBOL_FLOOR,
        "only {symbols} symbols in {files} files"
    );
    // Visible under `--nocapture`, so the two floors above can be re-derived
    // rather than guessed at when the corpus changes.
    eprintln!("swept {files} files, {symbols} symbols");
}

/// The same invariants as a standalone assertion, so a regression in them is
/// reported as a range bug rather than as part of the corpus sweep.
#[test]
fn ranges_are_utf16_and_well_formed() {
    let src = "@require: list\n\nmodule M : sig\n  val f : int -> int\nend = struct\n  let f x = x\n  type t = A | B\nend\n";
    let syms = document_symbols(src, RustyfiVersion::V0_0);
    let mut complaints = Vec::new();
    check_ranges(&syms, src.lines().count() as u32, "inline", &mut complaints);
    assert!(complaints.is_empty(), "{}", complaints.join("\n"));
    assert!(count(&syms) >= 6);
}

/// `selectionRange ⊆ range`, `start ≤ end`, every child inside its parent,
/// and nothing past the last line.
fn check_ranges(syms: &[Symbol], lines: u32, path: &str, out: &mut Vec<String>) {
    for s in syms {
        let r = s.range;
        let sel = s.selection_range;
        if !le(r.start, r.end) {
            out.push(format!("{path}: `{}` has an inverted range", s.name));
        }
        if !le(r.start, sel.start) || !le(sel.end, r.end) {
            out.push(format!(
                "{path}: `{}` selectionRange {sel:?} is outside range {r:?}",
                s.name
            ));
        }
        if r.end.line > lines {
            out.push(format!(
                "{path}: `{}` ends on line {} of a {lines}-line file",
                s.name, r.end.line
            ));
        }
        for c in &s.children {
            if !le(r.start, c.range.start) || !le(c.range.end, r.end) {
                out.push(format!(
                    "{path}: `{}` is not inside its parent `{}`",
                    c.name, s.name
                ));
            }
        }
        check_ranges(&s.children, lines, path, out);
    }
}

fn le(a: rustyfi_lsp::Position, b: rustyfi_lsp::Position) -> bool {
    (a.line, a.character) <= (b.line, b.character)
}

/// Call `f` for every SATySFi source file under `dir`, recursively.
fn visit(dir: &std::path::Path, f: &mut impl FnMut(&std::path::Path, &str)) {
    let entries = std::fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
    let mut paths: Vec<_> = entries.flatten().map(|e| e.path()).collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            visit(&path, f);
            continue;
        }
        if !matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("saty" | "satyh" | "satyg")
        ) {
            continue;
        }
        let src =
            std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        f(&path, &src);
    }
}

// ---------------------------------------------------------------------------
// The backtracking budget
// ---------------------------------------------------------------------------

/// The outline walk runs the same parser the diagnostics do, so it inherits
/// the same exponential-backtracking hazard and must inherit the same bound.
/// Without the budget this prefix takes 11.5 seconds; an editor asks for an
/// outline on every jump to a symbol.
#[test]
fn a_pathological_prefix_is_bounded_here_too() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../lib-rustyfi/dist-v01/packages/std-ja.satyh"
    );
    let src = std::fs::read_to_string(path).expect("the vendored 0.1 corpus must be present");
    let src = &src[..14_223.min(src.len())];

    let started = std::time::Instant::now();
    let syms = document_symbols(src, RustyfiVersion::V0_1);
    assert!(
        started.elapsed() < std::time::Duration::from_secs(30),
        "the budget did not bound the walk"
    );
    // And it is not bounded by giving up on everything: the seventeen
    // headers above the truncation point are still there.
    assert!(
        syms.iter()
            .filter(|s| s.kind == SymbolKind::Package)
            .count()
            >= 17,
        "the headers should survive a truncated body:\n{}",
        outline(&syms)
    );
}
