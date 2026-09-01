//! Slices 1-4 for the **0.1** grammar: layout of 0.1 program text — line
//! breaks, indentation and intra-line spacing — plus layout for the three
//! text areas.
//!
//! The 0.0.6 twin is [`super::build006`], and this file reproduces its
//! *design* rather than only its behaviour. The three load-bearing pieces,
//! restated because each of them was a bug there first:
//!
//! - **Emission is cursor-driven over the atom stream, not tree-driven.**
//!   [`Build::emit_atom`] always emits the atom under the cursor and the gap
//!   before it, so whatever the walk does or fails to do, every atom's bytes
//!   and every gap's bytes are emitted exactly once in source order (the tail
//!   loop in [`build`] finishes anything the walk did not reach). The tree only
//!   decides which [`Doc::Nest`] frame an atom lands in — its *indentation*. A
//!   construct this file does not descend into therefore degrades to "its
//!   continuation lines sit at the enclosing depth", never to a changed token
//!   stream, and that is what makes a construct-by-construct rollout safe.
//! - **Walk/cursor desync is the only quiet failure, so it is counted.**
//!   [`Build::leaf`] checks the atom under the cursor against the span of the
//!   CST leaf it was handed and bumps [`Build::desync`];
//!   `tests/format_cst_slice1_v01.rs` asserts the count is zero over the whole
//!   0.1 corpus. Nothing else could see a drift: it misattributes indentation,
//!   and the token stream is identical either way. In 0.0.6 it caught
//!   `OpNameTok`, which is three atoms behind one precomputed span; 0.1 inherits
//!   that leaf through [`crate::format_cst::build01::Build::bind_name`], and the
//!   check is what says so.
//! - **One `Nest` step per frame, and the hug where slice 1 had a line
//!   count.** Slice 1 gave the step to the first frame opened on a source
//!   line, because it kept the author's breaks and a human indents one step
//!   past the line a construct started on. Slice 3 decides the breaks, so
//!   there is no such line: [`Build::enter`] is unconditional, a frame whose
//!   contents stay flat contributes no line and therefore no indentation, and
//!   the case the line count existed for — `embed-block-top ctx (fun c ->`,
//!   whose lambda body belongs one step past the CALL and not three — is
//!   [`hugs_last`]'s.
//!
//! # Scope
//!
//! **The line structure is the formatter's**, chosen by
//! [`super::render`]'s fit decision against [`super::CstOptions::max_width`];
//! every line's indentation is recomputed from CST depth; and the intra-line
//! gaps are canonicalised by the SAME rules 0.0.6 uses
//! ([`super::build006::default_space`]). A gap the author broke is **joined**
//! unless the construct around it offers a break there — `LineBreaks::Preserve`
//! is gone, enum and all, because `engine.md` section 6's hazard class 1 says
//! a rule that both reads the author's breaks and overrides some of them is not
//! idempotent and cannot be made so. Which constructs offer one is
//! [`super::build006::Breaks`], shared with 0.0.6 rather than transcribed. That is the whole
//! reason the staging sigils need no rule of their own: `&e`, `~e`, `~x` and
//! `val persistent ~x` are emitted as their own atoms with the author's own
//! (usually empty) gaps between them, and this builder opens no frame and emits
//! no `Line` between a sigil and its operand, so it can neither break after one
//! nor pull the two apart. `&` `&` fusing into one binop is
//! [`super::sep::must_separate`]'s row and stays its job for the slice that
//! starts writing gaps; `staging_sigils_bind_to_their_operand` pins both halves.
//!
//! Text areas — `{ }`, `'< >`, `${ }` — are **slice 4**, and they are the one
//! place this builder is level with 0.0.6 rather than a slice behind: layout is
//! indentation, which is slice 1's business, not spacing. Block text and math
//! get full freedom (their whitespace is gaps, so there is no token there to
//! change); inline text gets re-indentation only, under the predicate
//! [`super::build006::Build::emit_swallowed`] states. [`Build::verbatim`]
//! survives as the `AreaPolicy::Verbatim` arm each [`Areas`] flag selects when
//! it is off.
//!
//! # What 0.1 adds over 0.0.6, and where it is handled
//!
//! `Bind` (`val`/`val inline`/`val block`/`val math`/`val rec`/`val mutable`/
//! `type`/`module`/`signature`/`include`), the whole `modexpr`/`sigexpr`/`decl`
//! layer including functors, `use`/`use package` headers, and per-binding stage
//! prefixes. Each has a walk arm below. The two structural differences that
//! change layout rather than only vocabulary:
//!
//! - a 0.1 library file **is** one `module M :> S = struct … end`
//!   ([`v1::FileV1::Library`]), so `sig`'s decls and `struct`'s binds are the
//!   file's two indented blocks rather than a nested construct;
//! - a 0.1 `match` is closed by `end` ([`ast::Expr::Match`]), which lands at the
//!   `match` keyword's own depth for the same reason its arms do.

use rustyfi_syntax::cst::Header;
use rustyfi_syntax::cst_v1::{self as v1, ast};
use rustyfi_syntax::leaf::{
    AnyHorzCmdTok, AnyMathCmdTok, AnyVertCmdTok, BlockGroup, InlineGroup, MathGroup,
};
use rustyfi_syntax::span::Span;
use rustyfi_syntax::token::Atom;
use syan::parse::unparse::Unparse;
use syan::span::Spanned;

use super::atoms_of;
use super::comment;
use super::build006::{
    blank_line_in_gap, default_space, is_column, preserved_lines, Area, Br, Breaks, Flat, Frame,
    Mark, Pass, Space, Spacing, DEFAULT_MAX_BLANK_LINES, SLICE2, SLICE3,
};
use super::doc::{Doc, Mode};
use super::inline;
use super::render;
use super::sep;
use super::trivia::{self, Piece};

/// How many line terminators `s` holds, counting a CRLF as one.
///
/// The output-line counter [`Build::group_close`] compares against. Every gap
/// and every token's own text is counted, because both can carry one: a header
/// swallows its terminator (`lexer.rs:915-933`) and a text area's bytes are
/// copied whole.
fn count_terminators(s: &str) -> usize {
    let b = s.as_bytes();
    let (mut n, mut i) = (0usize, 0usize);
    while i < b.len() {
        match b[i] {
            b'\r' => {
                n += 1;
                i += 1;
                if i < b.len() && b[i] == b'\n' {
                    i += 1;
                }
            }
            b'\n' => {
                n += 1;
                i += 1;
            }
            _ => i += 1,
        }
    }
    n
}

/// Build a `Doc` for a parsed 0.1 file.
///
/// `indent` is columns per depth step (`CstOptions::tab_spaces`).
///
/// `None` declines: an unsupported mode, an atom stream whose spans do not tile
/// and advance, or a gap holding something [`trivia::classify`] refuses — each
/// of which means this code has misread the stream, and `format.rs:336-343`'s
/// reflex applies.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build<'s>(
    source: &'s str,
    atoms: &[Atom],
    file: &v1::FileV1,
    indent: usize,
    breaks: Breaks,
    wrap: Option<usize>,
    wrap_inline: bool,
    max_blank_lines: usize,
) -> Option<Doc<'s>> {
    let mut b = Build::new(source, atoms, indent, breaks, wrap, wrap_inline, max_blank_lines);
    // Pass 1: for each output line, was anything but a column relaid out on
    // it, and where are its rule-claimed gaps? Skipped entirely when the
    // alignment pass is off.
    let preserve = match b.rules.preserve_alignment {
        false => Vec::new(),
        true => {
            b.file(file);
            while b.cursor < b.atoms.len() {
                b.emit_atom();
            }
            if b.broken {
                return None;
            }
            preserved_lines(&b.relaid, &b.marks)
        }
    };
    // Pass 2, from a clean slate: same input, same tree, same rules, so the
    // same line breaks in the same order.
    b.reset(Pass::Emit, preserve);
    b.file(file);
    // Anything the walk did not reach — there should be nothing, and the desync
    // counter is what says so, but losing a token's bytes because a grammar arm
    // was missed is not a failure mode this code gets to have.
    while b.cursor < b.atoms.len() {
        b.emit_atom();
    }
    let tail = source.get(b.byte..)?;
    if !tail.is_empty() {
        trivia::classify(tail)?;
        b.push(Doc::Verbatim(tail));
    }
    if b.broken || b.frames.len() != 1 {
        return None;
    }
    let frame = b.frames.pop()?;
    Some(Doc::concat(frame.parts))
}

/// How many CST leaves asked for an atom other than the one under the cursor.
///
/// Zero on a correct walk. Exposed for the corpus test rather than for the
/// formatter, which cannot act on it — see the module header.
#[cfg(test)]
pub(crate) fn gap_census(
    source: &str,
    atoms: &[Atom],
    file: &v1::FileV1,
) -> Vec<super::build006::Obs> {
    let mut b = Build::new(source, atoms, 2, SLICE3, None, false, DEFAULT_MAX_BLANK_LINES);
    b.census = Some(Vec::new());
    b.file(file);
    while b.cursor < b.atoms.len() {
        b.emit_atom();
    }
    b.census.take().unwrap_or_default()
}

pub(crate) fn walk_desync(source: &str, atoms: &[Atom], file: &v1::FileV1, indent: usize) -> usize {
    let mut b = Build::new(source, atoms, indent, SLICE3, None, false, DEFAULT_MAX_BLANK_LINES);
    b.file(file);
    b.desync
}

/// Is this expression a `let … in` spine node — one of the shapes
/// [`Build::expr`] walks iteratively?
fn is_spine(e: &ast::Expr) -> bool {
    use ast::Expr as E;
    matches!(
        e,
        E::LetIn { .. }
            | E::LetRecIn { .. }
            | E::LetMutableIn { .. }
            | E::LetPatternIn { .. }
            | E::OpenIn { .. }
    )
}

/// Is this atom a delimited group — something with an interior that can absorb
/// a line break of its own? The 0.1 twin of
/// [`super::build006::is_delimited`].
fn is_delimited(atom: &ast::Atomic) -> bool {
    use ast::Atomic as A;
    matches!(
        atom,
        A::Paren { .. }
            | A::Record { .. }
            | A::List { .. }
            | A::InlineText { .. }
            | A::BlockText { .. }
            | A::MathText { .. }
    )
}

/// Does this argument run end in an argument that should be **hugged** —
/// emitted outside the run's frame, with no break point in front of it?
/// See [`super::build006::hugs_last`] for the argument.
fn hugs_last(args: &[ast::AppArg]) -> bool {
    args.last().is_some_and(hugs_arg)
}

/// [`hugs_last`] for one argument, so a command's run — whose first argument
/// is a separate field in the CST — can ask the same question.
fn hugs_arg(last: &ast::AppArg) -> bool {
    match last {
        ast::AppArg::Atom { atom, accesses, .. } if accesses.is_empty() => is_delimited(atom),
        _ => false,
    }
}

/// Would a break before this body only push a delimiter onto the next line?
/// See [`super::build006::hugs_body`].
fn hugs_body(e: &ast::Expr) -> bool {
    let ast::Expr::Ops(c) = e else {
        return false;
    };
    if !c.tail.is_empty() {
        return false;
    }
    let a = &c.head;
    if a.minus.is_some() || a.stage.is_some() || a.excl.is_some() {
        return false;
    }
    match a.args.is_empty() {
        true => a.head_accesses.is_empty() && is_delimited(&a.head),
        false => hugs_last(&a.args),
    }
}

/// [`hugs_body`] for a `type … =` right-hand side: a bare record type or
/// parenthesised type absorbs its own break.
fn hugs_type_body(b: &v1::TypeBodyV1) -> bool {
    let v1::TypeBodyV1::Synonym(ty) = b else {
        return false;
    };
    let ast::TypeExpr::Atom(prod) = ty else {
        return false;
    };
    if !prod.rest.is_empty() {
        return false;
    }
    let ast::TypeApp::Atom(atom) = &prod.first else {
        return false;
    };
    use ast::TypeAtom as A;
    matches!(atom, A::Paren { .. } | A::Record { .. })
}

/// Slice 4's per-area policies, mirroring [`super::build006::Areas`] field for
/// field — including the arguments for why the first two are provable and the
/// third is measured. Read that type; nothing about the 0.1 lexer differs here,
/// because the text-mode lexer does not fork by generation.
#[derive(Debug, Clone, Copy)]
struct Areas {
    block: bool,
    math: bool,
    inline: bool,
}

/// The spacing rules the **0.1** walk runs: 0.0.6's, unchanged.
///
/// It is the same constant rather than a copy on purpose. The exception list
/// is measured against the 0.0.6 corpus because that is where the evidence
/// is — 162 files against 47 — and the 0.1 census agrees with every one of
/// its calls where it has an opinion at all: a variable head is 0 tight
/// against 812 spaced, a command head 43 tight against 0, a bracket 5,313
/// tight against 22, a `#` access 226 against 0.
///
/// The alignment pass is on here too, and that is what the first draft got
/// wrong: it was turned off on a guess that 0.1 has no hand-built tables, and
/// the census says the 47 files carry **484 multi-space gaps**, of which 465
/// are in the residue and concentrated in exactly the column shapes — 100
/// `val f  = …`, 95 `| Ctor    -> …`, 83 `val f  : τ`, 99 `\\cmd  =`/`\\cmd  :`.
/// Shipping without it would have flattened every one of them.
const V01: Spacing = SLICE2;

/// What slice 4 ships.
const SLICE4: Areas = Areas {
    block: true,
    math: true,
    inline: true,
};

/// Whether a leaf's own bytes are emitted verbatim or decomposed.
///
/// The 0.1 twin of [`super::build006::Subst`] minus its `Header` arm, which is
/// a slice-2 rule this builder does not carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Subst {
    /// Its own bytes.
    None,
    /// A horizontal-mode token whose span swallowed trivia; the trivia's
    /// indentation is dropped and the renderer supplies one.
    Swallow,
    /// [`Subst::Swallow`] for a group's OPENING delimiter, whose trailing
    /// trivia is held until the group's frame is pushed.
    SwallowOpen,
}

/// Does `s` hold a line terminator?
fn holds_break(s: &str) -> bool {
    s.contains('\n') || s.contains('\r')
}

/// How long a prefix of `s` the lexer would have skipped.
fn trivia_len(s: &str) -> usize {
    let b = s.as_bytes();
    let mut i = 0usize;
    while i < b.len() {
        match b[i] {
            b' ' | b'\t' | b'\r' | b'\n' => i += 1,
            b'%' => {
                while i < b.len() && b[i] != b'\n' && b[i] != b'\r' {
                    i += 1;
                }
            }
            _ => break,
        }
    }
    i
}

/// Split a horizontal-mode token's own bytes into
/// `(swallowed-before, the token proper, swallowed-after)`.
/// See [`super::build006::split_swallowed`] for the lexer citations.
fn split_swallowed(t: &str) -> (&str, &str, &str) {
    let lead = trivia_len(t);
    let rest = &t[lead..];
    let core = rest
        .find([' ', '\t', '\r', '\n', '%'])
        .unwrap_or(rest.len());
    (&t[..lead], &rest[..core], &rest[core..])
}

struct Build<'s, 'a> {
    source: &'s str,
    atoms: &'a [Atom],
    /// Index of the next atom to emit. The single source of emission order.
    cursor: usize,
    /// End byte of the last atom emitted; the start of the next gap.
    byte: usize,
    /// Open `Doc::Nest` frames, innermost last, each with the delta it will be
    /// closed with. `frames[0]` is the file and its delta is never used.
    frames: Vec<Frame<'s>>,

