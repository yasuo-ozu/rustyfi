//! v0.0.6 / dev-0-1-0-Legacy resolution backend: `@require:`/`@import:`
//! header resolution against a lib-root (`resolve.rs`), plus dependency-
//! graph toposort/cycle-detection bookkeeping (`graph.rs`). Both modules
//! are pure relocations from the crate root (Ld1,
//! `docs/plans/satysfi-0-1-0-support.md` §2) — no behavior change from
//! what shipped before this module existed. Shared by BOTH `V0_0_6` and
//! `V0_1`-under-`LoadMode::Legacy` (dev-0-1-0's headers are byte-identical
//! to 0.0.6's minus `@stage:`, so this backend needs no per-version
//! branch — see Ld2 below).

pub(crate) mod graph;
pub(crate) mod resolve;
