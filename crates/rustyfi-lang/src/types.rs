//! The type language: base types, the mutable (union-find) representation of
//! type/row variables, monomorphic and polymorphic types, and level-based
//! generalization.
//!
//! This mirrors `mono_type_main` / `poly_type` / `kind` in v0.0.6's
//! `src/frontend/types.cppo.ml`, with two deliberate departures documented
//! at the relevant definitions below:
//!
//! 1. **Generalization is level-based (Rémy levels)**, not v0.0.6's
//!    `quantifiability` flag. See [`TypeContext`], [`generalize`] and
//!    [`instantiate`].
//! 2. **Extensible records are a first-class row type** (`Row::Empty` /
//!    `Row::Var` / `Row::Cons`), not v0.0.6's scheme of a *closed*
//!    `RecordType` plus a plain type variable that merely carries a
//!    `RecordKind` label-subset constraint. See [`Row`].

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

// ============================================================================
// Base types
// ============================================================================

/// Primitive types with no internal structure — the subset of v0.0.6's
/// `base_type` (`types.cppo.ml:255`) that this milestone's primitives need.
/// (`EnvType`/`RegExpType`/`InputPosType` are not yet used by anything in
/// `primitives.rs` and are left out; add them here when a primitive needs
/// them. `ImageType` was added for `load-image`/`use-image-by-width`
/// (`docs/plans/math-images.md` §Slice 1), and `PrePathType`/`PathType`/
/// `GraphicsType` for the Slice-1 graphics primitives — see
/// `docs/plans/graphics-subsystem.md`.)
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BaseType {
    Unit,
    Bool,
    Int,
    Float,
    Length,
    String,
    /// `inline-text` (v0.0.6: `TextRowType`).
    InlineText,
    /// `block-text` (v0.0.6: `TextColType`).
    BlockText,
    /// `math` (v0.0.6: `MathType`) — quoted math text. Reused, unmodified,
    /// as V0_1's `math-text` (upstream literally renamed 0.0.6's `math`,
    /// `research-stdlib-prims-backend.md:144-147`); see `MathBoxes` for the
    /// new V0_1-only half of the split.
    MathText,
    /// `math-boxes` (V0_1 only; `dev-0-1-0` `MathBoxesType`) — the
    /// evaluated math tree, bridged from `MathText` by the V0_1 primitive
    /// `read-math`. 0.0.6 has no name for this type (its `math` conflates
    /// both halves) and no value ever types as this under V0_0.
    MathBoxes,
    /// `image` (v0.0.6: `ImageType`) — a decoded raster image resource
    /// (`load-image`'s result; `docs/plans/math-images.md` §Slice 1).
    Image,
    /// `inline-boxes` (v0.0.6: `BoxRowType`).
    InlineBoxes,
    /// `block-boxes` (v0.0.6: `BoxColType`).
    BlockBoxes,
    Context,
    Document,
    /// `pre-path` (v0.0.6: `PrePathType`).
    PrePath,
    /// `path` (v0.0.6: `PathType`).
    Path,
    /// `graphics` (v0.0.6: `GraphicsType`).
    Graphics,
    /// `text-info` (v0.0.6: `TextInfoType`) — the text-mode context
    /// (`deepen-indent`/`get-initial-text-info`/`break`; docs/plans/
    /// context-box-prims.md §G sliver — see primitives.rs for the scoping
    /// note: the text/html backends themselves are out of scope).
    TextInfo,
}

impl BaseType {
    /// The SATySFi surface-syntax name, used by `Display`.
    pub fn name(self) -> &'static str {
        match self {
            BaseType::Unit => "unit",
            BaseType::Bool => "bool",
            BaseType::Int => "int",
            BaseType::Float => "float",
            BaseType::Length => "length",
            BaseType::String => "string",
            BaseType::InlineText => "inline-text",
            BaseType::BlockText => "block-text",
            BaseType::MathText => "math",
            BaseType::MathBoxes => "math-boxes",
            BaseType::Image => "image",
            BaseType::InlineBoxes => "inline-boxes",
            BaseType::BlockBoxes => "block-boxes",
            BaseType::Context => "context",
            BaseType::Document => "document",
            BaseType::PrePath => "pre-path",
            BaseType::Path => "path",
            BaseType::Graphics => "graphics",
            BaseType::TextInfo => "text-info",
        }
    }
}

impl fmt::Display for BaseType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

// ============================================================================
// A process-wide id source for variables minted where no `TypeContext` is
// available (see the doc comment on `instantiate` for why that happens).
// ============================================================================

/// `TypeContext` hands out ids from its own small counter (see below) for
/// ordinary inference-time freshness. `instantiate` and `unify`'s row
/// extension, however, have fixed signatures (per this crate's contract)
/// that carry no `TypeContext`, yet still need to mint brand new variables
/// (a fresh copy of each generalized variable; a fresh field/remainder pair
/// when extending an open row). They draw ids from this separate,
/// process-wide counter instead.
///
/// This is purely cosmetic: variable *identity* is always pointer equality
/// (`TyVarRef::same` / `RowVarRef::same`), never id equality, so the two
/// counters can never collide in any way that affects correctness — at
/// worst two unrelated variables coming from different sources print with
/// the same debug id. Seeded far away from `TypeContext`'s own counter just
/// to make that cosmetic overlap unlikely in small examples/tests.
static FRESH_ID: AtomicU64 = AtomicU64::new(1 << 32);

