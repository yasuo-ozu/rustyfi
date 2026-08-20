//! Slice X3a ("Slice X3 — forked-type export adapter (detailed
//! design)", specifically the X3a sub-slice, X3.1-X3.3): the boundary
//! TYPE adapter for a `V0_0` dependency spliced into a `V0_1` program.
//!
//! X2a already narrows the *value* half of X1's forked-name guard away (a
//! spliced binding's RHS runs inside `Ast::VersionScope(V0_0, _)`, so an
//! internally-used version-forked *primitive* resolves against its own
//! version). The *type* half stays conservative: `lib.rs`'s
//! `compile_document_v1_with_trials` still hard-rejects a dependency that
//! textually names ANY version-forked type (`typecheck::forked_type_names`)
//! anywhere in its `prelude`. X3a turns exactly ONE of those rejections into
//! an acceptance: `math` (0.0.6's math-text type) is representationally
//! IDENTICAL to 0.1's `math-text` — both lower to the same
//! `BaseType::MathText` (`typecheck.rs`'s `name_to_mono`), and both share the
//! same runtime `Value::MathText`/`Value::Math` representation
//! (`value.rs:39-56`, NOT forked between versions at all). So a `math`
//! reference can be safely RELABELED to `math-text` (the name a `V0_1`-typed
//! whole-program inference actually recognizes) with **zero runtime value
//! coercion** — the evaluated value crosses the boundary untouched. Every
//! OTHER forked type name (`math-text`/`math-boxes`/`pre-path`/`path`/
//! `graphics`/`image`/`deco`/`deco-set`/`font`/`paren`), plus `page` (X3.1's
//! note: `page`'s bare NAME lowers to the same nominal `Variant("page",[])`
//! under both versions, so it never appears in `forked_type_names()`'s
//! automatic name_to_mono diff — but its VALUE representation still forks
//! (0.0.6: a 9-ctor ADT, `Value::Ctor`; 0.1: a `length*length` tuple,
//! `Value::Product`), so it is added to the reject set here explicitly),
//! stays rejected — X3.8/S1's soundness backbone: no code path may ever hand
//! a `Value` of one shape to a site expecting another.
//!
//! **Where the type text that actually matters lives.** This port's 0.0.6
//! CST (`rustyfi_syntax::cst`) has NO type-ascription syntax on an ordinary
//! `let`/`let-inline`/`let-block`/`let-math` binding at all (`cst::TopLet`
//! etc. carry no `: ty` field) — a binding's type is 100% HM-inferred. Two
//! surface forms DO carry a `cst::ast::TypeExpr`:
//! `cst::ast::RecBinding::ascription` (a `let-rec`'s optional `: ty`) and
//! `cst::SigAnnot`'s `SigItem::Val*`/`DirectHorzCmd`/`DirectVertCmd` items (a
//! `module .. : sig .. end`'s declared types) — but BOTH are parsed and then
//! **entirely ignored** by `elaborate.rs` ("Parsed but not enforced" /
//! "accepted and ignored" — see that module's `TopBinding::Module`/
//! `RecBinding` handling): neither ever reaches `typecheck.rs`, so relabeling
//! them has zero effect on how the merged program actually typechecks.
//!
//! The ONE surface form that IS load-bearing is `cst::TopBinding::Type` (a
//! `type` declaration): `elaborate.rs`'s `lower_type_decl` clones its ctor
//! payload / synonym body `ast::TypeExpr`s VERBATIM into
//! `Program::type_decls`/`synonym_decls`, and `typecheck.rs`'s
//! `Checker::declare_variant`/`declare_synonym` — called ONCE per merged
//! program, under the single whole-program `Checker.version` (`V0_1` for
//! `compile_document_v1`'s pipeline, fixed by `v1::module_check::
//! check_program`'s session setup, NOT swapped by `Ast::VersionScope`, which
//! only wraps a binding's RHS *body*, never a `type` declaration) — lower
//! them via `name_to_mono(_, V0_1)`. A `type foo = A of math` spliced
//! unchanged would register `A`'s payload as the nominal
//! `Variant("math",[])` (0.1's "unbound type name" fallback,
//! `name_to_mono`'s doc comment) rather than `Base(MathText)`, so any
//! REAL math value later handed to `A` would fail to unify — a spurious
//! `TypeError`, not the transparent pass-through X3a promises. So the
//! splice-arm's realization (`lib.rs`) relabels exactly the
//! `TopBinding::Type` bodies (recursing through `TopBinding::Module`'s
//! nested `decls`, the only other place a `type` declaration can appear) —
//! see `relabel_type_decls` below. This is the textual-vs-boundary decision
//! X3a had latitude on (the design's own §X3.3 describes "for each exported
//! TopBinding with a type annotation" generically; this codebase's grammar
//! makes `TopBinding::Type` the ONLY site whose text is enforced, so that is
//! what gets rewritten — simplest AND sound, since leaving the decorative
//! `RecAscription`/`SigAnnot` sites un-rewritten has no behavioral effect).
//!
//! The ACCEPT/REJECT decision itself stays exactly where X1/X2a already
//! compute it (`lib.rs`'s `collect_free_globals` free-type scan, unchanged,
//! still conservatively over-approximating every textual site including the
//! decorative ones) — X3a only changes what happens once that scan says the
//! dependency's ENTIRE forked-type touch is exactly `{"math"}`.

use rustyfi_syntax::cst;
use rustyfi_syntax::cst::ast::{TypeApp, TypeAtom, TypeExpr, TypeProd};
use rustyfi_syntax::cst_v1;
use rustyfi_syntax::RustyfiVersion;

use crate::types::{CmdArgType, MonoType, PolyType, Row};
use crate::v1::surface::{self, SurfaceEnv};

/// The verdict for one crossing binding's boundary type (design doc X3.2).
#[derive(Debug, Clone, PartialEq)]
pub enum BoundaryError {
    /// A forked type name appears in the export signature and is not
    /// representationally shared across the boundary.
    ForkedTypeExport {
        /// The crossing binding/type this offending leaf was found under,
        /// e.g. `"Mod.bar"` or a bare type-declaration name — best-effort,
        /// may be empty when no such context is available (the standalone
        /// [`adapt_export_type`]/[`adapt_export_annotation`] helpers below
        /// don't take one; their callers can wrap/annotate further).
        binding: String,
        /// The offending leaf, e.g. `"page"` / `"deco"` / `"math-boxes"`.
        ty_name: String,
        /// The producer version (the dependency's own).
        from: RustyfiVersion,
        /// The consumer version (the splicing program's).
        to: RustyfiVersion,
        /// A human hint (why this particular name can't be relabeled).
        note: &'static str,
    },
}

impl std::fmt::Display for BoundaryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BoundaryError::ForkedTypeExport {
                binding,
                ty_name,
                from,
                to,
                note,
            } => {
                write!(
                    f,
                    "cross-version import (X3): {}exports a value whose type \
                     names `{ty_name}`, version-forked between {from:?} and \
                     {to:?} with no proven-identical runtime representation — \
                     {note}",
                    if binding.is_empty() {
                        String::new()
                    } else {
                        format!("`{binding}` ")
                    }
                )
            }
        }
    }
}

impl std::error::Error for BoundaryError {}

/// The set of type names X3a refuses to let cross the `V0_0` -> `V0_1`
/// boundary unadapted: every name [`crate::typecheck::forked_type_names`]
/// flags, plus `page` (X3.1's note — see this module's doc comment for why
/// `page` needs adding explicitly rather than falling out of the automatic
/// diff). `math` is deliberately NOT a member: it is the sole X3a
/// allow-and-relabel case, checked separately by every caller below.
pub fn reject_type_names() -> std::collections::BTreeSet<String> {
    let mut set = crate::typecheck::forked_type_names();
    set.insert("page".to_string());
    set
}

/// [`reject_type_names`] for a producer whose text was written as **0.0.6**,
/// i.e. the forward arm's spliced `V0_0` dependency: the shared set plus
/// `code`.
///
/// `code` is the third name whose fork the automatic
/// [`crate::typecheck::forked_type_names`] diff structurally cannot see, and
/// it is invisible for a third distinct reason:
///
/// - `math`/`deco`/… fork inside `name_to_mono`, so the diff finds them;
/// - `page` forks only in its VALUE representation (`Value::Ctor` vs
///   `Value::Product`) while its NAME lowers identically, so
///   [`reject_type_names`] adds it by hand;
/// - `code` forks one level ABOVE `name_to_mono`. It is not a bare type atom
///   at all — it is the constructor of a one-argument type APPLICATION, and
///   the gate lives in `typecheck::lower_type_app`'s `"code" if single
///   .is_some() && version.has_code_type_syntax()` arm. Under `V0_1` `int
///   code` is the real [`MonoType::Code`](crate::types::MonoType::Code);
///   under `V0_0` it stays the opaque nominal `Variant("code", [int])`,
///   because 0.0.6's manual-type decoder knows only `list` and `ref`
///   (upstream `v0.0.6 src/frontend/typeenv.ml:527-530`, against
///   `dev-0-1-0 src/frontend/manualTypeDecoder.ml:31-36` which adds `code`).
///   `name_to_mono` only ever lowers a bare atom, so no per-NAME diff of it
///   can ever report `code`.
///
/// Why that matters here and only here: a merged cross-version program has
/// ONE `Checker.version`, hard-coded to `V0_1`
/// (`v1::module_check::check_program_inner`), and a `TopBinding::Type`
/// declaration's body is registered under it — never inside an
/// `Ast::VersionScope` (this module's doc comment). So a 0.0.6 dependency
/// that WRITES `code` in a type declaration has that text re-read with 0.1's
/// vocabulary on the way in, and the dependency means something different
/// inside a 0.1 program than it does standalone — in both directions of harm:
/// a `XC (&(1))` upstream 0.0.6 refuses starts being accepted, and a package
/// that declares its own `type 'a code` starts failing to unify against it.
/// Refusing (loudly, `CompileError::CrossVersionUnsupportedName`) is the S1/S4
/// posture every other unbridgeable name here gets. Relabeling the leaf to a
/// private nominal would be the strictly-more-permissive upgrade if this ever
/// costs a real package; no bundled 0.0.6 package writes `code` in type
/// position today.
///
/// **Deliberately not in [`reject_type_names`] itself**, which both arms
/// share. This fork is a property of the PRODUCER's generation, not of the
/// crossing: a foreign **0.1** dependency's `code` is already written in the
/// merged program's own (hard-coded `V0_1`) vocabulary, reads correctly with
/// zero adaptation, and rejecting it would be a pure regression — pinned by
/// `xver_staging.rs`'s `a_zero_one_dependency_may_still_write_the_code_type`.
///
/// An INFERRED `code` export — the only kind a 0.0.6 package can otherwise
/// have, since `code τ` has no 0.0.6 spelling — writes no such text and is
/// untouched by this, correctly: `Value::Code { body, env }` is one struct
/// with no version field, and a quoted body's primitives freeze to the
/// generation it was written in at compile time (`compile.rs`'s `Ast::Next`
/// arm folds against `Compiler::current_version`), so the value crosses with
/// its meaning intact. See `xver_staging.rs` for both halves.
pub fn reject_type_names_from_v006() -> std::collections::BTreeSet<String> {
    let mut set = reject_type_names();
    set.insert("code".to_string());
    set
}

/// A human hint for why `name` (a member of [`reject_type_names`]) can't be
/// relabeled across the boundary — X3.1's classification table.
///
/// `pub(crate)` so `CompileError::CrossVersionUnsupportedName`'s `Display`
/// can append it (`lib.rs`). Without that the user-visible text for every
/// refusal was the same sentence — "a version-forked builtin, this slice
/// only supports the version-neutral subset" — which reads as "not
/// implemented yet" even for the names that are refused because the two
/// generations genuinely disagree about what the value IS.
pub(crate) fn forked_note(name: &str) -> &'static str {
    match name {
        "page" => {
            "0.0.6's page is a 9-ctor ADT (Value::Ctor); 0.1's is a length*length \
             tuple (Value::Product) — no shared runtime representation"
        }
        "math-boxes" => {
            "math-boxes is 0.1-only (the evaluated math tree); math must relabel to \
             math-text, never math-boxes (X3.8/S2) — no 0.0.6 value is ever a \
             math-boxes to begin with"
        }
        "math-text" => {
            "0.0.6 has no math-text primitive; a 0.0.6 package's OWN type named \
             math-text is an unrelated opaque user nominal, not a math value"
        }
        "deco" | "deco-set" => {
            "0.0.6 deco returns `graphics list`; 0.1 deco returns a single `graphics` \
             — the return shape differs, so crossing needs a value-level adapter. \
             X3b/X4b (classify_deco_exports/deco_coercion_prelude, and their reverse \
             twins) generate a POSITIONAL eta-expanding wrapper, which covers a \
             `deco`/`deco-set` TAIL after any number of MANDATORY arguments, at top \
             level or (nested) module scope; this particular occurrence is outside \
             that support — either an OPTIONAL-argument arrow (which has no positional \
             spelling to forward) or a `deco` leaf buried inside a compound type"
        }
        "paren" => {
            "0.0.6's paren is `length -> length -> length -> length -> color -> \
             (inline-boxes * (length -> length))` — (height, signed depth, axis, \
             fontsize, colour); 0.1's is `length -> length -> context -> \
             (inline-boxes * (length -> length))`, pulling the last three out of the \
             context instead. FORWARD that is a PROJECTION and X3b generates it: \
             `size = get-font-size ctx`, `axis = size *' \
             get-math-axis-height-ratio ctx`, `colour = get-text-color ctx`. \
             REVERSE there is no inverse to generate. The 0.0.6 call site \
             (`primitives::make_paren_run`) has only those five scalars and no \
             context at all, so a wrapper would have to invent one — and even \
             granting `set-font-size`/`set-text-color`, the caller's explicit AXIS \
             has no channel: 0.1 recovers the axis from the math font's MATH-table \
             height ratio, which no primitive in EITHER generation can set. A \
             reverse wrapper would therefore silently draw against the invented \
             context's axis rather than the caller's. This occurrence is either \
             that direction or outside the forward wrapper's support (an \
             OPEN optional row, or a `paren` leaf buried inside a compound type)"
        }
        // The build-out's own entry. This is the one place the port states,
        // in the user-visible error, that `font` is refused because the two
        // generations disagree about what a font VALUE is — not because
        // nobody has written the bridge yet.
        "font" => {
            "a REPRESENTATION FORK, not a missing feature. 0.1's `font` is an OPAQUE \
             HANDLE on one already-loaded face — upstream saphe-split registers \
             (\"font\", FontType) in types.cppo.ml's base_type_hash_table, spells it \
             tFONTKEY, and its only values are BCFontKey of FontKey.t, minted by a \
             font ENVELOPE from a font FILE path (envelopeChecker.ml's \
             check_font_envelope). 0.0.6 has NO `font` type at all: no such row in \
             its own base_type_hash_table and no `type font` in its bundled \
             packages, so the same word in 0.0.6 text is an unrelated opaque user \
             nominal. What 0.0.6 calls a font is the bare product `string * float * \
             float` (tFONT) whose head is an ABBREV naming a row of \
             dist/hash/fonts.satysfi-hash — a different naming universe, and it \
             names no forked type, so it already crosses as the string triple it is. \
             Neither direction has a total map: forward there is no 0.0.6 value that \
             is a face handle, and an untagged `string * float * float` cannot be \
             recognized as a font to coerce; reverse a handle is a store index with \
             no abbrev to recover from it"
        }
        "code" => {
            "0.0.6 has no `code` type spelling at all (its manual-type decoder knows \
             only `list` and `ref`), so `τ code` there is an opaque user nominal — \
             but a merged program reads every type declaration under one hard-coded \
             V0_1 Checker, where the same text means the real staged type. An \
             INFERRED `code` export (a `@stage: 0` binding's `&e`) is unaffected and \
             crosses fine; only WRITTEN `code` type text does not"
        }
        // Unreachable through `reject_type_names()` today: all four were
        // un-gated in `name_to_mono` once it turned out upstream registers
        // the same base type in BOTH generations, so the automatic
        // `forked_type_names()` diff no longer reports them. Kept because
        // `forked_note` is also reachable from the standalone
        // `adapt_export_type` walk, which takes any name.
        "pre-path" | "path" | "graphics" | "image" => {
            "0.0.6 has no such primitive; this name is an opaque user-nominal \
             stand-in there, with no shared representation against 0.1's real \
             primitive type"
        }
        _ => {
            "no proven-identical Value representation across the version boundary \
              (X3a's whitelist is `math` only)"
        }
    }
}

fn reject_if_forked(
    name: &str,
    from: RustyfiVersion,
    to: RustyfiVersion,
) -> Result<(), BoundaryError> {
    if name == "math" {
        return Ok(());
    }
    if reject_type_names().contains(name) {
        return Err(BoundaryError::ForkedTypeExport {
            binding: String::new(),
            ty_name: name.to_string(),
            from,
            to,
            note: forked_note(name),
        });
    }
    Ok(())
}

// ============================================================================
// `adapt_export_type` — the X3.2 spec function, operating on an already-
// inferred `PolyType` (a structural MonoType walk). This is the pure,
// directly-unit-testable statement of X3a's classification rule. It is NOT
// what the splice arm calls (X3.0(2): there is no standalone per-export
// `PolyType` object at the splice site — a 0.0.6 export's type only exists
// after whole-program inference); the splice arm instead calls
// `adapt_export_annotation`/`relabel_type_decls` below, on the SURFACE
// syntax, justified by the same rule this function states formally.
// ============================================================================

/// Adapt a producer-version export type into an equivalent consumer-version
/// type whose runtime `Value` representation is provably identical, or
/// reject.
///
/// `ty` is the export's type as it would be read under `from` (its `V0_0`
/// meaning). On `Ok`, the returned `PolyType` is what a `to`-version
/// consumer scope should bind for this export. On `Err`, the caller should
/// raise a compile error and abort.
///
/// X3a invariant: `adapt_export_type` only ever returns a type reachable
/// from `ty` by a **pure relabel with no value coercion** — any leaf that
// UNWIRED, and worth knowing why before relying on the doc comments below.
//
// These five functions are a `MonoType`-level cross-version boundary check.
// The live path does NOT use them: it is name-based, through
// `reject_type_names` / `relabel_or_reject_name`, driven by
// `typecheck::forked_type_names()`. So `adapt_export_type`'s claim to be "the
// soundness backbone" describes a design that is written but not in force.
//
// Kept because the structural check is strictly more precise than the
// name-based one (it can see a forked leaf nested inside a compound rather
// than matching a spelling) and is the obvious thing to wire when the boundary
// needs to tighten. Delete them if that is not the plan — dead code cannot
// make anything sound.
/// would require a runtime value adapter is an `Err`. This is the soundness
/// backbone (X3.6/S1). Because the sole accepted case (`math`) is already
/// `MathText` <-> `MathText` at the `MonoType` level (`types.rs` draws no
/// `math`/`math-text` distinction at all — see `BaseType::MathText`'s doc
/// comment), the `Ok` branch is simply `ty.clone()`: the "relabel" only ever
/// shows up at the surface-annotation level (`adapt_export_annotation`).
#[allow(dead_code)]
pub fn adapt_export_type(
    ty: &PolyType,
    from: RustyfiVersion,
    to: RustyfiVersion,
) -> Result<PolyType, BoundaryError> {
    check_mono_type(ty.body(), from, to)?;
    Ok(ty.clone())
}

#[allow(dead_code)]
fn check_mono_type(
    ty: &MonoType,
    from: RustyfiVersion,
    to: RustyfiVersion,
) -> Result<(), BoundaryError> {
    match ty {
        MonoType::Var(_) | MonoType::Base(_) => Ok(()),
        MonoType::Func(row, dom, cod) => {
            check_row(row, from, to)?;
            check_mono_type(dom, from, to)?;
            check_mono_type(cod, from, to)
        }
        MonoType::Product(items) => {
            for t in items {
                check_mono_type(t, from, to)?;
            }
            Ok(())
        }
        MonoType::List(t) | MonoType::Ref(t) | MonoType::Code(t) => check_mono_type(t, from, to),
        MonoType::Record(row) => check_row(row, from, to),
        MonoType::Variant(name, args) => {
            reject_if_forked(name, from, to)?;
            for t in args {
                check_mono_type(t, from, to)?;
            }
            Ok(())
        }
        MonoType::InlineCmd(items) | MonoType::BlockCmd(items) | MonoType::MathCmd(items) => {
            for c in items {
                check_cmd_arg(c, from, to)?;
            }
            Ok(())
        }
    }
}

#[allow(dead_code)]
fn check_row(row: &Row, from: RustyfiVersion, to: RustyfiVersion) -> Result<(), BoundaryError> {
    match row {
        Row::Empty | Row::Var(_) => Ok(()),
        Row::Cons(_, ty, rest) => {
            check_mono_type(ty, from, to)?;
            check_row(rest, from, to)
        }
    }
}

#[allow(dead_code)]
fn check_cmd_arg(
    c: &CmdArgType,
    from: RustyfiVersion,
    to: RustyfiVersion,
) -> Result<(), BoundaryError> {
    for (_, ty) in &c.opt_labels {
        check_mono_type(ty, from, to)?;
    }
    check_mono_type(&c.ty, from, to)
}

// ============================================================================
// `adapt_export_annotation` — the CST-level helper the splice arm actually
// exercises (X3.3): walks a SURFACE `cst::ast::TypeExpr`, rewriting every
// free `math` leaf to `math-text` and rejecting any other forked leaf
// (structural walk of `Fun`/`Atom`/`OptRowFun`/`TypeProd`/`TypeApp`/
// `TypeAtom`, mirroring `lib.rs`'s read-only `walk_type_expr` family used by
// `collect_free_globals`, but mutating instead of collecting).
// ============================================================================

/// Adapt one surface type annotation: clone `ann`, rewrite every `math` leaf
/// to `math-text`, and reject if any OTHER forked leaf (`reject_type_names`)
/// appears anywhere in the structure (S3: a forked leaf nested in a
/// compound rejects the WHOLE annotation, no partial acceptance).
#[allow(dead_code)]
pub fn adapt_export_annotation(
    ann: &TypeExpr,
    from: RustyfiVersion,
    to: RustyfiVersion,
) -> Result<TypeExpr, BoundaryError> {
    let mut out = ann.clone();
    relabel_type_expr(&mut out, from, to)?;
    Ok(out)
}

fn relabel_type_expr(
    te: &mut TypeExpr,
    from: RustyfiVersion,
    to: RustyfiVersion,
) -> Result<(), BoundaryError> {
    match te {
        TypeExpr::Fun { opts, dom, cod, .. } => {
            for o in opts.iter_mut() {
                relabel_type_prod(&mut o.ty, from, to)?;
            }
            relabel_type_prod(dom, from, to)?;
            relabel_type_expr(cod, from, to)
        }
        TypeExpr::Atom(prod) => relabel_type_prod(prod, from, to),
        TypeExpr::OptRowFun {
            opt_dom, dom, cod, ..
        } => {
            for e in opt_dom.entries.iter_mut() {
                relabel_type_expr(&mut e.ty.0, from, to)?;
            }
            relabel_type_prod(dom, from, to)?;
            relabel_type_expr(cod, from, to)
        }
    }
}

fn relabel_type_prod(
    tp: &mut TypeProd,
    from: RustyfiVersion,
    to: RustyfiVersion,
) -> Result<(), BoundaryError> {
    relabel_type_app(&mut tp.first, from, to)?;
    for st in tp.rest.iter_mut() {
        relabel_type_app(&mut st.ty, from, to)?;
    }
    Ok(())
}

fn relabel_type_app(
    ta: &mut TypeApp,
    from: RustyfiVersion,
    to: RustyfiVersion,
) -> Result<(), BoundaryError> {
    // Every atom of the application (arguments and the final constructor) is a
    // `TypeAtom`; `relabel_type_atom` already relabels a bare `Name` via
    // `relabel_or_reject_name` and passes a qualified `Mod.t` through, so
    // relabeling the whole run reproduces the old per-arg-then-ctor behavior.
    relabel_type_atom(&mut ta.head, from, to)?;
    for a in &mut ta.rest {
        relabel_type_atom(a, from, to)?;
    }
    Ok(())
}

