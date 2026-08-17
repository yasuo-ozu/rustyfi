//! SATySFi 0.1 (`dev-0-1-0`) support, Slice 1: lowering the `cst_v1` CST
//! into the shared 0.0.6-shaped pipeline. Sub-slice 2d-1 adds
//! `static_env.rs`/`sig_subtype.rs`/`module_check.rs`: signature ascription
//! (width/depth `val` matching), sealing, and value hiding, enforced by
//! `module_check::check_program` — the V0_1 pipeline's per-binding
//! replacement for `typecheck::typecheck_with_version` (see `lib.rs`'s
//! `compile_document_v1_with_trials`).
pub mod lower;
pub(crate) mod module_check;
pub(crate) mod sig_subtype;
pub(crate) mod static_env;
