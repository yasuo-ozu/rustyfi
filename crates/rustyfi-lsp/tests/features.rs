//! Hover, go-to-definition and completion, asked at known cursors.
//!
//! Each feature is exercised in **both generations**, with a **UTF-16 column
//! that has Japanese before it**, and on a **half-typed buffer** — the three
//! ways a language server that passes its ASCII unit tests still fails in an
//! editor.

use rustyfi_lsp::{
    build_model, completions, definition, hover, record_label_slot, Definition, LineIndex,
    Position, RustyfiVersion,
};

/// Ask a question the way a client does: by line and UTF-16 character.
fn cursor(src: &str, line: u32, character: u32) -> usize {
    LineIndex::new(src).offset(Position { line, character })
}

/// The byte offset just past the `n`th occurrence of `needle` — where the
/// caret sits when the user has finished typing it.
fn after(src: &str, needle: &str, n: usize) -> usize {
    let (i, _) = src
        .match_indices(needle)
        .nth(n)
        .unwrap_or_else(|| panic!("{needle:?} does not occur {} times", n + 1));
    i + needle.len()
}

fn labels(src: &str, byte: usize, lang: Option<RustyfiVersion>) -> Vec<String> {
    let m = build_model(src, lang);
    completions(&m, byte).into_iter().map(|c| c.label).collect()
}

// ---------------------------------------------------------------------------
// Hover
// ---------------------------------------------------------------------------

#[test]
fn hover_on_a_v006_mention_names_its_kind_and_where_it_was_bound() {
    let src = "let-inline \\emph it = it\nlet doc = {\\emph{hi}}\n";
    let m = build_model(src, None);
    let h = hover(&m, after(src, "\\emph", 1) - 1).expect("a hover");
    assert!(h.markdown.contains("\\emph"), "{}", h.markdown);
    assert!(
        h.markdown
            .contains("Inline command, bound by `let-inline` on line 1."),
        "{}",
        h.markdown
    );
}

#[test]
fn hover_shows_the_type_the_author_wrote_and_never_one_it_inferred() {
    let src = "let width : length -> length = fun x -> x\nlet z = width\n";
    let m = build_model(src, None);
    let h = hover(&m, after(src, "width", 1) - 1).unwrap();
    assert!(
        h.markdown
            .starts_with("```satysfi\nwidth : length -> length\n```"),
        "{}",
        h.markdown
    );

    // With no ascription there is no type line — not a guessed one.
    let src = "let width = 3pt\nlet z = width\n";
    let m = build_model(src, None);
    let h = hover(&m, after(src, "width", 1) - 1).unwrap();
    assert!(
        h.markdown.starts_with("```satysfi\nwidth\n```"),
        "{}",
        h.markdown
    );
}

#[test]
fn hover_on_a_name_from_another_file_says_so_rather_than_nothing() {
    let src = "@require: stdjabook\nlet doc = document\n";
    let m = build_model(src, None);
    let h = hover(&m, after(src, "document", 0) - 1).unwrap();
    assert!(h.markdown.contains("Value."), "{}", h.markdown);
    assert!(
        h.markdown.contains("required package"),
        "an honest hover must say WHERE it is not from: {}",
        h.markdown
    );
}

#[test]
fn hover_on_a_header_describes_how_it_is_resolved() {
    let src = "@require: stdjabook\n@import: local/thing\nlet x = 1\n";
    let m = build_model(src, None);
    let req = hover(&m, cursor(src, 0, 4)).unwrap();
    assert!(req.markdown.contains("library root"), "{}", req.markdown);
    let imp = hover(&m, cursor(src, 1, 4)).unwrap();
    assert!(
        imp.markdown.contains("relative to this file"),
        "{}",
        imp.markdown
    );
}

