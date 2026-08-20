//! Reading an OPAM repository — `packages/<name>/<name>.<version>/opam` — as a
//! package index, which is the shape Satyrographos' own registry has.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use rustyfi_satyrographos as sg;

fn tmp(tag: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let p = std::env::temp_dir().join(format!(
        "rustyfi-opamreg-{tag}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(&p).expect("temp dir");
    p
}

/// An index shaped like Satyrographos': one directory per package, one per
/// version, each holding an `opam` whose `url {}` names the source.
fn opam_index(dir: &PathBuf) {
    let pkg = dir.join("packages/satysfi-xpath/satysfi-xpath.0.3.0");
    fs::create_dir_all(&pkg).unwrap();
    fs::write(
        pkg.join("opam"),
        "opam-version: \"2.0\"\n\
         synopsis: \"XPath-like path combinators\"\n\
         url {\n  \
           src: \"https://example.invalid/xpath-0.3.0.tar.gz\"\n  \
           checksum: [\n    \
             \"md5=8e3c25e773e1ba012fafb0a59f552f47\"\n    \
             \"sha512=b7bea1d05b4019d6314ef17e036bd543aa6238ddf6ab3cd58d975a7293e4df71\"\n  \
           ]\n\
         }\n",
    )
    .unwrap();

    // An older version, and a `conf-` package with no source at all — an OPAM
    // repository holds entries that cannot be installed.
    let old = dir.join("packages/satysfi-xpath/satysfi-xpath.0.2.0");
    fs::create_dir_all(&old).unwrap();
    fs::write(
        old.join("opam"),
        "opam-version: \"2.0\"\nurl {\n  src: \"https://example.invalid/xpath-0.2.0.tar.gz\"\n  \
         checksum: [ \"sha512=aaaa\" ]\n}\n",
    )
    .unwrap();
    let conf = dir.join("packages/conf-yarn/conf-yarn.0.1");
    fs::create_dir_all(&conf).unwrap();
    fs::write(conf.join("opam"), "opam-version: \"2.0\"\nbuild: [[\"yarn\" \"--version\"]]\n").unwrap();
}

fn opts(index: &PathBuf) -> sg::RegistryOptions {
    sg::RegistryOptions {
        url: Some(index.display().to_string()),
        ..Default::default()
    }
}

#[test]
fn a_library_name_resolves_through_the_satysfi_prefix() {
    // Satyrographos publishes library `xpath` as package `satysfi-xpath`.
    let dir = tmp("prefix");
    opam_index(&dir);
    let resolved = sg::ops::registry_install::resolve("xpath", None, &opts(&dir), None)
        .expect("should resolve through the prefix");
    assert_eq!(resolved.version, "0.3.0", "the newest version wins");
    assert!(resolved.url.ends_with("xpath-0.3.0.tar.gz"));
    assert!(
        resolved.sha256.is_empty() && resolved.sha512.is_some(),
        "this index publishes sha512, not sha256: {resolved:?}"
    );
}

#[test]
fn search_skips_entries_it_cannot_install() {
    // `conf-yarn` has no source; one such entry must not sink the search.
    let dir = tmp("search");
    opam_index(&dir);
    let hits = sg::search(&["xpath"], &opts(&dir), None).expect("search");
    assert_eq!(hits.len(), 1, "{hits:?}");
    // `name` is what `install` accepts, not the registry's raw opam id — see
    // `search_result_name_is_what_install_accepts` below for the round-trip.
    assert_eq!(hits[0].name, "xpath");
    assert_eq!(hits[0].registry_name.as_deref(), Some("satysfi-xpath"));
    assert_eq!(hits[0].description.as_deref(), Some("XPath-like path combinators"));
}

#[test]
fn search_result_name_is_what_install_accepts() {
    // Copying a `search` hit's `name` straight into `install` must actually
    // work: `search` used to print the registry's raw opam id
    // (`satysfi-xpath`), which `install` also happens to accept, but is not
    // the name a user reaches for anywhere else (`@require:`, a `Satyristes`
    // dependency entry) — the two commands must agree on ONE name.
    let dir = tmp("roundtrip");
    opam_index(&dir);
    let hits = sg::search(&["xpath"], &opts(&dir), None).expect("search");
    assert_eq!(hits.len(), 1);
    let resolved = sg::ops::registry_install::resolve(&hits[0].name, None, &opts(&dir), None)
        .unwrap_or_else(|e| panic!("`install {}` should resolve: {e}", hits[0].name));
    assert_eq!(resolved.version, "0.3.0");
}

#[test]
fn a_checksum_keys_the_cache_by_whichever_digest_it_has() {
    // Keying by an empty sha256 would collide every sha512-only download onto
    // one cache entry.
    let only512 = sg::registry::Checksum::new("", Some("abc123"));
    assert_eq!(only512.key(), "abc123");
    let both = sg::registry::Checksum::new("deadbeef", Some("abc123"));
    assert_eq!(both.key(), "deadbeef", "sha256 wins when present");
}

#[test]
fn a_download_with_no_declared_checksum_is_refused() {
    let dir = tmp("nocheck");
    let file = dir.join("f");
    fs::write(&file, "x").unwrap();
    let err = sg::registry::Checksum::default()
        .verify(&file)
        .expect_err("nothing to verify against");
    assert!(matches!(err, sg::Error::ChecksumMismatch { .. }), "{err}");
}
