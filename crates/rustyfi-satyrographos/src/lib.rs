//! Satyrographos-style package management for the `rustyfi` port.
//!
//! This crate is the clap-free library half of the chimera CLI: given an
//! options struct and a source path, it materialises SATySFi packages under
//! `<lib_root>/dist/{packages,fonts,hash,md}/` (mirroring real
//! Satyrographos' layout) and records per-package receipts under
//! `<lib_root>/.satyrographos/receipts/`, so the thin `rustyfi` shell
//! only parses arguments and calls in here.
//!
//! Sources may be a directory, a `.tar.gz`/`.tgz` archive, a `{ registry =
//! … }`/`{ git = … }` reference, or a real (OCaml) Satyrographos
//! `Satyristes` — all funnel through the same install pipeline
//! (`ops::install`) and the same atomic stage-then-swap transaction
//! (`stage`). `ops::reconcile::install_manifest` diffs a project's
//! `Satyristes`/`Satyristes.lock` against installed receipts and
//! re-materialises only changed/missing entries. The registry fetch
//! verifies SHA-256 before touching `dist/`, and `--offline`/
//! `$RUSTYFI_OFFLINE` turn any would-be network request into a clean
//! [`error::Error::Offline`] instead of a silent fetch (slice S2, with
//! `cache`'s content-addressed archive cache in front of the fetch);
//! `{ git = … }` sources are slice S3. The HTTP transport is behind the
//! `http` cargo feature, default-on (slice S1); `--no-default-features`
//! builds a pure-offline embedder with no HTTP client compiled in.
//!
//! System fonts are out of scope here.

mod archive;
pub mod cache;
pub mod config;
pub mod error;
pub mod hashfile;
pub mod lockfile;
pub mod manifest;
pub mod opam;
pub mod ops;
pub mod receipts;
pub mod registry;
pub mod roots;
pub mod satyristes;
pub mod source;
pub mod solve;
mod stage;
mod util;
pub mod version;

pub use config::{Config, Registries};
pub use error::Error;
pub use manifest::Lang;

pub use ops::build::{build, BuildOptions, BuildReport};
pub use ops::install::{install, install_url, is_url, InstallOptions, InstallReport};
pub use ops::list::{list, PackageSummary};
pub use ops::reconcile::{install_manifest, install_manifest_reg, install_manifest_reg_multi, ManifestReport};
pub use ops::status::{status, PackageStatus, StatusReport};
pub use ops::uninstall::{uninstall, RootOptions};

pub use lockfile::{LockEntry, Lockfile};
pub use satyristes::{find_upward, LibraryMeta, Project};
pub use source::{LibraryEntry, RegistryConfig, RegistryKind, SourceKind, SourceSpec};

pub use ops::prepare::PrepareReport;
pub use ops::publish::{
    publish, publish_with_prompt, OpamFields, OpamPrompt, PublishOptions, PublishReport,
    RepoShape,
};
pub use ops::registry_install::{install_registry, Resolved};
pub use ops::search::{search, SearchHit};
pub use ops::update::{update, update_multi, Upgrade, UpdateReport};
pub use registry::{AcquiredRepo, Registry, RegistryOptions};

pub use solve::{solve, DepSource, Solution};
pub use version::{Constraint, Version};
