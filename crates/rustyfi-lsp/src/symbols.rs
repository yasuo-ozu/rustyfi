//! `textDocument/documentSymbol`: the outline of one buffer.
//!
//! Protocol-free, like [`crate::analyze`] beside it — source text in, a tree
//! of [`Symbol`]s out, no LSP types, no filesystem, nothing outside
//! `rustyfi-syntax`. The server turns the tree into JSON; the wasm playground
//! could render it however it likes.
//!
//! # What it is, and what it cannot be
//!
//! A **pure structural walk of the CST**. No name resolution, no types, no
//! `@require:` following — so unlike a diagnostic it cannot be *wrong*, only
//! incomplete. Every symbol here corresponds to a declaration the parser
//! actually read out of this one file.
//!
//! # Partial buffers
//!
//! An editor buffer is incomplete on most keystrokes, and a symbol pane that
//! empties itself while you type is worse than one that lags. So the walk
//! never parses the file as one all-or-nothing [`rustyfi_syntax::cst::File`]:
//! it parses the top-level declaration *sequence* (`Vec<TopBinding>` for
//! 0.0.6, `Vec<Bind>` for 0.1), which is a syan repetition — it stops at the
//! first declaration that does not parse, rolls that one back, and hands over
//! everything before it. The document body's `let … in …` spine is walked the
//! same way, one clause at a time, for the same reason: a 0.1 *document*
//! declares everything in that spine and nowhere else, so parsing it whole
//! would mean one typo costs the entire outline.
//!
//! The same backtracking budget that bounds the diagnostics parse (the
//! crate-private `budget` module) bounds this one, and for the same
//! reason.
//!
//! # Ranges
//!
//! Every [`Symbol`] carries two, exactly as LSP asks:
//!
//! - `range` — the whole declaration, taken from the tokens the node would
//!   *unparse* to (see [`node_span`]), so it is exact rather than guessed
//!   from where the next sibling starts;
//! - `selection_range` — just the name, for "go to symbol" to land on.
//!
//! Both are zero-based lines and **UTF-16** characters (see
//! [`crate::LineIndex`]), and `range` is widened to contain `selection_range`
//! by construction: a client that finds them inconsistent drops the whole
//! response.

use rustyfi_syntax::token::Atom;
use rustyfi_syntax::{RustyfiVersion, Span, Token};
use syan::parse::unparse::{Emitter, Unparse};
use syan::parse::Parse;

use rustyfi_syntax::stream::AtomStream;
use crate::line_index::{floor_boundary, LineIndex};
use crate::Position;

mod v0_0;
mod v0_1;

/// A half-open LSP range, zero-based lines and UTF-16 characters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Range {
    /// Where the range starts.
    pub start: Position,
    /// Where it ends (exclusive).
    pub end: Position,
}

/// What a [`Symbol`] is, as the subset of LSP's `SymbolKind` this port has
/// anything to say about.
///
/// The numbering is LSP's own (see [`SymbolKind::code`]), but the type names
/// nothing from the protocol, so the analysis half stays protocol-free.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolKind {
    /// `@import:` / `use … of \`…\`` — a dependency named by file path.
    File,
    /// `module M = struct … end`, and `use M`.
    Module,
    /// `@require:` / `use package M` — a dependency named by package.
    Package,
    /// A `type` declaration whose body is a variant list.
    Enum,
    /// One constructor of such a variant.
    EnumMember,
    /// `signature S = sig … end`, and a `sig … end` block annotating a
    /// module.
    Interface,
    /// A binding that takes parameters: `let f x = …`, every command, every
    /// `let-rec`, and a signature item whose declared type is an arrow.
    Function,
    /// A binding that takes none: `let x = …`, `let-mutable`, and a
    /// signature item whose declared type is not an arrow.
    Variable,
    /// A `type` declaration that is a synonym or is left opaque.
    TypeParameter,
}

impl SymbolKind {
    /// The LSP `SymbolKind` wire number.
    pub fn code(self) -> u8 {
        match self {
            SymbolKind::File => 1,
            SymbolKind::Module => 2,
            SymbolKind::Package => 4,
            SymbolKind::Enum => 10,
            SymbolKind::Interface => 11,
            SymbolKind::Function => 12,
            SymbolKind::Variable => 13,
            SymbolKind::EnumMember => 22,
            SymbolKind::TypeParameter => 26,
        }
    }
}