fn fresh_id() -> u64 {
    FRESH_ID.fetch_add(1, Ordering::Relaxed)
}

// ============================================================================
// Kind (mirrors v0.0.6's `mono_kind` / `FreeID_.kind`, types.cppo.ml:330-333)
// ============================================================================

/// The kind of a free type variable.
///
/// `Record(labels)` mirrors v0.0.6's `RecordKind`: it constrains a variable
/// that is not yet known to be anything in particular, but which field
/// access (`e#lbl`) or similar has already shown must eventually resolve to
/// *some* record type containing (at least) `labels`. Unlike v0.0.6's
/// `RecordKind`, which pairs each required label with its required field
/// type directly in the kind, this port stores only the label *names* here:
/// the field types themselves are tracked by the [`Row`] the variable
/// eventually gets bound to (see `unify::bind_var`'s `Kind::Record` branch).
/// This is strictly simpler and loses nothing, because in this port a
/// concrete record's structure is *always* a first-class `Row` — v0.0.6
/// needed to carry field types in the kind itself because its concrete
/// `RecordType` has no notion of "the type of label `l`" separate from the
/// whole closed association list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    Universal,
    Record(BTreeSet<String>),
}

// ============================================================================
// Type variables (mirrors v0.0.6's `FreeID_`/`mono_type_variable_info`,
// types.cppo.ml:121-191, 347-349)
// ============================================================================

/// The mutable union-find cell behind a type variable.
#[derive(Debug)]
pub enum TyVarLink {
    Free {
        id: u64,
        level: u32,
        kind: Kind,
    },
    /// This variable has been unified with a concrete type; `resolve`
    /// chases through this exactly like v0.0.6's `MonoLink`.
    Bound(MonoType),
}

/// A reference-counted handle to a type variable's union-find cell. Cloning
/// a `TyVarRef` shares the same cell (this is the union-find "pointer");
/// identity (not structure) is what `unify` and `generalize` compare.
#[derive(Clone, Debug)]
pub struct TyVarRef(Rc<RefCell<TyVarLink>>);

impl TyVarRef {
    pub(crate) fn new(id: u64, level: u32, kind: Kind) -> Self {
        TyVarRef(Rc::new(RefCell::new(TyVarLink::Free { id, level, kind })))
    }

    /// Identity comparison — the only correct notion of "same variable"
    /// once links can be mutated in place.
    pub fn same(&self, other: &TyVarRef) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }

    pub(crate) fn ptr_key(&self) -> usize {
        Rc::as_ptr(&self.0) as usize
    }

    /// `None` if this variable has already been bound.
    pub fn id(&self) -> Option<u64> {
        match &*self.0.borrow() {
            TyVarLink::Free { id, .. } => Some(*id),
            TyVarLink::Bound(_) => None,
        }
    }

    /// `None` if this variable has already been bound.
    pub fn level(&self) -> Option<u32> {
        match &*self.0.borrow() {
            TyVarLink::Free { level, .. } => Some(*level),
            TyVarLink::Bound(_) => None,
        }
    }

    /// No-op if this variable has already been bound.
    pub fn set_level(&self, new_level: u32) {
        if let TyVarLink::Free { level, .. } = &mut *self.0.borrow_mut() {
            *level = new_level;
        }
    }

    /// `Kind::Universal` if this variable has already been bound (asking a
    /// bound variable for its kind is meaningless; callers should `resolve`
    /// first).
    pub fn kind(&self) -> Kind {
        match &*self.0.borrow() {
            TyVarLink::Free { kind, .. } => kind.clone(),
            TyVarLink::Bound(_) => Kind::Universal,
        }
    }

    /// No-op if this variable has already been bound.
    pub fn set_kind(&self, new_kind: Kind) {
        if let TyVarLink::Free { kind, .. } = &mut *self.0.borrow_mut() {
            *kind = new_kind;
        }
    }

    /// Link this variable to a concrete type. Callers (`unify::bind_var`)
    /// are responsible for the occurs check; this just performs the store.
    pub fn bind(&self, ty: MonoType) {
        *self.0.borrow_mut() = TyVarLink::Bound(ty);
    }

    pub fn is_free(&self) -> bool {
        matches!(&*self.0.borrow(), TyVarLink::Free { .. })
    }
}

impl PartialEq for TyVarRef {
    fn eq(&self, other: &Self) -> bool {
        self.same(other)
    }
}
impl Eq for TyVarRef {}

pub(crate) fn new_ty_var(level: u32) -> TyVarRef {
    TyVarRef::new(fresh_id(), level, Kind::Universal)
}

// ============================================================================
// Row variables — the tail of an extensible record row. Structurally a
// mirror of `TyVarRef`/`TyVarLink`, except its "kind" is simply the set of
// labels already known to appear in whatever row it eventually resolves to
// (there is no `Universal` case: an empty set means "no labels required
// yet", which *is* the universal case for a row).
// ============================================================================

#[derive(Debug)]
pub enum RowVarLink {
    Free {
        id: u64,
        level: u32,
        kind: BTreeSet<String>,
    },
    Bound(Row),
}

#[derive(Clone, Debug)]
pub struct RowVarRef(Rc<RefCell<RowVarLink>>);

