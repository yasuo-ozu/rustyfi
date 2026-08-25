//! Hover, go-to-definition and completion, as protocol-free functions over a
//! [`Model`].
//!
//! No LSP types and no I/O, for the same reason [`crate::analyze`] has none:
//! the browser playground builds this crate for `wasm32-unknown-unknown` with
//! `--no-default-features`, and an editor feature that only exists behind a
//! JSON-RPC loop cannot be reused there. [`crate::server`] turns these results
//! into wire JSON and nothing else.
//!
//! # The rule these three obey
//!
//! **Say only what the buffer proves.** Every one of them can return nothing,
//! and does:
//!
//! - hover names what is under the cursor even when it cannot say where it
//!   came from, because "inline command `\emph`" is true and useful and
//!   "defined at line 40 of some other file" would be a guess;
//! - definition returns nothing rather than a plausible target — jumping to
//!   the wrong `x` costs more than not jumping;
//! - completion offers names that are **in scope in this buffer** and nothing
//!   else. It is a short list. A long list of names that may or may not exist
//!   is what makes a completion popup something to dismiss rather than read.

use crate::area::{area_at, Area};
use crate::model::{ByteRange, Def, HeaderKind, Hit, Model, Ns, Ref};
use rustyfi_syntax::{Atom, RustyfiVersion, Token};

// ---------------------------------------------------------------------------
// Hover
// ---------------------------------------------------------------------------

/// What to show for a cursor, and what to highlight while showing it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hover {
    /// The word the answer is about.
    pub range: ByteRange,
    /// Markdown, as LSP's `MarkupContent`.
    pub markdown: String,
}

/// Describe what is under the cursor.
///
/// Three shapes come out of this, and the difference between them is the whole
/// honesty argument:
///
/// | under the cursor | what is said |
/// |---|---|
/// | a binding | what it binds and how it was written |
/// | a mention this file binds | the same, plus the line it was bound on |
/// | a mention it does not | what *kind* of name it is, and that it is not from here |
///
/// The third is not a failure. A document's `\emph`, `+p` and `document` all
/// come from a `@require:`d package, so it is the commonest case of all, and
/// answering "inline command `\emph`" beats answering nothing.
///
/// A **type** appears whenever the author wrote one — an ascription, a `sig`'s
/// `val`, a synonym's right-hand side — quoted from the buffer. Nothing here
/// infers a type: `analyze`'s own doc comment explains why a lone buffer
/// cannot, and a hover that guessed would be wrong on exactly the documents
/// that compile.
pub fn hover(model: &Model<'_>, byte: usize) -> Option<Hover> {
    let hit = model.hit_at(byte)?;
    match hit {
        Hit::Def(d) => Some(Hover {
            range: d.name_span,
            markdown: format!("{}\n\n{}", signature(model, d), origin_of_definition(d)),
        }),
        Hit::Ref(r) => {
            let markdown = match model.resolve(r) {
                Some(d) => format!(
                    "{}\n\n{}, bound by `{}` on line {}.",
                    signature(model, d),
                    capitalised(d.ns.noun()),
                    d.form,
                    model.line_of(d.name_span.start) + 1
                ),
                None => format!("{}\n\n{}", code_block(&written(r)), elsewhere(r)),
            };
            Some(Hover {
                range: r.span,
                markdown,
            })
        }
        Hit::Header(h) => Some(Hover {
            range: h.span,
            markdown: format!(
                "{}\n\n{}",
                code_block(model.text(h.span).trim()),
                match h.kind {
                    HeaderKind::Require =>
                        format!("Package `{}`, searched for under the library root.", h.name),
                    HeaderKind::Import => format!(
                        "File `{}`, resolved relative to this file's own directory.",
                        h.name
                    ),
                    HeaderKind::Use => format!("Module `{}`, from another file.", h.name),
                }
            ),
        }),
    }
}

/// The fenced first line of a hover: the name, and its written type when there
/// is one.
fn signature(model: &Model<'_>, d: &Def) -> String {
    match d.ty {
        Some(ty) => code_block(&format!("{} : {}", d.name, model.text(ty))),
        None => code_block(&d.name),
    }
}

