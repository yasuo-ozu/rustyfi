//! The cursor → syntax mapping, over hand-written buffers and over the
//! bundled corpus.
//!
//! The interesting test here is not any single assertion — it is
//! [`every_identifier_in_the_corpus_is_classified`]. A hand-written walk over
//! a 60-type grammar fails by **silence**: a missed variant produces no error,
//! just a subtree whose names are invisible, and an invisible binder is worse
//! than a missing one because a reference to it then resolves to something
//! outer, which is a *wrong* answer. So the walk is checked against the lexer:
//! every identifier token the lexer produced must be covered by a `Def` or a
//! `Ref`. That turns "did you handle `MathArgBody::RecordEscape`?" from a
//! question about reading 2,000 lines of grammar into a test over 240 real
//! files.

use rustyfi_lsp::{build_model, Hit, Model, Ns, RustyfiVersion};
use rustyfi_syntax::Token;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a model and return the byte offset of the *n*th occurrence of
/// `needle`, so a test can name a cursor by what is under it rather than by a
/// magic number.
fn nth(source: &str, needle: &str, n: usize) -> usize {
    source
        .match_indices(needle)
        .nth(n)
        .unwrap_or_else(|| panic!("{needle:?} does not occur {} times in {source:?}", n + 1))
        .0
}

fn at(source: &str, needle: &str) -> usize {
    nth(source, needle, 0)
}

/// The definition the cursor's name resolves to, as `(form, name, line)`.
fn definition(m: &Model<'_>, byte: usize) -> Option<(&'static str, String, u32)> {
    let target = match m.hit_at(byte)? {
        Hit::Ref(r) => m.resolve(r)?,
        Hit::Def(d) => d,
        Hit::Header(_) => return None,
    };
    let line = rustyfi_lsp::LineIndex::new(m.source())
        .position(target.name_span.start)
        .line;
    Some((target.form, target.name.clone(), line))
}

fn kind(m: &Model<'_>, byte: usize) -> Option<Ns> {
    match m.hit_at(byte)? {
        Hit::Ref(r) => Some(r.ns),
        Hit::Def(d) => Some(d.ns),
        Hit::Header(_) => None,
    }
}

// ---------------------------------------------------------------------------
// 0.0.6
// ---------------------------------------------------------------------------

#[test]
fn a_top_level_let_is_found_from_a_later_mention() {
    let src = "let greeting = 1\nlet other = greeting\n";
    let m = build_model(src, None);
    assert_eq!(m.version(), RustyfiVersion::V0_0);
    assert_eq!(
        definition(&m, nth(src, "greeting", 1)),
        Some(("let", "greeting".into(), 0))
    );
}

#[test]
fn a_lambda_parameter_shadows_the_top_level_binding_of_the_same_name() {
    let src = "let x = 1\nlet f = fun x -> x\n";
    let m = build_model(src, None);
    // The `x` in the body is the parameter on line 1, not the `let` on line 0.
    assert_eq!(
        definition(&m, nth(src, "x", 2)),
        Some(("parameter", "x".into(), 1))
    );
    // …and the parameter is not visible outside the lambda.
    let src2 = "let x = 1\nlet f = fun x -> x\nlet g = x\n";
    let m2 = build_model(src2, None);
    assert_eq!(
        definition(&m2, nth(src2, "x", 3)),
        Some(("let", "x".into(), 0))
    );
}

#[test]
fn a_let_in_binding_is_not_visible_in_its_own_value() {
    let src = "let outer = 7\nlet f = let outer = outer in outer\n";
    let m = build_model(src, None);
    // The `outer` on the right of `=` still means the top-level one.
    assert_eq!(
        definition(&m, nth(src, "outer", 2)),
        Some(("let", "outer".into(), 0))
    );
    // The one after `in` means the inner binding.
    assert_eq!(
        definition(&m, nth(src, "outer", 3)),
        Some(("let", "outer".into(), 1))
    );
}

#[test]
fn a_let_rec_binding_is_visible_inside_its_own_body() {
    let src = "let-rec loop n = loop n\n";
    let m = build_model(src, None);
    assert_eq!(
        definition(&m, nth(src, "loop", 1)),
        Some(("let-rec", "loop".into(), 0))
    );
}

