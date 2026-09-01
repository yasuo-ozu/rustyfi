//! Slices 1-4: layout of **0.0.6** program text — line breaks, indentation
//! and intra-line spacing — plus layout for the three text areas.
//!
//! # What these slices do
//!
//! **The line structure is the formatter's**, chosen by
//! [`super::render`]'s fit decision against [`super::CstOptions::max_width`];
//! every line's indentation is recomputed from CST depth; and a short, named
//! list of intra-line gaps is rewritten to a canonical form:
//!
//! - a gap the author broke is **joined** unless the construct around it
//!   offers a break there ([`Breaks`], and [`Build::gap_upto`] case 5) — with
//!   one exception, `${ … }`, whose line structure is kept exactly as it was
//!   (case 3);
//! - a gap between two tokens that holds **no** line break is copied byte for
//!   byte *unless a slice-2 rule claims it*. [`Spacing`] is the whole list:
//!   one space around `=`, `->`, `<-`, a name/type `:`, a clause `|` and an
//!   `OpChain` operator; none just inside `(`, `[`, `(|` or before their
//!   closers; none before `;` or `,`; and **one at every argument boundary of
//!   an application** ([`Spacing::app_arg`], whose doc comment carries the
//!   census that scopes it to a *variable* head and spares a constructor's
//!   and a command's). Everything else — a parameter run, `#label`, the
//!   interior of `?:x`, a stage sigil's grip on its operand — is still
//!   copied, which is what keeps this slice's blast radius equal to the size
//!   of that list;
//! - and a **column** the author built survives even where a rule claims it,
//!   provided nothing else on that line moved AND an adjacent line actually
//!   lines up with it. That is [`Spacing::preserve_alignment`] and
//!   [`preserved_lines`]; it keeps 806 of the corpus lines the rules would
//!   otherwise flatten, and those two doc comments are where the measurement
//!   and the idempotence argument live;
//! - one rule edits bytes **inside** a token span, which is otherwise
//!   forbidden by `doc.rs`'s module comment and by `format.rs:34-40`:
//!   `@require:   x` becomes `@require: x`. It is safe for a reason specific
//!   to `lex_header` and to nothing else, argued in full at
//!   [`Build::header_leaf`];
//! - a gap holding a `%` comment, or one at a position the builder asked for
//!   a HARD break ([`Br::Hard`] — one top-level binding, one `sig` item, one
//!   `match` arm per line), becomes [`Doc::HardLine`] / [`Doc::BlankLine`]
//!   plus each comment verbatim, and the *indentation* in it is dropped — the
//!   renderer supplies it from the enclosing [`Doc::Nest`] chain. That
//!   subsumes tab expansion and trailing-whitespace trimming for free, and it
//!   is the only position at which the author's BLANK lines survive;
//! - **slice 4**: inline text `{ }`, block text `'< >` and math `${ }` are
//!   laid out rather than copied ([`Areas`], one flag each). Block text and
//!   math get full freedom, because their whitespace is *gaps* — `lex_vertical`
//!   and `lex_math` emit no token for it — so re-indenting them cannot change
//!   the token stream. Inline text gets **re-indentation only**: a whitespace
//!   run there IS one token, and the safe edit set is "keeps a newline if it
//!   had one, gains none if it had not, is never emptied"
//!   ([`Build::emit_swallowed`], and
//!   `docs/plans/formatter-cst/ground-truth-whitespace.md` for the
//!   measurement). Joining or splitting a line inside inline text is the
//!   gap-level re-wrap and is NOT here.
//!
//! The one thing that is NOT here is a rule that reads the author's layout.
//! Slice 1 shipped `LineBreaks::Preserve` — keep every break, recompute only
//! the indentation — and slice 3 deleted it rather than leaving it beside the
//! fit decision, because `engine.md` section 6's hazard class 1 says a rule
//! that both reads the author's breaks and overrides some of them is not
//! idempotent and cannot be made so. What replaced the switch is [`Breaks`],
//! whose flags choose between "the renderer decides here" and "there is no
//! break here" — never between "the renderer decides" and "the author
//! decided".
//!
//! # Inserting a space is the new hazard, and it has one owner
//!
//! Slice 1 could only ever *copy* a gap, so it could not fuse two tokens.
//! Slice 2 writes gaps, and one direction of that is dangerous: removing a
//! space can turn `:` `:` into `::`, `1` `pt` into one length, `&` `&` into
//! one binop (a quote of a quote), or `y` `->` into the identifier `y-`
//! followed by `>` — `-` is an identifier character in this grammar, which is
//! the one that surprises.
//! [`super::sep::must_separate`] is the validated answer to that question —
//! exhaustively, over every ordered pair of the corpus's 11,770 token
//! spellings — and [`Build::canonical_space`] is the only caller. There is
//! deliberately no second rule anywhere in this file.
//!
//! # What is left of the input's layout, and why each of the three is safe
//!
//! 1. **Blank lines**, as a count clamped to `{0,1,2}` between two anchors the
//!    printer reproduces — `engine.md` section 6's class 5, and admitted only
//!    at a [`Br::Hard`] position, because that is the only place the second
//!    pass is guaranteed to read a break back.
//! 2. **Comment placement**: an own-line comment keeps its own indentation
//!    ([`Doc::VerbatimIndent`]) and a comment of either kind brings a
//!    [`Doc::HardLine`], which forces every enclosing `Auto` group open. That
//!    is what excludes hazard class 3 — a comment is never measured in flat
//!    mode.
//! 3. **A `${ … }`'s line structure**, kept exactly. Math is atomic for
//!    line-breaking in both directions: no break is invented inside it and
//!    none is removed. Disjoint from the decided region by AREA — which is a
//!    property of the tree, not of the layout — so it is a fixpoint rather
//!    than hazard class 1. See [`Build::gap_upto`] case 3.
//! 4. **The bytes of a gap a rule declines to touch** ([`Space::Keep`],
//!    [`Spacing::preserve_alignment`]), which is where hand alignment lives.
//!    Slice 3 narrowed the second one: a column survives only on a line the
//!    BUILDER lays out completely, because the column model
//!    ([`Build::advance`]) is exact only while nothing on the line is a break
//!    point. See [`Build::mark_inexact`].
//!
//! # Why a missed construct cannot corrupt anything
//!
//! Emission is driven by a **cursor over the lexed atom stream**, not by the
//! tree: [`Build::emit_atom`] always emits the atom at the cursor and the gap
//! before it, and whatever the walk does or fails to do, every atom's bytes and
//! every gap's bytes are emitted exactly once, in source order (the tail loop
//! in [`build`] finishes any atoms the walk did not reach). The tree only
//! decides which [`Doc::Nest`] frame a given atom lands in — that is, its
//! *indentation*. So a construct this builder does not descend into degrades to
//! "its continuation lines sit at the enclosing depth", never to a changed
//! token stream. That is what makes a construct-by-construct rollout safe.
//!
//! The one thing that could go quietly wrong is the cursor drifting out of step
//! with the walk, which would attribute indentation to the wrong depth. It is
//! not left to argument: [`Build::leaf`] checks that the atom at the cursor is
//! the one the CST leaf it was handed actually names, and counts the
//! mismatches. `tests/format_cst_slice1.rs` asserts that count is zero over
//! the whole corpus.

use rustyfi_syntax::cst::{self, ast};
use rustyfi_syntax::leaf::{
    AnyHorzCmdTok, AnyMathCmdTok, AnyVertCmdTok, BlockGroup, InlineGroup, MathGroup,
};
use rustyfi_syntax::span::Span;
use rustyfi_syntax::token::Atom;
use rustyfi_syntax::Token;
use syan::parse::unparse::Unparse;
use syan::span::Spanned;

use super::atoms_of;
use super::comment;
use super::inline;
use super::doc::{Doc, Mode};
use super::render;
use super::sep;
use super::trivia::{self, Piece};

/// The spacing rules, one flag each.
///
/// Not a `CstOptions` field and deliberately not user-visible: `config.md`
/// §4.3's admission rule for an option is "two independent failures, counted,
/// evidenced by a file and line count from a named `rustyfi fmt --check`
/// sweep", and none of these has one. What they are for is **measurement** —
/// each rule's corpus impact was taken by flipping exactly one of these and
/// re-running the sweep, which is the only way to report a per-rule number
/// rather than a total.
///
/// # The default is INVERTED, and that is the whole design
///
/// This list used to be a list of SHAPES — `=`, `->`, `:`, `|`, a binop, `;`,
/// `,`, a bracket, an argument boundary — each taught to the walk at the CST
/// site that knows it, and every gap no shape claimed was copied verbatim.
/// That arrangement produced the same bug report four times, because the
/// failure mode of a shape list is silence: a gap nobody named is not
/// mis-formatted, it is *untouched*, and untouched is indistinguishable from
/// deliberate. `let open    Derive in`, `let    x = 1 in`, `if 1 > 0    then`,
/// the trailing argument boundary of `myFn(123)0`, and an argument that is
/// not a group (`` List.intersperse  `, ` ``) were five faces of one mistake.
///
/// So [`Spacing::universal`] is the rule and everything else here is an
/// EXCEPTION to it. A shape nobody thought of is now normalised rather than
/// skipped, and the failure mode flips from "did nothing" to "did the
/// ordinary thing". The consequence worth stating plainly: `eq`, `arrow`,
/// `colon`, `bar`, `binop` and `app_arg` are **gone** rather than kept beside
/// the default. Six flags that all requested one space, in a world whose
/// default is one space, are six ways for a rule to look load-bearing while
/// covering nothing — and two rules that cover each other are both
/// individually deletable with the suite green, which is exactly how a
/// regression gets in.
///
/// # What a flag may and may not express
///
/// One uniform answer per operator TOKEN, never per binding strength.
/// `cst.rs:1039-1058` flattens all ten precedence levels into
/// `head (op rhs)*` and defers precedence to the elaborator (`cst.rs:6-10`),
/// so a rule of the form "tighter around `*` than around `+`" is not
/// implementable from this tree at all. That was the reason the old `binop`
/// flag was one flag for every operator, and it is the reason the default is
/// allowed to be one rule for all of them.
///
/// # Where an exception is asked
///
/// Every exception but [`Spacing::cmd_arg`] is decided from **the two tokens
/// either side of the gap** ([`Build::default_space`]), not from the CST site
/// that emitted them. That is not a stylistic preference: [`Build::flat`]
/// emits every pattern and most type expressions with no shape knowledge at
/// all, so a bracket rule attached to `Atomic::Paren` reaches `(f x)` and not
/// the `(x, y)` of `let f (x, y) = …`. Asking the tokens reaches both.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Spacing {
    /// **Every gap that holds nothing but horizontal whitespace, in every
    /// area but inline text, is rewritten** — to exactly one space in program
    /// and active areas, to *at most* one in math and block text, unless an
    /// exception below claims it.
    ///
    /// # The census that says one space is the canonical form
    ///
    /// Every gap the walk sees, by area and by what the author wrote
    /// (`the_v006_gap_census` / `the_v01_gap_census`, which re-take it):
    ///
    /// ```text
    ///   0.0.6, 162 files      empty      one   multi   break/comment/indent
    ///   program              37,164  106,571   1,157         24,390
    ///   active                7,071    2,051       2            528
    ///   math                  2,789      807      12            135
    ///   block text              152       73       0          1,453
    ///   inline text           8,321        0       0          1,076
    ///
    ///   0.1, 47 files         empty      one   multi   break/comment/indent
    ///   program               7,048   25,268     484          5,895
    ///   active                  149        7       0              0
    ///   math                     80        3       0              0
    ///   block text               16        0       0              0
    ///   inline text             277        0       0              0
    /// ```
    ///
    /// The last column is the gaps no rule may touch — a line break, a `%`
    /// comment, or a line's own indentation — and inline text is the area
    /// this default never reaches. That leaves **157,849 reachable gaps in
    /// 0.0.6 and 33,055 in 0.1**, and the number the default rests on is what
    /// is left of them once the exceptions are taken out:
    ///
    /// ```text
    ///   the RESIDUE               empty      one   multi   one-space share
    ///   0.0.6 program               824  105,858   1,039        98.3%
    ///   0.0.6 active                 38    1,744       2        97.8%
    ///   0.1   program                 2   25,188     465        98.2%
    ///   0.1   active                  8        7       0        46.7%
    /// ```
    ///
    /// **98.3% of the gaps nothing excepts are already exactly one space.**
    /// That is what makes one space the default rather than a preference, and
    /// it is why the whole inversion costs 46 of 162 files and 226 lines.
    /// (0.1's active row is 15 gaps in 47 files and decides nothing either
    /// way.)
    ///
    /// # Where one space is NOT the corpus's answer, with the number
    ///
    /// Math and block text. Their residues are
    ///
    /// ```text
    ///   0.0.6 math                  557      802      12        58.5%
    ///   0.0.6 block text            152       73       0        32.4%
    /// ```
    ///
    /// so `${abc}` and `'<+p{x};>` are the corpus's own forms, and inserting
    /// 709 spaces into hand-written math and block text would be a rewrite
    /// rather than a canonicalisation. Their default is [`Space::Collapse`]
    /// — the part both halves of the distribution agree on — and it is why
    /// the math extension moves 5 lines in one file and the block extension
    /// nothing at all.
    ///
    /// # What it never touches
    ///
    /// A gap holding a **line break** or a **`%` comment** is dropped by
    /// [`Build::gap_upto`] before any request is consulted, and that is what
    /// stops any rule joining two lines or eating a comment. The inversion
    /// does not relax it: the request is universal, the guard is not.
    ///
    /// Inline text is not reached either, and for a different reason — its
    /// whitespace is a TOKEN (`lexer.rs:1149-1155`) whose identity is fixed
    /// by its first character, governed by [`super::inline`]'s measured
    /// re-wrap predicate. See [`Area`], which is where the four-areas /
    /// one-exception shape is argued.
    ///
    /// # The fusion hazard
    ///
    /// [`Space::One`] never writes *nothing*: it collapses a run to one space
    /// and inserts one where there were none, so the direction that fuses two
    /// tokens is unreachable, and [`Space::Collapse`] only ever removes
    /// spaces from a run that keeps one. Only the tight exceptions can fuse,
    /// and they ask [`sep::must_separate`] — the single fusion authority,
    /// validated over every ordered pair of the corpus's 11,770 distinct
    /// token spellings.
    pub(crate) universal: bool,
    /// **Tight just inside an opening delimiter and just before a closing
    /// one**: `(`, `[`, `(|`, `M.(`, `${`, `<[` and their partners.
    ///
    /// 28,545 gaps in the 0.0.6 corpus and 5,335 in 0.1 — **99.0% and 99.6%
    /// already tight**; 449 and 22 change. The one rule that only ever
    /// REMOVES, so it is the one that leans hardest on
    /// [`sep::must_separate`]: `[` followed by what was `x |)` is how `|` `)`
    /// gets written adjacently.
    ///
    /// `${` and `<[` are on the list because the TOKENS are, not because a
    /// CST arm was written for them — which is the difference between this
    /// and the `bracket()` call it replaces, and it is why `let f (x, y)`'s
    /// parentheses are reached at all: a pattern goes through
    /// [`Build::flat`], which knows no shapes.
    ///
    /// `{` and `'<` are deliberately absent. `{ … }` is inline text, whose
    /// interior the default never reaches, and block text's own delimiters
    /// are written with a space inside them in the corpus.
    pub(crate) bracket: bool,
    /// One space just inside a RECORD `(| … |)` or a BLOCK `'< … >`, where
    /// [`Spacing::bracket`] would otherwise tighten it.
    ///
    /// Measured over the 209 corpus files before any formatting ran: `(| X` is
    /// **90.2% spaced** (129 against 14) and `X |)` **74.1% spaced** (80 against
    /// 28). `bracket`'s own 99.0%-canonical figure is an aggregate over six
    /// delimiter kinds in which `(` and `[` dominate, so the record pair's
    /// opposite habit was invisible inside it — the reason this is a separate
    /// flag rather than a tweak to `bracket`.
    ///
    /// `'< X` measures 61.0% *tight* (64 against 41) and is included on
    /// instruction rather than on the count. The count is also weak: only
    /// same-line boundaries are visible to it, and the ordinary `'<` followed
    /// by a newline appears in neither column.
    pub(crate) bracket_pad: bool,
    /// **Tight before `;` and `,`** — 11,435 gaps in 0.0.6 and 1,362 in 0.1,
    /// 97.9% and 92.8% already tight.
    ///
    /// `Token::EndActive` — the `;` that terminates a command's arguments —
    /// is on the list, and it is the one this rule could not reach while it
    /// was structural: ``\code(`x`)  ;`` is the same `;` reached through a
    /// different lexer mode.
    ///
    /// Never the other side: `[a; b]` and `(x, y)` put one space AFTER the
    /// separator, which is the default and needs no rule. Under the shape
    /// list it stayed `;b`, because no shape named it.
    pub(crate) separator: bool,
    /// **Tight on both sides of a `#` label access** — `rcd#field`. 1,336
    /// gaps in 0.0.6 and 226 in 0.1, and **every one of them is already
    /// empty**: 100% tight, in both corpora, with no counter-example.
    pub(crate) access: bool,
    /// **Tight around the sigils `?:`, `?`, `&`, `~` and `!`** — 1,043 gaps
    /// in 0.0.6 (100% already tight) and 203 in 0.1 (85.7%).
    ///
    /// `&` and `~` are the staging sigils and bind to their operand
    /// (`engine.md` section 9); `?:` introduces an optional argument, `!` a
    /// dereference (`!x`, 205 tight against none spaced). `?*`
    /// (`Token::Omission`) is one whole token and needs no rule.
    ///
    /// `?` is the one read on the RIGHT of a gap as well, and that is not
    /// symmetry for its own sake: in a command's type it is a POSTFIX marker
    /// — `[float?; math;] math-cmd` — written tight 249 times and spaced
    /// never, and its prefix use is `?(l = e)`, 215 tight against 0 spaced
    /// (` ?(` does not occur in either corpus).
    pub(crate) sigil: bool,
    /// **Tight around a math script marker** — `^`, `_` and a prime run:
    /// `${x^2}` rather than `${x ^ 2}`. 240 gaps, 238 of them already tight.
    ///
    /// The one exception that exists only because math came inside the
    /// default; before that, math spacing was copied and the question never
    /// arose.
    ///
    /// `_` and `'` are asked and **refused**, by the right authority: both
    /// are WORD characters in program mode (`x_i` and `x'` are identifiers),
    /// so [`sep::must_separate`] answers "these would fuse" and one space is
    /// written instead. The table is mode-blind and it is not overruled here
    /// — the cost is 2 gaps in the corpus, and the direction that mattered is
    /// covered for free: an EMPTY gap is never widened, because the author
    /// having written the two tokens adjacent is a proof that they do not
    /// fuse (see [`Build::canonical_space`]).
    pub(crate) script: bool,
    /// **A command head's argument boundary is neither spaced nor
    /// tightened** — [`Space::Collapse`]: an empty gap stays empty, one
    /// space stays one space, and a RUN becomes one space. 3,050 gaps in
    /// 0.0.6 and 43 in 0.1.
    ///
    /// The measured exception. The group census that scoped it, over the
    /// 0.0.6 corpus:
    ///
    /// ```text
    ///                  -> (      -> (|    -> [     -> {      -> '<    -> ${
    ///   \cmd (inline)  463/11    –         12/0     51/3      4/0      –
    ///   +cmd (block)   326/2     1/0       11/2     570/155   18/0     –
    ///   \cmd (math)    32/0      –         –        8/0       –        416/0
    ///   a variable     16/4008   0/83      3/235    10/185    0/61     0/19
    ///   a constructor  874/131   0/10      –        –         –        3/0
    ///                                                        (tight/spaced)
    /// ```
    ///
    /// **A command head is 1912 tight against 173 spaced — 91.7%**, and 98.6%
    /// for the inline and math heads; the whole-boundary census agrees, 2,745
    /// tight against 305. `+cmd` -> `{` is the one mixed row (570/155, and
    /// the 155 sit in 7 Japanese manuals), which is why the answer is not
    /// `Tight` and not [`Space::One`]: canonicalising it either way rewrites
    /// hundreds of boundaries to match a minority style, and the formatter
    /// has no business having an opinion this thin.
    ///
    /// # Why `Collapse` and not [`Space::Keep`]
    ///
    /// The census above is a two-column table — tight against spaced — and
    /// what it licenses is *do not insert* and *do not delete*. It says
    /// nothing about a RUN, because there is no third column: **of the 3,050
    /// 0.0.6 gaps and the 43 in 0.1, 2,745 + 43 are empty, 305 are exactly
    /// one space, and zero are a multi-space run.** So `Collapse` and `Keep`
    /// agree on every gap in both corpora — the change is worth 0 corpus
    /// lines — and they disagree on `${\frac{a}   {6}}`, `\c   {x}` and
    /// `+math(${…}   {6})`, which `Keep` handed back unchanged. A boundary
    /// the corpus writes tight 91.7% of the time is not a boundary anybody
    /// builds a column at; it is one somebody's cursor slipped at. Pinned by
    /// `a_run_at_a_command_argument_boundary_collapses` and its 0.1 twin,
    /// both of which fail if this goes back to `Keep`.
    ///
    /// The only exception decided **structurally** rather than from the two
    /// tokens, because "the head is a command" is not the question — the
    /// question is "this gap is a command's argument boundary", and
    /// `let-inline \cmd = …` has a command token on the left of a gap that is
    /// not one.
    pub(crate) cmd_arg: bool,
    /// **A constructor head followed by an opening delimiter is neither
    /// spaced nor tightened** — `Some(x)`, [`Space::Collapse`]. 1,018 gaps
    /// in 0.0.6 (874 tight, 109 at one space, 35 multi) and 196 in 0.1
    /// (176 tight, 20 at one space, 0 multi).
    ///
    /// A data constructor with an argument is not a function applied to a
    /// group, and the corpus writes it tight 85.9% of the time — mixed
    /// enough that having no opinion between empty and one space is the only
    /// honest answer.
    ///
    /// # The 35 runs, which are the only ones any `Collapse` exception has
    ///
    /// This is the one boundary on the whole exception list whose third
    /// column is not zero, so the 35 were **read** rather than counted
    /// (`CENSUS_CLASS=ctor_arg` on the census test dumps each of them with
    /// 45 bytes of context). Every one of them is a hand-built column, in
    /// two shapes. 34 are the `set-font` pipeline of six slide themes:
    ///
    /// ```text
    ///   ctx |> set-font Latin          (`lmmono`, 1.0, 0.0)
    ///     |> set-font Kana           (`lmmono`, 1.0, 0.0)
    ///     |> set-font HanIdeographic (`lmmono`, 1.0, 0.0)
    /// ```
    ///
    /// — the very block [`Mark::idx`] was widened for — and the 35th is
    /// `latexcmds.satyh:232-234`'s `text-in-math MathOpen  (fun ctx -> …)` /
    /// `MathClose (` / `MathInner (`.
    ///
    /// **18 of the 35 survive and 17 collapse**, and that split is the
    /// reason this is `Collapse` rather than [`Space::One`] in disguise: the
    /// column test is not bypassed, because `Collapse` delegates a non-empty
    /// gap to [`Space::One`]'s arm, which is where
    /// [`Spacing::preserve_alignment`] lives. What survives is the 17 `Kana`
    /// rows and the `MathOpen` row — the rows that actually line up with a
    /// neighbour. What collapses is the 17 `Latin` rows, and they collapse
    /// because their padding is **two columns off** the column their own
    /// continuation rows form (the `ctx |> ` head is 2 wider than the `|> `
    /// the rows below it start with), which is the same class [`Mark::idx`]
    /// hand-checked and accepted for the bare-argument form of this exact
    /// pipeline. So no table that stood before this change is flattened by
    /// it; one row that never belonged to its table is.
    ///
    /// Pinned by `a_constructors_argument_column_survives_the_collapse`.
    ///
    /// Lexical rather than structural, and that is a real widening: the old
    /// rule lived in [`Build::app_expr`] and so reached an EXPRESSION only,
    /// while `match Some(x) with` reaches [`Build::flat`], which knows no
    /// shapes. Under the inversion that pattern would have become `Some (x)`
    /// in every `match` in the corpus.
    ///
    /// A constructor with a BARE argument is not covered and takes the
    /// default: `Some x` has to be spaced or it fuses, and the corpus writes
    /// it 18/18 at one space.
    pub(crate) ctor_arg: bool,
    /// A **unary** minus is neither tightened nor spaced — 37 tight in the
    /// corpus against 4 spaced, and (in both corpora) **zero runs**. See
    /// [`Build::app_expr`], which is where it is asked and why it is not
    /// `Tight`.
    ///
    /// [`Space::Collapse`] rather than [`Space::Keep`] for the same reason
    /// as [`Spacing::cmd_arg`]: the 37/4 split is a reason to decline
    /// between `(-1)` and `(- 1)`, not a reason to keep `(-   1)`. Nothing in
    /// either corpus moves. Pinned by
    /// `a_run_after_a_unary_minus_collapses`.
    pub(crate) minus: bool,
    /// `@require:   x` -> `@require: x`.
    ///
    /// The one rule in the whole design that edits bytes **inside** a token's
    /// span. See [`Build::header_leaf`] for why it is nevertheless safe.
    pub(crate) header: bool,
    /// A run of **two or more** spaces where a rule above wants exactly one is
    /// copied verbatim instead — but only **on an output line no rule has
    /// otherwise relaid out**.
    ///
    /// # The number
    ///
    /// Measured over the 162-file 0.0.6 corpus with every other rule on, by
    /// `rustyfi fmt` + `git diff --numstat`, counting only what the spacing
    /// rules themselves change (a run with every rule OFF is the baseline and
    /// is subtracted):
    ///
    /// ```text
    ///                      files changed   lines changed
    ///   off                     104            1,589
    ///   on                        84              777
    /// ```
    ///
    /// So **812 of the 1,589 lines the rules would touch — 51% — are lines
    /// whose only non-canonical spacing is a hand-built column**, and the
    /// shapes are exactly the ones `format.rs:111-118` and `engine.md`
    /// section 10 name: `val font-cjk-gothic   : string * float * float` in a
    /// `sig`, `| f init []        = init` in a `let-rec` clause table,
    /// `text-height   : length;` in a record type, and
    /// `fss/src/class-nzla/nzla.satyh`'s page-geometry record:
    ///
    /// ```text
    ///   show-pages    = true;          show-pages = true;
    ///   paper-size    = A4Paper;  ->   paper-size = A4Paper;
    ///   text-width    = 440pt;         text-width = 440pt;
    /// ```
    ///
    /// Widening from records to expressions is what made this decisive: on
    /// records alone the share was 102 of 106 lines, but the absolute number
    /// was small enough to argue about. Reaching `sig` items and `let-rec`
    /// clause tables put it into four figures.
    ///
    /// # Which of those runs is actually a COLUMN
    ///
    /// The line test above says *nothing else on this line moved*; it does
    /// **not** say the run lines up with anything. `1  +  2` on a line of its
    /// own passed it and came back unchanged however many times the user
    /// saved, which is the defect [`preserved_lines`] fixes: a run is kept
    /// only when an ADJACENT line lines up with it — the same token starting
    /// at the same display column — and the witness has to be a line this
    /// formatter provably does not move. The predicate, its three witness
    /// kinds and the idempotence argument are all on that function.
    ///
    /// Measured the same way, by rendering the whole 0.0.6 corpus three times
    /// and diffing the outputs line by line rather than through `git`:
    ///
    /// ```text
    ///                                     files   lines
    ///   runs the COARSE rule preserves      61      869
    ///   runs the NARROW rule preserves      50      806
    ///   newly collapsed                     31       63
    /// ```
    ///
    /// So **92.8% of the columns survive the narrowing**, including every
    /// shape section 10 names — the `sig` tables, the `let-rec` clause table,
    /// the record — and 63 lines stop being treated as columns. Hand-checked,
    /// those 63 are: 15 copies of one stray double space before a `^`, 17
    /// `ctx |> set-font Latin␣␣(…)` pipeline heads whose padding is two
    /// columns off the column their own continuation rows form, 5
    /// `start-path` lines two columns off the same way, 2 rows of a real
    /// column whose neighbours are `separator;` lines (the adjacency cost,
    /// pinned by `a_column_whose_rows_are_not_adjacent_is_not_seen`), 2 `sig`
    /// items whose column the `colon` rule was already destroying either way,
    /// 4 record fields whose collapse RESTORES the alignment the author
    /// mistyped, and 18 assorted stray runs. One class of loss, one class of
    /// repair, and no table that stood before and does not now.
    ///
    /// # Why per LINE and not per gap
    ///
    /// A per-gap version — "keep any run of 2+ spaces a rule wants to collapse"
    /// — leaves `(|a = 1;b   =   2;c = 3|)`, canonicalising either side of a
    /// column while leaving the column standing in the middle. That is the
    /// user's own example and nobody means it. So the question is asked once
    /// per output line: **did any rule change anything on this line that was
    /// not itself a column?** If nothing did, the author was maintaining a
    /// table and it is left alone; if something did, the line was being
    /// relaid out anyway and the column goes with it.
    ///
    /// The cost of asking it per line rather than per rule is real and is
    /// pinned by
    /// `a_record_types_column_goes_when_its_semi_is_tightened_on_the_same_line`:
    /// a stray space before a `;` takes that line's `:` column down with it.
    /// `engine.md` section 10's `% rustyfi-fmt: off` marker is the sanctioned
    /// escape hatch for a block where that is not acceptable.
    ///
    /// [`build`] therefore walks the file **twice**: [`Pass::Scan`] answers the
    /// question for every line and throws its `Doc` away, [`Pass::Emit`]
    /// consults the answers. The two walks see the same line breaks in the
    /// same order — no spacing rule can add or remove one, because
    /// [`Build::gap_upto`] drops any request over a gap holding a break — so
    /// the line indices mean the same thing in both. The scan is skipped
    /// entirely when this flag is off.
    ///
    /// # Why this is not the alignment pass section 6 forbids
    ///
    /// `engine.md` section 6, class 2, excludes *alignment* because an
    /// alignment pass is a fixpoint over its own output: widen one token and
    /// every aligned column moves, which may widen another. This flag computes
    /// no column and moves nothing. It only declines to collapse a run it did
    /// not create, so the bytes written are the bytes read — the same licence
    /// [`Doc::VerbatimIndent`] already runs on for an own-line comment's
    /// indentation.
    ///
    /// Narrowing it to actual columns made idempotence something to argue
    /// rather than something to inherit, because collapsing a run on line *N*
    /// moves every mark to its right and so changes what line *N*+1 could have
    /// lined up with. The argument is on [`preserved_lines`] and it turns on
    /// one asymmetry: a run survives into the output ONLY on a preserved line,
    /// so the second pass has exactly the same candidates as the first, while
    /// the witnesses it may consult only ever grow. Pinned by
    /// `a_collapse_cannot_hand_its_neighbour_a_column_on_the_second_pass`,
    /// which is the shape the argument is about.
    ///
    /// What it never preserves is a run before a `;`, a `,` or a closing
    /// bracket. Those rules want *nothing*, two spaces in front of a
    /// terminator is not a column anybody built, and the corpus has exactly
    /// one such line.
    pub(crate) preserve_alignment: bool,
}

/// What ships.
pub(crate) const SLICE2: Spacing = Spacing {
    universal: true,
    bracket: true,
    bracket_pad: true,
    separator: true,
    access: true,
    sigil: true,
    script: true,
    cmd_arg: true,
    ctor_arg: true,
    minus: true,
    header: true,
    preserve_alignment: true,
};

/// Slice 4's per-area policies — `engine.md` section 4's `AreaPolicy` hook,
/// one flag per area so each can be measured and mutated on its own.
///
/// `false` is `AreaPolicy::Verbatim`: the area is one [`Doc::Verbatim`],
/// interior included, which is what slices 0-3 do to all three.
///
/// The order they were turned on is the order of increasing certainty *cost*,
/// and each is a separate commit for that reason.
#[derive(Debug, Clone, Copy)]
struct Areas {
    /// Block text `'< … >` -> `AreaPolicy::Free`.
    ///
    /// **Provable.** `lex_vertical` emits no token for whitespace or comments
    /// (`lexer.rs:1029-1032`) and [`cst::ast::BlockElem`] has only `Embed` and
    /// `Cmd` — there is no whitespace variant in the CST at all. So every
    /// space and every newline in a block area is a *gap*, the same object
    /// program text's indentation is made of, and re-indenting one cannot
    /// change the token stream because there is no token there to change.
    block: bool,
    /// Math `${ … }` -> `AreaPolicy::Free`.
    ///
    /// **Provable, by the same argument**: `lex_math` skips whitespace and
    /// comments without emitting (`lexer.rs:1338-1340`), so a math area's
    /// whitespace is gaps too.
    math: bool,
    /// Inline text `{ … }` -> `AreaPolicy::Reindent`.
    ///
    /// **Not provable — measured**, and the one area where the whitespace is
    /// a token. A run collapses to one `Space`/`Break` whose identity is
    /// fixed by its FIRST character (`lexer.rs:1149-1155`), so a rewrite is
    /// output-preserving exactly when every run keeps a newline if it had one,
    /// gains none if it had not, and is never emptied — which is what
    /// `ground-truth-whitespace.md` measures (I24 EQUAL for a re-indented
    /// continuation line in CJK; I25 DIFFERING for the trailing-whitespace
    /// trim that turns a `Space` into a `Break`).
    ///
    /// Joining or splitting a line is NOT this flag: that is a gap-level
    /// re-wrap, its predicate is `README.md`'s rule 3, and nothing here can
    /// reach it — [`Build::gap_upto`] drops a request over a break and
    /// [`Build::break_structure`] emits one terminator per terminator read.
    inline: bool,
}

/// What slice 4 ships.
const SLICE4: Areas = Areas {
    block: true,
    math: true,
    inline: true,
};

/// **Slice 3.** Which constructs offer the renderer a place to break, one
/// flag each.
///
/// The counterpart of [`Spacing`], and it exists for the same two reasons: a
/// per-construct rollout (`engine.md` section 11 asks for exactly this, with
/// the corpus sweep run after each flag) and a per-construct *mutation* — a
/// rule whose flag can be turned off is a rule whose test can be shown to
/// fail.
///
/// Turning one off does not restore slice 1. There is no mode in which the
/// author's break positions are read any more: a gap the builder offers no
/// break at is **joined**, whatever the author wrote. That is
/// `engine.md` section 6's hazard class 1 removed rather than parameterised —
/// a rule that both reads the author's breaks and overrides some of them is
/// not idempotent and cannot be made so, so the flag chooses between "the
/// renderer decides here" and "there is no break here", never between "the
/// renderer decides" and "the author decided".
#[derive(Debug, Clone, Copy)]
pub(crate) struct Breaks {
    /// A bracketed group — `( )`, `[ ]`, `(| |)`, `${ }`, a `sig`-item record
    /// kind — breaks just inside its delimiters, as one [`Mode::Auto`] group
    /// whose closer lands back at the opener's own depth.
    pub(crate) groups: bool,
    /// One field per line in a `(| … |)`, one item per line in a `[ … ]`, one
    /// component per line in a tuple — or all of them flat.
    /// `progsynt.satyh:56-64` writes the same record both ways four lines
    /// apart, which is the textbook [`Mode::Auto`] case.
    pub(crate) items: bool,
    /// An application's argument run fills: as many arguments per line as fit
    /// ([`Doc::FillLine`]), with the trailing group-shaped argument *hugged*
    /// — see [`Build::hugs_last`].
    pub(crate) app_args: bool,
    /// `head (op rhs)*` is one [`Mode::Auto`] group, **all of it or none of
    /// it**, breaking *before* the operator.
    ///
    /// `cst.rs:1039-1058` flattens all ten precedence levels and defers
    /// precedence to the elaborator, so the formatter cannot know where a
    /// precedence boundary is. A fill would put `a + b *` and `c` on two lines
    /// and read as though the chain associated that way; all-or-nothing cannot
    /// tell that lie. `stdja.satyh:505-507` is the corpus idiom.
    pub(crate) op_chain: bool,
    /// The `let … in` spine is one [`Mode::Auto`] group at ONE nesting level,
    /// breaking after every `in` or after none.
    ///
    /// `stdja.satyh:272-289` is eleven consecutive `let … in` at one
    /// indentation; the walk is already iterative ([`Build::expr`]) so the
    /// flatness is inherited, and this flag adds only the break points.
    pub(crate) let_spine: bool,
    /// A body introduced by a token already on this line — `=`, `->`,
    /// `then`, `else`, `do` — may start on the next line, one step in.
    pub(crate) bodies: bool,
    /// `match` arms and `let-rec` clauses: **always** one per line, `|` at the
    /// head. Joining them is legal and unreadable, so this is a
    /// [`Doc::HardLine`] rather than a group mode.
    pub(crate) clauses: bool,
    /// `sig` items, `struct` bindings and the file's own top-level bindings:
    /// always one per line, one [`Doc::Nest`] step. Also a [`Doc::HardLine`],
    /// and it is the flag that keeps the author's **blank lines** — the one
    /// legitimate reader of input layout (`engine.md` section 6, class 5),
    /// admitted only where the second pass provably reads back what the first
    /// wrote.
    pub(crate) blocks: bool,
    /// A `'< … >` block text lays its `+cmd`s out one per line when they do
    /// not fit flat.
    pub(crate) block_items: bool,
    /// A `'< … >` item the author separated from the previous one by a **blank
    /// line** starts a new line unconditionally, and that blank line survives
    /// — clamped to [`Build::max_blank_lines`], exactly as [`Breaks::blocks`]
    /// clamps a top-level one.
    ///
    /// The second legitimate reader of input layout (`engine.md` section 6,
    /// class 5), and it is admitted on the same terms as the first: a blank
    /// line survives only where the builder turns the gap into a [`Br::Hard`],
    /// so the second pass reads back a blank line at exactly the positions the
    /// first pass wrote one. What makes that self-consistent here is that the
    /// PREDICATE and the OUTPUT are the same proposition — "this gap holds two
    /// or more terminators". A gap that satisfies it is emitted with at least
    /// one blank line still in it (`max_blank_lines >= 1` is part of the
    /// predicate, which is why `--max-blank-lines 0` turns the rule off rather
    /// than capping it to nothing), and a gap that does not is never given
    /// one, because nothing else in a block area emits a [`Doc::BlankLine`].
    /// So the fixpoint is not "the rule happens to agree with itself"; it is
    /// that the rule's input is recoverable from its own output.
    ///
    /// Subordinate to [`Breaks::block_items`]: that flag is the difference
    /// between "the renderer decides here" and "there is no break here", and a
    /// blank line cannot survive at a gap that is not a break at all.
    ///
    /// **Only BETWEEN two items.** A blank line just inside `'<` or just
    /// before `>` is dropped, and that is the same answer
    /// [`super::render::flush_blanks`] gives a leading blank line at the top
    /// of a file: a blank line is a separator between two things the printer
    /// reproduces, and at a delimiter there is only one thing. Pinned by
    /// `a_blank_line_against_either_block_delimiter_is_dropped`.
    pub(crate) block_blanks: bool,
    /// A function type breaks before its `->`, all arrows or none.
    pub(crate) type_arrows: bool,
}