    /// Whether the output is currently at the start of a line, so the next line
    /// terminator would produce a *blank* line rather than end a
    /// content-bearing one. True at the start of the file, and true after a
    /// token whose own text ends in a terminator — which is what a `@require:`
    /// header is (`lexer.rs:915-933` swallows the line break into the token).
    at_line_start: bool,
    /// Columns per depth step.
    indent: i32,
    /// The budget an own-line `%` comment is reflowed to, or `None` for
    /// "never reflow a comment". See [`super::comment`].
    wrap: Option<usize>,
    /// **Slice 6.** Re-wrap inline text gap by gap, filling to
    /// [`super::CstOptions::max_width`]. See [`super::inline`] for the
    /// predicate and [`super::build006::Build::fill_gap`] for the mechanism,
    /// which this file mirrors field for field.
    wrap_inline: bool,
    /// Index of the output line being written. Only ever compared for equality
    /// with a remembered value — see [`Build::group_close`].
    line: usize,
    desync: usize,
    broken: bool,
    /// Which of slice 4's areas are laid out rather than copied.
    areas: Areas,
    /// A group opener's trailing swallowed trivia, waiting for the group's
    /// frame to be pushed. See [`Subst::SwallowOpen`].
    held: Option<&'s str>,
    /// How many inline-text areas the walk is currently inside; consulted only
    /// to refuse REFLOWING a `%` comment in somebody's prose. See
    /// [`super::build006::Build::inline_depth`].
    inline_depth: usize,
    /// Which area the walk is inside — the switch that says whether
    /// [`super::build006::default_space`] reaches this gap. See
    /// [`super::build006::Area`].
    area: Area,
    /// What a rule wants in the gap before the next atom, if anything. The
    /// 0.1 walk asks for exactly one thing, [`Space::Collapse`], at a
    /// command's argument boundary and after a unary minus. Everything else
    /// is the default.
    space: Option<Space>,
    /// Which spacing rules are on. See [`Build::rules`]' own value, `V01`.
    rules: Spacing,
    /// Which of slice 3's constructs offer the renderer a break.
    breaks: Breaks,
    /// [`super::CstOptions::max_blank_lines`] as the builder sees it — the
    /// zero case, which turns [`Breaks::block_blanks`] off rather than
    /// clamping it. See [`super::build006::Build`]'s field of the same name.
    max_blank_lines: usize,
    /// What the builder wants of the LINE STRUCTURE at the gap before the next
    /// atom. See [`super::build006::Br`].
    br: Option<Br>,
    /// Is the walk already inside an arrow chain's own group?
    /// [`Build::type_expr`] is the only reader.
    in_type_chain: bool,
    #[cfg(test)]
    census: Option<Vec<super::build006::Obs>>,
    /// Which of the two walks this is. See [`super::build006::Pass`] — the
    /// alignment pass is the same two-walk arrangement here, for the same
    /// reason, and `preserved_lines` is shared rather than transcribed.
    pass: Pass,
    /// `relaid[i]`: did a rule change something on output line `i` that was
    /// not a column? [`Pass::Scan`] only.
    relaid: Vec<bool>,
    /// `marks[i]`: every rule-claimed gap on output line `i`. [`Pass::Scan`]
    /// only.
    marks: Vec<Vec<Mark<'s>>>,
    /// `preserve[i]`: does output line `i` keep the runs the author wrote?
    preserve: Vec<bool>,
    /// The display column the output has reached on the line being written.
    col: usize,
    /// Whether the current line still owes its indentation.
    owed_indent: bool,
}

/// What [`Build::group_open`] hands [`Build::group_close`].
///
/// Slice 1 carried the opener's line indentation here, so the closer could be
/// anchored back to it; the `Group`/`Nest` shape replaces that. What is left
/// is the arrow-chain flag, because a delimiter starts a new type chain and
/// its closer ends one ([`Build::type_expr`]).
#[derive(Debug, Clone, Copy)]
struct GroupAnchor {
    chain: bool,
}

impl<'s, 'a> Build<'s, 'a> {
    #[allow(clippy::too_many_arguments)]
    fn new(
        source: &'s str,
        atoms: &'a [Atom],
        indent: usize,
        breaks: Breaks,
        wrap: Option<usize>,
        wrap_inline: bool,
        max_blank_lines: usize,
    ) -> Self {
        Build {
            source,
            atoms,
            cursor: 0,
            byte: 0,
            frames: vec![Frame::nest(0)],
            at_line_start: true,
            indent: indent as i32,
            wrap,
            wrap_inline,
            line: 0,
            desync: 0,
            broken: false,
            areas: SLICE4,
            held: None,
            inline_depth: 0,
            area: Area::Program,
            space: None,
            rules: V01,
            breaks,
            max_blank_lines,
            br: None,
            in_type_chain: false,
            #[cfg(test)]
            census: None,
            pass: Pass::Scan,
            relaid: Vec::new(),
            marks: Vec::new(),
            preserve: Vec::new(),
            col: 0,
            owed_indent: false,
        }
    }

    /// Rewind to the start of the file for the second walk. `preserve` is the
    /// one thing carried across, which is the whole point of there being two.
    fn reset(&mut self, pass: Pass, preserve: Vec<bool>) {
        self.cursor = 0;
        self.byte = 0;
        self.frames = vec![Frame::nest(0)];
        self.at_line_start = true;
        self.broken = false;
        self.space = None;
        self.br = None;
        self.in_type_chain = false;
        self.pass = pass;
        self.line = 0;
        self.held = None;
        self.inline_depth = 0;
        self.area = Area::Program;
        self.preserve = preserve;
        self.col = 0;
        self.owed_indent = false;
    }

    /// Does this output line keep the runs the author wrote? [`Pass::Emit`]
    /// only.
    fn preserved_line(&self) -> bool {
        self.preserve.get(self.line).copied().unwrap_or(false)
    }

    /// Record a rule-claimed gap of `width` columns before the atom under the
    /// cursor. [`Pass::Scan`] only.
    fn note_mark(&mut self, width: usize, run: bool) {
        let len = self.source.len();
        let Some(text) = self.atoms.get(self.cursor).and_then(|a| {
            self.source
                .get(a.span.start.byte.min(len)..a.span.end.byte.min(len))
        }) else {
            return;
        };
        if self.marks.len() <= self.line {
            self.marks.resize(self.line + 1, Vec::new());
        }
        let col = self.col + width;
        let idx = self.marks[self.line].len();
        self.marks[self.line].push(Mark { col, text, run, idx });
    }

    /// Record that a rule changed something on this line. [`Pass::Scan`] only.
    fn mark_relaid(&mut self) {
        if self.relaid.len() <= self.line {
            self.relaid.resize(self.line + 1, false);
        }
        self.relaid[self.line] = true;
    }

    // -- frames --------------------------------------------------------------

    fn push(&mut self, d: Doc<'s>) {
        self.advance(&d);
        self.push_raw(d);
    }

    fn push_raw(&mut self, d: Doc<'s>) {
        match self.frames.last_mut() {
            Some(f) => f.parts.push(d),
            None => self.broken = true,
        }
    }

    /// Track what [`super::render`] would do to its column counter for `d`.
    ///
    /// The 0.1 twin of [`super::build006::Build::advance`], transcribed from
    /// the renderer's own arms including the lazy indent. Read that one for
    /// why slice 6's fill point is the place this stops being exact.
    fn advance(&mut self, d: &Doc<'s>) {
        match d {
            Doc::Token { text, .. } | Doc::Verbatim(text) => {
                if text.is_empty() {
                    return;
                }
                if self.owed_indent {
                    self.owed_indent = false;
                    self.col = self.level().max(0) as usize * self.indent.max(0) as usize;
                }
                match text.rfind(['\n', '\r']) {
                    Some(i) => self.col = render::width(&text[i + 1..]),
                    None => self.col += render::width(text),
                }
                if text.ends_with('\n') || text.ends_with('\r') {
                    self.owed_indent = true;
                }
            }
            Doc::VerbatimIndent(text) => {
                if self.owed_indent || self.col == 0 {
                    self.owed_indent = false;
                    self.col = render::width(text);
                }
            }
            Doc::HardLine => {
                self.owed_indent = true;
                self.col = 0;
            }
            Doc::FillLine => {
                if self.owed_indent {
                    self.owed_indent = false;
                    self.col = self.level().max(0) as usize * self.indent.max(0) as usize;
                }
                self.col += 1;
            }
            Doc::BlankLine => {}
            Doc::Nil | Doc::Concat(_) | Doc::Nest(..) | Doc::Group(..) => {}
            Doc::Line | Doc::SoftLine => {}
        }
    }

    /// Open a bare nest frame: one indentation step for whatever breaks inside
    /// it. The 0.1 twin of [`super::build006::Build::enter`], including why
    /// slice 3 made it unconditional — with the breaks decided rather than
    /// read there is no "line the construct started on" to be one step past,
    /// and the case slice 1's rule existed for survives through the hug
    /// ([`hugs_last`]) instead.
    fn enter(&mut self) {
        self.frames.push(Frame::nest(1));
    }

    /// Current indentation, in steps: the sum of the open frames' deltas.
    fn level(&self) -> i32 {
        self.frames.iter().map(|f| f.delta).sum()
    }

    /// Open a [`Mode`] group frame. Paired with [`Build::exit`].
    fn push_group(&mut self, mode: Mode) {
        self.frames.push(Frame::grouped(mode));
    }

    /// Close the innermost frame.
    fn exit(&mut self) {
        let Some(frame) = self.frames.pop() else {
            self.broken = true;
            return;
        };
        let inner = Doc::concat(frame.parts);
        let d = match frame.delta {
            0 => inner,
            n => Doc::Nest(n * self.indent, Box::new(inner)),
        };
        let d = match frame.group {
            Some(mode) => Doc::Group(mode, Box::new(d)),
            None => d,
        };
        // `push_raw`: the frame's contents were counted as they were emitted.
        self.push_raw(d);
    }

    /// Emit a group's opening delimiter, open its group and interior frames
    /// and offer a break just inside it.
    ///
    /// `open  Group(Auto, [ Nest(1, [ Opportunity, contents ]), Opportunity,
    /// close ])`, exactly as [`super::build006::Build::group_open`] — the
    /// opener outside so the group is measured from where its contents start,
    /// the closer inside so `fits` counts it, and the closer outside the
    /// `Nest` so a broken group puts it back at the opener's own depth.
    fn group_open<T: Spanned<Span = Span>>(&mut self, open: &T) -> GroupAnchor {
        self.group_open_as(open, Subst::None)
    }

    /// [`Build::group_open`] for a delimiter whose span swallowed trivia.
    fn group_open_as<T: Spanned<Span = Span>>(&mut self, open: &T, subst: Subst) -> GroupAnchor {
        self.leaf_as(open, subst);
        // A delimiter resets the arrow chain: a type inside `( … )` or
        // `(| … |)` is a chain of its own, and the caller's is restored by the
        // matching `group_close`.
        let chain = std::mem::replace(&mut self.in_type_chain, false);
        match self.breaks.groups {
            true => {
                self.push_group(Mode::Auto);
                self.frames.push(Frame::nest(1));
                self.br(Br::Opportunity);
            }
            false => self.frames.push(Frame::nest(1)),
        }
        if let Some(t) = self.held.take() {
            self.swallowed_part(t, false);
        }
        GroupAnchor { chain }
    }

    /// Open a group whose interior is **not** the renderer's to lay out:
    /// inline text, whose every whitespace run is a token, and a math group
    /// that is not the outermost one.
    fn area_open_as<T: Spanned<Span = Span>>(&mut self, open: &T, subst: Subst) {
        self.leaf_as(open, subst);
        self.frames.push(Frame::nest(1));
        if let Some(t) = self.held.take() {
            self.swallowed_part(t, false);
        }
    }

    /// Close a group opened by [`Build::group_open`].
    fn group_close<T: Spanned<Span = Span>>(&mut self, g: GroupAnchor, close: &T) {
        self.group_close_as(g, close, Subst::None)
    }

    /// [`Build::group_close`] for a delimiter whose span swallowed the
    /// whitespace run in front of it — which `}` always does
    /// (`lexer.rs:1122-1126`).
    fn group_close_as<T: Spanned<Span = Span>>(&mut self, g: GroupAnchor, close: &T, subst: Subst) {
        self.in_type_chain = g.chain;
        self.exit();
        if self.breaks.groups {
            self.br(Br::Opportunity);
        }
        self.leaf_as(close, subst);
        if self.breaks.groups {
            self.exit();
        }
    }

    /// [`Build::group_close_as`] for an area opened by
    /// [`Build::area_open_as`].
    fn area_close_as<T: Spanned<Span = Span>>(&mut self, close: &T, subst: Subst) {
        self.exit();
        self.leaf_as(close, subst);
    }

    // -- atoms and gaps ------------------------------------------------------

    /// Ask for a particular gap before the **next** atom emitted.
    fn space(&mut self, want: Space) {
        self.space = Some(want);
    }

    /// Ask for a break opportunity in the gap before the **next** atom.
    /// See [`super::build006::Build::br`].
    fn br(&mut self, want: Br) {
        self.br = Some(want);
    }

    /// A line ends here because the construct is a **block**: one top-level
    /// binding, one `sig` decl, one `struct` bind per line, whatever the
    /// width — and the only positions at which the author's blank lines
    /// survive.
    fn block_break(&mut self) {
        if self.breaks.blocks {
            self.br(Br::Hard);
        }
    }

    /// A line ends here because the construct is a **clause list**: a `match`
    /// arm, a `val rec` clause, a variant constructor.
    fn clause_break(&mut self) {
        if self.breaks.clauses {
            self.br(Br::Hard);
        }
    }

    /// A break opportunity between two items of a `,`/`;`-separated run.
    /// `n` is the item's index; the first item's break point is the one
    /// [`Build::group_open`] already offered.
    fn item_break(&mut self, n: usize) {
        if n > 0 && self.breaks.items {
            self.br(Br::Opportunity);
        }
    }

    /// The line being built is not one the builder lays out exactly, so no
    /// column on it may be preserved. See
    /// [`super::build006::Build::mark_inexact`].
    fn mark_inexact(&mut self) {
        self.mark_relaid();
    }

    /// What the spacing policy would write in this gap if the line did not
    /// break here. See [`super::build006::Build::flat_spelling`].
    fn flat_spelling(&self, want: Space, gap: &str, no_break: bool) -> Flat {
        // `prev` off the atom stream, exactly as [`Build::canonical_space`]
        // does here: 0.1 has no `last_text` field because no leaf in this
        // grammar emits substitute bytes whose spelling differs from its own.
        let separate = || {
            let len = self.source.len();
            let text = |i: usize| {
                self.atoms
                    .get(i)
                    .and_then(|a: &Atom| {
                        self.source
                            .get(a.span.start.byte.min(len)..a.span.end.byte.min(len))
                    })
                    .unwrap_or("")
            };
            let prev = self.cursor.checked_sub(1).map(text).unwrap_or("");
            sep::must_separate(prev, text(self.cursor))
        };
        match want {
            Space::One => Flat::Space,
            Space::Tight if no_break && gap.is_empty() => Flat::Empty,
            Space::Tight => match separate() {
                true => Flat::Space,
                false => Flat::Empty,
            },
            Space::Collapse if no_break && gap.is_empty() => Flat::Empty,
            Space::Collapse => Flat::Space,
            Space::Keep => Flat::Keep,
        }
    }

    /// Enter `area`, returning the one to restore. See
    /// [`super::build006::Build::enter_area`].
    fn enter_area(&mut self, area: Area) -> Area {
        std::mem::replace(&mut self.area, area)
    }

    /// A command head's argument boundary: neither tightened nor spaced, but
    /// a run there still collapses. See
    /// [`super::build006::Build::cmd_arg_boundary`].
    fn cmd_arg_boundary(&mut self) {
        if self.rules.cmd_arg {
            self.space(Space::Collapse);
        }
    }

    /// What the gap before the atom under the cursor should hold when no rule
    /// asked. One call into the shared decision — a second transcription of
    /// the exception list is exactly the fork this design exists to avoid.
    fn default_space(&self) -> Space {
        let prev = self
            .cursor
            .checked_sub(1)
            .and_then(|i| self.atoms.get(i))
            .map(|a| &a.slot);
        let next = self.atoms.get(self.cursor).map(|a| &a.slot);
        default_space(&self.rules, self.area, prev, next)
    }

    /// Write the gap a rule asked for, in place of the one the author wrote.
    ///
    /// The 0.1 twin of [`super::build006::Build::canonical_space`], minus the
    /// alignment pass (see [`V01`]). `gap` is guaranteed to be spaces and
    /// tabs only and the output is not at the start of a line, so there is
    /// nothing in the range this can destroy.
    fn canonical_space(&mut self, want: Space, gap: &'s str) {
        match want {
            Space::One => {
                let column = self.rules.preserve_alignment && is_column(gap);
                match self.pass {
                    Pass::Scan => {
                        if !column && gap != " " {
                            self.mark_relaid();
                        }
                        let text = match column {
                            true => gap,
                            false => " ",
                        };
                        self.note_mark(render::width(text), column);
                        self.push(Doc::Verbatim(text));
                        return;
                    }
                    Pass::Emit => {
                        if column && self.preserved_line() {
                            self.push(Doc::Verbatim(gap));
                            return;
                        }
                    }
                }
                self.push(Doc::Verbatim(" "));
            }
            // Empty stays empty, everything else becomes one space. A
            // delegation and not a third copy, so that the alignment pass on
            // the `One` arm reaches a collapse request too — a hand-built
            // column at a `Collapse` boundary is a column like any other, and
            // writing `" "` here directly would flatten it. Unreachable while
            // only math and block text asked for `Collapse` (0.1's two areas
            // hold 99 gaps between them and not one run); the three
            // exceptions that now ask for it are what make it reachable.
            Space::Collapse => {
                let want = match gap.is_empty() {
                    true => Space::Keep,
                    false => Space::One,
                };
                self.canonical_space(want, gap);
            }
            Space::Keep => {
                if !gap.is_empty() {
                    self.push(Doc::Verbatim(gap));
                }
            }
            // An empty gap is already tight AND is a proof that the two
            // tokens do not fuse — see the 0.0.6 twin.
            Space::Tight if gap.is_empty() => {
                if self.pass == Pass::Scan {
                    self.note_mark(0, false);
                }
            }
            Space::Tight => {
                // The one direction that can fuse two tokens, so it asks the
                // one fusion authority rather than a second opinion.
                let len = self.source.len();
                let text = |i: usize| {
                    self.atoms
                        .get(i)
                        .and_then(|a: &Atom| {
                            self.source
                                .get(a.span.start.byte.min(len)..a.span.end.byte.min(len))
                        })
                        .unwrap_or("")
                };
                let prev = self.cursor.checked_sub(1).map(text).unwrap_or("");
                let separate = sep::must_separate(prev, text(self.cursor));
                let canonical = match separate {
                    true => " ",
                    false => "",
                };
                if self.pass == Pass::Scan {
                    if gap != canonical {
                        self.mark_relaid();
                    }
                    self.note_mark(usize::from(separate), false);
                }
                if separate {
                    self.push(Doc::Verbatim(" "));
                }
            }
        }
    }