fn relabel_type_atom(
    atom: &mut TypeAtom,
    from: RustyfiVersion,
    to: RustyfiVersion,
) -> Result<(), BoundaryError> {
    match atom {
        TypeAtom::Cmd { args, .. } => {
            for a in args.iter_mut() {
                for l in a.opt_labels.iter_mut() {
                    relabel_type_expr(&mut l.ty.0, from, to)?;
                }
                relabel_type_expr(&mut a.ty.0, from, to)?;
            }
            Ok(())
        }
        TypeAtom::Paren { inner, .. } => relabel_type_expr(&mut inner.0, from, to),
        TypeAtom::Record { fields, .. } => {
            for f in fields.iter_mut() {
                relabel_type_expr(&mut f.ty.0, from, to)?;
            }
            Ok(())
        }
        // A bound type variable — never a forked-name candidate.
        TypeAtom::Var(_) => Ok(()),
        TypeAtom::Name(n) => relabel_or_reject_name(&mut n.name, from, to),
        // `Mod.t` — already qualified, never one of the unqualified fork
        // names this policy governs.
        TypeAtom::NameMod(_) => Ok(()),
        TypeAtom::RecordOpen { inner, .. } => {
            for f in inner.fields.iter_mut() {
                relabel_type_expr(&mut f.ty.0, from, to)?;
            }
            Ok(())
        }
    }
}

/// The one leaf-level policy decision (X3.1), generalized by DIRECTION for
/// Slice X4a (item 5 — this function's `from`/`to` parameters were already
/// threaded through every caller; only the branching itself was hardcoded
/// to the one forward case before X4a).
///
/// **Where each direction is actually WIRED (load-bearing asymmetry).** The
/// FORWARD `(V0_0, V0_1)` arm below IS reached from `lib.rs`'s splice arm,
/// via `relabel_type_decls` — necessary there because
/// `v1::module_check::check_program_inner` hard-codes `Checker.version =
/// V0_1` for EVERY type declaration in the merged program (`module_check.rs`
/// :238-239,271), so a spliced 0.0.6 dependency's own "math" spelling MUST
/// be rewritten into ambient vocabulary before it reaches
/// `program.type_decls`. The REVERSE `(V0_1, V0_0)` arm below is NOT wired
/// into `lib.rs`'s reverse splice arm (`compile_document_v006_xver_with_
/// trials`): a foreign 0.1 dependency's own "math-text"/"math-boxes"
/// spelling is ALREADY that same hard-coded ambient (`V0_1`) vocabulary, so
/// relabeling it to 0.0.6's "math" would corrupt, not fix, it — that splice
/// arm instead uses this module's `reject_type_names()` as a pure WHITELIST
/// GUARD (accept-if-`{"math-text","math-boxes"}`-only, else reject; splice
/// VERBATIM either way) — see that function's own doc comment. The reverse
/// arm below stays correct and unit-tested (this module's `tests` — the
/// `adapt_export_annotation_reverse_*` group) as a direction-complete pure
/// utility, ready for a future caller (e.g. an X4b elaborated-IR-level
/// adapter) not bound by `check_program`'s hard-coded-`V0_1` constraint.
///
/// - **`V0_0` -> `V0_1`** (X3a, unchanged): `"math"` relabels in place to
///   `"math-text"` — 0.0.6's undifferentiated math type maps to 0.1's
///   UNEVALUATED half, the correct direction since a crossing 0.0.6 `math`
///   value is exactly a `Value::MathText`/`Value::Math` (never a
///   `Value::MathBoxes`, which 0.0.6 has no primitive that ever produces —
///   X3.8/S2's soundness backbone).
/// - **`V0_1` -> `V0_0`** (X4a, NEW): `"math-text"` OR `"math-boxes"`
///   relabels in place to `"math"` — 0.1's split math types both COARSEN
///   safely into 0.0.6's one undifferentiated type. Unlike the forward
///   direction, there is no symmetric "must never alias" risk here (X3.8/S2's
///   mirror does not apply): 0.0.6-authored code has no syntax that could
///   ever observe the lost `math-text`/`math-boxes` distinction — it never
///   had that distinction to begin with, so a `math-boxes` (the EVALUATED
///   tree, `Value::Math`) or a `math-text` (the UNPARSED source,
///   `Value::MathText`) crossing into 0.0.6-authored code both land as
///   exactly what a 0.0.6 `math` value already always was: EITHER shape,
///   `value.rs:39-56`'s shared representation. Re-derived, not assumed (task
///   brief's soundness bar): both `Value::MathText` and `Value::Math` are
///   ALREADY valid `math`-typed runtime values under 0.0.6 (0.0.6 code
///   pattern-matches/embeds a `math` value structurally, never on which of
///   the two variants produced it), so this is a pure, zero-coercion relabel
///   exactly like the forward case.
/// - Every other member of [`reject_type_names`], in EITHER direction, is a
///   hard `Err` (X4a's guard is deliberately conservative — see this
///   module's doc comment and X4b's future work for `page`/`graphics`/`deco`/
///   `deco-set`/`pre-path`/`path`/`image`/`font`/`paren`).
/// - Everything else (an ordinary user type name, or a shared builtin like
///   `int`/`string`) passes through untouched, in either direction.
fn relabel_or_reject_name(
    name: &mut String,
    from: RustyfiVersion,
    to: RustyfiVersion,
) -> Result<(), BoundaryError> {
    match (from, to) {
        (RustyfiVersion::V0_0, RustyfiVersion::V0_1) if name == "math" => {
            *name = "math-text".to_string();
            Ok(())
        }
        (RustyfiVersion::V0_1, RustyfiVersion::V0_0)
            if name == "math-text" || name == "math-boxes" =>
        {
            *name = "math".to_string();
            Ok(())
        }
        _ => reject_if_forked(name, from, to),
    }
}

// ============================================================================
// `relabel_type_decls` — the actual splice-arm entry point (`lib.rs`'s
// `compile_document_v1_with_trials`): rewrite every LOAD-BEARING type-text
// site in a spliced `V0_0` dependency's `prelude` (see this module's doc
// comment for why `TopBinding::Type`, recursed through `TopBinding::
// Module`'s nested `decls`, is the complete set of such sites in this
// port's 0.0.6 grammar).
// ============================================================================

/// Clone `prelude` and relabel every `math` leaf inside a `TopBinding::Type`
/// declaration's body (a variant's ctor payloads, or a synonym's body),
/// recursing into `TopBinding::Module`'s nested `decls` — the only other
/// place a `type` declaration can appear. Every other `TopBinding` variant
/// carries no type text this port's typechecker ever consults (see the
/// module doc comment), so it is returned unchanged.
///
/// Precondition (enforced by the caller, `lib.rs`): the dependency's ENTIRE
/// free-type touch (`collect_free_globals`, checked against
/// [`reject_type_names`]) is exactly `{"math"}` — so this walk can never
/// actually observe another forked name and return `Err` in practice; the
/// `Result` is kept for defense in depth / symmetry with
/// `adapt_export_annotation`, not because failure is expected here.
pub(crate) fn relabel_type_decls(
    prelude: &[cst::TopBinding],
    from: RustyfiVersion,
    to: RustyfiVersion,
) -> Result<Vec<cst::TopBinding>, BoundaryError> {
    prelude
        .iter()
        .cloned()
        .map(|tb| relabel_top_binding_types(tb, from, to))
        .collect()
}

fn relabel_top_binding_types(
    mut tb: cst::TopBinding,
    from: RustyfiVersion,
    to: RustyfiVersion,
) -> Result<cst::TopBinding, BoundaryError> {
    match &mut tb {
        cst::TopBinding::Type(td) => {
            relabel_type_decl_body(&mut td.body, from, to)?;
            for a in td.ands.iter_mut() {
                relabel_type_decl_body(&mut a.body, from, to)?;
            }
        }
        cst::TopBinding::Module { decls, .. } => {
            for d in decls.iter_mut() {
                let inner = (*d.0).clone();
                *d.0 = relabel_top_binding_types(inner, from, to)?;
            }
        }
        // `LetRec`/`Let`/`LetInline`/`LetBlock`/`LetMath`/`LetMutable`/
        // `Open` carry no `type`-declaration text (this module's doc
        // comment) — nothing to relabel.
        _ => {}
    }
    Ok(tb)
}

fn relabel_type_decl_body(
    body: &mut cst::TypeDeclBody,
    from: RustyfiVersion,
    to: RustyfiVersion,
) -> Result<(), BoundaryError> {
    match body {
        cst::TypeDeclBody::Variant { first, rest, .. } => {
            relabel_variant_def(first, from, to)?;
            for bv in rest.iter_mut() {
                relabel_variant_def(&mut bv.def, from, to)?;
            }
            Ok(())
        }
        cst::TypeDeclBody::Synonym(ty) => relabel_type_expr(ty, from, to),
    }
}

fn relabel_variant_def(
    vd: &mut cst::VariantDef,
    from: RustyfiVersion,
    to: RustyfiVersion,
) -> Result<(), BoundaryError> {
    if let Some(of_ty) = &mut vd.of_ty {
        relabel_type_expr(&mut of_ty.ty, from, to)?;
    }
    Ok(())
}

// ============================================================================
// X3b (design-cross-version-import.md's "Slice X3" §X3.5, deferred by X3a):
// a real value-level coercion for the `deco`/`deco-set` (b)-class case —
// "0.0.6 deco returns `graphics list`; 0.1 deco returns a single `graphics`".
//
// Unlike `math`, `deco`'s bare NAME needs no textual relabel at all: once
// accepted, `typecheck::name_to_mono("deco", V0_1)` already resolves it to
// `t_deco(V0_1)` unconditionally (`typecheck.rs`'s `name_to_mono` — gated on
// `version == V0_1`, with no analogous `V0_0` arm, so the SAME bare word
// means the SAME 0.1 arrow-type once the splice stops rejecting it). So a
// `deco`/`deco-set` mention with no VALUE attached (a bare `type foo = deco`
// synonym, exactly the fixture `xver_boundary_deco_export_coerces_and_renders`
// exercises) is *already* safe with zero further work — `relabel_type_decls`
// above is not even called for it.
//
// What genuinely needs adapting is the VALUE: a `V0_0`-authored `deco`
// closure's body, once fully applied, evaluates to `Value::List` of
// `Value::Graphics` (`prim_types::t_graphics_output`'s `!graphics_is_collection`
// arm; `primitives::coerce_graphics_result`'s `else` branch, which the
// compile-time `Ast::VersionScope` mechanism (X2) does NOT retroactively
// change — a closure's *body* just runs whatever it literally constructs).
// Every real `V0_1` call site that *applies* a `deco`
// (`primitives::apply_deco`, invoked by `lib.rs`'s `fire_inline_frame`/
// `fire_hooks` at render time — a genuine consumer, not a stand-in: see
// `prim_types::t_deco`'s doc comment for which primitives are stand-ins and
// which aren't) runs `coerce_graphics_result` under the AMBIENT
// `interp.version` at the time of that call — `V0_1` for any call reached
// from ordinary consumer code outside a `VersionScope` — which then expects
// a SINGLE `graphics`, not a list, and `as_graphics` on a `Value::List`
// fails. So a crossing `deco` value must be COERCED, not merely relabeled.
//
// **Scope.** X3b started at exactly ONE shape (a bare-leaf `: deco`/
// `: deco-set` ascription on a prelude-ROOT `TopBinding::LetRec`) and has
// since grown, one increment at a time, to every shape whose wrapper can be
// written as a POSITIONAL eta-expansion:
//
//   - arrow-PREFIXED (`length -> color -> color -> deco`) — the wrapper
//     forwards `lead_arity` extra parameters before `deco`'s own four;
//   - MODULE-scoped (a `module .. : sig .. end`'s `val` item, or a member's
//     own ascription inside the struct body) — a top-level `let
//     Deco.simple-frame` is not syntax, so the wrapper is appended INSIDE
//     the module's own `decls` (`inject_module_deco_wrappers`) where
//     ordinary sequential shadowing applies;
//   - NESTED-module scoped (a `module .. = struct module .. = struct ..`
//     chain), same mechanism one or more levels deeper — `DecoExport::
//     module_path` carries the whole chain and the injector matches on it.
//
// The one shape that still has NO sound wrap here, and REJECTS: an
// OPTIONAL-argument arrow anywhere in the export's type (`?(l = ty) -> ..`
// / a `TypeExpr::Fun` with a non-empty `opts`). The generated wrapper
// forwards its parameters POSITIONALLY and an optional argument has no
// positional spelling at all, so eta-expanding one would silently drop it —
// [`deco_tail_of`] returns `None` for that case on purpose, and the export
// falls through to the ordinary rejection path. Same for a `deco` leaf
// buried in a compound (a product/record/list member): the coercion is
// defined on the RESULT of a curried application, not on an arbitrary
// occurrence. `classify_deco_exports` REJECTS (rather than silently drops)
// every such occurrence, so the splice arm (`lib.rs`) fails loudly, matching
// every other unsupported-shape case in this file.
//
// One more narrowing specific to `deco-set`: its RecBinding's own
// `rb.params` must be EXACTLY one `PatBot::Unit` (`()`). `deco-set` is a
// plain 4-TUPLE, not a function (unlike `deco`) — but `elaborate.rs`
// REQUIRES a `let-rec` binding's RHS to be a function (a recursive VALUE
// binding, as opposed to a recursive FUNCTION binding, is rejected: "must
// be a function, got tuple"), so a bare `deco-set` export can ONLY ever be
// legally WRITTEN as `let-rec name : deco-set | () = (d0, d1, d2, d3)` —
// the `| ()` idiom this test suite's OTHER "plain value" fixtures also use
// (e.g. `xver_import.rs`'s `xver-get-page : page | () = A4Paper`) — never
// as a bare bodyless `| = ..` (which does not elaborate at all for a
// non-function RHS). `deco_coercion_prelude`'s wrap therefore applies the
// mandatory unit thunk (`{name} ()`) before destructuring the tuple. (`deco`
// itself needs no such params check: the wrap always APPLIES exactly 4
// fresh arguments through ordinary function application — matching `deco`'s
// own natural function shape directly, no unit thunk involved — and HM
// unification is structural, so a `rb.params` mismatch of any shape can
// only ever produce an ordinary `TypeError`, since a partially-applied
// curried-function type can never unify with `unite-graphics`'s expected
// `list graphics` argument type. `deco-set`'s `{name} ()`-then-`match` has
// no such automatic backstop against a WRONG params shape: applying `()`
// to, or tuple-matching, a value of some other shape is *also* an ordinary
// `TypeError`, but a needlessly confusing one — checking `rb.params`
// up front turns it into this module's clear `BoundaryError` instead.)
// ============================================================================

/// Which `deco`-family shape a [`DecoExport`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DecoKind {
    /// `deco = point -> length -> length -> length -> {list graphics /
    /// graphics}` — a 4-ary curried closure once fully applied.
    Deco,
    /// `deco-set = deco * deco * deco * deco` — a 4-tuple of `deco`s.
    DecoSet,
    /// A `paren` export (`math.satyh`'s `paren-left`/`brace-left`/… — 17 of
    /// them, all module-scoped, two arrow-tailed). 0.0.6 spells it `h -> d ->
    /// axis -> size -> color -> (inline-boxes, length -> length)`; 0.1 spells
    /// it `h -> d -> context -> ..` and has the closure pull fontsize, axis
    /// ratio and colour out of the context itself (`t_paren`,
    /// `primitives::make_paren_run`). The wrapper presents 0.1's interface and
    /// re-derives 0.0.6's three extra arguments from the context.
    Paren,
    /// Not a deco PRODUCER but a deco CONSUMER: an export that TAKES a
    /// `deco`/`deco-set` as an argument, e.g. `code.satyh`'s `val scheme :
    /// deco-set -> color -> context -> string -> block-boxes`. The coercion
    /// runs the other way — contravariantly. A 0.1 caller supplies a 0.1
    /// deco (returning one `graphics`), and the 0.0.6 callee will invoke it
    /// expecting a `graphics list`, so each such argument is DOWNGRADED by
    /// wrapping its result in a singleton list — the literal inverse of the
    /// `unite-graphics` upgrade. Which positions to downgrade is carried in
    /// [`DecoExport::arg_downgrades`].
    Consumer,
}

/// How one LEAD argument position of a [`DecoExport`] spells its optional
/// arguments, if any. The two generations model optionals so differently that
/// the generated wrapper has to reproduce each one in its own terms — see
/// [`deco_wrapper_src`] (0.0.6 → 0.1) and [`deco_downgrade_prelude`]
/// (0.1 → 0.0.6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LeadOpt {
    /// One plain mandatory argument, no optionals — every position of every
    /// export that predates optional-argument support.
    Mandatory,
    /// **0.0.6**, `ty ?->`: this position IS the optional slot. 0.0.6's
    /// optional arguments are POSITIONAL in this port — `lower_type_expr`
    /// turns each `ty ?->` domain into a mandatory `option ty ->` domain, and
    /// `elaborate.rs`'s `app_arg_to_ast` desugars a call site's `?:e`/`?*` to
    /// the plain `Some e`/`None` constructors — so a positional
    /// eta-expansion forwards one by VALUE with nothing lost.
    ///
    /// The one thing the wrapper must still reproduce is the `?:` MARKER on
    /// its own parameter: `elaborate.rs`'s `param_optional_shape` records it
    /// into [`Scope::optional_shape`], which is what lets a marker-less call
    /// site (`frame p w h d`, no `?*`) auto-omit the slot. A wrapper with
    /// plain parameters would shadow the export with one that has no recorded
    /// shape, silently breaking every marker-less call.
    V006Optional,
    /// **0.1**, `?(l : τ, …) dom ->`: this position takes one mandatory
    /// argument PLUS a LABELLED optional row (`MonoType::Func`'s [`Row`]),
    /// named here in declared order. Labelled optionals are not positional
    /// and cannot be forwarded by value: `Ast::LambdaOpt` binds the callee's
    /// binder at `τ option` while `Ast::ApplyOpt`'s `?(l = e)` takes the
    /// RAW `τ` and wraps it in `Some` itself, so there is no spelling that
    /// hands an already-`option` value to a labelled slot. The wrapper
    /// therefore CASE-SPLITS on each label's `option` and re-supplies exactly
    /// the labels that were present — see [`deco_downgrade_prelude`].
    V01Labels(Vec<String>),
}

/// The most optional LABELS (summed over every lead position) a 0.1 export
/// may declare and still cross. The reverse wrapper's case split is
/// exponential in this count (each label is independently present or absent
/// at the call that reaches it), so it is bounded rather than unbounded; four
/// labels is sixteen generated application sites, already far past anything
/// the bundled corpora declare on a single `deco`.
const V01_MAX_OPT_LABELS: usize = 4;

/// One binding a cross-version splice can soundly value-coerce: some number
/// of leading arguments (mandatory, or optional in whichever of the two
/// generations' spellings — see [`LeadOpt`]) followed by a
/// `deco`/`deco-set`/`paren` tail — see this section's doc comment for the
/// exact scope.
///
/// **Shared by both directions.** X3b classifies a `V0_0` dependency's
/// exports for a `V0_1` consumer (`classify_deco_exports`, off the 0.0.6
/// surface: a `RecBinding.ascription` or a `module .. : sig .. end` item);
/// X4b classifies a `V0_1` dependency's exports for a `V0_0` consumer
/// (`classify_deco_exports_v01_sig`, off the 0.1 `:>` sig — the ONE site
/// 0.1's grammar can name such a type at all). The `DecoExport` shape is the
/// same either way; only the generated wrapper differs — `unite-graphics`
/// (list -> single) forward, a singleton list (single -> list) in reverse.
#[derive(Debug, Clone)]
pub(crate) struct DecoExport {
    pub name: String,
    pub kind: DecoKind,
    /// How many arguments the export takes BEFORE its `deco` tail — the
    /// `3` in `simple-frame : length -> color -> color -> deco`. The
    /// generated wrapper eta-expands over exactly this many extra
    /// parameters, then over `deco`'s own four. `0` is the bare `: deco`
    /// case X3b originally supported.
    pub lead_arity: usize,
    /// One [`LeadOpt`] per lead position (so `lead_opts.len() == lead_arity`
    /// whenever it is non-empty). EMPTY is the shorthand for "every position
    /// is [`LeadOpt::Mandatory`]" — read it through [`DecoExport::lead_opt`],
    /// never by direct indexing — which leaves every construction site with
    /// no optionals to describe exactly as it was.
    pub lead_opts: Vec<LeadOpt>,
    /// The enclosing `module .. = struct .. end` chain, outermost first;
    /// empty for a top-level binding. A module-scoped export CANNOT be
    /// wrapped by a top-level shadowing binding — `let Deco.simple-frame`
    /// is not syntax — so its wrapper is appended INSIDE the module's own
    /// `decls` instead (`inject_module_deco_wrappers`), where ordinary
    /// sequential shadowing applies (`elaborate.rs`'s `walk_bindings`
    /// folds decls through a `running` scope, so a later decl shadows an
    /// earlier one of the same name).
    pub module_path: Vec<String>,
    /// For [`DecoKind::Consumer`]: one slot per leading argument, `Some(kind)`
    /// where that argument is a bare `deco`/`deco-set` needing the
    /// contravariant downgrade, `None` where it passes straight through.
    /// Empty for every producer.
    pub arg_downgrades: Vec<Option<DecoKind>>,
    /// [`DecoKind::DecoSet`] only: whether the ORIGINAL binding must be
    /// applied to a mandatory `()` thunk before its 4-tuple can be
    /// destructured. `true` for a `let-rec name : deco-set | () = (d0, ..)`
    /// export (`elaborate.rs` refuses a non-function `let-rec` RHS, so that
    /// spelling is the ONLY legal one for a bare, argument-less `deco-set`
    /// `let-rec`); `false` for a sig-declared member bound by an ordinary
    /// `let` to the bare tuple, and for every arrow-tailed `deco-set` (whose
    /// leading arguments are applied instead). Meaningless — and always
    /// `false` — for the other three kinds.
    pub unit_thunk: bool,
}

impl DecoExport {
    /// Lead position `i`'s optional spelling, defaulting to
    /// [`LeadOpt::Mandatory`] for an export whose `lead_opts` is the empty
    /// shorthand (see that field's doc comment).
    fn lead_opt(&self, i: usize) -> &LeadOpt {
        const MANDATORY: LeadOpt = LeadOpt::Mandatory;
        self.lead_opts.get(i).unwrap_or(&MANDATORY)
    }

    /// Whether ANY lead position carries optional arguments — i.e. whether
    /// the generated wrapper needs the optional-aware shape at all. `false`
    /// keeps every generator below on the byte-identical pre-optional path.
    fn has_optionals(&self) -> bool {
        self.lead_opts
            .iter()
            .any(|o| !matches!(o, LeadOpt::Mandatory))
    }

    /// The private name a wrapper binds the export's UNSHADOWED original
    /// under, when it cannot simply name the export itself.
    ///
    /// The forward wrapper normally re-applies the export by its own name and
    /// relies on ordinary shadowing. That stops working the moment the export
    /// has 0.0.6-style optionals: the ORIGINAL binding may carry a
    /// [`Scope::optional_shape`] entry (a `let frame ?:t p w h d = ..`
    /// implementing a sig's `length ?-> deco`), and `elaborate.rs`'s
    /// `app_chain_generic` then reads that shape at the wrapper's OWN call to
    /// it and synthesizes a `None` for the slot instead of consuming the
    /// wrapper's forwarded parameter — shifting every later argument by one.
    /// Binding the original to a fresh name first dodges that, but only if
    /// the alias does not INHERIT the shape, which `alias_optional_shape`
    /// makes it do for a bare `let x = y`; hence the parenthesised RHS in
    /// [`deco_wrapper_src`] (`head_optional_shape` reads a shape only off a
    /// bare `Var`/`VarWithMod` head).
    fn opt_src_alias(&self) -> String {
        format!("xver-opt-src-{}", self.dash_key())
    }

    /// The export's own fully-qualified key — `"M.frame"` for a module
    /// member, the bare `"frame"` for a top-level one. This is the name a
    /// consumer of EITHER generation writes, and the one X3c's schedule
    /// rebinds to whichever view that consumer should see.
    ///
    /// Deliberately NOT [`deco_export_qualified_name`], which is X4b's and
    /// assumes a non-empty `module_path` (it would spell a top-level export
    /// `".frame"`); the forward direction classifies top-level exports too.
    fn qualified_key(&self) -> String {
        if self.module_path.is_empty() {
            self.name.clone()
        } else {
            format!("{}.{}", self.module_path.join("."), self.name)
        }
    }

    /// `qualified_key` with `.` swapped for `-`, so it can be embedded in a
    /// surface identifier (a `.` cannot appear in one, a `-` can).
    fn dash_key(&self) -> String {
        let mut key: Vec<&str> = self.module_path.iter().map(String::as_str).collect();
        key.push(&self.name);
        key.join("-")
    }

