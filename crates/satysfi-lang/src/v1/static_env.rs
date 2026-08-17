//! Sub-slice 2d-1 (`…/tmp/slice2d-sealing.md` §2.1): the semantic-signature
//! data model — the port-shaped analogue of upstream `staticEnv.ml`'s
//! `struct_signature`/`type_environment` (`staticEnv.ml:106-141`), radically
//! first-order: no `abstracted` quantifier wrapper (2d-2's stamps are
//! globally fresh strings, so the `quantifier = kind OpaqueIDMap.t`
//! bookkeeping collapses — see §3.1 of the spec), no macros/stages, functor
//! signatures deferred to 2f.
//!
//! 2d-1 populates and consumes only [`DeclaredVal`]/[`StaticEnv::seals`]/
//! [`StaticEnv::hidden`]/[`StampMint`]; [`TypeOpacity`]/[`DeclaredType`] and
//! the non-`vals` fields of [`ElabSig`] are shape-only placeholders that pin
//! the §3.1 representation decision (stamped nominal names, e.g. `"M.t#3"`)
//! ahead of 2d-2, which is their first writer — per the spec's §6 item 10
//! scope guard, do not widen these further until that sub-slice lands.

use crate::types::{MonoType, PolyType};
use satysfi_syntax::span::Span;
use std::collections::HashMap;

/// A member's declared value entry (upstream `value_entry`,
/// staticEnv.ml:82-86, minus stage). `scheme` is what sealing COMMITS;
/// `rigid` is the skolemized body the subsumption check ([`crate::v1::
/// sig_subtype::val_subsumes`]) unifies against (§3.4 — both are lowered
/// from the same source `TypeExpr`, so they are alpha-equivalent by
/// construction).
#[derive(Clone, Debug)]
pub(crate) struct DeclaredVal {
    /// The bare member name (`"x"`, `"\emph"`-style, or a `( + )`-style
    /// operator `BindName` string) — NOT qualified; the qualified key
    /// (`"M.x"`) is [`StaticEnv::seals`]'s map key, not stored redundantly
    /// here.
    pub(crate) name: String,
    /// Quantified over the decl's `quant` tyvars — the scheme sealing
    /// COMMITS at the alias binding (§4.2 step 4).
    pub(crate) scheme: PolyType,
    /// The same body with `quant` tyvars lowered to rigid nominal stamps
    /// (`"'a#7"`, §3.4) instead — the fixed pattern the depth check unifies
    /// the module's inferred (instantiated) scheme against.
    pub(crate) rigid: MonoType,
    /// The decl's `val` keyword span (diagnostics).
    pub(crate) span: Span,
    /// This decl's own `"#N"` stamp suffix (shared by every rigid tyvar
    /// `rigid` mentions — one [`StampMint`] draw per `Decl::Val`, not per
    /// tyvar; see `module_check.rs`'s seal-table walk). Not part of the
    /// spec's §2.1 struct sketch verbatim, but required by [`crate::v1::
    /// sig_subtype::val_subsumes`]'s escaped-skolem check (§4.2 step 5) to
    /// know which stamps belong to THIS check — keeping it alongside
    /// `rigid` (rather than a parallel map) avoids any risk of the two
    /// drifting apart.
    pub(crate) stamp_marker: String,
}

/// One resolved type entry (`types → (arity, opacity)`, upstream `lookup_
/// type_entry`). 2d-1 constructs none of these (sig `type` decls are 2d-2
/// placeholder errors, §4.7); the shape is fixed NOW because the abstract-
/// type representation decision (spec §3.1) pins it, and [`crate::v1::
/// sig_subtype`]'s unit tests exercise it directly to pin that decision
/// ahead of the sub-slice that first constructs one for real.
#[derive(Clone, Debug)]
#[allow(dead_code)] // first constructed by 2d-2
pub(crate) enum TypeOpacity {
    /// `type t = τ` in a sig: must equal the impl's type (2d-2).
    Transparent(MonoType),
    /// `type t :: kind` in a sig: the stamped abstract nominal key that
    /// escapes the seal, e.g. `"M.t#3"` (§3.1).
    Abstract { stamped: String },
}

#[derive(Clone, Debug)]
#[allow(dead_code)] // first constructed by 2d-2
pub(crate) struct DeclaredType {
    /// From `KindV1`: `o -> o` ⇒ 1 (kind_base count − 1).
    pub(crate) arity: usize,
    pub(crate) opacity: TypeOpacity,
    pub(crate) span: Span,
}

/// One elaborated signature body (upstream `struct_signature`,
/// staticEnv.ml:106-121). A `Vec`, not a map: decl order drives
/// deterministic error order (upstream folds the sig in order too). 2d-1
/// never constructs one of these (`module_check.rs`'s seal-table walk
/// writes `vals` straight into [`StaticEnv::seals`]/`hidden` without an
/// intermediate `ElabSig`); the type exists now so the representation is
/// pinned before 2d-2/2d-3 need it.
#[derive(Clone, Debug, Default)]
#[allow(dead_code)] // first constructed (as a whole) by 2d-2/2d-3
pub(crate) struct ElabSig {
    pub(crate) vals: Vec<DeclaredVal>,
    /// 2d-2.
    pub(crate) types: Vec<(String, DeclaredType)>,
    /// 2d-3 (`Decl::Module`).
    pub(crate) modules: Vec<(String, ElabSig)>,
    /// 2d-3 (`Decl::Signature`).
    pub(crate) signatures: Vec<(String, ElabSig)>,
}

/// The whole-program static environment the module checker threads
/// (upstream `type_environment`, staticEnv.ml:134-141, first-order). 2d-1
/// populates `seals`/`hidden` only; the other maps arrive with their owning
/// sub-slice (NOT stubbed with dead fields — each field arrives with its
/// writer, per the spec's §6 item 10 scope guard):
///
/// - 2d-2: `pub(crate) types: HashMap<String, DeclaredType>` (stamped
///   abstracts);
/// - 2d-3: `pub(crate) modules: HashMap<String, ElabSig>` (module aliases/
///   decls); `pub(crate) signatures: HashMap<String, ElabSig>` (`signature S
///   = …`).
#[derive(Default)]
pub(crate) struct StaticEnv {
    /// Sealed-module surface: qualified member name (`"M.x"`, `"M.N.y"`) →
    /// its declared entry. Consulted by the driver at each alias commit
    /// (`module_check.rs`'s `Ast::LetIn` interception, §4.2 steps 3-4).
    pub(crate) seals: HashMap<String, DeclaredVal>,
    /// Members hidden by a seal: qualified name → owning module path
    /// (`"M.N"`). Consulted by the unbound-use rewriter (§4.2 step 6).
    pub(crate) hidden: HashMap<String, String>,
}

/// Globally unique `#n` suffixes. `#` is not a lexable identifier char in
/// either SATySFi version, so a stamped name can never collide with (or be
/// spoofed by) a user-written nominal type name — the invariant both the
/// skolem trick (§3.4) and abstract stamping (§3.1, 2d-2) rest on.
#[derive(Default)]
pub(crate) struct StampMint(u64);

impl StampMint {
    pub(crate) fn next(&mut self) -> u64 {
        let n = self.0;
        self.0 += 1;
        n
    }
}