/// One node of the outline: a declaration, and whatever it declares inside
/// itself.
///
/// The tree shape is the point. A library is one `module`, and flattening its
/// thirty members up beside it turns a navigable outline into a wall of
/// names — so a module's members are its `children`, and a nested module's
/// are its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    /// The declared name, written the way the source writes it — `\emph`
    /// keeps its backslash, `+p` its plus, `( + )` its parentheses.
    pub name: String,
    /// A short word for what kind of declaration this is (`let-inline`,
    /// `val rec`, `type`), plus its stage qualifier where it has one. Shown
    /// greyed beside the name.
    pub detail: Option<String>,
    /// Which icon the editor draws.
    pub kind: SymbolKind,
    /// The whole declaration.
    pub range: Range,
    /// Just the name.
    pub selection_range: Range,
    /// Declarations nested inside this one.
    pub children: Vec<Symbol>,
}

/// Extract the outline of `source` under an explicitly chosen generation.
///
/// `lang` is taken literally — there is no fallback to the other generation.
/// Use [`document_symbols_auto`] to have it chosen from the text.
pub fn document_symbols(source: &str, lang: RustyfiVersion) -> Vec<Symbol> {
    // A lex failure leaves nothing to walk. Unlike a parse failure this one
    // is not partial-recoverable: the token stream is what the walk consumes,
    // and there is no prefix of it to keep. Diagnostics already report it.
    let Ok(atoms) = rustyfi_syntax::lex_with_version(source, lang) else {
        return Vec::new();
    };
    if atoms.iter().all(|a| a.slot == Token::Eoi) {
        return Vec::new();
    }
    let ranges = Ranges(LineIndex::new(source));
    let mut stream = crate::budget::stream(atoms);
    match lang {
        RustyfiVersion::V0_1 => v0_1::walk(&mut stream, &ranges),
        // `RustyfiVersion` is `#[non_exhaustive]`; anything not explicitly
        // 0.1 is read with the 0.0.6 grammar, matching `analysis`.
        _ => v0_0::walk(&mut stream, &ranges),
    }
}

/// [`document_symbols`], choosing the generation from the buffer itself.
///
/// Uses [`crate::analyze_detected`]'s rule rather than a second one of its
/// own — including the re-check that 32 of the 34 bundled 0.1 packages
/// depend on, since a `module M = struct` head is deliberately no version
/// signal at all. See [`crate::detect_version`].
pub fn document_symbols_auto(source: &str) -> Vec<Symbol> {
    document_symbols_detected(source).1
}

/// [`document_symbols_auto`], also reporting the generation it read the
/// buffer as.
pub fn document_symbols_detected(source: &str) -> (RustyfiVersion, Vec<Symbol>) {
    let lang = crate::detect_version(source);
    (lang, document_symbols(source, lang))
}

// ---------------------------------------------------------------------------
// Shared machinery for the two generation walks
// ---------------------------------------------------------------------------

/// Byte spans → LSP ranges, over one buffer.
pub(crate) struct Ranges<'s>(LineIndex<'s>);

impl Ranges<'_> {
    /// A span as an LSP range, with trailing whitespace trimmed off the end.
    ///
    /// The trim is not cosmetic. A `@require: foo` header token spans its own
    /// line terminator, so the untrimmed range ends at `{line + 1, 0}` — an
    /// editor then highlights the line break and the breadcrumb claims the
    /// header contains the first thing on the next line. The same applies to
    /// any declaration whose last token carries trailing layout.
    fn range(&self, span: Span) -> Range {
        let src = self.0.source();
        let start = floor_boundary(src, span.start.byte);
        let mut end = floor_boundary(src, span.end.byte.max(start));
        while end > start {
            let Some(c) = src[..end].chars().next_back().filter(|c| c.is_whitespace()) else {
                break;
            };
            end -= c.len_utf8();
        }
        Range {
            start: self.0.position(start),
            end: self.0.position(end),
        }
    }
}

/// A [`Symbol`] under construction, in source-byte coordinates.
///
/// Kept separate from [`Symbol`] so the walks never have to thread a
/// [`Ranges`] through every helper just to convert two spans, and so the
/// "`range` must contain `selection_range`" rule is enforced in exactly one
/// place ([`Sym::build`]) instead of at forty call sites.
pub(crate) struct Sym {
    name: String,
    detail: Option<String>,
    kind: SymbolKind,
    whole: Span,
    sel: Span,
    children: Vec<Symbol>,
}