impl RowVarRef {
    pub(crate) fn new(id: u64, level: u32, kind: BTreeSet<String>) -> Self {
        RowVarRef(Rc::new(RefCell::new(RowVarLink::Free { id, level, kind })))
    }

    pub fn same(&self, other: &RowVarRef) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }

    pub(crate) fn ptr_key(&self) -> usize {
        Rc::as_ptr(&self.0) as usize
    }

    pub fn id(&self) -> Option<u64> {
        match &*self.0.borrow() {
            RowVarLink::Free { id, .. } => Some(*id),
            RowVarLink::Bound(_) => None,
        }
    }

    pub fn level(&self) -> Option<u32> {
        match &*self.0.borrow() {
            RowVarLink::Free { level, .. } => Some(*level),
            RowVarLink::Bound(_) => None,
        }
    }

    pub fn set_level(&self, new_level: u32) {
        if let RowVarLink::Free { level, .. } = &mut *self.0.borrow_mut() {
            *level = new_level;
        }
    }

    pub fn kind(&self) -> BTreeSet<String> {
        match &*self.0.borrow() {
            RowVarLink::Free { kind, .. } => kind.clone(),
            RowVarLink::Bound(_) => BTreeSet::new(),
        }
    }

    pub fn set_kind(&self, new_kind: BTreeSet<String>) {
        if let RowVarLink::Free { kind, .. } = &mut *self.0.borrow_mut() {
            *kind = new_kind;
        }
    }

    pub fn bind(&self, row: Row) {
        *self.0.borrow_mut() = RowVarLink::Bound(row);
    }

    pub fn is_free(&self) -> bool {
        matches!(&*self.0.borrow(), RowVarLink::Free { .. })
    }
}

impl PartialEq for RowVarRef {
    fn eq(&self, other: &Self) -> bool {
        self.same(other)
    }
}
impl Eq for RowVarRef {}

pub(crate) fn new_row_var(level: u32) -> RowVarRef {
    RowVarRef::new(fresh_id(), level, BTreeSet::new())
}

// ============================================================================
// Monomorphic types
// ============================================================================

/// A monomorphic type. Mirrors v0.0.6's `mono_type` (the `type_main`
/// variant instantiated at `mono_type_variable_info ref`), minus
/// `SynonymType`/`CodeType` (no type synonyms or multi-stage code in this
/// milestone) and with `Row`-based records instead of a closed `RecordType`
/// (see the module doc comment and [`Row`]).
#[derive(Clone, Debug)]
pub enum MonoType {
    Var(TyVarRef),
    Base(BaseType),
    /// `?(row) dom -> cod` — a function type carrying a labeled
    /// optional-argument [`Row`] (upstream `FuncType of row * typ * typ`,
    /// SATySFi 0.1). The row is `Row::Empty` for every 0.0.6-constructed
    /// function (see [`crate::prim_types::arrow`]), where it prints nothing
    /// and unifies trivially, so 0.0.6 behavior is byte-identical. A
    /// non-empty row (`Cons(label, option-payload-type, …)`) records the
    /// value-level `?(l = e)` labeled optional arguments the function
    /// accepts (SATySFi 0.1 optional-argument rows). The field is
    /// **positional** (no `..` in any destructure) deliberately: widening
    /// this variant makes the compiler flag every match site, guarding
    /// against a silently-dropped row in the sealed-module subsumption path.
    ///
    /// The row is **boxed** (`Box<Row>`, not an inline `Row`) so widening
    /// `Func` does not enlarge `MonoType` itself (`Row` is a ~40-byte enum;
    /// inlining it would make `Func` the largest variant and grow every
    /// stack frame that holds a `MonoType` by value — enough to tip the deep
    /// recursive typecheck of a large merged program over the default stack).
    /// A `Box<Row>` keeps `MonoType` at its pre-widening size, so 0.0.6 stack
    /// usage is unchanged.
    Func(Box<Row>, Box<MonoType>, Box<MonoType>),
    /// A tuple type, always with at least two elements.
    Product(Vec<MonoType>),
    List(Box<MonoType>),
    Ref(Box<MonoType>),
    Record(Row),
    /// A user-defined variant type applied to its arguments, e.g.
    /// `Variant("option", [int])` for `int option`. Identified by name
    /// rather than by a fresh `TypeID.t` as in v0.0.6 (`types.cppo.ml:318`)
    /// — this milestone has no notion of shadowing/re-declaring a variant
    /// type under the same name within one compilation, so a `String` is
    /// an adequate (and much simpler) stand-in for v0.0.6's globally-fresh
    /// `TypeID.t`.
    Variant(String, Vec<MonoType>),
    /// `[...] inline-cmd` (v0.0.6: `HorzCommandType`).
    InlineCmd(Vec<CmdArgType>),
    /// `[...] block-cmd` (v0.0.6: `VertCommandType`).
    BlockCmd(Vec<CmdArgType>),
    /// `[...] math-cmd` (v0.0.6: `MathCommandType`).
    MathCmd(Vec<CmdArgType>),
}