fn origin_of_definition(d: &Def) -> String {
    match d.declaration {
        true => format!("{} declared by `{}`.", capitalised(d.ns.noun()), d.form),
        false => format!("{} bound by `{}`.", capitalised(d.ns.noun()), d.form),
    }
}

/// What to say about a mention this file does not bind.
fn elsewhere(r: &Ref) -> String {
    if r.ns == Ns::Field {
        // A label is not a binding anywhere, so "not defined here" would
        // suggest that it is defined somewhere, which it is not.
        return "Record label.".to_string();
    }
    if !r.quals.is_empty() {
        return format!(
            "{}, from module `{}` — which is not defined in this file.",
            capitalised(r.ns.noun()),
            r.quals.join(".")
        );
    }
    format!(
        "{}. Not bound in this file: it comes from a required package or from \
         the compiler's own prelude.",
        capitalised(r.ns.noun())
    )
}

fn written(r: &Ref) -> String {
    match r.quals.is_empty() {
        true => r.name.clone(),
        false => format!("{}.{}", r.quals.join("."), r.name),
    }
}

fn code_block(s: &str) -> String {
    format!("```satysfi\n{s}\n```")
}

fn capitalised(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

// ---------------------------------------------------------------------------
// Go to definition
// ---------------------------------------------------------------------------

/// Where a cursor's name is defined.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Definition {
    /// A range in the same buffer.
    Here(ByteRange),
    /// Another file, named by a header. Resolving the name to a path needs the
    /// filesystem, so the protocol-free half stops at naming it and
    /// [`crate::server`] does the lookup.
    OtherFile { kind: HeaderKind, name: String },
}

/// Resolve the cursor to a definition, or to nothing.
pub fn definition(model: &Model<'_>, byte: usize) -> Option<Definition> {
    match model.hit_at(byte)? {
        // Already at the binding. Answering with itself is what lets a client
        // show "1 definition" instead of flashing "no definition found" when
        // the user invokes it on the declaration.
        Hit::Def(d) => Some(Definition::Here(d.name_span)),
        Hit::Ref(r) => model.resolve(r).map(|d| Definition::Here(d.name_span)),
        Hit::Header(h) => Some(Definition::OtherFile {
            kind: h.kind,
            name: h.name.clone(),
        }),
    }
}

// ---------------------------------------------------------------------------
// Completion
// ---------------------------------------------------------------------------

/// One completion candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completion {
    /// The text to show and to insert.
    pub label: String,
    /// A one-line explanation: how it was bound, and its written type when it
    /// has one.
    pub detail: String,
    /// LSP's `CompletionItemKind`.
    pub kind: u8,
    /// The word being replaced, so a client that re-filters gets the same
    /// answer the server did.
    pub range: ByteRange,
}