/// What slice 3 ships.
pub(crate) const SLICE3: Breaks = Breaks {
    groups: true,
    items: true,
    app_args: true,
    op_chain: true,
    let_spine: true,
    bodies: true,
    clauses: true,
    blocks: true,
    block_items: true,
    block_blanks: true,
    type_arrows: true,
};

/// [`super::CstOptions::max_blank_lines`]'s default, and what the two walks
/// that are not a format — [`walk_desync`] and [`gap_census`] — run under.
///
/// It lives here rather than inline in `CstOptions::default` so there is one
/// 2 rather than three: neither of those two walks produces output, so a
/// number of their own could drift from the real default without anything
/// failing, and the walk they measure has to be the walk a default
/// `format_cst` performs.
pub(crate) const DEFAULT_MAX_BLANK_LINES: usize = 2;

/// Nothing breaks and nothing groups: the mutation control, and the shape
/// `Breaks` is measured against one flag at a time.
#[cfg(test)]
pub(crate) const NO_BREAKS: Breaks = Breaks {
    groups: false,
    items: false,
    app_args: false,
    op_chain: false,
    let_spine: false,
    bodies: false,
    clauses: false,
    blocks: false,
    block_items: false,
    block_blanks: false,
    type_arrows: false,
};

/// The flat rendering of one gap: what the spacing policy puts there when the
/// line does not break at it.
///
/// [`Flat::Keep`] is the author's own bytes and is not a spelling at all —
/// only [`Space::Keep`] produces it, and only before `Eoi` or in an area no
/// rule reaches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Flat {
    Empty,
    Space,
    Keep,
}

/// What the builder wants of the *line structure* at the gap before the atom
/// it is about to emit.
///
/// [`Space`]'s counterpart, and the two are asked together: a break request
/// carries no spelling of its own, so the flat rendering of an
/// [`Br::Opportunity`] is whatever [`Build::default_space`] would have put
/// there. That is what stops a break point from silently inserting a space
/// where the corpus writes none — `Some(x)` is an argument boundary whose
/// canonical gap is EMPTY, and a fill point there would render `Some (x)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Br {
    /// A line ends here, whatever the width. The author's blank lines survive
    /// at one of these and nowhere else.
    Hard,
    /// [`Doc::Line`] if the canonical gap here is one space, [`Doc::SoftLine`]
    /// if it is empty. The enclosing [`Mode::Auto`] group decides.
    Opportunity,
    /// [`Doc::FillLine`]: a greedy break, decided against what follows it
    /// alone. Offered **only** where the canonical gap is one space — a fill
    /// point that rendered flat as nothing would be a break with no visible
    /// counterpart, and one that rendered flat as a space where the corpus
    /// writes none would be a spacing change wearing a line-break's clothes.
    Fill,
}

/// What the builder wants in the gap before the atom it is about to emit.
///
/// Consumed by [`Build::gap_upto`], which is the single place a gap's bytes
/// are decided — so a rule can only ever be *requested*, never written
/// directly, and every request passes the same two guards: the gap must hold
/// no line break (a break is line structure, which is slice 3's) and no `%`
/// comment (rewriting one deletes it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Space {
    /// Exactly one space.
    ///
    /// Always safe against the fusion hazard: inserting whitespace can only
    /// separate two tokens, never join them.
    One,
    /// Nothing — unless [`sep::must_separate`] says the two ranges would lex
    /// as one token, in which case one space.
    ///
    /// This is the direction that can corrupt a document, which is why it asks
    /// the table rather than a second opinion.
    Tight,
    /// A run of two or more spaces becomes one; an empty gap stays empty.
    ///
    /// **Math and block text's default**, and it is a measured answer rather
    /// than a hedge: those two areas write 2,941 of their 3,833 reachable
    /// gaps tight, so one space is not their canonical form and inserting
    /// 557 spaces into hand-written math would be a rewrite rather than a
    /// canonicalisation. Collapsing is the part both halves of the corpus
    /// agree on. See [`Spacing::universal`]'s census.
    ///
    /// Cannot fuse: it only ever removes spaces from a run that keeps one.
    Collapse,
    /// The author's own bytes, whatever they are — a multi-space run
    /// included.
    ///
    /// Not the absence of a rule: the absence of a rule is now
    /// [`Spacing::universal`], which claims every gap. This is how an
    /// exception says *leave it alone*.
    ///
    /// # It answers exactly one boundary, and that boundary has no tokens
    ///
    /// The gap before `Eoi`, in [`default_space`]. Everywhere else the
    /// question an exception is really answering is *may the formatter
    /// INSERT a space here* — a claim about an EMPTY gap — and `Keep` also
    /// answers *may it collapse a RUN*, which is a second claim and one no
    /// census supports. `cmd_arg`, `ctor_arg` and `minus` all used to answer
    /// `Keep` and all three now answer [`Space::Collapse`], which grants the
    /// first and refuses the second; `${\frac{a}   {6}}`, `Some   (1)` and
    /// `\c   {x}` came back unchanged until they did.
    ///
    /// End of file is different in kind rather than in degree, and that is
    /// why it stays. The bytes never reach the output at all —
    /// [`super::render::finish`] trims every trailing space in the buffer —
    /// so `Collapse` there could not fix anything, while the SCAN pass would
    /// still record the last line as relaid out and take its column down. A
    /// rule with no upside and a real downside declines.
    Keep,
}

/// Which **area** the walk is inside, and therefore whether
/// [`Spacing::universal`] reaches the gap being emitted.
///
/// The one distinction the inverted default needs, and it has to come from
/// the WALK rather than from the token stream: an area fold over tokens is
/// exactly the computation `area.rs:47-65` records getting wrong at the
/// `<[`/`]>` boundary, and this one has a nesting the tokens do not show —
/// math can escape back into inline text (`!{ … }`, `lexer.rs:1357-1365`) and
/// inline text can nest math again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Area {
    /// Program text, including a command's argument run (`lex_active` emits
    /// no token for the whitespace there, `lexer.rs:1241-1243`).
    Program,
    /// `${ … }`. Whitespace here is a *gap* too — `lex_math` bumps past it
    /// without emitting (`lexer.rs:1338-1340`) — and is invisible to the
    /// typesetter: `${x   +   y}`, `${x + y}` and `${x+y}` compile to
    /// byte-identical PDFs, against a `${x - y}` control that differs. So the
    /// default reaches here on the same terms as program text.
    Math,
    /// `{ … }`. The one area whose whitespace **is a token**
    /// (`lexer.rs:1149-1155`), governed by [`super::inline`]'s measured
    /// re-wrap predicate and by nothing else. The default never reaches it.
    Inline,
    /// `'< … >`. Gaps again: `lex_vertical` skips whitespace without emitting
    /// (`lexer.rs:1029-1032`).
    Block,
    /// A command's argument run and its `;` terminator — `\cmd(x); `,
    /// `#var;`, `+p{…};`. Gaps again: `lex_active`'s whitespace arm is a bare
    /// `bump()` with no emit (`lexer.rs:1241-1243`).
    ///
    /// Distinguished from [`Area::Program`] for the census only; the rule is
    /// the same. Everything lexically inside the run counts as active,
    /// including a nested `(let x = 1 in x)` — the boundary the lexer draws
    /// there is not one the walk has any reason to redraw.
    Active,
}

impl Area {
    /// Does the inverted default reach a gap in this area?
    ///
    /// **Four of the five areas are one code path**, and the reason is one
    /// property rather than four coincidences: their whitespace produces no
    /// token, so there is nothing in it for the typesetter to see and nothing
    /// for a rewrite to lose. Inline text is the single exception — one run
    /// is one `Space`/`Break` token whose identity is fixed by its first
    /// character (`lexer.rs:1149-1155`) — and it has its own machinery
    /// ([`super::inline`]) rather than an exception inside this one.
    pub(crate) fn claimed(self) -> bool {
        match self {
            Area::Program | Area::Math | Area::Block | Area::Active => true,
            Area::Inline => false,
        }
    }
}

/// One gap, as [`gap_census`] sees it. Measurement only; nothing in the
/// formatter reads it, and it is not compiled into one.
#[cfg(test)]
#[derive(Debug, Clone, Copy)]
pub(crate) struct Obs {
    /// Index of the atom the gap sits in front of.
    pub(crate) at: usize,
    /// The gap's own byte range in the source.
    pub(crate) span: (usize, usize),
    /// Which area the walk was in.
    pub(crate) area: Area,
    /// What a rule asked for, *before* the default is applied.
    pub(crate) want: Option<Space>,
    /// Does the gap hold a line break? Such a gap is never rewritten.
    pub(crate) has_break: bool,
    /// Is the gap horizontal whitespace only (possibly empty)? A gap holding
    /// a `%` comment is not, and is never rewritten either.
    pub(crate) blank: bool,
    /// Was the output at the start of a line? Such a gap is the next line's
    /// INDENTATION, which the renderer writes; no spacing rule reaches it.
    pub(crate) at_line_start: bool,
}

/// Whether a leaf is emitted as its own source bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Subst {
    /// Its own bytes. Every leaf but one.
    None,
    /// A header line, re-emitted as `@name: rest`. See
    /// [`Build::header_leaf`].
    Header,
    /// A **horizontal-mode** token whose span swallowed trivia, re-emitted
    /// with that trivia's indentation dropped. See [`Build::emit_swallowed`].
    Swallow,
    /// [`Subst::Swallow`] for a group's OPENING delimiter, whose trailing
    /// trivia is *held* rather than emitted.
    ///
    /// `{` swallows the whitespace after it (`lexer.rs:562-567`), so a
    /// multi-line inline area's first line break is inside the `{` token. It
    /// has to be emitted **after** the group's frame is opened, or the first
    /// content line lands one step shallower than the rest —
    /// [`Build::group_open_as`] is the only caller and the only flusher.
    SwallowOpen,
}

/// How long a prefix of `s` the lexer would have skipped: spaces, tabs, line
/// terminators and `%` comments.
///
/// The same alphabet [`trivia::classify`] accepts, scanned rather than
/// classified, because this is asked about the INTERIOR of a token's span
/// rather than about a gap.
fn trivia_len(s: &str) -> usize {
    let b = s.as_bytes();
    let mut i = 0usize;
    while i < b.len() {
        match b[i] {
            b' ' | b'\t' | b'\r' | b'\n' => i += 1,
            // To end of line, exactly as `Lexer::comment` does.
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
///
/// In inline text the lexer does not leave whitespace in a **gap**: the
/// `(break|space)* <terminator>` family swallows the run before `}`, `{`, `<`,
/// `|` and an item bullet into that token's span (`lexer.rs:1112-1147`), a `{`
/// then calls `skip_spaces()` and swallows what follows it too
/// (`lexer.rs:562-567`, `:1114-1119`), and a whitespace run that is *not*
/// followed by one of those becomes one whole `Space`/`Break` token
/// (`lexer.rs:1149-1155`). So re-indenting inline text is unreachable from a
/// gap; it has to read the token.
///
/// `core` is empty exactly for `Space`/`Break`, which is the case that needs
/// `keep_first_space` — see [`Build::break_structure`].
fn split_swallowed(t: &str) -> (&str, &str, &str) {
    let lead = trivia_len(t);
    let rest = &t[lead..];
    let core = rest
        .find([' ', '\t', '\r', '\n', '%'])
        .unwrap_or(rest.len());
    (&t[..lead], &rest[..core], &rest[core..])
}

/// An opening delimiter, for [`Build::default_space`]'s bracket rule.
///
/// `BMathGrp` is `${` **and** a nested math `{`, and `BPath` is `<[`; both are
/// here because the TOKEN is, which is the difference between this rule and
/// the `Atomic::Paren`-shaped one it replaces. `BHorzGrp` and `BVertGrp` are
/// deliberately absent: `{ … }` is inline text, whose interior this rule
/// never reaches, and `'< … >` is written with a space inside it in the
/// corpus.
pub(crate) fn opens_a_group(t: &Token) -> bool {
    matches!(
        t,
        Token::LParen
            | Token::BList
            | Token::BRecord
            | Token::OpenModule(_)
            | Token::BMathGrp
            | Token::BPath
    )
}

/// A closing delimiter. The partners of [`opens_a_group`].
pub(crate) fn closes_a_group(t: &Token) -> bool {
    matches!(
        t,
        Token::RParen | Token::EList | Token::ERecord | Token::EMathGrp | Token::EPath
    )
}

/// `;` or `,` — including `EndActive`, the `;` that terminates a command's
/// arguments, which is the same rule reached through a different lexer mode.
pub(crate) fn is_separator(t: &Token) -> bool {
    matches!(t, Token::ListPunct | Token::Comma | Token::EndActive)
}

/// A sigil that binds to the operand after it: `?:`, `?`, `&`, `~`, `!`.
///
/// `?` is also read on the RIGHT of a gap, and that is not symmetry for its
/// own sake: in a command's type it is a POSTFIX marker — `[float?; math;]
/// math-cmd` — written tight 249 times in the corpus and spaced never.
/// `Token::Omission` (`?*`) is one whole token and needs no rule.
pub(crate) fn is_sigil(t: &Token) -> bool {
    matches!(
        t,
        Token::Optional
            | Token::OptionalType
            | Token::ExactAmp
            | Token::ExactTilde
            | Token::UnopExclam(_)
    )
}

/// A math script marker: `^`, `_`, a prime run.
pub(crate) fn is_script(t: &Token) -> bool {
    matches!(t, Token::Superscript | Token::Subscript | Token::Primes(_))
}

/// Does `s` hold a line terminator?
fn holds_break(s: &str) -> bool {
    s.contains('\n') || s.contains('\r')
}

/// Is this gap a **column** — a run of two or more spaces where a rule wants
/// exactly one?
///
/// Spaces only, and two or more of them. A tab is not a column: it is one
/// character, its width depends on where it starts, and nobody builds a
/// column out of something whose width they cannot see.
pub(crate) fn is_column(gap: &str) -> bool {
    gap.len() >= 2 && gap.bytes().all(|b| b == b' ')
}

/// One rule-claimed gap on an output line, as the alignment predicate sees it.
///
/// A *mark* is the boundary a column is built out of: the display column at
/// which the token after the gap begins, and that token's own text. Two lines
/// are aligned when the same token starts at the same column on both — which
/// is what a reader means by a column and what the run's own width does not
/// say (`val font-cjk-gothic   :` and `val f                 :` share no run
/// width at all, and share the `:`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Mark<'s> {
    /// Display column, from the start of the line INCLUDING its indentation,
    /// at which the token after the gap begins.
    pub(crate) col: usize,
    /// That token's own source text.
    pub(crate) text: &'s str,
    /// Was the author's gap here a [`is_column`] run — the thing that is
    /// preserved or collapsed? A single canonical space is a mark too, and it
    /// is what lets a table's WIDEST row (which needs no padding) hold the
    /// column up for its neighbours.
    pub(crate) run: bool,
    /// Which rule-claimed gap on this line this is, counting from zero.
    ///
    /// The **second** witness, and it exists because the first one cannot see
    /// a column of VALUES. `text` equality is the right witness for a column
    /// built on a separator — `val f            : length` and
    /// `val font-cjk-gothic : string` share the `:` — and it is structurally
    /// blind to
    ///
    /// ```text
    ///   |> set-font Kana           font-cjk-mincho
    ///   |> set-font HanIdeographic font-cjk-mincho
    ///   |> set-font Latin          font-latin-roman
    /// ```
    ///
    /// where the aligned token is a different identifier on every row. The
    /// first two rows preserve each other by pure luck (they happen to name
    /// the same font), and the third had nothing to match, so widening
    /// [`Spacing::app_arg`] to every argument boundary flattened the LAST row
    /// of that block in eight corpus files and left the first two padded —
    /// which is worse than either flattening all three or keeping all three.
    ///
    /// So two marks also line up when they are the *same gap of the line*
    /// ending at the same column, which is what a reader means by a column
    /// when the entries differ. It is the conservative direction — a false
    /// positive keeps the author's own bytes — and it reads no more of the
    /// input's layout than `col` already did, so the idempotence argument is
    /// unchanged: the scan lays every line out as if every column survived,
    /// and the second pass reads back the columns the first one wrote.
    pub(crate) idx: usize,
}

/// Which output lines keep the runs the author wrote — the narrowed
/// [`Spacing::preserve_alignment`].
///
/// # The predicate
///
/// A line is a **candidate** when nothing on it was relaid out for a reason
/// other than a column (the coarse rule's whole test) and it carries at least
/// one run mark. A candidate keeps its runs when it has a **witness**: an
/// adjacent line — *L*-1 or *L*+1, no further — that is not itself relaid out
/// and that carries a mark equal to one of this line's marks, the same token
/// text at the same display column, at or past the first run this line has to
/// pad with. Three kinds of witness, and the difference between them is not
/// aesthetic — it is what the second pass depends on:
///
/// - **A**, *mutual*: both lines carry a **run** at that mark. Symmetric, so
///   each is the other's witness and both are preserved.
/// - **B**, *anchored*: the witness line carries **no run at all**. This is
///   the one a two-row table needs, because the WIDEST row of a hand-built
///   column is the one that needed no padding: `| f (x :: xs) = …` holds the
///   `=` column up for `| f []        = …` while having nothing of its own to
///   preserve.
/// - **C**, *chained*: the witness line is one this function has already
///   preserved. `enumitem.satyh:217-220` is the shape — four `match` arms
///   whose `->` line up because `1`, `5` and `_` are right-padded against
///   `10`; only the arm next to `10` has a runless witness, and the other two
///   reach it one row at a time.
///
/// A and B are the base cases and C is the closure, so every preserved line's
/// justification is **grounded** in a line the formatter provably does not
/// move. A line that merely has runs of its own is NOT a witness: it is about
/// to have them collapsed, which would move the very mark it was matched at.
///
/// # Why the second pass agrees
///
/// A run survives into the output only on a preserved line — a collapsed line
/// and a relaid line both write one canonical space — so **the set of lines
/// carrying a run in the output is exactly the set this function returns**.
/// The second pass therefore has the same candidates, and each of them still
/// has its witness: an A or C witness is preserved and so emitted unchanged, a
/// B witness has no run to lose and no rule relaying it out, so it is emitted
/// unchanged too. Same candidates, same marks, same witnesses, same answer.
///
/// The second pass does *gain* witnesses — a collapsed line is runless
/// afterwards, and a relaid line is canonical afterwards — but a witness can
/// only preserve a **candidate**, and the candidate set cannot grow. That
/// asymmetry is what closes the argument: `format(format(x)) == format(x)`
/// holds by construction, not by testing, exactly as it did for the coarse
/// rule that could only copy bytes it had not written.
///
/// # What it deliberately does not reach
///
/// Adjacency is literal. `figbox/doc/manual.saty:533-537` is a real column of
/// three `hconcat ?:align-… [` rows with a `separator;` line between each
/// pair, and this collapses the one row that needed padding. Widening the
/// window is not free — it would have to name a distance, and any distance is
/// a guess — so the cost is taken and `% rustyfi-fmt: off` (`engine.md`
/// section 10) is the escape hatch, as it is for every other column this slice
/// gives up.
pub(crate) fn preserved_lines(relaid: &[bool], marks: &[Vec<Mark<'_>>]) -> Vec<bool> {
    let n = marks.len();
    let live = |i: usize| !relaid.get(i).copied().unwrap_or(false);
    let runless: Vec<bool> = marks.iter().map(|m| !m.iter().any(|k| k.run)).collect();
    // The leftmost run on the line: a run pads what comes AFTER it, so it can
    // only be holding up a column at or past its own end.
    let anchor: Vec<usize> = marks
        .iter()
        .map(|m| {
            m.iter()
                .filter(|k| k.run)
                .map(|k| k.col)
                .min()
                .unwrap_or(usize::MAX)
        })
        .collect();
    let same = |a: &Mark<'_>, b: &Mark<'_>| a.col == b.col && a.text == b.text;
    // Do lines `l` and `o` agree on the column of every rule-claimed gap up
    // to and including the `k`-th?
    //
    // The SECOND witness. `same` is the right one for a column built on a
    // separator and structurally blind to a column of VALUES — see
    // [`Mark::idx`]. A hand-built table is the same run of gaps repeated with
    // padding, so "the k-th gap of both lines lands in the same place, and so
    // does every gap before it" is what a reader is looking at when the
    // entries differ.
    //
    // **Only clause A and the closure consult it, never clause B.** B is the
    // ungrounded case — two lines that are both padded, justifying each other
    // — and the prefix witness would make any run of uniformly padded lines
    // self-justifying, so `|  1 -> …` / `|  5 -> …` / `|  _ -> …` with no
    // runless row among them would never canonicalise again
    // (`a_column_reaches_its_far_rows_one_neighbour_at_a_time`'s control is
    // exactly that file). A still requires a row whose spacing the formatter
    // would not move; all this widens is what counts as lining UP with one.
    //
    // The equal COUNT is not decoration either, and neither is the prefix. `idx` and `col` alone preserved
    // `val get-conclusion : t ->  math` — a typo whose second space happens
    // to put its type where the NEXT line's `t list` starts — and
    // `let h-straight =  halflen …` for the same kind of coincidence. Both
    // lines disagree about an EARLIER gap, so both are rejected by the
    // prefix and collapse, while the `set-font` block agrees about every gap
    // and survives. Without the count, a match arm's `|  _ -> 0x003F` lines
    // up with the `in f 1` under it — one claimed gap each, both landing at
    // column 5 — and grounds a whole ungrounded table on a coincidence
    // between two lines that share no structure at all. A column is the same
    // row SHAPE repeated with padding, so the shapes have to match.
    let prefix = |l: usize, o: usize, k: usize| {
        marks[l].len() == marks[o].len()
            && marks[l].len() > k
            && (0..=k).all(|i| marks[l][i].col == marks[o][i].col)
    };
    let neighbours = |l: usize| [l.checked_sub(1), (l + 1 < n).then_some(l + 1)];
    // Does line `l` line up with line `o` at or past `l`'s first run, given
    // that `o`'s marks are ones the formatter will not move?
    let aligns = |l: usize, o: usize| {
        marks[o].iter().any(|m| {
            m.col >= anchor[l]
                && (marks[l].iter().any(|k| same(k, m)) || prefix(l, o, m.idx))
        })
    };
    // Clause B's witness test, and the widening the matrices needed: `o`
    // grounds a mark at column `c` when it carries **no run at or before
    // `c`** — not only when it carries none at all.
    //
    // The old form was the special case `first_run[o] == usize::MAX`, and it
    // could not ground a table whose padding ALTERNATES between columns:
    //
    // ```text
    //   |      \frac{d}{ad - bc} | \neg \frac{b}{ad - bc}
    //   | \neg \frac{c}{ad - bc} |      \frac{a}{ad - bc}
    // ```
    //
    // (`azmath/doc/azmath.saty:835`, and it is a real matrix a reader is
    // looking at). Neither row is runless, so clause B declined both; clause
    // A wants a RUN at the same mark on both and each row's run is in the
    // other's unpadded column, so it declined too. But row 2's first-column
    // `\frac` sits where it does with no padding at all — every byte before
    // it is canonical — so it is exactly as good a witness for that column as
    // a wholly runless line would be.
    //
    // Groundedness survives, and so does idempotence: a mark at a column
    // BEFORE `o`'s first run is at that column whether or not `o`'s runs are
    // collapsed, so the second pass reads the same witness. The documented
    // counter-case still refuses — `|  1 -> …` / `|  5 -> …` / `|  _ -> …`
    // carry their run in the FIRST mark, so every mark at or past a
    // neighbour's anchor is also at or past the witness's own run, and no row
    // grounds another (`a_column_reaches_its_far_rows_one_neighbour_at_a_time`
    // owns the control).
    let grounds = |o: usize, c: usize| !marks[o].iter().any(|k| k.run && k.col <= c);
    let mut out = vec![false; n];
    // A and B: the grounded witnesses.
    for l in 0..n {
        if !live(l) || runless[l] {
            continue;
        }
        out[l] = neighbours(l).into_iter().flatten().any(|o| {
            live(o)
                && (marks[o].iter().any(|m| {
                    m.col >= anchor[l]
                        && grounds(o, m.col)
                        && (marks[l].iter().any(|k| same(k, m)) || prefix(l, o, m.idx))
                }) || marks[o]
                    .iter()
                    .any(|m| m.run && marks[l].iter().any(|k| k.run && same(k, m))))
        });
    }
    // C: the closure, as a worklist so a long table costs one visit per row
    // rather than one sweep per row.
    let mut work: Vec<usize> = (0..n).filter(|&l| out[l]).collect();
    while let Some(o) = work.pop() {
        for l in neighbours(o).into_iter().flatten() {
            if out[l] || !live(l) || runless[l] || !aligns(l, o) {
                continue;
            }
            out[l] = true;
            work.push(l);
        }
    }
    out
}

/// How many line terminators `s` holds, counting a CRLF as one.
///
/// The output-line counter [`Spacing::preserve_alignment`] indexes by. It has
/// to agree between the scan pass and the emit pass, which it does for free:
/// no rule can add or remove a line break, so both passes see the same
/// terminators in the same order.
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

/// Does the gap the walk is **about to emit** hold a blank line?
///
/// A lookahead rather than a look-back, because the rules that read the
/// author's blank lines have to decide *before* the gap is emitted — a
/// [`Br::Hard`] is a request, and [`Build::gap_upto`] is what turns it into
/// bytes. `byte` is the end of the last atom emitted and `atoms[cursor]` is
/// the next one, so the gap is exactly the span between them; that is the same
/// range `gap_upto` will read, which is what makes the predicate and the
/// output the same proposition.
///
/// "Blank line" is two or more terminators: one ends the previous line, and
/// every further one is a blank. A CRLF counts once
/// ([`count_terminators`]). Shared by both builders.
pub(crate) fn blank_line_in_gap(source: &str, atoms: &[Atom], cursor: usize, byte: usize) -> bool {
    let Some(next) = atoms.get(cursor) else {
        return false;
    };
    let start = next.span.start.byte;
    let Some(gap) = source.get(byte..start) else {
        return false;
    };
    count_terminators(gap) >= 2
}

/// Which of the two walks [`build`] runs this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Pass {
    /// Answer, for each output line, "did a rule change anything on it other
    /// than a column?". The `Doc` this pass produces is thrown away.
    Scan,
    /// Emit, consulting that answer.
    Emit,
}

/// Build a `Doc` for a parsed 0.0.6 file.
///
/// `indent` is columns per depth step (`CstOptions::tab_spaces`).
///
/// `None` declines: an unsupported mode, an atom stream whose spans do not
/// tile and advance, or a gap holding something [`trivia::classify`] refuses —
/// each of which means this code has misread the stream, and
/// `format.rs:336-343`'s reflex applies.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build<'s>(
    source: &'s str,
    atoms: &[Atom],
    file: &cst::File,
    indent: usize,
    breaks: Breaks,
    wrap: Option<usize>,
    wrap_inline: bool,
    max_blank_lines: usize,
) -> Option<Doc<'s>> {
    let mut b = Build {
        source,
        atoms,
        cursor: 0,
        byte: 0,
        frames: vec![Frame::nest(0)],
        at_line_start: true,
        indent: indent as i32,
        wrap,
        wrap_inline,
        desync: 0,
        broken: false,
        rules: SLICE2,
        breaks,
        max_blank_lines,
        in_type_chain: false,
        br: None,
        areas: SLICE4,
        held: None,
        inline_depth: 0,
        area: Area::Program,
        #[cfg(test)]
        census: None,
        space: None,
        last_text: "",
        pass: Pass::Scan,
        line: 0,
        relaid: Vec::new(),
        marks: Vec::new(),
        preserve: Vec::new(),
        col: 0,
        owed_indent: false,
    };
    // Pass 1: for each output line, was anything but a column relaid out on
    // it, and where are its rule-claimed gaps? Only
    // [`Spacing::preserve_alignment`] consults the answer, so the walk is
    // skipped entirely when it is off.
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
    // Pass 2, from a clean slate. Same input, same tree, same rules — so the
    // same line breaks in the same order, which is what makes `preserve`'s
    // indices mean the same thing in both.
    b.reset(Pass::Emit, preserve);
    b.file(file);
    // Anything the walk did not reach — there should be nothing, and the
    // desync counter is what says so, but losing a token's bytes because a
    // grammar arm was missed is not a failure mode this code gets to have.
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
/// formatter, which cannot act on it: a drift changes *indentation*, and the
/// token stream is identical either way — so nothing the verifier or the sweep
/// checks would notice. The invariant has to be asserted from outside.
pub(crate) fn walk_desync(source: &str, atoms: &[Atom], file: &cst::File, indent: usize) -> usize {
    let breaks = SLICE3;
    let max_blank_lines = DEFAULT_MAX_BLANK_LINES;
    let mut b = Build {
        source,
        atoms,
        cursor: 0,
        byte: 0,
        frames: vec![Frame::nest(0)],
        at_line_start: true,
        indent: indent as i32,
        wrap: None,
        wrap_inline: false,
        desync: 0,
        broken: false,
        rules: SLICE2,
        breaks,
        max_blank_lines,
        in_type_chain: false,
        br: None,
        areas: SLICE4,
        held: None,
        inline_depth: 0,
        area: Area::Program,
        #[cfg(test)]
        census: None,
        space: None,
        last_text: "",
        pass: Pass::Scan,
        line: 0,
        relaid: Vec::new(),
        marks: Vec::new(),
        preserve: Vec::new(),
        col: 0,
        owed_indent: false,
    };
    b.file(file);
    b.desync
}

/// Every gap the walk sees, for the census that scoped the inverted default.
///
/// Measurement, not formatting: nothing in [`build`] reads it, and the
/// formatter always passes `census: None`. It lives here rather than in a
/// test because the exception list below is justified by counts, and a count
/// nobody can re-take is a number that quietly goes stale — the same reason
/// `crates/rustyfi/tests/xver_sweep.rs` exists. `build006.rs`'s
/// `the_program_area_gap_census` prints the table.
#[cfg(test)]
pub(crate) fn gap_census(source: &str, atoms: &[Atom], file: &cst::File) -> Vec<Obs> {
    let breaks = SLICE3;
    let max_blank_lines = DEFAULT_MAX_BLANK_LINES;
    let mut b = Build {
        source,
        atoms,
        cursor: 0,
        byte: 0,
        frames: vec![Frame::nest(0)],
        at_line_start: true,
        indent: 2,
        wrap: None,
        wrap_inline: false,
        desync: 0,
        broken: false,
        rules: SLICE2,
        breaks,
        max_blank_lines,
        in_type_chain: false,
        br: None,
        areas: SLICE4,
        held: None,
        inline_depth: 0,
        area: Area::Program,
        census: Some(Vec::new()),
        space: None,
        last_text: "",
        pass: Pass::Emit,
        line: 0,
        relaid: Vec::new(),
        marks: Vec::new(),
        preserve: Vec::new(),
        col: 0,
        owed_indent: false,
    };
    b.file(file);
    while b.cursor < b.atoms.len() {
        b.emit_atom();
    }
    b.census.take().unwrap_or_default()
}

/// Is this expression a `let … in` spine node — one of the six shapes
/// [`Build::expr`] walks iteratively?
fn is_spine(e: &ast::Expr) -> bool {
    use ast::Expr as E;
    matches!(
        e,
        E::LetIn { .. }
            | E::LetRecIn { .. }
            | E::LetPatternIn { .. }
            | E::LetMutableIn { .. }
            | E::LetMathIn { .. }
            | E::OpenIn { .. }
    )
}

/// Does this argument run end in an argument that should be **hugged** —
/// emitted outside the run's frame, with no break point in front of it?
///
/// This is `engine.md` section 12's example (6) as a predicate.
/// `itemize.satyh:48-52` is
///
/// ```text
///   embed-block-top ctx ((get-text-width ctx) -' parent-indent) (fun ctx ->
///     form-paragraph …
///   )
/// ```
///
/// and `format.rs:96-110` uses it to argue that a bracket counter would be
/// worse than the input, because the last argument's body is indented one step
/// past the CALL with no bracket opened in between. A doc builder reproduces it
/// only if the trailing lambda is *not* inside the argument run's own `Nest` —
/// otherwise the body lands two steps in, and breaking before `(fun` would
/// leave `embed-block-top ctx (…)` alone on a line above a dangling block.
///
/// The predicate is on the argument's SHAPE rather than on its width: a
/// delimited group and a text area both have an interior that can absorb the
/// break, and nothing else does. An `?:`-labelled optional never hugs — the
/// sigil binds it to its label and the corpus writes it inline.
fn hugs_last(args: &[ast::AppArg]) -> bool {
    args.last().is_some_and(hugs_arg)
}

/// [`hugs_last`] for one argument, so a command's run — whose first argument
/// is a separate field in the CST ([`ast::CmdTail::Args`]) — can ask the same
/// question.
fn hugs_arg(last: &ast::AppArg) -> bool {
    let atom = match last {
        ast::AppArg::Atom { atom, accesses, .. } if accesses.is_empty() => atom,
        // A `#label` after the group is content on the same line, so the
        // group is no longer the last thing: it cannot absorb the break.
        _ => return false,
    };
    is_delimited(atom)
}

/// Is this atom a delimited group — something with an interior that can absorb
/// a line break of its own?
///
/// The one predicate behind both hugs ([`hugs_last`], [`hugs_body`]). A
/// `Unit` is deliberately absent: `()` has no interior at all.
fn is_delimited(atom: &ast::Atomic) -> bool {
    use ast::Atomic as A;
    matches!(
        atom,
        A::Paren { .. }
            | A::Record { .. }
            | A::List { .. }
            | A::OpenModule { .. }
            | A::InlineText { .. }
            | A::BlockText { .. }
            | A::MathText { .. }
    )
}

/// Would a break before this body only push a delimiter onto the next
/// line?
///
/// [`hugs_last`]'s rule one level up. `let x = (| … |)` whose record does
/// not fit wants
///
/// ```text
///   let x = (|
///     a = 1;
///   |)
/// ```
///
/// and not the same thing with `(|` alone on the second line and every
/// field a step further in. A delimited group absorbs the break itself, so
/// offering one in front of it buys a line and loses two columns.
fn hugs_body(e: &ast::Expr) -> bool {
    let ast::Expr::Ops(c) = e else {
        return false;
    };
    if !c.tail.is_empty() || c.before.is_some() {
        return false;
    }
    let a = &c.head;
    if a.minus.is_some() || a.stage.is_some() || a.excl.is_some() {
        return false;
    }
    match a.args.is_empty() {
        true => a.head_accesses.is_empty() && is_delimited(&a.head),
        // An APPLICATION whose own trailing argument hugs hugs too, and this
        // is the shape every SATySFi document's last binding has:
        //
        //     let document record content = StdJa.document record '<%
        //       …
        //     >%
        //
        // The block text forces a break, so measuring the whole body would say
        // "does not fit" and put `StdJa.document record '<%` on a line of its
        // own — leaving `=` alone above it and every element of the block one
        // step further in. The break the body needs is the one INSIDE the
        // trailing group, and `hugs_last` is what says the trailing group can
        // take it.
        false => hugs_last(&a.args),
    }
}

/// [`hugs_body`] for a `type … =` right-hand side: a bare record type,
/// parenthesised type or command-argument list absorbs its own break.
fn hugs_type_body(b: &cst::TypeDeclBody) -> bool {
    let cst::TypeDeclBody::Synonym(ty) = b else {
        return false;
    };
    let ast::TypeExpr::Atom(prod) = ty else {
        return false;
    };
    if !prod.rest.is_empty() || !prod.first.rest.is_empty() {
        return false;
    }
    use ast::TypeAtom as A;
    matches!(
        prod.first.head,
        A::Paren { .. } | A::Record { .. } | A::RecordOpen { .. } | A::Cmd { .. }
    )
}

/// One open [`Doc::Nest`] / [`Doc::Group`] frame.
///
/// Two nodes rather than one because slice 3 needs them in a fixed order: a
/// group's CLOSING delimiter belongs inside the group (so `fits` measures it
/// — the fencepost that decides whether `(| a = 1 |)` counts as two columns
/// wider than its contents) and outside the nest (so it lands back at the
/// opener's own depth). `exit` renders `Group(mode, Nest(delta, parts))`, so
/// one frame expresses both and a group's interior is one further frame.
pub(crate) struct Frame<'s> {
    pub(crate) parts: Vec<Doc<'s>>,
    /// Indentation steps this frame adds when something inside it breaks.
    pub(crate) delta: i32,
    /// The group mode wrapped *outside* the nest, or `None` for a bare nest.
    pub(crate) group: Option<Mode>,
}

