//! v0.0.6 / dev-0-1-0-Legacy resolution backend: `@require:`/`@import:`
//! header resolution against a lib-root (`resolve.rs`). Shared by BOTH `V0_0`
//! and `V0_1`-under-`LoadMode::Legacy` (dev-0-1-0's headers are byte-identical
//! to 0.0.6's minus `@stage:`, so this backend needs no per-version branch).
//! The dependency-graph toposort/cycle bookkeeping lives at the
//! crate root (`graph.rs`): it is mode-agnostic (`u32`/`PathBuf` only) and
//! the Envelopes backend (`v01x/`) needs the identical machinery.

pub mod resolve;