/// Candidates for the cursor.
///
/// The list is empty far more often than it is not, and that is the design:
///
/// - **prose gets nothing.** A bare word inside `{ … }` is text, not a name,
///   and popping a list of value names on every word typed is the single most
///   annoying thing a language server does.
/// - **a sigil decides the namespace.** `\` in inline text offers inline
///   commands, `\` in math offers math commands, `+` offers block commands,
///   `#` offers the values an embed can name. Each of those is exactly one
///   namespace, so nothing plausible-but-wrong gets in.
/// - **a qualified prefix stays inside its module.** `M.` offers `M`'s
///   members, and offers nothing at all when `M` came from a `use` header —
///   its members live in a file this buffer cannot see.
/// - **a type position offers types.** See `in_type_position`.
pub fn completions(model: &Model<'_>, byte: usize) -> Vec<Completion> {
    let source = model.source();
    let byte = crate::line_index::floor_boundary(source, byte);
    // A header being typed, before anything below can look at a token stream:
    // `@re` does not lex at all — the lexer wants a whole `@require:` and
    // reports an illegal token otherwise — which is precisely the state a
    // header is in while it is being written. Read from the TEXT for the same
    // reason [`word_before`] is.
    if let Some(items) = header_keywords(source, byte, model.version()) {
        return items;
    }
    let word = word_before(source, byte);
    // `lex_partial`, not `lex_with_version`: the buffer being completed into
    // is half-typed by definition, and its two most important shapes —
    // `{\emp` and `'<+p` — do not lex at all. Every token *before* the
    // cursor is still well-formed, and those are the only ones consulted here.
    let (atoms, _) = rustyfi_syntax::lex_partial(source, model.version());
    // Everything strictly before the partial name, sigil included: the
    // half-typed word must not get a vote on which area it is in. `Eoi` is
    // dropped because the author did not write it, and letting it end the
    // backward scan in `in_type_position` would report every end-of-file
    // cursor as an expression position.
    let before: Vec<&Atom> = atoms
        .iter()
        .filter(|a| a.slot != Token::Eoi && a.span.end.byte <= word.sigil_start)
        .collect();
    let area = area_at(&before);

    // A sigil only means what it means in a text area. In program text a `+`
    // before a name is an operator (`a+b`), and reading it as a block command
    // would offer `+p` where a value belongs.
    let sigil = match (word.sigil, area) {
        (Some('\\'), Area::Inline | Area::Math) => Some('\\'),
        (Some('+'), Area::Block) => Some('+'),
        (Some('#'), Area::Inline | Area::Block | Area::Math) => Some('#'),
        // `#` in PROGRAM text is field access — `cfg#title` — not an embed.
        // The same character, a different namespace, decided by the area
        // exactly as `\` and `+` already are.
        (Some('#'), Area::Program) => Some('#'),
        _ => None,
    };
    let namespaces: &[Ns] = match (sigil, area) {
        (Some('\\'), Area::Math) => &[Ns::MathCmd],
        (Some('\\'), _) => &[Ns::InlineCmd],
        (Some('+'), _) => &[Ns::BlockCmd],
        (Some('#'), Area::Program) => &[Ns::Field],
        (Some('#'), _) => &[Ns::Value],
        (None, Area::Program) => match in_type_position(&before) {
            true => &[Ns::Type, Ns::TypeVar],
            // A record LABEL slot takes labels and nothing else. Offering the
            // values in scope here — which is what this did — puts a list of
            // every binding in the file where only a field name can go, and
            // hides the one answer that is useful.
            false if record_label_position(&before) => &[Ns::Field],
            false => &[Ns::Value, Ns::Ctor, Ns::Module],
        },
        // Prose.
        _ => &[],
    };

    // A command's own name carries its sigil, so `\emp` is matched against
    // `\emph` directly. `#` is not part of a value's name, so it stays out of
    // both the needle and the range a client replaces.
    let start = match sigil {
        Some('\\') | Some('+') => word.sigil_start,
        _ => word.name_start,
    };
    let needle = &source[start..byte];
    let range = ByteRange::new(start, byte);

    let mut out: Vec<Completion> = Vec::new();
    for ns in namespaces {
        // Labels come from mentions rather than bindings — see
        // [`field_labels`] — so they cannot go through `in_scope`, and a
        // qualified `M.` prefix means nothing for one either: a label is not a
        // module member.
        if *ns == Ns::Field {
            if word.quals.is_empty() {
                for name in field_labels(model, range) {
                    if name.starts_with(needle) {
                        out.push(Completion {
                            label: name.to_string(),
                            detail: "record label".to_string(),
                            kind: completion_kind(Ns::Field),
                            range,
                        });
                    }
                }
            }
            continue;
        }
        let candidates: Vec<&Def> = match word.quals.is_empty() {
            true => model.in_scope(*ns, byte),
            false => match model.module_at(&word.quals, byte) {
                Some(m) => model
                    .members(m)
                    .into_iter()
                    .filter(|d| d.ns == *ns)
                    .collect(),
                None => Vec::new(),
            },
        };
        for d in candidates {
            if !d.name.starts_with(needle) {
                continue;
            }
            out.push(Completion {
                label: d.name.clone(),
                detail: match d.ty {
                    Some(ty) => format!("{} : {}", d.form, model.text(ty)),
                    None => d.form.to_string(),
                },
                kind: completion_kind(d.ns),
                range,
            });
        }
    }
    out.sort_by(|a, b| a.label.cmp(&b.label));
    out.dedup_by(|a, b| a.label == b.label);
    out
}

