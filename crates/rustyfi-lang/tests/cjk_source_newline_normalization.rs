//! A source newline between two CJK characters must typeset as if it had never
//! been written — the same boxes, the same JLreq pair spacing, the same break
//! opportunity.
//!
//! Upstream normalizes the whole character list BEFORE it classifies anything:
//! `append_property` folds `normalization_rule`
//! (`chardecoder/lineBreakDataMap.ml:143-157`) over the entire list
//! (`:315-332`), and only the result is handed to `cut_into_segment_record`
//! (`:482`). So for upstream `日本語\n日本語` and `日本語日本語` are literally the
//! same text.
//!
//! This port deleted the same characters, but did it as a `continue` INSIDE the
//! box-building loop, whose two lookaheads still read the raw text:
//!
//!   * `cjk_pair_space`'s `next_char` saw the newline, decided the boundary was
//!     not CJK-CJK, and emitted no pair spacing at all;
//!   * `boundary[after]` answered for the `語`/`\n` boundary (LB6 — never break
//!     before a line feed) instead of for the `語`/`日` one (a legal break).
//!
//! Every wrapped CJK paragraph therefore lost its pair spacing at one boundary
//! per source line, and the bundled corpus is largely Japanese.
//! `normalize_source_whitespace` now runs as its own pass ahead of
//! `uax14_boundaries`, which is upstream's order.
//!
//! # How this file asserts, and why
//!
//! The obvious gate for "`日本語\n日本語` typesets like `日本語日本語`" is
//! `boxes_for(a) == boxes_for(b)`, and this file used to be six of those. A
//! relative assertion is only as good as its reference side: any change that
//! moves BOTH sides survives it. A mutation sweep of
//! `normalize_source_whitespace` confirmed that concretely — deleting
//! `Script::Kana` from its CJK predicate, or dropping `next_cjk` from the
//! "keep one space between two CJK characters" condition, left every equality
//! in this file true while changing what the typesetter emits.
//!
//! So every case here now states ABSOLUTELY what the boxes are —
//! `spacing_shape` renders the run into one `String` per box naming its glyph
//! content and the exact spacing it carries. Where equality with a reference
//! string genuinely IS the property (`日\n日` must render exactly as `日日`),
//! the equality is kept AND the reference side is pinned absolutely next to it,
//! so a wrong reference cannot hide.
//!
//! `spacing_shape` deliberately does NOT render a `Discretionary`'s penalty.
//! Everything this file is about — which source whitespace survives, and what
//! spacing the resulting boundary carries — lives in the glyph runs, the
//! interword/inter-script glue and the `no_break` pair kern, none of which a
//! penalty change touches. `break_shape` adds the penalties back.
//!
//! # The break at a JOINED boundary is withheld, and only there
//!
//! Deleting a character JOINS its two neighbours, and that join has two
//! separable consequences: the two survivors must be SPACED as if the author
//! had written them adjacent, and UAX#14 run over the joined text would also
//! offer a BREAK between them that the author's own text never offered
//! (`語\n日` classifies as `語日`; the raw text's boundary was LB6, never break
//! before a line feed). This port takes the first and declines the second —
//! `joined_at` in `text_to_boxes` clears `boundary` at every join, so the site
//! emits the identical spacing boxes under a `NO_BREAK_PENALTY` discretionary.
//!
//! That is a scope decision, and it was MEASURED: granting the new break
//! opportunities lets the line-breaker pack one extra line into `easytable`
//! (`lines_dev` 2 -> 3, a layout-fidelity gate) while spacing alone leaves
//! every gated metric at or better than baseline. Whether the break should
//! ALSO be granted is a separate change with its own fidelity consequences.
//!
//! Three groups of assertions here therefore concern break PLACEMENT rather
//! than whitespace deletion, and a variant that changed the decision above
//! would move exactly these and nothing else:
//!
//!   * `the_boundary_under_a_deleted_newline_carries_pair_spacing` and
//!     `a_deleted_newline_leaves_an_ordinary_cjk_break_opportunity`, the two
//!     `break_shape` tests;
//!   * every use of `bare_boundary()`, the zero-spacing `Discretionary` a
//!     Latin/CJK seam carries. It is pure break placement — it holds no boxes
//!     at all — so a joined seam does not emit it. Nothing is lost: the
//!     0.24em inter-script glue that follows is an `OuterEmpty`, and
//!     `PureHorzBox::is_break_point` counts any glue as a candidate, so the
//!     seam stays breakable either way. `joined_seam_*` spells the joined
//!     form and `unjoined_seam_*` the written-adjacent one.
//!
//! Every other test pins normalization, which the break decision does not
//! touch.
//!
//! # The one mutation this file deliberately does not catch
//!
//! `normalize_source_whitespace` updates `prev` on a KEPT space. NOT doing so
//! -- leaving `prev` at the last kept NON-whitespace character -- is an
//! EQUIVALENT mutant, and no test should be written for it.
//!
//! `prev` is read in exactly one place, `prev.is_some_and(is_cjk)`, and only
//! while looking at a whitespace character. The two versions can first differ
//! only just after a whitespace character `w` was kept. Let `p` be the last
//! kept NON-whitespace character before `w` -- the value the mutant retains
//! where the original stores `w`.
//!
//!   * If `is_cjk(p)`, then `prev_cjk` was true at `w`, so `w` survived only
//!     via `c == ' ' && !run_continues && ..`, and `!run_continues` says the
//!     next character is not whitespace. A non-whitespace character is always
//!     kept, so it overwrites `prev` in BOTH versions before any later
//!     whitespace character can read it: the divergence dies unread.
//!   * If `!is_cjk(p)`, the two `prev` values are `w` (a space or a newline,
//!     never CJK) and `p` (not CJK). Both answer `prev_cjk == false` at every
//!     whitespace character that reads them, and the next non-whitespace
//!     character reconverges them.
//!
//! So `prev_cjk` takes the same value at every whitespace character under both
//! versions, and the outputs are equal. `!run_continues` and the
//! `prev`-on-a-kept-space update are two encodings of one fact, which is why
//! dropping EITHER alone is invisible while dropping both is not.
//!
//! Corroborated exhaustively rather than by sampling: the function inspects a
//! character only through `c == ' ' || c == '\n'` and `is_cjk(c)`, so its
//! output depends only on the input's projection onto {CJK, OTHER, SP, BR}.
//! The two versions agree on all 22 369 621 strings of length 0..=12 over
//! those four classes, which is every input of length <= 12.

