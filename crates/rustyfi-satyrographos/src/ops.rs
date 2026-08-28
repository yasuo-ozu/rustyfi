//! The package-manager operations, each a thin orchestration over
//! `roots`/`archive`/`manifest`/`stage`/`receipts`.

pub mod build;
pub mod prepare;
pub mod install;
pub mod list;
pub mod publish;
pub mod reconcile;
pub mod registry_install;
pub mod search;
pub mod status;
pub mod uninstall;
pub mod update;