#[test]
fn hover_on_a_v01_val_shows_the_type_its_signature_declares() {
    let src = "\
module Geometry :> sig
  val area : float -> float
end = struct
  val area r = r
end
";
    let m = build_model(src, None);
    assert_eq!(m.version(), RustyfiVersion::V0_1);
    let h = hover(&m, after(src, "val area r", 0) - 2).unwrap();
    assert!(
        h.markdown.contains("area : float -> float"),
        "the signature's type must reach the binding: {}",
        h.markdown
    );
    assert!(h.markdown.contains("bound by `val`"), "{}", h.markdown);
}

#[test]
fn hover_at_a_utf16_column_with_japanese_before_it_finds_the_right_word() {
    let src = "let-inline \\ruby it = it\nlet doc = {日本語と\\ruby{かな}}\n";
    let m = build_model(src, None);
    // `let doc = {` is 11 UTF-16 units, `日本語と` is 4 more, so the command
    // starts at character 15 — while its BYTE offset is 12 higher.
    let byte = cursor(src, 1, 17);
    let h = hover(&m, byte).expect("a hover in the middle of `\\ruby`");
    assert!(
        h.markdown.contains("bound by `let-inline` on line 1"),
        "{}",
        h.markdown
    );
}

#[test]
fn hover_works_on_a_half_typed_buffer() {
    // The last binding is incomplete; the ones before it are still described.
    let src = "let alpha = 1\nlet beta = alpha\nlet gamma = \n";
    let m = build_model(src, None);
    assert!(!m.is_complete());
    let h = hover(&m, after(src, "alpha", 1) - 1).unwrap();
    assert!(
        h.markdown.contains("bound by `let` on line 1"),
        "{}",
        h.markdown
    );
}

// ---------------------------------------------------------------------------
// Definition
// ---------------------------------------------------------------------------

#[test]
fn definition_jumps_to_the_binding_in_v006() {
    let src = "let-block +para ctx bt = bt\nlet doc = '<+para(1);>\n";
    let m = build_model(src, None);
    let Some(Definition::Here(r)) = definition(&m, after(src, "+para", 1) - 1) else {
        panic!("expected a local definition")
    };
    assert_eq!(&src[r.start..r.end], "+para");
    assert_eq!(LineIndex::new(src).position(r.start).line, 0);
}

#[test]
fn definition_jumps_to_the_binding_in_v01() {
    let src = "\
module M = struct
  val helper = 1
  val user = helper
end
";
    let m = build_model(src, None);
    let Some(Definition::Here(r)) = definition(&m, after(src, "helper", 1)) else {
        panic!("expected a local definition")
    };
    assert_eq!(LineIndex::new(src).position(r.start).line, 1);
}

#[test]
fn definition_declines_rather_than_guessing() {
    // `document` is bound by a package, and there is no honest answer here.
    let src = "@require: stdjabook\nlet doc = document\n";
    let m = build_model(src, None);
    assert_eq!(definition(&m, after(src, "document", 0) - 1), None);
}

#[test]
fn definition_on_a_header_names_the_file_it_would_open() {
    let src = "@require: stdjabook\nlet x = 1\n";
    let m = build_model(src, None);
    assert_eq!(
        definition(&m, cursor(src, 0, 3)),
        Some(Definition::OtherFile {
            kind: rustyfi_lsp::HeaderKind::Require,
            name: "stdjabook".into(),
        })
    );
}

#[test]
fn definition_at_a_utf16_column_with_japanese_before_it() {
    // SATySFi identifiers are ASCII, so what has to survive the conversion is
    // a name written *after* Japanese prose on the same line.
    let src = "let-inline \\ruby it = it\nlet doc = {ふりがな\\ruby{か}}\n";
    let m = build_model(src, None);
    let byte = cursor(src, 1, 16); // just inside `\ruby`
    let Some(Definition::Here(r)) = definition(&m, byte) else {
        panic!("expected a local definition at a UTF-16 column")
    };
    assert_eq!(&src[r.start..r.end], "\\ruby");
    assert_eq!(LineIndex::new(src).position(r.start).line, 0);
}