/// LSP `CompletionItemKind` for a namespace. The numbering is the
/// specification's, mirrored here the way [`crate::Severity::code`] mirrors
/// `DiagnosticSeverity`.
fn completion_kind(ns: Ns) -> u8 {
    match ns {
        Ns::Value => 6,                                  // Variable
        Ns::InlineCmd | Ns::BlockCmd | Ns::MathCmd => 3, // Function
        Ns::Type => 7,                                   // Class
        Ns::TypeVar => 25,                               // TypeParameter
        Ns::Ctor => 20,                                  // EnumMember
        Ns::Module => 9,                                 // Module
        Ns::Signature => 8,                              // Interface
        Ns::Field => 5,                                  // Field
    }
}

/// The partial name the cursor is typing, as offsets into the buffer.
///
/// Offsets rather than owned strings because which slice is the *needle*
/// depends on the area — a `\` belongs to a command's name and a `#` does not
/// — and that is not known until the tokens before it have been folded.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Word {
    /// A leading `\`, `+` or `#`, if one is written there. Whether it *means*
    /// anything is decided by the area, not here.
    sigil: Option<char>,
    /// Where the sigil starts, or `name_start` when there is none — the point
    /// before which the area is computed.
    sigil_start: usize,
    /// Where the name part starts, sigil excluded.
    name_start: usize,
    /// A `M.` / `A.B.` qualification written before it.
    quals: Vec<String>,
}

/// Read the partial name ending at `byte` straight out of the text.
///
/// Not from the token stream: the token stream cannot see a lone `\`, which is
/// the very first thing a user types when they want a command, and a lexer
/// asked to read one reports an illegal token rather than a partial name.
fn word_before(source: &str, byte: usize) -> Word {
    let bytes = source.as_bytes();
    let mut name_start = byte;
    while name_start > 0 && is_name_byte(bytes[name_start - 1]) {
        name_start -= 1;
    }
    let sigil = match name_start > 0 {
        true => match bytes[name_start - 1] {
            b'\\' => Some('\\'),
            b'+' => Some('+'),
            b'#' => Some('#'),
            _ => None,
        },
        false => None,
    };
    let sigil_start = name_start - usize::from(sigil.is_some());

    // A `A.B.` qualification: repeated capitalised segments, each followed by
    // a dot, immediately before the word.
    let mut quals: Vec<String> = Vec::new();
    let mut cut = sigil_start;
    while cut > 1 && bytes[cut - 1] == b'.' {
        let mut seg = cut - 1;
        while seg > 0 && is_name_byte(bytes[seg - 1]) {
            seg -= 1;
        }
        let name = &source[seg..cut - 1];
        if name.is_empty() || !name.starts_with(|c: char| c.is_uppercase()) {
            break;
        }
        quals.insert(0, name.to_string());
        cut = seg;
    }

    Word {
        sigil,
        sigil_start,
        name_start,
        quals,
    }
}

fn is_name_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'-' || b == b'_'
}

/// Whether the cursor is writing a type rather than a value.
///
/// Walks back over the tokens a type expression is made of and asks what stops
/// the walk. A `:` stops it in a type — that is the only thing a bare `:` ever
/// introduces in either grammar (`?:` is its own token) — and anything else
/// stops it in an expression.
///
/// Deliberately *not* keyed on `->`: an arrow ends `fun x ->` and a match arm
/// as well as separating a domain from a codomain, so treating one as decisive
/// would offer type names in the body of every lambda. It is only ever
/// *crossed*, on the way back to the `:` that decides.
///
/// `|` is likewise excluded, though an open row type uses one: including it
/// would read `let f : τ | x = …` — upstream's `nonrecdecargpart`, which real
/// packages write — as a type at `x`, and getting an ordinary parameter wrong
/// is worse than missing a row variable.
/// LSP `CompletionItemKind::Keyword`. Not in [`completion_kind`], which maps a
/// [`Ns`] — a header keyword is not a name in any namespace, which is exactly
/// why it had no candidates before.
const KEYWORD_KIND: u8 = 14;