use rustyfi_backend::{Context, FontKey, FontMetrics, HorzBox, Length, PureHorzBox};
use rustyfi_lang::eval::Interp;
use rustyfi_lang::primitives;
use rustyfi_lang::quoted::IText;
use rustyfi_lang::value::Env;

/// Every char is half an em wide (mirrors `cjk_prevented_break_glue.rs`), so
/// widths are deterministic and no real font data is needed.
struct Mono;

impl FontMetrics for Mono {
    fn advance(&self, _f: FontKey, _c: char, size: Length) -> Option<Length> {
        Some(size * 0.5)
    }
    fn ascender(&self, _f: FontKey, size: Length) -> Length {
        size * 0.75
    }
    fn descender(&self, _f: FontKey, size: Length) -> Length {
        size * 0.25
    }
}

fn boxes_for(ctx: &Context, text: &str) -> Vec<HorzBox> {
    let mono = Mono;
    let mut interp = Interp::new(&mono);
    let elems = vec![IText::Text(text.to_string())];
    primitives::read_inline(&mut interp, ctx, &elems, &Env::root()).expect("read_inline")
}

fn ctx() -> Context {
    Context::initial(Length::pt(200.0))
}

// ---------------------------------------------------------------------------
// Absolute rendering
// ---------------------------------------------------------------------------

/// Lengths are reported as a ratio of `font_size`, so the constants below read
/// as the em ratios `Context::initial` actually holds
/// (`space_natural`/`space_shrink`/`space_stretch` = 0.33/0.08/0.16, the
/// Latin/CJK `script_space_map` entry = 0.24) rather than as points.
fn em(ctx: &Context, l: Length) -> String {
    format!("{:.4}", l.0 / ctx.font_size.0)
}