#[test]
fn definition_works_on_a_half_typed_buffer() {
    let src = "let alpha = 1\nlet beta = alpha\nlet gamma = beta +\n";
    let m = build_model(src, None);
    assert!(!m.is_complete());
    let Some(Definition::Here(r)) = definition(&m, after(src, "alpha", 1)) else {
        panic!("expected the binding on line 0")
    };
    assert_eq!(LineIndex::new(src).position(r.start).line, 0);
}

// ---------------------------------------------------------------------------
// Completion
// ---------------------------------------------------------------------------

#[test]
fn completion_after_a_backslash_offers_inline_commands_only() {
    let src = "\
let-inline \\emph it = it
let-inline \\emphasise it = it
let-block +para ctx bt = bt
let emphatic = 1
let doc = {\\emp}
";
    let got = labels(src, after(src, "{\\emp", 0), None);
    assert_eq!(got, vec!["\\emph", "\\emphasise"]);
}

#[test]
fn completion_after_a_plus_offers_block_commands_only() {
    let src = "\
let-block +para ctx bt = bt
let-inline \\para it = it
let doc = '<+pa>
";
    let got = labels(src, after(src, "'<+pa", 0), None);
    assert_eq!(got, vec!["+para"]);
}

#[test]
fn completion_inside_math_offers_math_commands_and_not_inline_ones() {
    let src = "\
let-math \\frac a b = a
let-inline \\frame it = it
let doc = {${\\fr}}
";
    let got = labels(src, after(src, "${\\fr", 0), None);
    assert_eq!(got, vec!["\\frac"], "an inline command cannot go in math");
}

#[test]
fn completion_in_prose_offers_nothing() {
    // The single most annoying thing a language server can do is pop a list of
    // value names while the user writes a sentence.
    let src = "let emphasis = 1\nlet doc = {this is emp}\n";
    assert!(labels(src, after(src, "is emp", 0), None).is_empty());
}

#[test]
fn completion_in_program_position_offers_values_constructors_and_modules() {
    let src = "\
type shape = Round | Square
module Helpers = struct
  let inner = 1
end
let rounding = 1
let z = Ro
";
    let mut got = labels(src, after(src, "let z = Ro", 0), None);
    got.sort();
    assert_eq!(got, vec!["Round"]);

    let got = labels(src, after(src, "let z = R", 0), None);
    assert_eq!(got, vec!["Round"]);

    // An empty prefix in program position offers everything in scope, which
    // here includes the module and the value.
    let all = labels(src, after(src, "let z = ", 0), None);
    assert!(all.contains(&"rounding".to_string()), "{all:?}");
    assert!(all.contains(&"Helpers".to_string()), "{all:?}");
    assert!(all.contains(&"Round".to_string()), "{all:?}");
    assert!(
        !all.contains(&"shape".to_string()),
        "a TYPE must not be offered where a value goes: {all:?}"
    );
}

#[test]
fn completion_after_a_colon_offers_types_and_not_values() {
    let src = "\
type length-pair = int
let lengthy = 1
let f : len
";
    let got = labels(src, after(src, "let f : len", 0), None);
    assert_eq!(got, vec!["length-pair"], "after `:` only type names belong");
}

/// A header being typed offers the header keywords.
///
/// `@re` does not lex — the lexer wants a whole `@require:` and reports an
/// illegal token otherwise — so this is read from the text, and the test says
/// so by asking at exactly that half-typed state.
#[test]
fn completion_offers_the_file_headers_while_one_is_being_typed() {
    assert_eq!(labels("@re\n", 3, None), vec!["@require:"]);
    assert_eq!(
        labels("@\n", 1, None),
        vec!["@require:", "@import:", "@stage:"]
    );
    // A word that is no header answers nothing, rather than falling through to
    // the value namespaces and offering a name where only a header can go.
    assert!(labels("@zz\n", 3, None).is_empty());
}