    /// Emit the gap ending at `start`.
    ///
    /// Slice 1's whole rule for gap bytes, in two cases:
    ///
    /// 1. **The gap holds no line break and the output is mid-line** — copied
    ///    byte for byte. Hand alignment, the two spaces before a trailing
    ///    comment, `a=1` written tight: all survive, because slice 1 has no
    ///    opinion about intra-line spacing and inventing one would need
    ///    [`super::sep::must_separate`].
    /// 2. **Otherwise** — the break structure is reproduced as
    ///    `HardLine`/`BlankLine`, each `%` comment is copied verbatim, and the
    ///    *indentation* in the gap is dropped: the renderer supplies it from the
    ///    enclosing [`Doc::Nest`] chain. That subsumes tab expansion and
    ///    trailing-whitespace trimming for free.
    fn gap_upto(&mut self, start: usize) {
        let want = self.space.take();
        let br = self.br.take();
        if start < self.byte {
            self.broken = true;
            return;
        }
        let Some(gap) = self.source.get(self.byte..start) else {
            self.broken = true;
            return;
        };
        let Some(pieces) = trivia::classify(gap) else {
            // Not a gap. Something has been misread; say so rather than
            // guessing, and let `build`'s caller decline.
            self.broken = true;
            return;
        };
        let no_break = !pieces.iter().any(|p| matches!(p, Piece::Newline(_)));
        #[cfg(test)]
        let blank = pieces.iter().all(|p| matches!(p, Piece::Space(_)));
        let commented = pieces.iter().any(|p| matches!(p, Piece::Comment(_)));
        #[cfg(test)]
        if let Some(c) = self.census.as_mut() {
            c.push(super::build006::Obs {
                at: self.cursor,
                span: (self.byte, start),
                area: self.area,
                want,
                has_break: !no_break,
                blank,
                at_line_start: self.at_line_start,
            });
        }
        // 1. A comment owns its own line structure, wherever it sits — and the
        //    `Doc::HardLine` it brings is what forces its enclosing group open,
        //    so a comment is never measured in flat mode.
        if commented {
            let reflow = self.inline_depth == 0;
            self.break_structure(pieces, false, reflow);
            return;
        }
        // 2. A break the BUILDER asked for, and the author's blank lines with
        //    it. The only position at which a blank line survives.
        if br == Some(Br::Hard) {
            match (no_break, self.at_line_start) {
                (false, _) => self.break_structure(pieces, false, false),
                (true, false) => {
                    self.line += 1;
                    self.push(Doc::HardLine);
                }
                // A header has already ended its own line, so this emits no
                // terminator — only the blank lines the gap holds.
                (true, true) => {}
            }
            self.at_line_start = true;
            return;
        }
        // 3. **Math keeps the author's line structure.** The 0.1 twin of
        //    [`super::build006::Build::gap_upto`]'s case 3 — read that
        //    comment for why an area boundary is not `engine.md` section 6's
        //    hazard class 1: the two regimes are disjoint by AREA, and an area
        //    is a property of the tree rather than of the layout.
        if !no_break && self.area == Area::Math {
            self.break_structure(pieces, false, false);
            return;
        }
        // 4. Indentation, which the renderer owns.
        if self.at_line_start {
            return;
        }
        // 5. A break opportunity, or a canonical space, in place of whatever
        //    the author wrote — line breaks included. This is the whole of
        //    `LineBreaks::Preserve`'s removal: outside a text area there is no
        //    branch here that reproduces an author's break.
        let want = match (want, self.rules.universal && self.area.claimed()) {
            (Some(w), _) => w,
            (None, true) => self.default_space(),
            (None, false) => Space::Keep,
        };
        let spelling = self.flat_spelling(want, gap, no_break);
        match (br, spelling) {
            (Some(Br::Opportunity), Flat::Space) => {
                self.mark_inexact();
                self.push(Doc::Line);
            }
            (Some(Br::Opportunity), _) => {
                self.mark_inexact();
                self.push(Doc::SoftLine);
            }
            (Some(Br::Fill), Flat::Space) => {
                self.mark_inexact();
                self.push(Doc::FillLine);
            }
            _ => match no_break {
                true => self.canonical_space(want, gap),
                false => match spelling {
                    Flat::Space => self.push(Doc::Verbatim(" ")),
                    Flat::Empty | Flat::Keep => {}
                },
            },
        }
    }

    /// Reproduce a run of trivia's break structure, dropping every indent in it
    /// so the renderer supplies one from the enclosing [`Doc::Nest`] chain.
    ///
    /// The 0.1 twin of [`super::build006::Build::break_structure`], including
    /// `keep_first_space` — the flag that keeps a `Token::Space` from becoming
    /// a `Token::Break` when its leading horizontal whitespace is dropped.
    fn break_structure(
        &mut self,
        pieces: Vec<Piece<'s>>,
        keep_first_space: bool,
        reflow_comments: bool,
    ) {
        let mut pending_space: Option<&'s str> = None;
        // Whether the *next* piece would begin a line. Not `self.at_line_start`
        // alone, because it is also what distinguishes the first terminator in
        // this gap from a blank-line one.
        let mut line_start = self.at_line_start;
        let mut first = true;
        for p in pieces {
            match p {
                // Held: whether it survives depends on what follows it. Before a
                // terminator it is trailing whitespace and dies; before a
                // comment on a content line it is the author's spacing and lives
                // verbatim; before a comment on a line of its own it is that
                // line's *indentation*, which is what this slice owns.
                Piece::Space(s) => match first && keep_first_space {
                    true => {
                        self.push(Doc::Verbatim(" "));
                        self.at_line_start = false;
                        line_start = false;
                    }
                    false => pending_space = Some(s),
                },
                Piece::Comment(c) => {
                    match line_start {
                        true => self.own_line_comment(pending_space.take(), c, reflow_comments),
                        false => {
                            if let Some(s) = pending_space.take() {
                                self.push(Doc::Verbatim(s));
                            }
                            self.push(Doc::Verbatim(c));
                        }
                    }
                    self.at_line_start = false;
                    line_start = false;
                }
                Piece::Newline(_) => {
                    self.line += 1;
                    pending_space = None;
                    match self.at_line_start {
                        true => self.push(Doc::BlankLine),
                        false => self.push(Doc::HardLine),
                    }
                    self.at_line_start = true;
                    line_start = true;
                }
            }
            first = false;
        }
        // A trailing run of spaces is the next line's indentation. Dropped: the
        // renderer writes it, from the nesting rather than from the input.
    }

    /// A `%` comment that sits on a line of its own, with whatever leading
    /// whitespace the author gave it.
    ///
    /// **The author's indentation is kept**, which is the variant the 0.0.6
    /// builder measured and chose (`build006::Build::own_line_comment` carries
    /// the corpus table): a block of `%`-disabled code parked at column 0 was
    /// parked there deliberately, and moving it asserts a scope membership the
    /// author declined to give it. This is the one place slice 1 does not
    /// re-indent a line, and [`Doc::VerbatimIndent`] is why it is still a
    /// fixpoint — the bytes written are the bytes read.
    fn own_line_comment(&mut self, indent: Option<&'s str>, c: &'s str, reflow: bool) {
        let indent = indent.unwrap_or("");
        if let (Some(budget), true) = (self.wrap, reflow) {
            if let Some((marker, lines)) = comment::reflow(c, render::width(indent), budget) {
                for (i, line) in lines.iter().enumerate() {
                    if i > 0 {
                        self.push(Doc::HardLine);
                    }
                    self.push(Doc::VerbatimIndent(indent));
                    self.push(Doc::Verbatim(marker));
                    self.push(Doc::Verbatim(line));
                }
                return;
            }
        }
        self.push(Doc::VerbatimIndent(indent));
        self.push(Doc::Verbatim(c));
    }

    /// Emit the atom under the cursor, and the gap before it.
    fn emit_atom(&mut self) {
        self.emit_atom_as(Subst::None);
    }

    /// Emit the atom under the cursor, and the gap before it, decomposing the
    /// trivia its span swallowed when asked.
    fn emit_atom_as(&mut self, subst: Subst) {
        let Some(atom) = self.atoms.get(self.cursor) else {
            self.broken = true;
            return;
        };
        let len = self.source.len();
        let start = atom.span.start.byte.min(len);
        let end = atom.span.end.byte.min(len).max(start);
        if start < self.byte {
            // Spans must tile and advance (`atoms_roundtrip.rs` pins it).
            self.broken = true;
            return;
        }
        self.gap_upto(start);
        match self.source.get(start..end) {
            // Its own bookkeeping, because the pieces decide where the lines
            // fall rather than the token's last byte.
            Some(text)
                if matches!(subst, Subst::Swallow | Subst::SwallowOpen) && !text.is_empty() =>
            {
                self.emit_swallowed(text, subst == Subst::SwallowOpen);
            }
            Some(text) if !text.is_empty() => {
                self.line += count_terminators(text);
                self.at_line_start = text.ends_with('\n') || text.ends_with('\r');
                self.push(Doc::Token {
                    text,
                    atom: self.cursor,
                });
            }
            Some(_) => {}
            None => self.broken = true,
        }
        self.byte = end;
        self.cursor += 1;
    }

    /// A horizontal-mode token, with the indentation it swallowed recomputed.
    ///
    /// The 0.1 twin of [`super::build006::Build::emit_swallowed`], which
    /// carries the argument for why this may edit bytes inside a token's span,
    /// and what the one mistake is.
    fn emit_swallowed(&mut self, text: &'s str, hold_trail: bool) {
        let (lead, core, trail) = split_swallowed(text);
        if !hold_trail && !holds_break(lead) && !holds_break(trail) {
            self.at_line_start = false;
            self.push(Doc::Token {
                text,
                atom: self.cursor,
            });
            return;
        }
        self.swallowed_part(lead, core.is_empty());
        if !core.is_empty() {
            self.push(Doc::Verbatim(core));
            self.at_line_start = false;
        }
        match hold_trail {
            true => self.held = Some(trail),
            false => self.swallowed_part(trail, false),
        }
    }

    /// **Slice 6**, the 0.1 twin of
    /// [`super::build006::Build::fill_gap`], mechanism for mechanism.
    ///
    /// No `mark_relaid` counterpart, and that is a real difference rather
    /// than an omission: this file has no [`super::build006::Spacing`] and no
    /// alignment pass, so there is no column bookkeeping for a fill point to
    /// invalidate. The predicate itself is shared verbatim
    /// ([`super::inline`]) precisely so the two generations cannot disagree
    /// about which gap may move.
    fn fill_gap(&mut self) {
        let Some(atom) = self.atoms.get(self.cursor) else {
            self.broken = true;
            return;
        };
        let len = self.source.len();
        let start = atom.span.start.byte.min(len);
        let end = atom.span.end.byte.min(len).max(start);
        if start < self.byte {
            self.broken = true;
            return;
        }
        self.gap_upto(start);
        self.mark_relaid();
        self.push(Doc::FillLine);
        self.at_line_start = false;
        self.byte = end;
        self.cursor += 1;
    }

    /// A `Space` or `Break` inside inline text: a fill point if slice 6 may
    /// re-spell it, and slice 4's re-indented run if not.
    fn inline_gap<T: Spanned<Span = Span>>(&mut self, t: &T) {
        let len = self.source.len();
        let text = self
            .atoms
            .get(self.cursor)
            .and_then(|a| self.source.get(a.span.start.byte.min(len)..a.span.end.byte.min(len)))
            .unwrap_or("");
        let fill = self.wrap_inline
            && inline::run_bytes_allow_reflow(text)
            && inline::gap_is_reflowable(self.atoms, self.cursor);
        match fill {
            true => {
                if self.atoms.get(self.cursor).map(|a| a.span.start.byte)
                    != Some(t.span().start.byte)
                {
                    self.desync += 1;
                }
                self.fill_gap();
            }
            false => self.leaf_as(t, Subst::Swallow),
        }
    }

    /// One of [`Build::emit_swallowed`]'s two trivia runs.
    fn swallowed_part(&mut self, part: &'s str, keep_first_space: bool) {
        if part.is_empty() {
            return;
        }
        if !holds_break(part) {
            self.push(Doc::Verbatim(part));
            self.at_line_start = false;
            return;
        }
        let Some(pieces) = trivia::classify(part) else {
            self.broken = true;
            return;
        };
        // Never reflowed: a comment reached from here is inside inline text.
        self.break_structure(pieces, keep_first_space, false);
    }

    /// Emit one atom, checking it is the one this CST leaf names.
    fn leaf<T: Spanned<Span = Span>>(&mut self, t: &T) {
        self.leaf_at(t.span());
    }

    /// [`Build::leaf`], emitting the atom under a substitution.
    fn leaf_as<T: Spanned<Span = Span>>(&mut self, t: &T, subst: Subst) {
        if self.atoms.get(self.cursor).map(|a| a.span.start.byte) != Some(t.span().start.byte) {
            self.desync += 1;
        }
        self.emit_atom_as(subst);
    }

    fn leaf_at(&mut self, span: Span) {
        if self.atoms.get(self.cursor).map(|a| a.span.start.byte) != Some(span.start.byte) {
            self.desync += 1;
        }
        self.emit_atom();
    }

    /// A binding-position name.
    ///
    /// `BindName` is either a `VarTok` (one atom) or an `OpNameTok` (three) —
    /// `val ( +++> ) = …`, which the 0.1 corpus writes too. Its `span` cannot
    /// tell the two apart and its `repr` is private, so the count comes from the
    /// node. This is the leaf whose *precomputed* span desynced 788 atoms of one
    /// 0.0.6 file before the check was added.
    fn bind_name(&mut self, n: &rustyfi_syntax::cst::BindName) {
        match atoms_of(n).len() {
            1 => self.leaf_at(n.span),
            count => {
                if self.atoms.get(self.cursor).map(|a| a.span.start.byte) != Some(n.span.start.byte)
                {
                    self.desync += 1;
                }
                self.flat_n(count);
            }
        }
    }

    /// Emit a whole node's atoms at the current depth, keeping its breaks.
    ///
    /// The leaf treatment for the parts of the grammar slice 1 does not lay out
    /// — patterns and math. Their line breaks survive and their continuation
    /// lines are re-indented to the enclosing depth, which is the honest answer
    /// for a construct nobody has written a rule for yet.
    fn flat<T: Unparse<Atom> + ?Sized>(&mut self, node: &T) {
        self.flat_n(atoms_of(node).len());
    }

    fn flat_n(&mut self, n: usize) {
        for _ in 0..n {
            if self.cursor >= self.atoms.len() {
                break;
            }
            self.emit_atom();
        }
    }

    /// Emit a whole node as one untouched byte range: the area boundary.
    ///
    /// Used for inline text, block text and math and for nothing else. The
    /// interior — comments, prose, nested program text — is copied, so no rule
    /// in this slice can reach a byte the ground-truth measurement governs.
    fn verbatim<T: Unparse<Atom> + ?Sized>(&mut self, node: &T) {
        self.verbatim_n(atoms_of(node).len());
    }

    /// [`Build::verbatim`] for a group whose delimiters are not part of the
    /// element list: the two delimiters plus every element's atoms. Reached
    /// only when an area's flag is off while an enclosing area's is on, which
    /// nothing ships.
    fn verbatim_elems<T: Unparse<Atom>>(&mut self, elems: &[T]) {
        let n = 2 + elems.iter().map(|e| atoms_of(e).len()).sum::<usize>();
        self.verbatim_n(n);
    }

