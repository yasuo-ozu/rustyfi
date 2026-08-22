//! SATySFi 0.1 (`dev-0-1-0`) support: lowering the `cst_v1` CST into the
//! shared 0.0.6-shaped pipeline (`lower.rs`), plus the module system.
//!
//! `static_env.rs`/`sig_subtype.rs`/`module_check.rs` implement signature
//! ascription (width/depth `val` matching), sealing and value hiding,
//! enforced by `module_check::check_program` — the V0_1 pipeline's
//! per-binding replacement for `typecheck::typecheck_with_version` (see
//! `lib.rs`'s `compile_document_v1_with_trials`).
//!
//! See `module_check.rs`'s doc comment for the phase-A-through-D algorithm,
//! `surface.rs`'s for the syntactic surface/signature table both `lower.rs`
//! and `module_check.rs` consult, and `xver_adapt.rs`'s for the
//! cross-version import boundary.
pub(crate) mod functor;
pub mod lower;
pub(crate) mod module_check;
pub(crate) mod sig_subtype;
pub(crate) mod static_env;
pub(crate) mod surface;
pub(crate) mod xver_adapt;
