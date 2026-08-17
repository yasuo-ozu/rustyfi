//! saphe-split / "Envelopes" resolution backend (`LoadMode::Envelopes`):
//! `use package` / `use … of` headers resolved against local files and
//! (Ld3b) a pre-solved `satysfi-deps.yaml` envelope graph. Parallels `v006/`
//! (the Legacy backend) exactly as `docs/plans/satysfi-0-1-0-support.md`
//! §1.5's crate map lays out. Transcribed from `saphe-split @ b836d512`,
//! `src/frontend/openFileDependencyResolver.ml` (`open_doc`) — see the Ld3a
//! spec §0 for the full citation table.
//!
//! Ld3a ships only `open_doc` (the open/document resolver: `use … of` local
//! resolution + typed errors for `use package` / bare `use`). The Ld3b
//! siblings (`deps.rs` ≈ `depsConfig.ml`, `envelope.rs` ≈ `envelopeReader.ml`,
//! `closed.rs` ≈ `closedFileDependencyResolver.ml`) are NOT built here — every
//! path that would need them returns a named [`crate::LoadError`] pointing at
//! Ld3b, so Ld3a commits to ZERO YAML/manifest formats.

pub(crate) mod open_doc;