    fn verbatim_n(&mut self, n: usize) {
        if n == 0 || self.cursor + n > self.atoms.len() {
            self.flat_n(n);
            return;
        }
        let len = self.source.len();
        let start = self.atoms[self.cursor].span.start.byte.min(len);
        let end = self.atoms[self.cursor + n - 1]
            .span
            .end
            .byte
            .min(len)
            .max(start);
        if start < self.byte {
            self.broken = true;
            return;
        }
        self.gap_upto(start);
        match self.source.get(start..end) {
            Some(text) if !text.is_empty() => {
                self.line += count_terminators(text);
                self.at_line_start = text.ends_with('\n') || text.ends_with('\r');
                self.push(Doc::Verbatim(text));
            }
            Some(_) => {}
            None => self.broken = true,
        }
        self.byte = end;
        self.cursor += n;
    }

    /// Open the frame a **body** lands in — a body being anything introduced
    /// by a token already emitted on this line: a `then`/`else` keyword, a
    /// `->` arrow, a `=`, a functor's `->`.
    ///
    /// One [`Mode::Auto`] group over `Nest(1, [ Opportunity, body ])`: the body
    /// stays on the introducer's line if it fits there, and otherwise takes
    /// the next line one step in. See
    /// [`super::build006::Build::body`] for the argument, which is the same
    /// one — the group is what stops a long field value inside a broken record
    /// from breaking after its own `=`.
    fn open_body(&mut self) {
        match self.breaks.bodies {
            true => {
                self.push_group(Mode::Auto);
                self.frames.push(Frame::nest(1));
                self.br(Br::Opportunity);
            }
            false => self.enter(),
        }
    }

    /// Close what [`Build::open_body`] opened.
    fn close_body(&mut self) {
        self.exit();
        if self.breaks.bodies {
            self.exit();
        }
    }

    fn body(&mut self, e: &ast::Expr) {
        // A hugged body opens NO frame at all: its own delimiter is what
        // indents its contents ([`hugs_body`]).
        if self.breaks.bodies && hugs_body(e) {
            self.expr(e);
            return;
        }
        self.open_body();
        self.expr(e);
        self.close_body();
    }

    /// One element of a `,`-separated run: a list item, a tuple component, a
    /// record field, an optional-argument entry.
    ///
    /// **No frame of its own.** The elements of a run are siblings and must
    /// all get the same answer, and the enclosing group's own `Nest` already
    /// supplies it — a further step per element is the staircase a 147-entry
    /// colour table produced in 0.0.6 when this shared the body's anchor.
    fn element(&mut self, e: &ast::Expr) {
        self.expr(e);
    }

    // -- files ---------------------------------------------------------------

    fn file(&mut self, f: &v1::FileV1) {
        match f {
            v1::FileV1::Document { headers, body, eoi } => {
                self.headers(headers);
                // The document body sits at the file's own depth, as the corpus
                // writes it.
                self.block_break();
                self.expr(body);
                self.leaf(eoi);
            }
            v1::FileV1::Library {
                headers,
                module_kw,
                name,
                sig_annot,
                eq,
                struct_kw,
                binds,
                end_kw,
                eoi,
            } => {
                self.headers(headers);
                // A 0.1 library IS this one binding, so `sig`'s decls and
                // `struct`'s binds are the file's two indented blocks. Both
                // keywords stay at the file's own depth and their items go one
                // step in — the shape every file in `dist-v01/packages` writes.
                //
                // `group_open`/`group_close` rather than `enter`/`exit`,
                // because `end` is a closing delimiter in every sense that
                // matters here: it belongs at the indentation of the line its
                // opener sat on. The two agree whenever `struct` began its own
                // line and differ when it did not, which in 0.1 is ordinary —
                // `module Make : (Key : S) -> sig` is one such line.
                // The file's one binding starts a line of its own, and the
                // request also carries the blank line the corpus writes
                // between the headers and it. A `use` header — unlike a legacy
                // `@require:` — does NOT swallow a terminator, so without this
                // `use package open Stdlib module M = struct` came out on one
                // line. `fmt_cli.rs`'s `a_clean_v01_file_exits_zero_without_a_
                // lang_flag` is what caught it.
                self.block_break();
                self.leaf(module_kw);
                self.leaf(name);
                if let Some(s) = sig_annot {
                    self.sig_annot(s);
                }
                self.leaf(eq);
                self.leaf(struct_kw);
                self.enter();
                for b in binds {
                    self.block_break();
                    self.bind(b);
                }
                self.exit();
                self.block_break();
                self.leaf(end_kw);
                self.leaf(eoi);
            }
        }
    }

    fn headers(&mut self, headers: &[v1::HeaderV1]) {
        for h in headers {
            // A `use` header is an ordinary token sequence and needs the
            // terminator; a legacy `@require:` swallowed its own
            // (`lexer.rs:915-933`), so the request emits none there and
            // carries only the author's blank lines.
            self.block_break();
            match h {
                v1::HeaderV1::UsePackage {
                    use_kw,
                    package_kw,
                    open_kw,
                    path,
                } => {
                    self.leaf(use_kw);
                    self.leaf(package_kw);
                    if let Some(k) = open_kw {
                        self.leaf(k);
                    }
                    self.mod_chain(path);
                }
                v1::HeaderV1::UseOf {
                    use_kw,
                    open_kw,
                    path,
                    of_kw,
                    relpath,
                } => {
                    self.leaf(use_kw);
                    if let Some(k) = open_kw {
                        self.leaf(k);
                    }
                    self.mod_chain(path);
                    self.leaf(of_kw);
                    self.leaf(relpath);
                }
                v1::HeaderV1::Use {
                    use_kw,
                    open_kw,
                    path,
                } => {
                    self.leaf(use_kw);
                    if let Some(k) = open_kw {
                        self.leaf(k);
                    }
                    self.mod_chain(path);
                }
                // `lex_header` swallows the line's terminator INTO the token
                // (`lexer.rs:915-933`), so this leaf's own bytes end the line and
                // `emit_atom` sets `at_line_start` from them. Nothing here may
                // add a terminator of its own.
                v1::HeaderV1::Legacy(Header::Require(t)) => self.leaf(t),
                v1::HeaderV1::Legacy(Header::Import(t)) => self.leaf(t),
                v1::HeaderV1::Legacy(Header::Stage(t)) => self.leaf(t),
            }
        }
    }

    fn mod_chain(&mut self, c: &ast::ModChainV1) {
        match c {
            ast::ModChainV1::Long(t) => self.leaf(t),
            ast::ModChainV1::Single(t) => self.leaf(t),
        }
    }

    // -- binds ---------------------------------------------------------------

    /// The shape every `val`-family bind shares: the keyword sits at the
    /// construct's own depth and *everything after it* — the stage prefix, the
    /// parameters and the body — is one step in.
    ///
    /// One `Nest` rather than two (a header nest plus a body nest) is
    /// deliberate: a nest per field would compound to two steps for every
    /// binding in the corpus.
    fn bind(&mut self, b: &v1::Bind) {
        use v1::Bind as B;
        match b {
            B::Value {
                kw,
                stage,
                name,
                params,
                eq,
                body,
            } => {
                self.leaf(kw);
                self.enter();
                self.bind_stage(stage.as_ref());
                self.bind_name(name);
                self.params(params);
                self.leaf(eq);
                self.exit();
                self.body(body);
            }
            B::ValueInline {
                kw,
                stage,
                inline_kw,
                ctx,
                cmd,
                params,
                eq,
                body,
            } => {
                self.leaf(kw);
                self.enter();
                self.bind_stage(stage.as_ref());
                self.leaf(inline_kw);
                if let Some(c) = ctx {
                    self.leaf(c);
                }
                self.horz_cmd(cmd);
                self.params(params);
                self.leaf(eq);
                self.exit();
                self.body(body);
            }
            B::ValueBlock {
                kw,
                stage,
                block_kw,
                ctx,
                cmd,
                params,
                eq,
                body,
            } => {
                self.leaf(kw);
                self.enter();
                self.bind_stage(stage.as_ref());
                self.leaf(block_kw);
                if let Some(c) = ctx {
                    self.leaf(c);
                }
                self.vert_cmd(cmd);
                self.params(params);
                self.leaf(eq);
                self.exit();
                self.body(body);
            }
            B::ValueMath {
                kw,
                stage,
                math_kw,
                ctx,
                cmd,
                params,
                scripts,
                eq,
                body,
            } => {
                self.leaf(kw);
                self.enter();
                self.bind_stage(stage.as_ref());
                self.leaf(math_kw);
                self.leaf(ctx);
                self.horz_cmd(cmd);
                self.params(params);
                if let Some(s) = scripts {
                    self.leaf(&s.with_kw);
                    self.leaf(&s.sub);
                    self.leaf(&s.sup);
                }
                self.leaf(eq);
                self.exit();
                self.body(body);
            }
            B::ValueRec {
                kw,
                stage,
                rec_kw,
                first,
                ands,
            } => {
                self.leaf(kw);
                self.enter();
                self.bind_stage(stage.as_ref());
                self.leaf(rec_kw);
                self.rec_clause(first);
                self.exit();
                for a in ands {
                    self.leaf(&a.and_kw);
                    self.enter();
                    self.rec_clause(&a.clause);
                    self.exit();
                }
            }
            B::ValueMutable {
                kw,
                stage,
                mutable_kw,
                name,
                arrow,
                value,
            } => {
                self.leaf(kw);
                self.enter();
                self.bind_stage(stage.as_ref());
                self.leaf(mutable_kw);
                self.leaf(name);
                self.leaf(arrow);
                self.exit();
                self.body(value);
            }
            B::Type { kw, first, ands } => {
                self.leaf(kw);
                self.type_bind(first);
                for a in ands {
                    self.leaf(&a.and_kw);
                    self.type_bind(&a.bind);
                }
            }
            B::Module {
                module_kw,
                name,
                sig_annot,
                eq,
                body,
            } => {
                self.leaf(module_kw);
                self.leaf(name);
                if let Some(s) = sig_annot {
                    self.sig_annot(s);
                }
                self.leaf(eq);
                self.mod_expr(body);
            }
            B::Signature { kw, name, eq, sig_ } => {
                self.leaf(kw);
                self.leaf(name);
                self.leaf(eq);
                self.sig_expr(sig_);
            }
            B::Include { kw, body } => {
                self.leaf(kw);
                self.mod_expr(body);
            }
        }
    }

    /// `~` / `persistent ~` before a bound name.
    ///
    /// Never a break and never a space after the sigil: the sigil binds to the
    /// name (`engine.md` section 9). Slice 1 could not insert one anyway — it
    /// only ever copies the gap that is there — and it opens no frame here, so
    /// it cannot break one either.
    fn bind_stage(&mut self, s: Option<&v1::BindStageV1>) {
        if let Some(s) = s {
            if let Some(p) = &s.persistent {
                self.leaf(p);
            }
            self.leaf(&s.tilde);
        }
    }

    fn rec_clause(&mut self, c: &ast::RecClauseV1) {
        self.bind_name(&c.name);
        self.params(&c.params);
        self.leaf(&c.eq);
        self.body(&c.value);
    }

    /// `name tyvars = body`, framing itself.
    ///
    /// The header (`name`, its type variables, the `=`) is one nest step and
    /// the BODY is a [`Build::open_body`] group of its own, exactly as a
    /// value binding's is — so a record type that does not fit puts its fields
    /// one step past the declaration and its `|)` back at the declaration's
    /// column, rather than two and one. Both callers therefore open no frame
    /// of their own.
    fn type_bind(&mut self, t: &v1::TypeBindSingleV1) {
        self.enter();
        self.leaf(&t.name);
        for v in &t.tyvars {
            self.leaf(v);
        }
        self.leaf(&t.eq);
        self.exit();
        // A bare record or parenthesised type absorbs its own break; anything
        // else gets the group.
        let hug = self.breaks.bodies && hugs_type_body(&t.body);
        match hug {
            true => {}
            false => self.open_body(),
        }
        match &t.body {
            v1::TypeBodyV1::Variant {
                leading_bar,
                first,
                rest,
            } => {
                // A variant's constructors are a clause list, exactly as a
                // `match`'s arms are: one `| Ctor of ty` per line.
                self.clause_break();
                if let Some(bar) = leading_bar {
                    self.leaf(bar);
                }
                self.variant_def(first);
                for r in rest {
                    self.clause_break();
                    self.leaf(&r.bar);
                    self.variant_def(&r.def);
                }
            }
            v1::TypeBodyV1::Synonym(ty) => self.type_expr(ty),
        }
        if !hug {
            self.close_body();
        }
    }

    fn variant_def(&mut self, v: &v1::VariantDefV1) {
        self.leaf(&v.ctor);
        if let Some(of) = &v.of_ty {
            self.leaf(&of.of_kw);
            self.type_expr(&of.ty);
        }
    }

    fn type_binds(&mut self, t: &v1::TypeBindsV1) {
        self.type_bind(&t.first);
        for a in &t.ands {
            self.leaf(&a.and_kw);
            self.type_bind(&a.bind);
        }
    }

    // -- modules and signatures ----------------------------------------------

    /// `:> S` — 0.1's annotation sigil, never 0.0.6's `: sig … end`.
    fn sig_annot(&mut self, s: &v1::SigAnnotV1) {
        self.leaf(&s.coerce);
        self.sig_expr(&s.sig_);
    }

    fn mod_expr(&mut self, m: &ast::ModExpr) {
        use ast::ModExpr as M;
        match m {
            M::Functor {
                fun_kw,
                lp,
                param,
                colon,
                dom,
                rp,
                arrow,
                body,
            } => {
                self.leaf(fun_kw);
                let anchor = self.group_open(lp);
                self.leaf(param);
                self.leaf(colon);
                self.sig_expr(dom);
                self.group_close(anchor, rp);
                self.leaf(arrow);
                // The functor's result is a body in exactly the `->` sense: one
                // step past the line the arrow sits on when it starts its own.
                self.open_body();
                self.mod_expr(body);
                self.close_body();
            }
            M::Coerce { name, coerce, sig_ } => {
                self.leaf(name);
                self.leaf(coerce);
                self.sig_expr(sig_);
            }
            M::App { func, arg } => {
                self.mod_chain(func);
                self.mod_chain(arg);
            }
            M::Var(c) => self.mod_chain(c),
            M::Struct {
                struct_kw,
                binds,
                end_kw,
            } => {
                self.leaf(struct_kw);
                self.enter();
                for b in binds {
                    self.block_break();
                    self.bind(&b.0);
                }
                self.exit();
                self.block_break();
                self.leaf(end_kw);
            }
        }
    }

    fn sig_expr(&mut self, s: &ast::SigExpr) {
        use ast::SigExpr as S;
        match s {
            S::Functor {
                lp,
                param,
                colon,
                dom,
                rp,
                arrow,
                cod,
            } => {
                let anchor = self.group_open(lp);
                self.leaf(param);
                self.leaf(colon);
                self.sig_expr(dom);
                self.group_close(anchor, rp);
                self.leaf(arrow);
                self.open_body();
                self.sig_expr(cod);
                self.close_body();
            }
            S::WithType {
                base,
                with_kw,
                path,
                type_kw,
                binds,
            } => {
                self.sig_bot(base);
                self.leaf(with_kw);
                if let Some(p) = path {
                    self.mod_chain(p);
                }
                self.leaf(type_kw);
                self.type_binds(binds);
            }
            S::Bot(b) => self.sig_bot(b),
        }
    }

    fn sig_bot(&mut self, s: &ast::SigBotV1) {
        use ast::SigBotV1 as S;
        match s {
            S::Path(t) => self.leaf(t),
            S::Var(t) => self.leaf(t),
            S::Sig {
                sig_kw,
                decls,
                end_kw,
            } => {
                self.leaf(sig_kw);
                self.enter();
                for d in decls {
                    self.block_break();
                    self.decl(&d.0);
                }
                self.exit();
                self.block_break();
                self.leaf(end_kw);
            }
        }
    }

    fn decl(&mut self, d: &ast::Decl) {
        use ast::Decl as D;
        match d {
            D::Val {
                kw,
                stage,
                name,
                quant,
                colon,
                ty,
            } => {
                self.leaf(kw);
                self.enter();
                self.bind_stage(stage.as_ref());
                self.bind_name(name);
                for q in quant {
                    self.leaf(q);
                }
                self.leaf(colon);
                self.exit();
                self.type_expr(ty);
            }
            D::ValHorzCmd {
                kw,
                cmd,
                quant,
                colon,
                ty,
            } => {
                self.leaf(kw);
                self.enter();
                self.leaf(cmd);
                for q in quant {
                    self.leaf(q);
                }
                self.leaf(colon);
                self.exit();
                self.type_expr(ty);
            }
            D::ValVertCmd {
                kw,
                cmd,
                quant,
                colon,
                ty,
            } => {
                self.leaf(kw);
                self.enter();
                self.leaf(cmd);
                for q in quant {
                    self.leaf(q);
                }
                self.leaf(colon);
                self.exit();
                self.type_expr(ty);
            }
            D::TypeOpaque {
                kw,
                name,
                cons,
                kind,
            } => {
                self.leaf(kw);
                self.enter();
                self.leaf(name);
                self.leaf(cons);
                self.leaf(&kind.first);
                for r in &kind.rest {
                    self.leaf(&r.arrow);
                    self.leaf(&r.base);
                }
                self.exit();
            }
            D::Type { kw, binds } => {
                self.leaf(kw);
                self.enter();
                self.type_binds(binds);
                self.exit();
            }
            D::Module {
                kw,
                name,
                colon,
                sig_,
            } => {
                self.leaf(kw);
                self.leaf(name);
                self.leaf(colon);
                self.sig_expr(sig_);
            }
            D::Signature { kw, name, eq, sig_ } => {
                self.leaf(kw);
                self.leaf(name);
                self.leaf(eq);
                self.sig_expr(sig_);
            }
            D::Include { kw, sig_ } => {
                self.leaf(kw);
                self.sig_expr(sig_);
            }
        }
    }