fn glue_desc(ctx: &Context, p: &PureHorzBox) -> String {
    match p {
        PureHorzBox::OuterEmpty {
            natural,
            shrinkable,
            stretchable,
        } => format!(
            "glue {} -{} +{}",
            em(ctx, *natural),
            em(ctx, *shrinkable),
            em(ctx, *stretchable)
        ),
        // The RIGID half of a JLreq pair space (`cjk_pair_space`'s
        // `ideographic_single` kerns, always negative here — they tighten).
        // A joined boundary keeps it; see the module note.
        PureHorzBox::FixedEmpty { width } => format!("kern {}", em(ctx, *width)),
        other => format!("{other:?}"),
    }
}

/// One `String` per box: its glyph content, or the exact spacing it carries.
/// A `Discretionary`'s PENALTY is deliberately omitted — see the module note.
fn spacing_shape(ctx: &Context, boxes: &[HorzBox]) -> Vec<String> {
    boxes
        .iter()
        .map(|HorzBox::Pure(p)| match p {
            PureHorzBox::InnerString { text, width, .. } => {
                format!("{text:?} w={}", em(ctx, *width))
            }
            PureHorzBox::Discretionary {
                pre_break,
                post_break,
                no_break,
                ..
            } => format!(
                "boundary pre[{}] post[{}] nobreak[{}]",
                pre_break
                    .iter()
                    .map(|b| glue_desc(ctx, b))
                    .collect::<Vec<_>>()
                    .join(", "),
                post_break
                    .iter()
                    .map(|b| glue_desc(ctx, b))
                    .collect::<Vec<_>>()
                    .join(", "),
                no_break
                    .iter()
                    .map(|b| glue_desc(ctx, b))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            other => glue_desc(ctx, other),
        })
        .collect()
}

/// `spacing_shape` plus every `Discretionary`'s penalty. Used ONLY by the two
/// break-placement tests.
fn break_shape(ctx: &Context, boxes: &[HorzBox]) -> Vec<String> {
    boxes
        .iter()
        .zip(spacing_shape(ctx, boxes))
        .map(|(HorzBox::Pure(p), d)| match p {
            PureHorzBox::Discretionary { penalty, .. } => format!("{d} penalty={penalty}"),
            _ => d,
        })
        .collect()
}

fn shape(text: &str) -> Vec<String> {
    let ctx = ctx();
    spacing_shape(&ctx, &boxes_for(&ctx, text))
}

// --- expected-box constructors ---------------------------------------------
//
// Naming one box each, so an expectation reads as the run it describes and a
// wrong count/order shows up as a diff rather than as a bare `false`.

/// A glyph run. Width is asserted too: `Mono` advances every char by half an
/// em, so `"Alpha"` must measure 2.5em — a run silently split or merged moves
/// this.
fn run(text: &str) -> String {
    format!("{:?} w={:.4}", text, 0.5 * text.chars().count() as f64)
}

/// A CJK-CJK boundary carrying the JLreq pair space: zero natural width with
/// 0.025em of stretch and no shrink (`cjk_pair_space`'s default class pair).
fn cjk_pair() -> String {
    "boundary pre[] post[] nobreak[glue 0.0000 -0.0000 +0.0250]".to_string()
}

/// A CJK-CJK boundary whose left character is `hwsoft` (`、`, `。`): net-zero
/// natural width with a quarter em of give either way.
fn cjk_pair_hwsoft() -> String {
    "boundary pre[] post[] nobreak[glue 0.0000 -0.2500 +0.2500]".to_string()
}

/// The bare chunk boundary at a Latin/CJK seam — no pair space of its own; the
/// inter-script glue that follows supplies the spacing. Being empty, it is
/// PURE break placement, and a joined seam withholds it (module note).
fn bare_boundary() -> String {
    "boundary pre[] post[] nobreak[]".to_string()
}

/// `set-space-ratio`'s interword glue — what an author's SURVIVING space
/// becomes. Its presence is the whole question in most of this file.
fn interword() -> String {
    "glue 0.3300 -0.0800 +0.1600".to_string()
}

/// `script_space_map[Latin][Han]` = 0.24em, rigid. Emitted by
/// `insert_box_interscript_glue` from the two scripts alone, so it appears
/// whether or not the author wrote a space there — which is exactly why the
/// author's space at such a seam is DELETED rather than kept.
fn interscript() -> String {
    "glue 0.2400 -0.0000 +0.0000".to_string()
}

/// `日本語`, the three-glyph CJK run with its two internal pair boundaries.
fn nihongo() -> Vec<String> {
    vec![run("日"), cjk_pair(), run("本"), cjk_pair(), run("語")]
}

fn chain(parts: &[&[String]]) -> Vec<String> {
    parts.iter().flat_map(|p| p.iter().cloned()).collect()
}

/// `日本語Alpha` written adjacent: the seam's own bare break candidate, then
/// the inter-script glue.
fn unjoined_seam_cjk_latin() -> Vec<String> {
    chain(&[&nihongo(), &[bare_boundary(), interscript(), run("Alpha")]])
}

/// The same seam reached by DELETING whitespace. Identical spacing — the
/// 0.24em glue and nothing else — with the bare candidate withheld, because a
/// join grants no break the author did not write (module note). The glue is
/// itself a breakpoint, so the seam is no less breakable than above.
fn joined_seam_cjk_latin() -> Vec<String> {
    chain(&[&nihongo(), &[interscript(), run("Alpha")]])
}

fn unjoined_seam_latin_cjk() -> Vec<String> {
    chain(&[&[run("Alpha"), bare_boundary(), interscript()], &nihongo()])
}

fn joined_seam_latin_cjk() -> Vec<String> {
    chain(&[&[run("Alpha"), interscript()], &nihongo()])
}

// ---------------------------------------------------------------------------
// The fix
// ---------------------------------------------------------------------------

/// The bug, at its smallest: a bare newline between two CJK runs. ABSOLUTE —
/// the six glyphs and the five pair boundaries, one of which sits exactly where
/// the newline was, and no interword glue anywhere.
#[test]
fn a_newline_between_two_cjk_runs_typesets_as_if_absent() {
    let joined = chain(&[&nihongo(), &[cjk_pair()], &nihongo()]);
    assert_eq!(
        shape("日本語\n日本語"),
        joined,
        "the newline leaves a pair boundary, not glue"
    );
    // The reference side, pinned: `日本語日本語` really is the eleven boxes
    // above. Without this the equality below could be two identical wrongs.
    assert_eq!(shape("日本語日本語"), joined);
    assert_eq!(
        shape("日本語\n日本語"),
        shape("日本語日本語"),
        "a newline between two CJK characters must leave no trace"
    );
}

/// The one-glyph form of the same thing, where "renders exactly as the
/// reference string" is the whole property and there is nothing else in the
/// run to distract from it.
#[test]
fn the_smallest_deleted_newline_is_one_pair_boundary() {
    assert_eq!(shape("日\n日"), vec![run("日"), cjk_pair(), run("日")]);
    assert_eq!(shape("日日"), vec![run("日"), cjk_pair(), run("日")]);
    assert_eq!(shape("日\n日"), shape("日日"));
}

/// A newline followed by a trailing space is deleted WHOLE: the space matches
/// `([nonspaced; set [SP; INBR]], [], [])` (its right neighbour is the newline,
/// not a character), and the newline then matches
/// `([nonspaced; exact INBR], [], [nonspaced])`.
///
/// ABSOLUTE, because the interesting failure is "one space survives" — which
/// keeps the two sides of the old equality unequal only if the reference is
/// right, and shows up here directly as an `interword()` box that should not be
/// there.
#[test]
fn a_space_then_a_newline_between_two_cjk_runs_typesets_as_if_absent() {
    let joined = chain(&[&nihongo(), &[cjk_pair()], &nihongo()]);
    assert_eq!(
        shape("日本語 \n日本語"),
        joined,
        "a trailing space before the newline is deleted along with it"
    );
    assert_eq!(shape("日本語日本語"), joined);
}

/// CONTROL, and a correction to a note this fix nearly repeated: a newline
/// followed by a continuation-line INDENT does NOT vanish entirely. Upstream's
/// `normalize` matches each rule's left context against the ALREADY-NORMALIZED
/// output (`bihead :: Alist.to_list_rev biacc`) and its right context against
/// the RAW tail, so a run of whitespace between two CJK characters collapses to
/// exactly ONE space rather than to nothing: every member but the last is
/// deleted by rule 5 (its right neighbour is more whitespace), and the last one
/// then sees the CJK character directly and is preserved as `bispace` by
/// `([nonspaced; exact SP], [bispace], [nonspaced])`. The port matched this
/// before the pass moved and still matches it — moving the rewrite earlier must
/// not quietly turn it into a full collapse.
///
/// ABSOLUTE: exactly one `interword()` box at the seam and no pair boundary
/// there. The old `== boxes_for("日本語 日本語")` form could not tell "collapses
/// to one space" from "collapses to two" if the reference had been wrong, and
/// could not tell either from "collapses to nothing" without its companion
/// `assert_ne!`.
#[test]
fn a_whitespace_run_between_two_cjk_runs_collapses_to_one_space() {
    let one_space = chain(&[&nihongo(), &[interword()], &nihongo()]);
    assert_eq!(
        shape("日本語\n      日本語"),
        one_space,
        "the LAST space of the run survives as `bispace`, and only it"
    );
    // Three more spellings of "a whitespace run between two CJK characters",
    // all of which must land on the same single space.
    assert_eq!(shape("日本語  日本語"), one_space);
    assert_eq!(shape("日本語 \n 日本語"), one_space);
    // The reference the old relative form leaned on, pinned.
    assert_eq!(shape("日本語 日本語"), one_space);
    // ... and the boundary of the rule, which is what makes it a COLLAPSE and
    // not a "keep one of whatever the run was": a run that ends in a NEWLINE
    // has no `bispace` survivor, so it goes entirely. `\n\n` and `\n   \n`
    // are the wrapped-paragraph shapes that hit this.
    let joined = chain(&[&nihongo(), &[cjk_pair()], &nihongo()]);
    assert_eq!(
        shape("日本語\n\n日本語"),
        joined,
        "a run ending in a newline leaves nothing"
    );
    assert_eq!(shape("日本語 \n  \n日本語"), joined);
}

/// VACUITY CONTROL for the two above: the boundary the newline sat at is not
/// merely joined, it carries the pair spacing and the break opportunity that
/// any other CJK-CJK boundary carries. `、あ` is `AllowBreak` (LB13 forbids a
/// break before a comma, not after one) and its `hwsoft` class space nets to
/// zero natural width with 0.25em of give either way — see
/// `cjk_prevented_break_glue::allowed_boundary_is_still_a_candidate`. Before
/// the normalization pass moved ahead of `uax14_boundaries` this boundary
/// emitted NOTHING: no glue and no candidate.
///
/// BREAK PLACEMENT: one of the two `break_shape` tests. The SPACING is the
/// written-adjacent spacing exactly — same box, same glue, same widths — and
/// the boundary is a `NO_BREAK_PENALTY` one where the written-adjacent form is
/// a penalty-0 candidate. That contrast is the whole of the module note's
/// rule, in three lines.
#[test]
fn the_boundary_under_a_deleted_newline_carries_pair_spacing() {
    let ctx = ctx();
    let joined = vec![
        run("、"),
        format!("{} penalty={}", cjk_pair_hwsoft(), i32::MAX),
        run("あ"),
    ];
    assert_eq!(
        break_shape(&ctx, &boxes_for(&ctx, "、\nあ")),
        joined,
        "the hwsoft pair glue, under a withheld break"
    );
    assert_eq!(
        break_shape(&ctx, &boxes_for(&ctx, "、あ")),
        vec![
            run("、"),
            format!("{} penalty=0", cjk_pair_hwsoft()),
            run("あ"),
        ],
        "written adjacent, the same glue IS a candidate"
    );
    // And the spacing — which is what this fix is about — is identical.
    assert_eq!(shape("、\nあ"), shape("、あ"));
}

/// BREAK PLACEMENT, the general case: a deleted newline leaves a boundary that
/// spaces like every other CJK-CJK boundary in the run and, alone among them,
/// declines to be a candidate.
#[test]
fn a_deleted_newline_leaves_an_ordinary_cjk_break_opportunity() {
    let ctx = ctx();
    let penalties = |t: &str| -> Vec<i32> {
        boxes_for(&ctx, t)
            .iter()
            .filter_map(|HorzBox::Pure(p)| match p {
                PureHorzBox::Discretionary { penalty, .. } => Some(*penalty),
                _ => None,
            })
            .collect()
    };
    assert_eq!(
        penalties("日本語日本語"),
        vec![0; 5],
        "written adjacent: five legal CJK boundaries"
    );
    assert_eq!(
        penalties("日本語\n日本語"),
        vec![0, 0, i32::MAX, 0, 0],
        "wrapped: the same five, the joined one withheld"
    );
    // Everything but the penalty is the same, which `spacing_shape` states
    // directly (and `a_newline_between_two_cjk_runs_typesets_as_if_absent`
    // pins absolutely).
    assert_eq!(shape("日本語\n日本語"), shape("日本語日本語"));
}

/// The joined boundary keeps the RIGID half of the pair space as well as the
/// elastic one. This is the one property that separates the shipped behaviour
/// from the cheaper variant that simply clears `boundary` and lets the
/// existing `NO_BREAK_PENALTY` arm run: that arm drops the kern (a deliberate,
/// measured divergence at boundaries where UPSTREAM emits `LBPure`), and a
/// join is not such a boundary — upstream deletes the whitespace and then
/// spaces the survivors exactly as if written adjacent.
///
/// `・` is the pair that shows it: `ideographic_single` gives it a −0.25em
/// trailing kern, so dropping the kern would set `・\nあ` a quarter em WIDER
/// than `・あ`, at every source line end. `、`/`。` cannot show it — their
/// class space and their kern net to zero, which is why every other case in
/// this file is blind to the difference.
#[test]
fn a_joined_boundary_keeps_the_rigid_half_of_the_pair_space() {
    let ctx = ctx();
    let kern = "kern -0.2500".to_string();
    let paired = format!("boundary pre[] post[] nobreak[{kern}, glue 0.0000 -0.0000 +0.0250]");
    assert_eq!(
        shape("・\nあ"),
        vec![run("・"), paired.clone(), run("あ")],
        "the −0.25em nakaten kern survives the join"
    );
    assert_eq!(
        shape("・あ"),
        vec![run("・"), paired, run("あ")],
        "written adjacent, byte for byte the same spacing"
    );
    assert_eq!(
        break_shape(&ctx, &boxes_for(&ctx, "・\nあ"))[1],
        format!(
            "boundary pre[] post[] nobreak[{kern}, glue 0.0000 -0.0000 +0.0250] penalty={}",
            i32::MAX
        ),
        "and only the penalty differs"
    );
}

/// VACUITY CONTROL: a SINGLE space between two CJK characters is the one piece
/// of CJK-adjacent whitespace upstream preserves
/// (`([nonspaced; exact SP], [bispace], [nonspaced])`). A fix that just deleted
/// every space would pass the equalities above and break this.
///
/// ABSOLUTE: the kept space is interword glue, and it REPLACES the pair
/// boundary rather than joining it. The old `assert_ne!` pair could not say
/// which of those two facts it was testing.
#[test]
fn a_single_space_between_two_cjk_runs_is_still_kept() {
    assert_eq!(
        shape("日本語 日本語"),
        chain(&[&nihongo(), &[interword()], &nihongo()]),
        "one literal space between two CJK characters survives as interword glue"
    );
    assert_ne!(
        shape("日本語 日本語"),
        shape("日本語日本語"),
        "a space is kept where a newline is deleted — the two are NOT equivalent"
    );
    assert_ne!(shape("日本語 日本語"), shape("日本語\n日本語"));
}

/// VACUITY CONTROL: whitespace that touches no CJK character is untouched, so
/// Latin prose is byte-identical to what it was before the pass existed (a
/// newline still becomes ordinary interword glue).
#[test]
fn whitespace_away_from_cjk_is_untouched() {
    assert_eq!(
        shape("Alpha\nbeta"),
        vec![run("Alpha"), interword(), run("beta")],
        "a Latin newline is interword glue, as before"
    );
    assert_eq!(
        shape("Alpha beta"),
        vec![run("Alpha"), interword(), run("beta")]
    );
    // A Latin whitespace RUN reaches the box builder untouched by
    // `normalize_source_whitespace` and becomes ONE interword glue there — the
    // box builder emits one glue per whitespace run, so "two spaces" and "one
    // space" were already indistinguishable downstream for Latin. Pinned so
    // that a pass which started deleting Latin whitespace (which would produce
    // NO glue and a hyphenation point, see the `Alphabeta` control below)
    // cannot be mistaken for this.
    assert_eq!(
        shape("Alpha  beta"),
        vec![run("Alpha"), interword(), run("beta")],
        "no CJK in sight, so nothing is deleted"
    );
    assert_ne!(
        shape("Alpha beta"),
        shape("Alphabeta"),
        "and it is not deleted"
    );
}

/// A CJK/Latin boundary's whitespace was already DELETED before this change
/// (the 0.24em inter-script glue supplies that spacing, not the author's
/// space) — but its CLASSIFICATION was not. `boundary[after]` still answered
/// for the deleted character's offset, i.e. LB6's "never break before a line
/// feed". Now the pass runs first, so the SPACING at a wrapped seam is exactly
/// the spacing at a written-adjacent one: the 0.24em glue, alone, with no
/// interword glue doubling it.
///
/// BREAK PLACEMENT: the written-adjacent form also carries a bare, zero-width
/// break candidate in front of that glue, and the joined form does not — the
/// module note's rule. Asserted here rather than hidden, because the two
/// shapes are otherwise identical and a reader will notice the difference.
#[test]
fn a_cjk_latin_boundary_across_a_newline_matches_the_unspaced_one_s_spacing() {
    assert_eq!(shape("日本語\nAlpha"), joined_seam_cjk_latin());
    assert_eq!(
        shape("日本語Alpha"),
        unjoined_seam_cjk_latin(),
        "the reference, pinned"
    );

    assert_eq!(shape("Alpha\n日本語"), joined_seam_latin_cjk());
    assert_eq!(
        shape("Alpha日本語"),
        unjoined_seam_latin_cjk(),
        "the reference, pinned"
    );

    // The ONLY difference between the two is that candidate: strip it from the
    // written-adjacent shape and the two are equal, box for box. So the seam's
    // spacing really is insensitive to the author's line breaks, which is what
    // this test is named for.
    let without = |v: Vec<String>| -> Vec<String> {
        v.into_iter().filter(|b| *b != bare_boundary()).collect()
    };
    assert_eq!(shape("日本語\nAlpha"), without(shape("日本語Alpha")));
    assert_eq!(shape("Alpha\n日本語"), without(shape("Alpha日本語")));
}

// ---------------------------------------------------------------------------
// The arms the equalities above never reached
// ---------------------------------------------------------------------------

/// `normalize_source_whitespace`'s doc comment claims `CJK + SP + Latin ->
/// deleted` and `Latin + SP + CJK -> deleted`, and until now nothing tested
/// either: every case in this file put CJK on BOTH sides, where the condition's
/// `prev_cjk` and `next_cjk` conjuncts are both true and neither can be told
/// apart from the other.
///
/// Dropping `next_cjk` from that condition keeps this space, which is visible
/// here as an interword glue in front of the inter-script glue — the exact
/// double-counting the pass exists to remove (`あります。 1 つは`).
#[test]
fn a_space_at_a_cjk_latin_seam_is_deleted_in_both_directions() {
    assert_eq!(
        shape("日本語 Alpha"),
        joined_seam_cjk_latin(),
        "CJK + SP + Latin: the 0.24em inter-script glue is the spacing, alone"
    );
    assert_eq!(
        shape("Alpha 日本語"),
        joined_seam_latin_cjk(),
        "Latin + SP + CJK: likewise"
    );
    // NON-VACUITY: the same space between two LATIN runs survives, so the
    // deletions above are the CJK rule firing and not the space simply never
    // reaching the box builder.
    assert!(
        shape("Alpha beta").contains(&interword()),
        "control: a Latin/Latin space is kept"
    );
    assert!(
        !shape("日本語 Alpha").contains(&interword()),
        "and a CJK/Latin one is not"
    );
}

/// `is_cjk` accepts `Script::Kana` as well as `Script::HanIdeographic`, and
/// every case above happens to put a Han character on at least one side of the
/// boundary — so deleting `Kana` from the predicate changed nothing any of them
/// could see. Kana-only Japanese is ordinary text (`ひらがな`, `カタカナ`), and
/// a newline inside it must be deleted like any other.
#[test]
fn kana_only_boundaries_are_cjk_too() {
    let joined = vec![
        run("あ"),
        cjk_pair(),
        run("い"),
        cjk_pair(),
        run("う"),
        cjk_pair(),
        run("え"),
    ];
    assert_eq!(
        shape("あい\nうえ"),
        joined,
        "kana on BOTH sides of the newline"
    );
    assert_eq!(shape("あいうえ"), joined, "the reference, pinned");

    // Katakana, and the two mixed orders — a Kana/Han seam must be deleted
    // whichever side the Han is on.
    assert_eq!(shape("アイ\nウエ"), shape("アイウエ"));
    assert_eq!(shape("あ\n日"), vec![run("あ"), cjk_pair(), run("日")]);
    assert_eq!(shape("日\nあ"), vec![run("日"), cjk_pair(), run("あ")]);

    // NON-VACUITY: a single space between two kana is KEPT, exactly as between
    // two Han characters — so the newline deletions above are the rule firing,
    // and kana is not simply being treated as whitespace-transparent.
    assert_eq!(
        shape("あい うえ"),
        vec![
            run("あ"),
            cjk_pair(),
            run("い"),
            interword(),
            run("う"),
            cjk_pair(),
            run("え")
        ],
        "one space between two kana survives, like `bispace` anywhere else"
    );
}

/// The `next_cjk` lookahead skips the WHOLE whitespace run before it asks what
/// follows (`rest.chars().find(|ch| !matches!(ch, ' ' | '\n'))`). Making it
/// read only the immediate next character instead leaves the FIRST member of a
/// run at a Latin/CJK seam looking like ordinary Latin whitespace, so it
/// survives and the seam gets an interword glue it must not have. Every
/// multi-space case in this file until now sat between two CJK runs, where the
/// surviving LAST space made the result look right anyway.
#[test]
fn a_whitespace_run_at_a_cjk_latin_seam_is_deleted_whole() {
    let latin_then_cjk = joined_seam_latin_cjk();
    assert_eq!(
        shape("Alpha  日本語"),
        latin_then_cjk,
        "two spaces: BOTH deleted, because the lookahead skips past them to 日"
    );
    assert_eq!(shape("Alpha   日本語"), latin_then_cjk, "three, likewise");
    assert_eq!(
        shape("Alpha \n 日本語"),
        latin_then_cjk,
        "and a wrapped one"
    );

    let cjk_then_latin = joined_seam_cjk_latin();
    assert_eq!(shape("日本語  Alpha"), cjk_then_latin, "and mirrored");
    assert_eq!(shape("日本語\n   Alpha"), cjk_then_latin);

    // NON-VACUITY: the identical run between two LATIN words survives, as the
    // one interword glue a whitespace run always becomes. So the seams above
    // lose their glue because of the CJK rule, not because a run of spaces
    // produces nothing anywhere.
    assert_eq!(
        shape("Alpha  beta"),
        vec![run("Alpha"), interword(), run("beta")]
    );
}

/// Leading and trailing whitespace next to a CJK run is deleted, so a source
/// line that begins or ends with an indent contributes nothing.
#[test]
fn whitespace_at_the_edges_of_a_cjk_run_is_deleted() {
    assert_eq!(shape("  日本語"), nihongo());
    assert_eq!(shape("日本語  "), nihongo());
    assert_eq!(shape("\n日本語\n"), nihongo());
}

/// The port's CJK predicate is a RANGE test (`rustyfi-backend`'s `char_script`,
/// `font.rs:81-97`), and its `0xFF00..=0xFFEF` row means the fullwidth Latin
/// forms `Ａ`/`Ｂ` (U+FF21/FF22) classify as `HanIdeographic` — deliberately,
/// so `「」。、` and the fullwidth forms render in the mincho face. That makes
/// them CJK for this pass too: a newline between two fullwidth letters is
/// deleted, and a single space between them is kept, exactly as for `日`. The
/// same two letters in their halfwidth spelling do neither.
#[test]
fn fullwidth_forms_count_as_cjk_for_this_pass() {
    assert_eq!(shape("Ａ\nＢ"), vec![run("Ａ"), cjk_pair(), run("Ｂ")]);
    assert_eq!(shape("ＡＢ"), vec![run("Ａ"), cjk_pair(), run("Ｂ")]);
    assert_eq!(
        shape("Ａ Ｂ"),
        vec![run("Ａ"), interword(), run("Ｂ")],
        "one space between two fullwidth letters is `bispace`, and kept"
    );
    // The halfwidth control: `A\nB` is one Latin word pair with ordinary
    // interword glue, so the case above is the fullwidth RANGE talking and not
    // something that would have happened to any two letters.
    assert_eq!(shape("A\nB"), vec![run("A"), interword(), run("B")]);
}