impl<'s> Frame<'s> {
    pub(crate) fn nest(delta: i32) -> Frame<'s> {
        Frame { parts: Vec::new(), delta, group: None }
    }

    pub(crate) fn grouped(mode: Mode) -> Frame<'s> {
        Frame { parts: Vec::new(), delta: 0, group: Some(mode) }
    }
}

struct Build<'s, 'a> {
    source: &'s str,
    atoms: &'a [Atom],
    /// Index of the next atom to emit. The single source of emission order.
    cursor: usize,
    /// End byte of the last atom emitted; the start of the next gap.
    byte: usize,
    /// Open [`Frame`]s, innermost last. `frames[0]` is the file and its delta
    /// is never used.
    frames: Vec<Frame<'s>>,
    /// Whether the output is currently at the start of a line, so the next
    /// line terminator would produce a *blank* line rather than end a
    /// content-bearing one. True at the start of the file, and true after a
    /// token whose own text ends in a terminator — which is what a `@require:`
    /// header is (`lexer.rs:915-933` swallows the line break into the token).
    at_line_start: bool,
    /// Columns per depth step.
    indent: i32,
    /// The budget an own-line `%` comment is reflowed to, or `None` for
    /// "never reflow a comment" See [`super::comment`].
    wrap: Option<usize>,
    /// **Slice 6.** Re-wrap inline text gap by gap, filling to
    /// [`super::CstOptions::max_width`]. See [`super::inline`] for the
    /// predicate that says which gaps may move and [`Build::fill_gap`] for
    /// what a gap that may becomes.
    wrap_inline: bool,
    desync: usize,
    broken: bool,
    /// Which of slice 2's rules are on.
    rules: Spacing,
    /// Which of slice 3's constructs offer the renderer a break.
    breaks: Breaks,
    /// [`super::CstOptions::max_blank_lines`], as the BUILDER sees it.
    ///
    /// The renderer holds the same number and does the actual clamping
    /// ([`super::render::flush_blanks`]); what this copy answers is the one
    /// question the clamp cannot be deferred for — is the cap **zero**, in
    /// which case a rule that reads the author's blank lines must not fire at
    /// all. Deferring that one would cost idempotence: the first pass would
    /// hard-break at a gap the renderer then emptied, and the second pass,
    /// reading a gap with no blank line left in it, would be free to join the
    /// two items back onto one line. See [`Breaks::block_blanks`].
    max_blank_lines: usize,
    /// Is the walk already inside an arrow chain's own group?
    /// [`Build::type_expr`] is the only reader; a delimiter resets it, which
    /// is what [`Build::group_open_as`]'s return value carries.
    in_type_chain: bool,
    /// What the builder wants of the LINE STRUCTURE at the gap before the next
    /// atom, if anything. Set by the walk, consumed by [`Build::gap_upto`],
    /// and it overrides [`Build::space`]'s spelling rather than competing with
    /// it — a break point's flat rendering IS that spelling.
    br: Option<Br>,
    /// Which of slice 4's areas are laid out rather than copied.
    areas: Areas,
    /// A group opener's trailing swallowed trivia, waiting for the group's
    /// frame to be pushed. See [`Subst::SwallowOpen`].
    held: Option<&'s str>,
    /// How many inline-text areas the walk is currently inside.
    ///
    /// One thing consults it, and only since slice 4 made it reachable:
    /// whether a `%` comment may be REFLOWED. A comment that directly abuts a
    /// token in horizontal mode is skipped by `lex_horizontal`'s own `'%'` arm
    /// (`lexer.rs:1106-1109`), which re-takes `start` and so leaves it in a
    /// GAP rather than inside a `Space`/`Break` span — so `easytable.saty`'s
    /// `}% これは …` really does reach [`Build::gap_upto`], and with
    /// `wrap_comments` on it really was being rewrapped inside somebody's
    /// prose. Reflowing there is token-safe (the whole run produces no token
    /// at all) and is still wrong: `engine.md` section 11 lists reflowing
    /// comment text among the non-goals, and inside a text area the reader of
    /// that comment is reading a paragraph.
    inline_depth: usize,
    /// Which area the walk is inside. See [`Area`], and note it is a single
    /// value rather than a depth: areas nest into each other in both
    /// directions, so what matters is the innermost one.
    area: Area,
    /// Where [`gap_census`] collects every gap the walk sees, or `None` —
    /// which is what the formatter itself always passes. Measurement only,
    /// and compiled out of everything but the test binary.
    #[cfg(test)]
    census: Option<Vec<Obs>>,
    /// What slice 2 wants in the gap before the next atom, if anything.
    /// Set by the walk, consumed by [`Build::gap_upto`].
    space: Option<Space>,
    /// The bytes most recently emitted, for [`sep::must_separate`]'s left-hand
    /// side. Not the atom under the cursor minus one: a header emits
    /// *substitute* text, and what matters for fusion is what was written.
    last_text: &'s str,
    /// Which walk this is. See [`Pass`].
    pass: Pass,
    /// Index of the output line being written.
    line: usize,
    /// `relaid[i]`: did a rule change something on output line `i` that was
    /// **not** a column? [`Pass::Scan`] only.
    relaid: Vec<bool>,
    /// `marks[i]`: every rule-claimed gap on output line `i`, as
    /// [`preserved_lines`] wants to see it. [`Pass::Scan`] only.
    marks: Vec<Vec<Mark<'s>>>,
    /// `preserve[i]`: does output line `i` keep the runs the author wrote?
    /// Computed from `relaid` and `marks` between the two walks, read by
    /// [`Pass::Emit`].
    preserve: Vec<bool>,
    /// The display column the output has reached on the line being written.
    ///
    /// A model of [`super::render`]'s own `col`, and it is exact rather than
    /// approximate because slices 1, 2 and 4 emit no [`Doc::Group`] and no
    /// [`Doc::Line`]: nothing the renderer does to this document depends on a
    /// fit decision, so the column of every token is fixed by the moment the
    /// builder emits it. Slice 3 breaks that, and [`preserved_lines`] is the
    /// only reader.
    col: usize,
    /// Whether the current line still owes its indentation, exactly as the
    /// renderer's `pending_indent` does — the indent is written by the first
    /// content byte, so the column it lands at is `level * indent`.
    owed_indent: bool,
}

impl<'s> Build<'s, '_> {
    /// Rewind to the start of the file for the second walk.
    ///
    /// Everything positional goes back to its initial value; `relaid` is the
    /// one thing carried across, which is the whole point of there being two
    /// walks.
    fn reset(&mut self, pass: Pass, preserve: Vec<bool>) {
        self.cursor = 0;
        self.byte = 0;
        self.frames = vec![Frame::nest(0)];
        self.at_line_start = true;
        self.broken = false;
        self.br = None;
        self.space = None;
        self.last_text = "";
        self.pass = pass;
        self.in_type_chain = false;
        self.line = 0;
        self.held = None;
        self.inline_depth = 0;
        self.area = Area::Program;
        self.preserve = preserve;
        self.col = 0;
        self.owed_indent = false;
    }

    /// Does this output line keep the runs the author wrote?
    /// [`Pass::Emit`] only.
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

    /// Emit one content or line-structure node, advancing the column model.
    ///
    /// Every node whose bytes reach the output goes through here; the one
    /// caller that must not is [`Build::exit`], which re-pushes a frame whose
    /// contents were already counted.
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
    /// Transcribed from the renderer's `Doc::Token`/`Doc::Verbatim`,
    /// `VerbatimIndent` and `newline` arms, including the lazy indent: a
    /// terminator OWES the next line its indentation rather than writing it,
    /// so the column a line starts at is `level * indent` read at the moment
    /// the first content lands, not at the moment the line opened.
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
            // Replaces the indent this line is owed, and only then — the same
            // condition the renderer applies.
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
            // **Slice 6 is where this function stops being exact**, and the
            // emit site says so out loud rather than here: the renderer
            // decides at render time whether a fill point is a space or a
            // line break, and the builder has no lookahead to reach the same
            // answer. Modelled as the space, and [`Build::fill_gap`] calls
            // [`Build::mark_relaid`] so that the line carrying one is never
            // used as an alignment witness and never has a column preserved
            // on it — the columns it reports past that point may be wrong by
            // a whole line. Deterministic either way, so idempotence is
            // untouched.
            Doc::FillLine => {
                if self.owed_indent {
                    self.owed_indent = false;
                    self.col = self.level().max(0) as usize * self.indent.max(0) as usize;
                }
                self.col += 1;
            }
            // Counted by the renderer, not emitted; the indent stays owed.
            Doc::BlankLine => {}
            Doc::Nil | Doc::Concat(_) | Doc::Nest(..) | Doc::Group(..) => {}
            // Slices 1, 2 and 4 emit neither, so there is no fit decision to
            // model. Slice 3 must revisit this whole function.
            Doc::Line | Doc::SoftLine => {}
        }
    }

    /// Open a bare nest frame: one indentation step for whatever breaks
    /// inside it.
    ///
    /// **Slice 3 made this unconditional, and that is the whole difference.**
    /// Slice 1's version granted the step to the *first* frame opened on a
    /// source line and none to the rest, because it kept the author's breaks
    /// and a human indents one step past the line a construct started on. With
    /// the breaks decided rather than read there is no such line to be one step
    /// past: a frame whose contents stay flat contributes no line and therefore
    /// no visible indentation, and a frame whose contents break wants its own
    /// step. That is the ordinary Wadler discipline, and it is what makes
    /// `Doc::Nest` mean something a `fits` decision has not already invalidated.
    ///
    /// The case slice 1's rule existed for — `embed-block-top ctx (fun c ->`,
    /// three constructs opened on one line and the corpus indenting the
    /// lambda's body one step past that line rather than three
    /// (`itemize.satyh:48-52`) — survives, but through
    /// [`Build::hugs_last`] rather than through a line count: the trailing
    /// group-shaped argument is emitted OUTSIDE the argument run's frame, so
    /// the lambda's body is one step past the call and not three.
    fn enter(&mut self) {
        self.frames.push(Frame::nest(1));
    }

    /// Current indentation, in steps: the sum of the open frames' deltas.
    fn level(&self) -> i32 {
        self.frames.iter().map(|f| f.delta).sum()
    }

    /// Open a [`Mode`] group frame. Paired with [`Build::exit`].
    ///
    /// Note what is NOT inside it: the gap before whatever comes next has
    /// already been written by the time a caller reaches here, so a group
    /// never swallows its own leading separator and `fits` measures from the
    /// column the group really starts at.
    fn push_group(&mut self, mode: Mode) {
        // Deliberately NOT `mark_inexact`. A group with no break point inside
        // it renders identically flat and broken, so the column model is
        // still exact on its line; what invalidates the model is a `Line`,
        // `SoftLine` or `FillLine`, and those mark it where they are emitted.
        // Marking here as well cost `sig`-block alignment for no reason —
        // a `val … : τ` opens an arrow-chain group whether or not the type
        // has an arrow to break at.
        self.frames.push(Frame::grouped(mode));
    }

    /// Emit a group's opening delimiter, then open its group and interior
    /// frames and offer a break just inside it.
    ///
    /// The shape, and every part of it is load-bearing:
    ///
    /// ```text
    ///   open  Group(Auto, [ Nest(1, [ Opportunity, contents ]),
    ///                       Opportunity, close ])
    /// ```
    ///
    /// - the opener is OUTSIDE the group, so the group is measured from the
    ///   column its contents actually start at;
    /// - the closer is INSIDE it, so `fits` counts the `|)` that would
    ///   otherwise push a record one column over budget after the decision was
    ///   taken;
    /// - the closer is outside the `Nest`, so a broken group puts it back at
    ///   the opener's own depth rather than one step in — the dangling closer
    ///   that `format.rs:94-118` calls the most visible thing a pure-`Nest`
    ///   printer gets wrong on this corpus.
    ///
    /// The two break points are [`Br::Opportunity`], so their FLAT spelling is
    /// whatever [`Build::default_space`] would have written: `(` and `[` are
    /// tight and get a [`Doc::SoftLine`], `(|` and `'<` are padded
    /// ([`Spacing::bracket_pad`], 90.2% of the corpus) and get a
    /// [`Doc::Line`]. Nothing here decides spacing; it only decides where a
    /// line may end.
    fn group_open<T: Spanned<Span = Span>>(&mut self, open: &T) -> i32 {
        self.group_open_as(open, Subst::None)
    }

    /// [`Build::group_open`] for a delimiter whose span swallowed trivia.
    ///
    /// The return value is vestigial — slice 1 used it to carry the opener's
    /// line indentation to [`Build::group_close`], and the `Nest` shape above
    /// replaces it — but it is kept so the two calls still visibly pair.
    fn group_open_as<T: Spanned<Span = Span>>(&mut self, open: &T, subst: Subst) -> i32 {
        self.leaf_as(open, subst);
        // A delimiter resets the arrow chain: a type inside `( … )`,
        // `(| … |)` or `[ … ] inline-cmd` is a chain of its own, and the
        // caller's is restored by the matching `group_close`. This is what the
        // return value carries now that the `Nest` shape has replaced slice
        // 1's line anchor.
        let outer_chain = std::mem::replace(&mut self.in_type_chain, false);
        match self.breaks.groups {
            true => {
                self.push_group(Mode::Auto);
                self.frames.push(Frame::nest(1));
                self.br(Br::Opportunity);
            }
            // The nest alone: a group nobody may break inside still indents
            // the continuation lines of anything that breaks for another
            // reason — a text area's own terminators, most of all.
            false => self.frames.push(Frame::nest(1)),
        }
        // Now that the frame is open, the opener's swallowed line break can be
        // written: it is the contents' first line, not the opener's.
        if let Some(t) = self.held.take() {
            self.swallowed_part(t, false);
        }
        i32::from(outer_chain)
    }

    /// Open a group whose interior is **not** the renderer's to lay out:
    /// inline text, whose every whitespace run is a token.
    ///
    /// The nest is still opened, because slice 4 re-indents an inline area's
    /// continuation lines and those lines come from [`Doc::HardLine`]s the
    /// area's own `Token::Break`s produce. What is absent is the group and the
    /// two break points: there is no legal place for the renderer to end a
    /// line inside `{ … }` that the author did not already write one at.
    fn area_open_as<T: Spanned<Span = Span>>(&mut self, open: &T, subst: Subst) {
        self.leaf_as(open, subst);
        self.frames.push(Frame::nest(1));
        if let Some(t) = self.held.take() {
            self.swallowed_part(t, false);
        }
    }

    /// Close a group opened by [`Build::group_open`].
    fn group_close<T: Spanned<Span = Span>>(&mut self, anchor: i32, close: &T) {
        self.group_close_as(anchor, close, Subst::None)
    }

    /// [`Build::group_close`] for a delimiter whose span swallowed trivia —
    /// which `}` always does, since the whitespace run before it lives inside
    /// `EHorzGrp` (`lexer.rs:1122-1126`).
    fn group_close_as<T: Spanned<Span = Span>>(&mut self, anchor: i32, close: &T, subst: Subst) {
        self.in_type_chain = anchor != 0;
        // Out of the interior nest first, so the break offered before the
        // closer — and the closer itself — land at the opener's own depth.
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
        // `push_raw`: the frame's contents were counted as they were emitted,
        // and re-counting them here would count every byte twice per open
        // frame.
        self.push_raw(d);
    }

    // -- atoms and gaps ------------------------------------------------------

    /// What the gap before the atom under the cursor should hold, when no
    /// rule has asked for anything — [`Spacing::universal`] and its
    /// exceptions.
    ///
    /// **This is the whole of the spacing policy.** It is asked once, in one
    /// place, about every gap in every area but inline text, and it reads the
    /// two tokens either side rather than the CST site that emitted them. A
    /// shape nobody thought of gets [`Space::One`]; the exceptions are the
    /// named, counted list on [`Spacing`].
    ///
    /// Reading the TOKENS is what makes it total. [`Build::flat`] emits every
    /// pattern and most type expressions with no shape knowledge at all, so
    /// the `(x, y)` of `let f (x, y) = …` reaches this function with a
    /// `LParen` and a `Comma` around it and nothing else — which is exactly
    /// what the rule needs.
    fn default_space(&self) -> Space {
        let prev = self
            .cursor
            .checked_sub(1)
            .and_then(|i| self.atoms.get(i))
            .map(|a| &a.slot);
        let next = self.atoms.get(self.cursor).map(|a| &a.slot);
        default_space(&self.rules, self.area, prev, next)
    }
}

/// [`Build::default_space`]'s body, as a free function so that
/// [`super::build01`] decides every gap by the same rule rather than by a
/// second transcription of it. A fork here would be the shape-list mistake
/// again, one generation at a time.
pub(crate) fn default_space(
    rules: &Spacing,
    area: Area,
    prev: Option<&Token>,
    next: Option<&Token>,
) -> Space {
    // End of file. The gap before `Eoi` is not between two tokens at all,
    // and a space written there is trailing whitespace that
    // [`super::render::finish`] trims — but the SCAN pass would still
    // have recorded the line as relaid out, taking a column on the last
    // line of 38 corpus files down with it.
    if next.is_none() || matches!(next, Some(Token::Eoi)) {
        return Space::Keep;
    }
    // A constructor with a group argument: `Some(x)`, in an expression
    // and in a pattern alike. Asked first, because both of its tokens
    // also match a tight class and the corpus is too evenly split for
    // either answer.
    //
    // [`Space::Collapse`] and not [`Space::Keep`]: the split says do not
    // INSERT, which is a claim about an EMPTY gap, and it says nothing
    // about a RUN. See [`Spacing::ctor_arg`].
    if rules.ctor_arg
        && matches!(prev, Some(Token::Constructor(_)) | Some(Token::LongUpper(_, _)))
        && next.is_some_and(opens_a_group)
    {
        return Space::Collapse;
    }
    // A RECORD or BLOCK delimiter is padded, not tightened, and the corpus is
    // what says so. Counting the same-line boundaries in the 209 bundled and
    // third-party files, BEFORE any of this formatting ran:
    //
    //     (| X     tight    14   spaced  129   ->  90.2% SPACED
    //     X |)     tight    28   spaced   80   ->  74.1% SPACED
    //     '< X     tight    64   spaced   41   ->  61.0% tight
    //
    // The first two are decisive and the tight rule was simply wrong for them:
    // `bracket`'s aggregate was 99.0% canonical across all six delimiter kinds,
    // and `(`/`[` dominate that sample so heavily that the record pair's
    // opposite habit disappeared inside it. An exception measured in aggregate
    // can be wrong about every member of a minority class without the number
    // moving, which is what happened here.
    //
    // `'<` is included on the user's instruction rather than on the count,
    // which runs the other way — and weakly: the regex behind it sees only
    // SAME-LINE boundaries, so the ordinary multi-line `'<` followed by a
    // newline is absent from both columns, leaving a small and unrepresentative
    // sample. A gap holding a break is never rewritten anyway, so the rule
    // reaches only the same-line form either way.
    //
    // An EMPTY group is not padded: `'<>` and `(||)` have nothing inside to put
    // a space beside, and `'< >` would be inventing content-shaped whitespace
    // in a group the author wrote as empty. Caught by
    // `an_empty_group_is_not_padded`, which `'<>` in a real corpus fixture
    // (`read-block ctx '<>`) found before the test did.
    if rules.bracket_pad
        && !matches!(
            (prev, next),
            (Some(Token::BRecord), Some(Token::ERecord))
                | (Some(Token::BVertGrp), Some(Token::EVertGrp))
        )
        && (matches!(prev, Some(Token::BRecord) | Some(Token::BVertGrp))
            || matches!(next, Some(Token::ERecord) | Some(Token::EVertGrp)))
    {
        return Space::One;
    }
    if rules.bracket
        && (prev.is_some_and(opens_a_group) || next.is_some_and(closes_a_group))
    {
        return Space::Tight;
    }
    if rules.separator && next.is_some_and(is_separator) {
        return Space::Tight;
    }
    if rules.access
        && (matches!(prev, Some(Token::Access)) || matches!(next, Some(Token::Access)))
    {
        return Space::Tight;
    }
    if rules.sigil
        && (prev.is_some_and(is_sigil) || matches!(next, Some(Token::OptionalType)))
    {
        return Space::Tight;
    }
    if rules.script && (prev.is_some_and(is_script) || next.is_some_and(is_script)) {
        return Space::Tight;
    }
    match area {
        Area::Program | Area::Active | Area::Inline => Space::One,
        Area::Math | Area::Block => Space::Collapse,
    }
}