    // -- types ---------------------------------------------------------------

    /// A type expression.
    ///
    /// Slice 1 makes no layout *decisions* here — an arrow chain is one flat
    /// sequence — but it walks the structure rather than treating a type as an
    /// opaque atom run, for one reason: the record type `(| … |)` and the
    /// command types `inline [ … ]`. Those are the constructs the corpus writes
    /// across lines *inside a type*, and without [`Build::group_close`] their
    /// closer lands a step too deep.
    fn type_expr(&mut self, t: &ast::TypeExpr) {
        use ast::TypeExpr as T;
        // One `Mode::Auto` group over the OUTERMOST arrow chain, and only
        // there — see [`super::build006::Build::type_expr`].
        let chain = self.breaks.type_arrows
            && !self.in_type_chain
            && matches!(t, T::Fun { .. } | T::OptRowFun { .. });
        if chain {
            self.in_type_chain = true;
            self.push_group(Mode::Auto);
            self.frames.push(Frame::nest(1));
        }
        self.type_expr_inner(t);
        if chain {
            self.exit();
            self.exit();
            self.in_type_chain = false;
        }
    }

    fn type_expr_inner(&mut self, t: &ast::TypeExpr) {
        use ast::TypeExpr as T;
        match t {
            T::OptRowFun {
                opt_dom,
                dom,
                arrow,
                cod,
            } => {
                self.leaf(&opt_dom.q);
                let anchor = self.group_open(&opt_dom.paren.open);
                for (i, e) in opt_dom.inner.entries.iter().enumerate() {
                    self.item_break(i);
                    self.leaf(&e.label);
                    self.leaf(&e.colon);
                    self.type_expr(&e.ty);
                    if let Some(c) = &e.comma {
                        self.leaf(c);
                    }
                }
                if let Some(r) = &opt_dom.inner.row_tail {
                    self.leaf(&r.bar);
                    self.leaf(&r.var);
                }
                self.group_close(anchor, &opt_dom.paren.close);
                self.type_prod(dom);
                if self.breaks.type_arrows {
                    self.br(Br::Opportunity);
                }
                self.leaf(arrow);
                self.type_expr(cod);
            }
            T::Fun { dom, arrow, cod } => {
                self.type_prod(dom);
                if self.breaks.type_arrows {
                    self.br(Br::Opportunity);
                }
                self.leaf(arrow);
                // No nest: `a -> b -> c` is one sequence, and the CST's
                // right-nesting of it is not indentation the corpus writes.
                self.type_expr(cod);
            }
            T::Atom(p) => self.type_prod(p),
        }
    }

    fn type_prod(&mut self, p: &ast::TypeProd) {
        self.type_app(&p.first);
        for r in &p.rest {
            self.leaf(&r.star);
            self.type_app(&r.ty);
        }
    }

    fn type_app(&mut self, a: &ast::TypeApp) {
        use ast::TypeApp as A;
        match a {
            A::InlineCmdTy { kw, ilist, args } => {
                self.leaf(kw);
                let anchor = self.group_open(&ilist.open);
                self.cmd_arg_items(args);
                self.group_close(anchor, &ilist.close);
            }
            A::BlockCmdTy { kw, blist, args } => {
                self.leaf(kw);
                let anchor = self.group_open(&blist.open);
                self.cmd_arg_items(args);
                self.group_close(anchor, &blist.close);
            }
            A::MathCmdTy { kw, mlist, args } => {
                self.leaf(kw);
                let anchor = self.group_open(&mlist.open);
                self.cmd_arg_items(args);
                self.group_close(anchor, &mlist.close);
            }
            A::AppliedLong { ctor, first, rest } => {
                self.leaf(ctor);
                self.type_atom(first);
                for r in rest {
                    self.type_atom(r);
                }
            }
            A::Applied { ctor, first, rest } => {
                self.leaf(ctor);
                self.type_atom(first);
                for r in rest {
                    self.type_atom(r);
                }
            }
            A::Atom(t) => self.type_atom(t),
        }
    }

    fn cmd_arg_items(&mut self, args: &[ast::TypeCmdArgItemV1]) {
        for it in args {
            if let Some(o) = &it.opts {
                self.leaf(&o.q);
                let anchor = self.group_open(&o.paren.open);
                for (i, e) in o.entries.iter().enumerate() {
                    self.item_break(i);
                    self.leaf(&e.label);
                    self.leaf(&e.colon);
                    self.type_expr(&e.ty);
                    if let Some(c) = &e.comma {
                        self.leaf(c);
                    }
                }
                self.group_close(anchor, &o.paren.close);
            }
            self.type_expr(&it.ty);
            if let Some(c) = &it.comma {
                self.leaf(c);
            }
        }
    }

    fn type_atom(&mut self, a: &ast::TypeAtom) {
        use ast::TypeAtom as A;
        match a {
            A::Paren { paren, inner } => {
                let anchor = self.group_open(&paren.open);
                self.type_expr(inner);
                self.group_close(anchor, &paren.close);
            }
            A::Record { rec, inner } => {
                let anchor = self.group_open(&rec.open);
                for (i, f) in inner.fields.iter().enumerate() {
                    self.item_break(i);
                    self.leaf(&f.name);
                    self.leaf(&f.colon);
                    self.type_expr(&f.ty);
                    if let Some(c) = &f.comma {
                        self.leaf(c);
                    }
                }
                if let Some(r) = &inner.row_tail {
                    self.leaf(&r.bar);
                    self.leaf(&r.var);
                }
                self.group_close(anchor, &rec.close);
            }
            A::Var(t) => self.leaf(t),
            A::LongName(t) => self.leaf(t),
            A::Name(t) => self.leaf(t),
        }
    }

    // -- expressions ---------------------------------------------------------

    /// The `let … in` spine, walked **iteratively into one flat frame**.
    ///
    /// The CST nests these to the right, one level per binding, but they read as
    /// a statement sequence and the corpus writes them flat. A recursive printer
    /// would indent the eleventh binding twenty-two columns in. The loop below
    /// is the fix, and the *absence* of an `enter()` around the continuation is
    /// the statement of intent — there is nothing to check in a recursive
    /// version that "happens to pass the same depth".
    fn expr(&mut self, e: &ast::Expr) {
        use ast::Expr as E;
        // One `Mode::Auto` group over the WHOLE spine, so consecutive
        // `let … in` break together or not at all. See
        // [`super::build006::Build::expr`].
        let spine = self.breaks.let_spine && is_spine(e);
        if spine {
            self.push_group(Mode::Auto);
        }
        let mut cur = e;
        loop {
            match cur {
                E::LetIn {
                    kw,
                    name,
                    params,
                    eq,
                    value,
                    in_kw,
                    body,
                } => {
                    self.leaf(kw);
                    self.enter();
                    self.bind_name(name);
                    self.params(params);
                    self.leaf(eq);
                    self.exit();
                    self.body(value);
                    self.leaf(in_kw);
                    if spine {
                        self.br(Br::Opportunity);
                    }
                    cur = body;
                }
                E::LetRecIn {
                    let_kw,
                    rec_kw,
                    first,
                    ands,
                    in_kw,
                    body,
                } => {
                    self.leaf(let_kw);
                    self.enter();
                    self.leaf(rec_kw);
                    self.rec_clause(first);
                    self.exit();
                    for a in ands {
                        self.leaf(&a.and_kw);
                        self.enter();
                        self.rec_clause(&a.clause);
                        self.exit();
                    }
                    self.leaf(in_kw);
                    if spine {
                        self.br(Br::Opportunity);
                    }
                    cur = body;
                }
                E::LetMutableIn {
                    let_kw,
                    mutable_kw,
                    name,
                    arrow,
                    init,
                    in_kw,
                    body,
                } => {
                    self.leaf(let_kw);
                    self.enter();
                    self.leaf(mutable_kw);
                    self.leaf(name);
                    self.leaf(arrow);
                    self.exit();
                    self.body(init);
                    self.leaf(in_kw);
                    if spine {
                        self.br(Br::Opportunity);
                    }
                    cur = body;
                }
                E::LetPatternIn {
                    kw,
                    pat,
                    eq,
                    value,
                    in_kw,
                    body,
                } => {
                    self.leaf(kw);
                    self.enter();
                    self.flat(pat);
                    self.leaf(eq);
                    self.exit();
                    self.body(value);
                    self.leaf(in_kw);
                    if spine {
                        self.br(Br::Opportunity);
                    }
                    cur = body;
                }
                E::OpenIn {
                    let_kw,
                    open_kw,
                    name,
                    in_kw,
                    body,
                } => {
                    self.leaf(let_kw);
                    self.leaf(open_kw);
                    self.leaf(name);
                    self.leaf(in_kw);
                    if spine {
                        self.br(Br::Opportunity);
                    }
                    cur = body;
                }
                other => {
                    self.expr_leaf(other);
                    break;
                }
            }
        }
        if spine {
            self.exit();
        }
    }

    /// Everything that is not a `… in …` spine node.
    fn expr_leaf(&mut self, e: &ast::Expr) {
        use ast::Expr as E;
        match e {
            E::If {
                kw,
                cond,
                then_kw,
                then_branch,
                else_kw,
                else_branch,
            } => {
                self.leaf(kw);
                self.enter();
                self.expr(cond);
                self.exit();
                // `then` and `else` are at the `if`'s own depth, and a branch
                // body that starts its own line is one step past the line its
                // keyword is on — SYMMETRICALLY, which is [`Build::open_body`]'s
                // whole point.
                // **One group over BOTH branches**, so they break together:
                // two independent bodies left the `else` arm trailing the
                // `then` arm's last line. See
                // [`super::build006::Build::expr_leaf`].
                match self.breaks.bodies {
                    true => {
                        self.push_group(Mode::Auto);
                        self.leaf(then_kw);
                        self.frames.push(Frame::nest(1));
                        self.br(Br::Opportunity);
                        self.expr(then_branch);
                        self.exit();
                        self.br(Br::Opportunity);
                        self.leaf(else_kw);
                        self.frames.push(Frame::nest(1));
                        self.br(Br::Opportunity);
                        self.expr(else_branch);
                        self.exit();
                        self.exit();
                    }
                    false => {
                        self.leaf(then_kw);
                        self.body(then_branch);
                        self.leaf(else_kw);
                        self.body(else_branch);
                    }
                }
            }
            E::Fun {
                kw,
                params,
                arrow,
                body,
            } => {
                self.leaf(kw);
                self.enter();
                self.params(params);
                self.leaf(arrow);
                self.exit();
                self.body(body);
            }
            E::Match {
                kw,
                scrutinee,
                with_kw,
                leading_bar,
                first,
                rest,
                end_kw,
            } => {
                self.leaf(kw);
                self.enter();
                self.expr(scrutinee);
                self.exit();
                self.leaf(with_kw);
                // Arms at the `match`'s own depth, and 0.1's mandatory `end`
                // with them: the corpus writes the leading `|` and the `end` in
                // the `match` keyword's column.
                self.clause_break();
                if let Some(bar) = leading_bar {
                    self.leaf(bar);
                }
                self.match_arm(first);
                for r in rest {
                    self.clause_break();
                    self.leaf(&r.bar);
                    self.match_arm(&r.arm);
                }
                self.clause_break();
                self.leaf(end_kw);
            }
            E::Overwrite { name, arrow, value } => {
                self.leaf(name);
                self.leaf(arrow);
                self.body(value);
            }
            E::Ops(chain) => self.op_chain(chain),
            // The spine nodes, reached only if `expr` is bypassed.
            other => self.expr(other),
        }
    }

    fn match_arm(&mut self, a: &ast::MatchArm) {
        self.flat(&a.pat);
        self.leaf(&a.arrow);
        self.body(&a.body);
    }

    /// `head (op rhs)*`, with the tail one step in.
    ///
    /// All precedence levels are flattened in this grammar, so the formatter
    /// cannot know precedence and must not indent by it. One depth for the whole
    /// tail is the only rule available, and it is also the corpus idiom — a
    /// `|>` pipeline breaks before the operator at one indentation. 0.1 has no
    /// `before` postfix, so unlike 0.0.6 the tail is the only thing here.
    /// `head (op rhs)*` — one [`Mode::Auto`] group, **all of it or none of
    /// it**, breaking before the operator.
    ///
    /// The head is outside the group deliberately, and it costs nothing: the
    /// renderer resolves a group against the column it has already reached.
    /// See [`super::build006::Build::op_chain`] for the precedence argument.
    fn op_chain(&mut self, c: &ast::OpChain) {
        self.app_expr(&c.head);
        if c.tail.is_empty() {
            return;
        }
        let grouped = self.breaks.op_chain;
        if grouped {
            self.push_group(Mode::Auto);
        }
        self.enter();
        for r in &c.tail {
            if grouped {
                self.br(Br::Opportunity);
            }
            self.leaf(&r.op);
            self.app_expr(&r.rhs);
        }
        self.exit();
        if grouped {
            self.exit();
        }
    }

    fn app_expr(&mut self, a: &ast::AppExpr) {
        if let Some(m) = &a.minus {
            self.leaf(m);
            // A UNARY minus, left as whichever of `(-1)` and `(- 1)` the
            // author wrote, but not as `(-   1)`. Structural, because
            // `Token::ExactMinus` is subtraction too and only the tree can
            // tell them apart — see [`super::build006::Build::app_expr`],
            // which also carries why this is neither `Tight` nor `Keep`.
            if self.rules.minus {
                self.space(Space::Collapse);
            }
        }
        self.stage_prefix(a.stage.as_ref());
        if let Some(e) = &a.excl {
            self.leaf(e);
        }
        self.atomic(&a.head);
        for s in &a.head_accesses {
            self.leaf(&s.hash);
            self.leaf(&s.label);
        }
        self.app_args(&a.args);
    }

    /// The argument run, one frame — the shape `format.rs:96-110` uses against a
    /// bracket counter: an argument line indented one step past its function
    /// with no bracket opened in between.
    fn app_args(&mut self, args: &[ast::AppArg]) {
        if args.is_empty() {
            return;
        }
        let hug = self.breaks.app_args && hugs_last(args);
        let split = match hug {
            true => args.len() - 1,
            false => args.len(),
        };
        if split > 0 {
            self.enter();
            for a in &args[..split] {
                if self.breaks.app_args {
                    self.br(Br::Fill);
                }
                self.app_arg(a);
            }
            self.exit();
        }
        if hug {
            if let Some(a) = args.last() {
                self.app_arg(a);
            }
        }
    }

    fn app_arg(&mut self, a: &ast::AppArg) {
        use ast::AppArg as A;
        match a {
            A::Bundled {
                opts,
                excl,
                atom,
                accesses,
            } => {
                self.opt_args(opts);
                if let Some(e) = excl {
                    self.leaf(e);
                }
                self.atomic(atom);
                for s in accesses {
                    self.leaf(&s.hash);
                    self.leaf(&s.label);
                }
            }
            A::BundledCtor { opts, ctor } => {
                self.opt_args(opts);
                self.leaf(ctor);
            }
            A::Atom {
                stage,
                excl,
                atom,
                accesses,
            } => {
                self.stage_prefix(stage.as_ref());
                if let Some(e) = excl {
                    self.leaf(e);
                }
                self.atomic(atom);
                for s in accesses {
                    self.leaf(&s.hash);
                    self.leaf(&s.label);
                }
            }
            A::Ctor(t) => self.leaf(t),
        }
    }

    /// `&e` / `~e` on an operand.
    ///
    /// Never a break and never a space after the sigil: the sigil binds to its
    /// operand. No frame is opened here and no `Line` is emitted, so slice 1
    /// cannot put a break between the two even in principle; the `&` `&` fusion
    /// hazard belongs to [`super::sep::must_separate`] and to the first slice
    /// that writes a gap.
    fn stage_prefix(&mut self, s: Option<&ast::StagePrefix>) {
        match s {
            Some(ast::StagePrefix::Next(t)) => self.leaf(t),
            Some(ast::StagePrefix::Prev(t)) => self.leaf(t),
            None => {}
        }
    }