#[test]
fn a_match_arm_binds_only_within_its_own_arm() {
    let src = "let v = 0\nlet f = fun o -> match o with\n| Some v -> v\n| None -> v\n";
    let m = build_model(src, None);
    assert_eq!(
        definition(&m, nth(src, "v", 2)),
        Some(("match binding", "v".into(), 2)),
        "the `v` in the Some arm is the pattern binding"
    );
    assert_eq!(
        definition(&m, nth(src, "v", 3)),
        Some(("let", "v".into(), 0)),
        "the `v` in the None arm is the top-level one"
    );
}

#[test]
fn the_three_command_namespaces_do_not_collide() {
    let src = "\
let-inline \\same it = it
let-block +same ctx bt = bt
let-math \\same m = m
let doc = '<+same(1);>
";
    let m = build_model(src, None);
    assert_eq!(kind(&m, at(src, "+same(")), Some(Ns::BlockCmd));
    assert_eq!(
        definition(&m, at(src, "+same(")),
        Some(("let-block", "+same".into(), 1)),
        "a `+cmd` use must not resolve to the `\\cmd` of the same name"
    );
    assert_eq!(kind(&m, at(src, "\\same")), Some(Ns::InlineCmd));
    // The `let-math` binding is a MATH command even though its token is the
    // same `\name` an inline command uses.
    assert_eq!(kind(&m, nth(src, "\\same", 1)), Some(Ns::MathCmd));
}

#[test]
fn an_inline_command_resolves_from_inside_inline_text() {
    let src = "let-inline \\emph it = it\nlet doc = {\\emph{hi}}\n";
    let m = build_model(src, None);
    assert_eq!(
        definition(&m, nth(src, "\\emph", 1)),
        Some(("let-inline", "\\emph".into(), 0))
    );
}

#[test]
fn a_math_command_resolves_from_inside_math() {
    let src = "let-math \\frac a b = a\nlet doc = {${\\frac{1}{2}}}\n";
    let m = build_model(src, None);
    assert_eq!(
        definition(&m, nth(src, "\\frac", 1)),
        Some(("let-math", "\\frac".into(), 0))
    );
}

#[test]
fn a_user_defined_operator_resolves_from_its_use() {
    let src = "let ( +++ ) a b = a\nlet z = 1 +++ 2\n";
    let m = build_model(src, None);
    assert_eq!(
        definition(&m, at(src, "1 +++") + 2),
        Some(("let", "+++".into(), 0))
    );
}

#[test]
fn a_variant_constructor_resolves_to_its_type_declaration() {
    let src = "type colour = Red | Green of int\nlet c = Green 1\n";
    let m = build_model(src, None);
    assert_eq!(
        definition(&m, nth(src, "Green", 1)),
        Some(("variant constructor", "Green".into(), 0))
    );
    assert_eq!(kind(&m, at(src, "colour")), Some(Ns::Type));
}

#[test]
fn a_module_member_resolves_through_its_qualification() {
    let src = "module M = struct\n  let inner = 1\nend\nlet z = M.inner\n";
    let m = build_model(src, None);
    assert_eq!(
        definition(&m, at(src, "M.inner")),
        Some(("let", "inner".into(), 1))
    );
    assert_eq!(kind(&m, at(src, "M.inner")), Some(Ns::Value));
}

#[test]
fn a_member_of_a_module_this_file_does_not_define_resolves_to_nothing() {
    let src = "let z = Elsewhere.thing\n";
    let m = build_model(src, None);
    assert_eq!(definition(&m, at(src, "Elsewhere")), None);
}

#[test]
fn opening_a_local_module_brings_its_members_into_scope() {
    let src = "module M = struct\n  let inner = 1\nend\nopen M\nlet z = inner\n";
    let m = build_model(src, None);
    assert_eq!(
        definition(&m, nth(src, "inner", 1)),
        Some(("let", "inner".into(), 1))
    );
}