    /// The name X3c binds the export's UNWRAPPED (0.0.6-shaped) original
    /// under, in the SAME scope the export itself lives in — a sibling
    /// `StructDecl` for a module member, a sibling top-level binding for a
    /// top-level export. Emitted BEFORE the wrapper, while the original is
    /// still the innermost binding of its own name.
    ///
    /// It has to live in that scope rather than at top level because a module
    /// member's original is not reachable from outside once the wrapper has
    /// shadowed it — and a 0.0.6 `sig .. end` seals nothing (`elaborate.rs`'s
    /// `TopBinding::Module` arm accepts `val` items and ignores them; only
    /// `direct` items bind anything), so an extra member is visible to every
    /// later consumer as `M.xver-fwd-orig-frame` with its own inferred type.
    fn orig_capture_name(&self) -> String {
        format!("xver-fwd-orig-{}", self.name)
    }

    /// [`orig_capture_name`](Self::orig_capture_name) qualified the same way
    /// the export itself is — the expression an X3c `Restore` rebinds the
    /// export's key to.
    fn orig_capture_key(&self) -> String {
        if self.module_path.is_empty() {
            self.orig_capture_name()
        } else {
            format!("{}.{}", self.module_path.join("."), self.orig_capture_name())
        }
    }

    /// The private TOP-LEVEL name X3c's `Capture` step binds the WRAPPED
    /// (0.1-shaped) view under, so a later `Install` can put it back without
    /// regenerating the wrapper. A pure function of the export's own key, so
    /// the separate `Capture`/`Install` calls agree on it.
    fn view_capture_name(&self) -> String {
        format!("xver-fwd-view-{}", self.dash_key())
    }
}

/// Scan a spliced `V0_0` dependency's `prelude` for every `deco`/
/// `deco-set` occurrence reachable from a `V0_1` consumer (the SAME
/// boundary sites `lib.rs`'s `collect_free_globals` already treats as
/// export text: a top-level `TopBinding::LetRec`'s own ascription, a
/// `TopBinding::Module`'s `sig` items, and — recursively — a module's own
/// `decls`), and classify each:
///
/// - a bare-leaf (or arrow-tailed) ascription on a `TopBinding::LetRec`,
///   whether top-level or nested inside a `module .. = struct .. end` →
///   sound to wrap, pushed onto the returned `Vec<DecoExport>`;
/// - a `TopBinding::Type` body (a synonym/ctor payload merely NAMING
///   `deco`/`deco-set`, no value attached) → SAFE, no coercion needed at
///   all (see this section's doc comment) — silently skipped (not even
///   visited: `classify_top_binding_deco`'s `_` arm);
/// - anything else that could carry a REAL `deco`/`deco-set`-typed VALUE
///   across the boundary (a `deco` leaf buried in a compound type, or an
///   OPTIONAL-argument arrow — see [`deco_tail_of`] for why the generated
///   positional wrapper cannot express one) → `Err` — X3b has no sound
///   wrap for these; the caller (`lib.rs`) rejects the WHOLE dependency,
///   exactly as X3a did before any `DecoExport` existed.
pub(crate) fn classify_deco_exports(
    prelude: &[cst::TopBinding],
    from: RustyfiVersion,
    to: RustyfiVersion,
) -> Result<Vec<DecoExport>, BoundaryError> {
    let mut out = Vec::new();
    let skip = std::collections::HashSet::new();
    for tb in prelude {
        classify_top_binding_deco(tb, &mut out, &[], &skip, from, to)?;
    }
    Ok(out)
}

/// `skip` names an ENCLOSING module's already-scheduled sig-item wrappers:
/// the `decls` walk must not schedule a SECOND wrapper for a member whose
/// `sig` item already produced one (that would wrap the wrap — two
/// `unite-graphics` layers). It is per-module-level: each nested
/// `TopBinding::Module` arm below computes its OWN set and passes that down,
/// never the parent's.
fn classify_top_binding_deco(
    tb: &cst::TopBinding,
    out: &mut Vec<DecoExport>,
    module_path: &[String],
    skip: &std::collections::HashSet<String>,
    from: RustyfiVersion,
    to: RustyfiVersion,
) -> Result<(), BoundaryError> {
    match tb {
        cst::TopBinding::LetRec { first, ands, .. } => {
            classify_rec_binding_deco(first, out, module_path, skip, from, to)?;
            for a in ands {
                classify_rec_binding_deco(&a.binding, out, module_path, skip, from, to)?;
            }
            Ok(())
        }
        cst::TopBinding::Module {
            name, sig, decls, ..
        } => {
            let mut inner = module_path.to_vec();
            inner.push(name.name.clone());
            // A module's SIG is the export surface: a `val x : .. -> deco`
            // item names a real value crossing the boundary, and (unlike the
            // top-level case) its `decls` counterpart may be an ordinary
            // `let`, so the sig is the only place the type is written. Wrap
            // what has a deco TAIL; reject anything that merely mentions
            // `deco` somewhere else in the type, which the positional
            // wrapper could not express.
            let mut wrapped: std::collections::HashSet<String> = std::collections::HashSet::new();
            if let Some(sig) = sig {
                for item in &sig.items {
                    if let Some(ty) = sig_item_value_ty(item) {
                        match (
                            sig_item_value_name(item),
                            deco_tail_of(ty),
                            deco_consumer_plan(ty),
                        ) {
                            (Some(n), Some((kind, lead_arity, lead_opts)), _) => {
                                wrapped.insert(n.to_string());
                                out.push(DecoExport {
                                    name: n.to_string(),
                                    kind,
                                    lead_arity,
                                    lead_opts,
                                    module_path: inner.clone(),
                                    arg_downgrades: Vec::new(),
                                    unit_thunk: false,
                                });
                            }
                            (Some(n), None, Some((plan, lead_opts))) => {
                                wrapped.insert(n.to_string());
                                out.push(DecoExport {
                                    name: n.to_string(),
                                    kind: DecoKind::Consumer,
                                    lead_arity: plan.len(),
                                    lead_opts,
                                    module_path: inner.clone(),
                                    arg_downgrades: plan,
                                    unit_thunk: false,
                                });
                            }
                            _ => reject_if_mentions_deco(ty, from, to)?,
                        }
                    }
                }
            }
            // RECURSE into the struct body (X4b's sibling nested-module
            // increment; this used to call a `reject_if_nested_value_
            // mentions_deco` walk that only ever rejected). A member's own
            // `: ty` ascription and a NESTED `module .. = struct .. end`'s
            // own `sig`/`decls` are classified exactly as this level's are,
            // just under a longer `module_path` — which is all
            // `inject_module_deco_wrappers` needs, since it already walks
            // nested modules and matches on the full path. `wrapped` is
            // passed as the skip set so a member already scheduled from
            // THIS module's `sig` is not wrapped a second time from its own
            // ascription (its ascription, if any, names the same type the
            // sig does).
            for d in decls {
                classify_top_binding_deco(&d.0, out, &inner, &wrapped, from, to)?;
            }
            Ok(())
        }
        // `Let`/`LetInline`/`LetBlock`/`LetMath`/`LetMutable`/`Open` carry no
        // `: ty` ascription this port's grammar could name `deco`/
        // `deco-set` in at all; `Type`'s body is the SAFE, no-value case
        // (this section's doc comment) — nothing to classify or reject.
        _ => Ok(()),
    }
}

fn classify_rec_binding_deco(
    rb: &cst::ast::RecBinding,
    out: &mut Vec<DecoExport>,
    module_path: &[String],
    skip: &std::collections::HashSet<String>,
    from: RustyfiVersion,
    to: RustyfiVersion,
) -> Result<(), BoundaryError> {
    let Some(asc) = &rb.ascription else {
        return Ok(());
    };
    if skip.contains(&rb.name.name) {
        // Already scheduled from the enclosing module's own `sig` item —
        // see `classify_top_binding_deco`'s `skip` doc comment.
        return Ok(());
    }
    // An arrow-PREFIXED deco (`length -> color -> color -> deco`, the shape
    // every real module export uses) is wrappable the same way a bare one
    // is — the wrapper just eta-expands over the leading arguments first.
    if let Some((kind, lead_arity, lead_opts)) = deco_tail_of(&asc.ty) {
        if lead_arity > 0 {
            out.push(DecoExport {
                name: rb.name.name.clone(),
                kind,
                lead_arity,
                lead_opts,
                module_path: module_path.to_vec(),
                arg_downgrades: Vec::new(),
                unit_thunk: false,
            });
            return Ok(());
        }
    }
    match type_expr_bare_name(&asc.ty) {
        Some("deco") => {
            out.push(DecoExport {
                name: rb.name.name.clone(),
                kind: DecoKind::Deco,
                lead_arity: 0,
                lead_opts: Vec::new(),
                module_path: module_path.to_vec(),
                arg_downgrades: Vec::new(),
                unit_thunk: false,
            });
            Ok(())
        }
        // `deco-set` (unlike `deco`) is NOT itself a function — its VALUE is
        // a bare 4-tuple. But `elaborate.rs` REQUIRES a `let-rec` binding's
        // RHS to be a function (recursive VALUE bindings, as opposed to
        // recursive FUNCTION bindings, are rejected — "must be a function,
        // got tuple"), so a `deco-set` export can ONLY legally be written
        // with the `| ()` idiom — `let-rec name : deco-set | () = (d0, d1,
        // d2, d3)` — the SAME idiom every other "plain value" `let-rec`
        // fixture in this test suite uses (e.g. `xver_import.rs`'s
        // `xver-get-page : page | () = A4Paper`). So `rb.params` must be
        // EXACTLY one `PatBot::Unit` — `deco_coercion_prelude`'s wrapper
        // applies `{name} ()` (the mandatory unit thunk) before matching the
        // tuple. Any OTHER params shape (empty, or a real destructuring
        // pattern) is outside X3b's scoped support.
        Some("deco-set") if matches!(rb.params.as_slice(), [cst::ast::PatBot::Unit { .. }]) => {
            out.push(DecoExport {
                name: rb.name.name.clone(),
                kind: DecoKind::DecoSet,
                lead_arity: 0,
                lead_opts: Vec::new(),
                module_path: module_path.to_vec(),
                arg_downgrades: Vec::new(),
                unit_thunk: true,
            });
            Ok(())
        }
        _ => reject_if_mentions_deco(&asc.ty, from, to),
    }
}

fn sig_item_value_ty(item: &cst::SigItem) -> Option<&TypeExpr> {
    use cst::SigItem;
    match item {
        SigItem::ValHorzCmd { ty, .. }
        | SigItem::ValVertCmd { ty, .. }
        | SigItem::Val { ty, .. }
        | SigItem::DirectHorzCmd { ty, .. }
        | SigItem::DirectVertCmd { ty, .. } => Some(ty),
        SigItem::Type { .. } => None,
    }
}

/// The value name a sig item declares, for the `Val` forms whose type can
/// carry a `deco` tail. Command items (`val \\cmd`/`val +cmd`, `direct`) are
/// deliberately excluded: a command's binder is not an ordinary identifier a
/// generated `let` could shadow.
fn sig_item_value_name(item: &cst::SigItem) -> Option<&str> {
    use cst::SigItem;
    match item {
        SigItem::Val { name, .. } => Some(name.name.as_str()),
        _ => None,
    }
}

/// `Some(name)` iff `te` is *exactly* one bare `TypeAtom::Name(name)` with
/// no `Fun` wrapper and no `TypeProd` continuation (`rest` empty) — the one
/// shape X3b's wrap knows how to handle with no currying-prefix arithmetic.
/// If `te` is a (possibly arrow-prefixed) `deco`/`deco-set`, return its kind,
/// how many arguments precede the tail, and each of those positions'
/// optional-argument spelling. `length -> color -> color -> deco` is `(Deco,
/// 3, [Mandatory; 3])`; a bare `deco` is `(Deco, 0, [])`.
///
/// An OPTIONAL-argument arrow (`ty ?-> ..`) is NOT a rejection here: 0.0.6's
/// optionals are positional in this port (`lower_type_expr` gives each `ty
/// ?->` domain the mandatory type `option ty ->`), so `config ?-> length ->
/// deco` contributes TWO lead positions — `[V006Optional, Mandatory]` — and
/// the wrapper forwards both by value. See [`LeadOpt::V006Optional`] for the
/// one thing that still has to be reproduced (the `?:` marker).
///
/// 0.1's LABELLED-optional arrow (`?(l : τ) dom -> ..`,
/// `TypeExpr::OptRowFun`) DOES return `None`. It cannot appear in genuine
/// 0.0.6 source at all — `typecheck::check_type_expr_v0_1_only` rejects the
/// node under `V0_0` with a version error — so a dependency this classifier
/// sees carrying one is 0.1-shaped text in a 0.0.6 file, not something this
/// direction's positional wrapper should be guessing at.
fn deco_tail_of(te: &TypeExpr) -> Option<(DecoKind, usize, Vec<LeadOpt>)> {
    let mut lead_opts: Vec<LeadOpt> = Vec::new();
    let mut cur = te;
    loop {
        match cur {
            TypeExpr::Fun { opts, cod, .. } => {
                for _ in opts {
                    lead_opts.push(LeadOpt::V006Optional);
                }
                lead_opts.push(LeadOpt::Mandatory);
                cur = cod;
            }
            TypeExpr::OptRowFun { .. } => return None,
            _ => {
                let kind = match type_expr_bare_name(cur)? {
                    "deco" => DecoKind::Deco,
                    "deco-set" => DecoKind::DecoSet,
                    "paren" => DecoKind::Paren,
                    _ => return None,
                };
                return Some((kind, lead_opts.len(), lead_opts));
            }
        }
    }
}

/// If `te` TAKES one or more bare `deco`/`deco-set` arguments and its result
/// mentions neither, return the per-argument downgrade plan plus each
/// position's optional-argument spelling. Anything subtler — a deco nested
/// inside a product/application, one in BOTH argument and result position, or
/// one behind a `ty ?->` (whose domain is `option deco`, a compound the
/// singleton-list downgrade is not defined on) — returns `None` and falls
/// through to rejection.
fn deco_consumer_plan(te: &TypeExpr) -> Option<(Vec<Option<DecoKind>>, Vec<LeadOpt>)> {
    let mut plan: Vec<Option<DecoKind>> = Vec::new();
    let mut lead_opts: Vec<LeadOpt> = Vec::new();
    let mut cur = te;
    loop {
        match cur {
            TypeExpr::Fun { opts, dom, cod, .. } => {
                for o in opts {
                    // `ty ?->` is its own positional `option ty` slot, and it
                    // passes straight through — but only if it is not itself
                    // deco-shaped, which no `option`-wrapped downgrade covers.
                    if type_prod_mentions_deco(&o.ty).is_some() {
                        return None;
                    }
                    plan.push(None);
                    lead_opts.push(LeadOpt::V006Optional);
                }
                let dom_te = TypeExpr::Atom(dom.clone());
                plan.push(match type_expr_bare_name(&dom_te) {
                    Some("deco") => Some(DecoKind::Deco),
                    Some("deco-set") => Some(DecoKind::DecoSet),
                    _ => {
                        if type_expr_mentions_deco(&dom_te).is_some() {
                            return None;
                        }
                        None
                    }
                });
                lead_opts.push(LeadOpt::Mandatory);
                cur = cod;
            }
            TypeExpr::OptRowFun { .. } => return None,
            _ => {
                if type_expr_mentions_deco(cur).is_some() {
                    return None;
                }
                return if plan.iter().any(Option::is_some) {
                    Some((plan, lead_opts))
                } else {
                    None
                };
            }
        }
    }
}

fn type_expr_bare_name(te: &TypeExpr) -> Option<&str> {
    match te {
        TypeExpr::Atom(TypeProd {
            first:
                TypeApp {
                    head: TypeAtom::Name(n),
                    rest: app_rest,
                },
            rest,
        }) if rest.is_empty() && app_rest.is_empty() => Some(n.name.as_str()),
        _ => None,
    }
}

fn reject_if_mentions_deco(
    te: &TypeExpr,
    from: RustyfiVersion,
    to: RustyfiVersion,
) -> Result<(), BoundaryError> {
    if let Some(name) = type_expr_mentions_deco(te) {
        return Err(BoundaryError::ForkedTypeExport {
            binding: String::new(),
            ty_name: name.clone(),
            from,
            to,
            note: forked_note(&name),
        });
    }
    Ok(())
}

/// Structural (read-only) walk mirroring `relabel_type_expr`'s traversal —
/// `Some(name)` for the first `"deco"`/`"deco-set"` leaf found anywhere in
/// `te`, `None` if there is none.
fn type_expr_mentions_deco(te: &TypeExpr) -> Option<String> {
    match te {
        TypeExpr::Fun { opts, dom, cod, .. } => opts
            .iter()
            .find_map(|o| type_prod_mentions_deco(&o.ty))
            .or_else(|| type_prod_mentions_deco(dom))
            .or_else(|| type_expr_mentions_deco(cod)),
        TypeExpr::Atom(prod) => type_prod_mentions_deco(prod),
        TypeExpr::OptRowFun {
            opt_dom, dom, cod, ..
        } => opt_dom
            .entries
            .iter()
            .find_map(|e| type_expr_mentions_deco(&e.ty.0))
            .or_else(|| type_prod_mentions_deco(dom))
            .or_else(|| type_expr_mentions_deco(cod)),
    }
}

fn type_prod_mentions_deco(tp: &TypeProd) -> Option<String> {
    type_app_mentions_deco(&tp.first)
        .or_else(|| tp.rest.iter().find_map(|st| type_app_mentions_deco(&st.ty)))
}

fn type_app_mentions_deco(ta: &TypeApp) -> Option<String> {
    // `type_atom_mentions_deco` checks a bare `Name` against `deco_leaf_name`
    // and ignores a qualified `Mod.t`, so scanning every atom (arguments and
    // the final constructor) reproduces the old arg-plus-ctor check.
    std::iter::once(&ta.head)
        .chain(ta.rest.iter())
        .find_map(type_atom_mentions_deco)
}

fn type_atom_mentions_deco(atom: &TypeAtom) -> Option<String> {
    match atom {
        TypeAtom::Cmd { args, .. } => args.iter().find_map(|a| {
            a.opt_labels
                .iter()
                .find_map(|l| type_expr_mentions_deco(&l.ty.0))
                .or_else(|| type_expr_mentions_deco(&a.ty.0))
        }),
        TypeAtom::Paren { inner, .. } => type_expr_mentions_deco(&inner.0),
        TypeAtom::Record { fields, .. } => {
            fields.iter().find_map(|f| type_expr_mentions_deco(&f.ty.0))
        }
        TypeAtom::Var(_) => None,
        TypeAtom::Name(n) => deco_leaf_name(&n.name),
        // `Mod.t` — a qualified name; never itself a bare builtin fork name.
        TypeAtom::NameMod(_) => None,
        TypeAtom::RecordOpen { inner, .. } => inner
            .fields
            .iter()
            .find_map(|f| type_expr_mentions_deco(&f.ty.0)),
    }
}

fn deco_leaf_name(name: &str) -> Option<String> {
    if name == "deco" || name == "deco-set" || name == "paren" {
        Some(name.to_string())
    } else {
        None
    }
}

/// Build synthetic, `V0_1`-authored `TopBinding`s that COERCE each
/// [`DecoExport`]'s already-spliced (`V0_0`-semantics, list-returning)
/// value into the single-`graphics` shape a `V0_1` consumer's call sites
/// expect (this section's doc comment). For `DecoKind::Deco`, splices a
/// SECOND top-level `let` of the SAME name that shadows the original
/// (ordinary non-recursive-`let` shadowing: its own body's reference to
/// that name resolves to the PREVIOUS, not-yet-shadowed binding — the
/// original `let-rec`), forwarding the deco's own 4 curried arguments
/// (point, length, length, length — `prim_types::t_deco`'s arity)
/// positionally and uniting the `graphics list` result via the real `V0_1`
/// `unite-graphics : list graphics -> graphics` primitive
/// (`primitives.rs`'s `prim_unite_graphics`). For `DecoKind::DecoSet`,
/// destructures the original 4-tuple and rewraps each component the same
/// way. Every identifier this generates is `xver-`-prefixed to avoid
/// colliding with the export's own (unknown-to-us) parameter names.
///
/// The generated source is parsed via [`rustyfi_syntax::parse_file`]
/// (mirroring this module's own test helpers, `prelude_of`/`parse_ty`) —
/// `panic!`s on a parse failure, since that would mean this function itself
/// generated malformed syntax (an internal bug), never a symptom of the
/// user's own dependency source.
/// The wrapper source for one export, as a struct/top-level `let`.
///
/// `lead_arity` extra parameters are forwarded positionally before `deco`'s
/// own four, so `simple-frame : length -> color -> color -> deco` becomes
/// `let simple-frame xver-a0 xver-a1 xver-a2 xver-p xver-w xver-h xver-d =
/// unite-graphics (simple-frame xver-a0 .. xver-d)`. Every generated
/// identifier is `xver-`-prefixed so it cannot capture one of the export's
/// own (unknown-to-us) parameter names.
///
/// [`DecoExport::unit_thunk`] distinguishes the two argument-less
/// `deco-set` spellings — see that field's own doc comment.
pub(crate) const XVER_UNITE_HELPER: &str = "xver-unite-graphics";
pub(crate) const XVER_AXIS_RATIO_HELPER: &str = "xver-math-axis-height-ratio";
pub(crate) const XVER_DOWN_DECO: &str = "xver-downgrade-deco";
pub(crate) const XVER_DOWN_DECOSET: &str = "xver-downgrade-decoset";

/// A `V0_1`-authored binding of [`XVER_UNITE_HELPER`], to be spliced BEFORE a
/// dependency whose module-scoped deco exports need wrapping.
///
/// An in-module wrapper cannot call `unite-graphics` itself. The whole module
/// is a spliced `V0_0` binding, so `elaborate.rs` wraps its members in
/// `Ast::VersionScope(V0_0, _)` — and under that scope `unite-graphics`, a
/// `V0_1`-only primitive, is an unbound variable at run time (observed, not
/// theorised). Top-level wrappers dodge this by being appended OUTSIDE the
/// dependency's index range; an in-module one has nowhere else to go.
///
/// So the `V0_1` primitive is captured once, outside any version scope, into
/// an ordinary user binding. Version scoping governs which `PrimDef` a
/// primitive NAME folds to; it does not change how a plain variable resolves,
/// so the scoped wrapper can call this helper and still get 0.1's
/// `unite-graphics`. Eta-expanded rather than bound bare so it goes through
/// the ordinary application path.
pub(crate) fn unite_helper_prelude() -> Vec<cst::TopBinding> {
    let src = format!(
        "let {XVER_UNITE_HELPER} xver-gs = unite-graphics xver-gs\n\
         let {XVER_AXIS_RATIO_HELPER} xver-c = get-math-axis-height-ratio xver-c\n\
         let {XVER_DOWN_DECO} xver-f xver-p xver-w xver-h xver-d =\n\
         \x20 [xver-f xver-p xver-w xver-h xver-d]\n\
         let {XVER_DOWN_DECOSET} xver-s =\n\
         \x20 match xver-s with\n\
         \x20 | (xver-s0, xver-s1, xver-s2, xver-s3) ->\n\
         \x20   ({XVER_DOWN_DECO} xver-s0, {XVER_DOWN_DECO} xver-s1,\n\
         \x20    {XVER_DOWN_DECO} xver-s2, {XVER_DOWN_DECO} xver-s3)\n"
    );
    rustyfi_syntax::parse_file(&src)
        .unwrap_or_else(|e| panic!("xver_adapt::unite_helper_prelude failed to parse: {e}"))
        .prelude
}

/// Whether any of `exports` needs [`unite_helper_prelude`] spliced — i.e. is
/// module-scoped, and so wrapped from INSIDE the dependency's version scope
/// where a `V0_1`-only primitive cannot be named directly.
pub(crate) fn needs_unite_helper(exports: &[DecoExport]) -> bool {
    exports
        .iter()
        .any(|e| !e.module_path.is_empty() || e.kind == DecoKind::Consumer)
}