impl Sym {
    fn new(name: impl Into<String>, kind: SymbolKind, whole: Span, sel: Span) -> Self {
        Sym {
            name: name.into(),
            detail: None,
            kind,
            whole,
            sel,
            children: Vec::new(),
        }
    }

    fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    fn children(mut self, children: Vec<Symbol>) -> Self {
        self.children = children;
        self
    }

    fn build(self, r: &Ranges<'_>) -> Symbol {
        Symbol {
            name: self.name,
            detail: self.detail,
            kind: self.kind,
            // United, not merely asserted: a client that receives a
            // `selectionRange` outside its `range` rejects the whole
            // response, and losing every symbol in the file to one bad pair
            // is a far worse outcome than a range one token too generous.
            range: r.range(self.whole.unite(self.sel)),
            selection_range: r.range(self.sel),
            children: self.children,
        }
    }
}

/// The exact source extent of any CST node.
///
/// Neither the CST nodes nor the syan derive carry a `Spanned` impl, and
/// hand-writing one per node would mean transcribing thirty-odd `Expr`
/// variants. But every node *does* implement `Unparse`, and the leaves replay
/// the spans they were parsed with — so writing the node into a sink that
/// keeps nothing but the union of the spans it is handed recovers the extent
/// exactly, at the cost of one walk over the node's own tokens.
///
/// The alternative — ending a declaration where the next one begins — was
/// tried first and is worse: it swallows the comment between two
/// declarations, and it needs a different terminator for every kind of block.
///
/// Shared with the cursor-driven half ([`crate::walk006`], [`crate::walk01`]),
/// which reaches for it only where a boundary keyword cannot supply the
/// answer — the extent of a type ascription, which ends wherever it happens to
/// end. Every *scope* boundary there comes from a keyword instead, which is
/// cheaper and needs no walk at all.
pub(crate) fn node_span<T: Unparse<Atom> + ?Sized>(node: &T) -> Span {
    let mut sink = SpanSink {
        span: Span::default(),
    };
    // `SpanSink` cannot fail, so this discards an `Infallible`.
    let _ = node.unparse(&mut sink);
    sink.span
}

/// The sink [`node_span`] writes into: it drops every atom and keeps only how
/// far they reach.
struct SpanSink {
    span: Span,
}

impl Emitter<Atom> for SpanSink {
    type Error = std::convert::Infallible;

    fn write_one(&mut self, atom: Atom) -> Result<(), Self::Error> {
        self.span = self.span.unite(atom.span);
        Ok(())
    }

    fn write_sep(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

/// Parse one `T`, or leave the stream untouched and answer `None`.
///
/// `Option<T>`'s own syan impl is what does the rollback; this only spells
/// away its `Result<_, Infallible>`.
pub(crate) fn opt<T: Parse<Atom>>(stream: &mut AtomStream) -> Option<T> {
    <Option<T> as Parse<Atom>>::parse_stream(stream)
        .ok()
        .flatten()
}

/// Parse as many `T`s as will parse, stopping (and rolling back) at the first
/// that will not.
///
/// This is the whole partial-buffer story: syan's repetition impl never
/// fails, so a half-typed declaration ends the sequence instead of destroying
/// it.
pub(crate) fn many<T: Parse<Atom>>(stream: &mut AtomStream) -> Vec<T> {
    <Vec<T> as Parse<Atom>>::parse_stream(stream).unwrap_or_default()
}

/// `\cmd` / `\Mod.cmd` and `+cmd` / `+Mod.cmd`, written the way the source
/// writes them.
///
/// The lexer keeps the sigil on the *name* (`Token::HorzCmd("\\emph")`) and
/// the module path beside it, so a qualified command has to be reassembled
/// rather than joined: a naive `mods.join(".") + "." + name` reads
/// `Mod.\emph`.
pub(crate) fn qualified_command(mods: &[String], name: &str) -> String {
    if mods.is_empty() {
        return name.to_string();
    }
    let (sigil, bare) = name.split_at(name.len() - name.trim_start_matches(['\\', '+']).len());
    format!("{sigil}{}.{bare}", mods.join("."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_qualified_command_keeps_its_sigil_in_front() {
        assert_eq!(qualified_command(&[], "\\emph"), "\\emph");
        assert_eq!(
            qualified_command(&["Mod".to_string()], "\\emph"),
            "\\Mod.emph"
        );
        assert_eq!(
            qualified_command(&["A".to_string(), "B".to_string()], "+p"),
            "+A.B.p"
        );
    }
}
