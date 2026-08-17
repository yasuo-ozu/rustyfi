//! The milestone-1 surface grammar (a subset of the v0.0.6 `parser.mly`),
//! parsed with syan derives over the SATySFi token atoms.
//!
//! Application and the binary-operator levels are left-recursive in the Menhir
//! grammar; here they are head-plus-arguments sequences (`Vec`), folded left
//! during elaboration — recursive descent must never see left recursion.

use crate::leaf::*;
use crate::span::Span;
use crate::stream::TokenStream;
use syan::parse::{Parse, Unparse};

/// A whole `.saty` file: headers, top-level `let`s, `in`, the document
/// expression (`main`/`nxtoplevel`/`nxtopsubseq` in parser.mly).
#[derive(Parse, Unparse, Debug, Clone, PartialEq)]
pub struct File {
    pub headers: Vec<Header>,
    pub prelude: Vec<TopLet>,
    /// Required whenever `prelude` is non-empty (checked at elaboration).
    pub in_kw: Option<KwIn>,
    pub body: ast::Expr,
    pub eoi: EoiTok,
}

/// `@require:` / `@import:` header element. Accepted and currently ignored.
#[derive(Parse, Unparse, Debug, Clone, PartialEq)]
pub enum Header {
    Require(HeaderRequireTok),
    Import(HeaderImportTok),
}

/// A top-level non-recursive binding: `let name param* = expr`.
#[derive(Parse, Unparse, Debug, Clone, PartialEq)]
pub struct TopLet {
    pub let_kw: KwLet,
    pub name: VarTok,
    pub params: Vec<VarTok>,
    pub eq: DefEqTok,
    pub value: ast::Expr,
}

/// The mutually recursive expression/text grammar. Program expressions embed
/// inline/block text (`{…}`, `'<…>`), text embeds commands, and command
/// arguments re-enter program expressions — one strongly connected component.
///
/// `#[recurse]`'s depth engine only decrements at *self-referential* (root)
/// types, and every sub-cycle must pass through a root. `Expr` is the single
/// root (self-referencing through `Fun`); to keep it on *every* cycle, a
/// command's arguments are represented as one application-chain `Expr`
/// (`CmdTail::Args`) rather than a dedicated argument list — faithful to the
/// OCaml AST, where command arguments are a curried `UTApply` chain anyway.
/// Elaboration flattens that chain back into the argument list.
#[syan::parse::recurse]
pub mod ast {
    use super::super::leaf::*;
    use syan::parse::{Parse, Unparse};

    /// `nxlet`..`nxapp`: a function-ish expression.
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub enum Expr {
        /// `fun x y -> body`
        Fun {
            kw: KwFun,
            params: Vec<VarTok>,
            arrow: ArrowTok,
            body: Box<Expr>,
        },
        /// Application chain: `head arg1 arg2 …` (left-folded in elaboration).
        App { head: Atomic, args: Vec<Atomic> },
    }

    /// `nxbot`: an atomic expression.
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub enum Atomic {
        Length(LengthTok),
        Float(FloatTok),
        Int(IntTok),
        Literal(LiteralTok),
        True(KwTrue),
        False(KwFalse),
        Var(VarTok),
        /// `()`
        Unit { paren: ParenGroup<()> },
        /// `( expr )`
        Paren {
            paren: ParenGroup<()>,
            #[group(self.paren)]
            inner: Box<Expr>,
        },
        /// `(| label = expr; … |)`
        Record {
            rec: RecordGroup<()>,
            #[group(self.rec)]
            fields: Vec<RecordField>,
        },
        /// `[ expr; … ]`
        List {
            list: ListGroup<()>,
            #[group(self.list)]
            items: Vec<ListItem>,
        },
        /// `{ inline text }`
        InlineText {
            igrp: InlineGroup<()>,
            #[group(self.igrp)]
            elems: Vec<InlineElem>,
        },
        /// `'< block text >`
        BlockText {
            bgrp: BlockGroup<()>,
            #[group(self.bgrp)]
            elems: Vec<BlockElem>,
        },
    }

    /// One record field `label = expr;` (the last `;` is optional).
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub struct RecordField {
        pub name: VarTok,
        pub eq: DefEqTok,
        pub value: Expr,
        pub semi: Option<ListPunctTok>,
    }

    /// One list element `expr;` (the last `;` is optional).
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub struct ListItem {
        pub value: Expr,
        pub semi: Option<ListPunctTok>,
    }

    /// One inline-text element (`ih`/`ihtext`/`ihcmd` in parser.mly).
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub enum InlineElem {
        Char(CharTok),
        Space(SpaceTok),
        Break(BreakTok),
        /// `\cmd …`
        Cmd { name: HorzCmdTok, tail: CmdTail },
    }

    /// One block-text element (`vxbot`).
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub enum BlockElem {
        /// `+cmd …`
        Cmd { name: VertCmdTok, tail: CmdTail },
    }

    /// A command's arguments (`narg* sargs` in parser.mly). Either a bare `;`
    /// (no arguments) or the argument application chain — each argument is one
    /// `Atomic` of the chain: `(expr)`, `(|record|)`, `[list]`, `{inline}`,
    /// `<block>` — optionally `;`-terminated when the last argument is a
    /// program-mode one. The lexer's active-mode rules already reject
    /// everything else, so the chain shape is validated during elaboration.
    #[derive(Parse, Unparse, Debug, Clone, PartialEq)]
    pub enum CmdTail {
        /// `;` — no arguments.
        Semi(EndActiveTok),
        /// The argument chain.
        Args {
            args: Box<Expr>,
            semi: Option<EndActiveTok>,
        },
    }
}

/// A parse failure with the source position the parser got furthest to.
#[derive(Debug, thiserror::Error)]
#[error("{span}: parse error: {message}")]
pub struct ParseFileError {
    pub span: Span,
    pub message: String,
}

/// Lex and parse a whole `.saty` source file.
pub fn parse_file(src: &str) -> Result<File, ParseFileError> {
    let atoms = crate::lexer::lex(src).map_err(|e| ParseFileError {
        span: e.span,
        message: e.msg,
    })?;
    let mut stream = TokenStream::new(atoms);
    <File as Parse<_>>::parse(&mut stream).map_err(|e| ParseFileError {
        span: stream.high_water_span(),
        message: render_parse_error(&e),
    })
}

/// Flatten syan's nested error tree into one readable line.
fn render_parse_error(err: &syan::error::ParseError) -> String {
    format!("{err:?}")
}