fn deco_wrapper_src(exp: &DecoExport) -> String {
    // A top-level wrapper is spliced outside the dependency's version-scoped
    // range and can name the primitive directly; an in-module one is inside
    // it and must go through the pre-bound helper (see above).
    let unite = if exp.module_path.is_empty() {
        "unite-graphics"
    } else {
        XVER_UNITE_HELPER
    };
    let lead: Vec<String> = (0..exp.lead_arity).map(|i| format!("xver-a{i}")).collect();
    // The ARGUMENT spelling is always positional — a 0.0.6 optional argument
    // IS an `option`-typed positional slot in this port, so forwarding one by
    // value is exact (`LeadOpt::V006Optional`). Only the PARAMETER spelling
    // differs: a `?:` marker is reproduced so the wrapper records the same
    // `Scope::optional_shape` the export declared, keeping marker-less call
    // sites working.
    let lead_args = if lead.is_empty() {
        String::new()
    } else {
        format!("{} ", lead.join(" "))
    };
    let lead_params = if lead.is_empty() {
        String::new()
    } else {
        let marked: Vec<String> = (0..exp.lead_arity)
            .map(|i| match exp.lead_opt(i) {
                LeadOpt::V006Optional => format!("?:xver-a{i}"),
                _ => format!("xver-a{i}"),
            })
            .collect();
        format!("{} ", marked.join(" "))
    };
    // With no optionals the wrapper re-applies the export by its own name and
    // relies on ordinary shadowing (byte-identical to every pre-optional
    // wrapper). With optionals it must go through a private, shape-less alias
    // instead — see `DecoExport::opt_src_alias` for the marker-less-defaulting
    // trap that forces it, and note the PARENTHESISED right-hand side, which
    // is what stops `elaborate.rs`'s `alias_optional_shape` from copying the
    // original's shape straight back onto the alias.
    let alias = exp.opt_src_alias();
    let (orig, alias_binding) = if exp.has_optionals() {
        (
            alias.as_str(),
            format!("let {alias} = ({})\n", exp.name),
        )
    } else {
        (exp.name.as_str(), String::new())
    };
    // `get-font-size`/`get-text-color` exist under BOTH versions, so a scoped
    // wrapper may name them directly; the axis RATIO is V0_1-only and needs
    // the same pre-bound-helper treatment as `unite-graphics`.
    let axis_ratio = if exp.module_path.is_empty() {
        "get-math-axis-height-ratio"
    } else {
        XVER_AXIS_RATIO_HELPER
    };
    match exp.kind {
        // 0.1 hands the closure `(h, signed d, ctx)`; 0.0.6 wants
        // `(h, signed d, axis, size, color)`. Both versions pass the SAME
        // signed depth (`make_paren_run` negates before either call), so h and
        // d forward untouched. The three extra arguments come out of the
        // context: `axis = size *' ratio` reproduces `MathC::axis(size)`
        // exactly (`primitives.rs`: `axis(s) = s * axis_height`), and `size` is
        // the LOCAL script-scaled one because `make_paren_run` sets
        // `c2.font_size = size` before applying — the detail whose absence
        // would silently oversize every script-level delimiter.
        // Contravariant: forward every argument, downgrading the deco-typed
        // ones. `xver-downgrade-deco` wraps a 0.1 deco's single `graphics`
        // result in a singleton list, which is exactly what the 0.0.6 callee's
        // `as_list` expects — the inverse of the `unite-graphics` upgrade.
        DecoKind::Consumer => {
            let args: Vec<String> = exp
                .arg_downgrades
                .iter()
                .enumerate()
                .map(|(i, down)| match down {
                    Some(DecoKind::DecoSet) => format!("({XVER_DOWN_DECOSET} xver-a{i})"),
                    Some(_) => format!("({XVER_DOWN_DECO} xver-a{i})"),
                    None => format!("xver-a{i}"),
                })
                .collect();
            let params: Vec<String> = (0..exp.arg_downgrades.len())
                .map(|i| match exp.lead_opt(i) {
                    LeadOpt::V006Optional => format!("?:xver-a{i}"),
                    _ => format!("xver-a{i}"),
                })
                .collect();
            format!(
                "{alias_binding}let {name} {} =\n\x20 {orig} {}\n",
                params.join(" "),
                args.join(" "),
                name = exp.name
            )
        }
        DecoKind::Paren => format!(
            "{alias_binding}let {name} {lead_params}xver-h xver-d xver-ctx =\n\
             \x20 {orig} {lead_args}xver-h xver-d\n\
             \x20   ((get-font-size xver-ctx) *' ({axis_ratio} xver-ctx))\n\
             \x20   (get-font-size xver-ctx)\n\
             \x20   (get-text-color xver-ctx)\n",
            name = exp.name
        ),
        DecoKind::Deco => format!(
            "{alias_binding}let {name} {lead_params}xver-p xver-w xver-h xver-d =\n\
             \x20 {unite} ({orig} {lead_args}xver-p xver-w xver-h xver-d)\n",
            name = exp.name
        ),
        DecoKind::DecoSet => {
            // The original binding, applied to whatever it needs before its
            // 4-tuple is reachable: the mandatory `()` thunk of a bare
            // `let-rec name : deco-set | () = ..` (`unit_thunk`), or — for an
            // arrow-tailed `deco-set` — the same leading arguments the
            // wrapper itself just took.
            let scrutinee = if exp.unit_thunk {
                format!("{orig} ()")
            } else if lead.is_empty() {
                orig.to_string()
            } else {
                format!("{orig} {}", lead.join(" "))
            };
            let mut out = format!(
                "{alias_binding}let {name} {lead_params}=\n\
                 \x20 match {scrutinee} with\n\
                 \x20 | (xver-d0, xver-d1, xver-d2, xver-d3) ->\n",
                name = exp.name
            );
            let wrap = |i: usize| {
                format!(
                    "(fun xver-p xver-w xver-h xver-d -> \
                     {unite} (xver-d{i} xver-p xver-w xver-h xver-d))"
                )
            };
            out.push_str(&format!(
                "   ({}, {}, {}, {})\n",
                wrap(0),
                wrap(1),
                wrap(2),
                wrap(3)
            ));
            out
        }
    }
}

/// Append each module-scoped [`DecoExport`]'s wrapper INSIDE its own module,
/// as one more `StructDecl` after the export's original binding.
///
/// This is the half `deco_coercion_prelude` cannot do. A module member is
/// reached as `Deco.simple-frame`, and there is no syntax for a top-level
/// `let Deco.simple-frame = ..`, so the shadow has to live one scope deeper.
/// `elaborate.rs`'s `walk_bindings` folds a module's `decls` sequentially
/// through a `running` scope, so a later decl of the same name shadows the
/// earlier one and the module's export surface picks up the wrapper —
/// exactly the mechanism the top-level case already relies on.
///
/// The decls are built by parsing a synthetic `module .. = struct .. end`
/// and lifting its `decls`, so the wrapper text goes through the real parser
/// rather than being hand-constructed as CST.
pub(crate) fn inject_module_deco_wrappers(prelude: &mut [cst::TopBinding], exports: &[DecoExport]) {
    for tb in prelude.iter_mut() {
        inject_into_top_binding(tb, &[], exports);
    }
}

fn inject_into_top_binding(tb: &mut cst::TopBinding, path: &[String], exports: &[DecoExport]) {
    let cst::TopBinding::Module { name, decls, .. } = tb else {
        return;
    };
    let mut here = path.to_vec();
    here.push(name.name.clone());
    let mine: Vec<&DecoExport> = exports.iter().filter(|e| e.module_path == here).collect();
    if !mine.is_empty() {
        let mut src = String::from("module XverWrap = struct\n");
        for exp in &mine {
            // X3c: keep the UNWRAPPED original reachable under a private
            // sibling name before the wrapper shadows it — see
            // `DecoExport::orig_capture_name` for why it must live in this
            // scope, and `deco_upgrade_prelude`'s **Placement** section for
            // what reads it.
            src.push_str(&format!(
                "let {} = {}\n",
                exp.orig_capture_name(),
                exp.name
            ));
            src.push_str(&deco_wrapper_src(exp));
        }
        src.push_str("end\n");
        let file = rustyfi_syntax::parse_file(&src).unwrap_or_else(|e| {
            panic!(
                "xver_adapt::inject_module_deco_wrappers: internally-generated X3b \
                 wrapper source failed to parse (a bug in xver_adapt.rs, not user \
                 input): {e}\n--- generated source ---\n{src}"
            )
        });
        if let Some(cst::TopBinding::Module { decls: gen, .. }) = file.prelude.into_iter().next() {
            decls.extend(gen);
        }
    }
    for d in decls.iter_mut() {
        inject_into_top_binding(&mut d.0, &here, exports);
    }
}

pub(crate) fn deco_coercion_prelude(exports: &[DecoExport]) -> Vec<cst::TopBinding> {
    if exports.is_empty() {
        return Vec::new();
    }
    let mut src = String::new();
    for exp in exports {
        // Module-scoped exports are wrapped in place by
        // `inject_module_deco_wrappers`; a top-level shadow cannot name them.
        if !exp.module_path.is_empty() {
            continue;
        }
        // X3c: the unwrapped original, kept reachable under a private name
        // before the wrapper shadows it (see `inject_module_deco_wrappers`
        // for the module-scoped twin, and `deco_upgrade_prelude` for what
        // reads it).
        src.push_str(&format!(
            "let {} = {}\n",
            exp.orig_capture_name(),
            exp.name
        ));
        src.push_str(&deco_wrapper_src(exp));
    }
    if src.is_empty() {
        return Vec::new();
    }
    // Deliberately NO trailing dummy body: `File.body` is legitimately
    // `Option`-al (`cst.rs`'s doc comment — "Absent for a library file
    // (`nxtopsubseq`'s bare `EOI` case)"), and a bare literal like `0` is a
    // valid ATOM that a preceding `let`'s value expression's application
    // chain would happily keep consuming as one more argument (nothing
    // about a top-level decl boundary is whitespace-sensitive here — only a
    // following reserved keyword like `let`/`type`/`module` stops an
    // application chain). Parsing as a bare `prelude* EOI` library file
    // sidesteps that trap entirely; only `.prelude` is ever read below.
    let file = rustyfi_syntax::parse_file(&src).unwrap_or_else(|e| {
        panic!(
            "xver_adapt::deco_coercion_prelude: internally-generated X3b wrapper \
             source failed to parse (a bug in xver_adapt.rs, not user input): {e}\n\
             --- generated source ---\n{src}"
        )
    });
    file.prelude
}

// ============================================================================
// X3c — WHICH consumers see X3b's adapted view.
//
// X3b installs the 0.1-shaped view of a crossed `deco`/`deco-set`/`paren`
// export by SHADOWING the export's own name: a later `StructDecl` inside the
// exporting module (`inject_module_deco_wrappers`), or a later top-level
// binding (`deco_coercion_prelude`). Both are permanent — the merged prelude
// is one flat `Ast::LetIn` chain and `Ast::VersionScope(V0_0, _)` wraps a
// binding's RHS, never the continuation after it, so a shadow is visible to
// EVERYTHING that follows regardless of which generation authored it.
//
// That is right for the 0.1 entry and wrong for a later 0.0.6-AUTHORED
// dependency, which is elaborated in `Ast::VersionScope(V0_0, _)` and means
// 0.0.6's shape by every name it writes. Multi-package 0.0.6 corpora hit it
// constantly, because a package that exports a `deco`/`paren` is exactly the
// kind of package other packages build on:
//
//   - `math.satyh` declares `val paren-right : paren` and `latexcmds` applies
//     it with 0.0.6's five arguments — `type mismatch: expected `length`,
//     found `context``, unlocated, from a document that named neither file;
//   - the same for `deco`: an exporter's `graphics list` result is united into
//     one `graphics`, and the next 0.0.6 package's `inline-frame-outer` (typed
//     `t_deco(V0_0)` inside its own version scope) refuses it with `expected
//     `graphics list`, found `graphics``.
//
// It is the exact mirror of the defect X4b's placement schedule fixed in the
// other direction, and it takes the exact mirror of that fix: the adapted view
// becomes POSITION-INDEXED rather than permanent. What makes a
// position-indexed view sufficient is unchanged from X4b — each block
// `lib.rs`'s forward loop splices is homogeneous (a `V0_0` dependency's whole
// `prelude` goes into `v006_indices`, a `V0_1` dependency's whole `lowered`
// stays out of it), the entry is 0.1-authored and last, and the loader orders
// dependencies topologically so a consumer always follows what it
// `@require:`s.
//
// The three steps are [`UpgradeStep`]; both transitions are lazy, so a program
// whose 0.0.6 dependencies never consume each other's crossed exports (every
// pre-X3c fixture, and the single-dependency case) emits NOTHING and is
// byte-identical to before.
// ============================================================================

/// Which of X3c's three placement bindings [`deco_upgrade_prelude`] should
/// generate. The forward twin of [`DowngradeStep`], and deliberately its
/// mirror image: there the 0.1 view is the resting state and the 0.0.6 one is
/// installed at a transition, here the 0.1 view is what X3b has already
/// installed and the 0.0.6 one is what a transition has to put BACK.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UpgradeStep {
    /// `let xver-fwd-view-M-frame = M.frame` — capture X3b's WRAPPED view
    /// under a private name, so a later [`Install`](UpgradeStep::Install) can
    /// put it back without regenerating the wrapper. Binds a private name
    /// only, so it is invisible to consumers of either generation. Emitted
    /// once per export, at the first transition into 0.0.6-authored code —
    /// which is the last position at which naming the export's own key still
    /// yields the wrapped view.
    Capture,
    /// `let M.frame = M.xver-fwd-orig-frame` — put the UNWRAPPED, 0.0.6-shaped
    /// original back. Emitted on entering a 0.0.6-authored block. The
    /// right-hand side is the in-place capture X3b's two injectors emit next
    /// to the original itself (`DecoExport::orig_capture_name`).
    Restore,
    /// `let M.frame = xver-fwd-view-M-frame` — re-install the 0.1-shaped view
    /// from [`Capture`](UpgradeStep::Capture). Emitted on entering a
    /// 0.1-authored block (a foreign 0.1 dependency, or the entry) once the
    /// 0.0.6 view has been restored.
    Install,
}

/// Build the `V0_1`-authored placement bindings for `exports` — see
/// [`UpgradeStep`] for which of the three this call emits, and this section's
/// banner for why the view has to move at all.
///
/// [`Restore`](UpgradeStep::Restore) and [`Install`](UpgradeStep::Install)
/// both REBIND the export's own key, which for a module member is the dotted
/// `M.frame`. No surface syntax spells a top-level `let M.frame = ..`, but
/// `elaborate.rs`'s `push_named_binding` takes the binder name as an opaque
/// `String`, so — exactly as [`deco_downgrade_prelude`] does in the other
/// direction — the binding is parsed for its SHAPE under a private name whose
/// `BindName` is then replaced by the qualified key.
///
/// **Why `Restore`'s right-hand side is a MEMBER and `Install`'s is not.**
/// `Install` puts back a value that exists at top level the moment X3b's
/// wrapper has run, so a top-level capture reaches it. `Restore` puts back the
/// value the wrapper SHADOWED, which at top level no longer has a name at all
/// — hence the in-place `xver-fwd-orig-` sibling X3b's injectors now emit next
/// to the original. That sibling is reachable from outside because a 0.0.6
/// `module M : sig .. end = struct .. end` SEALS NOTHING in this port:
/// `elaborate.rs`'s `TopBinding::Module` arm accepts `val` items and ignores
/// them (only `direct` items bind anything), and `v1::module_check`'s
/// `static_env.seals` is built from the 0.1 `cst_v1` dependencies alone. So
/// the extra member neither changes the module's declared surface nor trips a
/// conformance check.
///
/// **Optional shapes.** A 0.0.6 export may carry `?:`-marked leading
/// parameters, and `elaborate.rs`'s `Scope::optional_shape` follows a bare
/// `let x = y` alias (`alias_optional_shape`). That is what these three want:
/// every binding here is a bare `Var`/`VarWithMod` alias, so the restored
/// original keeps the 0.0.6 shape its marker-less call sites need and the
/// re-installed wrapper keeps the shape it declared. (The wrapper's OWN
/// shape-less source alias is a separate, parenthesised binding —
/// [`DecoExport::opt_src_alias`].)
pub(crate) fn deco_upgrade_prelude(
    exports: &[DecoExport],
    step: UpgradeStep,
) -> Vec<cst::TopBinding> {
    if exports.is_empty() {
        return Vec::new();
    }
    let mut src = String::new();
    let mut shadow_names: Vec<(String, String)> = Vec::new();
    for exp in exports {
        let qualified = exp.qualified_key();
        let view = exp.view_capture_name();
        match step {
            UpgradeStep::Capture => {
                src.push_str(&format!("let {view} = {qualified}\n"));
            }
            UpgradeStep::Restore => {
                let shadow = format!("xver-fwd-shadow-{}", exp.dash_key());
                src.push_str(&format!("let {shadow} = {}\n", exp.orig_capture_key()));
                shadow_names.push((shadow, qualified));
            }
            UpgradeStep::Install => {
                let shadow = format!("xver-fwd-shadow-{}", exp.dash_key());
                src.push_str(&format!("let {shadow} = {view}\n"));
                shadow_names.push((shadow, qualified));
            }
        }
    }
    // Parsed as a bare `prelude* EOI` library file, for the same reason
    // `deco_coercion_prelude` is — see its comment about a trailing dummy body
    // being swallowed by the preceding application chain.
    let file = rustyfi_syntax::parse_file(&src).unwrap_or_else(|e| {
        panic!(
            "xver_adapt::deco_upgrade_prelude: internally-generated X3c placement \
             source failed to parse (a bug in xver_adapt.rs, not user input): {e}\n\
             --- generated source ---\n{src}"
        )
    });
    let mut prelude = file.prelude;
    rebind_shadows_to_qualified(&mut prelude, &shadow_names, "deco_upgrade_prelude");
    prelude
}

/// Replace each generated `let {shadow} = ..`'s binder with the dotted
/// qualified key it stands for. Shared by [`deco_upgrade_prelude`] (X3c) and
/// [`deco_downgrade_prelude`] (X4b): both need a top-level binding of a name
/// no surface syntax can spell, and both get it the same way — parse under a
/// private name, then swap the `BindName`, which `elaborate.rs`'s
/// `push_named_binding` treats as an opaque `String`.
fn rebind_shadows_to_qualified(
    prelude: &mut [cst::TopBinding],
    shadow_names: &[(String, String)],
    who: &str,
) {
    for (shadow, qualified) in shadow_names {
        let mut found = false;
        for tb in prelude.iter_mut() {
            if let cst::TopBinding::Let(tl) = tb {
                if tl.name.name == *shadow {
                    tl.name = cst::BindName::from(rustyfi_syntax::leaf::VarTok {
                        name: qualified.clone(),
                        span: tl.name.span,
                    });
                    found = true;
                    break;
                }
            }
        }
        assert!(
            found,
            "xver_adapt::{who}: generated shadow `{shadow}` vanished from its own \
             parse (a bug in xver_adapt.rs)"
        );
    }
}

// ============================================================================
// X4b (the reverse mirror of X3b, above) — a REAL crossing, no longer a
// negative finding: a foreign `V0_1` dependency's `deco`/`deco-set` export
// returns a single `graphics` (0.1 semantics); every REAL `V0_0`-authored
// consumer call site (`primitives::apply_deco`/`coerce_graphics_result`,
// fired at render time under `interp.version == V0_0` — `lib.rs`'s
// `compile_document_v006_xver_with_trials` always calls
// `eval_document_trials(.., RustyfiVersion::V0_0)`) expects a `graphics
// list` back (`coerce_graphics_result`'s `!graphics_is_collection()`
// branch, `as_list`). The coercion is the literal INVERSE of X3b's
// `unite-graphics` wrap: a SINGLETON LIST, `let name p w h d = [name p w h
// d]`. It is a type-level requirement too, not merely a runtime one — a
// 0.0.6-authored consumer is elaborated inside `Ast::VersionScope(V0_0, _)`,
// where `inline-frame-outer`/`inline-frame-breakable` carry `t_deco(V0_0)`/
// `t_decoset(V0_0)`, so an unwrapped 0.1 deco fails to unify long before
// eval.
//
// **Where the export's type is NAMED, and what that costs.** 0.1's grammar
// has NO bare top-level type-ascription syntax at all (`cst_v1::Bind::
// Value`/`ValueRec`'s own doc comments — no `: ty` on a plain `val`/`val
// rec`), so the ONLY textual site a 0.1 export's `deco`/`deco-set` type can
// ever be NAMED is a `module M :> sig val name : deco .. end = struct ..
// end` SIG item — which is exactly what `classify_deco_exports_v01_sig`
// (below) reads, off the PRE-lowering `cst_v1::FileV1` (lowering DROPS
// `sig_annot`). An UNANNOTATED 0.1 deco export is therefore invisible to
// this scan and does NOT cross; it fails with an ordinary `TypeError` at the
// consumer's call site, exactly as it did before X4b. That is a
// false-NEGATIVE (a refusal), never a false-accept.
//
// **The one obstacle, and how it is resolved.** Every 0.1 module signature
// annotation is the `:>` (COERCE) form — 0.1 has no other annotation keyword
// (`SigAnnotV1.coerce: CoerceTok`, unconditionally) — and
// `v1::module_check`'s phase-D spine walk conformance-checks EVERY such
// annotation for EVERY `Ast::LetIn` node whose name matches a
// `static_env.seals` entry. That check is PURELY NAME-KEYED, not "first
// occurrence only", so the coercion shadow (a second binding of the same
// qualified name, whose whole POINT is to have a DIFFERENT shape from the
// module's declared `deco`) trips it a second time no matter where it is
// spliced — verified empirically for both candidate positions (inside the
// module's own `decls`, and as a later top-level binding under the same
// dotted key; `elaborate.rs`'s `push_named_binding` treats a binder name as
// an opaque `String`, so the latter is expressible).
//
// The resolution is an EXPLICIT, caller-supplied exemption rather than a
// trick: `lib.rs`'s reverse arm passes the set of qualified names it is
// about to shadow to `v1::module_check::check_program_with_xver_shadows`,
// and that walk exempts only the SECOND-and-later `Ast::LetIn` of such a
// name (the FIRST — the module's own export alias — is still fully
// conformance-checked against the declared `deco`, unchanged). Nothing is
// left unchecked by that exemption: the shadow's own body is `[orig a.. p w
// h d]` where `orig` is bound to the module's SEALED scheme, so ordinary HM
// inference proves its type is exactly `t_deco(V0_0)` (with the same leading
// prefix). Rebinding a name to a version-adapted VIEW for the 0.0.6-authored
// code that follows is precisely what a cross-version bridge is; re-checking
// that derived binding against the exporter's own signature is a false
// positive of the name-keyed heuristic.
//
// **Placement — which consumers see the adapted view.** The merged prelude is
// ONE flat `Ast::LetIn` chain, and `Ast::VersionScope(V0_0, _)` is not a
// lexical scope for names (it wraps one binding's RHS, never the continuation
// after it), so a rebinding of `M.frame` is visible to everything that
// follows, whichever generation authored it. Splicing the shadow
// unconditionally right after its dependency therefore also handed the
// list-shaped view to a LATER *0.1* dependency consuming the same export,
// which then failed its own `:>` conformance check.
//
// The view is now SCHEDULED instead: captured once under a private name while
// the 0.1 view is in force, INSTALLED lazily on entering a 0.0.6-authored
// block, RESTORED on entering a 0.1-authored one. What makes a
// position-indexed view sufficient is that each block `lib.rs`'s reverse loop
// splices is homogeneous (wholly in `v006_indices` or wholly out of it), the
// entry is 0.0.6-authored and last, and the loader orders dependencies
// topologically so a consumer always follows what it `@require:`s. See
// [`DowngradeStep`]/[`deco_downgrade_prelude`]'s **Placement** section for the
// full derivation; both transitions are lazy, so a program with no
// interleaving emits exactly one install and no restore at all.
//
// A bare `type foo = deco` synonym (no value attached — safe with zero
// coercion, the same reasoning as the forward direction's `type
// xver-deco-alias = deco`) is unaffected: it is not a sig `val` item at all,
// so this scan never sees it, and the ordinary POST-lowering
// `collect_free_globals` scan (`lib.rs`) already lets it splice verbatim.
// ============================================================================