#[test]
fn opening_an_unknown_module_makes_earlier_bindings_unresolvable() {
    // `open Unknown` may bring an `x` of its own, which would shadow the one
    // on line 0 — so the honest answer for the mention on line 2 is nothing.
    let src = "let x = 1\nopen Unknown\nlet z = x\n";
    let m = build_model(src, None);
    assert_eq!(definition(&m, nth(src, "x", 1)), None);

    // A binding written AFTER the open cannot be shadowed by it, so that one
    // still resolves.
    let src = "open Unknown\nlet x = 1\nlet z = x\n";
    let m = build_model(src, None);
    assert_eq!(
        definition(&m, nth(src, "x", 1)),
        Some(("let", "x".into(), 1))
    );
}

#[test]
fn a_signature_supplies_the_type_a_binding_does_not_write() {
    let src = "\
module M : sig
  val inner : int -> int
end = struct
  let inner x = x
end
let z = M.inner
";
    let m = build_model(src, None);
    let d = match m.hit_at(at(src, "M.inner")).unwrap() {
        Hit::Ref(r) => m.resolve(r).unwrap(),
        other => panic!("expected a reference, got {other:?}"),
    };
    assert_eq!(d.form, "let", "the jump target is the binding, not the sig");
    assert_eq!(m.text(d.ty.expect("the sig's type")), "int -> int");
}

#[test]
fn an_ascription_is_quoted_verbatim_from_the_buffer() {
    let src = "let f : int -> bool = fun x -> true\n";
    let m = build_model(src, None);
    let Some(Hit::Def(d)) = m.hit_at(at(src, "f ")) else {
        panic!("expected the binding")
    };
    assert_eq!(m.text(d.ty.unwrap()), "int -> bool");
}

// ---------------------------------------------------------------------------
// 0.1
// ---------------------------------------------------------------------------

#[test]
fn a_v01_library_binds_its_members_to_its_module() {
    let src = "\
module Greet = struct
  val hello = 1
  val there = hello
end
";
    let m = build_model(src, None);
    assert_eq!(m.version(), RustyfiVersion::V0_1);
    assert_eq!(
        definition(&m, nth(src, "hello", 1)),
        Some(("val", "hello".into(), 1))
    );
}

#[test]
fn a_v01_signature_declares_the_type_of_a_val() {
    let src = "\
module M :> sig
  val f : int -> int
end = struct
  val f x = x
end
";
    let m = build_model(src, None);
    let Some(Hit::Def(d)) = m.hit_at(at(src, "val f x") + 4) else {
        panic!("expected the binding")
    };
    assert_eq!(d.form, "val");
    assert_eq!(
        m.text(d.ty.expect("adopted from the signature")),
        "int -> int"
    );
}

#[test]
fn a_v01_command_binding_keeps_its_own_namespace() {
    let src = "\
module M = struct
  val inline \\emph it = it
  val block +para ctx bt = bt
  val math ctx \\frac a b = a
end
";
    let m = build_model(src, None);
    assert_eq!(m.version(), RustyfiVersion::V0_1);
    assert_eq!(kind(&m, at(src, "\\emph")), Some(Ns::InlineCmd));
    assert_eq!(kind(&m, at(src, "+para")), Some(Ns::BlockCmd));
    assert_eq!(kind(&m, at(src, "\\frac")), Some(Ns::MathCmd));
}

#[test]
fn a_v01_use_header_binds_a_module_name_it_cannot_look_into() {
    let src = "use package Stdlib\nStdlib.document\n";
    let m = build_model(src, Some(RustyfiVersion::V0_1));
    assert_eq!(
        definition(&m, at(src, "Stdlib.document")),
        None,
        "the member lives in another file"
    );
    // But the module NAME itself is known, and hovering it is useful.
    let Some(Hit::Ref(r)) = m.hit_at(at(src, "Stdlib.document")) else {
        panic!("expected a reference")
    };
    assert_eq!(r.quals, vec!["Stdlib".to_string()]);
    assert_eq!(
        m.resolve_in_scope(Ns::Module, "Stdlib", at(src, "Stdlib.document"))
            .map(|d| d.form),
        Some("use package")
    );
}