/// `@stage:` is 0.0.6 only, so 0.1 must not offer it.
///
/// Not a nicety: SATySFi 0.1 dropped the whole-file header for a per-binding
/// `val persistent ~x`, and its lexer treats `@stage:` as a DIRECT error. An
/// editor offering it there would be offering a compile failure.
#[test]
fn completion_does_not_offer_the_stage_header_in_v01() {
    let v01 = Some(RustyfiVersion::V0_1);
    assert_eq!(labels("@\n", 1, v01), vec!["@require:", "@import:"]);
    assert!(labels("@st\n", 3, v01).is_empty());
    // …and still does under 0.0.6, so the test above is a contrast and not a
    // statement that `@stage:` is never offered.
    assert_eq!(labels("@st\n", 3, None), vec!["@stage:"]);
}

/// An `@` that is not a header answers nothing.
///
/// The guard is two-sided — only whitespace before the `@`, only letters after
/// it — because an address in prose and an already-written header are both
/// ordinary things to have in a buffer.
#[test]
fn completion_does_not_read_an_at_sign_in_prose_as_a_header() {
    let src = "@require: stdja\ndocument (||) \'<\n  +p { mail@ex }\n>\n";
    let at = src.find("mail@ex").unwrap() + "mail@e".len();
    assert!(labels(src, at, None).is_empty(), "an address is not a header");
}

