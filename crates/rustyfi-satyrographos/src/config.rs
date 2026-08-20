//! The user's own configuration — `config.toml` under the rustyfi config
//! directory.
//!
//! One thing lives here today: the default package repository, so `rustyfi
//! search KEYWORD` and `rustyfi install NAME` work in a directory that has no
//! project and no exported environment. A registry named here is the LAST
//! word, below the flag, the environment, and the project's own
//! `(registry …)` — a personal default must never quietly redirect a project
//! that states where its packages come from.
//!
//! ```toml
//! # ~/.config/rustyfi/config.toml
//! [registry]
//! url = "https://github.com/yasuo-ozu/rustyfi-registry"
//! kind = "git"                     # or "sparse"/"auto"
//! mirrors = ["https://mirror.example/registry"]
//! ```
//!
//! An absent file is not an error: it is the same as an empty one.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::error::Error;
use crate::source::RegistryConfig;
use crate::util;

/// The file name looked for inside the config directory.
pub const CONFIG_NAME: &str = "config.toml";

/// A parsed `config.toml`.
#[derive(Debug, Default, Clone, Deserialize)]
pub struct Config {
    /// The default package repositories. Written either as one `[registry]`
    /// table or as several `[[registry]]` ones; both land here.
    #[serde(default)]
    pub registry: Option<Registries>,
}

/// One `[registry]`, or a list of `[[registry]]`.
///
/// TOML spells the two differently while sharing the key, so this accepts
/// either shape rather than making a user choose the right punctuation for
/// how many repositories they happen to have.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum Registries {
    One(Box<RegistryConfig>),
    Many(Vec<RegistryConfig>),
}

impl Registries {
    pub fn as_slice(&self) -> &[RegistryConfig] {
        match self {
            Registries::One(one) => std::slice::from_ref(one),
            Registries::Many(many) => many,
        }
    }
}

impl Config {
    /// Every configured repository, in the order written — the order they are
    /// searched, and the order `install NAME` tries them in.
    pub fn registries(&self) -> &[RegistryConfig] {
        self.registry.as_ref().map(|r| r.as_slice()).unwrap_or(&[])
    }

    /// The first repository, for the single-registry paths.
    pub fn registry(&self) -> Option<&RegistryConfig> {
        self.registries().first()
    }

    pub fn registry_url(&self) -> Option<&str> {
        self.registry().and_then(|r| r.url.as_deref())
    }

    pub fn registry_mirrors(&self) -> &[String] {
        self.registry().map(|r| r.mirrors.as_slice()).unwrap_or(&[])
    }

    pub fn registry_kind(&self) -> Option<crate::source::RegistryKind> {
        self.registry().and_then(|r| r.kind)
    }
}

/// Every directory a config may live in, in precedence order:
///
/// 1. `$RUSTYFI_CONFIG_DIR` — one setup overriding everything, and what the
///    tests point somewhere harmless.
/// 2. `$XDG_CONFIG_HOME/rustyfi`, else `~/.config/rustyfi` — the user's own.
/// 3. `<prefix>/share/rustyfi` — the one SHIPPED beside the binary, found
///    relative to the executable so an unpacked archive carries its own
///    defaults wherever it sits.
///
/// The shipped file is last: a default that travelled with the program must
/// never win over a config its user wrote.
pub fn config_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(dir) = std::env::var_os("RUSTYFI_CONFIG_DIR") {
        dirs.push(PathBuf::from(dir));
    }
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        dirs.push(PathBuf::from(xdg).join("rustyfi"));
    } else if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        dirs.push(PathBuf::from(home).join(".config").join("rustyfi"));
    }
    if let Some(prefix) = crate::roots::exe_prefix() {
        dirs.push(prefix.join("share").join("rustyfi"));
    }
    dirs
}

/// The first config file that exists, if any.
pub fn config_path() -> Option<PathBuf> {
    config_dirs()
        .into_iter()
        .map(|dir| dir.join(CONFIG_NAME))
        .find(|p| p.is_file())
}

/// Read the user's config. An absent file yields the default; a malformed one
/// is an error, because silently ignoring a config someone wrote is worse than
/// telling them it is wrong.
pub fn load() -> Result<Config, Error> {
    match config_path() {
        Some(path) => read(&path),
        None => Ok(Config::default()),
    }
}

/// Read a config from an explicit path.
pub fn read(path: &Path) -> Result<Config, Error> {
    let text = util::read_to_string(path)?;
    toml::from_str(&text).map_err(|source| Error::Config {
        path: path.to_path_buf(),
        source,
    })
}