/// One command argument type: `ty` for a mandatory argument, or `ty?` for
/// an optional one (v0.0.6: `MandatoryArgumentType` / `OptionalArgumentType`,
/// types.cppo.ml:326-328). `optional`/`opt_labels` are version-discriminated
/// by construction (optional-arg-rows increment 3a): under `V0_0`
/// (positional model) `optional` marks a whole-slot `ty?` optional and
/// `opt_labels` is always empty; under `V0_1` (labeled model, upstream
/// `CommandArgType of typ LabelMap.t * typ`, `types.cppo.ml:214`) `optional`
/// is always `false` and `opt_labels` carries this slot's `?(l:τ,…)` bundle —
/// a CLOSED map (no row variable: upstream discards one if written,
/// `parser.mly:866`'s `TODO (error)`). Kept **sorted by label** at every
/// producer (`command_scheme`'s harvest, `lower_type_atom`'s sig lowering) so
/// `unify`/`Display`/sealing are order-insensitive — see `unify_cmd_args`'s
/// zip-equal equal-domain test.
#[derive(Clone, Debug)]
pub struct CmdArgType {
    pub optional: bool,
    pub opt_labels: Vec<(String, MonoType)>,
    pub ty: MonoType,
}

/// An extensible record row: a sequence of `label : type` bindings ending
/// either in `Empty` (a *closed* record — exactly these labels and no
/// others) or in `Var` (an *open* record — at least these labels, plus
/// whatever the row variable's eventual binding adds).
///
/// **Deviation from v0.0.6**: v0.0.6 has no such type former. Its
/// `RecordType` (types.cppo.ml:319) is always closed, and the only
/// polymorphism available for records is indirect: a plain type variable
/// can carry a `RecordKind` (a label-typed lower bound), and unifying that
/// variable against a concrete closed `RecordType` succeeds if the kind's
/// labels are a subset of the record's (typechecker.ml:480-500,
/// `Assoc.domain_included`). That scheme cannot express an open record
/// *type* standing on its own (e.g. as a function's return type) — only a
/// variable can be "open". Giving rows their own recursive type former
/// (`Row::Cons`/`Var`/`Empty`, Rémy-style row polymorphism, as used in
/// e.g. OCaml's own object/polymorphic-variant rows) is strictly more
/// general, lets `unify` do genuine label-subsumption with a *remainder*
/// row variable (see `unify::row_extract`), and is the more standard
/// design for a fresh implementation. `Kind::Record` is kept for the one
/// case v0.0.6 also has it for: a variable not yet known to be a record at
/// all (see `Kind`'s doc comment).
#[derive(Clone, Debug)]
pub enum Row {
    Empty,
    Var(RowVarRef),
    Cons(String, Box<MonoType>, Box<Row>),
}

// ============================================================================
// resolve / shallow_follow — chase `Bound` links, union-find "find".
// ============================================================================

/// Follow `Var(_)` → `Bound(ty)` links until reaching either a free
/// variable or a non-variable type. Does **not** recurse into the
/// structure of compound types (that's what makes it "shallow": a `Func`
/// whose domain is itself a bound variable is returned as-is, domain still
/// unresolved) — callers that need a fully dereferenced tree should
/// `resolve` again at each level as they recurse, which is exactly what
/// `unify` and `Display` do.
///
/// # Why `Cow`
///
/// This is the hottest function in the typechecker — ~316k calls on a corpus
/// document, and its cost is dominated by COPYING types, not by following
/// links. It used to return an owned `MonoType`, which meant the common case
/// (the argument is already resolved: not a variable, or a free one) ended in
/// `ty.clone()` — a full deep copy of a type produced solely to hand back an
/// owned value the caller then only inspected. Measured, that pointless tail
/// copy was **87-89% of all type nodes cloned during typechecking**, and
/// typecheck time tracks cloned-node volume near-linearly across the whole
/// corpus.
///
/// So the common case now borrows. Only the link-following path allocates,
/// and only because the `Bound` payload lives behind a `RefCell` whose guard
/// cannot outlive this frame. Callers that just match on the result want
/// `&*resolve(..)`; the few that keep it want `.into_owned()`.
pub fn resolve(ty: &MonoType) -> Cow<'_, MonoType> {
    if let MonoType::Var(v) = ty {
        let next = match &*v.0.borrow() {
            TyVarLink::Bound(inner) => Some(inner.clone()),
            TyVarLink::Free { .. } => None,
        };
        if let Some(inner) = next {
            return Cow::Owned(resolve(&inner).into_owned());
        }
    }
    Cow::Borrowed(ty)
}

/// The row analogue of [`resolve`], `Cow` for the same reason.
pub fn resolve_row(row: &Row) -> Cow<'_, Row> {
    if let Row::Var(v) = row {
        let next = match &*v.0.borrow() {
            RowVarLink::Bound(inner) => Some(inner.clone()),
            RowVarLink::Free { .. } => None,
        };
        if let Some(inner) = next {
            return Cow::Owned(resolve_row(&inner).into_owned());
        }
    }
    Cow::Borrowed(row)
}

// ============================================================================
// Polymorphic types and level-based generalization
// ============================================================================