#[test]
fn a_v01_match_arm_ends_at_its_own_end_keyword() {
    let src = "\
module M = struct
  val v = 0
  val f o = match o with
    | Some v -> v
    | None -> v
    end
end
";
    let m = build_model(src, None);
    assert_eq!(
        definition(&m, nth(src, "v", 4)),
        Some(("match binding", "v".into(), 3))
    );
    assert_eq!(
        definition(&m, nth(src, "v", 5)),
        Some(("val", "v".into(), 1))
    );
}

// ---------------------------------------------------------------------------
// UTF-16 and Japanese
// ---------------------------------------------------------------------------

#[test]
fn a_cursor_after_japanese_text_still_lands_on_the_right_name() {
    let src = "let-inline \\ruby it = it\nlet doc = {日本語の文章と\\ruby{ふりがな}}\n";
    let m = build_model(src, None);
    let idx = rustyfi_lsp::LineIndex::new(src);
    // Column of the `\ruby` USE, counted the way an editor counts: the line is
    // `let doc = {日本語の文章と\ruby{ふりがな}}`, so `\` is 17 UTF-16 units in
    // and 23 bytes in — the numbers a byte-based implementation confuses.
    let byte = nth(src, "\\ruby", 1);
    let pos = idx.position(byte);
    // `let doc = {` is 11 UTF-16 units and the seven kana/kanji are 7 more,
    // where the same seven characters are 21 BYTES — the number a byte-based
    // conversion would have handed the editor instead.
    assert_eq!((pos.line, pos.character), (1, 18));
    assert_eq!(
        definition(&m, idx.offset(pos)),
        Some(("let-inline", "\\ruby".into(), 0))
    );
}

// ---------------------------------------------------------------------------
// Half-typed buffers
// ---------------------------------------------------------------------------

#[test]
fn a_buffer_that_does_not_parse_still_yields_the_bindings_before_the_break() {
    let src = "let alpha = 1\nlet beta = 2\nlet gamma = \n";
    let m = build_model(src, None);
    assert!(!m.is_complete());
    let names: Vec<&str> = m
        .in_scope(Ns::Value, src.len())
        .iter()
        .map(|d| d.name.as_str())
        .collect();
    assert!(
        names.contains(&"alpha") && names.contains(&"beta"),
        "{names:?}"
    );
}

#[test]
fn a_half_typed_v01_library_still_yields_its_earlier_members() {
    let src = "\
module M = struct
  val alpha = 1
  val beta = 2
  val gamma =
";
    let m = build_model(src, Some(RustyfiVersion::V0_1));
    assert!(!m.is_complete());
    let names: Vec<&str> = m
        .in_scope(Ns::Value, src.len())
        .iter()
        .map(|d| d.name.as_str())
        .collect();
    assert!(
        names.contains(&"alpha") && names.contains(&"beta"),
        "{names:?}"
    );
}

#[test]
fn a_buffer_that_does_not_even_lex_keeps_what_came_before_the_failure() {
    // An unterminated string literal. `lex_partial` hands back the tokens up
    // to it, so the bindings written earlier are still there — and the one
    // being typed is not, because nothing about it is knowable yet.
    let src = "let alpha = 1\nlet beta = `unterminated\n";
    let m = build_model(src, None);
    assert!(!m.is_complete());
    let names: Vec<&str> = m
        .in_scope(Ns::Value, src.len())
        .iter()
        .map(|d| d.name.as_str())
        .collect();
    assert_eq!(names, vec!["alpha"]);
}

// ---------------------------------------------------------------------------
// The corpus
// ---------------------------------------------------------------------------

/// Every `.saty`/`.satyh`/`.satyg` file this repository ships.
fn corpus() -> Vec<PathBuf> {
    let root = repo_root();
    let mut out = Vec::new();
    for dir in [
        "layout-tests/corpus",
        "lib-rustyfi/dist/packages",
        "lib-rustyfi/dist-v01/packages",
        "crates/rustyfi/tests/fixtures",
        "manual",
    ] {
        collect(&root.join(dir), &mut out);
    }
    out.sort();
    assert!(
        out.len() > 200,
        "expected the bundled corpus, found {} files — is the checkout complete?",
        out.len()
    );
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
    // `CARGO_MANIFEST_DIR` is `<root>/crates/rustyfi-lsp`.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the crate sits two levels below the workspace root")
        .to_path_buf()
}