/// The file headers, when the cursor is typing one.
///
/// `Some` — including `Some(empty)` — means the cursor IS in a header word, so
/// nothing else can be meant there and the caller must not fall through to the
/// name namespaces. `None` means it is not, and the ordinary machinery applies.
///
/// **`@stage:` is 0.0.6 only.** SATySFi 0.1 dropped the whole-file header for a
/// per-binding `val persistent ~x`, and its lexer treats `@stage:` as a direct
/// error rather than an unknown name (`lexer.rs`'s `"stage"` arm says so), so
/// offering it under 0.1 would be offering a compile error.
///
/// The guard is narrow on purpose: only whitespace may precede the `@`, and
/// what follows it must be bare letters. An `@` inside prose — `+p { mail@ex }`
/// — fails the first test, and a written-out `@require: a @b` fails the second.
fn header_keywords(source: &str, byte: usize, version: RustyfiVersion) -> Option<Vec<Completion>> {
    let line_start = source[..byte].rfind('\n').map_or(0, |i| i + 1);
    let typed = &source[line_start..byte];
    let at = typed.find('@')?;
    if !typed[..at].chars().all(|c| c == ' ' || c == '\t') {
        return None;
    }
    if !typed[at + 1..].chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    let range = ByteRange::new(line_start + at, byte);
    let needle = &source[range.start..byte];
    let mut out = Vec::new();
    for (label, detail) in [
        ("@require:", "Package, searched for under the library root."),
        ("@import:", "File, resolved relative to this file's own directory."),
        ("@stage:", "This file's stage: `0`, `1` or `persistent`."),
    ] {
        if label == "@stage:" && version != RustyfiVersion::V0_0 {
            continue;
        }
        if label.starts_with(needle) {
            out.push(Completion {
                label: label.to_string(),
                detail: detail.to_string(),
                kind: KEYWORD_KIND,
                range,
            });
        }
    }
    Some(out)
}

/// Is the cursor where a record LABEL goes, rather than where a value does?
///
/// True immediately after `(|`, and after a `;` whose enclosing bracket is a
/// `(|` — the two places a field name may be written in `(| a = 1; b = 2 |)`.
/// False once a `=` has been passed at that level, because everything after it
/// is the field's VALUE and offering labels there would be as wrong as
/// offering values in the label slot.
///
/// Token-stream rather than tree, for [`Area`]'s reason: a record being typed
/// into has no closing `|)` yet, so it does not parse, and this has to answer
/// anyway. The backward scan is bracket-aware so that a nested construct
/// (`(| a = f (x; y) |)`, a list, an inline group) is skipped whole rather
/// than voting with its own punctuation.
///
/// A depth-0 `;` sets `saw_sep`, after which a `=` no longer decides: that `=`
/// belongs to the PREVIOUS field, which the `;` already closed. Without it,
/// `(| a = 1; b` would read the first field's `=` and answer "value".
fn record_label_position(before: &[&Atom]) -> bool {
    let mut depth = 0usize;
    let mut saw_sep = false;
    for a in before.iter().rev() {
        match &a.slot {
            Token::RParen
            | Token::ERecord
            | Token::EList
            | Token::EHorzGrp
            | Token::EVertGrp
            | Token::EMathGrp
            | Token::EPath => depth += 1,
            Token::LParen
            | Token::BRecord
            | Token::BList
            | Token::BHorzGrp
            | Token::BVertGrp
            | Token::BMathGrp
            | Token::BPath => {
                if depth == 0 {
                    // The bracket the cursor is directly inside. Only a record
                    // opens a label position.
                    return matches!(a.slot, Token::BRecord);
                }
                depth -= 1;
            }
            Token::DefEq if depth == 0 && !saw_sep => return false,
            Token::ListPunct if depth == 0 => saw_sep = true,
            _ => {}
        }
    }
    false
}

/// [`record_label_position`] asked about a cursor in a buffer, for a caller
/// that has no token stream of its own.
///
/// Exported because the browser playground decides namespaces TWICE — once
/// here for the buffer's own names, and once in its own `Word::namespaces` for
/// the compiled-in package corpus, which this crate knows nothing about. Those
/// two decisions have to agree, and a second copy of the backward scan is
/// exactly the fork that would drift.
pub fn record_label_slot(source: &str, version: RustyfiVersion, byte: usize) -> bool {
    let byte = crate::line_index::floor_boundary(source, byte);
    let word = word_before(source, byte);
    let (atoms, _) = rustyfi_syntax::lex_partial(source, version);
    let before: Vec<&Atom> = atoms
        .iter()
        .filter(|a| a.slot != Token::Eoi && a.span.end.byte <= word.sigil_start)
        .collect();
    area_at(&before) == Area::Program && record_label_position(&before)
}