/// A type scheme: a monomorphic body plus the set of that body's free
/// variables which are quantified over it.
///
/// **Deviation from v0.0.6**: v0.0.6 (types.cppo.ml:351-364) represents a
/// generalized variable by *converting* its `MonoFree` cell into a
/// `PolyBound` id, so the same physical type carries different
/// representations depending on whether you're looking at it "as a mono
/// type" or "as a poly type", and instantiating walks the body rebuilding
/// `PolyBound` occurrences into fresh `MonoFree` cells
/// (`typechecker.ml`/`typeenv.ml`'s `instantiate`). This port instead keeps
/// the quantified variables as ordinary (still-`Free`) `TyVarRef`/
/// `RowVarRef` cells and simply *remembers which ones they are* (`vars`/
/// `row_vars` below). `instantiate` then deep-copies `body`, replacing each
/// remembered variable (found by pointer identity, not by re-deriving
/// "is this quantifiable") with a fresh one, and leaving every other
/// variable in `body` (i.e. anything free in the surrounding, not-yet-
/// generalized context) completely untouched and shared. This is the
/// standard "efficient generalization via levels" technique used by many
/// modern ML-family implementations; it avoids v0.0.6's need for a
/// `quantifiability` flag (`Quantifiable`/`Unquantifiable`,
/// types.cppo.ml:54) to guard against generalizing a variable that
/// unification has already linked to something outside the current let
/// binding — here that's instead simply a consequence of levels: a
/// variable that unification touches from an outer scope gets its level
/// lowered (see `unify::occurs_var`/`occurs_var_in_row`), so by the time
/// `generalize` runs, it no longer looks "deep enough" to quantify.
#[derive(Clone, Debug)]
pub struct PolyType {
    vars: Vec<TyVarRef>,
    row_vars: Vec<RowVarRef>,
    body: MonoType,
}

impl PolyType {
    /// A trivial scheme with no quantified variables at all.
    pub fn mono(ty: MonoType) -> PolyType {
        PolyType {
            vars: Vec::new(),
            row_vars: Vec::new(),
            body: ty,
        }
    }

    /// Build a scheme by hand, explicitly naming which variables (which
    /// must occur free in `body`) are quantified. Used by `prim_types`,
    /// which constructs polymorphic primitive signatures (`::`, `!`)
    /// directly rather than via `generalize` (there is no enclosing
    /// inference level to generalize *from* at primitive-table
    /// construction time).
    pub(crate) fn from_vars(
        vars: Vec<TyVarRef>,
        row_vars: Vec<RowVarRef>,
        body: MonoType,
    ) -> PolyType {
        PolyType {
            vars,
            row_vars,
            body,
        }
    }

    /// The scheme's body, before instantiation — exposed for inspection
    /// (e.g. arity-checking) without needing to mint fresh variables.
    pub fn body(&self) -> &MonoType {
        &self.body
    }

    pub fn is_monomorphic(&self) -> bool {
        self.vars.is_empty() && self.row_vars.is_empty()
    }
}

impl fmt::Display for PolyType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.body, f)
    }
}

/// Per-inference-run state: the level stack for generalization, and a
/// counter for fresh variable ids (see `FRESH_ID`'s doc comment for why
/// `instantiate`/`unify` use a *different* counter than this one — the two
/// never need to agree, since identity is always by pointer).
pub struct TypeContext {
    next_id: u64,
    level: u32,
}

impl TypeContext {
    pub fn new() -> Self {
        TypeContext {
            next_id: 0,
            level: 0,
        }
    }

    fn next_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub fn level(&self) -> u32 {
        self.level
    }

    /// Enter a new `let`-nesting level. Call before inferring the
    /// right-hand side of a `let`.
    pub fn enter_level(&mut self) {
        self.level += 1;
    }

    /// Leave the current level. Call after inferring the right-hand side
    /// of a `let`, before calling `generalize`.
    pub fn leave_level(&mut self) {
        self.level -= 1;
    }

    pub fn fresh_var(&mut self) -> TyVarRef {
        self.fresh_var_with_kind(Kind::Universal)
    }

    pub fn fresh_var_with_kind(&mut self, kind: Kind) -> TyVarRef {
        TyVarRef::new(self.next_id(), self.level, kind)
    }

    pub fn fresh_row_var(&mut self) -> RowVarRef {
        self.fresh_row_var_with_kind(BTreeSet::new())
    }

    pub fn fresh_row_var_with_kind(&mut self, kind: BTreeSet<String>) -> RowVarRef {
        RowVarRef::new(self.next_id(), self.level, kind)
    }
}

impl Default for TypeContext {
    fn default() -> Self {
        Self::new()
    }
}

/// Quantify every free variable in `ty` whose level is deeper than `level`
/// (i.e. was created after entering the let binding being generalized).
/// Typical usage:
///
/// ```ignore
/// ctx.enter_level();
/// let ty = infer(ctx, rhs)?;
/// ctx.leave_level();
/// let scheme = generalize(ctx.level(), &ty);
/// ```
pub fn generalize(level: u32, ty: &MonoType) -> PolyType {
    let mut vars = Vec::new();
    let mut row_vars = Vec::new();
    collect_generalizable(level, ty, &mut vars, &mut row_vars);
    PolyType {
        vars,
        row_vars,
        body: ty.clone(),
    }
}