/// Which tokens name something. Everything else — punctuation, keywords,
/// literals, and the `Char`/`MathChar` runs that make up prose — is not a
/// name and is not expected to be classified.
fn is_identifier(t: &Token) -> bool {
    matches!(
        t,
        Token::Var(_)
            | Token::VarWithMod(..)
            | Token::Constructor(_)
            | Token::TypeVar(_)
            | Token::LongUpper(..)
            | Token::OpenModule(_)
            | Token::HorzCmd(_)
            | Token::HorzCmdWithMod(..)
            | Token::VertCmd(_)
            | Token::VertCmdWithMod(..)
            | Token::MathCmd(_)
            | Token::MathCmdWithMod(..)
            | Token::VarInHorz(..)
            | Token::VarInVert(..)
            | Token::VarInMath(..)
    )
}

/// **The completeness test.** For every corpus file that parses whole, every
/// identifier token the lexer produced must be covered by something the walk
/// recorded.
///
/// A gap here is a subtree the walk does not descend into, and the failure
/// message names the file, the line and the token so the missing variant is
/// one grep away.
#[test]
fn every_identifier_in_the_corpus_is_classified() {
    let mut checked = 0usize;
    let mut misses: Vec<String> = Vec::new();
    for path in corpus() {
        let Ok(src) = std::fs::read_to_string(&path) else {
            continue;
        };
        let m = build_model(&src, None);
        if !m.is_complete() {
            // A file this crate cannot parse whole is the parser's business,
            // not the walk's; `analysis.rs` already reports it.
            continue;
        }
        checked += 1;
        let Ok(atoms) = rustyfi_syntax::lex_with_version(&src, m.version()) else {
            continue;
        };
        let index = rustyfi_lsp::LineIndex::new(&src);
        for a in atoms.iter().filter(|a| is_identifier(&a.slot)) {
            if m.hit_at(a.span.start.byte).is_none() {
                let pos = index.position(a.span.start.byte);
                misses.push(format!(
                    "{}:{}:{} {:?}",
                    path.display(),
                    pos.line + 1,
                    pos.character + 1,
                    a.slot
                ));
            }
        }
    }
    assert!(checked > 100, "only {checked} corpus files parsed whole");
    assert!(
        misses.is_empty(),
        "{} identifier tokens the walk never visited (first 20):\n{}",
        misses.len(),
        misses
            .iter()
            .take(20)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// **The no-panic sweep.** Every byte offset of a sample of real files, plus
/// every offset of a few files in full, asked every question the server can
/// ask.
///
/// A language server that panics takes the editor's whole session down, and
/// the positions an editor sends are not the positions a test thinks of:
/// inside a multi-byte character, one past the end, in the middle of a
/// comment.
#[test]
fn sweeping_every_cursor_position_over_the_corpus_never_panics() {
    for path in corpus() {
        let Ok(src) = std::fs::read_to_string(&path) else {
            continue;
        };
        let m = build_model(&src, None);
        let index = rustyfi_lsp::LineIndex::new(&src);
        // Every byte, not every character: an offset landing inside a
        // multi-byte character is exactly what a mis-converted UTF-16 column
        // produces, and it must not panic either.
        //
        // `hit_at` is a linear scan of the file's own names, so sweeping every
        // byte is quadratic in the file size — fine for the 240 files under
        // 8 KB, and minutes for the handful of 50 KB documents. Those are
        // strided; `tests/features.rs` sweeps six real files at every byte
        // through all three features, which is where the exhaustive coverage
        // lives.
        let stride = match src.len() > 8 * 1024 {
            true => 5,
            false => 1,
        };
        for byte in (0..=src.len()).step_by(stride) {
            if let Some(Hit::Ref(r)) = m.hit_at(byte) {
                let _ = m.resolve(r);
            }
            let _ = index.position(byte);
            // `in_scope` allocates per call and is quadratic in the file's
            // binding count, so it is sampled rather than swept: it reads the
            // same table `resolve` does, and a position it could reject is one
            // the every-byte loop above has already visited.
            if byte % 32 == 0 {
                let _ = m.in_scope(Ns::Value, byte);
            }
        }
    }
}