impl<'s> Build<'s, '_> {
    /// The boundary between a **command head** and one of its arguments,
    /// which [`Spacing::cmd_arg`] neither tightens nor spaces — it only
    /// collapses a run.
    fn cmd_arg_boundary(&mut self) {
        if self.rules.cmd_arg {
            self.space(Space::Collapse);
        }
    }

    /// Enter `area`, returning the one to restore.
    ///
    /// Always paired, and always around the area's *contents* rather than
    /// around its delimiters: the gap in front of a `${` belongs to the
    /// program text that wrote it, and the gap in front of the matching `}`
    /// belongs to the math.
    fn enter_area(&mut self, area: Area) -> Area {
        std::mem::replace(&mut self.area, area)
    }

    /// Ask for a particular gap before the **next** atom emitted.
    ///
    /// A request, not a write: [`Build::gap_upto`] decides whether it applies.
    /// It survives any number of frame pushes in between, which is what lets
    /// `record_field` ask for the space before a value that
    /// [`Build::body`] has not opened a frame for yet.
    fn space(&mut self, want: Space) {
        self.space = Some(want);
    }

    /// Ask for a break opportunity in the gap before the **next** atom.
    ///
    /// Independent of [`Build::space`] and asked at the same sites, because a
    /// break point has no spelling of its own: what it renders flat is
    /// whatever the spacing policy would have written there. The two are
    /// resolved together in [`Build::gap_upto`].
    fn br(&mut self, want: Br) {
        self.br = Some(want);
    }

    /// A line ends here because the construct is a **block**: one top-level
    /// binding, one `sig` item, one `struct` binding per line, whatever the
    /// width.
    ///
    /// The [`Br::Hard`] positions are exactly the positions the author's blank
    /// lines survive at, and that is not a coincidence — `engine.md` section
    /// 6's class 5 admits a clamped count only *between two anchors the printer
    /// reproduces*, and a hard break is the only thing this builder emits that
    /// the second pass is guaranteed to read back as a break.
    fn block_break(&mut self) {
        if self.breaks.blocks {
            self.br(Br::Hard);
        }
    }

    /// A line ends here because the construct is a **clause list**: a `match`
    /// arm, a `let-rec` clause. `Mode::Break` expressed as a break rather than
    /// as a group mode, because there is nothing else in the group to decide.
    fn clause_break(&mut self) {
        if self.breaks.clauses {
            self.br(Br::Hard);
        }
    }

    /// A break opportunity between two items of a `;`/`,`-separated run —
    /// record fields, list items, tuple components — belonging to the run's
    /// own [`Mode::Auto`] group, so they break together or not at all.
    ///
    /// `n` is the item's index: the first item's break point is the one
    /// [`Build::group_open`] already offered just inside the delimiter, and
    /// offering a second there would be a doubled space in flat mode.
    fn item_break(&mut self, n: usize) {
        if n > 0 && self.breaks.items {
            self.br(Br::Opportunity);
        }
    }

    /// Note that the line being built is **not** one the builder lays out
    /// exactly, so no column on it may be preserved and it may not witness a
    /// neighbour's.
    ///
    /// The whole of slice 3's answer to [`Spacing::preserve_alignment`], and
    /// the reason it is a marking rather than a switch is in
    /// [`preserved_lines`]'s own terms: alignment is preserved by *modelling*
    /// the output column of every gap on a line ([`Build::advance`]), and that
    /// model is exact only while nothing on the line is a [`Doc::Group`] or a
    /// break point — because those are decided by [`super::render`] afterwards,
    /// from a width the builder has not got. A line the renderer may re-wrap
    /// therefore loses its column, and a line laid out entirely by the builder
    /// keeps it. That is not a compromise between the two rules; it is the
    /// largest domain on which the column model is still true.
    ///
    /// What survives in practice is the `sig` block —
    /// `val font-cjk-gothic   : string * float * float` has no group and no
    /// break point anywhere on it — and what dies is the `match` arm and the
    /// `let-rec` clause, whose bodies are groups. Both halves are pinned.
    fn mark_inexact(&mut self) {
        self.mark_relaid();
    }

    /// Emit the gap ending at `start`.
    ///
    /// The one place a gap's bytes are chosen, and — since slice 3 — the one
    /// place its LINE STRUCTURE is chosen too. Four cases, in order:
    ///
    /// 1. **The gap holds a `%` comment.** Copied verbatim with its own line
    ///    structure, every request dropped. Rewriting it would delete a
    ///    comment; and the [`Doc::HardLine`] it brings is what forces its
    ///    enclosing group open, so a comment is never measured in flat mode
    ///    (`engine.md` section 6, hazard class 3).
    /// 2. **The builder asked for a hard break here** ([`Br::Hard`]). One
    ///    line ends. This is the ONLY position at which the author's blank
    ///    lines survive — hazard class 5's clamped count between two anchors,
    ///    admitted exactly where the second pass provably reads back what the
    ///    first wrote, because the first wrote a break there unconditionally.
    /// 3. **The gap is inside `${ … }` and holds a break.** Math keeps the
    ///    author's line structure exactly — the formatter neither invents a
    ///    break there nor removes one. The case comment below carries why this
    ///    is not hazard class 1.
    /// 4. **The output is at the start of a line.** The gap is that line's
    ///    indentation and the renderer writes it, from the nesting.
    /// 5. **Everything else.** The gap's own line breaks are **discarded**
    ///    and what replaces them is a canonical space, a
    ///    [`Doc::Line`]/[`Doc::SoftLine`] or a [`Doc::FillLine`], according to
    ///    what the construct asked for and what the spacing policy spells.
    ///
    /// Case 5 is the whole of `LineBreaks::Preserve`'s removal. Slice 1 read
    /// the author's break out of the gap and reproduced it; there is no branch
    /// here that can, and `engine.md` section 6's hazard class 1 is therefore
    /// excluded structurally rather than by a switch nobody may flip.
    fn gap_upto(&mut self, start: usize) {
        // Taken unconditionally: a request is for the gap before *this* atom,
        // and must not leak onto the next one if this gap declines it.
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
        if self.pass == Pass::Emit {
            if let Some(c) = self.census.as_mut() {
                c.push(Obs {
                    at: self.cursor,
                    span: (self.byte, start),
                    area: self.area,
                    want,
                    has_break: !no_break,
                    blank,
                    at_line_start: self.at_line_start,
                });
            }
        }
        // 1. A comment owns its own line structure, wherever it sits.
        if commented {
            let reflow = self.inline_depth == 0;
            self.break_structure(pieces, false, reflow);
            return;
        }
        // 2. A break the BUILDER asked for, and the author's blank lines with
        //    it.
        if br == Some(Br::Hard) {
            match (no_break, self.at_line_start) {
                // The gap's own terminators, so its BLANK lines come with
                // them: `break_structure` reads the first one as a `HardLine`
                // or a `BlankLine` according to whether a line is already
                // open, which is the same question this arm is asking.
                (false, _) => self.break_structure(pieces, false, false),
                (true, false) => {
                    self.line += 1;
                    self.push(Doc::HardLine);
                }
                // Already at the start of a line, and the gap holds nothing:
                // a `HardLine` here would be a blank line nobody wrote. This
                // is what a `@require:` header leaves behind — `lex_header`
                // swallows the terminator into the token
                // (`lexer.rs:915-933`), so the gap after it is EMPTY and the
                // line is already open.
                (true, true) => {}
            }
            self.at_line_start = true;
            return;
        }
        // 3. **Math keeps the author's line structure**, and this is the one
        //    place slice 3 reads input layout outside a `Br::Hard` position.
        //
        //    [`Build::math_text`] says the formatter never INVENTS a break
        //    inside `${ … }`; joining the ones the author wrote was that same
        //    decision left half-made, and the two halves are not separable —
        //    "atomic" has to mean the line structure is neither added to nor
        //    taken away, or a hand-laid-out equation comes back as one long
        //    line nobody can break again.
        //
        //    **Why this is not `engine.md` section 6's hazard class 1.** The
        //    hazard is a rule that both READS the author's breaks and
        //    OVERRIDES some of them, which cannot be a fixpoint. Here the two
        //    regimes are disjoint by AREA, and an area is a property of the
        //    TREE — which delimiters you are inside — not of the layout:
        //    inside `${ … }` the builder offers no break point at all and
        //    reproduces exactly what it read, so the second pass reads back
        //    what the first wrote; outside it every break is decided and
        //    nothing is read. `LineBreaks::Preserve` was dangerous because it
        //    applied EVERYWHERE, so slice 3 had to override part of it.
        //
        //    It is also what makes math consistent with the other two text
        //    areas rather than the odd one out. Inline text's whitespace runs
        //    are TOKENS, so [`Build::emit_swallowed`] has always reproduced
        //    their break structure; block text is a `+cmd …;` STATEMENT
        //    SEQUENCE and is re-broken on purpose. A `${ … }` is one
        //    expression. This is `engine.md` section 4's
        //    `AreaPolicy::Reindent` for math.
        if !no_break && self.area == Area::Math {
            self.break_structure(pieces, false, false);
            return;
        }
        // 4. Indentation, which the renderer owns.
        if self.at_line_start {
            return;
        }
        // 5. A break opportunity, or a canonical space, in place of whatever
        //    the author wrote — line breaks included. Outside a text area
        //    there is no branch here that reproduces an author's break.
        let want = match (want, self.rules.universal && self.area.claimed()) {
            (Some(w), _) => w,
            (None, true) => self.default_space(),
            (None, false) => Space::Keep,
        };
        let spelling = self.flat_spelling(want, gap, no_break);
        match (br, spelling) {
            // A fill point whose flat spelling is not a space is not a fill
            // point: `Some(x)`'s argument boundary is EMPTY and a break there
            // would render `Some (x)` flat and `Some⏎(x)` broken, neither of
            // which anybody writes.
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
                // The author's own bytes are still on the table when they hold
                // no break: this is where `preserve_alignment` and the
                // `Space::Keep` exception live.
                true => self.canonical_space(want, gap),
                // A gap the author broke and nobody offered a break at: the
                // break is dropped and the canonical spelling put in its
                // place. `flat_spelling` has already asked
                // [`sep::must_separate`] about the pair, because the
                // "an empty gap is a PROOF the two do not fuse" shortcut
                // `canonical_space` relies on is not available here — the gap
                // was not empty, it held a newline.
                false => match spelling {
                    Flat::Space => self.push(Doc::Verbatim(" ")),
                    // Includes `Space::Keep` before `Eoi`, whose bytes
                    // `super::render::finish` would trim anyway.
                    Flat::Empty | Flat::Keep => {}
                },
            },
        }
    }

    /// What the spacing policy would write in this gap if the line did not
    /// break here — the flat rendering of a break point, and the answer that
    /// says whether one may be offered at all.
    ///
    /// `no_break` is what decides whether the empty-gap shortcut is available.
    /// [`Build::canonical_space`] treats an empty gap as a **proof** that the
    /// two ranges do not fuse — the author wrote them adjacent and the lexer
    /// read two tokens — and that proof does not extend to a gap that held a
    /// newline, because there the author never wrote them adjacent at all. So
    /// a joined gap asks [`sep::must_separate`] where a tight one may not.
    fn flat_spelling(&self, want: Space, gap: &str, no_break: bool) -> Flat {
        let separate = || {
            let next = self
                .atoms
                .get(self.cursor)
                .and_then(|a| {
                    let len = self.source.len();
                    self.source
                        .get(a.span.start.byte.min(len)..a.span.end.byte.min(len))
                })
                .unwrap_or("");
            sep::must_separate(self.last_text, next)
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

    /// Reproduce a run of trivia's **break structure**, dropping every indent
    /// in it so the renderer supplies one from the enclosing [`Doc::Nest`]
    /// chain — bar an own-line comment's, which is the author's
    /// ([`Build::own_line_comment`]).
    ///
    /// Split out of [`Build::gap_upto`] because slice 4 has a second caller
    /// that is not a gap at all: the trivia a horizontal-mode token
    /// **swallowed into its own span** ([`Build::emit_swallowed`]).
    ///
    /// `keep_first_space` is that caller's one extra requirement, and it is
    /// the whole of inline text's safety. A `Token::Space` and a
    /// `Token::Break` differ only in whether their run's FIRST character is a
    /// newline (`lexer.rs:1149-1155`), so dropping the leading horizontal
    /// whitespace of `"  \n  "` would turn a `Space` into a `Break` — a
    /// different token, a different typeset line (measured: I25 in
    /// `ground-truth-whitespace.md` DIFFERS for CJK), and a verifier decline.
    /// With the flag the run keeps exactly **one** space there, which is
    /// canonical rather than copied and therefore still a fixpoint: the
    /// second pass reads back one space and writes one space.
    fn break_structure(
        &mut self,
        pieces: Vec<Piece<'s>>,
        keep_first_space: bool,
        reflow_comments: bool,
    ) {
        let mut pending_space: Option<&'s str> = None;
        // Whether the *next* piece would begin a line. Not `self.at_line_start`
        // alone, because it is also what distinguishes the first terminator in
        // this run from a blank-line one.
        let mut line_start = self.at_line_start;
        let mut first = true;
        for p in pieces {
            match p {
                // Held: whether it survives depends on what follows it. Before
                // a terminator it is trailing whitespace and dies; before a
                // comment on a content line it is the author's spacing and
                // lives verbatim; before a comment on a line of its own it is
                // that line's *indentation*, which is what this slice owns.
                Piece::Space(s) => match first && keep_first_space {
                    true => {
                        self.push(Doc::Verbatim(" "));
                        self.last_text = " ";
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
        // A trailing run of spaces is the next line's indentation. Dropped:
        // the renderer writes it, from the nesting rather than from the input.
    }

    /// Write the gap a rule asked for, in place of the one the author wrote.
    ///
    /// `gap` is guaranteed to be spaces and tabs only (possibly empty), and
    /// the output is not at the start of a line — both checked by
    /// [`Build::gap_upto`], because the whole safety of this function is that
    /// there is nothing in the range it could destroy.
    ///
    /// # The fusion hazard, and which half of it is real
    ///
    /// [`Space::One`] cannot corrupt anything: whitespace between two copied
    /// ranges only ever *separates* them, so a token can be split by it but
    /// never joined — and it cannot be split either, because the two ranges
    /// are whole atoms with a gap already between them in the CST.
    ///
    /// [`Space::Tight`] is the direction that can, which is why it asks
    /// [`sep::must_separate`] rather than a second hand-rolled rule. That
    /// function was validated exhaustively — every ordered pair of the 11,770
    /// distinct token spellings in the corpus, 5.59 M pairs where it answers
    /// `false`, none of which fuses — and a second opinion here would be a
    /// second thing to get wrong. `;` fuses with nothing (it is neither a word
    /// character nor one of `lexer.rs`'s `is_opsymbol`), so today the call
    /// always answers `false`; it is here for the next rule, and because a
    /// tight rule that did not ask would be indistinguishable from one that
    /// asked and got lucky.
    fn canonical_space(&mut self, want: Space, gap: &'s str) {
        match want {
            Space::One => {
                let column = self.rules.preserve_alignment && is_column(gap);
                match self.pass {
                    // Anything that is NOT a column and is not already the one
                    // space this rule wants is a relayout of this line.
                    //
                    // The scan also lays the line out AS IF every column
                    // survived, which is what makes `self.col` the column a
                    // preserved run would really land at — and the columns of
                    // a line that ends up collapsed are never read
                    // ([`preserved_lines`], clause 4).
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
            // Empty stays empty, everything else becomes one space. Written
            // as a delegation rather than a third copy of the two arms below
            // so that a change to either reaches it.
            Space::Collapse => {
                let want = match gap.is_empty() {
                    true => Space::Keep,
                    false => Space::One,
                };
                self.canonical_space(want, gap);
            }
            Space::Keep => {
                // The author's own bytes. NOT a mark and NOT a relayout: a
                // gap this rule declines to touch is one no column argument
                // has to reason about, exactly as an unclaimed gap was
                // before the default was inverted.
                if !gap.is_empty() {
                    self.push(Doc::Verbatim(gap));
                }
            }
            // An EMPTY gap is already tight, and it is also a PROOF: the
            // author wrote these two ranges adjacent and the lexer read them
            // as the two tokens we are looking at, so they demonstrably do
            // not fuse. Asking the table there is not conservative, it is
            // wrong — `${\int_1^2}` came back as `${\int _ 1^2}`, a space
            // INSERTED into a file that lexed fine without it, because `_` is
            // a word character in program mode and the table is mode-blind.
            // The table is still the only authority for the direction that
            // can corrupt: REMOVING a space the author wrote.
            Space::Tight if gap.is_empty() => {
                if self.pass == Pass::Scan {
                    self.note_mark(0, false);
                }
            }
            Space::Tight => {
                let next = self
                    .atoms
                    .get(self.cursor)
                    .and_then(|a| {
                        let len = self.source.len();
                        self.source
                            .get(a.span.start.byte.min(len)..a.span.end.byte.min(len))
                    })
                    .unwrap_or("");
                let separate = sep::must_separate(self.last_text, next);
                let canonical = match separate {
                    true => " ",
                    false => "",
                };
                // A tight rule never preserves a column: it wants NOTHING, and
                // two spaces in front of a `;` or a `)` is not a column
                // anybody built. So any difference is a relayout — and the
                // mark it leaves is never a run, which is exactly what makes
                // it usable as a witness for a NEIGHBOUR's column.
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

    /// A `%` comment that sits on a line of its own, with whatever leading
    /// whitespace the author gave it.
    ///
    /// **Variant B: the author's indentation is kept.** The alternative —
    /// re-indenting the comment to the enclosing block's depth, which is what
    /// the first draft of this file did — was measured against the whole
    /// corpus and is the larger diff either way round:
    ///
    /// ```text
    ///                                   files changed   diff lines
    ///   A, re-indent                        127            14,977
    ///   B, keep the author's                125            13,723
    ///   A, with `body`/`element` fixed       124            14,523
    ///   B, with `body`/`element` fixed       122            13,289
    /// ```
    ///
    /// 1,234 lines and two whole files, and every one of the biggest *indent*
    /// moves in the corpus was A pulling `%`-disabled code in from column 0 by
    /// up to eight columns. B is also the better of the two on the merits: a
    /// block of commented-out code parked at column 0 was parked there
    /// deliberately, and moving it asserts a scope membership the author
    /// declined to give it. A comment the author already put at block depth
    /// keeps that depth for free, since keeping it is now the rule.
    ///
    /// This is the one place slice 1 does not re-indent a line, and
    /// [`Doc::VerbatimIndent`] is why it is still a fixpoint: the bytes
    /// written are the bytes read.
    ///
    /// It also retires the gap the first draft recorded — "a comment on its
    /// own line immediately before `end` or `in` is emitted in the enclosing
    /// frame, so it dedents to the outer depth". Such a comment is no longer
    /// emitted at *any* frame's depth, so which frame it landed in has stopped
    /// being observable.
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

    /// Emit the atom under the cursor, and the gap before it, optionally
    /// writing *substitute* bytes for the atom itself.
    ///
    /// `Subst::None` is the invariant every other slice relies on: a leaf's
    /// bytes are the source's bytes. [`Subst::Header`] is the one exception in
    /// the design, argued at [`Build::header_leaf`]; it is threaded through
    /// here rather than around this function so that a substituted leaf still
    /// advances the cursor, still updates `byte`, and still passes through
    /// [`Build::gap_upto`] — the three things a bypass would quietly skip.
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
                // Read off the ATOM's own bytes, not the substitute's: a
                // header rewrite only ever drops spaces after the colon, so
                // the two agree, and reading the source keeps this line
                // honest for any substitute a later slice adds.
                self.line += count_terminators(text);
                self.at_line_start = text.ends_with('\n') || text.ends_with('\r');
                match subst {
                    // Handled above, before the generic bookkeeping.
                    Subst::Swallow | Subst::SwallowOpen | Subst::None => {
                        self.push(Doc::Token {
                            text,
                            atom: self.cursor,
                        });
                        self.last_text = text;
                    }
                    Subst::Header if self.rules.header => self.push_header(text),
                    Subst::Header => {
                        self.push(Doc::Token {
                            text,
                            atom: self.cursor,
                        });
                        self.last_text = text;
                    }
                }
            }
            Some(_) => {}
            None => self.broken = true,
        }
        self.byte = end;
        self.cursor += 1;
    }

    /// `@name:` + one space + the rest, in place of the header's own bytes.
    ///
    /// Emitted as three [`Doc::Verbatim`] pieces rather than one
    /// [`Doc::Token`], because none of the three IS the token — `Doc::Token`'s
    /// contract is "one token's own source bytes" and only the whole span
    /// satisfies it. Nothing is lost by the demotion: `Doc::Token`'s only
    /// extra power is being a [`sep::must_separate`] candidate, and a header
    /// can never be one. On its right it ends in its own line terminator
    /// (`lexer.rs:915-933`) unless it is the last line of a file with no final
    /// newline, in which case only the zero-width `Eoi` follows; on its left
    /// sits either the start of the file or another header, which likewise
    /// ends in a terminator.
    fn push_header(&mut self, text: &'s str) {
        let Some(colon) = text.find(':') else {
            // `lex_header` cannot produce one without a colon, so this is
            // unreachable; copy rather than guess.
            self.push(Doc::Verbatim(text));
            self.last_text = text;
            return;
        };
        let (prefix, rest) = text.split_at(colon + 1);
        // ONLY `\x20`, because that is the only character `lex_header` strips
        // into the payload (`lexer.rs:911-914` is `while self.peek() ==
        // Some(' ')`). A tab after the colon is part of the payload, so
        // trimming it would change the token and the verifier would decline.
        let rest = rest.trim_start_matches(' ');
        self.push(Doc::Verbatim(prefix));
        // No space before a terminator (that would be trailing whitespace),
        // none before nothing at all, and none before a character `lex_header`
        // would not have stripped — a tab keeps its own place rather than
        // gaining a space in front of it.
        if !rest.starts_with(|c: char| c.is_whitespace()) && !rest.is_empty() {
            self.push(Doc::Verbatim(" "));
        }
        if !rest.is_empty() {
            self.push(Doc::Verbatim(rest));
        }
        self.last_text = rest;
    }

    /// A horizontal-mode token, with the indentation it swallowed recomputed.
    ///
    /// **Slice 4, inline text's whole mechanism.** Emitted as pieces rather
    /// than as one [`Doc::Token`] — the same demotion [`Build::push_header`]
    /// makes, and for a related reason: none of the pieces IS the token.
    ///
    /// # Why this may edit bytes inside a token's span
    ///
    /// `doc.rs`'s module comment forbids printing a leaf from anything but a
    /// slice of the source, because a token's *payload* is not recoverable
    /// from its spelling for six token kinds. None of them is here. The seven
    /// tokens this reaches — `Space`, `Break`, `BHorzGrp`, `EHorzGrp`,
    /// `BVertGrp`, `Item`, `Sep` — carry either **no payload at all** or a
    /// payload (`Item`'s bullet count) that is read off the `core` this copies
    /// verbatim. What moves is only trivia the lexer had already decided to
    /// throw away, and which it happened to throw away *inside* a span rather
    /// than into a gap.
    ///
    /// So `same_tokens` — slots and payloads, never spans — is not weakened to
    /// let this through: it passes on the nose, and it is the backstop for the
    /// one mistake this function could make.
    ///
    /// # The one mistake
    ///
    /// `Space` and `Break` are the same run of characters and differ only in
    /// whether the FIRST one is a newline (`lexer.rs:1152`, `if
    /// is_break(first)`). Dropping the leading horizontal whitespace of
    /// `"  \n  "` turns a `Space` into a `Break`, which is a different token
    /// and — for CJK, where a space, a newline and nothing are three different
    /// typeset results — a different document (I25 in
    /// `ground-truth-whitespace.md` measures exactly this edit as DIFFERING).
    /// `keep_first_space` is what stops it.
    ///
    /// # And the no-op case, which is nearly all of them
    ///
    /// If neither swallowed run holds a line terminator there is no
    /// indentation to recompute and the token is emitted as its own bytes, so
    /// every single-line inline area comes out byte-identical by construction
    /// rather than by luck.
    fn emit_swallowed(&mut self, text: &'s str, hold_trail: bool) {
        let (lead, core, trail) = split_swallowed(text);
        if !hold_trail && !holds_break(lead) && !holds_break(trail) {
            self.at_line_start = false;
            self.push(Doc::Token {
                text,
                atom: self.cursor,
            });
            self.last_text = text;
            return;
        }
        // `core.is_empty()` is exactly `Space`/`Break`, the only two of the
        // seven whose identity the leading whitespace decides.
        self.swallowed_part(lead, core.is_empty());
        if !core.is_empty() {
            self.push(Doc::Verbatim(core));
            self.last_text = core;
            self.at_line_start = false;
        }
        match hold_trail {
            true => self.held = Some(trail),
            false => self.swallowed_part(trail, false),
        }
    }

    /// **Slice 6**: a whitespace run that the renderer, not the author,
    /// decides the spelling of.
    ///
    /// The atom is consumed exactly as [`Build::emit_swallowed`] would
    /// consume it — cursor advanced, `byte` advanced, the (empty) gap before
    /// it emitted — and a [`Doc::FillLine`] is pushed in place of its bytes.
    /// So a run is never invented and never emptied; only re-spelled, which
    /// is the whole of what [`super::inline`]'s predicate licenses.
    ///
    /// # Why the line is marked relaid
    ///
    /// [`Build::advance`] models a fill point as a space, because it has no
    /// lookahead and cannot know which way the renderer will go. Every column
    /// this builder reports for the rest of the line is therefore a guess, so
    /// the line is marked relaid: [`preserved_lines`]'s `live` excludes it as
    /// an alignment witness AND from having a column preserved on it. That is
    /// the conservative direction — a hand-built column sharing a line with a
    /// re-wrapped inline area collapses rather than being preserved at a
    /// column nobody can compute — and it costs nothing measurable, because a
    /// text area is the last thing on its line in every corpus file that has
    /// one.
    ///
    /// The BUILDER's line numbering is untouched, and that is what
    /// idempotence rests on: a fill point never increments `self.line`, in
    /// either pass and in either application, so `preserve`'s indices mean
    /// the same thing every time. What drifts is only the correspondence
    /// between a builder line and a RENDERED line, which nothing reads.
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
        // One space is what a fill point renders flat, and `last_text` is
        // read only by [`sep::must_separate`], which asks whether two ranges
        // would fuse — across a space they cannot.
        self.last_text = " ";
        self.at_line_start = false;
        self.byte = end;
        self.cursor += 1;
    }

    /// Is the whitespace token under the cursor one slice 6 may re-spell?
    ///
    /// Three questions, and they are deliberately asked in this file rather
    /// than folded into [`super::inline`]: the first two are that module's
    /// (the script on either side, and the run's own bytes), the third is
    /// this slice's own switch.
    fn gap_is_fillable(&self, text: &str) -> bool {
        self.wrap_inline
            && inline::run_bytes_allow_reflow(text)
            && inline::gap_is_reflowable(self.atoms, self.cursor)
    }

    /// A `Space` or `Break` inside inline text: a fill point if slice 6 may
    /// re-spell it, and slice 4's re-indented run if not.
    ///
    /// A **frozen** gap keeps its bytes' break structure exactly — it is not
    /// merely "not filled", it is the same run the author wrote, so a
    /// paragraph that mixes the two comes out with the CJK boundaries where
    /// they were and the Latin ones re-flowed around them.
    fn inline_gap<T: Spanned<Span = Span>>(&mut self, t: &T) {
        let len = self.source.len();
        let text = self
            .atoms
            .get(self.cursor)
            .and_then(|a| self.source.get(a.span.start.byte.min(len)..a.span.end.byte.min(len)))
            .unwrap_or("");
        match self.gap_is_fillable(text) {
            true => {
                if self.atoms.get(self.cursor).map(|a| a.span.start.byte) != Some(t.span().start.byte)
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
            // No line structure in it, so nothing to re-indent: the author's
            // bytes, copied. That keeps `{  a` from silently becoming `{a`,
            // which is invisible but is not what this slice is for.
            self.push(Doc::Verbatim(part));
            self.last_text = part;
            self.at_line_start = false;
            return;
        }
        let Some(pieces) = trivia::classify(part) else {
            // Unreachable: `split_swallowed` cuts on the same alphabet
            // `classify` accepts. Decline rather than guess.
            self.broken = true;
            return;
        };
        // NEVER reflowed. A `%` comment reached from here is INSIDE a
        // `Token::Space`/`Break` span (`trivia.rs`'s module comment, trap 1),
        // which is to say inside inline text — and an inline comment can
        // DELETE a space (`{Alpha% c⏎beta}` sets `Alphabeta`, measured as I14
        // and I15). Rewrapping one onto two lines rewrites prose. Its own
        // indentation is still the author's, as everywhere else.
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

    /// A `Spanned` leaf that is **more than one atom**.
    ///
    /// `OpNameTok` is the case: `( ++ )` parses as three atoms and unparses as
    /// three, while its precomputed `span` covers all of them
    /// (`leaf.rs:494-530`). Emitting it as one atom left the cursor pointing at
    /// the `++` and desynced the rest of the file — 788 mismatches in
    /// `azmath/src/equation.satyh`, from one `List.fold-left (++) inline-nil`.
    fn leaf_multi<T: Spanned<Span = Span> + Unparse<Atom>>(&mut self, t: &T) {
        if self.atoms.get(self.cursor).map(|a| a.span.start.byte) != Some(t.span().start.byte) {
            self.desync += 1;
        }
        self.flat(t);
    }

    /// A header line, re-emitted with exactly one space after its colon.
    ///
    /// # Why this one rule may edit bytes inside a token span
    ///
    /// `doc.rs`'s module comment forbids printing a leaf from anything but a
    /// slice of the source, and `format.rs:34-40` states the same invariant
    /// from the lex-based formatter's side. The reason is that a token's
    /// *payload* is not recoverable from its spelling for six token kinds —
    /// `0x1F` lexes to `IntConst(31)`, ``` ``x`` ``` and `` #`x` `` both lex
    /// to `Literal { body: "x", .. }` — so a printer that re-renders a leaf
    /// silently rewrites the program.
    ///
    /// A header is the one leaf where the rewrite provably does **not** reach
    /// the payload, and the argument is about `lex_header` rather than about
    /// headers in general. Reading `lexer.rs:899-965`:
    ///
    /// - the token's span covers the whole line **including its terminator**,
    ///   because the loop `bump()`s the break before it stops (and takes both
    ///   halves of a CRLF, deliberately — the comment there records the four
    ///   bugs a split pair caused);
    /// - between the `:` and the content it runs `while self.peek() ==
    ///   Some(' ') { self.bump() }` — **spaces only, and it keeps none of
    ///   them**. So the payload of `@require:   x` is already `x`, exactly as
    ///   the payload of `@require: x` is.
    ///
    /// Therefore rewriting the run of spaces after the colon to one space is a
    /// change to bytes that are inside a span but outside everything the span
    /// *means*: `Token::HeaderRequire(content)` compares equal before and
    /// after, so `same_tokens` — which compares slots and payloads, never
    /// spans — passes rather than being weakened to let this through. That is
    /// the whole of the licence, and it does not generalise: the same edit one
    /// character to the left (the header's name) or anywhere in any other
    /// token kind changes the payload and the verifier declines.
    ///
    /// Three things this must not do, each of which is a recorded bug from the
    /// lex-based formatter:
    ///
    /// - **add or drop a terminator.** `format.rs:536-556` records a buffer
    ///   whose last token is a header having an empty final gap, so judging by
    ///   emitted text alone appended a newline and made format-on-save
    ///   non-idempotent. Here the terminator is simply never touched — it is
    ///   the tail of `rest`, copied.
    /// - **split a CRLF.** Same reason: `rest` is copied whole.
    /// - **strip a tab.** `lex_header` does not, so the tab is payload.
    ///
    /// [`Build::push_header`] is where those three become code.
    fn header_leaf<T: Spanned<Span = Span>>(&mut self, t: &T) {
        if self.atoms.get(self.cursor).map(|a| a.span.start.byte) != Some(t.span().start.byte) {
            self.desync += 1;
        }
        self.emit_atom_as(Subst::Header);
    }

    /// Emit one atom without a check, for the few leaves that are token
    /// *alternations* (`AnyHorzCmdTok` and friends) rather than generated
    /// leaves, and so carry no single `Spanned`.
    fn one(&mut self) {
        self.emit_atom();
    }

    /// A binding-position name.
    ///
    /// `BindName` is either a `VarTok` (one atom) or an `OpNameTok` (three) —
    /// `let (+++>) = …`, which `itemize.satyh` and `progsynt.satyh` both
    /// write. Its `span` cannot tell the two apart and its `repr` is private,
    /// so the count comes from the node.
    fn bind_name(&mut self, n: &cst::BindName) {
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
    /// The leaf treatment for the parts of the grammar slice 1 does not lay
    /// out: types, patterns and math. Their line breaks survive and their
    /// continuation lines are re-indented to the enclosing depth, which is the
    /// honest answer for a construct nobody has written a rule for yet.
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
    /// element list: the two delimiters plus every element's atoms.
    ///
    /// Reached only when an area's own flag is off while an ENCLOSING area's
    /// is on — a math area holding a `!{ inline text }` escape, say. Nothing
    /// ships in that configuration; it exists so that turning one flag off is
    /// a real experiment rather than a compile error.
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
        let end = self.atoms[self.cursor + n - 1].span.end.byte.min(len).max(start);
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
                self.last_text = text;
            }
            Some(_) => {}
            None => self.broken = true,
        }
        self.byte = end;
        self.cursor += n;
    }

    // -- files ---------------------------------------------------------------

    fn file(&mut self, f: &cst::File) {
        for h in &f.headers {
            // A header ends its own line (`lex_header` swallows the
            // terminator, `lexer.rs:915-933`), so this emits no terminator —
            // what it carries is the author's BLANK lines, and the gap between
            // an `@require:` block and an `@import:` block is where a file
            // most often has one.
            self.block_break();
            match h {
                cst::Header::Require(t) => self.header_leaf(t),
                cst::Header::Import(t) => self.header_leaf(t),
                cst::Header::Stage(t) => self.header_leaf(t),
            }
        }
        for b in &f.prelude {
            // The FIRST one asks too, and that is not a redundancy: the
            // request is also what carries the author's blank lines
            // ([`Build::gap_upto`] case 2), and the gap between the last
            // `@require:` and the first binding is where a file's one
            // deliberate blank line most often is. A header has already ended
            // its own line, so the request emits no terminator there — only
            // the blank lines.
            self.block_break();
            self.top_binding(b);
        }
        if let Some(k) = &f.in_kw {
            self.block_break();
            self.leaf(k);
        }
        // The document body sits at the file's own depth, because that is where
        // the corpus writes it: `in` at column 0 and `document (| … |) '< … >`
        // under it, not indented from it.
        if let Some(body) = &f.body {
            self.block_break();
            self.expr(body);
        }
        self.leaf(&f.eoi);
    }

    // -- bindings ------------------------------------------------------------

    /// The shape every `let`-family binding shares: the keyword sits at the
    /// construct's own depth and *everything after it* — the ascription, the
    /// parameters, and the value — is one step in.
    ///
    /// One `Nest` rather than two (a header nest plus a value nest) is
    /// deliberate: `stdja.satyh:159`'s `let title-deco =` wants its body one
    /// step in, not two, and a nest per field would compound to two steps for
    /// every binding in the corpus.
    fn top_binding(&mut self, b: &cst::TopBinding) {
        use cst::TopBinding as T;
        match b {
            T::LetRec { kw, stage, first, ands } => {
                self.leaf(kw);
                self.enter();
                self.stage(stage.as_ref());
                self.rec_binding(first);
                self.exit();
                for a in ands {
                    self.leaf(&a.and_kw);
                    self.enter();
                    self.rec_binding(&a.binding);
                    self.exit();
                }
            }
            T::Let(l) => {
                self.leaf(&l.let_kw);
                self.enter();
                self.stage(l.stage.as_ref());
                self.bind_name(&l.name);
                self.ascription(l.ascription.as_ref());
                if let Some(bar) = &l.leading_bar {
                    self.leaf(bar);
                }
                self.params(&l.params);
                self.leaf(&l.eq);
                self.exit();
                self.body(&l.value);
            }
            T::LetPattern { let_kw, pat, eq, value } => {
                self.leaf(let_kw);
                self.enter();
                self.flat(&**pat);
                self.leaf(eq);
                self.exit();
                self.body(value);
            }
            T::LetInline { kw, stage, ctx, cmd, params, eq, value } => {
                self.leaf(kw);
                self.enter();
                self.stage(stage.as_ref());
                if let Some(c) = ctx {
                    self.leaf(c);
                }
                self.leaf(cmd);
                self.params(params);
                self.leaf(eq);
                self.exit();
                self.body(value);
            }
            T::LetBlock { kw, stage, ctx, cmd, params, eq, value } => {
                self.leaf(kw);
                self.enter();
                self.stage(stage.as_ref());
                if let Some(c) = ctx {
                    self.leaf(c);
                }
                self.leaf(cmd);
                self.params(params);
                self.leaf(eq);
                self.exit();
                self.body(value);
            }
            T::LetMath { kw, stage, cmd, params, eq, value } => {
                self.leaf(kw);
                self.enter();
                self.stage(stage.as_ref());
                self.leaf(cmd);
                self.params(params);
                self.leaf(eq);
                self.exit();
                self.body(value);
            }
            T::Type(t) => self.type_decl(t),
            T::LetMutable { kw, stage, name, arrow, value } => {
                self.leaf(kw);
                self.enter();
                self.stage(stage.as_ref());
                self.leaf(name);
                self.leaf(arrow);
                self.exit();
                self.body(value);
            }
            T::Module { kw, name, sig, eq, struct_kw, decls, end_kw } => {
                self.leaf(kw);
                self.leaf(name);
                // `: sig … end` and `= struct … end` stay at the module's own
                // depth and their *items* go one step in, which is how
                // `list.satyg:4-26` reads once the hand indentation is gone.
                if let Some(s) = sig {
                    self.leaf(&s.colon);
                    self.leaf(&s.sig_kw);
                    self.enter();
                    for item in &s.items {
                        self.block_break();
                        self.sig_item(item);
                    }
                    self.exit();
                    self.block_break();
                    self.leaf(&s.end_kw);
                }
                self.leaf(eq);
                self.leaf(struct_kw);
                self.enter();
                for d in decls {
                    self.block_break();
                    self.top_binding(&d.0);
                }
                self.exit();
                self.block_break();
                self.leaf(end_kw);
            }
            T::Open { kw, name } => {
                self.leaf(kw);
                self.leaf(name);
            }
        }
    }

    fn stage(&mut self, s: Option<&cst::TopStage>) {
        if let Some(s) = s {
            if let Some(p) = &s.persistent {
                self.leaf(p);
            }
            self.leaf(&s.tilde);
        }
    }

    fn ascription(&mut self, a: Option<&ast::RecAscription>) {
        if let Some(a) = a {
            self.leaf(&a.colon);
            self.enter();
            self.type_expr(&a.ty);
            self.exit();
        }
    }

    /// `let-rec`'s clause list: the name, the first clause's parameters and
    /// body, then one `| params = value` per further clause.
    ///
    /// The clauses land at the depth the caller opened, which for `let-rec` is
    /// one step in from the keyword — `engine.md` section 12's example (3).
    fn rec_binding(&mut self, b: &ast::RecBinding) {
        self.bind_name(&b.name);
        self.ascription(b.ascription.as_ref());
        if let Some(bar) = &b.leading_bar {
            self.clause_break();
            self.leaf(bar);
        }
        self.pat_params(&b.params);
        self.leaf(&b.eq);
        self.body(&b.value.0);
        for c in &b.extra {
            self.clause_break();
            self.leaf(&c.bar);
            self.pat_params(&c.params);
            self.leaf(&c.eq);
            self.body(&c.value.0);
        }
    }

    fn sig_item(&mut self, item: &cst::SigItem) {
        use cst::SigItem as S;
        let (ty, constraints) = match item {
            S::ValHorzCmd { kw, name, colon, ty, constraints } => {
                self.leaf(kw);
                self.leaf(name);
                self.leaf(colon);
                (Some(ty), constraints)
            }
            S::ValVertCmd { kw, name, colon, ty, constraints } => {
                self.leaf(kw);
                self.leaf(name);
                self.leaf(colon);
                (Some(ty), constraints)
            }
            S::Val { kw, name, colon, ty, constraints } => {
                self.leaf(kw);
                self.bind_name(name);
                self.leaf(colon);
                (Some(ty), constraints)
            }
            S::DirectHorzCmd { kw, name, colon, ty, constraints } => {
                self.leaf(kw);
                self.leaf(name);
                self.leaf(colon);
                (Some(ty), constraints)
            }
            S::DirectVertCmd { kw, name, colon, ty, constraints } => {
                self.leaf(kw);
                self.leaf(name);
                self.leaf(colon);
                (Some(ty), constraints)
            }
            S::Type { kw, tyvars, name, constraints } => {
                self.leaf(kw);
                for v in tyvars {
                    self.leaf(v);
                }
                self.leaf(name);
                (None, constraints)
            }
        };
        // No frame of its own. A `val`'s type opens its own arrow-chain group
        // (with its own `Nest`) and a `constraint`'s record kind opens a
        // group's; a frame here would put both one step further in than the
        // item they belong to, which is the doubled indentation
        // `type config = (|` showed before `type_body` stopped doing the same.
        if let Some(ty) = ty {
            self.type_expr(ty);
        }
        for c in constraints {
            self.leaf(&c.kw);
            self.leaf(&c.tyvar);
            self.leaf(&c.cons);
            self.record_kind(&c.kind);
        }
    }

    /// A `constraint`'s record kind: `(| name : ty; … |)`. The same `(| … |)`
    /// a record type is, reached from a `sig` item rather than from a type or
    /// a value.
    fn record_kind(&mut self, k: &cst::RecordKind) {
        let anchor = self.group_open(&k.rec.open);
        for (i, f) in k.fields.iter().enumerate() {
            self.item_break(i);
            self.leaf(&f.name);
            self.leaf(&f.colon);
            self.type_expr(&f.ty);
            if let Some(s) = &f.semi {
                self.leaf(s);
            }
        }
        self.group_close(anchor, &k.rec.close);
    }

    fn type_decl(&mut self, t: &cst::TypeDecl) {
        self.leaf(&t.kw);
        self.enter();
        for v in &t.tyvars {
            self.leaf(v);
        }
        self.leaf(&t.name);
        self.leaf(&t.eq);
        self.exit();
        self.type_body(&t.body);
        for a in &t.ands {
            self.leaf(&a.and_kw);
            self.enter();
            for v in &a.tyvars {
                self.leaf(v);
            }
            self.leaf(&a.name);
            self.leaf(&a.eq);
            self.exit();
            self.type_body(&a.body);
        }
    }

    /// A `type … = ` right-hand side, as [`Build::body`] treats a value's.
    ///
    /// One [`Mode::Auto`] group over one `Nest` step, so a synonym that fits
    /// stays on the declaration's line and one that does not takes the next
    /// line one step in — which is what puts a 9-field record type's fields at
    /// 2 columns and its `|)` at 0, rather than 4 and 2.
    fn type_body(&mut self, b: &cst::TypeDeclBody) {
        if self.breaks.bodies && hugs_type_body(b) {
            self.type_decl_body(b);
            return;
        }
        match self.breaks.bodies {
            true => {
                self.push_group(Mode::Auto);
                self.frames.push(Frame::nest(1));
                self.br(Br::Opportunity);
                self.type_decl_body(b);
                self.exit();
                self.exit();
            }
            false => {
                self.enter();
                self.type_decl_body(b);
                self.exit();
            }
        }
    }

    fn type_decl_body(&mut self, b: &cst::TypeDeclBody) {
        match b {
            cst::TypeDeclBody::Variant { leading_bar, first, rest } => {
                // A variant's constructors are a clause list, exactly as a
                // `match`'s arms are: `list.satyg` and `stdja.satyh` both write
                // one `| Ctor of ty` per line however short they are.
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
            cst::TypeDeclBody::Synonym(ty) => self.type_expr(ty),
        }
    }

    fn variant_def(&mut self, v: &cst::VariantDef) {
        self.leaf(&v.ctor);
        if let Some(of) = &v.of_ty {
            self.leaf(&of.of_kw);
            self.type_expr(&of.ty);
        }
    }

    // -- types ---------------------------------------------------------------

    /// A type expression.
    ///
    /// Slice 1 makes no layout *decisions* here — an arrow chain is emitted as
    /// one flat sequence — but it does have to walk the structure rather than
    /// treat a type as an opaque run of atoms, for one reason: the record type
    /// `(| … |)`. It is the one construct in this grammar that the corpus
    /// routinely writes across lines *inside a type* (`code-printer.satyh`'s
    /// `type syntax` and `type design` are 14 and 20 fields), and without
    /// [`Build::group_close`] its `|)` lands a step too deep.
    fn type_expr(&mut self, t: &ast::TypeExpr) {
        use ast::TypeExpr as T;
        // One `Mode::Auto` group over the OUTERMOST arrow chain, and only
        // there: `a -> b -> c` right-nests in the CST but reads as one
        // sequence, so a group per node would break the first arrow and leave
        // the rest flat. The flag is what makes "outermost" a property of the
        // walk rather than of the node — a type reached from inside a record
        // field or a `[…] inline-cmd` argument opens its own chain, because
        // the delimiter it sits behind reset it.
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
            T::Fun { opts, dom, arrow, cod } => {
                for o in opts {
                    self.type_prod(&o.ty);
                    self.leaf(&o.arrow);
                }
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
            T::OptRowFun { opt_dom, dom, arrow, cod } => {
                self.leaf(&opt_dom.q);
                let anchor = self.group_open(&opt_dom.paren.open);
                for e in &opt_dom.entries {
                    self.leaf(&e.label);
                    self.leaf(&e.colon);
                    self.type_expr(&e.ty.0);
                    if let Some(c) = &e.comma {
                        self.leaf(c);
                    }
                }
                self.group_close(anchor, &opt_dom.paren.close);
                self.type_prod(dom);
                if self.breaks.type_arrows {
                    self.br(Br::Opportunity);
                }
                self.leaf(arrow);
                self.type_expr(cod);
            }
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
        self.type_atom(&a.head);
        for r in &a.rest {
            self.type_atom(r);
        }
    }

    fn type_atom(&mut self, a: &ast::TypeAtom) {
        use ast::TypeAtom as A;
        match a {
            A::Cmd { list, args, kind } => {
                let anchor = self.group_open(&list.open);
                for it in args {
                    for l in &it.opt_labels {
                        self.leaf(&l.label);
                        self.leaf(&l.colon);
                        self.type_expr(&l.ty.0);
                        if let Some(c) = &l.comma {
                            self.leaf(c);
                        }
                    }
                    self.type_expr(&it.ty.0);
                    if let Some(o) = &it.opt {
                        self.leaf(o);
                    }
                    if let Some(sc) = &it.semi {
                        self.leaf(sc);
                    }
                }
                self.group_close(anchor, &list.close);
                match kind {
                    ast::CmdTypeKind::Inline(t) => self.leaf(t),
                    ast::CmdTypeKind::Block(t) => self.leaf(t),
                    ast::CmdTypeKind::Math(t) => self.leaf(t),
                }
            }
            A::Paren { paren, inner } => {
                let anchor = self.group_open(&paren.open);
                self.type_expr(&inner.0);
                self.group_close(anchor, &paren.close);
            }
            // A record TYPE — the same `(| … |)` a record literal is, so the
            // same three rules reach it: `colon`, `separator` and `bracket`.
            A::Record { rec, fields } => {
                let anchor = self.group_open(&rec.open);
                for (i, f) in fields.iter().enumerate() {
                    self.item_break(i);
                    self.leaf(&f.name);
                    self.leaf(&f.colon);
                    self.enter();
                    self.type_expr(&f.ty.0);
                    self.exit();
                    if let Some(sc) = &f.semi {
                        self.leaf(sc);
                    }
                }
                self.group_close(anchor, &rec.close);
            }
            A::Var(t) => self.leaf(t),
            A::Name(t) => self.leaf(t),
            A::NameMod(t) => self.leaf(t),
            A::RecordOpen { orec, inner } => {
                let anchor = self.group_open(&orec.open);
                for f in &inner.fields {
                    self.leaf(&f.name);
                    self.leaf(&f.colon);
                    self.type_expr(&f.ty.0);
                    if let Some(c) = &f.comma {
                        self.leaf(c);
                    }
                }
                self.leaf(&inner.bar);
                self.leaf(&inner.var);
                self.group_close(anchor, &orec.close);
            }
        }
    }

    // -- expressions ---------------------------------------------------------

    /// The `let … in` spine, walked **iteratively into one flat frame**.
    ///
    /// The CST nests these to the right, one level per binding
    /// (`cst.rs:767-782`), but they read as a statement sequence and the corpus
    /// writes them flat: `stdja.satyh:272-289` is eleven consecutive
    /// `let … in` at one indentation. A recursive printer would indent the
    /// eleventh twenty-two columns in. The loop below is the fix, and the
    /// *absence* of an `enter()` around the continuation is the statement of
    /// intent — there is nothing to check in a recursive version that "happens
    /// to pass the same depth".
    fn expr(&mut self, e: &ast::Expr) {
        use ast::Expr as E;
        // One `Mode::Auto` group over the WHOLE spine, so eleven consecutive
        // `let … in` break together or not at all. All-or-nothing rather than
        // a fill for the same reason `match` arms are: half a statement
        // sequence on one line reads as though the halves belonged together.
        let spine = self.breaks.let_spine && is_spine(e);
        if spine {
            self.push_group(Mode::Auto);
        }
        let mut cur = e;
        loop {
            match cur {
                E::LetIn { kw, name, ascription, leading_bar, params, eq, value, in_kw, body } => {
                    self.leaf(kw);
                    self.enter();
                    self.bind_name(name);
                    self.ascription(ascription.as_ref());
                    if let Some(bar) = leading_bar {
                        self.leaf(bar);
                    }
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
                E::LetRecIn { kw, first, ands, in_kw, body } => {
                    self.leaf(kw);
                    self.enter();
                    self.rec_binding(first);
                    self.exit();
                    for a in ands {
                        self.leaf(&a.and_kw);
                        self.enter();
                        self.rec_binding(&a.binding);
                        self.exit();
                    }
                    self.leaf(in_kw);
                    if spine {
                        self.br(Br::Opportunity);
                    }
                    cur = body;
                }
                E::LetPatternIn { kw, pat, eq, value, in_kw, body } => {
                    self.leaf(kw);
                    self.enter();
                    self.flat(&**pat);
                    self.leaf(eq);
                    self.exit();
                    self.body(value);
                    self.leaf(in_kw);
                    if spine {
                        self.br(Br::Opportunity);
                    }
                    cur = body;
                }
                E::LetMutableIn { kw, name, arrow, init, in_kw, body } => {
                    self.leaf(kw);
                    self.enter();
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
                E::LetMathIn { kw, cmd, params, eq, value, in_kw, body } => {
                    self.leaf(kw);
                    self.enter();
                    self.leaf(cmd);
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
                E::OpenIn { kw, name, in_kw, body } => {
                    self.leaf(kw);
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
            E::If { kw, cond, then_kw, then_branch, else_kw, else_branch } => {
                self.leaf(kw);
                self.enter();
                self.expr(cond);
                self.exit();
                // `then` and `else` are at the `if`'s own depth, and a
                // branch body that starts its own line is one step past the
                // line its keyword is on — SYMMETRICALLY, which is
                // `Build::body`'s whole point. The condition's frame has
                // already spent the `if` line's step, so `Build::enter` alone
                // gave `then` nothing and `else` one.
                // **One group over BOTH branches**, so they break together.
                // Two independent `body` groups gave
                //
                //     if x > 0 then
                //       `positive` else `negative`
                //
                // because the `then` branch's own group was the only one that
                // did not fit; an `if` reads as one construct and its two
                // arms have one answer.
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
            E::Fun { kw, params, arrow, body } => {
                self.leaf(kw);
                self.enter();
                self.pat_params(params);
                self.leaf(arrow);
                self.exit();
                self.body(body);
            }
            E::FunRows { kw, opts, param, arrow, body } => {
                self.leaf(kw);
                self.enter();
                self.opt_binders(opts);
                self.flat(param);
                self.leaf(arrow);
                self.exit();
                self.body(body);
            }
            E::Match { kw, scrutinee, with_kw, leading_bar, first, rest } => {
                self.leaf(kw);
                self.enter();
                self.expr(scrutinee);
                self.exit();
                self.leaf(with_kw);
                // Arms at the `match`'s own depth: `progsynt.satyh:49-53` and
                // `list.satyg:28-30` both write the leading `|` in the `match`
                // keyword's column.
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
            }
            E::WhileDo { kw, cond, do_kw, body } => {
                self.leaf(kw);
                self.enter();
                self.expr(cond);
                self.exit();
                self.leaf(do_kw);
                self.body(body);
            }
            E::Overwrite { name, arrow, value } => {
                self.leaf(name);
                self.leaf(arrow);
                self.body(&value.0);
            }
            E::Ops(chain) => self.op_chain(chain),
            // The spine nodes, reached only if `expr` is bypassed.
            other => self.expr(other),
        }
    }

    fn match_arm(&mut self, a: &ast::MatchArm) {
        self.flat(&*a.pat);
        if let Some(g) = &a.guard {
            self.leaf(&g.when_kw);
            self.enter();
            self.expr(&g.cond.0);
            self.exit();
        }
        self.leaf(&a.arrow);
        self.body(&a.body.0);
    }

    /// A body **introduced by a token already emitted on this line** — a
    /// `then`/`else`/`do` keyword, a `->` arrow, a binding's `=`.
    ///
    /// One [`Mode::Auto`] group over `Nest(1, [ Opportunity, body ])`: the body
    /// stays on the introducer's line if it fits there, and otherwise takes the
    /// next line one step in. That is the shape the corpus writes for every
    /// one of them —
    ///
    /// ```text
    ///   let title-deco =        | Some(i) ->        if x > 0 then
    ///     let pads = … in           term-paren m      long-branch
    /// ```
    ///
    /// — and it is one rule rather than the three special cases slice 1 needed,
    /// because slice 1 had to *guess* from the author's break whether the body
    /// owned the next line. Here it does not have to guess: whichever way the
    /// group goes, the `Nest` is inside it, so a body that stays flat
    /// contributes no line and no indentation and a body that breaks gets
    /// exactly one step.
    ///
    /// # Why the group and not just the nest
    ///
    /// Without the group the break point would belong to whatever group
    /// encloses it — a record's, say — and `a = <long>` inside a record that
    /// broke would break after its `=` too, one field at a time, whether or
    /// not the field fitted. A body's fit question is its own.
    fn body(&mut self, e: &ast::Expr) {
        // A hugged body opens NO frame at all. Its own delimiter is what
        // indents its contents, and a frame here would put them one step
        // further in than the group's closer — the doubled indentation
        // `let x = (|` had before this branch existed.
        if self.breaks.bodies && hugs_body(e) {
            self.expr(e);
            return;
        }
        match self.breaks.bodies {
            true => {
                self.push_group(Mode::Auto);
                self.frames.push(Frame::nest(1));
                self.br(Br::Opportunity);
                self.expr(e);
                self.exit();
                self.exit();
            }
            false => {
                self.enter();
                self.expr(e);
                self.exit();
            }
        }
    }

    /// One element of a `;`- or `,`-separated run: a list item, a tuple
    /// component.
    ///
    /// **No frame of its own.** The elements of a run are siblings and must
    /// all get the same answer, and the enclosing group's own `Nest` already
    /// supplies it ([`Build::group_open`]) — a further step per element is the
    /// staircase `color-ext.satyg`'s 147-entry colour table produced when this
    /// shared [`Build::body`]'s anchor:
    ///
    /// ```text
    ///   (            [
    ///     1,           1;
    ///       2,           2;
    ///       3            3;
    ///   )            ]
    /// ```
    ///
    /// A *continuation* line of one long element is therefore at the run's own
    /// depth rather than one step past it, which is the honest answer while
    /// nothing distinguishes the two: the element's own constructs open their
    /// own frames, and those are what indent a wrapped element.
    fn element(&mut self, e: &ast::Expr) {
        self.expr(e);
    }

    /// `head (op rhs)* (before e)?`, with the tail one step in.
    ///
    /// All ten precedence levels are flattened here (`cst.rs:6-10`), so the
    /// formatter cannot know precedence and must not indent by it. One depth
    /// for the whole tail is the only rule available, and it is also the corpus
    /// idiom — `stdja.satyh:505-507`'s `|>` pipeline breaks before the
    /// operator at one indentation.
    fn op_chain(&mut self, c: &ast::OpChain) {
        self.app_expr(&c.head);
        if c.tail.is_empty() && c.before.is_none() {
            return;
        }
        // The head is OUTSIDE the group deliberately, and it costs nothing:
        // the renderer resolves a group against the column it has already
        // reached, so measuring the tail from after the head is measuring the
        // whole chain from before it. What it buys is that a head which
        // breaks for its own reasons — a multi-line record — does not force
        // the operators open.
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
        if let Some(b) = &c.before {
            if grouped {
                self.br(Br::Opportunity);
            }
            self.leaf(&b.kw);
            self.expr(&b.body.0);
        }
        self.exit();
        if grouped {
            self.exit();
        }
    }

    fn app_expr(&mut self, a: &ast::AppExpr) {
        if let Some(m) = &a.minus {
            self.leaf(m);
            // A UNARY minus keeps whichever of `(-1)` and `(- 1)` the author
            // wrote — 37 tight in the corpus against 4 spaced. Structural
            // rather than lexical because the token cannot tell the two `-`s
            // apart — `Token::ExactMinus` is subtraction as well, and `a - 1`
            // is 65 gaps that must keep their space; the CST is the only
            // thing here that knows which is which.
            //
            // Not [`Space::Tight`], and the reason is [`sep::must_separate`]
            // rather than taste: `-` is a WORD character in this grammar
            // (`set-font`), so the table refuses to write `-1` and a tight
            // request would come back as `(- 1)` — worse than either of the
            // two things the corpus writes. The table is the only fusion
            // authority and it is not overruled here; the rule declines
            // instead.
            //
            // [`Space::Collapse`] and not [`Space::Keep`]: declining to pick
            // between empty and one space is not a reason to keep `(-   1)`,
            // and collapsing cannot fuse because it always leaves a space
            // behind. See [`Spacing::minus`].
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
        // The constructor-head exception is NOT here any more, and that is
        // the point of the inversion: `Some(x)` is a constructor followed by
        // an opening delimiter, which [`Spacing::ctor_arg`] recognises from
        // the two tokens themselves — so it holds in a PATTERN too, where
        // this arm never runs (`match_arm` emits a pattern through
        // [`Build::flat`], which knows no shapes at all).
        self.app_args(&a.args);
    }

    /// The argument run, one frame.
    ///
    /// `itemize.satyh:48-52` is the exhibit `format.rs:96-110` uses against a
    /// bracket counter: its last argument line is indented one step past its
    /// function *with no bracket opened in between*, which a bracket counter
    /// cannot produce and this frame cannot help producing. The other half of
    /// that shape — the `(fun ctx -> …)` argument whose body sits one step past
    /// the call rather than three — is [`Build::enter`]'s doing, not a rule
    /// here.
    /// `spaced` is [`Spacing::app_arg`]'s one predicate: false for a data
    /// constructor head, true for everything else. It is the head's, and it
    /// is the only thing consulted — the boundary's own two sides are not.
    ///
    /// # Every boundary, not the group-shaped ones
    ///
    /// The rule was written twice as a question about groups (does the
    /// argument OPEN one; does the thing to its left CLOSE one) and both were
    /// the same mistake: `` List.intersperse  `, ` `` and `f  xs` kept their
    /// double space because a literal and a variable are not groups. A
    /// boundary has no shape, so the predicate does not ask about one. See
    /// [`Spacing::app_arg`] for the 18,413-boundary census that scopes it.
    fn app_args(&mut self, args: &[ast::AppArg]) {
        if args.is_empty() {
            return;
        }
        // The trailing group-shaped argument is HUGGED: emitted outside the
        // run's own frame and with no break point in front of it. See
        // [`Build::hugs_last`].
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
            // No break point in front of it: hugging means the line ends
            // INSIDE the argument, not before it. Breaking before a trailing
            // `(fun ctx -> …)` or `'< … >` would leave the call head alone on
            // a line and the block dangling under it, which is what
            // `itemize.satyh:48-52` is written to avoid.
            if let Some(a) = args.last() {
                self.app_arg(a);
            }
        }
    }

    fn app_arg(&mut self, a: &ast::AppArg) {
        use ast::AppArg as A;
        match a {
            A::Optional { q, value } => {
                self.leaf(q);
                self.atomic(value);
            }
            A::Omission(t) => self.leaf(t),
            A::Atom { stage, excl, atom, accesses } => {
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
            A::Bundled { opts, excl, atom, accesses } => {
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
        }
    }

    fn stage_prefix(&mut self, s: Option<&ast::StagePrefix>) {
        // Never a break and never a space after the sigil: `&`/`~` bind to
        // their operand (`engine.md` section 9). Slice 1 could not insert one
        // anyway — it only ever copies the gap that is there.
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
            A::Literal(t) => self.leaf(t),
            A::OpRef(t) => self.leaf_multi(t),
            A::VarWithMod(t) => self.leaf(t),
            A::Float(_) | A::Int(_) => self.one(),
            A::True(t) => self.leaf(t),
            A::False(t) => self.leaf(t),
            A::Ctor(t) => self.leaf(t),
            A::Var(t) => self.leaf(t),
            A::Command { kw, .. } => {
                self.leaf(kw);
                // `AnyHorzCmdTok` is an alternation, not a generated leaf.
                self.one();
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
            A::OpenModule { grp, body } => {
                let anchor = self.group_open(&grp.open);
                self.paren_body(body);
                self.group_close(anchor, &grp.close);
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
                    self.element(&i.value.0);
                    if let Some(s) = &i.semi {
                        self.leaf(s);
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
        self.expr(&b.first.0);
        for c in &b.rest {
            self.leaf(&c.comma);
            self.item_break(1);
            self.element(&c.value.0);
        }
    }

    fn record_body(&mut self, b: &ast::RecordBody) {
        match b {
            ast::RecordBody::Update { base, with_kw, fields } => {
                self.expr(&base.0);
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

    /// `name = value;` inside a record literal — **slice 2's first rule**.
    ///
    /// One space on each side of the `=`, no space before the `;`. Requests
    /// only: a `=` the author put on a line of its own keeps its line, because
    /// [`Build::gap_upto`] drops a request over a break.
    ///
    /// Note where the second request sits — *before* [`Build::body`], not
    /// inside it. `body` opens a frame and only then emits, and a request
    /// survives frame pushes, so the space lands in front of the value's first
    /// atom wherever the frame put it.
    fn record_field(&mut self, f: &ast::RecordField) {
        self.leaf(&f.name);
        self.leaf(&f.eq);
        self.body(&f.value.0);
        if let Some(s) = &f.semi {
            self.leaf(s);
        }
    }

    fn opt_args(&mut self, o: &ast::CstOptArgs) {
        self.leaf(&o.q);
        let anchor = self.group_open(&o.paren.open);
        for e in &o.entries {
            self.leaf(&e.label);
            self.leaf(&e.eq);
            self.body(&e.value.0);
            if let Some(c) = &e.comma {
                self.leaf(c);
            }
        }
        self.group_close(anchor, &o.paren.close);
    }

    fn opt_binders(&mut self, o: &ast::CstOptBinders) {
        self.leaf(&o.q);
        let anchor = self.group_open(&o.paren.open);
        for e in &o.entries {
            self.leaf(&e.label);
            self.leaf(&e.eq);
            self.leaf(&e.var);
            if let Some(c) = &e.comma {
                self.leaf(c);
            }
        }
        self.group_close(anchor, &o.paren.close);
    }

    // -- the three text areas (slice 4) --------------------------------------

    /// `'< … >`, laid out.
    ///
    /// Nothing in here is a new mechanism: block text's whitespace is *gaps*
    /// (see [`Areas::block`]), so [`Build::gap_upto`] reproduces the author's
    /// breaks and drops their indentation exactly as it does for program
    /// text, and [`Build::group_open`]/[`Build::group_close`] put the `>` back
    /// at its opener's column.
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

    /// The gap between two `'< … >` items: a break OPPORTUNITY normally, a
    /// [`Br::Hard`] when the author put a **blank line** there.
    ///
    /// The blank line is the only thing that upgrades it, and it upgrades it
    /// rather than merely surviving it because a blank line can only exist
    /// between two lines — [`Build::gap_upto`] carries the author's blank
    /// lines at a `Br::Hard` position and nowhere else, and the reason it can
    /// is that a `Br::Hard` is unconditional. An `Br::Opportunity` the
    /// renderer chose to render flat would put the two items on one line with
    /// the blank line simply gone, and the second pass would then have nothing
    /// to read.
    ///
    /// See [`Breaks::block_blanks`] for why this stays a fixpoint, and why the
    /// `max_blank_lines == 0` case turns the rule off instead of clamping it.
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
                // `#var;` is an ACTIVE area: `lex_active` reads the `;`, and
                // the gap in front of it is one of its own.
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

    /// `{ … }`, re-indented.
    ///
    /// The delimiters go through [`Subst::Swallow`] because both of them
    /// swallow trivia: `{` takes what follows it (`lexer.rs:562-567`) and `}`
    /// takes the whitespace run in front of it (`lexer.rs:1122-1126`).
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
    /// **No [`Spacing::app_arg`] request here, and that is the measurement
    /// rather than an omission**: a command head is 1912 tight against 173
    /// spaced in the corpus, so `\emph{x}` and `+p{hi}` keep their arguments
    /// tight — and the trailing boundary agrees, 499 tight against 52. A
    /// command's arguments never reach [`Build::app_args`], so widening that
    /// rule to every argument shape does not reach them either. See that
    /// flag's two censuses.
    ///
    /// The arguments themselves are ordinary program text — `lex_active`
    /// emits no token for the whitespace between a command and its arguments
    /// (`lexer.rs:1241-1243`), so those gaps were always gaps and this is
    /// `engine.md` section 4's "the area boundary the CST gains".
    fn cmd_tail(&mut self, tail: &ast::CmdTail) {
        match tail {
            ast::CmdTail::Semi(t) => {
                let outer = self.enter_area(Area::Active);
                self.leaf(t);
                self.area = outer;
            }
            ast::CmdTail::Args { first, rest, semi } => {
                let outer = self.enter_area(Area::Active);
                // The trailing `{ … }` is HUGGED, exactly as an application's
                // trailing group is ([`hugs_last`]) — and here it is not a
                // refinement but the difference between
                //
                //     +p {          and      +p {
                //       hello                    hello
                //     }                        }
                //
                // because `+p{…}`'s inline text is the LAST thing on the line
                // and its own frame is what indents its contents. A frame
                // around the run as well puts them two steps in and leaves the
                // `}` one step in — which is what `manual.saty` came back as.
                let all: Vec<&ast::AppArg> = std::iter::once(&**first)
                    .chain(rest.iter().map(|a| &**a))
                    .collect();
                let hug = self.breaks.app_args && all.last().is_some_and(|a| hugs_arg(a));
                let split = match hug {
                    true => all.len() - 1,
                    false => all.len(),
                };
                if split > 0 {
                    self.enter();
                    for a in &all[..split] {
                        self.cmd_arg_boundary();
                        self.app_arg(a);
                    }
                    self.exit();
                }
                if hug {
                    if let Some(a) = all.last() {
                        self.cmd_arg_boundary();
                        self.app_arg(a);
                    }
                }
                if let Some(t) = semi {
                    self.leaf(t);
                }
                self.area = outer;
            }
        }
    }

    /// `${ … }` — re-indented, and its line structure left exactly as it was.
    ///
    /// **Math is atomic for line-breaking, in both directions.** The formatter
    /// neither invents a break inside `${ … }` nor removes one the author
    /// wrote ([`Build::gap_upto`] case 3). Those are one decision rather than
    /// two: a rule that refuses to add breaks but happily deletes them turns
    /// every hand-laid-out equation into a single long line and leaves the
    /// author no way to lay it out again.
    ///
    /// # Why not break it
    ///
    /// Math's whitespace IS a gap — `lex_math` skips it without emitting
    /// (`lexer.rs:1338-1340`) — so a break here is token-preserving and the
    /// corpus sweep confirms it. Token-preserving is not the same as legible.
    /// A math group's delimiters are not separable from their content the way
    /// a record's are: `\frac{n + 1}{6}`'s second argument is one operand, and
    /// offering a break just inside its braces put `{`, `6` and `}` on three
    /// lines of a real 145-column equation, and split `e^{-x^2}` in a second.
    /// Breaking only at TOP-LEVEL atom boundaries fixed both, and the answer
    /// asked for is the simpler one: math does not reflow.
    ///
    /// A long equation with no break in it therefore stays one long line, and
    /// that is `engine.md` section 1.8's case rather than a gap in this slice:
    /// overflow is normal in SATySFi, which is why `error_on_line_overflow` is
    /// not shipped. What the author DID break stays broken, so the escape
    /// hatch is the author's own newline.
    ///
    /// The `Nest` is what re-indents those kept lines; it is also live for a
    /// break that arrives another way, an escape back into inline or block
    /// text.
    fn math_text(&mut self, grp: &MathGroup<()>, elems: &[cst::MathErased]) {
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
                // `\frac{a}{b}`'s last group hugs for [`Build::cmd_tail`]'s
                // reason: it is the last thing on the line, and its own frame
                // is what indents its contents.
                let hug = self.breaks.app_args && !args.is_empty();
                let split = match hug {
                    true => args.len() - 1,
                    false => args.len(),
                };
                if split > 0 {
                    self.enter();
                    for a in &args[..split] {
                        self.cmd_arg_boundary();
                        self.math_arg(a);
                    }
                    self.exit();
                }
                if hug {
                    if let Some(a) = args.last() {
                        self.cmd_arg_boundary();
                        self.math_arg(a);
                    }
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

    fn math_arg(&mut self, a: &ast::MathArg) {
        match a {
            ast::MathArg::Optional { q, body } => {
                self.leaf(q);
                self.math_arg_body(body);
            }
            ast::MathArg::Omission(t) => self.leaf(t),
            ast::MathArg::Plain(b) => self.math_arg_body(b),
        }
    }

    /// A math command's argument, including the four `!`-escapes back into
    /// inline text, block text and program mode.
    fn math_arg_body(&mut self, b: &ast::MathArgBody) {
        use ast::MathArgBody as B;
        match b {
            B::Math { mgrp, elems } => self.math_text(mgrp, elems),
            B::Inline { igrp, elems } => match self.areas.inline {
                true => self.inline_text(igrp, elems),
                false => self.verbatim_elems(elems),
            },
            B::Block { bgrp, elems } => match self.areas.block {
                true => self.block_text(bgrp, elems),
                false => self.verbatim_elems(elems),
            },
            // `!(`, `![` and `!(|` escape back to PROGRAM mode — the lexer
            // consumes the sigil and emits an ordinary `LParen`/`BList`/
            // `BRecord` (`cst.rs:1920-1930`), so nothing but this switch says
            // the contents are program text rather than math.
            // `!(`, `![` and `!(|` escape back to PROGRAM mode, and their
            // CONTENTS are laid out as program text — but the delimiters
            // themselves are a math command's ARGUMENT and are atomic for the
            // same reason `\frac`'s braces are ([`Build::math_text`]).
            // `azmath.saty:783` is the exhibit: `\pmatrix!(2){| … |}` came
            // back with the `2` alone on a line between its own parentheses.
            B::ParenEscape { paren, inner } => {
                self.area_open_as(&paren.open, Subst::None);
                let outer = self.enter_area(Area::Program);
                self.paren_body(inner);
                self.area_close_as(&paren.close, Subst::None);
                self.area = outer;
            }
            B::ListEscape { list, items } => {
                self.area_open_as(&list.open, Subst::None);
                let outer = self.enter_area(Area::Program);
                for (n, i) in items.iter().enumerate() {
                    self.item_break(n);
                    self.element(&i.value.0);
                    if let Some(sc) = &i.semi {
                        self.leaf(sc);
                    }
                }
                self.area_close_as(&list.close, Subst::None);
                self.area = outer;
            }
            B::RecordEscape { rec, body } => {
                self.area_open_as(&rec.open, Subst::None);
                let outer = self.enter_area(Area::Program);
                self.record_body(body);
                self.area_close_as(&rec.close, Subst::None);
                self.area = outer;
            }
        }
    }

    fn horz_cmd(&mut self, c: &AnyHorzCmdTok) {
        use AnyHorzCmdTok as C;
        match c {
            C::Plain(t) => self.leaf(t),
            C::Mod(t) => self.leaf(t),
        }
    }

    fn vert_cmd(&mut self, c: &AnyVertCmdTok) {
        use AnyVertCmdTok as C;
        match c {
            C::Plain(t) => self.leaf(t),
            C::Mod(t) => self.leaf(t),
        }
    }

    fn math_cmd(&mut self, c: &AnyMathCmdTok) {
        use AnyMathCmdTok as C;
        match c {
            C::Plain(t) => self.leaf(t),
            C::Mod(t) => self.leaf(t),
        }
    }

    /// A binding's parameter run, filled exactly as an application's argument
    /// run is ([`Build::app_args`]).
    ///
    /// The binding site's half of the same rule, and the corpus needs it:
    /// `cd.satyh:32` is `let draw-arr-scheme arrowf ctx m ?:t-name-opt
    /// ?:len-name-opt (((x1r, _), _) as obj1r) (((x2r, _), _) as obj2r) =` at
    /// 114 columns, and it is one of only two program lines in the whole 0.0.6
    /// corpus that stayed over budget when the argument runs alone filled.
    /// No hug: a parameter is a pattern, and the last one is followed by the
    /// `=` that has to stay with it.
    fn params(&mut self, params: &[ast::Param]) {
        for p in params {
            if self.breaks.app_args {
                self.br(Br::Fill);
            }
            match p {
                ast::Param::Optional { q, name } => {
                    // `?:x` abuts, and slice 1 cannot separate it anyway.
                    self.leaf(q);
                    self.leaf(name);
                }
                ast::Param::Pat(b) => self.flat(b),
                ast::Param::Bundled { opts, body } => {
                    self.opt_binders(opts);
                    self.flat(body);
                }
            }
        }
    }

    fn pat_params(&mut self, params: &[ast::PatBot]) {
        for p in params {
            if self.breaks.app_args {
                self.br(Br::Fill);
            }
            self.flat(p);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustyfi_syntax::{RustyfiVersion, Token};

    /// Format under slice 1's rules, or `None` if the file does not parse.
    fn fmt(src: &str) -> Option<String> {
        super::super::format_cst(src, RustyfiVersion::V0_0, &super::super::CstOptions::default())
    }

    /// [`fmt`] with slice 6 OFF, for the slice-4 claims that are about what
    /// happens to a gap the re-wrap is not allowed to touch.
    fn fmt_no_wrap(src: &str) -> Option<String> {
        super::super::format_cst(
            src,
            RustyfiVersion::V0_0,
            &super::super::CstOptions {
                wrap_inline_text: false,
                ..Default::default()
            },
        )
    }

    /// [`fmt`] at an explicit column budget, which is what slice 6's
    /// decisions are a function of.
    #[allow(dead_code)]
    fn fmt_at(src: &str, max_width: usize) -> Option<String> {
        super::super::format_cst(
            src,
            RustyfiVersion::V0_0,
            &super::super::CstOptions { max_width, ..Default::default() },
        )
    }

    fn parsed(src: &str) -> (Vec<Atom>, cst::File) {
        let atoms = rustyfi_syntax::lex_with_version(src, RustyfiVersion::V0_0).expect("lexes");
        let file = rustyfi_syntax::cst::parse_file(src).expect("parses");
        (atoms, file)
    }

    /// The census that scopes [`Spacing::universal`] and every exception on
    /// it — every gap the walk sees in the bundled corpus, by area, by what
    /// the author wrote, and by which exception (if any) claims it.
    ///
    /// `#[ignore]`d because it reads the corpus off disk and prints rather
    /// than asserts: it is the harness for a NUMBER, and the number belongs
    /// in the doc comment of the rule that rests on it. Run it with
    ///
    /// ```text
    /// cargo test -p rustyfi-lsp --lib gap_census -- --ignored --nocapture
    /// ```
    ///
    /// whenever an exception is added, removed or re-scoped. A rule justified
    /// by a count nobody can re-take is a rule whose justification has
    /// quietly expired.
    #[test]
    #[ignore = "prints a census over the corpus; run it when an exception changes"]
    fn the_v006_gap_census() {
        let mut c = super::Census::default();
        for path in super::corpus_files(&["lib-rustyfi/dist/packages", "layout-tests/corpus"]) {
            let Ok(src) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Ok(atoms) = rustyfi_syntax::lex_with_version(&src, RustyfiVersion::V0_0) else {
                continue;
            };
            let Ok(file) = rustyfi_syntax::cst::parse_file(&src) else {
                continue;
            };
            c.add(&path, &src, &atoms, gap_census(&src, &atoms, &file));
        }
        c.report("0.0.6");
    }

    /// **`LineBreaks::Preserve` is gone, and this is what says so.**
    ///
    /// It was an enum with two variants and a guard at the top of [`build`]
    /// that declined the second; slice 3 deleted both, because a rule that
    /// both reads the author's breaks and overrides some of them is not
    /// idempotent and cannot be made so (`engine.md` section 6, hazard class
    /// 1). What replaces the switch is [`Breaks`], which chooses between "the
    /// renderer decides here" and "there is no break here" — never between
    /// "the renderer decides" and "the author decided".
    ///
    /// The property that would have been lost with the enum: with EVERY
    /// construct's break flag off, the author's line breaks still do not
    /// survive. `NO_BREAKS` is not `Preserve`.
    #[test]
    fn with_every_construct_off_the_authors_breaks_still_do_not_survive() {
        let src = "let x =\n  1\nin\nx\n";
        let (atoms, file) = parsed(src);
        let doc = build(src, &atoms, &file, 2, NO_BREAKS, None, false, DEFAULT_MAX_BLANK_LINES)
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
        assert_eq!(out, "let x = 1 in x\n");
    }

    #[test]
    fn the_walk_stays_in_step_with_the_atom_stream() {
        // If this drifts, indentation is attributed to the wrong depth and
        // nothing else in the suite would notice: the token stream is
        // identical either way.
        for src in [
            "@require: stdja\nlet x = 1\nin\nx\n",
            "module M : sig val f : int -> int end = struct let f x = x end\n",
            "let-rec f\n| 0 = 1\n| n = n * f (n - 1)\nin f 3\n",
            "type t =\n  | A\n  | B of int\nin\n()\n",
            "let x = (| a = 1; b = 2 |) in x#a\n",
            "let-mutable r <- 0\nin\nlet () = r <- 1 in\n!r\n",
            "let x = match y with | A -> 1 | B when c -> 2 in x\n",
            "let-inline ctx \\foo a b = read-inline ctx a\nin ()\n",
            "let f = fun ?(x = a) y -> a in f ?(x = 1) 2\n",
            "let x = [1; 2; 3] |> List.map (fun n -> n * 2) in x\n",
            "let x = if a then b else c in while a do b\n",
            "let x = Mod.(y) in x before ()\n",
            "let x = 1 in ${ \\frac{1}{2} }\n",
            "let-block ctx +p it = '< +q{a} > in ()\n",
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

    /// `stdja.satyh:159-161` — `engine.md` section 12's example (4), the one it
    /// calls "slice 1, no gates, real bug".
    #[test]
    fn a_binding_at_the_wrong_indentation_is_moved_to_its_siblings_column() {
        let before = "\
module StdJa = struct
  let a = 1
let title-deco =
    let pads = (5pt, 5pt, 10pt, 10pt) in
      pads
end
in ()
";
        // At the default budget the whole binding fits on one line and the
        // claim is only about the `let` itself. **At 40 columns the body
        // breaks**, which is what makes this a test about indentation rather
        // than about joining — the `let` moves to its siblings' column AND its
        // body lands one step past it, not five.
        let after = "module StdJa = struct\n  let a = 1\n  let title-deco =\n    let pads = (5pt, 5pt, 10pt, 10pt) in\n    pads\nend\nin\n()\n";
        assert_eq!(fmt_at(before, 40).as_deref(), Some(after));
        assert_eq!(fmt_at(after, 40).as_deref(), Some(after), "not a fixpoint");
    }

    /// A nested `module … : sig … = struct`: two depths of items, and the
    /// `sig`/`struct` keywords at the module's own column.
    #[test]
    fn a_nested_module_indents_both_its_sig_and_its_struct() {
        let before = "\
module Outer = struct
        module Inner
: sig
val f : int -> int
end
= struct
let f x = x
end
end
in ()
";
        let after = "module Outer = struct\n  module Inner : sig\n    val f : int -> int\n  end = struct\n    let f x = x\n  end\nend\nin\n()\n";
        assert_eq!(fmt(before).as_deref(), Some(after));
    }

    /// A multi-clause `let-rec`: `engine.md` section 12's example (3). The
    /// clauses go one step in from the keyword and the alignment before `=`
    /// survives — slice 2's `=` rule is a RECORD FIELD's `=`, and this is not
    /// one. 33 corpus lines write this table; flattening it was measured and
    /// declined ([`Spacing::record_eq`]).
    #[test]
    fn a_let_rec_with_bar_clauses_indents_its_clauses_one_step() {
        let before = "\
module M = struct
    let-rec map
      | f []        = []
      | f (x :: xs) = (f x) :: map f xs
end
in ()
";
        let after = "module M = struct\n  let-rec map\n    | f [] = []\n    | f (x :: xs) = (f x) :: map f xs\nend\nin\n()\n";
        assert_eq!(fmt(before).as_deref(), Some(after));
    }

    /// The spine property: eleven `let … in` at one depth stay at one depth.
    /// A recursive printer would stair-step them to twenty-two columns.
    #[test]
    fn a_deep_let_in_chain_does_not_stair_step() {
        let mut before = String::from("let f x =\n");
        for i in 0..11 {
            before.push_str(&format!("      let a{i} = {i} in\n"));
        }
        before.push_str("  x\nin ()\n");

        let mut want = String::from("let f x =\n");
        for i in 0..11 {
            want.push_str(&format!("  let a{i} = {i} in\n"));
        }
        want.push_str("  x\nin\n()\n");

        assert_eq!(fmt(&before).as_deref(), Some(want.as_str()));
    }

    // -- the properties, on shapes a corpus file may not contain -------------

    #[test]
    fn every_program_area_gap_is_claimed_whatever_shape_it_sits_in() {
        // **The inversion, and the four reported defects.** Every one of
        // these was "unchanged" before, because no shape in the rule list
        // named it: a keyword boundary has no operator, a parameter run has
        // no separator, and a `;` in an active area is reached through a
        // different lexer mode. None of them is a shape now.
        for (src, want, why) in [
            ("let x   =    1   in    x\n", "let x = 1\nin\nx\n", "around `in`"),
            ("let f  x  y = x in f 1 2\n", "let f x y = x\nin\nf 1 2\n", "a parameter run"),
            ("if 1 > 0    then 1 else 2\n", "if 1 > 0 then 1 else 2\n", "before `then`"),
            ("open    Derive in 1\n", "open Derive\nin\n1\n", "after `open`"),
            (
                "match 1 with    | _ -> 2\n",
                "match 1 with\n| _ -> 2\n",
                "after `with` — the one the old `bar` rule happened to claim",
            ),
            (
                "let x = '<+p{ \\code(`c`)  ; }> in x\n",
                "let x = '< +p{ \\code(`c`); } >\nin\nx\n",
                "an ACTIVE-area `;`, which no program-area rule could reach",
            ),
        ] {
            assert_eq!(fmt(src).as_deref(), Some(want), "{why}");
            assert_eq!(fmt(want).as_deref(), Some(want), "{why}: not a fixpoint");
        }
    }

    /// Math and block text: the default reaches them, and it **collapses**
    /// rather than inserting.
    ///
    /// Their whitespace produces no token — `lex_math` (`lexer.rs:1338-1340`)
    /// and `lex_vertical` (`:1029-1032`) bump past it without emitting — and
    /// it is invisible to the typesetter: `${x   +   y}`, `${x + y}` and
    /// `${x+y}` compile to byte-identical PDFs. So the gap is as free as a
    /// program-area gap.
    ///
    /// What it is NOT is one space. The census says math writes 2,789 of its
    /// 3,608 reachable gaps tight and block text 152 of 225, so `${abc}` is
    /// the corpus's own form and inserting 557 spaces into hand-written math
    /// would be a rewrite rather than a canonicalisation. [`Space::Collapse`]
    /// is the part both halves agree on, and it is why the corpus impact of
    /// the math extension is 12 gaps and of the block extension is none.
    #[test]
    fn math_and_block_gaps_collapse_but_never_gain_a_space() {
        for (src, want, why) in [
            ("let x = 1 in ${x   +   y}\n", "let x = 1\nin\n${x + y}\n", "a run collapses"),
            ("let x = 1 in ${a  b  c}\n", "let x = 1\nin\n${a b c}\n", "and so does a bare one"),
            ("let x = 1 in ${abc}\n", "let x = 1\nin\n${abc}\n", "but nothing is INSERTED"),
            (
                "let x = 1 in ${   x + y   }\n",
                "let x = 1\nin\n${x + y}\n",
                "the `${`/`}` boundary is a bracket, and brackets are tight",
            ),
            (
                "let x = 1 in ${x ^  2}\n",
                "let x = 1\nin\n${x^2}\n",
                "a script marker binds to its operand",
            ),
            (
                "let x = 1 in '<   +p{hi}   >\n",
                "let x = 1\nin\n'< +p{hi} >\n",
                "block text, same rule",
            ),
        ] {
            assert_eq!(fmt(src).as_deref(), Some(want), "{why}");
            assert_eq!(fmt(want).as_deref(), Some(want), "{why}: not a fixpoint");
        }
        // `_` and `'` are the two script markers the script rule asks to
        // tighten and [`sep::must_separate`] refuses, because both are WORD
        // characters in program mode (`x_i`, `x'` are identifiers) and the
        // table is mode-blind. It is the only fusion authority and it is not
        // overruled: the request is made, the table says separate, and one
        // space is written. Two gaps in the whole corpus.
        assert_eq!(fmt("let x = 1 in ${x _  i}\n").as_deref(), Some("let x = 1\nin\n${x _ i}\n"));
        // Inline text nested back INSIDE math (`!{ … }`) keeps its own
        // predicate — math's freedom must not leak across that boundary, and
        // the area fold has been wrong at exactly this kind of boundary
        // before (`area.rs:47-65`, the `<[`/`]>` case).
        let nested = "let x = 1\nin\n${\\foo!{a  b}}\n";
        assert_eq!(fmt_no_wrap(nested).as_deref(), Some(nested), "math reached into inline text");
    }

    /// **Only one of the inversion's two guards survives slice 3, and the
    /// other one flipped on purpose.**
    ///
    /// A gap holding a `%` COMMENT still refuses every request — the run
    /// around it is the author's, byte for byte, and the comment brings its own
    /// line structure. A gap holding a LINE BREAK no longer does: slice 1 read
    /// the break and reproduced it, and slice 3 discards it and puts back
    /// whatever the construct asked for. That is `LineBreaks::Preserve`'s
    /// removal seen from the gap, and there is no branch left that could
    /// reproduce an author's break.
    #[test]
    fn a_comment_in_the_gap_still_refuses_every_request_and_a_break_no_longer_does() {
        // A break the author wrote where no construct offers one: JOINED. The
        // `=`'s body group fits on the line, so it takes it.
        assert_eq!(
            fmt("let x =\n  1\nin x\n").as_deref(),
            Some("let x = 1\nin\nx\n")
        );
        // A comment: the run in front of it is the author's, INCLUDING the two
        // spaces a spacing rule would otherwise collapse, and the line ends
        // after it whatever the width.
        assert_eq!(
            fmt("let x = 1  % why\nin x\n").as_deref(),
            Some("let x = 1  % why\nin\nx\n")
        );
    }

    /// A **single-line** text or math area is still byte-identical **under
    /// slice 4**, and there that is a construction rather than a policy:
    /// nothing in these areas holds a line terminator, so no indentation
    /// exists to recompute (`Build::emit_swallowed`'s early return, and
    /// `Build::gap_upto`'s break-free arm, which copies).
    ///
    /// Slice 6 narrows the claim rather than keeping it, which is why this
    /// test asks [`fmt_no_wrap`]: `{a  b}` is a reflowable gap and the
    /// re-wrap writes every reflowable gap as exactly one space. That is
    /// licensed by rule 1 of the measurement — a run's LENGTH is free
    /// everywhere, 123 of 123 — because the lexer collapses the whole run to
    /// one token and `elaborate.rs:2989-2990` maps it to a single `' '`. The
    /// gaps the re-wrap must NOT touch are the swallowed edges (`{  a  }`)
    /// and the bullet/separator runs, and those are still byte-identical
    /// with the re-wrap ON — asserted below.
    #[test]
    fn a_single_line_text_or_math_area_is_still_byte_identical() {
        // NOT math and NOT block text any more, and that is the point of
        // bringing them inside the default — see
        // `math_and_block_gaps_collapse_but_never_gain_a_space`. **Inline
        // text is the one area left**, and it is the one whose whitespace is
        // a token.
        for src in [
            "let x = 1\nin\n{a  b}\n",
            // Whitespace at a group's edges, which the lexer swallows into the
            // delimiter's own span (`lexer.rs:562-567`, `:1122-1126`).
            "let x = 1\nin\n{  a  }\n",
            // An itemize bullet and a `|` separator, which swallow the run in
            // front of them.
            "let x = 1\nin\n{* a * b}\n",
            "let x = 1\nin\n{a | b}\n",
        ] {
            assert_eq!(fmt_no_wrap(src).as_deref(), Some(src), "{src:?}");
        }
        // With the re-wrap ON, everything the lexer swallowed into a
        // delimiter, a bullet or a separator is STILL byte-identical: those
        // runs are not `Space`/`Break` tokens, so `Build::inline_gap` never
        // sees them.
        for src in [
            "let x = 1\nin\n{  a  }\n",
            "let x = 1\nin\n{* a * b}\n",
            "let x = 1\nin\n{a | b}\n",
        ] {
            assert_eq!(fmt(src).as_deref(), Some(src), "re-wrap on: {src:?}");
        }
    }

    /// **Slice 4's one visible side effect on a single line**: the program
    /// text a command's arguments are made of is now reachable, so slice 2's
    /// bracket rule reaches it too.
    ///
    /// This case used to be pinned as a no-op, with the comment "a program
    /// sub-area *inside* inline text: reachable in slice 4, not here". It is
    /// slice 4, and the prediction was right. Note what still does NOT move:
    /// the `{a}` argument's own bytes, and the tightness of `\frame` against
    /// its first argument.
    #[test]
    fn a_commands_argument_is_program_text_and_slice_2_now_reaches_it() {
        assert_eq!(
            fmt("let x = 1 in {\\frame(2pt)(   1pt   ){a}}\n").as_deref(),
            Some("let x = 1\nin\n{\\frame(2pt)(1pt){a}}\n")
        );
    }

    // -- slice 4: the three text areas ---------------------------------------

    /// **Rule 2a, block text.** `'< … >` gets real layout: its `+cmd` items
    /// sit one step in from the line the `'<` is on, a nested `<%` block one
    /// step further, and every closer lands at its opener's column.
    ///
    /// Provably token-preserving: `lex_vertical` emits no token for whitespace
    /// or comments (`lexer.rs:1029-1032`) and `ast::BlockElem` has no
    /// whitespace variant at all, so every byte moved here is a **gap**.
    #[test]
    fn block_text_indents_its_items_and_its_nested_blocks() {
        let before = "\
let d = document (| a = 1 |) '<%
+p{
hello
}
+q{x}<%
+r{y}
>%
>%
in ()
";
        let after = "let d = document (| a = 1 |) '<%\n  +p{\n    hello\n  }\n  +q{x}<%\n    +r{y}\n  >%\n>%\nin\n()\n";
        assert_eq!(fmt(before).as_deref(), Some(after));
        assert_eq!(fmt(after).as_deref(), Some(after), "not a fixpoint");
    }

    /// **Rule 2b, math.** Same argument, same mechanism: `lex_math` skips
    /// whitespace and comments without emitting (`lexer.rs:1338-1340`).
    #[test]
    fn math_keeps_its_line_structure_and_re_indents_it() {
        // The author's breaks SURVIVE, and only the indentation is recomputed.
        let before = "\
let m = ${
\\frac{1}{2}
      + x
} in m
";
        let after = "let m = ${\n  \\frac{1}{2}\n  + x\n}\nin\nm\n";
        assert_eq!(fmt(before).as_deref(), Some(after));
        assert_eq!(fmt(after).as_deref(), Some(after), "not a fixpoint");
        // **Math is atomic for line-breaking in BOTH directions**
        // ([`Build::math_text`]): no break is invented and none is removed.
        // Those are one decision — a rule that refuses to add breaks but
        // deletes the author's turns every hand-laid-out equation into one
        // long line and leaves no way to lay it out again.
        //
        // Neither half moves at any width. The two reported equations are in
        // the list: one broke inside `\\frac`'s argument group and one inside
        // `e^{-x^2}`.
        for src in [
            // Written flat: stays flat, however narrow the budget.
            "let m = ${\\frac{n \\paren{n + 1}}{6} + \\alpha \\beta \\gamma}\nin\nm\n",
            "let m = ${e^{-x^2} + \\alpha \\beta \\gamma \\delta}\nin\nm\n",
            "let m = ${aaaa+bbbb+cccc+dddd}\nin\nm\n",
            // Written broken: stays broken, however wide the budget.
            "let m = ${\n  \\frac{1}{2}\n  + x\n}\nin\nm\n",
            "let m = ${\n  a\n  + b\n  + c\n}\nin\nm\n",
        ] {
            for w in [8usize, 20, 40, 1000] {
                assert_eq!(
                    fmt_at(src, w).as_deref(),
                    Some(src),
                    "a math area moved at width {w}"
                );
            }
        }
        // A blank line inside math survives too, capped like any other.
        let blanks = "let m = ${\n  a\n\n  + b\n}\nin\nm\n";
        assert_eq!(fmt(blanks).as_deref(), Some(blanks));
        // What DOES still move: an intra-line run collapses, and an empty gap
        // stays empty. `Space::Collapse` is math's canonical spelling, and it
        // is a claim about a RUN rather than about a line.
        assert_eq!(
            fmt("let m = ${x   +   y}\nin\nm\n").as_deref(),
            Some("let m = ${x + y}\nin\nm\n")
        );
        assert_eq!(
            fmt("let m = ${abc}\nin\nm\n").as_deref(),
            Some("let m = ${abc}\nin\nm\n")
        );
    }

    /// **Rule 2c, inline text.** Continuation lines are re-indented and every
    /// line break the author wrote survives — which is the whole predicate,
    /// because a whitespace run is ONE token and the run's first character is
    /// what decides whether it is a `Space` or a `Break`.
    ///
    /// Latin and CJK are both here because they are not the same measurement:
    /// `ground-truth-whitespace.md`'s I24 re-indents a CJK continuation line
    /// and gets an EQUAL PDF, which is the case the predicate was doubted for.
    #[test]
    fn inline_text_re_indents_its_continuation_lines_in_both_scripts() {
        for (before, after, script) in [
            (
                "let t = {\nalpha\n      beta\n   gamma\n} in t\n",
                "let t = {\n  alpha\n  beta\n  gamma\n}\nin\nt\n",
                "latin",
            ),
            (
                "let t = {\n日本語の文章を\n      書きます\n} in t\n",
                "let t = {\n  日本語の文章を\n  書きます\n}\nin\nt\n",
                "cjk",
            ),
            // An itemize bullet swallows the run in front of it, so it is
            // re-indented by the same mechanism rather than by a rule of its
            // own.
            (
                "let t = {\n* one\n     * two\n} in t\n",
                "let t = {\n  * one\n  * two\n}\nin\nt\n",
                "an itemize bullet",
            ),
        ] {
            // [`fmt_no_wrap`], because this is slice 4's claim: with slice 6
            // on, the latin arm's three lines are joined to
            // `{\n  alpha beta gamma\n}` and the itemize bullets are
            // untouched (they are not whitespace tokens at all). The CJK arm
            // is unchanged either way, which
            // [`the_cjk_arm_of_the_re_indent_test_is_frozen_by_slice_6_too`]
            // asserts against the live default.
            assert_eq!(fmt_no_wrap(before).as_deref(), Some(after), "{script}");
            assert_eq!(fmt_no_wrap(after).as_deref(), Some(after), "{script}: not a fixpoint");
        }
    }

    /// The CJK half of the test above, against the LIVE default — because
    /// "slice 4 does not move it" and "slice 6 is not allowed to move it" are
    /// two different claims and only the second one is at risk now.
    #[test]
    fn the_cjk_arm_of_the_re_indent_test_is_frozen_by_slice_6_too() {
        let after = "let t = {\n  日本語の文章を\n  書きます\n}\nin\nt\n";
        assert_eq!(fmt("let t = {\n日本語の文章を\n      書きます\n} in t\n").as_deref(), Some(after));
        assert_eq!(fmt(after).as_deref(), Some(after), "not a fixpoint");
        // And at a width that would obviously join it if anything could.
        assert_eq!(fmt_at(after, 200).as_deref(), Some(after), "joined a frozen gap");
        // An itemize bullet swallows its run, so slice 6 never sees it.
        let bullets = "let t = {\n  * one\n  * two\n}\nin\nt\n";
        assert_eq!(fmt(bullets).as_deref(), Some(bullets));
    }

    /// **The one mistake rule 2c could make**, pinned on both sides.
    ///
    /// `Space` and `Break` are the same run of characters and differ only in
    /// whether the first one is a newline (`lexer.rs:1152`). A run that starts
    /// with a space must keep one — trimming the trailing whitespace of
    /// `"a  \n  b"` would make it a `Break`, which `ground-truth-whitespace.md`
    /// measures as a DIFFERENT PDF for CJK (I25). And a run must never be
    /// emptied.
    ///
    /// The formatter's own verifier would decline rather than return a wrong
    /// answer, so the failure mode this test guards is `None`, not a wrong
    /// string — which is why the expectation is spelled out rather than
    /// asserted as "some output".
    #[test]
    fn a_whitespace_run_keeps_its_newline_and_is_never_emptied() {
        // [`fmt_no_wrap`]: this is the invariant slice 4 holds for EVERY gap,
        // and slice 6 holds for every gap it freezes. The frozen half is
        // asserted against the live default by
        // [`a_frozen_run_still_keeps_its_newline_under_the_re_wrap`].
        for (before, after, what) in [
            // Starts with a space and holds a newline: exactly ONE space
            // survives in front of the break, so the token is still a `Space`.
            ("let t = {a  \n      b} in t\n", "let t = {a \n  b}\nin\nt\n", "a `Space` run"),
            ("let t = {日本  \n      語} in t\n", "let t = {日本 \n  語}\nin\nt\n", "a CJK `Space` run"),
            // Starts with a newline: no leading space is invented, or it would
            // become a `Space`.
            ("let t = {a\n      b} in t\n", "let t = {a\n  b}\nin\nt\n", "a `Break` run"),
            // No newline at all: copied, and in particular not emptied and not
            // given one.
            ("let t = {a  b}\nin\nt\n", "let t = {a  b}\nin\nt\n", "a single-line run"),
        ] {
            assert_eq!(fmt_no_wrap(before).as_deref(), Some(after), "{what}");
            assert_eq!(fmt_no_wrap(after).as_deref(), Some(after), "{what}: not a fixpoint");
        }
    }

    /// The same invariant, on the gaps slice 6 is not allowed to move, and
    /// against the LIVE default.
    ///
    /// A `Space` between two CJK characters is the sharpest case in the
    /// whole feature: the run starts with a space and holds a newline, so
    /// dropping the leading space would make it a `Break` and joining the two
    /// lines would make it a bare `Space` — and I25 measures the first as
    /// DIFFERING while the predicate refuses the second. One space, one
    /// newline, exactly where the author put them.
    #[test]
    fn a_frozen_run_still_keeps_its_newline_under_the_re_wrap() {
        for (before, after, what) in [
            ("let t = {日本  \n      語} in t\n", "let t = {日本 \n  語}\nin\nt\n", "a CJK `Space` run"),
            ("let t = {日本\n      語} in t\n", "let t = {日本\n  語}\nin\nt\n", "a CJK `Break` run"),
        ] {
            assert_eq!(fmt(before).as_deref(), Some(after), "{what}");
            assert_eq!(fmt(after).as_deref(), Some(after), "{what}: not a fixpoint");
            // At a width that would join it in a heartbeat if it could.
            assert_eq!(fmt_at(after, 200).as_deref(), Some(after), "{what}: joined");
        }
    }

    /// A `%` comment inside inline text is **not** reflowed, and keeps its own
    /// column.
    ///
    /// Slice 4 made it reachable, which slices 0-3 did not: a comment that
    /// directly abuts a token in horizontal mode is skipped by
    /// `lex_horizontal`'s own `'%'` arm (`lexer.rs:1106-1109`), which re-takes
    /// `start` and leaves it in a GAP. Reflowing it there is token-safe and is
    /// still rewriting somebody's prose — `easytable.saty` is the file that
    /// found it.
    #[test]
    fn a_comment_inside_inline_text_is_neither_reflowed_nor_moved() {
        let opts = super::super::CstOptions {
            wrap_comments: true,
            max_width: 20,
            ..super::super::CstOptions::default()
        };
        let src = "\
let t = {
alpha% and a very long trailing comment that would be wrapped in program text
beta
} in t
";
        let want = "let t = {\n  alpha% and a very long trailing comment that would be wrapped in program text\n  beta\n}\nin\nt\n";
        let got = super::super::format_cst(src, RustyfiVersion::V0_0, &opts);
        assert_eq!(got.as_deref(), Some(want));
    }

    /// The **anchor** [`Build::group_open_as`] added, on the shape that needed
    /// it and on the shape that must not move.
    ///
    /// A line that begins with a closing delimiter renders shallower than
    /// [`Build::level`] says, because [`Build::group_close`] pushed a negative
    /// frame for it. `enter` did not know that, so a group opened on such a
    /// line indented its contents one step too far. Two of them in the corpus:
    /// `document (| … |) '< … >` (the block text after the record's ` |)`) and
    /// `easytable`'s `fun i row -> (` nested inside another `(`.
    #[test]
    fn a_group_opened_on_a_line_that_begins_with_a_closer_still_takes_one_step() {
        // The `(` opens on the `fun i row -> (` line, which is itself the
        // continuation of a group; before the anchor its body came out at the
        // enclosing column instead of one step in.
        let src = "let f xs = xs |> List.map (fun i row -> (row |> g))\nin\n()\n";
        assert_eq!(fmt(src).as_deref(), Some(src), "a correct nesting must be a no-op");
        // And the block-text form, where the `'<` sits after the record's ` |)`
        // on a line rendered at column 0.
        let before = "let d = document (| \n  a = 1;\n |) '<\n+p{x}\n>\nin ()\n";
        let after = "let d = document (| a = 1; |) '< +p{x} >\nin\n()\n";
        assert_eq!(fmt(before).as_deref(), Some(after));
        assert_eq!(fmt(after).as_deref(), Some(after), "not a fixpoint");
    }

    #[test]
    fn a_blank_line_survives_and_a_run_of_them_is_capped() {
        let before = "let a = 1\n\nlet b = 2\n\n\n\n\nlet c = 3\nin ()\n";
        let after = "let a = 1\n\nlet b = 2\n\n\nlet c = 3\nin\n()\n";
        assert_eq!(fmt(before).as_deref(), Some(after));
    }

    /// [`fmt`] at an explicit blank-line cap — the option
    /// [`Breaks::block_blanks`] is a function of.
    fn fmt_blanks(src: &str, max_blank_lines: usize) -> Option<String> {
        super::super::format_cst(
            src,
            RustyfiVersion::V0_0,
            &super::super::CstOptions { max_blank_lines, ..Default::default() },
        )
    }

    /// **The same rule, one area in.** A blank line the author put between two
    /// `'< … >` items survives, and it is capped by the same option that caps
    /// a top-level one.
    ///
    /// The layout consequence is the point of the `+p{aaa} +p{bbb}` control
    /// below: without the rule those two items fit flat, so the blank line was
    /// not merely capped away, the whole block came back on one line. A blank
    /// line therefore forces the item list open — a [`Br::Hard`] forces every
    /// enclosing [`Mode::Auto`] group ([`Doc::forces_break`]) — which is why
    /// the fourth and fifth items go one per line here too.
    #[test]
    fn a_blank_line_between_two_block_text_items_survives_and_is_capped() {
        // No blank line: the corpus's own flat form, unchanged.
        let flat = "let d = document (| a = 1 |) '< +p{aaa} +p{bbb} >\nin\n()\n";
        assert_eq!(
            fmt("let d = document (| a = 1 |) '<\n+p{aaa}\n+p{bbb}\n>\nin ()\n").as_deref(),
            Some(flat),
            "without a blank line a short block still flattens",
        );
        // One blank line, and a four-terminator run capped to the default 2.
        let before = "\
let d = document (| a = 1 |) '<
+p{aaa}

+p{bbb}



+p{ccc}
+p{ddd}
>
in ()
";
        let after = "\
let d = document (| a = 1 |) '<
  +p{aaa}

  +p{bbb}


  +p{ccc}
  +p{ddd}
>
in
()
";
        assert_eq!(fmt(before).as_deref(), Some(after));
        assert_eq!(fmt(after).as_deref(), Some(after), "not a fixpoint");
        // The cap is the OPTION, not a 2 in this file: at 1 the same input
        // comes back with one blank line at each of the two positions.
        let at_one = "\
let d = document (| a = 1 |) '<
  +p{aaa}

  +p{bbb}

  +p{ccc}
  +p{ddd}
>
in
()
";
        assert_eq!(fmt_blanks(before, 1).as_deref(), Some(at_one));
        assert_eq!(fmt_blanks(at_one, 1).as_deref(), Some(at_one), "not a fixpoint at 1");
    }

    /// **`--max-blank-lines 0` collapses them in block text too**, and the
    /// collapse is a fixpoint.
    ///
    /// The fixpoint half is the whole reason [`Build::max_blank_lines`] is a
    /// builder field rather than a renderer-only clamp. Were the rule to fire
    /// at a zero cap, the first pass would hard-break these two items apart
    /// and the renderer would then delete the blank line that justified it —
    /// leaving a second pass that reads no blank line, offers a mere
    /// [`Br::Opportunity`], and joins them back. Two passes, two files.
    #[test]
    fn a_zero_cap_collapses_a_block_texts_blank_lines_and_settles() {
        let before = "let d = document (| a = 1 |) '<\n+p{aaa}\n\n\n+p{bbb}\n>\nin ()\n";
        let after = "let d = document (| a = 1 |) '< +p{aaa} +p{bbb} >\nin\n()\n";
        assert_eq!(fmt_blanks(before, 0).as_deref(), Some(after));
        // The fixpoint asserted from the AUTHOR'S text rather than from the
        // settled form: a rule that fired at a zero cap would come to rest
        // eventually, and only the first-pass output tells the two apart.
        let once = fmt_blanks(before, 0).expect("laid out");
        assert_eq!(fmt_blanks(&once, 0).as_deref(), Some(once.as_str()), "not a fixpoint at 0");
        // A zero cap does the same to a top-level blank line, which is what
        // says the two rules answer to one option.
        let top = "let a = 1\n\nlet b = 2\nin ()\n";
        assert_eq!(fmt_blanks(top, 0).as_deref(), Some("let a = 1\nlet b = 2\nin\n()\n"));
    }

    /// **A blank line against either `'< … >` delimiter is dropped.**
    ///
    /// The request did not settle this; the answer is the one
    /// [`super::render::flush_blanks`] already gives a blank line at the top
    /// of a file. A blank line is a separator between two things the printer
    /// reproduces, and at a delimiter there is only one thing — so `'<`
    /// followed by a blank line is a blank line with nothing on one side of
    /// it. Keeping it would also mean deciding what a blank line *before* `>`
    /// separates from, and the honest answer is nothing.
    ///
    /// The interior blank line in the same fixture is what makes this a claim
    /// about the delimiters rather than about the rule being off.
    #[test]
    fn a_blank_line_against_either_block_delimiter_is_dropped() {
        let before = "\
let d = document (| a = 1 |) '<

+p{aaa}

+p{bbb}

>
in ()
";
        let after = "\
let d = document (| a = 1 |) '<
  +p{aaa}

  +p{bbb}
>
in
()
";
        assert_eq!(fmt(before).as_deref(), Some(after));
        assert_eq!(fmt(after).as_deref(), Some(after), "not a fixpoint");
    }

    /// A blank line between two `#var;` embeds, and between an embed and a
    /// `+cmd`: [`ast::BlockElem`] has two variants and the rule is written
    /// over the gap rather than over either of them, so both boundaries are
    /// pinned rather than the one the fixtures happened to reach.
    #[test]
    fn a_blank_line_survives_between_block_embeds_too() {
        let before = "let d = document (| a = 1 |) '<\n#x;\n\n#y;\n\n+p{a}\n>\nin ()\n";
        let after = "\
let d = document (| a = 1 |) '<
  #x;

  #y;

  +p{a}
>
in
()
";
        assert_eq!(fmt(before).as_deref(), Some(after));
        assert_eq!(fmt(after).as_deref(), Some(after), "not a fixpoint");
    }

    /// A blank line inside a **nested** block text, and one in the outer block
    /// around it: the rule is per-area rather than per-file, and the nested
    /// items keep their own depth.
    #[test]
    fn a_blank_line_survives_inside_a_nested_block_text() {
        let before = "\
let d = document (| a = 1 |) '<
+p{a}

+q{x}<
+r{y}

+r{z}
>
>
in ()
";
        let after = "\
let d = document (| a = 1 |) '<
  +p{a}

  +q{x}<
    +r{y}

    +r{z}
  >
>
in
()
";
        assert_eq!(fmt(before).as_deref(), Some(after));
        assert_eq!(fmt(after).as_deref(), Some(after), "not a fixpoint");
    }

    /// **Change 2, variant B.** An own-line comment keeps whatever indentation
    /// the author gave it — column 0, block depth, or deeper — while a
    /// trailing comment keeps its intra-line spacing, as before.
    ///
    /// The rule this replaced re-indented the first comment here to block
    /// depth. See [`Build::own_line_comment`] for the corpus measurement that
    /// chose between them; the `%`-disabled code at column 0 in the third
    /// comment is the shape it is protecting, and it is the one the old rule
    /// moved furthest.
    #[test]
    fn an_own_line_comment_keeps_the_authors_indentation_whatever_it_is() {
        let before = "\
module M = struct
% about a
        let a = 1  % trailing
  % already at block depth
      % deeper than the block
end
in ()
";
        // Every comment line byte-identical to the input; only `let a = 1`
        // moves, and it moves because it is code.
        let after = "module M = struct\n% about a\n  let a = 1  % trailing\n  % already at block depth\n      % deeper than the block\nend\nin\n()\n";
        assert_eq!(fmt(before).as_deref(), Some(after));
    }

    /// The gap the first draft recorded — "a comment on its own line
    /// immediately before `end` or `in` is emitted in the enclosing frame, so
    /// it dedents to the outer depth" — is **retired** by variant B, not
    /// fixed. Such a comment is no longer emitted at any frame's depth, so
    /// which frame it landed in stopped being observable.
    #[test]
    fn a_comment_just_before_end_or_in_no_longer_dedents_because_nothing_indents_it() {
        let src = "\
module M = struct
  let a = 1
  % the last word on M
end
in
  % and on the file
()
";
        assert_eq!(fmt(src).as_deref(), Some(src));
    }

    /// Variant B reads the input's leading whitespace and writes it back, so
    /// the second pass reads what the first wrote. Asserted here as well as in
    /// the corpus sweep, on a shape that mixes both comment positions with a
    /// line the formatter *does* move.
    #[test]
    fn keeping_a_comments_own_indentation_is_a_fixpoint() {
        let before = "\
module M = struct
% flush
      let a =
      if a then
      1
      else
      2
        % trailing thought
end
in ()
";
        let once = fmt(before).expect("formats");
        let twice = fmt(&once).expect("formats its own output");
        assert_eq!(once, twice, "second pass differs");
        // And a third, because a two-cycle would pass a two-pass test.
        assert_eq!(fmt(&twice).as_deref(), Some(once.as_str()));
    }

    // -- change 1: `Build::body`'s anchor ------------------------------------

    /// The user-visible bug: `then` and `else` were asymmetric because the
    /// **condition's** frame spent the `if` line's step, so `Build::enter` had
    /// none left for `then` while `else` — which broke its own line — got one.
    #[test]
    fn an_if_indents_both_branch_bodies_that_begin_their_own_line() {
        let before = "\
let f x =
  if x > 0 then
  `pos`
  else
  `neg`
in ()
";
        // Flat when it fits; at 30 columns both branches break TOGETHER, one
        // step in, with `else` back at the `if`'s own column. One group over
        // the pair is what makes them agree — two independent bodies gave
        // ``if x > 0 then⏎  `positive` else `negative` ``.
        assert_eq!(
            fmt(before).as_deref(),
            Some("let f x = if x > 0 then `pos` else `neg`\nin\n()\n")
        );
        let wide = "let f x = if x > 0 then `positive` else `negative`\nin ()\n";
        let broken = "let f x =\n  if x > 0 then\n    `positive`\n  else\n    `negative`\nin\n()\n";
        assert_eq!(fmt_at(wide, 30).as_deref(), Some(broken));
        assert_eq!(fmt_at(broken, 30).as_deref(), Some(broken), "not a fixpoint");
    }

    /// The larger half of the same bug: it did not merely fail to add a step,
    /// it **removed** one from a `then` body the author had already indented
    /// correctly. `else` is the reference — it was right all along.
    #[test]
    fn an_if_does_not_dedent_a_then_body_the_author_already_indented() {
        let src = "let f x = if x > 0 then let y = 1 in y else 0\nin\n()\n";
        assert_eq!(fmt(src).as_deref(), Some(src), "a correct `if` must be a no-op");
    }

    /// A branch that stays on the keyword's line has no line of its own to
    /// indent, so the rule does not fire and nothing moves.
    #[test]
    fn an_if_whose_branches_stay_on_the_keyword_line_gains_no_step() {
        let src = "let f x = if x > 0 then `pos` else `neg`\nin\n()\n";
        assert_eq!(fmt(src).as_deref(), Some(src));
    }

    /// `match` arms were already right — the `|` starts its own line, so
    /// nothing had spent that line's step. They must stay right: a rule that
    /// fixed `if` by always forcing a step would double-indent these.
    #[test]
    fn a_match_arm_body_on_its_own_line_takes_exactly_one_step_from_its_arm() {
        let before = "\
let f x =
  match x with
  | A ->
  1
  | B ->
    2
  | C -> 3
in ()
";
        let after = "let f x =\n  match x with\n  | A -> 1\n  | B -> 2\n  | C -> 3\nin\n()\n";
        assert_eq!(fmt(before).as_deref(), Some(after));
        // And the arm body that does NOT fit: exactly one step past its arm,
        // whatever the arm's own width.
        let long = "let f x =\n  match x with\n  | A -> alpha-value\n  | B -> beta-value\nin ()\n";
        let want = "let f x =\n  match x with\n  | A -> alpha-value\n  | B -> beta-value\nin\n()\n";
        assert_eq!(fmt_at(long, 24).as_deref(), Some(want));
    }

    /// `fun` had `if`'s bug for `if`'s reason — the parameter list's frame
    /// spends the `fun … ->` line's step. The body is anchored to the line the
    /// `->` sits on, so a lambda that is *itself* mid-line still gets exactly
    /// one step (`a_trailing_group_argument_is_not_indented_twice` is the
    /// other half of this).
    #[test]
    fn a_fun_body_on_its_own_line_is_one_step_past_the_line_its_arrow_sits_on() {
        let before = "\
let f =
  fun x y ->
  x + y
in ()
";
        let after = "let f = fun x y -> x + y\nin\n()\n";
        assert_eq!(fmt(before).as_deref(), Some(after));
        // The same lambda that does not fit: the body takes exactly one step
        // past the `fun … ->` line, and the `let`'s own body takes one more.
        let narrow = "let f = fun x y -> x + y + 1\nin ()\n";
        let want = "let f =\n  fun x y ->\n    x + y + 1\nin\n()\n";
        assert_eq!(fmt_at(narrow, 20).as_deref(), Some(want));
        assert_eq!(fmt_at(want, 20).as_deref(), Some(want), "not a fixpoint");
    }

    /// The distinction between [`Build::body`] and [`Build::element`], which
    /// the corpus found the hard way.
    ///
    /// A list item has no introducer of its own: the `;` before it sits at the
    /// end of the *previous* item's line, so `line_level` names a sibling's
    /// line rather than this item's. Anchoring an element to it indents every
    /// item one step past the one above — `color-ext.satyg`'s colour table
    /// staircased from 2 changed lines to 296 while `body` and `element` were
    /// still the same function.
    #[test]
    fn a_sequence_element_is_not_anchored_to_the_previous_elements_line() {
        let src = "let xs = [(`a`, 1); (`b`, 2); (`c`, 3);]\nin\n()\n";
        assert_eq!(fmt(src).as_deref(), Some(src), "a list must not staircase");
        // The same shape with `,` instead of `;`, which is `paren_body`'s
        // rest-component path rather than `A::List`'s.
        let tuple = "let t = (1, 2, 3)\nin\n()\n";
        assert_eq!(fmt(tuple).as_deref(), Some(tuple), "a tuple must not staircase");
        // **And the same claim where it can actually fail**: a run that does
        // NOT fit breaks one item per line at ONE depth. Before the elements
        // shared `Build::body`'s anchor, `color-ext.satyg`'s 147-entry colour
        // table came out as a staircase — 2 changed lines became 296 — and at
        // the default budget nothing here would have noticed.
        let wide = "let xs = [(`a`, 1); (`b`, 2); (`c`, 3);]\nin ()\n";
        let want = "let xs = [\n  (`a`, 1);\n  (`b`, 2);\n  (`c`, 3);\n]\nin\n()\n";
        assert_eq!(fmt_at(wide, 20).as_deref(), Some(want));
        assert_eq!(fmt_at(want, 20).as_deref(), Some(want), "not a fixpoint");
        let wide_t = "let t = (11111, 22222, 33333)\nin ()\n";
        let want_t = "let t = (\n  11111,\n  22222,\n  33333\n)\nin\n()\n";
        assert_eq!(fmt_at(wide_t, 20).as_deref(), Some(want_t));
    }

    /// The user's third rule, pinned rather than implemented: **the
    /// continuation after a binding's `in` takes no step.** `Build::expr`'s
    /// spine loop is what holds it — every `… = … in` node is consumed
    /// iteratively into `cur` at the *same* depth instead of recursing — and
    /// `a_deep_let_in_chain_does_not_stair_step` covers the plain `let`. This
    /// covers the rest of the family, and the `in` on a line of its own, which
    /// is the shape that looked like a violation before `Build::body` landed.
    #[test]
    fn the_continuation_after_a_bindings_in_takes_no_step() {
        for src in [
            "let f x = let y = 1 in y\nin\n()\n",
            "let f x = let-mutable r <- 0 in !r\nin\n()\n",
            "let f x =\n  let-rec g\n    | 0 = 1\n    | n = n in\n  g x\nin\n()\n",
            "let f x = let (a, b) = (1, 2) in a\nin\n()\n",
            // Inside a `then` branch: the shape whose apparent post-`in`
            // over-indent was really `Build::body`'s missing step.
            "let f x = if x > 0 then let y = 1 in y else 0\nin\n()\n",
        ] {
            assert_eq!(fmt(src).as_deref(), Some(src), "{src:?} must be a no-op");
        }
    }

    #[test]
    fn tab_indentation_is_expanded_because_indentation_is_recomputed() {
        assert_eq!(fmt("\t\tlet x = 1 in x\n").as_deref(), Some("let x = 1\nin\nx\n"));
        assert_eq!(
            fmt("module M = struct\n\t\tlet a = 1\nend\nin ()\n").as_deref(),
            Some("module M = struct\n  let a = 1\nend\nin\n()\n")
        );
    }

    #[test]
    fn trailing_whitespace_dies_and_the_last_line_gets_exactly_one_terminator() {
        assert_eq!(
            fmt("let a = 1   \nin ()\n").as_deref(),
            Some("let a = 1\nin\n()\n")
        );
        // The last line goes through `render::finish`, which trims trailing
        // whitespace and writes exactly one terminator — `insertFinalNewline`
        // and `trimFinalNewlines`, the two an LSP client expects.
        assert_eq!(fmt("let x = 1 in x  ").as_deref(), Some("let x = 1\nin\nx\n"));
    }

    #[test]
    fn a_header_keeps_its_own_terminator_and_the_gap_after_it_is_blank_lines() {
        // `lex_header` swallows the line break into the token, so a builder
        // that adds one makes format-on-save append a blank line every time.
        assert_eq!(
            fmt("@require: stdja\nlet x = 1 in x\n").as_deref(),
            Some("@require: stdja\nlet x = 1\nin\nx\n")
        );
        assert_eq!(
            fmt("@require: stdja\n\n\n\n\nlet x = 1 in x\n").as_deref(),
            Some("@require: stdja\n\n\nlet x = 1\nin\nx\n")
        );
    }

    #[test]
    fn crlf_files_come_back_with_crlf() {
        let out = fmt("@require: stdja\r\nmodule M = struct\r\n\tlet a = 1\r\nend\r\nin ()\r\n")
            .expect("formats");
        assert!(!out.contains('\n') || out.contains("\r\n"));
        for (i, b) in out.as_bytes().iter().enumerate() {
            if *b == b'\n' {
                assert_eq!(out.as_bytes().get(i.wrapping_sub(1)), Some(&b'\r'), "lone LF at {i}");
            }
        }
        assert_eq!(
            out,
            "@require: stdja\r\nmodule M = struct\r\n  let a = 1\r\nend\r\nin\r\n()\r\n"
        );
    }

    #[test]
    fn a_trailing_group_argument_is_not_indented_twice() {
        // The `(fun ctx -> …)` idiom: one step from the call, not two.
        let before = "\
let f ctx =
  embed-block-top ctx (fun c ->
              form-paragraph c
                (read-inline c)
  )
in ()
";
        let after = "let f ctx = embed-block-top ctx (fun c -> form-paragraph c (read-inline c))\nin\n()\n";
        assert_eq!(fmt(before).as_deref(), Some(after));
        // **The claim, where it can fail.** At 40 columns the trailing
        // `(fun c -> …)` is HUGGED — no break in front of it, and its body one
        // step past the CALL rather than two. `format.rs:96-110` uses exactly
        // this line to argue a bracket counter would be worse than the input:
        // the last argument's body is indented one step past its function with
        // no bracket opened in between.
        let want = "let f ctx = embed-block-top ctx (\n  fun c -> form-paragraph c (\n    read-inline c\n  )\n)\nin\n()\n";
        assert_eq!(fmt_at(after, 40).as_deref(), Some(want));
        assert_eq!(fmt_at(want, 40).as_deref(), Some(want), "not a fixpoint");
    }

    /// `stdja.satyh:58-63`, verbatim: three constructs — a `let … in` body, a
    /// parenthesis, a `fun` body and a second parenthesis — all opening on one
    /// line. One indentation step between them, not three.
    ///
    /// This is the case that pins [`Build::enter`]'s one-step-per-line rule; a
    /// version that always takes the step stair-steps it, and NOTHING in the
    /// corpus sweep can see that (token identity, text areas, idempotence and
    /// the content-line count all still hold).
    #[test]
    fn constructs_stacked_on_one_line_share_a_single_indentation_step() {
        let before = "\
module M = struct
  let generate-fresh-label =
    let-mutable count <- 0 in
      (fun () -> (
        let () = count <- !count + 1 in
          `generated:` ^ (arabic (!count))
      ))
end
in ()
";
        let after = "module M = struct\n  let generate-fresh-label =\n    let-mutable count <- 0 in\n    (fun () -> (let () = count <- !count + 1 in `generated:` ^ (arabic (!count))))\nend\nin\n()\n";
        assert_eq!(fmt(before).as_deref(), Some(after));
    }

    #[test]
    fn an_operator_chain_breaks_before_the_operator_at_one_depth() {
        let before = "\
let f ctx =
  ctx |> set-a 1
      |> set-b 2
in ()
";
        // Flat when it fits — and **all of it or none of it** when it does
        // not. `cst.rs:1039-1058` flattens all ten precedence levels, so the
        // formatter cannot know where a precedence boundary is; a fill would
        // put `ctx |> set-a 1` and `|> set-b 2` on two lines and read as
        // though the chain associated that way.
        let after = "let f ctx = ctx |> set-a 1 |> set-b 2\nin\n()\n";
        assert_eq!(fmt(before).as_deref(), Some(after));
        let want = "let f ctx =\n  ctx\n    |> set-a 1\n    |> set-b 2\nin\n()\n";
        assert_eq!(fmt_at(after, 24).as_deref(), Some(want));
        assert_eq!(fmt_at(want, 24).as_deref(), Some(want), "not a fixpoint");
    }

    #[test]
    fn a_record_literal_broken_by_the_author_stays_broken_and_gets_one_step() {
        let before = "\
let paren e =
      (|
    math = 1;
        assoc = 0;
      |)
in ()
";
        // `engine.md` section 12, example (1), which is the textbook `Auto`
        // case: `progsynt.satyh:56-64` writes the SAME record broken and flat
        // four lines apart, and one rule decides both. The broken one JOINS
        // because it fits; the same record at 24 columns breaks one field per
        // line with `(|` hugged onto the `=`'s line and `|)` back at column 0.
        let after = "let paren e = (| math = 1; assoc = 0; |)\nin\n()\n";
        assert_eq!(fmt(before).as_deref(), Some(after));
        let want = "let paren e = (|\n  math = 1;\n  assoc = 0;\n|)\nin\n()\n";
        assert_eq!(fmt_at(after, 24).as_deref(), Some(want));
        assert_eq!(fmt_at(want, 24).as_deref(), Some(want), "not a fixpoint");
    }

    #[test]
    fn a_file_that_does_not_parse_is_whitespace_tidied_rather_than_re_indented() {
        // Tier 2 in `engine.md` section 8 is `crate::format`, and it IS wired
        // now. So this file is not re-indented — no tree, no layout rule — but
        // it is not returned untouched either: the tab expands to
        // `CstOptions::tab_spaces` columns and the trailing whitespace goes,
        // which is precisely what the whitespace-only formatter is safe to do
        // on a buffer that does not parse.
        //
        // The identity that used to be asserted here was the silent-failure
        // defect: it is byte-for-byte what an ALREADY FORMATTED file looks
        // like, so `--check` could not tell them apart.
        let src = "\t\tlet x = in x  \n";
        assert_eq!(fmt(src).as_deref(), Some("    let x = in x\n"));
    }

    // -- slice 2, change 1: record spacing ----------------------------------

    /// The user's own example, verbatim: one space around a record field's
    /// `=`, and no space before a `;`.
    #[test]
    fn a_record_literal_gets_one_space_around_eq_and_none_before_semi() {
        assert_eq!(
            fmt("let x = (| a=1;b   =   2 ;c=3 |) in x\n").as_deref(),
            Some("let x = (| a = 1; b = 2; c = 3 |)\nin\nx\n")
        );
    }

    /// A record with no spaces at all. **This is the first rule in the whole
    /// formatter that writes a byte where the author wrote none**, and it is
    /// the direction that cannot fuse: whitespace between two copied ranges
    /// only ever separates them. (The other direction, `;`, is the one that
    /// consults [`sep::must_separate`].)
    #[test]
    fn a_record_literal_with_no_spaces_gains_them_around_eq_only() {
        assert_eq!(
            fmt("let x = (| a=1;b=2 |) in x\n").as_deref(),
            Some("let x = (| a = 1; b = 2 |)\nin\nx\n")
        );
        // And the OTHER side of the `;` gains one, which is the inversion
        // rather than a second rule: `separator` says *tight before*, and
        // what follows a `;` is an ordinary gap that nothing excepts. Under
        // the shape list it stayed `;b`, because no shape named it.
        assert!(fmt("let x = (| a=1;b=2 |) in x\n").expect("formats").contains("; b"));
    }

    /// A record whose `=` signs the author lined up into a column: the column
    /// **survives**, because nothing else on any of those lines needed
    /// relaying out. [`Spacing::preserve_alignment`].
    ///
    /// 1,363 of the 3,105 corpus lines the rules touch are this shape — 44% —
    /// so this is the single largest behavioural decision in the slice.
    #[test]
    fn a_hand_aligned_column_survives_on_a_line_nothing_else_moved() {
        // **The surviving domain, with a REAL column in it.** `stdja.satyh:36-42`
        // is the shape: a `sig` item whose type has no arrow, no group and no
        // break point anywhere on its line, so the builder's column model is
        // exact and the run is kept.
        let sig = "\
module M : sig
  val font-cjk-gothic   : string * float * float
  val f                 : int
  val g                 : int
end = struct
  let font-cjk-gothic = 1
  let f = 2
  let g = 3
end
in
()
";
        assert_eq!(fmt(sig).as_deref(), Some(sig), "a `sig` column died");
        assert_eq!(fmt(sig).as_deref(), Some(sig), "not a fixpoint");
    }

    /// The defect the column predicate exists for: **a run with nothing to
    /// align WITH is not a column, it is extra spaces**.
    ///
    /// The coarse rule could not tell the two apart — it preserved any run on
    /// a line it had not otherwise moved — so the user's `1  +  2` came back
    /// unchanged however many times they saved.
    #[test]
    fn a_run_with_no_neighbour_to_line_up_with_is_collapsed() {
        for (src, want) in [
            ("let x = 1  +  2 in x\n", "let x = 1 + 2\nin\nx\n"),
            ("let x = 1   +   2 in x\n", "let x = 1 + 2\nin\nx\n"),
            // Not only binops: a lone record field, a lone ascription.
            ("let r = (| a   = 1 |) in r\n", "let r = (| a = 1 |)\nin\nr\n"),
            ("let f : int   -> int = fun x -> x in f\n", "let f : int -> int = fun x -> x\nin\nf\n"),
        ] {
            assert_eq!(fmt(src).as_deref(), Some(want), "{src:?}");
            assert_eq!(fmt(want).as_deref(), Some(want), "{src:?}: not a fixpoint");
        }
    }

    /// Witness **A**, the symmetric one: two adjacent lines each padded to the
    /// same column. Both keep their runs, and they keep each other's.
    ///
    /// The near-miss control is the point of the test: one column apart is not
    /// a column, and the rule has to be able to say so.
    #[test]
    fn a_record_literals_column_is_collapsed_because_the_line_may_re_wrap() {
        // **Slice 3 took this one.** A record literal's fields sit inside an
        // `Auto` group, so every gap on those lines is a break point the
        // renderer decides after the builder has finished — the column model
        // ([`Build::advance`]) cannot say where the run would land, and a run
        // preserved at a column nobody can compute is worse than none. See
        // [`Build::mark_inexact`] for why the answer is a marking rather than
        // a switch, and `a_hand_aligned_column_survives_on_a_line_nothing_else_moved`
        // for the domain that is left.
        let aligned = "let r = (| aa = 1; bbb = 2; |)\nin\n()\n";
        assert_eq!(fmt(aligned).as_deref(), Some(aligned));
        let padded = "let r = (|\n  aa   = 1;\n  bbbb = 2;\n|)\nin ()\n";
        assert_eq!(
            fmt(padded).as_deref(),
            Some("let r = (| aa = 1; bbbb = 2; |)\nin\n()\n"),
            "a record field column must NOT survive a line the renderer may re-wrap"
        );
        let near_miss = "\
let r = (|
  aa   = 1;
  bbb   = 2;
|)
in ()
";
        let collapsed = "let r = (| aa = 1; bbb = 2; |)\nin\n()\n";
        assert_eq!(fmt(near_miss).as_deref(), Some(collapsed));
        assert_eq!(fmt(collapsed).as_deref(), Some(collapsed));
    }

    /// Witness **B**: the WIDEST row of a hand-built column is the one that
    /// needed no padding, so it has no run of its own — and it is exactly what
    /// holds the column up for the rows that do.
    ///
    /// A rule that demanded a run on both sides would collapse the whole of
    /// this table, which is the first thing the corpus disagrees with.
    #[test]
    fn a_runless_widest_row_holds_the_column_up_for_its_neighbours() {
        let src = "module M : sig\n  val a   : int\n  val bbb : int\nend = struct\n  let a = 1\n  let bbb = 2\nend\nin\n()\n";
        assert_eq!(fmt(src).as_deref(), Some(src));
    }

    /// Witness **C**, the closure: `enumitem.satyh:217-220`, reduced.
    ///
    /// The four arms line up at their `->` because `1`, `5` and `_` are
    /// right-padded against `10` — and only the arm NEXT to `10` has a runless
    /// witness. The other two reach it one row at a time, which is why the
    /// rule has a closure step rather than a single adjacency test.
    #[test]
    fn a_match_arms_column_is_collapsed_because_its_body_is_a_group() {
        // A `match` arm's body is a `Mode::Auto` group with a break point in
        // it ([`Build::body`]), so the arm's line is one the renderer may
        // re-wrap and its column goes. That is the largest single thing slice
        // 3 costs against slice 2, and it is not recoverable: the column's
        // position is a function of a fit decision the builder has not taken.
        let src = "let f n =\n  match n with\n  | 1 -> 0x0049\n  | 5 -> 0x0056\n  | 10 -> 0x0058\n  | _ -> 0x003F\nin\nf 1\n";
        assert_eq!(fmt(src).as_deref(), Some(src));
        let padded = "let f n =\n  match n with\n  |  1 -> 0x0049\n  | 10 -> 0x0058\nin f 1\n";
        assert_eq!(
            fmt(padded).as_deref(),
            Some("let f n =\n  match n with\n  | 1 -> 0x0049\n  | 10 -> 0x0058\nin\nf 1\n"),
            "a `match` arm column must NOT survive"
        );
        // The control: take the runless row away and the same four lines are
        // just padding, so all of them collapse.
        let no_anchor = "\
let f n =
  match n with
  |  1 -> 0x0049
  |  5 -> 0x0056
  |  _ -> 0x003F
in f 1
";
        let flat = "let f n =\n  match n with\n  | 1 -> 0x0049\n  | 5 -> 0x0056\n  | _ -> 0x003F\nin\nf 1\n";
        assert_eq!(fmt(no_anchor).as_deref(), Some(flat));
        assert_eq!(fmt(flat).as_deref(), Some(flat));
    }

    /// A table whose padding **alternates between columns**, which is what a
    /// hand-aligned matrix is — and which clause B could not ground until it
    /// learned to ask about a witness's runs *up to a column* rather than
    /// about the whole line.
    ///
    /// `azmath/doc/azmath.saty:835` is the real one, and it is real math
    /// inside `${ … }`, so nothing put a spacing request on it until math
    /// came inside the default. Row 1 pads the first column, row 2 pads the
    /// second; neither is runless and neither has a run at the other's mark,
    /// so clauses A and B both declined and the matrix flattened.
    ///
    /// What grounds it is that row 2's first-column `\frac` sits at its
    /// column with **no padding before it at all** — every byte to its left
    /// is canonical — so it is exactly as good a witness there as a wholly
    /// runless line. Row 1 is preserved by that, and row 2 then by the
    /// closure.
    #[test]
    fn a_math_matrix_keeps_its_rows_and_its_hand_built_columns() {
        // **Both survive, and the second one is a consequence of the first.**
        // Math keeps the author's line structure ([`Build::math_text`]), so a
        // matrix written one row per line stays that way — joining it is the
        // one edit no reader can undo. And because math offers no break point
        // at all, those lines are ones the BUILDER lays out completely, which
        // is exactly [`Build::mark_inexact`]'s condition for
        // [`Spacing::preserve_alignment`] to keep a column. Making math atomic
        // handed hand-aligned math back its columns as a side effect.
        let src = "let m = ${\n  |      \\frac{d}{ad - bc} | \\neg \\frac{b}{ad - bc}\n  | \\neg \\frac{c}{ad - bc} |      \\frac{a}{ad - bc}\n|}\nin\nm\n";
        let want = "let m = ${\n  |      \\frac{d}{ad - bc} | \\neg \\frac{b}{ad - bc}\n  | \\neg \\frac{c}{ad - bc} |      \\frac{a}{ad - bc}\n  |}\nin\nm\n";
        assert_eq!(fmt(src).as_deref(), Some(want), "the rows or the columns moved");
        assert_eq!(fmt(want).as_deref(), Some(want), "not a fixpoint");
        // The control, so this is not "keep every run": break the alignment by
        // one column and nothing lines up, so the padding collapses while the
        // rows still stay.
        let skew = "let m = ${\n  |      \\frac{d}{ad - bc} | \\neg \\frac{b}{ad - bc}\n  | \\negg \\frac{c}{ad - bc} |      \\frac{a}{ad - bc}\n|}\nin\nm\n";
        let flat = "let m = ${\n  | \\frac{d}{ad - bc} | \\neg \\frac{b}{ad - bc}\n  | \\negg \\frac{c}{ad - bc} | \\frac{a}{ad - bc}\n  |}\nin\nm\n";
        assert_eq!(fmt(skew).as_deref(), Some(flat), "a skewed table was preserved");
        assert_eq!(fmt(flat).as_deref(), Some(flat), "not a fixpoint");
        // And a matrix the author wrote on ONE line stays on one line, however
        // far past the budget it runs — the other half of atomic.
        let one = "let m = ${| \\frac{d}{ad - bc} | \\neg \\frac{b}{ad - bc} | \\neg \\frac{c}{ad - bc} | \\frac{a}{ad - bc} |}\nin\nm\n";
        assert_eq!(fmt(one).as_deref(), Some(one), "a one-line matrix was broken up");
    }

    /// A column whose entries are all DIFFERENT tokens, which the text
    /// witness alone cannot see.
    ///
    /// The shape is the corpus's, from `stdja.satyh:123-125` and seven other
    /// files. Widening [`Spacing::app_arg`] to every argument boundary is
    /// what first put a spacing request on these gaps, and with `text`
    /// equality as the only witness the first two rows preserved each other
    /// by luck — they name the same font — while the third had nothing to
    /// match and was flattened. Half a column is worse than none.
    #[test]
    fn an_operator_chains_value_column_is_collapsed_for_the_same_reason() {
        let src = "let x =\n  ctx\n    |> set-font Kana font-cjk-mincho\n    |> set-font HanIdeographic font-cjk-mincho\n    |> set-font Latin font-latin-roman\nin\nx\n";
        assert_eq!(fmt(src).as_deref(), Some(src), "a value column was flattened");
    }

    /// And the discrimination that makes it a rule rather than a licence to
    /// keep every double space.
    #[test]
    fn a_run_that_does_not_actually_line_up_still_collapses() {
        // The corpus's other `set-font` shape, and it is NOT a column: the
        // first row carries a `ctx ` and sits two columns to the left, so its
        // `font` lands at 35 and the next row's at 33. The author padded
        // `Kana` to `HanIdeographic`'s width without noticing that the two
        // lines start in different places. Rows that DO line up are kept.
        let pseudo = "\
let x =
  let g font ctx =
    ctx |> set-font HanIdeographic font
      |> set-font Kana           font
  in g
in x
";
        let want = "let x = let g font ctx = ctx |> set-font HanIdeographic font |> set-font Kana font in g\nin\nx\n";
        assert_eq!(fmt(pseudo).as_deref(), Some(want));
        assert_eq!(fmt(want).as_deref(), Some(want), "not a fixpoint");
        // A stray double space whose line disagrees with its neighbours about
        // an EARLIER gap. `satysfi-base/src/typeset/derive/ast.satyh:11` is
        // the real one: `t ->  math` puts `math` exactly where the next
        // line's `t list` begins, so a witness that asked only "same gap
        // index, same column" would preserve a typo.
        let typo = "\
module M : sig
  val get-sub-label : t -> inline-text option
  val get-conclusion : t ->  math
  val get-assumptions : t -> t list
end = struct
  let get-sub-label = 1
  let get-conclusion = 2
  let get-assumptions = 3
end
";
        let fixed = typo.replace("t ->  math", "t -> math");
        assert_eq!(fmt(typo).as_deref(), Some(fixed.as_str()));
    }

    /// The idempotence hazard a cross-line predicate has and the coarse rule
    /// did not, pinned rather than argued: **collapsing a run on one line
    /// moves every mark to its right, which is what a neighbour might have
    /// been matched against.**
    ///
    /// Here `aaa  =` collapses to `aaa =`, which puts its `=` at the very
    /// column `bb  =`'s run is padding to. A rule that re-read the columns of
    /// its own output would preserve `bb  =` on the second pass and produce a
    /// different file on every save. This one cannot: a collapsed line carries
    /// no run afterwards, so it is not a candidate the second time round, and
    /// a line that still has runs was preserved and did not move. See
    /// [`preserved_lines`].
    #[test]
    fn a_collapse_cannot_hand_its_neighbour_a_column_on_the_second_pass() {
        let src = "\
let r = (|
  aaa  = 1;
  bb  = 2;
|)
in ()
";
        let once = fmt(src).expect("formats");
        assert_eq!(
            once,
            "let r = (| aaa = 1; bb = 2; |)\nin\n()\n"
        );
        // `aaa =` now sits at exactly the column `bb  =` was padding to, and
        // nothing comes back.
        let twice = fmt(&once).expect("formats its own output");
        assert_eq!(twice, once, "the second pass resurrected a column");
        let thrice = fmt(&twice).expect("formats its own output");
        assert_eq!(thrice, twice);
    }

    /// A relaid line is not a witness, because its own bytes are about to
    /// move. `enumitem.satyh:372-375` is the corpus case: `+enumerate:` is
    /// written tight, so the `colon` rule pushes its `:` one column right and
    /// the column `+listing  :` was padding to stops existing.
    #[test]
    fn a_line_the_rules_are_rewriting_cannot_hold_a_column_up() {
        let src = "\
module M : sig
  direct +listing  : [itemize] block-cmd
  direct +enumerate: [itemize] block-cmd
end = struct
  let-block ctx +listing it = read-block ctx '<>
  let-block ctx +enumerate it = read-block ctx '<>
end
in ()
";
        let want = "module M : sig\n  direct +listing : [itemize] block-cmd\n  direct +enumerate : [itemize] block-cmd\nend = struct\n  let-block ctx +listing it = read-block ctx '<>\n  let-block ctx +enumerate it = read-block ctx '<>\nend\nin\n()\n";
        assert_eq!(fmt(src).as_deref(), Some(want));
        assert_eq!(fmt(want).as_deref(), Some(want));

        // The sharp version of the same exclusion, and the reason it is not
        // merely tidy. Here the relaid line carries a column of its OWN, so
        // admitting it as a witness would preserve `a   =` against a `bb  =`
        // that the very next thing the emitter does is collapse — and the
        // second pass, seeing `bb = 11;`, would find nothing to line up with
        // and take `a   =` down too. Two saves, two different files.
        let sharp = "\
let r =
  (|
    a   = 22;
    bb  = 11 ;
  |)
in ()
";
        let flat = "let r = (| a = 22; bb = 11; |)\nin\n()\n";
        assert_eq!(fmt(sharp).as_deref(), Some(flat));
        let once = fmt(sharp).expect("formats");
        assert_eq!(fmt(&once).as_deref(), Some(once.as_str()), "not a fixpoint");
    }

    /// Adjacency is literal, and this is what that costs:
    /// `figbox/doc/manual.saty:533-537` is a real three-row column with a
    /// `separator;` line between each pair, and the one row that needed
    /// padding loses it. Pinned so the cost is a decision rather than a
    /// surprise — widening the window would have to name a distance, and any
    /// distance is a guess.
    #[test]
    fn a_column_whose_rows_are_not_adjacent_is_not_seen() {
        let src = "\
let xs = [
  z;
  f ?:aaa    [1];
  z;
  f ?:bbbbbb [2];
]
in xs
";
        let want = "let xs = [z; f ?:aaa [1]; z; f ?:bbbbbb [2];]\nin\nxs\n";
        assert_eq!(fmt(src).as_deref(), Some(want));
        assert_eq!(fmt(want).as_deref(), Some(want));
    }

    /// And the other half of the same rule: a column on a line the formatter
    /// **did** relay out is collapsed with everything else.
    ///
    /// This is the user's own example, and it is why the rule is per-LINE
    /// rather than per-gap. `a=1` and `c=3` are not columns and they move, so
    /// the line was not a table the author was maintaining — and `b   =   2`
    /// goes with them. A per-gap version of `preserve_alignment` would leave
    /// `b   =   2` standing in the middle of a line it had just canonicalised
    /// either side of, which is not what anybody meant by "preserve
    /// alignment".
    #[test]
    fn a_column_on_a_line_the_rules_already_moved_goes_with_it() {
        assert_eq!(
            fmt("let x = (| a=1;b   =   2 ;c=3 |) in x\n").as_deref(),
            Some("let x = (| a = 1; b = 2; c = 3 |)\nin\nx\n")
        );
    }

    /// A record TYPE: the same `(| … |)` a record literal is, so `colon`,
    /// `separator` and `bracket` all reach it.
    ///
    /// It also pins the sharp edge of the per-line rule. The author wrote both
    /// a `:` column and a stray space before each `;`. Removing the space is a
    /// relayout of the line, so the column on that line goes too — the two
    /// halves are not separable, because "has this line been relaid out" is
    /// one question per line and not one per rule. A file whose columns matter
    /// more than its `;`s should reach for `% rustyfi-fmt: off`
    /// (`engine.md` section 10), which is the escape hatch designed for it.
    #[test]
    fn a_record_types_column_goes_when_its_semi_is_tightened_on_the_same_line() {
        let before = "\
module M : sig
  val f : (|
    text-width    : length ;
    text-height   : length ;
  |) -> int
end = struct
  let f r = 1
end
in ()
";
        let after = "module M : sig\n  val f : (| text-width : length; text-height : length; |) -> int\nend = struct\n  let f r = 1\nend\nin\n()\n";
        assert_eq!(fmt(before).as_deref(), Some(after));
        // The control: with the `;`s already tight, the column stays.
        assert_eq!(fmt(after).as_deref(), Some(after));
    }

    /// A rule may not change line structure. `Build::gap_upto` drops a request
    /// whose gap holds a line break, so a field written across two lines keeps
    /// both — no rule in slice 2 can join or split a line, which is what keeps
    /// a regression here confined to one.
    #[test]
    fn a_record_field_broken_across_lines_keeps_its_break() {
        let src = "let x = (| a = 1; b = 2; |)\nin\n()\n";
        assert_eq!(fmt(src).as_deref(), Some(src));
    }

    /// A `%` comment inside the gap a rule wants: copied, not replaced.
    /// Rewriting that gap would DELETE the comment, so `Build::gap_upto`
    /// requires the gap to be horizontal whitespace and nothing else. Only
    /// reachable break-free at end of file, which is why the fixture ends
    /// there.
    #[test]
    fn a_rule_never_rewrites_a_gap_holding_a_comment() {
        let src = "let x = (| a =% not a space\n1 |) in x\n";
        let out = fmt(src).expect("formats");
        assert!(out.contains("% not a space"), "the comment was lost: {out:?}");
    }

    /// The rules reach every `=`, `->`, `:`, `|`, operator, bracket and
    /// separator the grammar has a leaf for — one fixture per rule, each
    /// written non-canonically so the rule has to fire.
    #[test]
    fn each_rule_fires_on_its_own_construct() {
        for (src, want, rule) in [
            ("let x=1 in x\n", "let x = 1\nin\nx\n", "eq: a plain `let`"),
            (
                "let-rec f\n  | 0=1\n  | n=n\nin f 3\n",
                "let-rec f\n  | 0 = 1\n  | n = n\nin\nf 3\n",
                "eq: a `let-rec` clause",
            ),
            (
                "let f = fun ?(x=a) y -> y in f 2\n",
                // TIGHT before the `?`: [`Spacing::sigil`], and the corpus is
                // unambiguous — 215 tight `?(` against 0 spaced, and 249
                // postfix `float?` markers against 0.
                "let f = fun?(x = a) y -> y\nin\nf 2\n",
                "sigil: an optional binder",
            ),
            ("type t=int\nin ()\n", "type t = int\nin\n()\n", "eq: a `type` decl"),
            // Note the space BEFORE each arrow: `-` is an identifier
            // character in this grammar, so `y->true` lexes as the identifier
            // `y-` followed by `>`, not as `y` `->` `true`. That is exactly
            // the fusion `sep::must_separate` answers for (`is_word('-')`),
            // and it is why a rule may only ever ADD a space here.
            (
                "let g : int ->bool = fun y ->true in g\n",
                "let g : int -> bool = fun y -> true\nin\ng\n",
                "arrow: a type arrow and a `fun` arrow",
            ),
            (
                "let m x = match x with |A ->1 |B ->2 in m\n",
                "let m x =\n  match x with\n  | A -> 1\n  | B -> 2\nin\nm\n",
                "arrow and bar: `match` arms",
            ),
            (
                "let-mutable r <-0 in let () = r <-1 in !r\n",
                "let-mutable r <- 0\nin\nlet () = r <- 1 in\n!r\n",
                "arrow: both `<-` forms",
            ),
            ("let g:int = 1 in g\n", "let g : int = 1\nin\ng\n", "colon: an ascription"),
            // `=|` is one operator token, so the `=` keeps its space here for
            // the same reason the arrows above do.
            (
                "type t = |A |B of int\nin ()\n",
                "type t =\n  | A\n  | B of int\nin\n()\n",
                "bar: variant alternatives",
            ),
            ("let z = 1+2*3 in z\n", "let z = 1 + 2 * 3\nin\nz\n", "binop: an OpChain"),
            (
                "let xs = [ 1 ; 2 ] in ( xs , xs )\n",
                "let xs = [1; 2]\nin\n(xs, xs)\n",
                "bracket and separator",
            ),
        ] {
            assert_eq!(fmt(src).as_deref(), Some(want), "{rule}");
            assert_eq!(fmt(want).as_deref(), Some(want), "{rule}: not a fixpoint");
        }
    }

    /// **Rule 1**: one space between an applied head and an argument that
    /// begins with a group opener, for every opener the grammar has.
    ///
    /// The two shapes the request names come first. Note that the rule has to
    /// be *canonicalising* rather than gap-filling: the corpus writes
    /// `document (| … |)` with the space 93 times out of 93, so a rule that only
    /// filled the missing case would have almost nothing to act on.
    #[test]
    fn an_argument_that_begins_with_a_group_is_separated_from_its_head() {
        for (src, want, what) in [
            ("let f = g(| x = 1 |) in f\n", "let f = g (| x = 1 |)\nin\nf\n", "a record"),
            (
                "let d = document(| title = t |) '<+p{a}>\nin ()\n",
                "let d = document (| title = t |) '< +p{a} >\nin\n()\n",
                "the document call",
            ),
            ("let y = f(x) in y\n", "let y = f (x)\nin\ny\n", "a parenthesised argument"),
            ("let y = f[1; 2] in y\n", "let y = f [1; 2]\nin\ny\n", "a list"),
            ("let y = f() in y\n", "let y = f ()\nin\ny\n", "unit"),
            ("let y = f{a} in y\n", "let y = f {a}\nin\ny\n", "inline text"),
            ("let y = f'<+p{a}> in y\n", "let y = f '< +p{a} >\nin\ny\n", "block text"),
            ("let y = f${x} in y\n", "let y = f ${x}\nin\ny\n", "math"),
            // Every argument in a run, not only the first, and after a bare
            // one too.
            (
                "let y = f(a)(b) x(c) in y\n",
                "let y = f (a) (b) x (c)\nin\ny\n",
                "a run of arguments",
            ),
            // The head may be a group itself.
            ("let y = (g x)(a) in y\n", "let y = (g x) (a)\nin\ny\n", "a parenthesised head"),
            // `#label` binds to its head; what follows is still an argument.
            ("let y = r#f(a) in y\n", "let y = r#f (a)\nin\ny\n", "after a field access"),
        ] {
            assert_eq!(fmt(src).as_deref(), Some(want), "{what}");
            assert_eq!(fmt(want).as_deref(), Some(want), "{what}: not a fixpoint");
        }
    }

    /// **Rule 1, the other side of the same boundary**: an argument that
    /// ENDS in a group closer is separated from whatever argument follows it.
    ///
    /// `myFn(123)0` was the defect: the leading half spaced `(123)` off its
    /// head and left `)0` fused, canonicalising one side of a group and not
    /// the other. In an application chain every argument boundary is the same
    /// boundary, and the census agrees more sharply here than for the leading
    /// half — 1439 spaced against 1 tight.
    #[test]
    fn an_argument_that_ends_a_group_is_separated_from_the_next_one() {
        for (src, want, what) in [
            // The user's own three.
            ("let y = myFn(123)0 in y\n", "let y = myFn (123) 0\nin\ny\n", "a bare argument after `)`"),
            ("let y = myFn(123)(4) in y\n", "let y = myFn (123) (4)\nin\ny\n", "a group after `)`"),
            ("let y = myFn(123)foo in y\n", "let y = myFn (123) foo\nin\ny\n", "a variable after `)`"),
            ("let y = myFn[1]2 in y\n", "let y = myFn [1] 2\nin\ny\n", "a bare argument after `]`"),
            // Every closer the grammar has, each followed by a BARE argument,
            // which is the case the leading half cannot reach.
            ("let y = f(| a = 1 |)x in y\n", "let y = f (| a = 1 |) x\nin\ny\n", "after ` |)`"),
            ("let y = f{a}x in y\n", "let y = f {a} x\nin\ny\n", "after an inline `}`"),
            ("let y = f${a}x in y\n", "let y = f ${a} x\nin\ny\n", "after a math `}`"),
            ("let y = f'<+p{a}>x in y\n", "let y = f '< +p{a} > x\nin\ny\n", "after `>`"),
            ("let y = f()x in y\n", "let y = f () x\nin\ny\n", "after unit"),
            ("let y = f Mod.(a)x in y\n", "let y = f Mod.(a) x\nin\ny\n", "after `Mod.( … )`"),
            // The head is the left side of the FIRST argument's boundary, and
            // it is the same question.
            ("let y = (g x)0 in y\n", "let y = (g x) 0\nin\ny\n", "a group HEAD"),
            // Three in a row, so the rule is a boundary rule rather than a
            // one-shot.
            ("let y = f(a)b(c)d in y\n", "let y = f (a) b (c) d\nin\ny\n", "a whole run"),
        ] {
            assert_eq!(fmt(src).as_deref(), Some(want), "{what}");
            assert_eq!(fmt(want).as_deref(), Some(want), "{what}: not a fixpoint");
        }
    }

    /// What the trailing half must NOT reach, for the same reasons the leading
    /// half does not.
    ///
    /// A `#label` access moves the argument's last token OFF the closer — the
    /// one asymmetry between [`arg_opens_a_group`] and [`arg_closes_a_group`]
    /// — and a command's arguments stay tight on this boundary too, measured:
    /// 499 tight against 52 spaced.
    #[test]
    fn the_trailing_half_spares_a_label_a_command_and_a_constructor() {
        for (src, why) in [
            ("let y = f (| a = 1 |)#a0\nin\ny\n", "a `#label` after the group"),
            ("let y = f r#a0\nin\ny\n", "a `#label` on a bare argument"),
            ("let-inline ctx \\c a b = read-inline ctx a\nin\n{\\c{x}{y}}\n", "an inline command"),
            ("let-block ctx +c a b = read-block ctx a\nin\n'< +c{x}{y} >\n", "a block command"),
            ("let y = Some(1)\nin\ny\n", "a constructor head"),
        ] {
            assert_eq!(fmt(src).as_deref(), Some(src), "{why} must be untouched");
        }
    }

    /// The two heads the census excludes, and the argument shapes the rule
    /// does not name. Each must be a **no-op**, which is a stronger claim than
    /// "the rule is off": all of these reach [`Build::app_args`].
    #[test]
    fn the_group_rule_spares_a_constructor_a_command_and_a_sigil() {
        for (src, why) in [
            // 874 tight against 144 spaced: `Some(x)` is a data constructor
            // with an argument, not a function applied to a group.
            ("let y = Some(1)\nin\ny\n", "a constructor head"),
            ("let y = Ctor(| a = 1 |)\nin\ny\n", "a constructor applied to a record"),
            // 1912 tight against 173 spaced. A command's arguments are a
            // `CmdTail`, which this rule does not reach at all.
            ("let-inline ctx \\c a = read-inline ctx a\nin\n{\\c{x}}\n", "an inline command"),
            ("let-block ctx +c a = read-block ctx a\nin\n'< +c{x} >\n", "a block command"),
            // A sigil binds to its operand (`engine.md` section 9), so a space
            // in front of the group would land inside the sigil's grip.
            ("let y = f &(1)\nin\n~y\n", "a staged argument"),
            ("let y = f ?:(1) 2\nin\ny\n", "a supplied optional"),
        ] {
            assert_eq!(fmt(src).as_deref(), Some(src), "{why} must be untouched");
        }
    }

    /// **The widening**: every argument boundary is exactly one space,
    /// whatever the argument's shape.
    ///
    /// The two tests above only ever reach a boundary with a GROUP on one
    /// side of it, which is how the rule shipped twice while
    /// `` List.intersperse  `, ` `` and `f  xs` came back unchanged — a
    /// literal is not a group and a variable is not a group, so no rule
    /// claimed the gap and slice 1 copied it. Measured before the fix:
    ///
    /// ```text
    ///   List.intersperse  (1)   ->  List.intersperse (1)     normalised
    ///   List.intersperse  `, `  ->  List.intersperse  `, `   UNCHANGED
    ///   List.intersperse  xs    ->  List.intersperse  xs     UNCHANGED
    ///   f  `x` y                ->  f  `x` y                 UNCHANGED
    /// ```
    ///
    /// The census that scopes the widening is on [`Spacing::app_arg`]: over
    /// every argument boundary in the 162-file corpus a variable head is 36
    /// tight, 17,610 at one space and 89 at more, and the two new rows are
    /// the sharpest of the lot (a variable argument 7818/29, a literal
    /// argument 999/4).
    #[test]
    fn every_argument_boundary_is_one_space_whatever_the_arguments_shape() {
        for (src, want, what) in [
            // The user's own case, and its controls.
            (
                "let y = List.intersperse  `, ` xs in y\n",
                "let y = List.intersperse `, ` xs\nin\ny\n",
                "a backtick literal argument",
            ),
            (
                "let y = f  `x` y in y\n",
                "let y = f `x` y\nin\ny\n",
                "a literal in the middle of a run",
            ),
            (
                "let y = List.intersperse  xs in y\n",
                "let y = List.intersperse xs\nin\ny\n",
                "a bare variable argument",
            ),
            (
                "let y = List.intersperse  (1) in y\n",
                "let y = List.intersperse (1)\nin\ny\n",
                "the group case, which already worked",
            ),
            // A literal abutting its head with NO space is 4 boundaries in
            // the corpus (`let-math \succ = rel`≻``) against 999 spaced.
            ("let y = f`x` in y\n", "let y = f `x`\nin\ny\n", "a literal written tight"),
            // Every other bare atomic shape the census counted.
            ("let y = f  1  2 in y\n", "let y = f 1 2\nin\ny\n", "integers"),
            ("let y = f  1.5  2pt in y\n", "let y = f 1.5 2pt\nin\ny\n", "a float and a length"),
            ("let y = f  true  false in y\n", "let y = f true false\nin\ny\n", "booleans"),
            ("let y = f  Some  x in y\n", "let y = f Some x\nin\ny\n", "a constructor ARGUMENT"),
            // An optional and an omission: 56/1 spaced, 0 tight. The sigil's
            // grip on its own operand is never touched — only the boundary in
            // front of it.
            (
                "let y = f  ?:(1)  2 in y\n",
                "let y = f ?:(1) 2\nin\ny\n",
                "an optional argument",
            ),
            ("let y = f  ?*  2 in y\n", "let y = f ?* 2\nin\ny\n", "an omitted optional"),
            // A stage sigil, likewise: the space lands in front of `&`, never
            // between `&` and its operand.
            ("let y = f  &(1) in ~y\n", "let y = f &(1)\nin\n~y\n", "a staged argument"),
            // A three-deep mixed run, so the rule is a boundary rule rather
            // than a first-argument rule.
            (
                "let y = f  `a`  b  (c)  1 in y\n",
                "let y = f `a` b (c) 1\nin\ny\n",
                "a mixed run",
            ),
        ] {
            assert_eq!(fmt(src).as_deref(), Some(want), "{what}");
            assert_eq!(fmt(want).as_deref(), Some(want), "{what}: not a fixpoint");
        }
    }

    /// The widening still stops at the head census: a **constructor** head and
    /// a **command**'s arguments are never SPACED and never TIGHTENED.
    ///
    /// Worth its own fixture rather than leaving it to the group tests,
    /// because widening the argument side is exactly the change that could
    /// leak past the head scoping — and a constructor's bare-argument row is
    /// 18 boundaries at one space, so a leak there would be invisible without
    /// a fixture that writes the gap both ways.
    ///
    /// Both ways is what this asserts now, and it is the narrowing that comes
    /// with [`Space::Collapse`]. The fixture used to be `{\c  {x}  {y}}`,
    /// held unchanged — which reads as "the head is spared" but actually
    /// pinned a second claim nothing measured: that a multi-space RUN at a
    /// command's argument boundary survives. It does not, and
    /// `a_run_at_a_command_argument_boundary_collapses` is where that now
    /// lives. What the census supports is the pair below: an empty gap stays
    /// empty and one space stays one space, so the formatter has no opinion
    /// between the corpus's 91.7% and its 8.3%.
    #[test]
    fn the_widened_rule_still_spares_a_constructor_head_and_a_command() {
        for (src, why) in [
            ("let y = Some(1)\nin\ny\n", "a constructor head with a group, tight"),
            ("let y = Some (1)\nin\ny\n", "a constructor head with a group, spaced"),
            (
                "let-inline ctx \\c a b = read-inline ctx a\nin\n{\\c{x}{y}}\n",
                "an inline command's arguments, tight",
            ),
            (
                "let-inline ctx \\c a b = read-inline ctx a\nin\n{\\c {x} {y}}\n",
                "an inline command's arguments, spaced",
            ),
            (
                "let-block ctx +c a b = read-block ctx a\nin\n'< +c{x}{y} >\n",
                "a block command's arguments, tight",
            ),
            (
                "let-block ctx +c a b = read-block ctx a\nin\n'< +c {x} {y} >\n",
                "a block command's arguments, spaced",
            ),
        ] {
            assert_eq!(fmt(src).as_deref(), Some(src), "{why} must be untouched");
        }
    }

    /// A multi-space run between two arguments that IS a column survives, and
    /// one that is not collapses.
    ///
    /// New ground for [`Spacing::preserve_alignment`]: the widened rule puts a
    /// request on gaps it previously never saw, and the corpus's 29
    /// multi-space variable-argument boundaries are almost all columns —
    /// `|> set-font Kana  …` / `|> set-font Latin …` runs and aligned
    /// `-> dot-arabic  ctx` clauses. If the widening flattened those it would
    /// be a 29-boundary regression dressed as a fix.
    #[test]
    fn an_argument_column_is_collapsed_because_the_run_may_fill() {
        // An argument run FILLS ([`Build::app_args`]), so every boundary in
        // it is a `Doc::FillLine` and the line is one the renderer may
        // re-wrap. The column goes with it.
        let column = "let x = let a = f Kana g in let b = f Latin g in a\nin\nx\n";
        assert_eq!(fmt(column).as_deref(), Some(column));
        // The same shape with nothing to line up with: collapsed.
        let lone = "let x =\n  let a = f Kana   g in\n  a\nin x\n";
        assert_eq!(
            fmt(lone).as_deref(),
            Some("let x = let a = f Kana g in a\nin\nx\n"),
            "a lone multi-space run is not a column and must collapse"
        );
    }

    /// A request over a gap holding a line break is dropped, so this rule can
    /// never pull an argument up onto its head's line — the guard every
    /// slice-2 rule shares, restated here because this is the rule most
    /// obviously tempted to.
    #[test]
    fn the_group_rule_never_joins_an_argument_onto_its_heads_line() {
        let src = "let y = f (a)\nin\ny\n";
        assert_eq!(fmt(src).as_deref(), Some(src));
    }

    /// **A multi-space run at a command's argument boundary collapses** —
    /// the defect [`Spacing::cmd_arg`]'s `Keep` -> `Collapse` fixes, in all
    /// three of the shapes it was reported in.
    ///
    /// The census that scopes the exception is a two-column table, tight
    /// against spaced, and `Keep` answered a third question the table never
    /// asked. There are **zero** multi-space runs at this boundary in either
    /// corpus, so nothing measured could have caught it and only a fixture
    /// can: revert `cmd_arg_boundary` to `Space::Keep` and every one of these
    /// four comes back unchanged.
    #[test]
    /// A record or block group with nothing inside is NOT padded.
    ///
    /// `'<>` and `(||)` have no content to put a space beside, and `'< >` would
    /// be inventing content-shaped whitespace in a group the author wrote as
    /// empty. Found by a corpus-shaped fixture (`read-block ctx '<>`) rather
    /// than by this test — which is why the test exists now.
    #[test]
    fn an_empty_group_is_not_padded() {
        for src in [
            "let x = (||)\nin\nx\n",
            "let-block ctx +c a = read-block ctx '<>\nin\n()\n",
        ] {
            assert_eq!(
                fmt(src).as_deref(),
                Some(src),
                "an empty group must be left alone: {src:?}"
            );
        }
        // The control: the same delimiters WITH content are padded, so the
        // assertion above is about emptiness and not about the rule being off.
        assert_eq!(
            fmt("let x = (|a = 1|) in x\n").as_deref(),
            Some("let x = (| a = 1 |)\nin\nx\n")
        );
    }

    fn a_run_at_a_command_argument_boundary_collapses() {
        for (src, want, why) in [
            (
                "let-math \\frac a b = a\nin ${\\frac{a}   {6}}\n",
                "let-math \\frac a b = a\nin ${\\frac{a} {6}}\n",
                "a math command, the reported case",
            ),
            (
                "let-inline ctx \\c a = read-inline ctx a\nin {\\c   {x}}\n",
                "let-inline ctx \\c a = read-inline ctx a\nin {\\c {x}}\n",
                "an inline command",
            ),
            (
                "let-block ctx +c a = read-block ctx a\nin '<+c   {x}>\n",
                "let-block ctx +c a = read-block ctx a\nin '< +c {x} >\n",
                "a block command",
            ),
            (
                // The user's own line: a `+math` whose math argument is
                // followed by a run before its second group.
                "let-math \\frac a b = a\nlet-math \\paren a = a\nlet-block +math m = '<>\nin '<+math(${\\frac{\\paren{2n + 1}}   {6}});>\n",
                "let-math \\frac a b = a\nlet-math \\paren a = a\nlet-block +math m = '<>\nin '< +math(${\\frac{\\paren{2n + 1}} {6}}); >\n",
                "the reported `+math(…)` line",
            ),
        ] {
            assert_eq!(fmt(src).as_deref(), Some(want), "{why}");
        }
    }

    /// **A multi-space run after a constructor head collapses**, and one that
    /// is a COLUMN does not.
    ///
    /// The second half is the whole reason [`Spacing::ctor_arg`] is
    /// [`Space::Collapse`] and not [`Space::One`]: all 35 of the corpus's
    /// runs at this boundary are hand-built columns, so a `Collapse` that
    /// bypassed [`Spacing::preserve_alignment`] would be a 35-boundary
    /// regression dressed as a two-line fix. It does not bypass it —
    /// `Collapse` delegates a non-empty gap to [`Space::One`]'s arm, which is
    /// where the column test lives — and this is where that is pinned.
    #[test]
    fn a_constructors_argument_column_survives_the_collapse() {
        // Lone run, nothing to line up with: collapses.
        assert_eq!(
            fmt("let y = Some   (1) in y\n").as_deref(),
            Some("let y = Some (1)\nin\ny\n"),
            "a lone run after a constructor head is not a column"
        );
        // Two adjacent lines whose `(` lands at the same column: a column,
        // and it survives. This is `slydifi.saty:30-32` in miniature.
        let column = "let x = let a = f Kana (1) in let b = f Latin (1) in a\nin\nx\n";
        assert_eq!(
            fmt(column).as_deref(),
            Some(column),
            "a constructor-argument column died"
        );
    }

    /// **A run after a unary minus collapses**, while both of the forms the
    /// corpus actually writes survive.
    ///
    /// 37 tight against 4 spaced and zero runs, so — as with
    /// [`Spacing::cmd_arg`] — the split is a reason to decline between the
    /// first two and no reason at all to keep the third.
    #[test]
    fn a_run_after_a_unary_minus_collapses() {
        for src in ["let x = (-1)\nin\nx\n", "let x = (- 1)\nin\nx\n"] {
            assert_eq!(fmt(src).as_deref(), Some(src), "{src:?} must be a no-op");
        }
        assert_eq!(
            fmt("let x = (-   1) in x\n").as_deref(),
            Some("let x = (- 1)\nin\nx\n")
        );
    }

    /// **A collapse never empties a gap**, at any of the three boundaries
    /// that now ask for one.
    ///
    /// The claim [`Spacing::cmd_arg`], [`Spacing::ctor_arg`] and
    /// [`Spacing::minus`] all rest on when they say [`sep::must_separate`] is
    /// not their concern: a run becomes ONE SPACE, so no two ranges that were
    /// separated in the input are written adjacently in the output and no
    /// fusion is reachable. Asserted rather than argued, because a `Collapse`
    /// that had been implemented as "write nothing unless the table objects"
    /// would pass every other test in this file and corrupt `1   pt` into a
    /// single length.
    #[test]
    fn a_collapse_never_writes_an_empty_gap() {
        for (src, why) in [
            // `-` is a WORD character here, so `-1` and `- 1` are different
            // programs and this is the boundary where emptying would show.
            ("let x = (-   1) in x\n", "a unary minus"),
            // A constructor whose group opener could not fuse, and one whose
            // argument begins with a word — the general shape.
            ("let y = Some   (1) in y\n", "a constructor head"),
            (
                "let-math \\frac a b = a\nin ${\\frac{a}   {6}}\n",
                "a math command",
            ),
        ] {
            let out = fmt(src).expect("must format");
            assert!(
                !out.contains("(-1)") && out.contains(' '),
                "{why}: {out:?} lost a gap that held whitespace"
            );
            // Every run of spaces the input had is still at least one space.
            assert_eq!(
                out.replace("   ", " "),
                out,
                "{why}: a run survived instead of collapsing"
            );
        }
    }

    /// A `#label` access and a stage sigil's grip on its operand, which the
    /// exception list keeps tight.
    ///
    /// This test used to say the rules did **not reach** a parameter run, a
    /// `#label` or a sigil — three shapes nothing named. Two of them are
    /// exceptions now ([`Spacing::access`], [`Spacing::sigil`]: 1,336 and 807
    /// corpus gaps, every one already tight) and the third, a parameter run,
    /// is ordinary program text that takes the default. The difference
    /// between "no rule names it" and "a rule names it and says leave it
    /// alone" is the whole of this change: only the second survives somebody
    /// adding a rule next door.
    #[test]
    fn a_label_access_and_a_stage_sigil_stay_tight() {
        for src in ["let x = (| a = 1 |)\nin\nx#a\n", "let x = &1\nin\n~x\n"] {
            assert_eq!(fmt(src).as_deref(), Some(src), "{src:?} must be a no-op");
        }
        // And they are TIGHT rather than merely uncopied: written apart, they
        // come back together.
        assert_eq!(
            fmt("let x = (| a = 1 |) in x #a\n").as_deref(),
            Some("let x = (| a = 1 |)\nin\nx#a\n")
        );
        assert_eq!(fmt("let x = & 1 in ~ x\n").as_deref(), Some("let x = &1\nin\n~x\n"));
        // A parameter run is not an exception at all — it is ordinary program
        // text and takes the default.
        assert_eq!(
            fmt("let f  x  y = x in f 1 2\n").as_deref(),
            Some("let f x y = x\nin\nf 1 2\n")
        );
    }

    /// Idempotence, on the shapes slice 2 moves. The rules write a fixed
    /// answer that does not read the input's spacing, so the second pass
    /// rewrites what the first wrote — but the property is what the sweep
    /// asserts and it is cheap to pin here too.
    #[test]
    fn the_record_rules_are_a_fixpoint() {
        for src in [
            "let x = (| a=1;b   =   2 ;c=3 |) in x\n",
            "let x = (| \n  a    = 1 ;\n  b = 2;\n |)\nin ()\n",
        ] {
            let once = fmt(src).expect("formats");
            let twice = fmt(&once).expect("formats its own output");
            assert_eq!(once, twice, "{src:?}: second pass differs");
        }
    }

    /// The fusion hazard, on the one record shape where it is not academic.
    ///
    /// A negative field value puts `=` next to `-`, and **both are
    /// `lexer.rs`'s `is_opsymbol`**, so `=-` is a single operator token: a
    /// version of this rule that wrote *no* space rather than one would turn
    /// `(| a = -1 |)` into a file that no longer means the same thing.
    /// [`Space::One`] cannot, because adding whitespace only ever separates —
    /// but the shape is what [`sep::must_separate`] is for and it is worth a
    /// fixture, because the day somebody makes a spacing rule tight is the day
    /// this becomes reachable.
    #[test]
    fn a_negative_field_value_keeps_its_operators_apart() {
        assert_eq!(
            fmt("let x = (| a= -1;b=0-2 |) in x\n").as_deref(),
            Some("let x = (| a = -1; b = 0 - 2 |)\nin\nx\n")
        );
        // And the control that says the hazard is real rather than imagined:
        // written tight, `=` and `-` ARE one token, so this input is not a
        // record of two fields at all and comes back untouched.
        let fused = "let x = (| a =-1 |) in x\n";
        assert_eq!(fmt(fused).as_deref(), Some(fused));
        assert!(
            sep::must_separate("=", "-1"),
            "`=` and `-1` are both is_opsymbol-headed and fuse into `=-`"
        );
        assert!(!sep::must_separate("1", ";"), "`;` fuses with nothing");
    }

    // -- slice 2, change 2: header lines -------------------------------------

    /// `@require:   x` -> `@require: x`. THE one rule that edits bytes inside
    /// a token span; [`Build::header_leaf`] is the argument.
    #[test]
    fn a_header_with_extra_spaces_is_normalised_to_one() {
        assert_eq!(
            fmt("@require:   stdja-mini\nlet x = 1 in x\n").as_deref(),
            Some("@require: stdja-mini\nlet x = 1\nin\nx\n")
        );
        // And the other way: a header with NO space gains one.
        assert_eq!(
            fmt("@import:./local\nlet x = 1 in x\n").as_deref(),
            Some("@import: ./local\nlet x = 1\nin\nx\n")
        );
        // `@stage:` too — three `Header` variants, one rule.
        assert_eq!(
            fmt("@stage:  persistent\nlet x = 1 in x\n").as_deref(),
            Some("@stage: persistent\nlet x = 1\nin\nx\n")
        );
    }

    /// The header token owns its own line terminator (`lexer.rs:915-933`), so
    /// the rule must copy it rather than judge by emitted text.
    /// `format.rs:536-556` records the bug from the other side: a formatter
    /// that added one made format-on-save append a blank line every save.
    ///
    /// CRLF is the sharper half — the terminator is TWO bytes and the lexer
    /// deliberately takes both, so a rewrite that reconstructed it would be
    /// where a lone `\r` gets left behind.
    #[test]
    fn a_header_keeps_its_terminator_exactly_including_crlf() {
        let out = fmt("@require:   stdja\r\nlet x = 1 in x\r\n").expect("formats");
        assert_eq!(out, "@require: stdja\r\nlet x = 1\r\nin\r\nx\r\n");
        let bytes = out.as_bytes();
        for (i, b) in bytes.iter().enumerate() {
            match b {
                b'\r' => assert_eq!(bytes.get(i + 1), Some(&b'\n'), "lone CR at {i} in {out:?}"),
                b'\n' => assert_eq!(
                    i.checked_sub(1).and_then(| j| bytes.get(j)),
                    Some(&b'\r'),
                    "lone LF at {i} in {out:?}"
                ),
                _ => {}
            }
        }
    }

    /// A header that is the file's LAST line with no trailing newline. The
    /// token then has no terminator at all, so a rule that assumed one would
    /// either invent it (slice 1 does not invent a final newline) or index
    /// past the end.
    #[test]
    fn a_header_as_the_files_last_line_without_a_newline() {
        // Not a complete program, so this exercises the identity path's
        // sibling — the buffer lexes, does not parse, and comes back
        // unchanged. Assert on the parsing shape instead, with the header
        // last inside a file that ends without a break.
        // `render::finish` supplies the final terminator these files lack,
        // which is orthogonal to this rule — what matters here is that the
        // rule does not index past the end of a token that has none, and does
        // not invent a SECOND one for a header that does.
        assert_eq!(fmt("@require:   x").as_deref(), Some("@require: x\n"));
        assert_eq!(fmt("@require: x").as_deref(), Some("@require: x\n"));
        assert_eq!(fmt("@require: x\n").as_deref(), Some("@require: x\n"));
        // Nothing after the colon at all: no space is invented, because a
        // space before end-of-input is trailing whitespace.
        assert_eq!(fmt("@require:").as_deref(), Some("@require:\n"));
        assert_eq!(fmt("@require:   ").as_deref(), Some("@require:\n"));
    }

    /// Only `\x20` is stripped, because only `\x20` is what `lex_header`
    /// strips into the payload (`lexer.rs:911-914` is
    /// `while self.peek() == Some(' ')`). A tab after the colon IS payload, so
    /// removing it would change `Token::HeaderRequire`'s content and the
    /// verifier would decline the whole file — a silent no-format. It is left
    /// exactly where it is, and no space is inserted in front of it either.
    #[test]
    fn a_tab_after_the_colon_is_payload_and_is_left_alone() {
        assert_eq!(
            fmt("@require:\tstdja\nlet x = 1 in x\n").as_deref(),
            Some("@require:\tstdja\nlet x = 1\nin\nx\n")
        );
        // And with spaces BEFORE the tab, the spaces go and the tab stays:
        // the lexer would have stripped exactly those spaces too.
        assert_eq!(
            fmt("@require:  \tstdja\nlet x = 1 in x\n").as_deref(),
            Some("@require:\tstdja\nlet x = 1\nin\nx\n")
        );
    }

    /// The payload-identity argument, asserted directly rather than trusted.
    ///
    /// This is the claim the whole rule rests on: `lex_header` strips the
    /// spaces after the colon INTO the payload, so `@require:   x` and
    /// `@require: x` carry the same `Token::HeaderRequire("x")`. If that ever
    /// stopped being true the rule would be silent corruption, and
    /// `same_tokens` compares payloads — so the formatter would decline rather
    /// than corrupt, and this test says which of the two happened.
    #[test]
    fn a_headers_payload_does_not_depend_on_the_spaces_after_its_colon() {
        let toks = |s: &str| -> Vec<Token> {
            rustyfi_syntax::lex_with_version(s, RustyfiVersion::V0_0)
                .expect("lexes")
                .into_iter()
                .map(| a| a.slot)
                .collect()
        };
        for (a, b) in [
            ("@require:x\n", "@require: x\n"),
            ("@require:   x\n", "@require: x\n"),
            ("@import:  ./y\r\n", "@import: ./y\r\n"),
            ("@stage:  0\n", "@stage: 0\n"),
            ("@require:   x", "@require: x"),
        ] {
            assert_eq!(toks(a), toks(b), "{a:?} and {b:?} must lex identically");
        }
        // The control: one character to the LEFT of the colon is the header's
        // NAME, and editing there is a different token. Says the licence is
        // about the space run and not about headers in general.
        assert_ne!(toks("@require: x\n"), toks("@import: x\n"));
    }

    /// **Two guards in this file that no test can kill, and why they stay.**
    ///
    /// Both were found the way `mod.rs`'s `eoi_always_ends_at_the_source_length`
    /// was: by deleting the code and watching everything stay green. Recorded
    /// here so nobody reads that green as coverage.
    ///
    /// 1. **[`Build::canonical_space`]'s [`sep::must_separate`] call.** Delete
    ///    it — write nothing, unconditionally — and 45 unit tests, the 162-file
    ///    corpus sweep and the always-on re-lex verifier all still pass. The
    ///    reason is that a [`Space::Tight`] rule only ever has five right-hand
    ///    sides in this grammar — `;`, `,`, `)`, `]` and ` |)` — and the first
    ///    character of each is outside every fusion class: none is
    ///    `char::is_alphanumeric`, none is one of `lexer.rs`'s `is_opsymbol`,
    ///    and no `(x, ';')`, `(x, ',')`, `(x, ')')` or `(x, ']')` row exists in
    ///    `FUSED_DELIMITERS`. ` |)`'s `|` IS an opsymbol, but a `(| … |)` body
    ///    cannot end in one.
    ///
    ///    It stays because the sign of the error is not symmetric: a
    ///    superfluous call costs one comparison, and a missing one corrupts a
    ///    document the first time somebody makes a rule tight where the
    ///    grammar does allow a fusing pair. The asserts below pin the
    ///    *reasoning* rather than the branch, so a change to `sep.rs` that
    ///    invalidated it would be caught here.
    /// 2. **[`Build::gap_upto`]'s "the gap is spaces and tabs only" guard.**
    ///    Rewriting a gap that holds a `%` comment would DELETE the comment,
    ///    which is why the guard exists — but a `%` comment runs to end of
    ///    line (`lexer.rs:308-333`), so a gap holding one always holds a line
    ///    break too, and a gap with a break has already taken the other branch.
    ///    The single exception is a comment at end of file, where the next atom
    ///    is the zero-width `Eoi` — and no rule requests a gap before `Eoi`.
    ///
    ///    Same argument for keeping it: it is one `matches!` against deleting
    ///    a comment.
    #[test]
    fn the_two_guards_no_test_can_kill_and_the_reasoning_that_replaces_one() {
        // (1). Every right-hand side a `Space::Tight` rule can present,
        // against left-hand sides drawn from what can precede one.
        for prev in ["x", "1", "1pt", "`lit`", ")", "]", " |)", "true", "!r", "0.5"] {
            for next in [";", ",", ")", "]", " |)"] {
                assert!(
                    !sep::must_separate(prev, next),
                    "{prev:?} + {next:?} now fuses, so the tight rules are no                      longer covered by the reasoning in this test's doc                      comment — they need a real fixture, not an argument"
                );
            }
        }
        // And the control, so the loop above is not vacuous: the table DOES
        // answer `true` for pairs a tight rule would meet if the grammar put
        // one there.
        assert!(sep::must_separate("=", "-1"));
        assert!(sep::must_separate("1", "pt"));
        assert!(sep::must_separate("&", "&x"));
        assert!(sep::must_separate("|", ")"));

        // (2). A `%` comment always carries a line break with it, except at
        // end of file — which is the whole of the unreachability argument.
        assert_eq!(
            trivia::classify("% c\n"),
            Some(vec![Piece::Comment("% c"), Piece::Newline("\n")])
        );
        assert_eq!(trivia::classify("% c"), Some(vec![Piece::Comment("% c")]));
    }

    /// Tier 0: a buffer that does not LEX declines, unchanged from slice 0.
    /// Distinct from the tier-2 case above, and the distinction is the whole
    /// reason `format.rs:127-134` draws the line at lexing: without a token
    /// stream there is no area map either.
    #[test]
    fn a_file_that_does_not_lex_declines() {
        assert!(fmt("\t\tlet x = ) in x\n").is_none());
    }
}

/// The census accumulator, shared by both generations' `gap_census` tests so
/// that the two tables mean the same thing. Measurement only.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct Census {
    files: usize,
    /// area -> [empty, one, multi, break/comment/indent]
    by_area: std::collections::BTreeMap<String, [usize; 4]>,
    /// (area, exception class) -> [empty, one, multi]
    by_class: std::collections::BTreeMap<(String, &'static str), [usize; 3]>,
    /// the residue only, by token pair
    by_pair: std::collections::BTreeMap<String, [usize; 3]>,
}

#[cfg(test)]
impl Census {
    pub(crate) fn add(&mut self, path: &std::path::Path, src: &str, atoms: &[Atom], obs: Vec<Obs>) {
        self.files += 1;
        for o in obs {
            let text = &src[o.span.0..o.span.1];
            let slot = |i: usize| atoms.get(i).map(| a: &Atom| a.slot.clone());
            let bucket = match (o.has_break || !o.blank || o.at_line_start, text.len()) {
                (true, _) => 3,
                (false, 0) => 0,
                (false, 1) => 1,
                (false, _) => 2,
            };
            self.by_area.entry(format!("{:?}", o.area)).or_default()[bucket] += 1;
            if bucket == 3 || !o.area.claimed() {
                continue;
            }
            let prev = o.at.checked_sub(1).and_then(slot);
            let next = slot(o.at);
            let class = match o.want {
                // `cmd_arg` and `minus` both requested the same thing, so the
                // report used to conflate them. They are split on the token to
                // the LEFT — a unary minus's gap has an `ExactMinus` there and
                // a command's argument boundary a command head — because the
                // two are separate exceptions with separate corpus counts and
                // a shared row is a row neither of them can be re-measured
                // from.
                Some(Space::Collapse) if matches!(prev, Some(Token::ExactMinus)) => {
                    "unary minus (Collapse)"
                }
                Some(Space::Collapse) => "cmd_arg (Collapse)",
                Some(Space::One) => "explicit One",
                Some(Space::Keep) => "explicit Keep",
                Some(Space::Tight) => "explicit Tight",
                None => {
                    if matches!(prev, Some(Token::Constructor(_)) | Some(Token::LongUpper(_, _)))
                        && next.as_ref().is_some_and(opens_a_group)
                    {
                        "ctor_arg (Collapse)"
                    } else if prev.as_ref().is_some_and(opens_a_group)
                        || next.as_ref().is_some_and(closes_a_group)
                    {
                        "bracket (Tight)"
                    } else if next.as_ref().is_some_and(is_separator) {
                        "separator (Tight)"
                    } else if matches!(prev, Some(Token::Access))
                        || matches!(next, Some(Token::Access))
                    {
                        "access (Tight)"
                    } else if prev.as_ref().is_some_and(is_sigil)
                        || matches!(next, Some(Token::OptionalType))
                    {
                        "sigil (Tight)"
                    } else if prev.as_ref().is_some_and(is_script)
                        || next.as_ref().is_some_and(is_script)
                    {
                        "script (Tight)"
                    } else if matches!(next, Some(Token::Eoi)) || next.is_none() {
                        "end of file (Keep)"
                    } else {
                        "universal"
                    }
                }
            };
            self.by_class
                .entry((format!("{:?}", o.area), class))
                .or_default()[bucket] += 1;
            // `CENSUS_CLASS=<class>` dumps every MULTI-space gap that class
            // claims, with 45 bytes of context either side. The sampler the
            // `Keep` -> `Collapse` decision was hand-checked with: a count
            // says how many runs a boundary has, and only reading them says
            // whether any of them is deliberate.
            if bucket == 2 && std::env::var("CENSUS_CLASS").is_ok_and(| v| class.starts_with(&v)) {
                let lo = o.span.0.saturating_sub(45);
                let hi = (o.span.1 + 45).min(src.len());
                let lo = (lo..=o.span.0).find(| i| src.is_char_boundary(*i)).unwrap_or(o.span.0);
                let hi = (o.span.1..=hi).rev().find(| i| src.is_char_boundary(*i)).unwrap_or(o.span.1);
                eprintln!("RUN {} {}: {:?}", class, path.display(), &src[lo..hi]);
            }
            if class == "universal" {
                if std::env::var("CENSUS_PAIR").is_ok_and(| v| {
                    v == format!(
                        "{}_{}",
                        prev.as_ref().map(kind_of).unwrap_or("-"),
                        next.as_ref().map(kind_of).unwrap_or("-")
                    )
                }) && bucket == 0
                {
                    let lo = o.span.0.saturating_sub(45);
                    let hi = (o.span.1 + 45).min(src.len());
                    let lo = (lo..=o.span.0).find(| i| src.is_char_boundary(*i)).unwrap_or(o.span.0);
                    let hi = (o.span.1..=hi).rev().find(| i| src.is_char_boundary(*i)).unwrap_or(o.span.1);
                    eprintln!("SAMPLE {}: {:?}", path.display(), &src[lo..hi]);
                }
                let key = format!(
                    "{:8} {:>16} _ {:16}",
                    format!("{:?}", o.area),
                    prev.map(| t| kind_of(&t)).unwrap_or("-"),
                    next.map(| t| kind_of(&t)).unwrap_or("-"),
                );
                self.by_pair.entry(key).or_default()[bucket] += 1;
            }
        }
    }

    pub(crate) fn report(&self, what: &str) {
        eprintln!("\n=== {what}: {} files ===", self.files);
        eprintln!("\narea            empty     one   multi   break/comment/indent");
        for (a, c) in &self.by_area {
            eprintln!("{a:12} {:7} {:7} {:7} {:9}", c[0], c[1], c[2], c[3]);
        }
        eprintln!("\narea     exception class            empty     one   multi");
        for ((a, k), c) in &self.by_class {
            eprintln!("{a:8} {k:24} {:7} {:7} {:7}", c[0], c[1], c[2]);
        }
        let mut rows: Vec<_> = self.by_pair.iter().collect();
        rows.sort_by_key(| (_, c)| std::cmp::Reverse(c[0] + c[2]));
        eprintln!("\nthe RESIDUE (`universal`), by token pair, worst first:");
        for (k, c) in rows.iter().take(30) {
            eprintln!("  {k}  {:7} {:7} {:7}", c[0], c[1], c[2]);
        }
        eprintln!("  … {} distinct pairs in all", rows.len());
        let tot = |sel: fn(&[usize; 3]) -> usize| -> usize {
            self.by_class.values().map(&sel).sum()
        };
        eprintln!(
            "\nreachable gaps: {} empty, {} one, {} multi",
            tot(| c| c[0]),
            tot(| c| c[1]),
            tot(| c| c[2])
        );
    }
}

/// The token's variant name, for the census table.
#[cfg(test)]
fn kind_of(t: &Token) -> &'static str {
    let s: &'static str = Box::leak(format!("{t:?}").into_boxed_str());
    match s.find(['(', ' ']) {
        Some(i) => &s[..i],
        None => s,
    }
}

/// The corpus files under `dirs`, the same roots the sweeps use.
#[cfg(test)]
pub(crate) fn corpus_files(dirs: &[&str]) -> Vec<std::path::PathBuf> {
    fn collect(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                collect(&p, out);
            } else if matches!(
                p.extension().and_then(| s| s.to_str()),
                Some("saty" | "satyh" | "satyg")
            ) {
                out.push(p);
            }
        }
    }
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("the crate sits two levels below the workspace root")
        .to_path_buf();
    let mut out = Vec::new();
    for d in dirs {
        collect(&root.join(d), &mut out);
    }
    out.sort();
    out
}


// ---------------------------------------------------------------------------
// slice 3: line-break decisions
// ---------------------------------------------------------------------------

#[cfg(test)]
mod slice3 {
    //! The claims slice 3 adds, and the three that were reachable only once
    //! groups were live.
    //!
    //! `tests` above carries the per-construct layouts; this module carries
    //! the properties of the DECISION — the budget fence-post, the area
    //! boundary, and the per-construct switch that makes each rule
    //! individually falsifiable.

    use super::*;
    use rustyfi_syntax::RustyfiVersion;

    fn at(src: &str, max_width: usize) -> String {
        super::super::format_cst(
            src,
            RustyfiVersion::V0_0,
            &super::super::CstOptions { max_width, ..Default::default() },
        )
        .expect("a file that lexes and parses is laid out")
    }

    fn doc_of<'s>(src: &'s str, atoms: &[Atom], file: &cst::File, breaks: Breaks) -> Doc<'s> {
        build(src, atoms, file, 2, breaks, None, false, DEFAULT_MAX_BLANK_LINES)
            .expect("laid out")
    }

    fn render_with(doc: &Doc<'_>, max_width: usize) -> String {
        render::render(
            doc,
            &render::Options { max_width, indent: 2, newline: "\n", max_blank_lines: 2 },
        )
    }

    fn parsed(src: &str) -> (Vec<Atom>, cst::File) {
        let atoms = rustyfi_syntax::lex_with_version(src, RustyfiVersion::V0_0).expect("lexes");
        let file = rustyfi_syntax::cst::parse_file(src).expect("parses");
        (atoms, file)
    }

    /// **The fence-post.** A group is measured against `max_width - col`, and
    /// the two off-by-ones that survive every "is it under the budget"
    /// assertion are `- 1` and `- 2`: a corpus whose lines are mostly well
    /// under the budget never notices, and neither does a test that only
    /// checks the output fits.
    ///
    /// What catches it is a construct measured at EXACTLY the budget, one
    /// column under and one column over. `let x = (| aaa = 1; |)` is 21
    /// columns wide; at 21 it must stay flat, at 20 it must break, and the
    /// pair straddles every one-column error there is.
    #[test]
    fn a_group_that_is_exactly_the_budget_wide_stays_flat_and_one_column_over_breaks() {
        let src = "let x = (| aaa = 1; |)\nin\n()\n";
        let flat = "let x = (| aaa = 1; |)";
        assert_eq!(render::width(flat), 22);
        // Exactly the budget: flat.
        assert!(
            at(src, 22).starts_with(flat),
            "at exactly its own width the group must stay flat, got {:?}",
            at(src, 22)
        );
        // One under: broken.
        assert!(
            !at(src, 21).starts_with(flat),
            "one column under its width the group must break, got {:?}",
            at(src, 21)
        );
        // One over: still flat.
        assert!(at(src, 23).starts_with(flat));
        // And every one of the three is a fixpoint, which is the half a width
        // assertion cannot see.
        for w in [21usize, 22, 23] {
            let once = at(src, w);
            assert_eq!(at(&once, w), once, "not a fixpoint at {w}");
        }
    }

    /// The same, one construct up: **a wrapped line changes the width
    /// available to the next construct**, so the budget sweep has to include a
    /// shape whose second line is what the third decision is measured
    /// against.
    #[test]
    fn a_wrapped_line_leaves_the_next_construct_a_different_budget_and_still_settles() {
        let src = "\
let f ctx =
  let r = (| alpha = 1; beta = 2; gamma = 3; |) in
  ctx |> set-a r |> set-b r |> set-c r
in ()
";
        for w in 24..=64 {
            let once = at(src, w);
            let twice = at(&once, w);
            assert_eq!(twice, once, "not a fixpoint at max_width {w}");
            // And the token stream is the same at every width, which is what
            // says a break decision never fused or split a token.
            let a = rustyfi_syntax::lex_with_version(src, RustyfiVersion::V0_0).unwrap();
            let b = rustyfi_syntax::lex_with_version(&once, RustyfiVersion::V0_0).unwrap();
            assert_eq!(
                a.iter().map(|t| t.slot.clone()).collect::<Vec<_>>(),
                b.iter().map(|t| t.slot.clone()).collect::<Vec<_>>(),
                "the token stream changed at max_width {w}"
            );
        }
    }

    /// **The area boundary, now that groups are live.** `engine.md` section 4:
    /// a text area's re-wrappable width does not exist, so a MULTILINE area
    /// reports `fits == false` and forces its enclosing `Auto` group open.
    ///
    /// Slice 1 never reached this — it emitted no group at all — so
    /// `render.rs`'s `a_multiline_verbatim_forces_its_group_open` was the only
    /// thing asserting it, on a hand-built `Doc`. This asserts it end to end,
    /// from source: the record CANNOT be flat however wide the budget, because
    /// one of its fields holds a four-line inline area that must not be
    /// re-wrapped at any width.
    #[test]
    fn a_multiline_text_area_forces_its_enclosing_group_open_at_any_width() {
        let src = "\
let r = (| a = {alpha
beta
gamma
delta}; b = 1; |)
in ()
";
        let out = super::super::format_cst(
            src,
            RustyfiVersion::V0_0,
            &super::super::CstOptions {
                max_width: 10_000,
                wrap_inline_text: false,
                ..Default::default()
            },
        )
        .expect("laid out");
        assert!(
            out.lines().count() > 4,
            "a ten-thousand-column budget must NOT flatten a record holding a \
             multi-line text area — the area's newlines are content: {out:?}"
        );
        assert!(out.contains("(|\n"), "the record group must be open: {out:?}");
    }

    /// **Mutation control 1: every group forced FLAT.** With no construct
    /// offering a break, a long file comes out as one line per hard-broken
    /// block — and, critically, the author's own breaks still do not come
    /// back. `NO_BREAKS` is not `LineBreaks::Preserve`.
    #[test]
    fn with_no_construct_offering_a_break_nothing_wraps_and_nothing_is_preserved() {
        let src = "\
let f ctx =
  let r = (| alpha = 1;
             beta = 2; |) in
  ctx
in ()
";
        let (atoms, file) = parsed(src);
        let out = render_with(&doc_of(src, &atoms, &file, NO_BREAKS), 20);
        assert_eq!(
            out, "let f ctx = let r = (| alpha = 1; beta = 2; |) in ctx in ()\n",
            "with every break flag off, a 20-column budget must change nothing \
             and the author's breaks must still be gone"
        );
    }

    /// **Mutation control 2: `op_chain`'s all-or-nothing.** The rule exists
    /// because `cst.rs:1039-1058` flattens all ten precedence levels, so the
    /// formatter cannot know where a precedence boundary is. Turning the flag
    /// off leaves the chain unbroken — which is the honest failure — and the
    /// test that would catch a FILL instead is the layout assertion in
    /// `an_operator_chain_breaks_before_the_operator_at_one_depth`: every
    /// operator on its own line, never some of them.
    #[test]
    fn the_op_chain_rule_is_all_or_nothing_and_its_flag_turns_it_off() {
        let src = "let f ctx = ctx |> set-a 1 |> set-b 2 |> set-c 3\nin\n()\n";
        let (atoms, file) = parsed(src);
        let on = render_with(
            &doc_of(src, &atoms, &file, SLICE3),
            24,
        );
        // Every operator broke, or none did — never two of three.
        let broken = on.matches("\n    |>").count();
        assert_eq!(broken, 3, "all three operators must break together: {on:?}");
        let off = Breaks { op_chain: false, ..SLICE3 };
        let out = render_with(&doc_of(src, &atoms, &file, off), 24);
        assert!(
            !out.contains("\n    |>"),
            "with the flag off the chain must not break at all: {out:?}"
        );
    }

    /// **Mutation control 3: the per-construct switch is real.** Each flag,
    /// turned off on its own, must change the output of the construct it
    /// names and of nothing else — otherwise the rollout `engine.md` section
    /// 11 asks for is a fiction and a rule cannot be measured on its own.
    #[test]
    fn every_break_flag_changes_its_own_constructs_layout() {
        for (name, breaks, src, width) in [
            (
                "items",
                Breaks { items: false, ..SLICE3 },
                "let r = (| alpha = 1; beta = 2; gamma = 3; |)\nin\n()\n",
                24usize,
            ),
            (
                "clauses",
                Breaks { clauses: false, ..SLICE3 },
                "let f x =\n  match x with\n  | A -> 1\n  | B -> 2\nin\n()\n",
                100,
            ),
            (
                "blocks",
                Breaks { blocks: false, ..SLICE3 },
                "let a = 1\nlet b = 2\nin\n()\n",
                100,
            ),
            (
                "app_args",
                Breaks { app_args: false, ..SLICE3 },
                "let y = someFunction argOne argTwo argThree argFour\nin\n()\n",
                24,
            ),
            (
                "let_spine",
                Breaks { let_spine: false, ..SLICE3 },
                "let f x =\n  let aaaa = 1 in\n  let bbbb = 2 in\n  aaaa\nin\n()\n",
                24,
            ),
            (
                "bodies",
                Breaks { bodies: false, ..SLICE3 },
                "let title-deco = someLongFunction argumentOne argumentTwo\nin\n()\n",
                30,
            ),
            (
                "groups",
                Breaks { groups: false, ..SLICE3 },
                "let r = (| alpha = 1; beta = 2; gamma = 3; |)\nin\n()\n",
                24,
            ),
            (
                "block_items",
                Breaks { block_items: false, ..SLICE3 },
                "let d = document (| a = 1 |) '< +p{aaa} +p{bbb} +p{ccc} >\nin\n()\n",
                24,
            ),
            (
                // The only arm whose ON output is wider than its OFF output:
                // the flag does not offer a break, it takes one the author
                // already wrote and makes it unconditional.
                "block_blanks",
                Breaks { block_blanks: false, ..SLICE3 },
                "let d = document (| a = 1 |) '<\n+p{aaa}\n\n+p{bbb}\n>\nin\n()\n",
                100,
            ),
            (
                "type_arrows",
                Breaks { type_arrows: false, ..SLICE3 },
                "module M : sig\n  val f : alpha -> beta -> gamma -> delta\nend = struct\n  let f a b c = a\nend\nin\n()\n",
                30,
            ),
        ] {
            let (atoms, file) = parsed(src);
            let on = render_with(&doc_of(src, &atoms, &file, SLICE3), width);
            let off = render_with(&doc_of(src, &atoms, &file, breaks), width);
            assert_ne!(
                on, off,
                "turning `{name}` off changed nothing, so the flag is not \
                 wired to the construct it names"
            );
            // Both arms still have to be fixpoints: a flag is a layout choice,
            // never a correctness one.
            let (a2, f2) = parsed(&off);
            assert_eq!(
                render_with(&doc_of(&off, &a2, &f2, breaks), width),
                off,
                "`{name}` off is not a fixpoint"
            );
        }
    }
}