fn collect_generalizable(
    level: u32,
    ty: &MonoType,
    vars: &mut Vec<TyVarRef>,
    row_vars: &mut Vec<RowVarRef>,
) {
    match &*resolve(ty) {
        MonoType::Var(v) => {
            if let Some(lv) = v.level() {
                if lv > level && !vars.iter().any(|x| x.same(v)) {
                    vars.push(v.clone());
                }
            }
        }
        MonoType::Base(_) => {}
        MonoType::Func(row, a, b) => {
            collect_generalizable_row(level, &row, vars, row_vars);
            collect_generalizable(level, &a, vars, row_vars);
            collect_generalizable(level, &b, vars, row_vars);
        }
        MonoType::Product(ts) => {
            for t in ts {
                collect_generalizable(level, t, vars, row_vars);
            }
        }
        MonoType::List(t) | MonoType::Ref(t) => collect_generalizable(level, &t, vars, row_vars),
        MonoType::Record(row) => collect_generalizable_row(level, &row, vars, row_vars),
        MonoType::Variant(_, args) => {
            for t in args {
                collect_generalizable(level, t, vars, row_vars);
            }
        }
        MonoType::InlineCmd(cs) | MonoType::BlockCmd(cs) | MonoType::MathCmd(cs) => {
            for c in cs {
                collect_generalizable(level, &c.ty, vars, row_vars);
                for (_, lty) in &c.opt_labels {
                    collect_generalizable(level, lty, vars, row_vars);
                }
            }
        }
    }
}

fn collect_generalizable_row(
    level: u32,
    row: &Row,
    vars: &mut Vec<TyVarRef>,
    row_vars: &mut Vec<RowVarRef>,
) {
    match &*resolve_row(row) {
        Row::Empty => {}
        Row::Var(v) => {
            if let Some(lv) = v.level() {
                if lv > level && !row_vars.iter().any(|x| x.same(v)) {
                    row_vars.push(v.clone());
                }
            }
        }
        Row::Cons(_, t, rest) => {
            collect_generalizable(level, &t, vars, row_vars);
            collect_generalizable_row(level, &rest, vars, row_vars);
        }
    }
}

/// Instantiate a scheme: replace every quantified variable with a fresh
/// one at `level`, leaving everything else in the body shared as-is.
///
/// Note this takes no `&mut TypeContext` (per this module's contract) —
/// see `FRESH_ID`'s doc comment for how it still mints fresh, correctly
/// leveled variables.
pub fn instantiate(poly: &PolyType, level: u32) -> MonoType {
    let mut var_map: HashMap<usize, MonoType> = HashMap::new();
    for v in &poly.vars {
        let fresh = TyVarRef::new(fresh_id(), level, v.kind());
        var_map.insert(v.ptr_key(), MonoType::Var(fresh));
    }
    let mut row_map: HashMap<usize, Row> = HashMap::new();
    for v in &poly.row_vars {
        let fresh = RowVarRef::new(fresh_id(), level, v.kind());
        row_map.insert(v.ptr_key(), Row::Var(fresh));
    }
    substitute(&poly.body, &var_map, &row_map)
}

/// Deep-copy `ty`, replacing any (resolved) variable found in `var_map`/
/// `row_map` by pointer identity with its mapped replacement, and cloning
/// everything else structurally. Shared by `instantiate` (mapping
/// quantified variables to fresh ones) and by `prim_types::VariantDecl`
/// (mapping a declaration's parameter placeholders to the concrete
/// arguments of one particular constructor application).
pub(crate) fn substitute(
    ty: &MonoType,
    var_map: &HashMap<usize, MonoType>,
    row_map: &HashMap<usize, Row>,
) -> MonoType {
    match &*resolve(ty) {
        MonoType::Var(v) => var_map
            .get(&v.ptr_key())
            .cloned()
            .unwrap_or_else(|| MonoType::Var(v.clone())),
        MonoType::Base(b) => MonoType::Base(*b),
        MonoType::Func(row, a, b) => MonoType::Func(
            Box::new(substitute_row(&row, var_map, row_map)),
            Box::new(substitute(&a, var_map, row_map)),
            Box::new(substitute(&b, var_map, row_map)),
        ),
        MonoType::Product(ts) => {
            MonoType::Product(ts.iter().map(|t| substitute(t, var_map, row_map)).collect())
        }
        MonoType::List(t) => MonoType::List(Box::new(substitute(&t, var_map, row_map))),
        MonoType::Ref(t) => MonoType::Ref(Box::new(substitute(&t, var_map, row_map))),
        MonoType::Record(row) => MonoType::Record(substitute_row(&row, var_map, row_map)),
        MonoType::Variant(name, args) => MonoType::Variant(
            name.clone(),
            args.iter()
                .map(|t| substitute(t, var_map, row_map))
                .collect(),
        ),
        MonoType::InlineCmd(cs) => MonoType::InlineCmd(substitute_cmd_args(&cs, var_map, row_map)),
        MonoType::BlockCmd(cs) => MonoType::BlockCmd(substitute_cmd_args(&cs, var_map, row_map)),
        MonoType::MathCmd(cs) => MonoType::MathCmd(substitute_cmd_args(&cs, var_map, row_map)),
    }
}

pub(crate) fn substitute_row(
    row: &Row,
    var_map: &HashMap<usize, MonoType>,
    row_map: &HashMap<usize, Row>,
) -> Row {
    match &*resolve_row(row) {
        Row::Empty => Row::Empty,
        Row::Var(v) => row_map
            .get(&v.ptr_key())
            .cloned()
            .unwrap_or_else(|| Row::Var(v.clone())),
        Row::Cons(label, t, rest) => Row::Cons(
            label.clone(),
            Box::new(substitute(&t, var_map, row_map)),
            Box::new(substitute_row(&rest, var_map, row_map)),
        ),
    }
}