/// Classify every `deco`/`deco-set` VALUE export in a foreign `V0_1`
/// dependency's OWN top-level module signature, so `lib.rs`'s reverse splice
/// arm can generate a downgrade wrapper per export
/// ([`deco_downgrade_prelude`]); reject any `deco`-family mention this
/// positional wrapper cannot express. Operates on the dependency's file's
/// ORIGINAL, PRE-lowering `cst_v1::FileV1` — `v1::lower::
/// lower_file_v1_with_surfaces` DROPS a 0.1 module's `sig_annot` entirely
/// (`v1/lower.rs`'s own "sig_annot is then simply DROPPED" doc comment),
/// so this is the only stage at which the sig's text still exists; the
/// ordinary POST-lowering `collect_free_globals` scan (`lib.rs`) can never
/// see it.
///
/// Each returned [`DecoExport`] carries the exporting module chain as its
/// `module_path` — the top-level module's own name, plus one segment per
/// nested `Decl::Module` the scan descended through — so the generated shadow
/// can name the member the way a consumer does (`M.name`, `M.Inner.name`).
///
/// `Ok(vec![])` for a `FileV1::Document` (never a dependency), a `Library`
/// with no `sig_annot` at all, or one whose signature this scan cannot resolve
/// to a concrete decl list at all (see the "what still does not resolve" list
/// below) — a forked type hiding behind one of those is NOT a soundness gap
/// (X4.8/S2): HM still infers the crossing value's REAL shape at every use
/// site regardless of what this textual scan saw, so the worst case is an
/// ordinary `TypeError` far from its cause, never silent corruption.
///
/// A `deco` reached through a NESTED `module M : ..` decl DOES cross (the
/// forward direction's nested-module increment, mirrored here): the generated
/// shadow has to name the member under exactly the qualified key
/// `v1::module_check` seals it by, and that key is
/// `lower::qualify_type_key(mod_path, member)` — the SAME `mod_path`
/// `walk_nested_seals_a` composes by pushing each nested `Bind::Module`'s own
/// name onto its parent's (`module_check.rs`'s `child_path`), and the same
/// one `elaborate::push_named_binding` binds the member's export alias under.
/// So the recursion below simply pushes each nested `Decl::Module`'s name onto
/// `module_path` and classifies its sig's decls under it —
/// [`deco_export_qualified_name`] then spells `Outer.Inner.frame`, which is
/// exactly the `env.seals` key, the `Ast::LetIn` binder name, and the string a
/// 0.0.6 consumer's `Outer.Inner.frame` reference resolves through.
///
/// **NON-literal nested signatures resolve too.** The nested decl's signature
/// no longer has to be a literal `sig .. end`: [`v01_resolve_sig_decls`]
/// dereferences a named reference (`module M : S`, `module M : A.B.S`) and a
/// `with type` refinement through the very table `v1::module_check`'s own
/// `resolve_sig` consults — `surface::find_sig_keyed`, keyed and searched
/// OUTWARD from the same `site_path` (`module_check.rs`'s top-level seal and
/// its `handle_nested_module_decl` both pass the module's own path, which is
/// exactly this scan's `module_path`). `include` resolves too, and splices at
/// the ENCLOSING path rather than a lengthened one, mirroring
/// `module_check::splice_decls`. `signature S = ..` is SKIPPED rather than
/// rejected: a signature member declares no value at any path
/// (`handle_signature_decl` only identity-checks it against the struct's own
/// `signature` bind), so nothing a 0.0.6 consumer can name hides behind one —
/// but the definition it registers is exactly what a sibling `module M : S`
/// then resolves through.
///
/// **What still does not resolve, and why it is genuine.** Three shapes stay
/// unresolvable AT THIS POINT IN THE PIPELINE, and each keeps its precise
/// textual rejection ([`v1_reject_if_mentions_deco`], so a `deco` reachable
/// from one rejects rather than silently splicing):
///
/// - a FUNCTOR signature member (`module Make : (X : S1) -> S2`). A functor is
///   not a module: there is no member path `Outer.Make.frame` for a shadow to
///   rebind, and 0.0.6 has no syntax that could apply one. Its members become
///   reachable only through an APPLICATION (`module Inst = Outer.Make Arg`),
///   which `v1::functor` re-lowers at the APPLICATION's own path — in a
///   different file, possibly one this loop has not read yet. The path the
///   shadow would have to name is therefore not a function of this file's
///   signature at all;
/// - `S with M type t = τ` (a SUB-MODULE refinement). `module_check::
///   resolve_sig` rejects it outright as Sub-slice 2d-3b territory, so there
///   is no resolved decl list to scan even in principle;
/// - a named reference that does not resolve (unknown signature name, or an
///   `include`/`module` cycle through names). Both are hard, precise errors
///   from `module_check::resolve_sig` a moment later; this scan simply
///   declines to guess.
///
/// One PRE-EXISTING false negative is inherited rather than introduced: a
/// member declared at a type SYNONYM of `deco` (`sig type t = deco  val f : t
/// end`, or the same thing spelled `S with type t = deco`) reads as the bare
/// name `t`, not `deco`, so it neither classifies nor rejects. That is the
/// same X4.8/S2 shape as an unannotated export — an ordinary `TypeError` at
/// the consumer's call site, never silent corruption.
pub(crate) fn classify_deco_exports_v01_sig<'a>(
    file: &'a cst_v1::FileV1,
    surfaces: &SurfaceEnv<'a>,
) -> Result<Vec<DecoExport>, BoundaryError> {
    let cst_v1::FileV1::Library {
        name,
        sig_annot: Some(sig_annot),
        ..
    } = file
    else {
        return Ok(Vec::new());
    };
    let module_path = vec![name.name.clone()];
    let mut out = Vec::new();
    let mut visited: Vec<String> = Vec::new();
    classify_v01_sig_expr(
        &sig_annot.sig_.0,
        &module_path,
        surfaces,
        &mut visited,
        &V01Syns::default(),
        &[],
        &mut out,
    )?;
    Ok(out)
}

/// One signature EXPRESSION's value exports at `module_path` — resolve it to a
/// decl list first ([`v01_resolve_sig_decls`]), then classify that list.
///
/// `visited` is the named-signature cycle guard, keyed (like `module_check::
/// resolve_named_sig`'s own) by the RESOLVED table key rather than the written
/// suffix, so two differently-pathed same-suffix signatures do not
/// false-positive. A re-entry yields NO exports rather than an error: an
/// `include`/`module` cycle through names is `module_check::resolve_sig`'s own
/// precise diagnostic a moment later, and this scan never invents user-facing
/// text for it.
/// `inherited_refines` are an ENCLOSING layer's `with ⟨chain⟩ type`
/// refinements addressed to this signature (already stripped of the segments
/// that named the way here) — the same routing `module_check::
/// prescan_seal_types` performs, so what makes a member's type transparent
/// there makes it visible here.
fn classify_v01_sig_expr<'a>(
    se: &'a cst_v1::ast::SigExpr,
    module_path: &[String],
    surfaces: &SurfaceEnv<'a>,
    visited: &mut Vec<String>,
    syns: &V01Syns<'a>,
    inherited_refines: &[surface::Refine<'a>],
    out: &mut Vec<DecoExport>,
) -> Result<(), BoundaryError> {
    let Some(mut resolved) = v01_resolve_sig_decls(se, module_path, surfaces) else {
        // Genuinely unresolvable here (see [`classify_deco_exports_v01_sig`]'s
        // doc comment for the per-shape derivation) — guard textually, so a
        // `deco` reachable from one rejects rather than silently splicing.
        return v1_reject_if_mentions_deco(se, syns);
    };
    resolved.refines.extend(inherited_refines.iter().cloned());
    // This layer's OWN transparent type declarations (and any `with type`
    // refinement that made an opaque one transparent) extend the enclosing
    // signature's synonyms before a single `val` is classified — see
    // [`V01Syns`].
    let inner = syns.extended(resolved.decls, &resolved.refines, module_path, surfaces);
    let Some(k) = resolved.key else {
        // A LITERAL `sig .. end`: nesting is finite, no guard needed (the same
        // argument `module_check`'s own cycle guard rests on).
        return classify_v01_sig_decls(
            resolved.decls,
            module_path,
            surfaces,
            visited,
            &inner,
            &resolved.refines,
            out,
        );
    };
    if visited.contains(&k) {
        return Ok(());
    }
    visited.push(k);
    let r = classify_v01_sig_decls(
        resolved.decls,
        module_path,
        surfaces,
        visited,
        &inner,
        &resolved.refines,
        out,
    );
    visited.pop();
    r
}

/// One `sig .. end` body's decls, at the module path `module_path` — recursive
/// through `Decl::Module` (see [`classify_deco_exports_v01_sig`]'s doc comment
/// for why the composed path is exactly the seal key) and through
/// `Decl::Include` (at the SAME path, mirroring `module_check::splice_decls`).
fn classify_v01_sig_decls<'a>(
    decls: &'a [cst_v1::StructDeclV1],
    module_path: &[String],
    surfaces: &SurfaceEnv<'a>,
    visited: &mut Vec<String>,
    syns: &V01Syns<'a>,
    refines: &[surface::Refine<'a>],
    out: &mut Vec<DecoExport>,
) -> Result<(), BoundaryError> {
    for d in decls {
        match &*d.0 {
            cst_v1::ast::Decl::Val { name, ty, .. } => match v1_deco_tail_of(ty, syns) {
                Some((kind, lead_arity, lead_opts)) if kind != DecoKind::Paren => {
                    out.push(DecoExport {
                        name: name.name.clone(),
                        kind,
                        lead_arity,
                        lead_opts,
                        module_path: module_path.to_vec(),
                        arg_downgrades: Vec::new(),
                        unit_thunk: false,
                    })
                }
                // A `paren` export, or a `deco` this wrapper cannot express (a
                // leaf buried in a compound, or an optional-argument row this
                // direction's bounded case split will not enumerate — a row
                // VARIABLE tail, or more than `V01_MAX_OPT_LABELS` labels):
                // reject, loudly and specifically.
                _ => v1_reject_if_mentions_deco_ty(ty, syns)?,
            },
            cst_v1::ast::Decl::Module { name, sig_, .. } => {
                let mut inner = module_path.to_vec();
                inner.push(name.name.clone());
                // A `with N ⟨…⟩ type t = τ` refinement addressed to THIS
                // member descends into it with one segment consumed —
                // `prescan_seal_types`' own routing, reproduced.
                let child_refines: Vec<surface::Refine<'a>> = refines
                    .iter()
                    .filter(|r| r.path.first() == Some(&name.name))
                    .map(|r| {
                        let mut r = r.clone();
                        r.path.remove(0);
                        r
                    })
                    .collect();
                classify_v01_sig_expr(
                    sig_,
                    &inner,
                    surfaces,
                    visited,
                    syns,
                    &child_refines,
                    out,
                )?;
            }
            // `include S` splices S's OWN decls into the enclosing signature
            // in place, at the enclosing path — `module_check::splice_decls`,
            // so this layer's refinements apply to what it splices in.
            cst_v1::ast::Decl::Include { sig_, .. } => {
                classify_v01_sig_expr(sig_, module_path, surfaces, visited, syns, refines, out)?;
            }
            // `signature S = ..` declares a SIGNATURE, not a value: no member
            // of it is reachable at any path (`handle_signature_decl` only
            // identity-checks it against the struct's own `signature` bind, and
            // 0.0.6 has no signature syntax at all), so there is nothing here
            // to cross and nothing to refuse. `surface::build_file_surface`
            // has already registered the definition itself, which is what a
            // sibling `module M : S` resolves through.
            cst_v1::ast::Decl::Signature { .. } => {}
            other => {
                if let Some(n) = v1_decl_mentions_deco(other, syns) {
                    return Err(v1_boundary_error(&n));
                }
            }
        }
    }
    Ok(())
}

/// Resolve one signature expression to the decl list it denotes, plus the
/// `surfaces.sigs` table key it came from (`None` for a literal `sig .. end`,
/// which needs no cycle guard). `None` for the three genuinely unresolvable
/// shapes enumerated in [`classify_deco_exports_v01_sig`]'s doc comment.
///
/// Deliberately the SAME lookup `v1::module_check`'s `resolve_sig_bot`
/// performs — `surface::find_sig_keyed`, searched outward from `site_path` —
/// so a member found here sits at exactly the path `module_check` seals it
/// under, and the SAME `with type` refinement composition (an inline node's
/// own `binds`, plus a named signature's stored [`surface::SigDef::refines`])
/// — a refinement never changes a `val` decl's SPELLED type, but it DOES turn
/// an opaque `type t :: o` into the transparent synonym a `val` decl's
/// spelling may then name ([`V01Syns`]). What it deliberately does NOT
/// reproduce is `resolve_sig`'s eager `Decl::Include` flattening (this scan
/// recurses through `Decl::Include` in place instead, which is the same
/// traversal).
struct V01ResolvedSig<'a> {
    decls: &'a [cst_v1::StructDeclV1],
    /// The `surfaces.sigs` table key this came from — `None` for a literal
    /// `sig .. end`, which needs no cycle guard.
    key: Option<String>,
    refines: Vec<surface::Refine<'a>>,
}

fn v01_resolve_sig_decls<'a>(
    se: &'a cst_v1::ast::SigExpr,
    site_path: &[String],
    surfaces: &SurfaceEnv<'a>,
) -> Option<V01ResolvedSig<'a>> {
    use cst_v1::ast::SigExpr;
    match se {
        SigExpr::Bot(bot) => v01_resolve_sig_bot(bot, site_path, surfaces),
        SigExpr::WithType {
            base, path, binds, ..
        } => {
            let mut resolved = v01_resolve_sig_bot(base, site_path, surfaces)?;
            resolved
                .refines
                .extend(surface::collect_refines(binds, mod_chain_segments(path)));
            Some(resolved)
        }
        // A functor SIGNATURE — not a module signature; see the doc comment.
        SigExpr::Functor { .. } => None,
    }
}

fn v01_resolve_sig_bot<'a>(
    bot: &'a cst_v1::ast::SigBotV1,
    site_path: &[String],
    surfaces: &SurfaceEnv<'a>,
) -> Option<V01ResolvedSig<'a>> {
    use cst_v1::ast::SigBotV1;
    match bot {
        SigBotV1::Sig { decls, .. } => Some(V01ResolvedSig {
            decls: decls.as_slice(),
            key: None,
            refines: Vec::new(),
        }),
        SigBotV1::Var(t) => {
            surface::find_sig_keyed(surfaces, site_path, &t.name).map(|(key, def)| V01ResolvedSig {
                decls: def.decls,
                key: Some(key),
                refines: def.refines.clone(),
            })
        }
        SigBotV1::Path(t) => {
            let suffix = surface::sig_path_suffix(&t.mods, &t.name);
            surface::find_sig_keyed(surfaces, site_path, &suffix).map(|(key, def)| V01ResolvedSig {
                decls: def.decls,
                key: Some(key),
                refines: def.refines.clone(),
            })
        }
    }
}

/// The transparent type SYNONYMS a signature layer's `val` decls may name —
/// the whole of what makes
///
/// ```text
/// module M :> sig  type t = deco  val frame : length -> t  end = struct .. end
/// ```
///
/// cross. The scan reads a `val`'s SPELLED type, so without this the tail
/// reads as the bare name `t`, matches no forked builtin, and the export
/// silently declines to cross (surfacing much later as an ordinary
/// `TypeError` at a 0.0.6 consumer's call site rather than as this module's
/// own boundary diagnostic).
///
/// One entry per type name DECLARED by the signature layer being scanned, or
/// by any enclosing one (a nested `sig` sees its parent's type declarations,
/// and an `include`d signature's declarations splice into the includer's own
/// scope — so the map is threaded down, extended, never reset):
///
/// - `Some(body)` — a TRANSPARENT `type t = τ` with NO parameters, whose body
///   is kept whole and expanded IN PLACE at the tail
///   ([`v1_deco_tail_of`])/at a leaf ([`V01Syns::mentions_deco`]). Keeping the
///   body (rather than a pre-resolved verdict) is what makes an arrow-bodied
///   synonym — `type frame = length -> deco` — contribute its own lead
///   positions to the generated wrapper, exactly as if it had been spelled
///   out;
/// - `None` — a name this layer declares but that names no expandable
///   synonym: an OPAQUE `type t :: o`, a PARAMETERISED `type t 'a = ..` (a
///   bare `t` reference to which is ill-typed anyway), or a variant body.
///   Recorded rather than omitted so that it SHADOWS an enclosing layer's
///   entry — and so that a locally-declared name never falls through to the
///   builtin lookup below it.
///
/// A `with type t = τ` refinement (inline, or inherited from a named
/// signature's own stored refinements) is absorbed AFTER the decls, since its
/// whole job is to overwrite the `None` an opaque `type t :: o` just wrote.
///
/// Lookup is MAP-FIRST, builtin-second: a signature that declares its own
/// `type deco` shadows the builtin of that name for the layers below it, and
/// this scan must not then generate a coercion wrapper for a value that is
/// not a `deco` at all.
#[derive(Default, Clone)]
struct V01Syns<'a> {
    map: std::collections::HashMap<String, Option<&'a cst_v1::ast::TypeExpr>>,
}

/// What a bare type NAME denotes, as far as this scan can tell.
enum V01SynLookup<'a> {
    /// A transparent, zero-parameter synonym — expand its body in place.
    Body(&'a cst_v1::ast::TypeExpr),
    /// Declared by some enclosing signature layer, but not expandable (see
    /// [`V01Syns`]'s `None` case). Whatever it is, it is NOT the builtin of
    /// the same name.
    Opaque,
    /// Named by no signature layer in scope — a builtin (or an outright
    /// unknown, which is downstream's error, not this scan's).
    Undeclared,
}

impl<'a> V01Syns<'a> {
    fn lookup(&self, name: &str) -> V01SynLookup<'a> {
        match self.map.get(name) {
            Some(Some(body)) => V01SynLookup::Body(body),
            Some(None) => V01SynLookup::Opaque,
            None => V01SynLookup::Undeclared,
        }
    }

    /// This env extended with one signature layer's own type declarations
    /// (recursing through `include`, whose decls splice into the enclosing
    /// scope) and then its `with type` refinements.
    fn extended(
        &self,
        decls: &'a [cst_v1::StructDeclV1],
        refines: &[surface::Refine<'a>],
        site_path: &[String],
        surfaces: &SurfaceEnv<'a>,
    ) -> V01Syns<'a> {
        let mut out = self.clone();
        let mut visited: Vec<String> = Vec::new();
        out.absorb_decls(decls, site_path, surfaces, &mut visited);
        out.absorb_refines(refines);
        out
    }

    fn absorb_decls(
        &mut self,
        decls: &'a [cst_v1::StructDeclV1],
        site_path: &[String],
        surfaces: &SurfaceEnv<'a>,
        visited: &mut Vec<String>,
    ) {
        for d in decls {
            match &*d.0 {
                cst_v1::ast::Decl::Type { binds, .. } => {
                    for single in v01_flatten_type_binds(binds) {
                        self.map
                            .insert(single.name.name.clone(), v01_synonym_body(single));
                    }
                }
                cst_v1::ast::Decl::TypeOpaque { name, .. } => {
                    self.map.insert(name.name.clone(), None);
                }
                cst_v1::ast::Decl::Include { sig_, .. } => {
                    let Some(resolved) = v01_resolve_sig_decls(sig_, site_path, surfaces) else {
                        continue;
                    };
                    if let Some(k) = &resolved.key {
                        if visited.contains(k) {
                            continue;
                        }
                        visited.push(k.clone());
                        self.absorb_decls(resolved.decls, site_path, surfaces, visited);
                        self.absorb_refines(&resolved.refines);
                        visited.pop();
                    } else {
                        self.absorb_decls(resolved.decls, site_path, surfaces, visited);
                        self.absorb_refines(&resolved.refines);
                    }
                }
                _ => {}
            }
        }
    }

    /// A refinement that made an opaque declaration transparent overwrites
    /// the `None` that declaration just wrote. Only a refinement whose own
    /// `path` is EMPTY applies at this layer — `S with M type t = τ` refines
    /// the nested member `M`'s `t`, and reaches it as an empty-path
    /// refinement one layer down (`classify_v01_sig_decls`'s `Decl::Module`
    /// arm re-resolves `M`'s own signature, refinements and all).
    fn absorb_refines(&mut self, refines: &[surface::Refine<'a>]) {
        for r in refines {
            if !r.path.is_empty() {
                continue;
            }
            let body = match (r.tyvars.is_empty(), r.body) {
                (true, cst_v1::TypeBodyV1::Synonym(ty)) => Some(ty),
                _ => None,
            };
            self.map.insert(r.name.clone(), body);
        }
    }
}

/// A `type t = τ` bind's expandable body: `Some` only for a zero-parameter
/// SYNONYM (see [`V01Syns`]'s `None` case for why the rest are not).
fn v01_synonym_body(single: &cst_v1::TypeBindSingleV1) -> Option<&cst_v1::ast::TypeExpr> {
    match (single.tyvars.is_empty(), &single.body) {
        (true, cst_v1::TypeBodyV1::Synonym(ty)) => Some(ty),
        _ => None,
    }
}

/// `module_check::flatten_type_binds`' local twin (that one is private to its
/// own module, and this scan runs a whole phase earlier).
fn v01_flatten_type_binds(binds: &cst_v1::TypeBindsErasedV1) -> Vec<&cst_v1::TypeBindSingleV1> {
    let mut out = vec![&binds.0.first];
    for a in &binds.0.ands {
        out.push(&a.bind);
    }
    out
}

/// A `with M.N type ..` refinement's module chain, as path segments (empty
/// for the plain `with type ..` form).
fn mod_chain_segments(path: &Option<cst_v1::ast::ModChainV1>) -> Vec<String> {
    match path {
        None => Vec::new(),
        Some(cst_v1::ast::ModChainV1::Single(t)) => vec![t.name.clone()],
        Some(cst_v1::ast::ModChainV1::Long(t)) => {
            let mut segs = t.mods.clone();
            segs.push(t.name.clone());
            segs
        }
    }
}

fn v1_boundary_error(name: &str) -> BoundaryError {
    BoundaryError::ForkedTypeExport {
        binding: String::new(),
        ty_name: name.to_string(),
        from: RustyfiVersion::V0_1,
        to: RustyfiVersion::V0_0,
        note: forked_note(name),
    }
}

fn v1_reject_if_mentions_deco(
    se: &cst_v1::ast::SigExpr,
    syns: &V01Syns<'_>,
) -> Result<(), BoundaryError> {
    match v1_sigexpr_mentions_deco(se, syns) {
        Some(n) => Err(v1_boundary_error(&n)),
        None => Ok(()),
    }
}

fn v1_reject_if_mentions_deco_ty(
    ty: &cst_v1::ast::TypeExpr,
    syns: &V01Syns<'_>,
) -> Result<(), BoundaryError> {
    match v1_type_expr_mentions_deco(ty, syns) {
        Some(n) => Err(v1_boundary_error(&n)),
        None => Ok(()),
    }
}

/// The 0.1-grammar twin of [`deco_tail_of`]: if `te` is a (possibly
/// arrow-prefixed) `deco`/`deco-set`/`paren`, return its kind, how many
/// arguments precede the tail, and each of those positions' optional-argument
/// spelling.
///
/// A [`cst_v1::ast::TypeExpr::OptRowFun`] (0.1's `?(l : τ, …) dom -> ..`
/// LABELLED-optional arrow) contributes ONE lead position carrying
/// [`LeadOpt::V01Labels`] — the shadow case-splits on each label's `option`
/// rather than forwarding it (see [`deco_downgrade_prelude`]). Two shapes
/// still return `None`, and so still reject:
///
/// - a ROW-VARIABLE tail (`?(l : τ | ?'r) ->`): the label set is open, so
///   there is no finite case split to generate. (`v1/lower.rs` rejects the
///   tail with its own `LowerError` slightly later anyway — signature-level
///   row quantification is not implemented — but this scan runs PRE-lowering
///   and must not fall through to a wrapper it cannot write.)
/// - more than [`V01_MAX_OPT_LABELS`] labels in total: the case split is
///   exponential in the label count, and is bounded rather than unbounded.
///
/// A tail (or a whole type) spelled as a signature-declared type SYNONYM is
/// EXPANDED in place through `syns` ([`V01Syns`]) before any of the above is
/// decided, so `type t = deco  val f : length -> t` reads exactly as `val f :
/// length -> deco` does — including an arrow-bodied synonym, whose own lead
/// positions append to the ones already collected. A synonym cycle (`type t =
/// u  type u = t`, which a later phase rejects on its own terms) terminates
/// at the first repeat and declines, rather than looping.
fn v1_deco_tail_of<'a>(
    te: &'a cst_v1::ast::TypeExpr,
    syns: &V01Syns<'a>,
) -> Option<(DecoKind, usize, Vec<LeadOpt>)> {
    use cst_v1::ast::TypeExpr;
    let mut lead_opts: Vec<LeadOpt> = Vec::new();
    let mut labels = 0usize;
    let mut expanded: Vec<&str> = Vec::new();
    let mut cur = te;
    loop {
        match cur {
            TypeExpr::OptRowFun { opt_dom, cod, .. } => {
                if opt_dom.inner.row_tail.is_some() {
                    return None;
                }
                let here: Vec<String> = opt_dom
                    .inner
                    .entries
                    .iter()
                    .map(|e| e.label.name.clone())
                    .collect();
                labels += here.len();
                if labels > V01_MAX_OPT_LABELS {
                    return None;
                }
                lead_opts.push(LeadOpt::V01Labels(here));
                cur = cod;
            }
            TypeExpr::Fun { cod, .. } => {
                lead_opts.push(LeadOpt::Mandatory);
                cur = cod;
            }
            _ => {
                let name = v1_type_expr_bare_name(cur)?;
                match syns.lookup(name) {
                    V01SynLookup::Body(body) => {
                        if expanded.contains(&name) {
                            return None;
                        }
                        expanded.push(name);
                        cur = body;
                        continue;
                    }
                    // Declared, but naming no expandable synonym — whatever
                    // it is, it is NOT the builtin of the same name.
                    V01SynLookup::Opaque => return None,
                    V01SynLookup::Undeclared => {}
                }
                let kind = match name {
                    "deco" => DecoKind::Deco,
                    "deco-set" => DecoKind::DecoSet,
                    "paren" => DecoKind::Paren,
                    _ => return None,
                };
                return Some((kind, lead_opts.len(), lead_opts));
            }
        }
    }
}