/// Every record label the buffer mentions, in first-seen order.
///
/// Labels are the one namespace with no [`Def`] to enumerate: a label binds
/// nothing (see `Model::scope_index`), so it exists only as a REFERENCE — in a
/// record literal, in a record type, in an optional argument. Harvesting the
/// mentions is therefore the only candidate source there is, and it is a
/// genuinely useful one: the labels a document needs are overwhelmingly ones
/// its own dependencies already write, and a resolved `@require:` graph puts
/// those type declarations in this model.
///
/// Deliberately NOT scope-filtered. A label has no scope to filter by, and
/// pretending otherwise would drop the useful case — `document (| ti`, where
/// the only mention of `title` is in the doc class's own type text, far
/// outside any scope containing the cursor.
/// `typing` is the range the client is about to replace. The mention under the
/// cursor is skipped, or a half-typed label suggests ITSELF: `cfg#ti` parses,
/// so `ti` is already a recorded reference by the time this is asked. (In an
/// unclosed `(| ti` it is not, because nothing there parses — which is exactly
/// the inconsistency that makes filtering by range the right test rather than
/// trusting the parse.)
fn field_labels<'m>(model: &'m Model<'_>, typing: ByteRange) -> Vec<&'m str> {
    let mut out: Vec<&str> = Vec::new();
    for r in model.refs.iter().filter(|r| r.ns == Ns::Field) {
        if r.span.start < typing.end && typing.start < r.span.end {
            continue;
        }
        if !out.contains(&r.name.as_str()) {
            out.push(&r.name);
        }
    }
    out
}

fn in_type_position(before: &[&Atom]) -> bool {
    for a in before.iter().rev() {
        match &a.slot {
            Token::Colon => return true,
            Token::Var(_)
            | Token::VarWithMod(..)
            | Token::TypeVar(_)
            | Token::RowVar(_)
            | Token::Arrow
            | Token::OptionalArrow
            | Token::OptionalType
            | Token::ExactTimes
            | Token::Comma
            | Token::ListPunct
            | Token::LParen
            | Token::RParen
            | Token::BList
            | Token::EList
            | Token::BRecord
            | Token::ERecord
            | Token::HorzCmdType
            | Token::VertCmdType
            | Token::MathCmdType
            | Token::Inline
            | Token::Block
            | Token::Math => {}
            _ => return false,
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The word at the end of `src`, and the text from its sigil onward.
    fn word(src: &str) -> (Word, &str) {
        let w = word_before(src, src.len());
        let text = &src[w.sigil_start..];
        (w, text)
    }

    #[test]
    fn a_bare_word_is_read_without_a_sigil() {
        let (w, text) = word("let z = foo");
        assert_eq!(text, "foo");
        assert_eq!(w.sigil, None);
        assert!(w.quals.is_empty());
    }

    #[test]
    fn a_command_prefix_keeps_its_sigil_so_it_matches_the_binding() {
        // A command's `Def` name carries the sigil too, so the prefix test is
        // a plain `starts_with` with no normalisation to get wrong.
        let (w, text) = word("{\\emp");
        assert_eq!(text, "\\emp");
        assert_eq!(w.sigil, Some('\\'));
        let (w, text) = word("'<+pa");
        assert_eq!(text, "+pa");
        assert_eq!(w.sigil, Some('+'));
    }

    #[test]
    fn a_lone_sigil_is_a_prefix_matching_everything_in_its_namespace() {
        let (w, text) = word("{\\");
        assert_eq!((text, w.sigil), ("\\", Some('\\')));
    }

    #[test]
    fn a_qualified_prefix_is_split_from_its_module_path() {
        let (w, _) = word("let z = List.ma");
        assert_eq!(w.quals, vec!["List".to_string()]);
        let (w, _) = word("let z = A.B.ma");
        assert_eq!(w.quals, vec!["A".to_string(), "B".to_string()]);
    }

    #[test]
    fn a_lowercase_segment_before_a_dot_is_not_a_module_path() {
        // `x.y` is not a qualification in this grammar; reading it as one
        // would offer members of a module called `x`.
        let (w, _) = word("let z = x.y");
        assert!(w.quals.is_empty(), "{w:?}");
    }
}