    fn atomic(&mut self, a: &ast::Atomic) {
        use ast::Atomic as A;
        match a {
            A::Length(t) => self.leaf(t),
            A::Float(t) => self.leaf(t),
            A::Int(t) => self.leaf(t),
            A::Literal(t) => self.leaf(t),
            A::True(t) => self.leaf(t),
            A::False(t) => self.leaf(t),
            A::Ctor(t) => self.leaf(t),
            A::Var(t) => self.leaf(t),
            A::VarWithMod(t) => self.leaf(t),
            A::Command { kw, name } => {
                self.leaf(kw);
                self.horz_cmd(name);
            }
            A::Unit { paren } => {
                self.leaf(&paren.open);
                self.leaf(&paren.close);
            }
            A::Paren { paren, inner } => {
                let anchor = self.group_open(&paren.open);
                self.paren_body(inner);
                self.group_close(anchor, &paren.close);
            }
            A::Record { rec, body } => {
                let anchor = self.group_open(&rec.open);
                self.record_body(body);
                self.group_close(anchor, &rec.close);
            }
            A::List { list, items } => {
                let anchor = self.group_open(&list.open);
                for (n, i) in items.iter().enumerate() {
                    self.item_break(n);
                    self.element(&i.value);
                    if let Some(c) = &i.comma {
                        self.leaf(c);
                    }
                }
                self.group_close(anchor, &list.close);
            }
            // The area boundary — slice 4's three flags. An area whose flag
            // is off is one byte range, interior untouched.
            A::InlineText { igrp, elems } if self.areas.inline => self.inline_text(igrp, elems),
            A::BlockText { bgrp, elems } if self.areas.block => self.block_text(bgrp, elems),
            A::MathText { mgrp, elems } if self.areas.math => self.math_text(mgrp, elems),
            A::InlineText { .. } | A::BlockText { .. } | A::MathText { .. } => self.verbatim(a),
        }
    }

    fn paren_body(&mut self, b: &ast::ParenBody) {
        self.expr(&b.first);
        for c in &b.rest {
            self.leaf(&c.comma);
            self.item_break(1);
            self.element(&c.value);
        }
    }

    fn record_body(&mut self, b: &ast::RecordBody) {
        match b {
            ast::RecordBody::Update {
                base,
                with_kw,
                fields,
            } => {
                self.expr(base);
                self.leaf(with_kw);
                for (i, f) in fields.iter().enumerate() {
                    self.item_break(i);
                    self.record_field(f);
                }
            }
            ast::RecordBody::Fields(fields) => {
                for (i, f) in fields.iter().enumerate() {
                    self.item_break(i);
                    self.record_field(f);
                }
            }
        }
    }

    /// `label = value,` inside a record literal.
    ///
    /// The value is a [`Build::body`] — it is introduced by the `=` already
    /// emitted on this line — so a field written across two lines indents its
    /// value one step past the label's line rather than wherever the chain
    /// happens to sum to.
    fn record_field(&mut self, f: &ast::RecordField) {
        self.leaf(&f.name);
        self.leaf(&f.eq);
        self.body(&f.value);
        if let Some(c) = &f.comma {
            self.leaf(c);
        }
    }

    // -- the three text areas (slice 4) --------------------------------------

    /// `'< … >`, laid out. Block text's whitespace is *gaps*
    /// ([`Areas::block`]), so nothing here is a new mechanism.
    fn block_text(&mut self, grp: &BlockGroup<()>, elems: &[ast::BlockElem]) {
        let anchor = self.group_open(&grp.open);
        let outer = self.enter_area(Area::Block);
        for (i, e) in elems.iter().enumerate() {
            if i > 0 {
                self.block_item_break();
            }
            self.block_elem(e);
        }
        self.group_close(anchor, &grp.close);
        self.area = outer;
    }

    /// The gap between two `'< … >` items. The 0.0.6 twin, rule for rule:
    /// see [`super::build006::Build::block_item_break`] and
    /// [`Breaks::block_blanks`]. The grammar of a block area is the same in
    /// both generations — a `+cmd …;` / `#var;` sequence whose whitespace is
    /// gaps — so a fork here would be a difference with no cause.
    fn block_item_break(&mut self) {
        if !self.breaks.block_items {
            return;
        }
        let blank = self.breaks.block_blanks
            && self.max_blank_lines > 0
            && blank_line_in_gap(self.source, self.atoms, self.cursor, self.byte);
        match blank {
            true => self.br(Br::Hard),
            false => self.br(Br::Opportunity),
        }
    }

    fn block_elem(&mut self, e: &ast::BlockElem) {
        match e {
            ast::BlockElem::Embed { var, semi } => {
                self.leaf(var);
                let outer = self.enter_area(Area::Active);
                self.leaf(semi);
                self.area = outer;
            }
            ast::BlockElem::Cmd { name, tail } => {
                self.vert_cmd(name);
                self.cmd_tail(tail);
            }
        }
    }

    /// `{ … }`, re-indented. Both delimiters swallow trivia, so both go
    /// through [`Subst::Swallow`].
    fn inline_text(&mut self, grp: &InlineGroup<()>, elems: &[ast::InlineElem]) {
        self.area_open_as(&grp.open, Subst::SwallowOpen);
        self.inline_depth += 1;
        let outer = self.enter_area(Area::Inline);
        for e in elems {
            self.inline_elem(e);
        }
        self.inline_depth -= 1;
        self.area_close_as(&grp.close, Subst::Swallow);
        self.area = outer;
    }

    fn inline_elem(&mut self, e: &ast::InlineElem) {
        use ast::InlineElem as I;
        match e {
            I::Char(t) => self.leaf(t),
            I::CodeText(t) => self.leaf(t),
            // The whole point of the area: one whitespace run, one token.
            // Slice 4 recomputes its indentation; slice 6 may re-spell it
            // entirely. See [`Build::inline_gap`].
            I::Space(t) => self.inline_gap(t),
            I::Break(t) => self.inline_gap(t),
            // `*`+ and `|` swallow the run in front of them too.
            I::ItemBullet(t) => self.leaf_as(t, Subst::Swallow),
            I::Sep(t) => self.leaf_as(t, Subst::Swallow),
            I::Embed { var, semi } => {
                self.leaf(var);
                let outer = self.enter_area(Area::Active);
                self.leaf(semi);
                self.area = outer;
            }
            I::EmbedMath { mgrp, elems } => match self.areas.math {
                true => self.math_text(mgrp, elems),
                false => self.verbatim_elems(elems),
            },
            I::Cmd { name, tail } => {
                self.horz_cmd(name);
                self.cmd_tail(tail);
            }
        }
    }

    /// A command's argument run.
    ///
    /// **0.1 delta**: an optional LEADING `?(l = e, …)` bundle, peeled off the
    /// first argument by the grammar; and the arguments themselves are one
    /// application `Expr` rather than a flat `Vec<AppArg>`.
    fn cmd_tail(&mut self, tail: &ast::CmdTail) {
        match tail {
            ast::CmdTail::Semi(t) => {
                let outer = self.enter_area(Area::Active);
                self.leaf(t);
                self.area = outer;
            }
            ast::CmdTail::Args {
                lead_opts,
                args,
                semi,
            } => {
                let outer = self.enter_area(Area::Active);
                // No frame of its own: `+p{ … }`'s inline text is the LAST
                // thing on the line and its own frame is what indents its
                // contents — a frame around the run as well puts them two
                // steps in and leaves the `}` one step in. This is
                // [`super::build006::Build::cmd_tail`]'s hug, and in 0.1 it
                // needs no split because the arguments are one application
                // `Expr` rather than a flat list.
                self.cmd_arg_boundary();
                if let Some(o) = lead_opts {
                    self.opt_args(o);
                }
                // **0.1 delta**: the arguments are one application `Expr`
                // rather than a flat list, so only the FIRST boundary is
                // visible here. The rest are ordinary application argument
                // boundaries and take the default — which is what the 0.1
                // census says they already are (a variable head is 0 tight
                // against 1,076 spaced).
                self.cmd_arg_boundary();
                self.expr(args);
                if let Some(t) = semi {
                    self.leaf(t);
                }
                self.area = outer;
            }
        }
    }

    /// `${ … }`, laid out. Gaps again, exactly like block text.
    /// `${ … }` — re-indented, and its line structure left exactly as it was.
    ///
    /// The 0.1 twin of [`super::build006::Build::math_text`], and the same
    /// decision: math is ATOMIC for line-breaking **in both directions** — no
    /// break is invented inside `${ … }` and none is removed. Those are one
    /// decision rather than two, because a rule that refuses to add breaks but
    /// deletes the author's turns every hand-laid-out equation into a single
    /// long line and leaves no way to lay it out again.
    ///
    /// The `Nest` is what re-indents the lines it keeps.
    fn math_text(&mut self, grp: &MathGroup<()>, elems: &[v1::MathErasedV1]) {
        self.area_open_as(&grp.open, Subst::None);
        let outer = self.enter_area(Area::Math);
        for e in elems {
            self.math_elem(e);
        }
        self.area_close_as(&grp.close, Subst::None);
        self.area = outer;
    }

    fn math_elem(&mut self, m: &ast::MathElemCst) {
        self.math_bot(&m.base);
        for sc in &m.scripts {
            match sc {
                ast::MathScript::Super { hat, group } => {
                    self.leaf(hat);
                    self.math_group_arg(group);
                }
                ast::MathScript::Sub { under, group } => {
                    self.leaf(under);
                    self.math_group_arg(group);
                }
                ast::MathScript::Primes(t) => self.leaf(t),
            }
        }
    }

    fn math_bot(&mut self, m: &ast::MathBot) {
        use ast::MathBot as M;
        match m {
            M::Cmd { name, args } => {
                self.math_cmd(name);
                if !args.is_empty() {
                    self.enter();
                    for a in args {
                        self.cmd_arg_boundary();
                        self.math_arg(a);
                    }
                    self.exit();
                }
            }
            M::Chars(t) => self.leaf(t),
            M::Embed(t) => self.leaf(t),
            M::Sep(t) => self.leaf(t),
            M::Group { mgrp, elems } => self.math_text(mgrp, elems),
        }
    }

    fn math_group_arg(&mut self, g: &ast::MathGroupArg) {
        match g {
            ast::MathGroupArg::Group { mgrp, elems } => self.math_text(mgrp, elems),
            ast::MathGroupArg::Bot(b) => self.math_bot(b),
        }
    }

    /// A math command's argument, including the escapes back into inline text,
    /// block text and program mode. **0.1 delta**: no `?:`/`?*` wrapper —
    /// `MathArg` *is* the body.
    fn math_arg(&mut self, a: &ast::MathArg) {
        use ast::MathArg as B;
        match a {
            B::Math { mgrp, elems } => self.math_text(mgrp, elems),
            B::Inline { igrp, elems } => match self.areas.inline {
                true => self.inline_text(igrp, elems),
                false => self.verbatim_elems(elems),
            },
            B::Block { bgrp, elems } => match self.areas.block {
                true => self.block_text(bgrp, elems),
                false => self.verbatim_elems(elems),
            },
            // `!(`, `![` and `!(|` escape back to PROGRAM mode.
            B::ParenEscape { paren, inner } => {
                let anchor = self.group_open(&paren.open);
                let outer = self.enter_area(Area::Program);
                self.paren_body(inner);
                self.group_close(anchor, &paren.close);
                self.area = outer;
            }
            B::ListEscape { list, items } => {
                let anchor = self.group_open(&list.open);
                let outer = self.enter_area(Area::Program);
                for i in items {
                    self.element(&i.value);
                    if let Some(c) = &i.comma {
                        self.leaf(c);
                    }
                }
                self.group_close(anchor, &list.close);
                self.area = outer;
            }
            B::RecordEscape { rec, body } => {
                let anchor = self.group_open(&rec.open);
                let outer = self.enter_area(Area::Program);
                self.record_body(body);
                self.group_close(anchor, &rec.close);
                self.area = outer;
            }
        }
    }

    fn opt_args(&mut self, o: &ast::OptArgsV1) {
        self.leaf(&o.q);
        let anchor = self.group_open(&o.paren.open);
        for (i, e) in o.entries.iter().enumerate() {
            self.item_break(i);
            self.leaf(&e.label);
            self.leaf(&e.eq);
            self.body(&e.value);
            if let Some(c) = &e.comma {
                self.leaf(c);
            }
        }
        self.group_close(anchor, &o.paren.close);
    }

    fn opt_params(&mut self, o: &ast::OptParamsV1) {
        self.leaf(&o.q);
        let anchor = self.group_open(&o.paren.open);
        for (i, e) in o.entries.iter().enumerate() {
            self.item_break(i);
            self.leaf(&e.label);
            self.leaf(&e.eq);
            self.leaf(&e.var);
            if let Some(c) = &e.comma {
                self.leaf(c);
            }
        }
        self.group_close(anchor, &o.paren.close);
    }

    /// A binding's parameter run, filled exactly as an application's argument
    /// run is. See [`super::build006::Build::params`].
    fn params(&mut self, params: &[ast::Param]) {
        for p in params {
            if self.breaks.app_args {
                self.br(Br::Fill);
            }
            if let Some(o) = &p.opts {
                self.opt_params(o);
            }
            match &p.body {
                ast::ParamBody::Pat(b) => self.flat(b),
                ast::ParamBody::Ascribed { paren, inner } => {
                    let anchor = self.group_open(&paren.open);
                    self.flat(&inner.pat);
                    self.leaf(&inner.colon);
                    self.type_expr(&inner.ty);
                    self.group_close(anchor, &paren.close);
                }
            }
        }
    }

    // -- command-name alternations -------------------------------------------

    /// `AnyHorzCmdTok` and friends are token *alternations*, not generated
    /// leaves, so they carry no `Spanned` of their own — but each arm does, and
    /// matching on it keeps the desync check live where `Build::one`-style
    /// blind emission would silently give it up.
    fn horz_cmd(&mut self, c: &AnyHorzCmdTok) {
        match c {
            AnyHorzCmdTok::Plain(t) => self.leaf(t),
            AnyHorzCmdTok::Mod(t) => self.leaf(t),
        }
    }

    fn vert_cmd(&mut self, c: &AnyVertCmdTok) {
        match c {
            AnyVertCmdTok::Plain(t) => self.leaf(t),
            AnyVertCmdTok::Mod(t) => self.leaf(t),
        }
    }