/// The 0.1-grammar twin of [`type_expr_bare_name`]: `Some(name)` iff `te` is
/// *exactly* one bare `TypeAtom::Name` with no arrow wrapper, no product
/// continuation, and no type application.
fn v1_type_expr_bare_name(te: &cst_v1::ast::TypeExpr) -> Option<&str> {
    use cst_v1::ast::{TypeApp, TypeAtom, TypeExpr};
    let TypeExpr::Atom(prod) = te else {
        return None;
    };
    if !prod.rest.is_empty() {
        return None;
    }
    match &prod.first {
        TypeApp::Atom(TypeAtom::Name(n)) => Some(n.name.as_str()),
        _ => None,
    }
}

/// Which of X4b's three placement bindings [`deco_downgrade_prelude`] should
/// generate for a set of exports. The merged prelude is a single flat
/// `Ast::LetIn` chain, so "which view of `M.frame` is in force" is a function
/// of POSITION in that chain; these three steps are how `lib.rs`'s reverse arm
/// drives that position-indexed view (see [`deco_downgrade_prelude`]'s
/// **Placement** section for the schedule and its derivation).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DowngradeStep {
    /// `let xver-rev-orig-M-frame = M.frame` — capture the 0.1 original under
    /// a private name. Emitted ONCE per export, immediately after the
    /// dependency that defines it, while the 0.1 view is still the installed
    /// one. Binds a private name only: it never rebinds `M.frame`, so it is
    /// invisible to every consumer of either generation.
    Capture,
    /// `let M.frame = fun .. -> [xver-rev-orig-M-frame ..]` — install the
    /// 0.0.6-shaped view. Emitted at each transition INTO a 0.0.6-authored
    /// block (a native `V0_0` dependency, or the entry).
    Install,
    /// `let M.frame = xver-rev-orig-M-frame` — put the 0.1 view back. Emitted
    /// at each transition back into a 0.1-authored block, so a LATER 0.1
    /// dependency consuming the same export reads it at the shape its own
    /// `deco` means.
    Restore,
}

/// Build the `V0_1`-authored downgrade bindings for `exports` — see
/// [`DowngradeStep`] for which of the three this call emits. The load-bearing
/// one is [`DowngradeStep::Install`]: it REBINDS the export's own
/// fully-qualified name (`M.frame`) to a coerced view whose result is the
/// `graphics list` a `V0_0`-authored consumer expects.
///
/// Unlike the forward direction, the shadow CANNOT be appended inside the
/// exporting module's own `decls`: `elaborate.rs` emits one `Ast::LetIn`
/// export alias PER struct decl, so shadowing in place produces a FIRST
/// alias with the declared shape and a SECOND with the coerced one — and
/// `v1::module_check` conformance-checks BOTH, so the coerced one fails.
/// The shadow is therefore a TOP-LEVEL binding whose binder name is the
/// dotted qualified key directly. That is expressible even though no surface
/// syntax spells it: `elaborate.rs`'s `push_named_binding` takes the binder
/// name as an opaque `String`, so the generated `let xver-rev-shadow-N .. =
/// ..` is parsed for its SHAPE and its `BindName` then replaced with the
/// qualified key (the same "synthesize 0.0.6 CST out of already-parsed
/// pieces" move `v1/lower.rs` makes via `BindName: From<VarTok>`).
///
/// `Ast::Var` resolution follows suit for free: a consumer's `M.frame`
/// elaborates to a lookup of the string `"M.frame"` (`atomic`'s
/// `VarWithMod` arm via `qualify_key`), and `open M in ..`/`M.(..)` re-binds
/// the bare name to `Scope::resolve("M.frame")` — both land on whichever
/// binding of that key is innermost at that point, i.e. the shadow.
///
/// **Optional arguments ([`LeadOpt::V01Labels`]).** A 0.1 export may declare
/// LABELLED optionals (`?(thickness : length) length -> deco`), and the
/// shadow must present the same labelled interface or the export's optionals
/// vanish. It cannot do that by forwarding them, because 0.1's two halves
/// disagree about who owns the `option`:
///
/// - `Ast::LambdaOpt` (`typecheck.rs`'s `infer_lambda_opt`) binds the
///   receiving binder at `τ option` and leaves `Row::Cons(l, τ, …)` on the
///   arrow;
/// - `Ast::ApplyOpt` (`infer_apply_opt`) takes the RAW `τ` at `?(l = e)` and
///   `eval.rs`'s `push_opt_slots` wraps it in `Some` itself, filling every
///   label the call omits with `None`.
///
/// So `?(l = x)` where `x : τ option` is a type error, and there is no other
/// spelling — no surface form hands an already-`option` value to a labelled
/// slot. What the shadow CAN do is decide, per label, which of the two
/// spellings to use: `Some v` re-supplies it as `?(l = v)`, `None` omits the
/// label entirely and lets `push_opt_slots` restore the `None`. That is a
/// `match` per label, hence a case split with one application site per subset
/// of labels — bounded by [`V01_MAX_OPT_LABELS`]. The omitting branch relies
/// on plain `Ast::Apply` carrying an OPEN row var under `V0_1`
/// (`typecheck.rs`'s `Ast::Apply` arm), which absorbs the callee's whole
/// declared optional row; that is also what lets a 0.0.6-authored consumer
/// call the shadow with no optional syntax of its own.
///
/// **Placement — which consumers see the coerced view.** The merged prelude is
/// one flat `Ast::LetIn` chain, and `Ast::VersionScope(V0_0, _)` is NOT a
/// lexical scope for names (it wraps ONE binding's RHS, never the continuation
/// after it — `ast.rs`'s own doc comment), so a rebinding of `M.frame` is
/// visible to *everything* that follows it, whichever generation authored it.
/// Splicing the shadow unconditionally right after its dependency therefore
/// handed the 0.0.6-shaped view to a LATER 0.1 dependency too, which then
/// failed its own `:>` conformance check.
///
/// What makes a position-indexed view sufficient is that each splice unit is
/// homogeneous and correctly ordered: `lib.rs`'s reverse loop contributes one
/// CONTIGUOUS block per dependency, wholly 0.0.6-authored (a native `V0_0`
/// dependency's `prelude`, every index of it in `v006_indices`) or wholly
/// 0.1-authored (a foreign `V0_1` dependency's `lowered`, no index of it in
/// `v006_indices`), with the entry — always 0.0.6-authored — last; and the
/// loader orders dependencies topologically, so a consumer's block always
/// follows what it `@require:`s. So "which generation is reading `M.frame`
/// right now" is constant within a block and known at splice time, and the
/// schedule is exactly:
///
/// - [`Capture`](DowngradeStep::Capture) once, right after the defining
///   dependency (the 0.1 view is in force there — see the `Restore` below);
/// - [`Install`](DowngradeStep::Install) for EVERY export crossed so far, on
///   entering a 0.0.6-authored block, if the 0.0.6 view is not already
///   installed;
/// - [`Restore`](DowngradeStep::Restore) for every export crossed so far, on
///   entering a 0.1-authored block, if it is.
///
/// Both transitions are lazy, so a program with no interleaving (every 0.1
/// dependency, then the 0.0.6 entry — the common case, and every bundled
/// package) emits exactly one `Install` and no `Restore` at all: the same
/// bindings as the pre-schedule code, just positioned at the transition
/// instead of at each dependency.
///
/// `Install`/`Restore` both rebind a name `v1::module_check` has a `seals`
/// entry for, which is what `check_program_with_xver_shadows`'s exemption
/// covers — it exempts the SECOND-and-later `Ast::LetIn` of a listed name, so
/// a re-`Install` after a `Restore` is exempted for the same reason the first
/// one was, and the exporting module's OWN alias (the first) stays fully
/// conformance-checked.
pub(crate) fn deco_downgrade_prelude(
    exports: &[DecoExport],
    step: DowngradeStep,
) -> Vec<cst::TopBinding> {
    if exports.is_empty() {
        return Vec::new();
    }
    let mut src = String::new();
    let mut shadow_names: Vec<(String, String)> = Vec::new();
    for exp in exports.iter() {
        // `classify_deco_exports_v01_sig` never emits these two in this
        // direction (`paren` rejects; `Consumer` is a forward-only
        // contravariant case), so there is nothing to generate.
        if matches!(exp.kind, DecoKind::Paren | DecoKind::Consumer) {
            continue;
        }
        let qualified = deco_export_qualified_name(exp);
        // Mangled from the qualified key (a `.` cannot appear in a surface
        // identifier, a `-` can): the private names are a pure function of the
        // export's own qualified key, so [`DowngradeStep::Capture`]'s binding
        // and every later `Install`/`Restore` that reads it agree on the name
        // across the SEPARATE calls the placement schedule makes.
        let mangled = qualified.replace('.', "-");
        let orig = format!("xver-rev-orig-{mangled}");
        let shadow = format!("xver-rev-shadow-{mangled}");
        // The still-unshadowed original, captured under a private name. The
        // shadow's own body must NOT name `M.frame` — that is the key it is
        // about to rebind, and this indirection is what makes the rebinding
        // a plain (non-recursive) coercion rather than a self-reference. It is
        // ALSO what lets the 0.1 view be put back later: `Restore` simply
        // rebinds the key to this capture.
        if step == DowngradeStep::Capture {
            src.push_str(&format!("let {orig} = {qualified}\n"));
            continue;
        }
        if step == DowngradeStep::Restore {
            src.push_str(&format!("let {shadow} = {orig}\n"));
            shadow_names.push((shadow, qualified));
            continue;
        }
        let lead: Vec<String> = (0..exp.lead_arity).map(|k| format!("xver-a{k}")).collect();
        let lead_params = if lead.is_empty() {
            String::new()
        } else {
            format!("{} ", lead.join(" "))
        };
        // With labelled optionals the shadow needs `fun ?(l = x) p -> ..`
        // lambdas and a per-label case split (see this function's doc
        // comment); without them it keeps the plain parameter-list shape every
        // pre-optional shadow had, byte for byte.
        let lambdas = v01_shadow_lambdas(exp);
        let case_split = |tail: &str| {
            let slots = v01_opt_slots(exp);
            let mut chosen = vec![false; slots.len()];
            v01_opt_case_split(&slots, 0, &mut chosen, &|chosen| {
                format!("{orig} {}{tail}", v01_shadow_args(exp, &slots, chosen))
            })
        };
        match exp.kind {
            DecoKind::Deco if exp.has_optionals() => src.push_str(&format!(
                "let {shadow} = {lambdas}fun xver-p xver-w xver-h xver-d ->\n\
                 \x20 [{}]\n",
                case_split("xver-p xver-w xver-h xver-d")
            )),
            DecoKind::Deco => src.push_str(&format!(
                "let {shadow} {lead_params}xver-p xver-w xver-h xver-d =\n\
                 \x20 [{orig} {lead_params}xver-p xver-w xver-h xver-d]\n",
            )),
            DecoKind::DecoSet => {
                let scrutinee = if exp.has_optionals() {
                    case_split("")
                } else if lead.is_empty() {
                    orig.clone()
                } else {
                    format!("{orig} {}", lead.join(" "))
                };
                // `let {shadow} p0 p1 =` when every position is mandatory
                // (unchanged); `let {shadow} = fun ?(l = o) p0 -> ..` once a
                // labelled optional row has to be re-declared.
                let binder = if exp.has_optionals() {
                    format!("let {shadow} = {lambdas}")
                } else {
                    format!("let {shadow} {lead_params}= ")
                };
                let wrap = |k: usize| {
                    format!(
                        "(fun xver-p xver-w xver-h xver-d -> \
                         [xver-d{k} xver-p xver-w xver-h xver-d])"
                    )
                };
                src.push_str(&format!(
                    "{}\n\
                     \x20 match {scrutinee} with\n\
                     \x20 | (xver-d0, xver-d1, xver-d2, xver-d3) ->\n\
                     \x20   ({}, {}, {}, {})\n",
                    binder.trim_end(),
                    wrap(0),
                    wrap(1),
                    wrap(2),
                    wrap(3)
                ));
            }
            DecoKind::Paren | DecoKind::Consumer => unreachable!("skipped above"),
        }
        shadow_names.push((shadow, qualified));
    }
    if src.is_empty() {
        return Vec::new();
    }
    // Parsed as a bare `prelude* EOI` library file, for the same reason
    // `deco_coercion_prelude` is — see its comment about a trailing dummy
    // body being swallowed by the preceding application chain.
    let file = rustyfi_syntax::parse_file(&src).unwrap_or_else(|e| {
        panic!(
            "xver_adapt::deco_downgrade_prelude: internally-generated X4b wrapper \
             source failed to parse (a bug in xver_adapt.rs, not user input): {e}\n\
             --- generated source ---\n{src}"
        )
    });
    let mut prelude = file.prelude;
    rebind_shadows_to_qualified(&mut prelude, &shadow_names, "deco_downgrade_prelude");
    prelude
}

/// Every optional LABEL a 0.1 export declares, flattened to `(position,
/// index-within-position, label)`. The generated shadow binds each one's
/// `option` as `xver-o{position}-{index}` and, in the branch that re-supplies
/// it, its unwrapped payload as `xver-v{position}-{index}`.
fn v01_opt_slots(exp: &DecoExport) -> Vec<(usize, usize, String)> {
    let mut out = Vec::new();
    for i in 0..exp.lead_arity {
        if let LeadOpt::V01Labels(labels) = exp.lead_opt(i) {
            for (k, l) in labels.iter().enumerate() {
                out.push((i, k, l.clone()));
            }
        }
    }
    out
}

/// The shadow's parameter lambdas, one `fun .. ->` per lead position:
/// `fun ?(l = xver-o0-0) xver-a0 -> ` for a position with a labelled optional
/// row (`Expr::FunRows`, which elaborates to `Ast::LambdaOpt` and so puts the
/// same `Row::Cons(l, τ, …)` back on the shadow's own arrow), `fun xver-a0 ->
/// ` for a mandatory one. Empty when the export takes no leading arguments.
fn v01_shadow_lambdas(exp: &DecoExport) -> String {
    let mut out = String::new();
    for i in 0..exp.lead_arity {
        match exp.lead_opt(i) {
            LeadOpt::V01Labels(labels) => {
                let binders: Vec<String> = labels
                    .iter()
                    .enumerate()
                    .map(|(k, l)| format!("{l} = xver-o{i}-{k}"))
                    .collect();
                out.push_str(&format!("fun ?({}) xver-a{i} -> ", binders.join(", ")));
            }
            _ => out.push_str(&format!("fun xver-a{i} -> ")),
        }
    }
    out
}

/// The argument list of ONE leaf of the shadow's case split: every lead
/// position in order, each preceded by a `?(l = xver-v..)` bundle naming
/// exactly the labels `chosen` marks present at that position. A position
/// whose labels are all absent is spelled bare, so `Ast::Apply`'s open row
/// absorbs the callee's declared row and `push_opt_slots` restores the `None`s.
fn v01_shadow_args(exp: &DecoExport, slots: &[(usize, usize, String)], chosen: &[bool]) -> String {
    let mut out = String::new();
    for i in 0..exp.lead_arity {
        let here: Vec<String> = slots
            .iter()
            .zip(chosen)
            .filter(|((p, _, _), take)| *p == i && **take)
            .map(|((p, k, l), _)| format!("{l} = xver-v{p}-{k}"))
            .collect();
        if !here.is_empty() {
            out.push_str(&format!("?({}) ", here.join(", ")));
        }
        out.push_str(&format!("xver-a{i} "));
    }
    out
}

/// Expand `slots[idx..]` into nested `match .. with | None -> .. | Some(..) ->
/// ..` arms, calling `apply` at each of the `2^slots.len()` leaves with the
/// present/absent decision for every slot. Every generated `match` is
/// parenthesised, so nesting one inside an arm (and inside the list literal or
/// `match` scrutinee the caller wraps the whole thing in) is unambiguous.
fn v01_opt_case_split(
    slots: &[(usize, usize, String)],
    idx: usize,
    chosen: &mut Vec<bool>,
    apply: &dyn Fn(&[bool]) -> String,
) -> String {
    if idx == slots.len() {
        return apply(chosen);
    }
    let (p, k, _) = &slots[idx];
    chosen[idx] = false;
    let absent = v01_opt_case_split(slots, idx + 1, chosen, apply);
    chosen[idx] = true;
    let present = v01_opt_case_split(slots, idx + 1, chosen, apply);
    chosen[idx] = false;
    format!(
        "(match xver-o{p}-{k} with | None -> {absent} | Some(xver-v{p}-{k}) -> {present})"
    )
}

/// The qualified key [`deco_downgrade_prelude`] rebinds for `exp` — the same
/// string `v1::module_check` keys its `static_env.seals` entry by, and the
/// one `lib.rs` hands to `check_program_with_xver_shadows` as an exemption.
pub(crate) fn deco_export_qualified_name(exp: &DecoExport) -> String {
    format!("{}.{}", exp.module_path.join("."), exp.name)
}

/// Structural (read-only) walk of a 0.1 signature expression, looking for
/// any `deco`/`deco-set` mention anywhere reachable from it: a direct
/// inline `sig .. end` body's `val`/`val \cmd`/`val +cmd` items (recursing
/// into any nested `module`/`signature`/`include` declaration too), or a
/// `with type` refinement's base. A named-signature reference
/// (`SigBotV1::Path`/`Var`) is NOT chased further HERE — this is the
/// LAST-RESORT guard [`classify_v01_sig_expr`] falls back to once
/// [`v01_resolve_sig_decls`] has already declined to resolve the expression
/// at all (a functor signature, a `with M type` sub-module refinement, or a
/// name with no entry in `surfaces.sigs`); in the first two of those the
/// nested names it does reach are exactly the ones worth guarding, and in
/// the third there is nothing to chase. See
/// [`classify_deco_exports_v01_sig`]'s own doc comment for why a false
/// negative here is still sound (an ordinary `TypeError`, never unsoundness).
fn v1_sigexpr_mentions_deco(se: &cst_v1::ast::SigExpr, syns: &V01Syns<'_>) -> Option<String> {
    use cst_v1::ast::SigExpr;
    match se {
        SigExpr::Functor { dom, cod, .. } => v1_sigexpr_mentions_deco(dom, syns)
            .or_else(|| v1_sigexpr_mentions_deco(cod, syns)),
        SigExpr::WithType { base, .. } => v1_sigbot_mentions_deco(base, syns),
        SigExpr::Bot(bot) => v1_sigbot_mentions_deco(bot, syns),
    }
}

fn v1_sigbot_mentions_deco(bot: &cst_v1::ast::SigBotV1, syns: &V01Syns<'_>) -> Option<String> {
    use cst_v1::ast::SigBotV1;
    match bot {
        // An unresolved named-signature reference — not chased (this
        // section's doc comment).
        SigBotV1::Path(_) | SigBotV1::Var(_) => None,
        SigBotV1::Sig { decls, .. } => decls
            .iter()
            .find_map(|d| v1_decl_mentions_deco(&d.0, syns)),
    }
}

fn v1_decl_mentions_deco(decl: &cst_v1::ast::Decl, syns: &V01Syns<'_>) -> Option<String> {
    use cst_v1::ast::Decl;
    match decl {
        Decl::Val { ty, .. } | Decl::ValHorzCmd { ty, .. } | Decl::ValVertCmd { ty, .. } => {
            v1_type_expr_mentions_deco(ty, syns)
        }
        // A `type`/opaque-`type` sig item merely NAMES `deco`/`deco-set`
        // with no attached VALUE — safe, no coercion needed at all (this
        // section's doc comment's "bare `type foo = deco` synonym" case).
        Decl::TypeOpaque { .. } | Decl::Type { .. } => None,
        Decl::Module { sig_, .. } | Decl::Signature { sig_, .. } | Decl::Include { sig_, .. } => {
            v1_sigexpr_mentions_deco(sig_, syns)
        }
    }
}

/// Structural (read-only) walk of the WIDENED 0.1 type-expression grammar
/// (`cst_v1::ast::TypeExpr`) — the 0.1-grammar twin of this module's own
/// `type_expr_mentions_deco`, additionally covering 0.1-only shapes
/// (`OptRowFun`, prefix `TypeApp::Applied`/`AppliedLong`, the `inline
/// [..]`/`block [..]`/`math [..]` command-type forms). `Some(name)` for
/// the first `"deco"`/`"deco-set"` leaf found anywhere in `te`, `None` if
/// there is none.
fn v1_type_expr_mentions_deco(te: &cst_v1::ast::TypeExpr, syns: &V01Syns<'_>) -> Option<String> {
    use cst_v1::ast::TypeExpr;
    match te {
        TypeExpr::OptRowFun {
            opt_dom, dom, cod, ..
        } => opt_dom
            .inner
            .entries
            .iter()
            .find_map(|e| v1_type_expr_mentions_deco(&e.ty.0, syns))
            .or_else(|| v1_type_prod_mentions_deco(dom, syns))
            .or_else(|| v1_type_expr_mentions_deco(cod, syns)),
        TypeExpr::Fun { dom, cod, .. } => v1_type_prod_mentions_deco(dom, syns)
            .or_else(|| v1_type_expr_mentions_deco(cod, syns)),
        TypeExpr::Atom(prod) => v1_type_prod_mentions_deco(prod, syns),
    }
}

fn v1_type_prod_mentions_deco(tp: &cst_v1::ast::TypeProd, syns: &V01Syns<'_>) -> Option<String> {
    v1_type_app_mentions_deco(&tp.first, syns).or_else(|| {
        tp.rest
            .iter()
            .find_map(|st| v1_type_app_mentions_deco(&st.ty, syns))
    })
}

fn v1_type_app_mentions_deco(ta: &cst_v1::ast::TypeApp, syns: &V01Syns<'_>) -> Option<String> {
    use cst_v1::ast::TypeApp;
    match ta {
        // Prefix application (`list int`, 0.1-only shape): the CTOR itself
        // is the bare-name position here (unlike the universal postfix
        // grammar) — check it, plus every argument atom.
        TypeApp::Applied { ctor, first, rest } => v1_leaf_name_through_syns(&ctor.name, syns)
            .or_else(|| v1_type_atom_mentions_deco(first, syns))
            .or_else(|| {
                rest.iter()
                    .find_map(|a| v1_type_atom_mentions_deco(a, syns))
            }),
        // `M.t τ…` — a QUALIFIED ctor name; never itself a bare
        // `"deco"`/`"deco-set"` (those are unqualified builtins), only its
        // arguments can mention one.
        TypeApp::AppliedLong { first, rest, .. } => v1_type_atom_mentions_deco(first, syns)
            .or_else(|| {
                rest.iter()
                    .find_map(|a| v1_type_atom_mentions_deco(a, syns))
            }),
        TypeApp::InlineCmdTy { args, .. }
        | TypeApp::BlockCmdTy { args, .. }
        | TypeApp::MathCmdTy { args, .. } => args
            .iter()
            .find_map(|a| v1_type_cmd_arg_mentions_deco(a, syns)),
        TypeApp::Atom(atom) => v1_type_atom_mentions_deco(atom, syns),
    }
}

fn v1_type_cmd_arg_mentions_deco(
    item: &cst_v1::ast::TypeCmdArgItemV1,
    syns: &V01Syns<'_>,
) -> Option<String> {
    item.opts
        .as_ref()
        .and_then(|o| {
            o.entries
                .iter()
                .find_map(|e| v1_type_expr_mentions_deco(&e.ty.0, syns))
        })
        .or_else(|| v1_type_expr_mentions_deco(&item.ty.0, syns))
}

