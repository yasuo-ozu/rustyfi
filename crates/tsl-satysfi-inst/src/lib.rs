//! satysfi instantiation of `tsl-infer` and `tsl-exhaust`.
//!
//! ADOPTION: satysfi's own type/unification engine is absorbed as
//! `tsl-infer`'s seed (a one-time flow satysfi -> library, see
//! `tsl-infer`'s module doc), and this crate is where satysfi flows back
//! to depend on the now-generic library engine. Command types (`InlineCmd`
//! / `BlockCmd` / `MathCmd`) carry their argument optionality flags
//! directly in the `SatysfiFormer` tag (`Vec<bool>`, one entry per
//! parameter, `true` = optional) rather than in a separate side table.
#![forbid(unsafe_code)]

/// A satysfi type-constructor former.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SatysfiFormer {
    /// A named base type.
    Base(String),
    /// The function type constructor.
    Func,
    /// An n-ary product (tuple) type constructor.
    Product(usize),
    /// The list type constructor.
    List,
    /// The reference type constructor.
    Ref,
    /// A variant constructor, named, with its argument arity.
    Variant(String, usize),
    /// An inline command type; one optionality flag per parameter.
    InlineCmd(Vec<bool>),
    /// A block command type; one optionality flag per parameter.
    BlockCmd(Vec<bool>),
    /// A math command type; one optionality flag per parameter.
    MathCmd(Vec<bool>),
}

/// The kind annotation carried by a satysfi unification variable. Left
/// unit-shaped for now: satysfi's kind system (if any beyond simple types)
/// is not yet modeled.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SatysfiVarKind;

/// The satysfi instantiation of `tsl_infer::TypeLang`.
pub struct SatysfiLang;

impl tsl_infer::TypeLang for SatysfiLang {
    type Former = SatysfiFormer;
    type VarKind = SatysfiVarKind;

    fn unify_former(a: &Self::Former, b: &Self::Former) -> Result<(), tsl_infer::FormerClash> {
        use SatysfiFormer::*;
        match (a, b) {
            (Base(x), Base(y)) if x == y => Ok(()),
            (Func, Func) => Ok(()),
            (Product(x), Product(y)) if x == y => Ok(()),
            (List, List) => Ok(()),
            (Ref, Ref) => Ok(()),
            (Variant(x, ax), Variant(y, ay)) if x == y && ax == ay => Ok(()),
            (InlineCmd(x), InlineCmd(y)) if x == y => Ok(()),
            (BlockCmd(x), BlockCmd(y)) if x == y => Ok(()),
            (MathCmd(x), MathCmd(y)) if x == y => Ok(()),
            _ => Err(tsl_infer::FormerClash),
        }
    }

    fn meet_kinds(
        a: &Self::VarKind,
        b: &Self::VarKind,
    ) -> Result<Self::VarKind, tsl_infer::KindClash> {
        let _ = (a, b);
        // Only one kind exists today, so the meet is trivial; this is the
        // hook where a real kind lattice would be consulted.
        Ok(SatysfiVarKind)
    }
}

/// The satysfi instantiation of `tsl_exhaust::PatSig`, used to check
/// exhaustiveness/usefulness of satysfi's variant pattern matches.
pub struct SatysfiPatSig;

impl tsl_exhaust::PatSig for SatysfiPatSig {
    /// A variant constructor name (or a literal tag).
    type Ctor = String;
    /// A type head name, used to look up its complete constructor set.
    type TyTag = String;

    fn arity(&self, c: &Self::Ctor) -> usize {
        let _ = c;
        todo!("look up variant arity from the satysfi type environment")
    }

    fn complete(&self, head: &Self::TyTag) -> Option<Vec<Self::Ctor>> {
        let _ = head;
        todo!("look up the complete variant set for a satysfi sum type")
    }
}
