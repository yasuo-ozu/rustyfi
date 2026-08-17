//! v0.0.6 / dev-0-1-0-Legacy resolution backend: `@require:`/`@import:`
//! header resolution against a lib-root (`resolve.rs`). A pure relocation
//! from the crate root (Ld1, `docs/plans/rustyfi-0-1-0-support.md` §2) — no
//! behavior change from what shipped before this module existed. Shared by
//! BOTH `V0_0` and `V0_1`-under-`LoadMode::Legacy` (dev-0-1-0's headers are
//! byte-identical to 0.0.6's minus `@stage:`, so this backend needs no
//! per-version branch — see Ld2). The dependency-graph toposort/cycle
//! bookkeeping that used to live here (`graph.rs`) was promoted to the crate
//! root in Ld3a — it is mode-agnostic (`u32`/`PathBuf` only) and the
//! Envelopes backend (`v01x/`) needs the identical machinery.

pub(crate) mod resolve;