fn v1_type_atom_mentions_deco(atom: &cst_v1::ast::TypeAtom, syns: &V01Syns<'_>) -> Option<String> {
    use cst_v1::ast::TypeAtom;
    match atom {
        TypeAtom::Paren { inner, .. } => v1_type_expr_mentions_deco(&inner.0, syns),
        TypeAtom::Record { inner, .. } => inner
            .fields
            .iter()
            .find_map(|f| v1_type_expr_mentions_deco(&f.ty.0, syns)),
        // A bound type variable — never a forked-name candidate.
        TypeAtom::Var(_) => None,
        // `M.t` — a qualified name; never itself a bare builtin fork name.
        TypeAtom::LongName(_) => None,
        TypeAtom::Name(n) => v1_leaf_name_through_syns(&n.name, syns),
    }
}

/// [`deco_leaf_name`] THROUGH the signature's own type synonyms: a name a
/// signature layer in scope declares transparently is expanded (recursively,
/// cycle-guarded) and the expansion searched instead, so a `deco` reachable
/// only via a synonym — `type t = deco  val g : t list` — is refused as
/// loudly as a directly-spelled one, rather than quietly declining to cross.
/// A name declared NON-transparently (opaque, parameterised) is not a forked
/// builtin at all; a name declared nowhere falls through to the builtin test.
fn v1_leaf_name_through_syns(name: &str, syns: &V01Syns<'_>) -> Option<String> {
    v1_leaf_name_through_syns_guarded(name, syns, &mut Vec::new())
}

fn v1_leaf_name_through_syns_guarded(
    name: &str,
    syns: &V01Syns<'_>,
    expanded: &mut Vec<String>,
) -> Option<String> {
    match syns.lookup(name) {
        V01SynLookup::Body(body) => {
            if expanded.iter().any(|e| e == name) {
                return None;
            }
            expanded.push(name.to_string());
            // The body is searched with the SAME guard list, so a cycle
            // through any number of intermediate synonyms terminates.
            let out = v1_type_expr_mentions_deco_guarded(body, syns, expanded);
            expanded.pop();
            out
        }
        V01SynLookup::Opaque => None,
        V01SynLookup::Undeclared => deco_leaf_name(name),
    }
}