    fn math_cmd(&mut self, c: &AnyMathCmdTok) {
        match c {
            AnyMathCmdTok::Plain(t) => self.leaf(t),
            AnyMathCmdTok::Mod(t) => self.leaf(t),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The 0.1 half of the gap census — the same table, the same accumulator,
    /// over the 47-file 0.1 corpus. See
    /// [`super::super::build006::tests::the_v006_gap_census`].
    #[test]
    #[ignore = "prints a census over the corpus; run it when an exception changes"]
    fn the_v01_gap_census() {
        let mut c = super::super::build006::Census::default();
        for path in super::super::build006::corpus_files(&["lib-rustyfi/dist-v01/packages"]) {
            let Ok(src) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Ok(atoms) = rustyfi_syntax::lex_with_version(&src, RustyfiVersion::V0_1) else {
                continue;
            };
            let Ok(file) = rustyfi_syntax::cst_v1::parse_file_v1(&src) else {
                continue;
            };
            c.add(&path, &src, &atoms, gap_census(&src, &atoms, &file));
        }
        c.report("0.1");
    }
    use crate::format_cst::{format_cst, sep, CstOptions};
    use rustyfi_syntax::RustyfiVersion;

    /// Format under slice 1's rules, or `None` if the buffer does not lex.
    fn fmt(src: &str) -> Option<String> {
        format_cst(src, RustyfiVersion::V0_1, &CstOptions::default())
    }

    /// [`fmt`] with slice 6 OFF, for the slice-4 claims that are about what
    /// happens to a gap the re-wrap is not allowed to touch.
    fn fmt_no_wrap(src: &str) -> Option<String> {
        format_cst(
            src,
            RustyfiVersion::V0_1,
            &CstOptions { wrap_inline_text: false, ..Default::default() },
        )
    }

    /// [`fmt`] at an explicit column budget, which is what slice 6's
    /// decisions are a function of.
    fn fmt_at(src: &str, max_width: usize) -> Option<String> {
        format_cst(
            src,
            RustyfiVersion::V0_1,
            &CstOptions { max_width, ..Default::default() },
        )
    }

    fn parsed(src: &str) -> (Vec<Atom>, v1::FileV1) {
        let atoms =
            rustyfi_syntax::lex_with_version(src, RustyfiVersion::V0_1).expect("the source lexes");
        let file = rustyfi_syntax::cst_v1::parse_file_v1(src).expect("the source parses");
        (atoms, file)
    }

    /// A library file, since that is what 46 of the 47 corpus files are.
    fn lib(body: &str) -> String {
        format!("module M = struct\n{body}end\n")
    }

    /// **`LineBreaks::Preserve` is gone here too**, enum and all — see
    /// [`super::build006`]'s twin of this test. The property that would have
    /// been lost with it: with EVERY construct's break flag off, the author's
    /// line breaks still do not survive. `NO_BREAKS` is not `Preserve`.
    #[test]
    fn with_every_construct_off_the_authors_breaks_still_do_not_survive() {
        let src = "module M = struct\n  val x =\n    1\nend\n";
        let (atoms, file) = parsed(src);
        let no_breaks = super::super::build006::NO_BREAKS;
        let doc = build(src, &atoms, &file, 2, no_breaks, None, false, DEFAULT_MAX_BLANK_LINES)
            .expect("a file that parses is laid out");
        let out = render::render(
            &doc,
            &render::Options {
                max_width: 100,
                indent: 2,
                newline: "\n",
                max_blank_lines: 2,
            },
        );
        assert_eq!(out, "module M = struct val x = 1 end\n");
    }

    /// Property 5, on shapes chosen to reach every walk arm this file has.
    ///
    /// If the walk drifts, indentation is attributed to the wrong depth and
    /// nothing else in the suite would notice: the token stream is identical
    /// either way.
    #[test]
    fn the_walk_stays_in_step_with_the_atom_stream() {
        for src in [
            "@require: basic\nmodule M = struct\n  val x = 1\nend\n",
            "use package open Stdlib\nmodule M = struct\n  val x = 1\nend\n",
            "use Local of `./x`\nmodule M = struct\n  val x = 1\nend\n",
            "module M :> sig\n  val f : int -> int\n  val \\c : inline [int]\n  val +b : block [int]\nend = struct\n  val f x = x\n  val inline ctx \\c n = ctx\n  val block ctx +b n = ctx\nend\n",
            "module M = struct\n  signature S = sig\n    type t :: o\n    type u = int\n    module N : sig val g : int end\n    signature T = sig val h : int end\n    include S2\n  end\nend\n",
            "module M = struct\n  module Make = fun (X : S1) -> struct\n    val y = 1\n  end\nend\n",
            "module M = struct\n  module A = B\n  module C = D E\n  module F = G :> S\n  include H\nend\n",
            "module M = struct\n  val rec f x = g x\n  and g x = f x\nend\n",
            "module M = struct\n  val mutable r <- 0\n  val ~q = 1\n  val persistent ~p = 2\nend\n",
            "module M = struct\n  type t 'a = | A | B of int\n  and u = (| a : int, b : list 'a |)\nend\n",
            "module M = struct\n  val x = match y with | A -> 1 | B -> 2 end\nend\n",
            "module M = struct\n  val x = if a then b else c\n  val z = [1, 2, 3]\n  val w = (| p = 1, q = 2 |)\n  val v = (1, 2)\n  val u = ()\nend\n",
            "module M = struct\n  val f = fun ?(x = a) (y : int) -> y\n  val g = f ?(x = 1) 2\nend\n",
            "module M = struct\n  val x = let (a, b) = (1, 2) in a\n  val y = let open N in n\nend\n",
            "module M = struct\n  val x = r <- 1\n  val y = !r + 1\n  val z = M.f 1\n  val q = z#a\nend\n",
            "module M = struct\n  val math ctx \\lim with sub sup = base\nend\n",
            "module M = struct\n  val x = { a \\c{b} }\n  val y = '< +p{a} >\n  val z = ${ \\frac{1}{2} }\nend\n",
            "module M = struct\n  signature S = sig\n    val map 'a 'b : ('a -> 'b) -> list 'a\n  end\n  signature T = S with type t = int\n  signature U = S with M type t = int\n  signature V = (X : S) -> S\nend\n",
            "module M = struct\n  val ( +++ ) a b = a\nend\n",
            // The Document arm: a 0.1 `.saty` body is just an expression.
            "@require: basic\nlet x = 1 in\nx\n",
        ] {
            let (atoms, file) = parsed(src);
            assert_eq!(
                walk_desync(src, &atoms, &file, 2),
                0,
                "the walk drifted out of step on {src:?}"
            );
        }
    }

    // -- the hand-written before/after cases ---------------------------------

    /// A library's two indented blocks: `sig`'s decls and `struct`'s binds, both
    /// one step in from the `module` line, with `end`, `=` and `struct` back at
    /// the file's own depth.
    #[test]
    fn a_module_with_a_sig_annotation_indents_both_its_decls_and_its_binds() {
        let before = "\
module Annot :> sig
val href : string -> int
    val other : int
      end = struct
val href s = 1
        val other = 2
end
";
        let after = "\
module Annot :> sig
  val href : string -> int
  val other : int
end = struct
  val href s = 1
  val other = 2
end
";
        assert_eq!(fmt(before).as_deref(), Some(after));
    }

    /// `signature S = sig … end` as a struct member: the `sig` keyword stays on
    /// the `signature` line and its decls go one step past THAT line, not one
    /// step per construct.
    #[test]
    fn a_signature_bind_indents_its_decls_one_step() {
        let before = lib("  signature S = sig
val f : int -> int
        type t :: o
  end
");
        let after = lib("  signature S = sig
    val f : int -> int
    type t :: o
  end
");
        assert_eq!(fmt(&before).as_deref(), Some(after.as_str()));
    }

    /// `include` in both positions — a bind-include (a MODULE) and a
    /// decl-include (a SIGNATURE).
    #[test]
    fn an_include_keeps_its_operand_on_the_keywords_line_and_indents_a_struct() {
        let before = lib("    include struct
val a = 1
  end
  signature S = sig
      include T
  end
");
        let after = lib("  include struct
    val a = 1
  end
  signature S = sig
    include T
  end
");
        assert_eq!(fmt(&before).as_deref(), Some(after.as_str()));
    }

    /// A functor declaration. The signature parameter's `)` anchors to the line
    /// its `(` sat on, and the body after `->` is a body: one step past the
    /// arrow's line.
    #[test]
    fn a_functor_indents_its_body_one_step_past_the_arrow_line() {
        let before = lib("  module Make = fun (X : S1) ->
struct
val y = X.x
end
");
        let after = lib("  module Make = fun (X : S1) ->
    struct
      val y = X.x
    end
");
        assert_eq!(fmt(&before).as_deref(), Some(after.as_str()));
    }

    /// `val persistent ~x` and `val ~x`: 0.1's per-binding staging. The sigil
    /// binds to the name, and slice 1 neither inserts nor removes anything
    /// between them.
    #[test]
    fn staging_sigils_bind_to_their_operand() {
        // The prefixes survive a re-indent byte for byte, in both bind
        // positions and in a decl.
        let before = "\
module M :> sig
val persistent ~p : int
      val ~q : int
end = struct
val persistent ~p = 1
        val ~q = &(~p)
end
";
        let after = "\
module M :> sig
  val persistent ~p : int
  val ~q : int
end = struct
  val persistent ~p = 1
  val ~q = &(~p)
end
";
        assert_eq!(fmt(before).as_deref(), Some(after));
        // And the fusion rule that a slice which WRITES gaps will need: `&` `&`
        // is one binop token, so the two may never be written adjacently. Slice
        // 1 never writes a gap at all, so this is `must_separate`'s row rather
        // than a second rule here — asserted so that nobody adds one.
        assert!(
            sep::must_separate("&", "&x"),
            "`&` beside `&` fuses into one binop — the quote-of-a-quote hazard"
        );
        assert!(!sep::must_separate("~", "x"), "`~x` must stay tight");
    }

    /// The spine property: five `let … in` at one depth stay at one depth. A
    /// recursive printer would stair-step them.
    #[test]
    fn a_deep_let_in_chain_does_not_stair_step() {
        let mut before = String::from("module M = struct\n  val f x =\n");
        for i in 0..7 {
            before.push_str(&format!("{}let a{i} = {i} in\n", " ".repeat(6 + i * 2)));
        }
        before.push_str("      x\nend\n");

        let mut want = String::from("module M = struct\n  val f x =\n");
        for i in 0..7 {
            want.push_str(&format!("    let a{i} = {i} in\n"));
        }
        want.push_str("    x\nend\n");

        assert_eq!(fmt(&before).as_deref(), Some(want.as_str()));
    }

    /// The other spine forms, each of which must leave its continuation at the
    /// same depth.
    #[test]
    fn every_let_form_leaves_its_continuation_at_one_depth() {
        for body in [
            // Written at slice 3's own fixpoint: the spine is one `Mode::Auto`
            // group, so a chain that FITS comes back on one line. These are
            // formatted at 24 columns, where it does not.
            "  val f x =\n    let y = 1 in\n    y\n",
            "  val f x =\n    let rec g y = y in\n    g x\n",
            "  val f x =\n    let mutable r <- 0 in\n    !r\n",
            "  val f x =\n    let (a, b) = (1, 2) in\n    a\n",
            "  val f x =\n    let open N in\n    n\n",
        ] {
            let src = lib(body);
            // Formatted at the widest budget the SPINE still breaks at — one
            // column under the flat form's own width — so the continuation
            // after each `in` is on its own line at ONE depth, which is the
            // property this test is about. At the default budget the whole
            // chain fits and joins, which is the spine group's other arm and
            // says nothing about depth.
            let flat_width = body
                .lines()
                .map(|l| render::width(l.trim_start()))
                .sum::<usize>()
                + 2 // the `struct` indent
                + body.lines().count() // the spaces the joins put back
                - 2;
            let opts = super::super::CstOptions {
                max_width: flat_width - 1,
                wrap_inline_text: false,
                ..Default::default()
            };
            let out = super::super::format_cst(&src, RustyfiVersion::V0_1, &opts);
            assert_eq!(out.as_deref(), Some(src.as_str()), "{src:?}");
        }
        // With the re-wrap ON, the runs the lexer swallowed into a delimiter,
        // a bullet or a separator are still byte-identical — they are not
        // `Space`/`Break` tokens, so `Build::inline_gap` never sees them.
        for body in ["  val x = { * a * b }\n", "  val x = { a | b }\n"] {
            let src = lib(body);
            assert_eq!(fmt(&src).as_deref(), Some(src.as_str()), "re-wrap on: {src:?}");
        }
        // Math and block text are NOT byte-identical any more: their gaps
        // take the inverted default's [`Space::Collapse`], by the same
        // `default_space` 0.0.6 asks — see that file's
        // `math_and_block_gaps_collapse_but_never_gain_a_space`.
        for (body, want) in [
            ("  val x = ${x   +   y}\n", "  val x = ${x + y}\n"),
            ("  val x = '<   +p{hi}   >\n", "  val x = '< +p{hi} >\n"),
        ] {
            assert_eq!(fmt(&lib(body)).as_deref(), Some(lib(want).as_str()), "{body:?}");
        }
    }

    // -- the properties, on shapes a corpus file may not contain -------------

    #[test]
    fn a_gap_with_no_line_break_is_copied_byte_for_byte() {
        // Slice 1 has no intra-line rule at all, so hand alignment survives
        // everywhere: this is what slice 2 will start claiming, one named site
        // at a time.
        // **Slice 3 narrowed this.** A `val`'s body is a `Mode::Auto` group
        // with a break point in it, so the line is one the renderer may
        // re-wrap and the column model cannot say where a preserved run would
        // land — see [`super::build006::Build::mark_inexact`]. What survives
        // is a line the BUILDER lays out completely, which in 0.1 is the `sig`
        // decl below.
        let src = lib("  val x   =    1\n  val yy  =    2\n");
        assert_eq!(
            fmt(&src).as_deref(),
            Some(lib("  val x = 1\n  val yy = 2\n").as_str()),
            "a column on a re-wrappable line must collapse"
        );
        let sig = "\
module M :> sig
  val alpha   : int
  val b       : int
end = struct
  val alpha = 1
  val b = 2
end
";
        assert_eq!(fmt(sig).as_deref(), Some(sig), "a `sig` column died");
    }

    /// A **single-line** text or math area is still byte-identical, and under
    /// slice 4 that is a construction rather than a policy: nothing in these
    /// areas holds a line terminator, so there is no indentation to recompute.
    ///
    /// The `\frame(2pt)(   1pt   ){a}` row has MOVED off this list, and that
    /// is the 0.1 half of the inversion: a command's arguments are program
    /// text, slice 4 made them reachable, and 0.1 now runs the same rule set
    /// 0.0.6 does — so the run inside the parentheses tightens. The row is
    /// kept below, with what it becomes.
    #[test]
    fn a_single_line_text_or_math_area_is_still_byte_identical() {
        for body in [
            "  val x = { a  b }\n",
            "  val x = { * a * b }\n",
            "  val x = { a | b }\n",
        ] {
            let src = lib(body);
            // [`fmt_no_wrap`]: `{ a  b }` is a reflowable gap and slice 6
            // writes every reflowable gap as exactly one space, which rule 1
            // of the measurement licenses (a run's LENGTH is free everywhere,
            // 123 of 123). The 0.0.6 twin carries the whole argument.
            assert_eq!(fmt_no_wrap(&src).as_deref(), Some(src.as_str()), "{src:?}");
        }
        // With the re-wrap ON, the runs the lexer swallowed into a delimiter,
        // a bullet or a separator are still byte-identical — they are not
        // `Space`/`Break` tokens, so `Build::inline_gap` never sees them.
        for body in ["  val x = { * a * b }\n", "  val x = { a | b }\n"] {
            let src = lib(body);
            assert_eq!(fmt(&src).as_deref(), Some(src.as_str()), "re-wrap on: {src:?}");
        }
        // Math and block text are NOT byte-identical any more: their gaps
        // take the inverted default's [`Space::Collapse`], by the same
        // `default_space` 0.0.6 asks — see that file's
        // `math_and_block_gaps_collapse_but_never_gain_a_space`.
        for (body, want) in [
            ("  val x = ${x   +   y}\n", "  val x = ${x + y}\n"),
            ("  val x = '<   +p{hi}   >\n", "  val x = '< +p{hi} >\n"),
        ] {
            assert_eq!(fmt(&lib(body)).as_deref(), Some(lib(want).as_str()), "{body:?}");
        }
    }

    /// **Slice 4, all three areas**, on the 0.1 grammar. The mechanism is
    /// `build006.rs`'s verbatim — block text and math move bytes that are in
    /// *gaps*, inline text moves the indentation inside a `Space`/`Break`
    /// token — so the tests here are the 0.1 spelling of the same claims
    /// rather than a second measurement.
    #[test]
    fn the_three_text_areas_are_laid_out() {
        for (before, after, what) in [
            (
                "  val d = document (| a = 1 |) '<\n+p{\nhello\n}\n>\n",
                "  val d = document (| a = 1 |) '<\n    +p{\n      hello\n    }\n  >\n",
                "block text",
            ),
            (
                // Math is ATOMIC for line-breaking ([`Build::math_text`]) in
                // BOTH directions: no break is invented and none is removed.
                // Only the indentation is recomputed and the intra-line runs
                // collapse.
                "  val m = ${\n\\frac{1}{2}\n      + x\n}\n",
                "  val m = ${\n    \\frac{1}{2}\n    + x\n  }\n",
                "math",
            ),
            (
                "  val t = {\nalpha\n      beta\n}\n",
                "  val t = {\n    alpha\n    beta\n  }\n",
                "inline text",
            ),
            (
                "  val t = {\n日本語の文章を\n      書きます\n}\n",
                "  val t = {\n    日本語の文章を\n    書きます\n  }\n",
                "inline text, CJK",
            ),
            // The one mistake: a run starting with a space keeps exactly one,
            // or the `Space` token becomes a `Break` (`lexer.rs:1152`).
            (
                "  val t = {a  \n      b}\n",
                "  val t = {a \n    b}\n",
                "a `Space` run keeps its first character",
            ),
        ] {
            let (before, after) = (lib(before), lib(after));
            // [`fmt_no_wrap`]: this is slice 4's claim. With slice 6 on, the
            // latin arm's two lines join and the `Space` arm's run becomes
            // one space — both licensed, both asserted below.
            assert_eq!(fmt_no_wrap(&before).as_deref(), Some(after.as_str()), "{what}");
            assert_eq!(
                fmt_no_wrap(&after).as_deref(),
                Some(after.as_str()),
                "{what}: not a fixpoint"
            );
        }
    }

    /// **Slice 6 on the 0.1 grammar**: the same predicate, the same freeze.
    ///
    /// The 0.0.6 measurement is what licenses this, and the two generations
    /// share [`super::inline`] verbatim rather than each carrying a copy —
    /// but the 0.1 lexer and elaborator are a separate path, so the
    /// behaviour is pinned here rather than inferred. The CJK arm is the one
    /// that matters: it must come back exactly as written at any width.
    #[test]
    fn slice_6_re_wraps_0_1_inline_text_and_freezes_its_cjk() {
        // Latin: joined at a wide budget, split at a narrow one.
        let joined = lib("  val t = {\n    alpha beta gamma\n  }\n");
        assert_eq!(
            fmt(&lib("  val t = {\n    alpha\n    beta\n    gamma\n  }\n")).as_deref(),
            Some(joined.as_str()),
        );
        assert_eq!(fmt(&joined).as_deref(), Some(joined.as_str()), "not a fixpoint");
        let split = lib("  val t = {\n    alpha\n    beta\n    gamma\n  }\n");
        assert_eq!(fmt_at(&joined, 12).as_deref(), Some(split.as_str()));
        assert_eq!(fmt_at(&split, 12).as_deref(), Some(split.as_str()), "not a fixpoint");
        // CJK: frozen, at any width, in both directions.
        let cjk = lib("  val t = {\n    日本語の文章を\n    書きます\n  }\n");
        assert_eq!(fmt(&cjk).as_deref(), Some(cjk.as_str()));
        assert_eq!(fmt_at(&cjk, 200).as_deref(), Some(cjk.as_str()), "joined a frozen gap");
        let cjk_flat = lib("  val t = {\n    日本語の文章を書きます\n  }\n");
        assert_eq!(fmt_at(&cjk_flat, 10).as_deref(), Some(cjk_flat.as_str()), "split a frozen run");
        // And a `Space` run between two CJK characters keeps exactly one
        // space and its newline — never joined, never turned into a `Break`.
        let sp = lib("  val t = {日本 \n    語}\n");
        assert_eq!(fmt(&lib("  val t = {日本  \n      語}\n")).as_deref(), Some(sp.as_str()));
        assert_eq!(fmt(&sp).as_deref(), Some(sp.as_str()), "not a fixpoint");
    }

    #[test]
    fn a_blank_line_survives_and_a_run_of_them_is_capped() {
        let before = lib("  val a = 1\n\n  val b = 2\n\n\n\n\n  val c = 3\n");
        let after = lib("  val a = 1\n\n  val b = 2\n\n\n  val c = 3\n");
        assert_eq!(fmt(&before).as_deref(), Some(after.as_str()));
    }

    /// [`fmt`] at an explicit blank-line cap.
    fn fmt_blanks(src: &str, max_blank_lines: usize) -> Option<String> {
        format_cst(
            src,
            RustyfiVersion::V0_1,
            &CstOptions { max_blank_lines, ..Default::default() },
        )
    }

    /// **The 0.1 half of [`Breaks::block_blanks`]**, and it is a separate
    /// measurement rather than an inference: the two builders share the
    /// predicate ([`blank_line_in_gap`]) and the mechanism
    /// ([`Br::Hard`] in [`super::build006::Build::gap_upto`]) but not the
    /// walk, and it is the walk that decides where the rule is asked.
    ///
    /// Same three claims as 0.0.6's: the blank line survives, the run is
    /// capped by the OPTION, and both are fixpoints. The no-blank control is
    /// what says the surviving blank line also forced the item list open.
    #[test]
    fn a_blank_line_between_two_block_text_items_survives_and_is_capped() {
        let flat = lib("  val d = document (| a = 1 |) '< +p{aaa} +p{bbb} >\n");
        assert_eq!(
            fmt(&lib("  val d = document (| a = 1 |) '<\n+p{aaa}\n+p{bbb}\n>\n")).as_deref(),
            Some(flat.as_str()),
            "without a blank line a short block still flattens",
        );
        let before =
            lib("  val d = document (| a = 1 |) '<\n+p{aaa}\n\n+p{bbb}\n\n\n\n+p{ccc}\n>\n");
        let after = lib(
            "  val d = document (| a = 1 |) '<\n    +p{aaa}\n\n    +p{bbb}\n\n\n    +p{ccc}\n  >\n",
        );
        assert_eq!(fmt(&before).as_deref(), Some(after.as_str()));
        assert_eq!(fmt(&after).as_deref(), Some(after.as_str()), "not a fixpoint");
        let at_one = lib(
            "  val d = document (| a = 1 |) '<\n    +p{aaa}\n\n    +p{bbb}\n\n    +p{ccc}\n  >\n",
        );
        assert_eq!(fmt_blanks(&before, 1).as_deref(), Some(at_one.as_str()));
        assert_eq!(fmt_blanks(&at_one, 1).as_deref(), Some(at_one.as_str()), "not a fixpoint at 1");
    }

    /// **A zero cap collapses them here too**, and settles. `build006.rs`'s
    /// test of the same name carries why the second assertion — the fixpoint
    /// taken from the AUTHOR'S text — is the one that justifies the builder
    /// knowing the cap at all.
    #[test]
    fn a_zero_cap_collapses_a_block_texts_blank_lines_and_settles() {
        let before = lib("  val d = document (| a = 1 |) '<\n+p{aaa}\n\n\n+p{bbb}\n>\n");
        let after = lib("  val d = document (| a = 1 |) '< +p{aaa} +p{bbb} >\n");
        assert_eq!(fmt_blanks(&before, 0).as_deref(), Some(after.as_str()));
        // From the AUTHOR'S text, not from the settled form — see the 0.0.6
        // twin for why that is the assertion that can fail.
        let once = fmt_blanks(&before, 0).expect("laid out");
        assert_eq!(fmt_blanks(&once, 0).as_deref(), Some(once.as_str()), "not a fixpoint at 0");
    }

    /// A blank line just inside `'<` or just before `>` is dropped — the same
    /// answer 0.0.6 gives, for the same reason.
    #[test]
    fn a_blank_line_against_either_block_delimiter_is_dropped() {
        let before = lib("  val d = document (| a = 1 |) '<\n\n+p{aaa}\n\n+p{bbb}\n\n>\n");
        let after =
            lib("  val d = document (| a = 1 |) '<\n    +p{aaa}\n\n    +p{bbb}\n  >\n");
        assert_eq!(fmt(&before).as_deref(), Some(after.as_str()));
        assert_eq!(fmt(&after).as_deref(), Some(after.as_str()), "not a fixpoint");
    }

    #[test]
    fn an_own_line_comment_keeps_the_authors_indentation_whatever_it_is() {
        let before = "\
module M = struct
% about a
        val a = 1  % trailing
  % already at block depth
      % deeper than the block
end
";
        let after = "\
module M = struct
% about a
  val a = 1  % trailing
  % already at block depth
      % deeper than the block
end
";
        assert_eq!(fmt(before).as_deref(), Some(after));
    }

    #[test]
    fn tab_indentation_is_expanded_because_indentation_is_recomputed() {
        assert_eq!(
            fmt("module M = struct\n\t\tval a = 1\nend\n").as_deref(),
            Some("module M = struct\n  val a = 1\nend\n")
        );
    }

    #[test]
    fn an_if_indents_both_branch_bodies_that_begin_their_own_line() {
        let before = lib("  val f x =
    if x > 0 then
    `pos`
    else
    `neg`
");
        // Flat when it fits; at 30 columns both branches break TOGETHER, one
        // step in, with `else` back at the `if`'s own column.
        let flat = lib("  val f x = if x > 0 then `pos` else `neg`\n");
        assert_eq!(fmt(&before).as_deref(), Some(flat.as_str()));
        let wide = lib("  val f x = if x > 0 then `positive` else `negative`\n");
        let broken = lib("  val f x =\n    if x > 0 then\n      `positive`\n    else\n      `negative`\n");
        assert_eq!(fmt_at(&wide, 30).as_deref(), Some(broken.as_str()));
        assert_eq!(fmt_at(&broken, 30).as_deref(), Some(broken.as_str()), "not a fixpoint");
    }

    /// The larger half of the same bug: `Build::enter` alone did not merely fail
    /// to add a step, it **removed** one from a `then` body the author had
    /// already indented correctly.
    #[test]
    fn an_if_does_not_dedent_a_then_body_the_author_already_indented() {
        let src = lib("  val f x =
    if x > 0 then
      let y = 1 in
      y
    else
      0
");
        assert_eq!(
            fmt(&src).as_deref(),
            Some(lib("  val f x = if x > 0 then let y = 1 in y else 0\n").as_str()),
            "it fits, so it joins"
        );
        assert_eq!(fmt_at(&src, 30).as_deref(), Some(src.as_str()), "and breaks back");
    }

    /// A `match` arm body on its own line takes exactly one step from its arm,
    /// and 0.1's mandatory `end` lands in the `match` keyword's column.
    #[test]
    fn a_match_arm_body_takes_one_step_and_end_sits_under_match() {
        let before = lib("  val f x =
    match x with
    | A ->
    1
    | B ->
      2
    | C -> 3
        end
");
        let after = lib("  val f x =
    match x with
    | A -> 1
    | B -> 2
    | C -> 3
    end
");
        assert_eq!(fmt(&before).as_deref(), Some(after.as_str()));
        // And the arm body that does NOT fit: one step past its arm.
        let long = lib("  val f x =\n    match x with\n    | A -> alpha-value\n    | B -> beta\n    end\n");
        assert_eq!(fmt_at(&long, 26).as_deref(), Some(long.as_str()));
    }

    #[test]
    fn a_sequence_element_is_not_anchored_to_the_previous_elements_line() {
        // A list must not staircase: the `,` before an element sits at the end of
        // the PREVIOUS element's line, so `line_level` names a sibling's.
        // Both JOIN at the default budget — one rule for both ways, which is
        // `engine.md` section 12's example (1) — and both break one item per
        // line at ONE depth when they do not fit.
        let src = lib("  val xs = [
    (`a`, 1),
    (`b`, 2),
    (`c`, 3),
  ]
");
        let flat = lib("  val xs = [(`a`, 1), (`b`, 2), (`c`, 3),]\n");
        assert_eq!(fmt(&src).as_deref(), Some(flat.as_str()));
        assert_eq!(fmt_at(&flat, 24).as_deref(), Some(src.as_str()), "a list must not staircase");
        let rec = lib("  val r = (|
    aaaaa = 1,
    bbbbb = 2,
  |)
");
        let rec_flat = lib("  val r = (| aaaaa = 1, bbbbb = 2, |)\n");
        assert_eq!(fmt(&rec).as_deref(), Some(rec_flat.as_str()));
        assert_eq!(fmt_at(&rec_flat, 24).as_deref(), Some(rec.as_str()));
    }

    #[test]
    fn constructs_stacked_on_one_line_share_a_single_indentation_step() {
        // Three constructs — an argument run, a parenthesis and a `fun` body —
        // open on one line. One step between them, not three.
        let before = lib("  val f ctx =
    embed-block-top ctx (fun c ->
              form-paragraph c
                (read-inline c)
    )
");
        let flat = lib("  val f ctx = embed-block-top ctx (fun c -> form-paragraph c (read-inline c))\n");
        assert_eq!(fmt(&before).as_deref(), Some(flat.as_str()));
        // The claim where it can fail: at 44 columns the trailing
        // `(fun c -> …)` is HUGGED, so its body is one step past the CALL and
        // not two, and nothing breaks in front of the `(`.
        let want = lib("  val f ctx = embed-block-top ctx (\n    fun c -> form-paragraph c (\n      read-inline c\n    )\n  )\n");
        assert_eq!(fmt_at(&flat, 44).as_deref(), Some(want.as_str()));
        assert_eq!(fmt_at(&want, 44).as_deref(), Some(want.as_str()), "not a fixpoint");
    }

    #[test]
    fn an_operator_chain_breaks_before_the_operator_at_one_depth() {
        let before = lib("  val f ctx =
    ctx |> set-a 1
        |> set-b 2
");
        // Flat when it fits, **all of it or none of it** when it does not.
        let flat = lib("  val f ctx = ctx |> set-a 1 |> set-b 2\n");
        assert_eq!(fmt(&before).as_deref(), Some(flat.as_str()));
        let want = lib("  val f ctx =\n    ctx\n      |> set-a 1\n      |> set-b 2\n");
        assert_eq!(fmt_at(&flat, 28).as_deref(), Some(want.as_str()));
        assert_eq!(fmt_at(&want, 28).as_deref(), Some(want.as_str()), "not a fixpoint");
    }

    #[test]
    fn a_record_type_in_a_sig_closes_at_its_openers_line() {
        let before = "\
module M :> sig
val f : (|
text-width : length,
text-height : length,
|) -> int
end = struct
  val f r = 1
end
";
        let after = "module M :> sig\n  val f : (| text-width : length, text-height : length, |) -> int\nend = struct\n  val f r = 1\nend\n";
        assert_eq!(fmt(before).as_deref(), Some(after));
        // The arrow chain breaks too, at 40 columns — one group, all arrows or
        // none — so the record's own `|)` and the `-> int` land at the type's
        // depth rather than the declaration's.
        let narrow = "module M :> sig\n  val f : (|\n      text-width : length,\n      text-height : length,\n    |)\n    -> int\nend = struct\n  val f r = 1\nend\n";
        assert_eq!(fmt_at(after, 40).as_deref(), Some(narrow));
        assert_eq!(fmt_at(narrow, 40).as_deref(), Some(narrow), "not a fixpoint");
    }

    #[test]
    fn a_command_type_in_a_sig_closes_at_its_openers_line() {
        let before = "\
module M :> sig
val \\href : inline [
?(border : length * color) string,
inline-text,
]
end = struct
  val inline ctx \\href s t = ctx
end
";
        let after = "module M :> sig\n  val \\href : inline [?(border : length * color) string, inline-text,]\nend = struct\n  val inline ctx \\href s t = ctx\nend\n";
        assert_eq!(fmt(before).as_deref(), Some(after));
        // And the `]` back at its opener's own depth when the list breaks —
        // `dist-v01/packages/std-ja.satyh:90` is 272 columns of exactly this
        // shape before slice 3 reaches it.
        let narrow = "module M :> sig\n  val \\href : inline [\n    ?(\n      border : length * color\n    ) string, inline-text,\n  ]\nend = struct\n  val inline ctx \\href s t = ctx\nend\n";
        assert_eq!(fmt_at(after, 40).as_deref(), Some(narrow));
        assert_eq!(fmt_at(narrow, 40).as_deref(), Some(narrow), "not a fixpoint");
    }

    #[test]
    fn the_output_is_a_fixpoint_on_every_shape_this_module_moves() {
        for src in [
            "module M = struct\n% flush\n      val a =\n      if a then\n      1\n      else\n      2\n        % trailing thought\nend\n",
            "module M :> sig\nval f : int\nend = struct\nval f = 1\nend\n",
            "module M = struct\n  val f = fun x ->\n  x\nend\n",
        ] {
            let once = fmt(src).expect("formats");
            let twice = fmt(&once).expect("formats its own output");
            assert_eq!(once, twice, "{src:?}: second pass differs");
            // And a third, because a two-cycle would pass a two-pass test.
            assert_eq!(fmt(&twice).as_deref(), Some(once.as_str()));
        }
    }

    #[test]
    fn a_file_that_does_not_parse_is_whitespace_tidied_rather_than_re_indented() {
        // Tier 2 in `engine.md` section 8 is `crate::format`, and it IS wired
        // now — the 0.1 arm of the same claim `build006.rs` makes. No tree, so
        // no re-indentation; but the tab still expands to
        // `CstOptions::tab_spaces` columns and the final newline is still
        // added, because both are normalisations that need no tree.
        assert_eq!(
            fmt("\t\tmodule M = struct val x = end").as_deref(),
            Some("    module M = struct val x = end\n")
        );
    }

    #[test]
    fn a_file_that_does_not_lex_declines() {
        assert!(fmt("module M = struct val x = ) end\n").is_none());
    }

    // -- the universal normalisations ----------------------------------------

    /// The three edits every LSP formatter is expected to make, on the 0.1 path.
    /// They come from the renderer rather than from this walk, so they hold for
    /// both generations — `format_cst_slice1_v01.rs` carries the 0.0.6 half.
    #[test]
    fn a_file_gains_exactly_one_final_newline_and_loses_trailing_whitespace() {
        assert_eq!(
            fmt("module M = struct\n  val x = 1\nend").as_deref(),
            Some("module M = struct\n  val x = 1\nend\n")
        );
        assert_eq!(
            fmt("module M = struct\n  val x = 1   \nend\n\n\n").as_deref(),
            Some("module M = struct\n  val x = 1\nend\n")
        );
        // A header is the trap: `lex_header` swallows the line's terminator INTO
        // the token, so a file whose last token is a header already ends in a
        // newline and must not gain a second.
        assert_eq!(
            fmt("@require: basic\n").as_deref(),
            Some("@require: basic\n")
        );
        assert_eq!(fmt("@require: basic").as_deref(), Some("@require: basic\n"));
    }
}