fn substitute_cmd_args(
    cs: &[CmdArgType],
    var_map: &HashMap<usize, MonoType>,
    row_map: &HashMap<usize, Row>,
) -> Vec<CmdArgType> {
    cs.iter()
        .map(|c| CmdArgType {
            optional: c.optional,
            opt_labels: c
                .opt_labels
                .iter()
                .map(|(l, t)| (l.clone(), substitute(t, var_map, row_map)))
                .collect(),
            ty: substitute(&c.ty, var_map, row_map),
        })
        .collect()
}

pub(crate) fn ptr_key(v: &TyVarRef) -> usize {
    v.ptr_key()
}

// ============================================================================
// Display — a SATySFi-syntax-ish pretty printer for error messages.
//
// This is intentionally not byte-for-byte identical to v0.0.6's own
// printer (`display.ml`); it exists to make unification error messages
// readable, and picks a simple, consistent parenthesization convention:
// atoms (base types, variables, records, argument-less variants) never
// need parens; `list`/`ref`/single-argument variants are postfix and only
// parenthesize a compound (function/product) operand; a function's
// codomain is parenthesized whenever it isn't itself an atom, which is why
// `int -> (string list)` prints with parens around the list even though
// `list` binds tighter than `->` (there's no source-level ambiguity here —
// the parens are purely for readability in error output).
// ============================================================================

struct VarNamer {
    names: HashMap<usize, String>,
    next: usize,
}

impl VarNamer {
    fn new() -> Self {
        VarNamer {
            names: HashMap::new(),
            next: 0,
        }
    }

    fn name_for(&mut self, key: usize) -> String {
        if let Some(n) = self.names.get(&key) {
            return n.clone();
        }
        let n = Self::letter(self.next);
        self.next += 1;
        self.names.insert(key, n.clone());
        n
    }

    fn letter(i: usize) -> String {
        let letter = (b'a' + (i % 26) as u8) as char;
        let suffix = i / 26;
        if suffix == 0 {
            format!("'{letter}")
        } else {
            format!("'{letter}{suffix}")
        }
    }
}

fn is_atomic(ty: &MonoType) -> bool {
    match ty {
        MonoType::Base(_) | MonoType::Var(_) | MonoType::Record(_) => true,
        MonoType::Variant(_, args) => args.is_empty(),
        MonoType::Func(_, _, _)
        | MonoType::Product(_)
        | MonoType::List(_)
        | MonoType::Ref(_)
        | MonoType::InlineCmd(_)
        | MonoType::BlockCmd(_)
        | MonoType::MathCmd(_) => false,
    }
}

/// Needs parens as the operand of a postfix constructor (`list`/`ref`/a
/// single-argument variant) or as an element of a product.
fn needs_parens_as_operand(ty: &MonoType) -> bool {
    matches!(ty, MonoType::Func(_, _, _) | MonoType::Product(_))
}

/// Print `ty` as a postfix/product/function-domain operand, wrapping it in
/// parens exactly when [`needs_parens_as_operand`] says a bare rendering
/// would misgroup.
fn fmt_operand(ty: &MonoType, f: &mut fmt::Formatter<'_>, namer: &mut VarNamer) -> fmt::Result {
    if needs_parens_as_operand(&resolve(ty)) {
        f.write_str("(")?;
        fmt_mono(ty, f, namer)?;
        f.write_str(")")
    } else {
        fmt_mono(ty, f, namer)
    }
}

fn fmt_mono(ty: &MonoType, f: &mut fmt::Formatter<'_>, namer: &mut VarNamer) -> fmt::Result {
    let ty = resolve(ty);
    match &*ty {
        MonoType::Var(v) => write!(f, "{}", namer.name_for(v.ptr_key())),
        MonoType::Base(b) => write!(f, "{b}"),
        MonoType::Func(row, dom, cod) => {
            // A row that resolves to bare `Empty` (every 0.0.6 function)
            // prints nothing extra, keeping every pinned 0.0.6 error-message
            // string byte-identical. A non-empty row prints `?(l : τ, …) `
            // before the domain (a free-var tail adds `| ?'rN`).
            fmt_func_row(row, f, namer)?;
            fmt_operand(dom, f, namer)?;
            f.write_str(" -> ")?;
            let rcod = resolve(cod);
            if is_atomic(&rcod) {
                fmt_mono(cod, f, namer)
            } else {
                f.write_str("(")?;
                fmt_mono(cod, f, namer)?;
                f.write_str(")")
            }
        }
        MonoType::Product(ts) => {
            for (i, t) in ts.iter().enumerate() {
                if i > 0 {
                    f.write_str(" * ")?;
                }
                fmt_operand(t, f, namer)?;
            }
            Ok(())
        }
        MonoType::List(t) => fmt_postfix(t, "list", f, namer),
        MonoType::Ref(t) => fmt_postfix(t, "ref", f, namer),
        MonoType::Record(row) => fmt_row(row, f, namer),
        MonoType::Variant(name, args) => match args.as_slice() {
            [] => write!(f, "{name}"),
            [one] => fmt_postfix(one, name, f, namer),
            many => {
                f.write_str("(")?;
                for (i, t) in many.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    fmt_mono(t, f, namer)?;
                }
                write!(f, ") {name}")
            }
        },
        MonoType::InlineCmd(cs) => fmt_cmd(cs, "inline-cmd", f, namer),
        MonoType::BlockCmd(cs) => fmt_cmd(cs, "block-cmd", f, namer),
        MonoType::MathCmd(cs) => fmt_cmd(cs, "math-cmd", f, namer),
    }
}