/// [`v1_type_expr_mentions_deco`] with an explicit synonym-expansion guard —
/// only reachable from [`v1_leaf_name_through_syns_guarded`], which is the
/// only place a cycle can arise. A synonym body is a plain type expression,
/// so this reuses the ordinary walk by temporarily hiding the names already
/// being expanded.
fn v1_type_expr_mentions_deco_guarded(
    te: &cst_v1::ast::TypeExpr,
    syns: &V01Syns<'_>,
    expanded: &mut Vec<String>,
) -> Option<String> {
    let mut hidden = syns.clone();
    for name in expanded.iter() {
        hidden.map.insert(name.clone(), None);
    }
    v1_type_expr_mentions_deco(te, &hidden)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::BaseType;

    fn v006() -> RustyfiVersion {
        RustyfiVersion::V0_0
    }
    fn v01() -> RustyfiVersion {
        RustyfiVersion::V0_1
    }

    // ------------------------------------------------------------------
    // adapt_export_type (PolyType-level spec function)
    // ------------------------------------------------------------------

    #[test]
    fn adapt_export_type_accepts_bare_math_text_base() {
        // `math`'s 0.0.6 meaning IS `Base(MathText)` (name_to_mono already
        // performed the math->MathText fold before this function ever sees
        // it) — the identity/no-op accept path.
        let ty = PolyType::mono(MonoType::Base(BaseType::MathText));
        let out = adapt_export_type(&ty, v006(), v01()).expect("math (MathText) must be accepted");
        assert!(matches!(out.body(), MonoType::Base(BaseType::MathText)));
    }

    #[test]
    fn adapt_export_type_accepts_math_nested_in_function_and_list() {
        let ty = PolyType::mono(MonoType::Func(
            Box::new(Row::Empty),
            Box::new(MonoType::List(Box::new(MonoType::Base(BaseType::MathText)))),
            Box::new(MonoType::Base(BaseType::MathText)),
        ));
        assert!(adapt_export_type(&ty, v006(), v01()).is_ok());
    }

    #[test]
    fn adapt_export_type_rejects_page_nominal() {
        let ty = PolyType::mono(MonoType::Variant("page".to_string(), vec![]));
        let err = adapt_export_type(&ty, v006(), v01()).expect_err("page must reject");
        match err {
            BoundaryError::ForkedTypeExport { ty_name, .. } => assert_eq!(ty_name, "page"),
        }
    }

    #[test]
    fn adapt_export_type_rejects_opaque_nominals() {
        // `pre-path`/`path`/`graphics`/`image` used to be listed here. They are
        // not forked and never were: upstream 0.0.6's `base_type_hash_table`
        // (`types.cppo.ml:295-298`) maps all four to the same base types 0.1
        // uses, and this port's `t_prepath`/`t_path`/`t_graphics`/`t_image` and
        // their `Value` reps take no version either. Only the port's
        // `name_to_mono` disagreed, via a V0_1 gate — see the test below.
        for name in ["math-text", "math-boxes", "font"] {
            let ty = PolyType::mono(MonoType::Variant(name.to_string(), vec![]));
            let err = adapt_export_type(&ty, v006(), v01()).unwrap_err();
            match err {
                BoundaryError::ForkedTypeExport { ty_name, .. } => assert_eq!(ty_name, name),
            }
        }
    }

    #[test]
    fn graphics_tier_base_types_are_not_forked_and_cross_in_both_directions() {
        // The four graphics-tier base types resolve identically under both
        // versions, so they must not appear in the fork set at all...
        let forked = crate::typecheck::forked_type_names();
        for name in ["pre-path", "path", "graphics", "image"] {
            assert!(
                !forked.contains(name),
                "`{name}` must not be reported as version-forked: upstream 0.0.6 \
                 registers it as a base type exactly as 0.1 does"
            );
            assert!(
                !reject_type_names().contains(name),
                "`{name}` must not be rejected"
            );
        }
        // ...and an export mentioning one must cross unchanged, either way.
        for (from, to) in [(v006(), v01()), (v01(), v006())] {
            for name in ["pre-path", "path", "graphics", "image"] {
                let ann = parse_ty(name);
                adapt_export_annotation(&ann, from, to)
                    .unwrap_or_else(|e| panic!("`{name}` must cross {from:?}->{to:?}: {e:?}"));
            }
        }
    }

    #[test]
    fn adapt_export_type_rejects_forked_leaf_nested_in_a_compound() {
        // S3: a forked leaf anywhere in the structure rejects the whole
        // export, even alongside an otherwise-fine math leaf.
        let ty = PolyType::mono(MonoType::Product(vec![
            MonoType::Base(BaseType::MathText),
            MonoType::Variant("page".to_string(), vec![]),
        ]));
        assert!(adapt_export_type(&ty, v006(), v01()).is_err());
    }

    #[test]
    fn adapt_export_type_accepts_ordinary_user_nominal() {
        // A ordinary, non-forked user type name must pass through untouched
        // (S4's flip side: only the specific reject set is conservative,
        // not every nominal).
        let ty = PolyType::mono(MonoType::Variant(
            "option".to_string(),
            vec![MonoType::Base(BaseType::Int)],
        ));
        assert!(adapt_export_type(&ty, v006(), v01()).is_ok());
    }

    // ------------------------------------------------------------------
    // adapt_export_annotation (CST-level helper)
    // ------------------------------------------------------------------

    fn parse_ty(src: &str) -> TypeExpr {
        // Reuse a `type` declaration's RHS to get a real parsed TypeExpr —
        // simplest way to build one without hand-rolling the CST.
        let file =
            rustyfi_syntax::parse_file(&format!("type xver-probe = {src}\n0\n")).expect("parse");
        for tb in &file.prelude {
            if let cst::TopBinding::Type(td) = tb {
                if let cst::TypeDeclBody::Synonym(ty) = &td.body {
                    return ty.clone();
                }
            }
        }
        panic!("expected a type synonym declaration");
    }

    #[test]
    fn adapt_export_annotation_relabels_bare_math() {
        let ann = parse_ty("math");
        let out = adapt_export_annotation(&ann, v006(), v01()).expect("math must be accepted");
        match out {
            TypeExpr::Atom(TypeProd {
                first:
                    TypeApp {
                        head: TypeAtom::Name(n),
                        ..
                    },
                ..
            }) => assert_eq!(n.name, "math-text"),
            other => panic!("expected a bare relabeled Name, got {other:?}"),
        }
    }

    #[test]
    fn adapt_export_annotation_relabels_math_nested_in_function_type() {
        let ann = parse_ty("math -> math");
        let out =
            adapt_export_annotation(&ann, v006(), v01()).expect("math -> math must be accepted");
        // Round-trip through Display/Debug is overkill; just confirm no
        // `Err` and that unparsing still contains `math-text` twice, not
        // bare `math`.
        let unparsed = format!("{out:?}");
        assert!(
            !unparsed.contains("\"math\""),
            "no bare `math` should survive: {unparsed}"
        );
    }

    #[test]
    fn adapt_export_annotation_rejects_page() {
        let ann = parse_ty("page -> document");
        let err = adapt_export_annotation(&ann, v006(), v01()).expect_err("page must reject");
        match err {
            BoundaryError::ForkedTypeExport { ty_name, .. } => assert_eq!(ty_name, "page"),
        }
    }

    #[test]
    fn adapt_export_annotation_rejects_deco() {
        let ann = parse_ty("deco");
        let err = adapt_export_annotation(&ann, v006(), v01()).expect_err("deco must reject");
        match err {
            BoundaryError::ForkedTypeExport { ty_name, .. } => assert_eq!(ty_name, "deco"),
        }
    }

    // ------------------------------------------------------------------
    // Slice X4a: the REVERSE (V0_1 -> V0_0) direction of the leaf policy
    // (`relabel_or_reject_name`'s new match arm). NOTE: these pin the PURE
    // function's contract only — `lib.rs`'s `compile_document_v006_xver_
    // with_trials` deliberately does NOT call `adapt_export_annotation`/
    // `relabel_type_decls` in this direction (see that function's own doc
    // comment for why: `v1::module_check::check_program`'s `Checker.version`
    // is hard-coded `V0_1` for EVERY type declaration in the merged program,
    // so a foreign 0.1 dependency's OWN "math-text"/"math-boxes" spelling is
    // ALREADY the vocabulary that matters — relabeling it to 0.0.6's "math"
    // would corrupt it). This function stays available (and correctly
    // direction-aware) for a future caller — e.g. an X4b elaborated-IR-level
    // adapter — that isn't bound by that same hard-coded-V0_1 constraint.
    // ------------------------------------------------------------------

    #[test]
    fn adapt_export_annotation_reverse_relabels_math_text_to_math() {
        let ann = parse_ty("math-text");
        let out = adapt_export_annotation(&ann, v01(), v006()).expect("math-text must be accepted");
        match out {
            TypeExpr::Atom(TypeProd {
                first:
                    TypeApp {
                        head: TypeAtom::Name(n),
                        ..
                    },
                ..
            }) => assert_eq!(n.name, "math"),
            other => panic!("expected a bare relabeled Name, got {other:?}"),
        }
    }

    #[test]
    fn adapt_export_annotation_reverse_relabels_math_boxes_to_math() {
        let ann = parse_ty("math-boxes");
        let out =
            adapt_export_annotation(&ann, v01(), v006()).expect("math-boxes must be accepted");
        match out {
            TypeExpr::Atom(TypeProd {
                first:
                    TypeApp {
                        head: TypeAtom::Name(n),
                        ..
                    },
                ..
            }) => assert_eq!(n.name, "math"),
            other => panic!("expected a bare relabeled Name, got {other:?}"),
        }
    }

    #[test]
    fn adapt_export_annotation_reverse_rejects_page() {
        let ann = parse_ty("page -> document");
        let err = adapt_export_annotation(&ann, v01(), v006()).expect_err("page must reject");
        match err {
            BoundaryError::ForkedTypeExport { ty_name, .. } => assert_eq!(ty_name, "page"),
        }
    }

    #[test]
    fn adapt_export_annotation_reverse_rejects_a_genuinely_forked_name() {
        // Was `graphics`, which is not forked (see
        // `graphics_tier_base_types_are_not_forked_and_cross_in_both_directions`).
        // `font` still is: 0.0.6's is an opaque nominal, 0.1's a `string`
        // stand-in — different runtime reps, so it must not cross.
        let ann = parse_ty("font");
        let err = adapt_export_annotation(&ann, v01(), v006()).expect_err("font must reject");
        match err {
            BoundaryError::ForkedTypeExport { ty_name, .. } => assert_eq!(ty_name, "font"),
        }
    }

    #[test]
    fn adapt_export_annotation_forward_math_relabel_is_unaffected_by_the_reverse_arm() {
        // Non-regression: adding the (V0_1, V0_0) arm must not perturb the
        // EXISTING (V0_0, V0_1) "math"->"math-text" relabel.
        let ann = parse_ty("math");
        let out =
            adapt_export_annotation(&ann, v006(), v01()).expect("math must still be accepted");
        match out {
            TypeExpr::Atom(TypeProd {
                first:
                    TypeApp {
                        head: TypeAtom::Name(n),
                        ..
                    },
                ..
            }) => assert_eq!(n.name, "math-text"),
            other => panic!("expected a bare relabeled Name, got {other:?}"),
        }
    }

    // ------------------------------------------------------------------
    // relabel_type_decls (prelude-level splice-arm entry point)
    // ------------------------------------------------------------------

    fn prelude_of(src: &str) -> Vec<cst::TopBinding> {
        let file = rustyfi_syntax::parse_file(src).expect("parse");
        file.prelude
    }

    #[test]
    fn relabel_type_decls_rewrites_variant_ctor_payload() {
        let prelude = prelude_of("type xver-wrap = XverWrap of math\n0\n");
        let out =
            relabel_type_decls(&prelude, v006(), v01()).expect("math-only prelude must relabel");
        match &out[0] {
            cst::TopBinding::Type(td) => match &td.body {
                cst::TypeDeclBody::Variant { first, .. } => {
                    let ty = &first.of_ty.as_ref().unwrap().ty;
                    match ty {
                        TypeExpr::Atom(TypeProd {
                            first:
                                TypeApp {
                                    head: TypeAtom::Name(n),
                                    ..
                                },
                            ..
                        }) => assert_eq!(n.name, "math-text"),
                        other => panic!("expected relabeled Name, got {other:?}"),
                    }
                }
                other => panic!("expected a Variant body, got {other:?}"),
            },
            other => panic!("expected a Type binding, got {other:?}"),
        }
    }

    #[test]
    fn relabel_type_decls_recurses_into_nested_module() {
        let prelude =
            prelude_of("module M = struct\n  type inner-wrap = InnerWrap of math\nend\n0\n");
        let out = relabel_type_decls(&prelude, v006(), v01())
            .expect("nested math-only prelude must relabel");
        match &out[0] {
            cst::TopBinding::Module { decls, .. } => match decls[0].0.as_ref() {
                cst::TopBinding::Type(td) => match &td.body {
                    cst::TypeDeclBody::Variant { first, .. } => {
                        let ty = &first.of_ty.as_ref().unwrap().ty;
                        match ty {
                            TypeExpr::Atom(TypeProd {
                                first:
                                    TypeApp {
                                        head: TypeAtom::Name(n),
                                        ..
                                    },
                                ..
                            }) => assert_eq!(n.name, "math-text"),
                            other => panic!("expected relabeled Name, got {other:?}"),
                        }
                    }
                    other => panic!("expected a Variant body, got {other:?}"),
                },
                other => panic!("expected a nested Type binding, got {other:?}"),
            },
            other => panic!("expected a Module binding, got {other:?}"),
        }
    }

    #[test]
    fn relabel_type_decls_leaves_non_math_untouched() {
        // Sanity: a prelude with no forked type text at all is unaffected
        // (this exercises the same walk the touched=={} fast path in
        // lib.rs never even calls, but confirms the walk itself is a no-op
        // absent any `math`).
        let prelude = prelude_of("type ordinary = Foo of int\n0\n");
        let out = relabel_type_decls(&prelude, v006(), v01()).expect("no forked names present");
        assert_eq!(format!("{prelude:?}"), format!("{out:?}"));
    }

    // ------------------------------------------------------------------
    // X3b: classify_deco_exports / deco_coercion_prelude
    // ------------------------------------------------------------------

    #[test]
    fn classify_deco_exports_accepts_bare_top_level_letrec() {
        let prelude = prelude_of("let-rec xver-my-deco : deco | (x, y) w h d = []\n0\n");
        let exports =
            classify_deco_exports(&prelude, v006(), v01()).expect("bare `: deco` must be accepted");
        assert_eq!(exports.len(), 1);
        assert_eq!(exports[0].name, "xver-my-deco");
        assert_eq!(exports[0].kind, DecoKind::Deco);
    }

    #[test]
    fn classify_deco_exports_accepts_bare_decoset() {
        // `| ()` is the ONLY legal idiom for a plain `deco-set` VALUE export
        // — `elaborate.rs` requires a `let-rec`'s RHS to be a function, so
        // `let-rec name : deco-set | = (tuple)` (zero params, no `()`) does
        // not even elaborate; see `classify_rec_binding_deco`'s doc comment.
        let prelude = prelude_of("let-rec xver-my-decoset : deco-set | () = (0, 0, 0, 0)\n0\n");
        let exports = classify_deco_exports(&prelude, v006(), v01())
            .expect("`| ()` deco-set must be accepted");
        assert_eq!(exports.len(), 1);
        assert_eq!(exports[0].name, "xver-my-decoset");
        assert_eq!(exports[0].kind, DecoKind::DecoSet);
    }

    #[test]
    fn classify_deco_exports_rejects_decoset_with_wrong_params() {
        // A `deco-set` export with a REAL (non-unit) param — outside X3b's
        // scoped support (only the bare `| ()` idiom is handled).
        let prelude = prelude_of("let-rec xver-my-decoset : deco-set | t = (0, 0, 0, 0)\n0\n");
        let err = classify_deco_exports(&prelude, v006(), v01())
            .expect_err("a deco-set export with a non-unit param must still be rejected");
        match err {
            BoundaryError::ForkedTypeExport { ty_name, .. } => assert_eq!(ty_name, "deco-set"),
        }
    }

    #[test]
    fn classify_deco_exports_ignores_type_synonym() {
        // A bare `type .. = deco` synonym has no attached value — safe with
        // zero coercion, so it must not be classified as a `DecoExport` at
        // all (nothing to wrap).
        let prelude = prelude_of("type xver-deco-alias = deco\n0\n");
        let exports = classify_deco_exports(&prelude, v006(), v01())
            .expect("a type synonym must be accepted");
        assert!(exports.is_empty());
    }

    #[test]
    fn classify_deco_exports_accepts_curried_prefix() {
        // `length -> deco` — the arrow-PREFIXED shape. Was out of X3b's
        // original scope; the wrapper now eta-expands over the leading
        // arguments, so it is classified with its lead arity instead.
        let prelude =
            prelude_of("let-rec xver-my-deco : length -> deco | t (x, y) w h d = []\n0\n");
        let got = classify_deco_exports(&prelude, v006(), v01())
            .expect("a curried-prefix deco export is now wrappable");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].name, "xver-my-deco");
        assert_eq!(got[0].lead_arity, 1);
        assert!(got[0].module_path.is_empty());
    }

    #[test]
    fn classify_deco_exports_accepts_module_sig_item() {
        // The shape the whole 0.0.6 corpus actually uses: exports inside a
        // `module .. : sig .. end`. Recorded with the enclosing module path,
        // which is what routes it to `inject_module_deco_wrappers`.
        let prelude = prelude_of(
            "module M : sig\n  val simple : length -> deco\n  val plain : deco\nend = struct\n  \
             let simple t (x, y) w h d = []\n  let plain (x, y) w h d = []\nend\n0\n",
        );
        let got = classify_deco_exports(&prelude, v006(), v01())
            .expect("a module-scoped deco export is now wrappable");
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].name, "simple");
        assert_eq!(got[0].lead_arity, 1);
        assert_eq!(got[0].module_path, vec!["M".to_string()]);
        assert_eq!(got[1].name, "plain");
        assert_eq!(got[1].lead_arity, 0);
    }

    #[test]
    fn module_deco_wrapper_is_injected_inside_the_module() {
        let mut prelude = prelude_of(
            "module M : sig\n  val simple : length -> deco\nend = struct\n  \
             let simple t (x, y) w h d = []\nend\n0\n",
        );
        let exports = classify_deco_exports(&prelude, v006(), v01()).unwrap();
        let before = match &prelude[0] {
            cst::TopBinding::Module { decls, .. } => decls.len(),
            other => panic!("expected a module, got {other:?}"),
        };
        inject_module_deco_wrappers(&mut prelude, &exports);
        match &prelude[0] {
            cst::TopBinding::Module { decls, .. } => {
                assert_eq!(
                    decls.len(),
                    before + 2,
                    "the X3c capture AND the wrapper must be appended INSIDE the module"
                );
                // The capture of the UNWRAPPED original comes FIRST, while the
                // original is still the innermost binding of its own name
                // (X3c — `DecoExport::orig_capture_name`).
                match &*decls[decls.len() - 2].0 {
                    cst::TopBinding::Let(tl) => {
                        assert_eq!(tl.name.name, "xver-fwd-orig-simple")
                    }
                    other => panic!("expected the X3c original capture, got {other:?}"),
                }
                // ...and the wrapper must shadow, i.e. bind the SAME name, LAST.
                match &*decls[decls.len() - 1].0 {
                    cst::TopBinding::Let(tl) => assert_eq!(tl.name.name, "simple"),
                    other => panic!("expected a shadowing `let simple`, got {other:?}"),
                }
            }
            other => panic!("expected a module, got {other:?}"),
        }
        // A module-scoped export must NOT also get a top-level shadow: there
        // is no `let M.simple` to write.
        assert!(deco_coercion_prelude(&exports).is_empty());
    }

    #[test]
    fn deco_coercion_prelude_generates_parseable_wrap() {
        let exports = vec![
            DecoExport {
                name: "xver-my-deco".to_string(),
                kind: DecoKind::Deco,
                lead_arity: 0,
                lead_opts: Vec::new(),
                module_path: Vec::new(),
                arg_downgrades: Vec::new(),
                unit_thunk: false,
            },
            DecoExport {
                name: "xver-my-decoset".to_string(),
                kind: DecoKind::DecoSet,
                lead_arity: 0,
                lead_opts: Vec::new(),
                module_path: Vec::new(),
                arg_downgrades: Vec::new(),
                unit_thunk: true,
            },
        ];
        let out = deco_coercion_prelude(&exports);
        // TWO synthetic `TopBinding::Let`s per export, in order: X3c's capture
        // of the unwrapped original, then the shadowing wrapper.
        assert_eq!(out.len(), 4);
        let names: Vec<&str> = out
            .iter()
            .map(|tb| match tb {
                cst::TopBinding::Let(tl) => tl.name.name.as_str(),
                other => panic!("expected a Let binding, got {other:?}"),
            })
            .collect();
        assert_eq!(
            names,
            vec![
                "xver-fwd-orig-xver-my-deco",
                "xver-my-deco",
                "xver-fwd-orig-xver-my-decoset",
                "xver-my-decoset",
            ]
        );
    }

    #[test]
    fn deco_coercion_prelude_empty_is_empty() {
        assert!(deco_coercion_prelude(&[]).is_empty());
    }

    // ------------------------------------------------------------------
    // X4b: classify_deco_exports_v01_sig / deco_downgrade_prelude (the
    // reverse direction — a 0.1 dependency's `deco` export consumed by a
    // 0.0.6 document, coerced by wrapping its single `graphics` in a
    // singleton list)
    // ------------------------------------------------------------------

    fn v1_file(src: &str) -> cst_v1::FileV1 {
        cst_v1::parse_file_v1(src).expect("parse v1 fixture")
    }

    /// [`classify_deco_exports_v01_sig`] against `file`'s OWN surface — the
    /// same `build_file_surface`-then-classify order `lib.rs`'s reverse splice
    /// arm uses, so a `signature S = ..` bind in the fixture is already
    /// registered by the time a `module M : S` decl has to resolve it.
    fn classify_v1(file: &cst_v1::FileV1) -> Result<Vec<DecoExport>, BoundaryError> {
        let mut surfaces = SurfaceEnv::default();
        surface::build_file_surface(file, &mut surfaces);
        classify_deco_exports_v01_sig(file, &surfaces)
    }

    #[test]
    fn classify_v01_sig_accepts_bare_sig_val_deco() {
        // The ONE shape 0.1's grammar can express a bare `deco` ascription
        // at all: a top-level module's own `sig val name : deco` item
        // (`cst_v1::Bind::Value` has no ascription syntax of its own).
        let file = v1_file(
            "module M :> sig\n  val my-deco : deco\nend = struct\n  val my-deco = 0\nend\n",
        );
        let exports = classify_v1(&file).expect("a bare `: deco` sig item");
        assert_eq!(exports.len(), 1);
        assert_eq!(exports[0].name, "my-deco");
        assert_eq!(exports[0].kind, DecoKind::Deco);
        assert_eq!(exports[0].lead_arity, 0);
        assert_eq!(exports[0].module_path, vec!["M".to_string()]);
        assert_eq!(deco_export_qualified_name(&exports[0]), "M.my-deco");
    }

    #[test]
    fn classify_v01_sig_accepts_bare_sig_val_decoset() {
        let file = v1_file("module M :> sig\n  val my-decoset : deco-set\nend = struct\n  val my-decoset = 0\nend\n");
        let exports = classify_v1(&file).expect("a bare `: deco-set` sig item");
        assert_eq!(exports.len(), 1);
        assert_eq!(exports[0].kind, DecoKind::DecoSet);
        assert!(
            !exports[0].unit_thunk,
            "a 0.1 sig-declared `deco-set` is bound to the bare 4-tuple — no `()` thunk"
        );
    }

    #[test]
    fn classify_v01_sig_accepts_curried_sig_val() {
        let file =
            v1_file("module M :> sig\n  val my-deco : length -> color -> deco\nend = struct\n  val my-deco t c p w h d = 0\nend\n");
        let exports = classify_v1(&file).expect("an arrow-tailed `deco` export");
        assert_eq!(exports.len(), 1);
        assert_eq!(exports[0].kind, DecoKind::Deco);
        assert_eq!(exports[0].lead_arity, 2);
    }

    #[test]
    fn classify_v01_sig_accepts_an_optional_argument_arrow() {
        // Was a rejection for the same reason as the nested-module case above
        // (positional-only forwarding); the wrapper now carries labelled
        // optionals across, so a `?(l : ty) .. -> deco` export classifies.
        let file = v1_file(
            "module M :> sig\n  val my-deco : ?(thickness : length) length -> deco\nend \
             = struct\n  val my-deco = 0\nend\n",
        );
        let exports = classify_v1(&file).expect("a labelled-optional arrow is forwardable now");
        assert_eq!(exports.len(), 1);
        assert_eq!(exports[0].name, "my-deco");
    }

    #[test]
    fn classify_v01_sig_rejects_paren() {
        let file = v1_file(
            "module M :> sig\n  val my-paren : paren\nend = struct\n  val my-paren = 0\nend\n",
        );
        // NOT because 0.1's `paren` is a stand-in — `prim_types::t_paren`
        // matches saphe-split's `tPAREN` exactly, as the V0_0 arm matches
        // `v0.0.6 primitives.cppo.ml:86`. It rejects because the forward
        // wrapper is a PROJECTION of the 0.1 context onto 0.0.6's three
        // explicit scalars, and that has no inverse: the reverse call site has
        // no context to hand and the caller's explicit AXIS reaches 0.1 only
        // through the math font's MATH-table ratio, which nothing can set. See
        // `forked_note`'s `"paren"` arm.
        let err = classify_v1(&file)
            .expect_err("a 0.1 `paren` export must still reject in the reverse direction");
        match err {
            BoundaryError::ForkedTypeExport { ty_name, .. } => assert_eq!(ty_name, "paren"),
        }
    }

    #[test]
    fn classify_v01_sig_crosses_nested_module_under_composed_key() {
        // A nested member's seal key goes through `walk_nested_seals_a`'s path
        // composition — "push the child module's name onto the parent's" — so
        // the classifier reproduces exactly that, and the shadow's qualified
        // name is the composed `Outer.Inner.my-deco`.
        let file = v1_file(
            "module Outer :> sig\n  module Inner : sig val my-deco : deco end\nend = struct\n  \
             module Inner :> sig val my-deco : deco end = struct val my-deco = 0 end\nend\n",
        );
        let exports =
            classify_v1(&file).expect("a NESTED module's sig `val : deco` item must cross");
        assert_eq!(exports.len(), 1);
        assert_eq!(exports[0].kind, DecoKind::Deco);
        assert_eq!(
            deco_export_qualified_name(&exports[0]),
            "Outer.Inner.my-deco"
        );
    }

    #[test]
    fn classify_v01_sig_ignores_a_signature_member_that_binds_no_value() {
        // A `signature S = ..` MEMBER declares a signature, not a value:
        // `handle_signature_decl` only identity-checks it against the struct's
        // own `signature` bind, so no member of it is reachable at any path a
        // 0.0.6 consumer could name (0.0.6 has no signature syntax at all).
        // Nothing to cross — and therefore nothing to refuse either.
        let file = v1_file(
            "module Outer :> sig\n  signature S = sig val my-deco : deco end\nend = struct\n  \
             signature S = sig val my-deco : deco end\nend\n",
        );
        assert!(classify_v1(&file)
            .expect("a signature member binds no value — nothing to coerce")
            .is_empty());
    }

    #[test]
    fn classify_v01_sig_crosses_a_nested_module_typed_by_a_named_signature() {
        // The Task-1 headline: the nested decl's signature is a NAME, not a
        // literal `sig .. end`. `surface::find_sig_keyed` dereferences it —
        // outward from the same `site_path` `module_check::resolve_sig` uses —
        // so the member lands under the composed key `Outer.Inner.my-deco`,
        // exactly as the literal spelling does.
        let file = v1_file(
            "module Outer :> sig\n  signature S = sig val my-deco : deco end\n  \
             module Inner : S\nend = struct\n  \
             signature S = sig val my-deco : deco end\n  \
             module Inner :> S = struct val my-deco = 0 end\nend\n",
        );
        let exports =
            classify_v1(&file).expect("a nested module typed by a NAMED signature must now cross");
        assert_eq!(exports.len(), 1);
        assert_eq!(exports[0].kind, DecoKind::Deco);
        assert_eq!(
            deco_export_qualified_name(&exports[0]),
            "Outer.Inner.my-deco"
        );
    }

    #[test]
    fn classify_v01_sig_crosses_through_an_include_at_the_enclosing_path() {
        // `include S` splices S's decls into the ENCLOSING signature in place
        // (`module_check::splice_decls`), so the member's path is the
        // includer's own — `Outer.my-deco`, NOT `Outer.S.my-deco`.
        let file = v1_file(
            "module Outer :> sig\n  signature S = sig val my-deco : deco end\n  include S\nend \
             = struct\n  signature S = sig val my-deco : deco end\n  val my-deco = 0\nend\n",
        );
        let exports = classify_v1(&file).expect("an `include`d `deco` export must cross");
        assert_eq!(exports.len(), 1);
        assert_eq!(deco_export_qualified_name(&exports[0]), "Outer.my-deco");
    }

    #[test]
    fn classify_v01_sig_crosses_through_a_with_type_refinement() {
        // A `with type` refinement narrows a TYPE member; every `val` decl's
        // SPELLED type is the base's, unchanged, so the scan reads through to
        // the base's decl list.
        let file = v1_file(
            "module Outer :> sig\n  \
             signature S = sig type t :: o  val my-deco : deco end\n  \
             module Inner : S with type t = int\nend = struct\n  \
             signature S = sig type t :: o  val my-deco : deco end\n  \
             module Inner :> S with type t = int = struct\n    \
             type t = int\n    val my-deco = 0\n  end\nend\n",
        );
        let exports = classify_v1(&file)
            .expect("a `with type`-refined nested signature must resolve to its base");
        assert_eq!(exports.len(), 1);
        assert_eq!(
            deco_export_qualified_name(&exports[0]),
            "Outer.Inner.my-deco"
        );
    }

    #[test]
    fn classify_v01_sig_rejects_a_deco_behind_a_functor_signature_member() {
        // The narrowing that genuinely survives: a functor is not a module, so
        // there is no `Outer.Make.my-deco` for a shadow to rebind; its members
        // exist only at an APPLICATION's path, in a file this scan cannot see.
        let file = v1_file(
            "module Outer :> sig\n  \
             module Make : (X : sig val n : int end) -> sig val my-deco : deco end\nend \
             = struct\n  \
             module Make = fun (X : sig val n : int end) -> struct val my-deco = 0 end\nend\n",
        );
        let err = classify_v1(&file)
            .expect_err("a `deco` behind a functor signature member must still reject");
        match err {
            BoundaryError::ForkedTypeExport { ty_name, .. } => assert_eq!(ty_name, "deco"),
        }
    }

    #[test]
    fn classify_v01_sig_unknown_signature_name_neither_crosses_nor_panics() {
        // An unresolvable NAME: `module_check::resolve_sig` turns this into a
        // precise "unknown signature name" error a moment later, so this scan
        // declines to guess rather than inventing text of its own — and, with
        // nothing textually reachable, has no `deco` to refuse either.
        let file = v1_file(
            "module Outer :> sig\n  module Inner : Nope\nend = struct\n  \
             module Inner = struct val my-deco = 0 end\nend\n",
        );
        assert!(classify_v1(&file)
            .expect("an unresolved name is downstream's error, not this scan's")
            .is_empty());
    }

    #[test]
    fn classify_v01_sig_self_including_signature_terminates() {
        // The cycle guard: `signature S = sig include S end` would otherwise
        // recur forever. Keyed by the RESOLVED table key, like
        // `module_check::resolve_named_sig`'s own guard.
        let file = v1_file(
            "module Outer :> sig\n  signature S = sig include S end\n  module Inner : S\nend \
             = struct\n  signature S = sig include S end\n  \
             module Inner = struct val my-deco = 0 end\nend\n",
        );
        assert!(classify_v1(&file)
            .expect("an include cycle is downstream's precise error, not a hang")
            .is_empty());
    }

    #[test]
    fn classify_v01_sig_ignores_type_only_mention() {
        // A transparent `type .. = deco` sig item merely NAMING
        // `deco`/`deco-set` (no value attached) is safe with zero coercion
        // — nothing to classify, and nothing to reject.
        let file = v1_file(
            "module M :> sig\n  type xver-deco-alias = deco\nend = struct\n  type xver-deco-alias = deco\nend\n",
        );
        assert!(classify_v1(&file)
            .expect("a type-only mention is safe")
            .is_empty());
    }

    #[test]
    fn classify_v01_sig_crosses_a_member_declared_at_a_type_synonym() {
        // The scan reads a `val`'s SPELLED type, and `t` is not a builtin —
        // so without expanding the signature's OWN `type t = deco` this
        // export silently declined to cross (surfacing much later as an
        // ordinary `TypeError` at a 0.0.6 consumer's call site).
        let file = v1_file(
            "module M :> sig\n  type t = deco\n  val frame : length -> t\nend = struct\n  \
             type t = deco\n  val frame w p x y z = 0\nend\n",
        );
        let exports = classify_v1(&file).expect("a synonym OF `deco` is a deco export");
        assert_eq!(exports.len(), 1);
        assert_eq!(exports[0].name, "frame");
        assert_eq!(exports[0].kind, DecoKind::Deco);
        assert_eq!(exports[0].lead_arity, 1);
        assert_eq!(deco_export_qualified_name(&exports[0]), "M.frame");
    }

    #[test]
    fn classify_v01_sig_expands_a_synonym_chain_and_an_arrow_bodied_one() {
        // A chain (`u` -> `t` -> `deco`) resolves, and an ARROW-BODIED
        // synonym contributes its own lead positions — the wrapper has to
        // eta-expand over exactly as many arguments as the spelled-out form
        // would give it.
        let file = v1_file(
            "module M :> sig\n  type t = deco\n  type u = t\n  type framer = length -> u\n  \
             val frame : color -> framer\nend = struct\n  val frame c w p x y z = 0\nend\n",
        );
        let exports = classify_v1(&file).expect("a synonym CHAIN is still a deco export");
        assert_eq!(exports.len(), 1);
        assert_eq!(exports[0].kind, DecoKind::Deco);
        assert_eq!(
            exports[0].lead_arity, 2,
            "one lead position from the `val`'s own arrow, one from the synonym's body"
        );
    }

    #[test]
    fn classify_v01_sig_crosses_a_synonym_declared_in_an_included_signature() {
        // `include S` splices S's decls into the enclosing signature in
        // place, so a synonym S declares is in scope for the includer's own
        // `val` decls — the same scope `module_check::splice_decls` gives it.
        let file = v1_file(
            "module Outer :> sig\n  signature S = sig type t = deco end\n  include S\n  \
             val frame : t\nend = struct\n  signature S = sig type t = deco end\n  \
             type t = deco\n  val frame = 0\nend\n",
        );
        let exports = classify_v1(&file).expect("an `include`d synonym is in scope");
        assert_eq!(exports.len(), 1);
        assert_eq!(deco_export_qualified_name(&exports[0]), "Outer.frame");
    }

    #[test]
    fn classify_v01_sig_crosses_a_nested_members_use_of_an_enclosing_synonym() {
        // A nested `sig` sees its parent's type declarations, so the map is
        // threaded down rather than reset at each layer.
        let file = v1_file(
            "module Outer :> sig\n  type t = deco\n  module Inner : sig val frame : t end\nend \
             = struct\n  type t = deco\n  module Inner :> sig val frame : t end \
             = struct val frame = 0 end\nend\n",
        );
        let exports = classify_v1(&file).expect("an enclosing layer's synonym is in scope");
        assert_eq!(exports.len(), 1);
        assert_eq!(deco_export_qualified_name(&exports[0]), "Outer.Inner.frame");
    }

    #[test]
    fn classify_v01_sig_opaque_type_is_not_a_deco_and_does_not_cross() {
        // The distinction that matters: an OPAQUE `type t :: o` names no
        // forked type at all, so it must NOT start being coerced — nor
        // rejected. (And it is not the builtin of the same name either: a
        // signature-declared `type deco :: o` shadows it.)
        let file = v1_file(
            "module M :> sig\n  type t :: o\n  val frame : length -> t\nend = struct\n  \
             type t = int\n  val frame w = 0\nend\n",
        );
        assert!(classify_v1(&file)
            .expect("an opaque type is not a deco")
            .is_empty());

        let shadowed = v1_file(
            "module M :> sig\n  type deco :: o\n  val frame : deco\nend = struct\n  \
             type deco = int\n  val frame = 0\nend\n",
        );
        assert!(
            classify_v1(&shadowed)
                .expect("a locally-declared `deco` is not the builtin one")
                .is_empty(),
            "map-first lookup: a signature's own `type deco` shadows the builtin, so no \
             coercion wrapper may be generated for a value that is not a `deco` at all"
        );
    }

    #[test]
    fn classify_v01_sig_rejects_a_synonym_of_deco_buried_in_a_compound() {
        // The deliberate rejection survives synonym expansion: a `deco` the
        // positional wrapper cannot express is refused just as loudly when
        // it is spelled through a synonym.
        let file = v1_file(
            "module M :> sig\n  type t = deco\n  val frames : t list\nend = struct\n  \
             val frames = 0\nend\n",
        );
        let err = classify_v1(&file).expect_err("a buried deco must reject, synonym or not");
        match err {
            BoundaryError::ForkedTypeExport { ty_name, .. } => assert_eq!(ty_name, "deco"),
        }
    }

    #[test]
    fn classify_v01_sig_synonym_cycle_terminates() {
        // `type t = u  type u = t` is a later phase's error; this scan must
        // decline rather than loop.
        let file = v1_file(
            "module M :> sig\n  type t = u\n  type u = t\n  val frame : t\nend = struct\n  \
             val frame = 0\nend\n",
        );
        assert!(classify_v1(&file)
            .expect("a synonym cycle is downstream's error, not a hang")
            .is_empty());
    }

    #[test]
    fn classify_v01_sig_crosses_a_with_type_refined_deco() {
        // The other spelling of the same false negative: the base declares
        // `type t :: o` opaquely and the USE SITE refines it to `deco`, so
        // the member really is a `deco` at this signature.
        let file = v1_file(
            "module Outer :> sig\n  \
             signature S = sig type t :: o  val frame : t end\n  \
             module Inner : S with type t = deco\nend = struct\n  \
             signature S = sig type t :: o  val frame : t end\n  \
             module Inner :> S with type t = deco = struct\n    \
             type t = deco\n    val frame = 0\n  end\nend\n",
        );
        let exports = classify_v1(&file).expect("a `with type`-refined `deco` member crosses");
        assert_eq!(exports.len(), 1);
        assert_eq!(deco_export_qualified_name(&exports[0]), "Outer.Inner.frame");
    }

    #[test]
    fn classify_v01_sig_crosses_a_with_submodule_type_refined_deco() {
        // Task 2's half, from this scan's side: `S with M type t = deco`
        // used to have no decl list even in principle (`module_check::
        // resolve_sig` rejected the form outright), so the scan declined.
        // The refinement now descends into the named member, where it makes
        // that member's own `t` transparent — and the export crosses.
        let file = v1_file(
            "module Outer :> sig\n  \
             module Inner : sig type t :: o  val frame : t end\n\
             end with Inner type t = deco = struct\n  \
             module Inner :> sig type t :: o  val frame : t end \
             = struct type t = deco  val frame = 0 end\nend\n",
        );
        let exports =
            classify_v1(&file).expect("a `with M type`-refined `deco` member must cross");
        assert_eq!(exports.len(), 1);
        assert_eq!(deco_export_qualified_name(&exports[0]), "Outer.Inner.frame");
    }

    #[test]
    fn classify_v01_sig_empty_for_no_sig() {
        let file = v1_file("module M = struct\n  val my-deco p w h d = 0\nend\n");
        assert!(
            classify_v1(&file)
                .expect("no sig, nothing to see")
                .is_empty(),
            "an UNSEALED module (no sig_annot at all) has no textual site for this scan to read"
        );
    }

    #[test]
    fn classify_v01_sig_empty_for_document() {
        let file = v1_file("0\n");
        assert!(classify_v1(&file)
            .expect("a document is never a dependency")
            .is_empty());
    }

    fn one_deco_export() -> Vec<DecoExport> {
        let file = v1_file(
            "module M :> sig\n  val my-deco : length -> deco\nend = struct\n  val my-deco t p w h d = 0\nend\n",
        );
        classify_v1(&file).expect("classify")
    }

    fn binding_name(tb: &cst::TopBinding) -> &str {
        match tb {
            cst::TopBinding::Let(tl) => tl.name.name.as_str(),
            other => panic!("expected a Let binding, got {other:?}"),
        }
    }

    #[test]
    fn deco_downgrade_capture_binds_only_a_private_name() {
        // The capture must NOT rebind `M.my-deco`: it is spliced while the 0.1
        // view is still the installed one, and rebinding there would hand the
        // coerced shape to the very 0.1 code the schedule protects.
        let out = deco_downgrade_prelude(&one_deco_export(), DowngradeStep::Capture);
        assert_eq!(out.len(), 1);
        assert_eq!(binding_name(&out[0]), "xver-rev-orig-M-my-deco");
    }

    #[test]
    fn deco_downgrade_install_rebinds_the_qualified_key() {
        // The shadow is bound under the export's own DOTTED qualified key —
        // no surface syntax spells that; see `deco_downgrade_prelude`.
        let out = deco_downgrade_prelude(&one_deco_export(), DowngradeStep::Install);
        assert_eq!(out.len(), 1);
        assert_eq!(binding_name(&out[0]), "M.my-deco");
    }

    #[test]
    fn deco_downgrade_restore_rebinds_the_qualified_key_to_the_capture() {
        // The restore rebinds the same dotted key straight back to the private
        // capture, so a 0.1 dependency after a 0.0.6 one reads the export at
        // exactly the scheme the exporting module sealed.
        let out = deco_downgrade_prelude(&one_deco_export(), DowngradeStep::Restore);
        assert_eq!(out.len(), 1);
        assert_eq!(binding_name(&out[0]), "M.my-deco");
        let src = format!("{:?}", out[0]);
        assert!(
            src.contains("xver-rev-orig-M-my-deco"),
            "the restore's body must name the private capture, got: {src}"
        );
    }

    #[test]
    fn deco_downgrade_private_name_is_a_function_of_the_qualified_key_alone() {
        // Load-bearing for the placement schedule: `Capture` and every later
        // `Install`/`Restore` are SEPARATE calls, so they can only agree on the
        // private name if it depends on nothing call-local.
        let exports = one_deco_export();
        let capture = deco_downgrade_prelude(&exports, DowngradeStep::Capture);
        let restore = deco_downgrade_prelude(&exports, DowngradeStep::Restore);
        assert!(format!("{:?}", restore[0]).contains(binding_name(&capture[0])));
    }

    #[test]
    fn deco_downgrade_prelude_empty_is_empty() {
        assert!(deco_downgrade_prelude(&[], DowngradeStep::Capture).is_empty());
        assert!(deco_downgrade_prelude(&[], DowngradeStep::Install).is_empty());
        assert!(deco_downgrade_prelude(&[], DowngradeStep::Restore).is_empty());
    }

    #[test]
    fn classify_v01_sig_does_not_perturb_forward_toplevel_letrec() {
        // Non-regression: X4b's reverse scan must not change the FORWARD
        // direction's existing top-level `let-rec` acceptance path at all
        // (a wholly separate function operating on a wholly separate CST
        // type — `cst_v1::FileV1`, not `cst::TopBinding`).
        let prelude = prelude_of("let-rec xver-my-deco : deco | (x, y) w h d = []\n0\n");
        let exports = classify_deco_exports(&prelude, v006(), v01())
            .expect("bare `: deco` must still be accepted forward");
        assert_eq!(exports.len(), 1);
    }

    // ------------------------------------------------------------------
    // Nested-module deco exports (the forward direction's own recursion
    // increment): a `deco` export inside a `module .. = struct .. end` —
    // or inside a module inside a module — is now classified and wrapped
    // rather than rejected.
    // ------------------------------------------------------------------

    #[test]
    fn classify_deco_in_sigless_module_letrec_ascription() {
        let prelude = prelude_of(
            "module XverMod = struct\n  \
             let-rec frame : length -> deco | t (x, y) w h d = []\nend\n0\n",
        );
        let exports = classify_deco_exports(&prelude, v006(), v01()).expect(
            "a `let-rec .. : deco` inside a sig-less module must be classified, not rejected",
        );
        assert_eq!(exports.len(), 1);
        assert_eq!(exports[0].name, "frame");
        assert_eq!(exports[0].kind, DecoKind::Deco);
        assert_eq!(exports[0].lead_arity, 1);
        assert_eq!(exports[0].module_path, vec!["XverMod".to_string()]);
    }

    #[test]
    fn classify_deco_in_doubly_nested_module_sig() {
        let prelude = prelude_of(
            "module Outer = struct\n  \
             module Inner : sig\n    val frame : length -> deco\n  end = struct\n    \
             let frame t (x, y) w h d = []\n  end\nend\n0\n",
        );
        let exports = classify_deco_exports(&prelude, v006(), v01())
            .expect("a doubly-nested module's sig `deco` export must be classified");
        assert_eq!(exports.len(), 1);
        assert_eq!(
            exports[0].module_path,
            vec!["Outer".to_string(), "Inner".to_string()]
        );
    }

    #[test]
    fn classify_deco_in_module_sig_is_not_double_wrapped_by_its_own_ascription() {
        // The `skip` set: a member declared in the module's `sig` AND
        // carrying its own `: deco` ascription must yield exactly ONE
        // wrapper, never two (two would unite an already-united graphics).
        let prelude = prelude_of(
            "module XverMod : sig\n  val frame : length -> deco\nend = struct\n  \
             let-rec frame : length -> deco | t (x, y) w h d = []\nend\n0\n",
        );
        let exports = classify_deco_exports(&prelude, v006(), v01()).expect("classify");
        assert_eq!(
            exports.len(),
            1,
            "one wrapper per export, even when sig and ascription both name the type"
        );
    }

    #[test]
    fn classify_deco_in_nested_module_accepts_an_optional_argument_arrow() {
        // This used to assert a REJECTION: the generated wrapper forwarded its
        // parameters positionally, and an optional argument has no positional
        // spelling. The wrapper now forwards optionals too, so the rejection
        // is gone and what matters is that the recursion into a module still
        // classifies the export -- and records WHICH leading parameters are
        // optional, since forwarding them depends on knowing that.
        let prelude = prelude_of(
            "module XverMod = struct\n  \
             let-rec frame : length ?-> length -> deco | t (x, y) w h d = []\nend\n0\n",
        );
        let exports = classify_deco_exports(&prelude, v006(), v01())
            .expect("an optional-argument arrow is forwardable now");
        assert_eq!(exports.len(), 1);
        assert_eq!(exports[0].module_path, vec!["XverMod".to_string()]);
        assert!(
            exports[0].lead_opts.contains(&LeadOpt::V006Optional),
            "the optional slot must be recorded, not flattened into a positional one: {:?}",
            exports[0].lead_opts
        );
    }
}