/// A record LABEL slot offers labels, and nothing else.
///
/// Both halves matter. Before this, the slot offered every VALUE in scope —
/// a list of bindings where only a field name can go — and offered nothing at
/// all once a prefix was typed, because no value happened to start with it.
/// So the assertion is an equality, not a `contains`: the values must be gone.
#[test]
fn completion_in_a_record_label_slot_offers_labels_and_not_values() {
    let src = "\
type config = (| title : string; author : string |)
let titular = 1
let z = (| ti
";
    assert_eq!(labels(src, after(src, "let z = (| ti", 0), None), vec!["title"]);

    // An empty label slot offers every label the buffer knows.
    let mut all = labels(src, after(src, "let z = (| ", 0), None);
    all.sort();
    assert_eq!(all, vec!["author", "title"]);
    assert!(
        !all.contains(&"titular".to_string()),
        "a value cannot go in a label slot: {all:?}"
    );
}

/// After the `=`, the same record is an ordinary expression again.
///
/// The label slot and the value slot are one line apart and take disjoint
/// namespaces, so getting the boundary wrong in either direction is silent.
#[test]
fn completion_after_a_records_equals_is_a_value_position_again() {
    let src = "\
type config = (| title : string |)
let titular = 1
let z = (| title = ti
";
    assert_eq!(
        labels(src, after(src, "title = ti", 0), None),
        vec!["titular"],
        "past the `=` a field's VALUE is being written, not its name"
    );
}

/// A second field, after a `;`, is a label slot again.
///
/// The backward scan has to see that the `;` closed the previous field —
/// otherwise it finds that field's `=` and reports a value position, which is
/// the shape every record after the first field has.
#[test]
fn completion_after_a_semicolon_is_a_label_slot_again() {
    let src = "\
type config = (| title : string; author : string |)
let authorial = 1
let z = (| title = `x`; au
";
    assert_eq!(labels(src, after(src, "`x`; au", 0), None), vec!["author"]);
}

/// A nested construct inside a field's value does not vote.
///
/// `(| a = f (x) ; ` — the inner `(` and `)` are balanced and belong to the
/// value, so the scan must skip them whole rather than stopping at the inner
/// bracket and calling it the enclosing one.
#[test]
fn completion_sees_through_a_nested_group_in_a_field_value() {
    let src = "\
type config = (| title : string; author : string |)
let f x = x
let z = (| title = f (1 + 2); au
";
    assert_eq!(labels(src, after(src, "2); au", 0), None), vec!["author"]);
}

/// Field ACCESS offers labels: `#` means something different in program text.
///
/// In inline or block text `#` embeds a value; in program text `cfg#title` is
/// a field access. Same character, decided by the area — as `\\` and `+`
/// already are.
#[test]
fn completion_after_a_hash_in_program_text_offers_labels() {
    let src = "\
let cfg = (| title = `a`; author = `b` |)
let titular = 1
let z = cfg#ti
";
    assert_eq!(
        labels(src, after(src, "cfg#ti", 0), None),
        vec!["title"],
        "`#` in program text is field access, not an embed"
    );
}

/// A half-typed label does not suggest itself.
///
/// Labels are harvested from MENTIONS, because a label binds nothing — so the
/// one being typed is already a mention by the time completion is asked, in
/// every context that still parses.
#[test]
fn completion_does_not_offer_the_label_being_typed() {
    let src = "\
let cfg = (| title = `a` |)
let z = cfg#zz
";
    assert!(
        !labels(src, after(src, "cfg#zz", 0), None).contains(&"zz".to_string()),
        "the word under the cursor is not a candidate for itself"
    );
}

/// The exported form of the same question, for a caller with no token stream.
///
/// The browser playground decides namespaces twice — once inside `completions`
/// for the buffer's own names, once in its own corpus lookup — so this is
/// exported rather than duplicated. Pinned here because a fork would show up
/// only as the corpus half disagreeing with the buffer half, in a browser.
#[test]
fn the_record_label_slot_is_answerable_from_a_bare_cursor() {
    let src = "\
type config = (| title : string |)
let z = (| ti
";
    let v = None;
    assert!(record_label_slot(src, RustyfiVersion::V0_0, after(src, "let z = (| ti", 0)));
    assert!(record_label_slot(src, RustyfiVersion::V0_0, after(src, "let z = (| ", 0)));

    let val = "let z = (| title = ti\n";
    assert!(
        !record_label_slot(val, RustyfiVersion::V0_0, after(val, "title = ti", 0)),
        "past the `=` this is a value position"
    );

    // Prose is never a label slot, whatever punctuation it contains.
    let prose = "let doc = {a (| b}\n";
    assert!(!record_label_slot(prose, RustyfiVersion::V0_0, after(prose, "(| b", 0)));

    // It agrees with what `completions` itself does.
    assert_eq!(labels(src, after(src, "let z = (| ti", 0), v), vec!["title"]);
}

#[test]
fn completion_through_a_module_prefix_stays_inside_that_module() {
    let src = "\
module Helpers = struct
  let inner = 1
  let inward = 2
end
let innocent = 3
let z = Helpers.in
";
    let got = labels(src, src.len() - 1, None);
    assert_eq!(
        got,
        vec!["inner", "inward"],
        "`innocent` is in scope but is not a member of `Helpers`"
    );
}

#[test]
fn completion_through_a_module_from_another_file_offers_nothing() {
    let src = "use package Stdlib\nStdlib.doc\n";
    assert!(labels(src, src.len() - 1, Some(RustyfiVersion::V0_1)).is_empty());
}

#[test]
fn completion_in_v01_offers_the_modules_own_members() {
    let src = "\
module M = struct
  val alpha = 1
  val alphabet = 2
  val user = alph
end
";
    let got = labels(src, after(src, "val user = alph", 0), None);
    assert_eq!(got, vec!["alpha", "alphabet"]);
}

#[test]
fn completion_works_on_a_buffer_that_does_not_even_lex() {
    // `{\emp` with nothing applied to the command is a LEX error, not a parse
    // error — and it is what every buffer looks like the moment a user types a
    // command. `lex_partial` keeps the tokens before it, which is why this
    // works at all.
    let src = "let-inline \\emph it = it\nlet alpha = 1\nlet doc = {\\emp";
    let m = build_model(src, None);
    assert!(!m.is_complete());
    assert_eq!(labels(src, src.len(), None), vec!["\\emph"]);
}

#[test]
fn completion_works_on_a_buffer_that_does_not_parse() {
    // The last binding is missing its right operand, so the parse stops there
    // and everything before it is recovered.
    let src = "let alpha = 1\nlet gamma = al * \n";
    let m = build_model(src, None);
    assert!(!m.is_complete());
    let got = labels(src, after(src, "let gamma = al", 0), None);
    assert_eq!(got, vec!["alpha"]);
}

#[test]
fn completion_at_a_utf16_column_with_japanese_before_it() {
    let src = "\
let-inline \\ruby it = it
let doc = {日本語の文章と\\ru}
";
    let byte = cursor(src, 1, 21);
    assert_eq!(
        &src[..byte][byte - 3..],
        "\\ru",
        "the cursor must land just past `\\ru`"
    );
    assert_eq!(labels(src, byte, None), vec!["\\ruby"]);
}

// ---------------------------------------------------------------------------
// The sweep
// ---------------------------------------------------------------------------

/// **Every byte of a few real files, through all three features.**
///
/// The failure this rules out is the one that matters most in an editor: a
/// panic. It takes the whole session's diagnostics pane down with it, and the
/// positions an editor sends are not the positions a hand-written test thinks
/// of — inside a multi-byte character, one past the end of the buffer, in the
/// middle of a comment, on a `%` or a `}`.
///
/// A *few* files rather than all 247, because `completions` re-lexes on every
/// call and this is quadratic by construction; the whole-corpus sweep over the
/// mapping itself lives in `tests/model.rs`. These six are chosen to cover
/// both generations, both text and program-heavy code, and CJK content: a
/// document, a library, a 0.1 package, a math fixture, and two of the
/// bundled corpus's own sources.
#[test]
fn sweeping_every_cursor_position_of_real_files_never_panics() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("the crate sits two levels below the workspace root")
        .to_path_buf();
    let files = [
        // A 0.0.6 document whose prose is Japanese.
        "crates/rustyfi/tests/fixtures/cjk.saty",
        // Math mode, which has its own lexer area and its own command namespace.
        "crates/rustyfi/tests/fixtures/math.saty",
        // A 0.1 document.
        "crates/rustyfi/tests/fixtures/v01-stdja-report.saty",
        // A 0.0.6 library: signatures, `let-rec`, pattern matching.
        "lib-rustyfi/dist/packages/itemize.satyh",
        // Its 0.1 counterpart: `module … :> sig … end = struct … end`.
        "lib-rustyfi/dist-v01/packages/itemize.satyh",
        // A real hand-written document, 6 KB of mixed text and program code.
        "manual/manual.saty",
    ];
    let mut swept = 0usize;
    for rel in files {
        let path = root.join(rel);
        let Ok(src) = std::fs::read_to_string(&path) else {
            panic!("{rel} is missing — the sweep must not silently cover nothing");
        };
        let m = build_model(&src, None);
        for byte in 0..=src.len() {
            let _ = hover(&m, byte);
            let _ = definition(&m, byte);
            // Strided: `completions` re-lexes the buffer, so at every byte
            // this is quadratic in the file size for no extra coverage — the
            // interesting variation is which token the cursor is in, not which
            // byte of it. A stride coprime with nothing in particular still
            // lands inside every token of more than a few characters.
            if byte % 13 == 0 {
                let _ = completions(&m, byte);
            }
            swept += 1;
        }
    }
    assert!(swept > 20_000, "only {swept} positions swept");
}

#[test]
fn completion_offers_the_written_type_as_its_detail() {
    let src = "let width : length -> length = fun x -> x\nlet z = wid\n";
    let m = build_model(src, None);
    let items = completions(&m, after(src, "let z = wid", 0));
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].label, "width");
    assert_eq!(items[0].detail, "let : length -> length");
    assert_eq!(items[0].kind, 6, "LSP CompletionItemKind::Variable");
    assert_eq!(&src[items[0].range.start..items[0].range.end], "wid");
}
