//! The user's `config.toml`: where it is looked for, and what it may declare.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use rustyfi_satyrographos as sg;

fn tmp(tag: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let p = std::env::temp_dir().join(format!(
        "rustyfi-config-{tag}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(&p).expect("temp dir");
    p
}

#[test]
fn reads_a_default_repository() {
    let dir = tmp("registry");
    fs::write(
        dir.join("config.toml"),
        "[registry]\nurl = \"https://example.invalid/registry\"\nkind = \"sparse\"\n\
         mirrors = [\"https://mirror.invalid/registry\"]\n",
    )
    .unwrap();
    let cfg = sg::config::read(&dir.join("config.toml")).expect("parse");
    assert_eq!(cfg.registry_url(), Some("https://example.invalid/registry"));
    assert_eq!(cfg.registry_mirrors(), ["https://mirror.invalid/registry"]);
    assert_eq!(cfg.registry_kind(), Some(sg::RegistryKind::Sparse));
}

#[test]
fn an_absent_file_is_an_empty_config_not_an_error() {
    let dir = tmp("absent");
    // `load()` looks in the configured directory; point it at an empty one.
    let prev = std::env::var_os("RUSTYFI_CONFIG_DIR");
    std::env::set_var("RUSTYFI_CONFIG_DIR", &dir);
    let cfg = sg::config::load().expect("an absent config is not an error");
    match prev {
        Some(v) => std::env::set_var("RUSTYFI_CONFIG_DIR", v),
        None => std::env::remove_var("RUSTYFI_CONFIG_DIR"),
    }
    assert!(cfg.registry_url().is_none());
}

#[test]
fn a_malformed_file_is_reported_rather_than_ignored() {
    // Silently ignoring a config someone wrote is worse than saying it is
    // wrong — they would see the tool behave as if the file did not exist.
    let dir = tmp("malformed");
    fs::write(dir.join("config.toml"), "[registry]\nurl = 42\n").unwrap();
    let err = sg::config::read(&dir.join("config.toml")).expect_err("should report");
    assert!(err.to_string().contains("config.toml"), "{err}");
    assert!(
        err.to_string().contains("invalid config"),
        "a config is not a manifest: {err}"
    );
}

#[test]
fn several_repositories_are_read_in_order() {
    let dir = tmp("many");
    fs::write(
        dir.join("config.toml"),
        "[[registry]]\nurl = \"https://first.invalid\"\n\n\
         [[registry]]\nurl = \"https://second.invalid\"\nkind = \"sparse\"\n",
    )
    .unwrap();
    let cfg = sg::config::read(&dir.join("config.toml")).expect("parse");
    let repos = cfg.registries();
    assert_eq!(repos.len(), 2);
    assert_eq!(repos[0].url.as_deref(), Some("https://first.invalid"));
    assert_eq!(repos[1].url.as_deref(), Some("https://second.invalid"));
    assert_eq!(repos[1].kind, Some(sg::RegistryKind::Sparse));
    // The single-registry paths take the first.
    assert_eq!(cfg.registry_url(), Some("https://first.invalid"));
}

#[test]
fn one_registry_table_still_works() {
    // `[registry]` and `[[registry]]` share a key but not a shape; a user
    // with one repository should not have to write it as a list.
    let dir = tmp("one-shape");
    fs::write(
        dir.join("config.toml"),
        "[registry]\nurl = \"https://only.invalid\"\n",
    )
    .unwrap();
    let cfg = sg::config::read(&dir.join("config.toml")).expect("parse");
    assert_eq!(cfg.registries().len(), 1);
    assert_eq!(cfg.registry_url(), Some("https://only.invalid"));
}