fn fmt_postfix(
    operand: &MonoType,
    suffix: &str,
    f: &mut fmt::Formatter<'_>,
    namer: &mut VarNamer,
) -> fmt::Result {
    fmt_operand(operand, f, namer)?;
    write!(f, " {suffix}")
}

fn fmt_cmd(
    cs: &[CmdArgType],
    suffix: &str,
    f: &mut fmt::Formatter<'_>,
    namer: &mut VarNamer,
) -> fmt::Result {
    f.write_str("[")?;
    for (i, c) in cs.iter().enumerate() {
        if i > 0 {
            f.write_str("; ")?;
        }
        fmt_opt_labels(&c.opt_labels, f, namer)?;
        fmt_mono(&c.ty, f, namer)?;
        if c.optional {
            f.write_str("?")?;
        }
    }
    write!(f, "] {suffix}")
}

/// Prefix-print a command argument slot's closed optional-label map (0.1's
/// `CmdArgType.opt_labels`): `?(l : τ, …) ` before the slot's mandatory `ty`,
/// or nothing at all when the map is empty (guaranteeing byte-identical
/// output for every 0.0.6-reachable `CmdArgType`, since those are always
/// `opt_labels == []`) — the command-type analogue of `fmt_func_row`, minus
/// the row-variable tail (command optional maps are closed, never open).
fn fmt_opt_labels(
    labels: &[(String, MonoType)],
    f: &mut fmt::Formatter<'_>,
    namer: &mut VarNamer,
) -> fmt::Result {
    if labels.is_empty() {
        return Ok(());
    }
    let mut fields = labels.to_vec();
    fields.sort_by(|a, b| a.0.cmp(&b.0));
    f.write_str("?(")?;
    for (i, (label, ty)) in fields.iter().enumerate() {
        if i > 0 {
            f.write_str(", ")?;
        }
        write!(f, "{label} : ")?;
        fmt_mono(ty, f, namer)?;
    }
    f.write_str(") ")
}

/// Prefix-print a function type's optional-argument row: nothing at all for
/// an empty (0.0.6) row — guaranteeing byte-identical output — or `?(l : τ,
/// …) ` (a free-var tail adding `| ?'rN`) for a non-empty 0.1 row.
fn fmt_func_row(row: &Row, f: &mut fmt::Formatter<'_>, namer: &mut VarNamer) -> fmt::Result {
    let mut fields: Vec<(String, MonoType)> = Vec::new();
    let mut cur = resolve_row(row).into_owned();
    let tail_name = loop {
        match cur {
            Row::Empty => break None,
            Row::Var(v) => break Some(namer.name_for(v.ptr_key())),
            Row::Cons(label, ty, rest) => {
                fields.push((label, *ty));
                cur = resolve_row(&rest).into_owned();
            }
        }
    };
    if fields.is_empty() && tail_name.is_none() {
        return Ok(());
    }
    fields.sort_by(|a, b| a.0.cmp(&b.0));
    f.write_str("?(")?;
    for (i, (label, ty)) in fields.iter().enumerate() {
        if i > 0 {
            f.write_str(", ")?;
        }
        write!(f, "{label} : ")?;
        fmt_mono(ty, f, namer)?;
    }
    if let Some(name) = tail_name {
        if !fields.is_empty() {
            f.write_str(" ")?;
        }
        write!(f, "| ?{name}")?;
    }
    f.write_str(") ")
}

fn fmt_row(row: &Row, f: &mut fmt::Formatter<'_>, namer: &mut VarNamer) -> fmt::Result {
    let mut fields: Vec<(String, MonoType)> = Vec::new();
    let mut cur = resolve_row(row).into_owned();
    let tail_name = loop {
        match cur {
            Row::Empty => break None,
            Row::Var(v) => break Some(namer.name_for(v.ptr_key())),
            Row::Cons(label, ty, rest) => {
                fields.push((label, *ty));
                cur = resolve_row(&rest).into_owned();
            }
        }
    };
    fields.sort_by(|a, b| a.0.cmp(&b.0));
    f.write_str("(| ")?;
    for (i, (label, ty)) in fields.iter().enumerate() {
        if i > 0 {
            f.write_str("; ")?;
        }
        write!(f, "{label} : ")?;
        fmt_mono(ty, f, namer)?;
    }
    if let Some(name) = tail_name {
        if !fields.is_empty() {
            f.write_str(" ")?;
        }
        write!(f, "| {name}")?;
    }
    f.write_str(" |)")
}

impl fmt::Display for MonoType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut namer = VarNamer::new();
        fmt_mono(self, f, &mut namer)
    }
}

impl fmt::Display for Row {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut namer = VarNamer::new();
        fmt_row(self, f, &mut namer)
    }
}
