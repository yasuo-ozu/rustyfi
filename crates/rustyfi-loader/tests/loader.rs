//! Integration tests for the multi-file loader, driven against small
//! fixture trees built under a unique temp directory per test (cleaned up on
//! drop; no extra dev-dependencies needed beyond `std`).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use rustyfi_loader::{
    load, FileOrigin, LoadError, LoadMode, LoadOptions, LoadedProgram, RustyfiVersion,
};

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "rustyfi-loader-test-{tag}-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            n
        ));
        fs::create_dir_all(&path).expect("create temp dir");
        TempDir(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    /// Write `content` to `rel` (a `/`-separated path relative to the temp
    /// root), creating any needed parent directories. Returns the absolute
    /// path written.
    fn write(&self, rel: &str, content: &str) -> PathBuf {
        let p = self.0.join(rel);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).expect("create parent dir");
        }
        fs::write(&p, content).expect("write fixture file");
        p
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn no_lib_root() -> LoadOptions {
    LoadOptions {
        lib_root: None,
        ..Default::default()
    }
}

fn file_names(program: &LoadedProgram) -> Vec<String> {
    program
        .files
        .iter()
        .map(|f| f.path.file_name().unwrap().to_string_lossy().into_owned())
        .collect()
}

#[test]
fn single_doc_no_headers() {
    let dir = TempDir::new("single");
    let entry = dir.write("doc.saty", "let x = 1 in x");

    let program = load(&entry, &no_lib_root()).expect("load should succeed");

    assert_eq!(program.files.len(), 1);
    assert_eq!(program.files[0].path, fs::canonicalize(&entry).unwrap());
    assert!(program.files[0].cst.is_document());
}

#[test]
fn doc_plus_import_helper() {
    let dir = TempDir::new("import");
    dir.write("helper.satyh", "let helper = 42");
    let entry = dir.write("doc.saty", "@import: helper\nlet x = 1 in x");

    let program = load(&entry, &no_lib_root()).expect("load should succeed");

    assert_eq!(program.files.len(), 2);
    assert_eq!(file_names(&program), vec!["helper.satyh", "doc.saty"]);
}

#[test]
fn extension_preference_prefers_satyh_over_satyg() {
    let dir = TempDir::new("extpref");
    dir.write("helper.satyh", "let helper = 1");
    dir.write("helper.satyg", "let helper = 2");
    let entry = dir.write("doc.saty", "@import: helper\nlet x = 1 in x");

    let program = load(&entry, &no_lib_root()).expect("load should succeed");

    assert_eq!(program.files.len(), 2);
    assert!(program.files[0]
        .path
        .to_string_lossy()
        .ends_with("helper.satyh"));
}

#[test]
fn diamond_dedups_shared_dependency_and_orders_it_first() {
    let dir = TempDir::new("diamond");
    dir.write("base.satyh", "let base = 1");
    dir.write("a.satyh", "@import: base\nlet a = 1");
    dir.write("b.satyh", "@import: base\nlet b = 1");
    let entry = dir.write("doc.saty", "@import: a\n@import: b\nlet x = 1 in x");

    let program = load(&entry, &no_lib_root()).expect("load should succeed");

    assert_eq!(program.files.len(), 4, "base must be deduplicated to one node");
    let names = file_names(&program);
    assert_eq!(
        names.iter().filter(|n| n.as_str() == "base.satyh").count(),
        1
    );
    let base_pos = names.iter().position(|n| n == "base.satyh").unwrap();
    let a_pos = names.iter().position(|n| n == "a.satyh").unwrap();
    let b_pos = names.iter().position(|n| n == "b.satyh").unwrap();
    let doc_pos = names.iter().position(|n| n == "doc.saty").unwrap();
    assert!(base_pos < a_pos, "base must load before a");
    assert!(base_pos < b_pos, "base must load before b");
    assert_eq!(doc_pos, names.len() - 1, "the entry document must be last");
}

#[test]
fn require_resolves_against_lib_root_dist_packages() {
    let dir = TempDir::new("require");
    let lib_root = dir.path().join("lib");
    fs::create_dir_all(lib_root.join("dist").join("packages")).unwrap();
    fs::write(
        lib_root.join("dist").join("packages").join("stdlib.satyh"),
        "let s = 1",
    )
    .unwrap();
    let entry = dir.write("doc.saty", "@require: stdlib\nlet x = 1 in x");

    let opts = LoadOptions {
        lib_root: Some(lib_root),
        ..Default::default()
    };
    let program = load(&entry, &opts).expect("load should succeed");

    assert_eq!(program.files.len(), 2);
    assert_eq!(file_names(&program), vec!["stdlib.satyh", "doc.saty"]);
}

// ============================================================================
// Slice X4a (docs/plans/design-cross-version-import.md §"Slice X4 — reverse
// direction"): a V0_0_6-rooted load's `@require:` reaches the 0.1 corpus
// `dist-v01/packages/`, and the resolved target is tagged V0_1 (the Q4-mirror
// rule) even though its `module …` head gives `sniff_version` no signal.
// ============================================================================

#[test]
fn require_resolves_against_lib_root_dist_v01_packages() {
    let dir = TempDir::new("require-v01");
    let lib_root = dir.path().join("lib");
    fs::create_dir_all(lib_root.join("dist-v01").join("packages")).unwrap();
    fs::write(
        lib_root.join("dist-v01").join("packages").join("v01lib.satyh"),
        "module V01Lib = struct\n  val x = 1\nend\n",
    )
    .unwrap();
    let entry = dir.write("doc.saty", "@require: v01lib\nlet x = 1 in x");

    let opts = LoadOptions {
        lib_root: Some(lib_root),
        version: RustyfiVersion::V0_0_6,
        ..Default::default()
    };
    let program = load(&entry, &opts).expect("load should succeed");

    assert_eq!(program.files.len(), 2);
    assert_eq!(file_names(&program), vec!["v01lib.satyh", "doc.saty"]);
    assert!(
        matches!(program.files[0].version, RustyfiVersion::V0_1),
        "a dist-v01/packages/ @require: target must be tagged V0_1 (Q4-mirror), \
         even though its `module ..` head gives sniff_version no signal"
    );
    assert!(
        matches!(program.files[0].cst, rustyfi_loader::LoadedCst::V0_1(_)),
        "a V0_1-tagged file must carry a V0_1-parsed cst"
    );
    assert!(
        matches!(program.files[1].version, RustyfiVersion::V0_0_6),
        "the V0_0_6 entry must stay V0_0_6"
    );
}

/// A pure-0.0.6 load with NO `dist-v01/packages/` target must be completely
/// unaffected by the X4a Q4-mirror rule — every node stays `V0_0_6`, exactly
/// as `require_resolves_against_lib_root_dist_packages` above already pins,
/// re-asserted here with the `version` field explicit (the non-regression
/// invariant the X4a task brief calls out by name).
#[test]
fn pure_v006_load_is_unaffected_by_the_v01_q4_mirror_rule() {
    let dir = TempDir::new("require-pure-v006");
    let lib_root = dir.path().join("lib");
    fs::create_dir_all(lib_root.join("dist").join("packages")).unwrap();
    fs::write(
        lib_root.join("dist").join("packages").join("stdlib.satyh"),
        "let s = 1",
    )
    .unwrap();
    let entry = dir.write("doc.saty", "@require: stdlib\nlet x = 1 in x");

    let opts = LoadOptions {
        lib_root: Some(lib_root),
        version: RustyfiVersion::V0_0_6,
        ..Default::default()
    };
    let program = load(&entry, &opts).expect("load should succeed");

    for f in &program.files {
        assert_eq!(f.version, RustyfiVersion::V0_0_6, "{:?}", f.path);
        assert!(matches!(f.cst, rustyfi_loader::LoadedCst::V0_0_6(_)));
    }
}

#[test]
fn mutual_import_cycle_is_reported_naming_both_files() {
    let dir = TempDir::new("cycle");
    dir.write("a.satyh", "@import: b\nlet a = 1");
    dir.write("b.satyh", "@import: a\nlet b = 1");
    let entry = dir.write("doc.saty", "@import: a\nlet x = 1 in x");

    let err = load(&entry, &no_lib_root()).expect_err("cycle must be rejected");

    match err {
        LoadError::Cycle { chain } => {
            let names: Vec<String> = chain
                .iter()
                .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
                .collect();
            assert!(names.iter().any(|n| n == "a.satyh"), "chain: {names:?}");
            assert!(names.iter().any(|n| n == "b.satyh"), "chain: {names:?}");
        }
        other => panic!("expected LoadError::Cycle, got {other:?}"),
    }
}

#[test]
fn importing_a_document_as_a_dependency_is_rejected() {
    let dir = TempDir::new("docdep");
    // `.satyh` extension, but its content is a document (has an `in` body) —
    // libraries must not be documents.
    dir.write("baddep.satyh", "let z = 1 in z");
    let entry = dir.write("doc.saty", "@import: baddep\nlet x = 1 in x");

    let err = load(&entry, &no_lib_root()).expect_err("document dependency must be rejected");

    match err {
        LoadError::DocumentAsDependency { path } => {
            assert!(path.to_string_lossy().ends_with("baddep.satyh"));
        }
        other => panic!("expected LoadError::DocumentAsDependency, got {other:?}"),
    }
}

#[test]
fn a_library_entry_is_rejected() {
    let dir = TempDir::new("libentry");
    // No `in ...` body: this is library-shaped, not document-shaped.
    let entry = dir.write("entry.saty", "let x = 1");

    let err = load(&entry, &no_lib_root()).expect_err("library entry must be rejected");

    match err {
        LoadError::LibraryAsEntry { path } => {
            assert!(path.to_string_lossy().ends_with("entry.saty"));
        }
        other => panic!("expected LoadError::LibraryAsEntry, got {other:?}"),
    }
}

#[test]
fn unresolved_import_lists_the_candidates_it_searched() {
    let dir = TempDir::new("unresolved");
    let entry = dir.write("doc.saty", "@import: missing\nlet x = 1 in x");

    let err = load(&entry, &no_lib_root()).expect_err("missing import must fail");

    match err {
        LoadError::UnresolvedImport {
            name, searched, ..
        } => {
            assert_eq!(name, "missing");
            assert_eq!(searched.len(), 2);
            assert!(searched[0].to_string_lossy().ends_with("missing.satyh"));
            assert!(searched[1].to_string_lossy().ends_with("missing.satyg"));
        }
        other => panic!("expected LoadError::UnresolvedImport, got {other:?}"),
    }
}

#[test]
fn import_resolves_relative_to_the_containing_file_not_the_entry_dir() {
    let dir = TempDir::new("relimport");
    // A decoy at the entry document's directory: must NOT be picked.
    dir.write("common.satyh", "let common = 0");
    // The real target, next to the file that imports it.
    dir.write("sub/common.satyh", "let common = 1");
    dir.write("sub/inner.satyh", "@import: common\nlet inner = 1");
    let entry = dir.write("doc.saty", "@import: sub/inner\nlet x = 1 in x");

    let program = load(&entry, &no_lib_root()).expect("load should succeed");

    assert_eq!(program.files.len(), 3, "the decoy common.satyh must not be loaded");
    let common_file = program
        .files
        .iter()
        .find(|f| f.path.file_name().unwrap() == "common.satyh")
        .expect("sub/common.satyh must be loaded");
    assert!(
        common_file.path.parent().unwrap().ends_with("sub"),
        "expected sub/common.satyh, got {}",
        common_file.path.display()
    );
}

// `unimplemented_version_is_rejected_before_touching_disk` (formerly here)
// deleted at the 0.1.0 Slice 1 finale's flip commit
// (v1-finale-spec.md §7 item 2): its premise — `RustyfiVersion::V0_1` is
// gated out of `is_implemented()` — is now false, and `RustyfiVersion` has
// exactly two variants, so no unimplemented version remains to substitute.
// `LoadError::UnsupportedVersion` itself stays alive (`lib.rs`'s
// `is_implemented()` guard in `load()`) for any future third generation.

#[test]
fn default_load_options_targets_v0_0_6() {
    assert_eq!(LoadOptions::default().version, RustyfiVersion::V0_0_6);
}

// ---------------------------------------------------------------------------
// Ld3a: LoadMode::Envelopes (the saphe-split `use … of` open resolver).
// ---------------------------------------------------------------------------

/// Envelopes mode, 0.1, no deps config (the Ld3a happy path).
fn envelopes_v01() -> LoadOptions {
    LoadOptions {
        version: RustyfiVersion::V0_1,
        mode: LoadMode::Envelopes { deps: None },
        ..Default::default()
    }
}

#[test]
fn envelopes_use_of_local_chain() {
    let dir = TempDir::new("env-chain");
    dir.write("local.satyh", "module Local = struct\nval x = 1\nend");
    let entry = dir.write("doc.saty", "use open Local of `./local`\nlet x = 1 in x");

    let program = load(&entry, &envelopes_v01()).expect("envelopes load should succeed");

    assert_eq!(file_names(&program), vec!["local.satyh", "doc.saty"]);
    assert!(program.files[1].cst.is_document(), "entry is a document");
    assert!(!program.files[0].cst.is_document(), "dependency is a library");
}

#[test]
fn envelopes_use_of_transitive_dedup() {
    let dir = TempDir::new("env-dedup");
    dir.write("b.satyh", "module B = struct\nval b = 1\nend");
    dir.write(
        "a.satyh",
        "use open B of `./b`\nmodule A = struct\nval a = 1\nend",
    );
    let entry = dir.write(
        "doc.saty",
        "use open A of `./a`\nuse open B of `./b`\nlet x = 1 in x",
    );

    let program = load(&entry, &envelopes_v01()).expect("envelopes load should succeed");

    let names = file_names(&program);
    assert_eq!(
        names.iter().filter(|n| n.as_str() == "b.satyh").count(),
        1,
        "b must be deduplicated: {names:?}"
    );
    let b_pos = names.iter().position(|n| n == "b.satyh").unwrap();
    let a_pos = names.iter().position(|n| n == "a.satyh").unwrap();
    let doc_pos = names.iter().position(|n| n == "doc.saty").unwrap();
    assert!(b_pos < a_pos, "b before a: {names:?}");
    assert_eq!(doc_pos, names.len() - 1, "entry document last: {names:?}");
}

#[test]
fn envelopes_use_of_prefers_satyh_over_satyg() {
    let dir = TempDir::new("env-extpref");
    dir.write("local.satyh", "module Local = struct\nval x = 1\nend");
    dir.write("local.satyg", "module Local = struct\nval x = 2\nend");
    let entry = dir.write("doc.saty", "use open Local of `./local`\nlet x = 1 in x");

    let program = load(&entry, &envelopes_v01()).expect("envelopes load should succeed");

    assert!(program.files[0]
        .path
        .to_string_lossy()
        .ends_with("local.satyh"));
}

#[test]
fn envelopes_unresolved_use_of_lists_candidates() {
    let dir = TempDir::new("env-unresolved");
    let entry = dir.write("doc.saty", "use open Missing of `./missing`\nlet x = 1 in x");

    let err = load(&entry, &envelopes_v01()).expect_err("missing use…of must fail");

    match err {
        LoadError::UnresolvedUseOf {
            relpath, searched, ..
        } => {
            assert_eq!(relpath, "./missing");
            assert_eq!(searched.len(), 2);
            assert!(searched[0].to_string_lossy().ends_with("missing.satyh"));
            assert!(searched[1].to_string_lossy().ends_with("missing.satyg"));
        }
        other => panic!("expected LoadError::UnresolvedUseOf, got {other:?}"),
    }
}

#[test]
fn envelopes_use_of_cycle_detected() {
    let dir = TempDir::new("env-cycle");
    dir.write(
        "a.satyh",
        "use open B of `./b`\nmodule A = struct\nval a = 1\nend",
    );
    dir.write(
        "b.satyh",
        "use open A of `./a`\nmodule B = struct\nval b = 1\nend",
    );
    let entry = dir.write("doc.saty", "use open A of `./a`\nlet x = 1 in x");

    let err = load(&entry, &envelopes_v01()).expect_err("cycle must be rejected");

    match err {
        LoadError::Cycle { chain } => {
            let names: Vec<String> = chain
                .iter()
                .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
                .collect();
            assert!(names.iter().any(|n| n == "a.satyh"), "chain: {names:?}");
            assert!(names.iter().any(|n| n == "b.satyh"), "chain: {names:?}");
        }
        other => panic!("expected LoadError::Cycle, got {other:?}"),
    }
}

#[test]
fn envelopes_use_package_errors_without_deps() {
    let dir = TempDir::new("env-usepkg");
    let entry = dir.write("doc.saty", "use package Stdlib\nlet x = 1 in x");

    let err = load(&entry, &envelopes_v01()).expect_err("use package without deps must fail");

    match err {
        LoadError::PackageDependencyUnresolved { module, .. } => {
            assert_eq!(module, "Stdlib");
        }
        other => panic!("expected LoadError::PackageDependencyUnresolved, got {other:?}"),
    }
}

#[test]
fn envelopes_bare_use_at_document_level_errors() {
    let dir = TempDir::new("env-bareuse");
    let entry = dir.write("doc.saty", "use Local\nlet x = 1 in x");

    let err = load(&entry, &envelopes_v01()).expect_err("bare use at document level must fail");

    match err {
        LoadError::BareUseOutsidePackage { module, .. } => {
            assert_eq!(module, "Local");
        }
        other => panic!("expected LoadError::BareUseOutsidePackage, got {other:?}"),
    }
}

#[test]
fn envelopes_rejects_v006() {
    // A nonexistent entry path proves the version/mode guard fires BEFORE any
    // filesystem access.
    let opts = LoadOptions {
        version: RustyfiVersion::V0_0_6,
        mode: LoadMode::Envelopes { deps: None },
        ..Default::default()
    };
    let err = load(Path::new("/nonexistent/never-read.saty"), &opts)
        .expect_err("V0_0_6 + Envelopes must be rejected");

    match err {
        LoadError::InvalidModeVersion { version } => {
            assert_eq!(version, RustyfiVersion::V0_0_6);
        }
        other => panic!("expected LoadError::InvalidModeVersion, got {other:?}"),
    }
}

#[test]
fn envelopes_deps_file_missing() {
    let dir = TempDir::new("env-deps-missing");
    let entry = dir.write("doc.saty", "let x = 1 in x");
    let opts = LoadOptions {
        version: RustyfiVersion::V0_1,
        mode: LoadMode::Envelopes {
            deps: Some(dir.path().join("does-not-exist.yaml")),
        },
        ..Default::default()
    };

    let err = load(&entry, &opts).expect_err("a missing deps file must error");
    assert!(
        matches!(err, LoadError::DepsConfigNotFound { .. }),
        "got {err:?}"
    );
}

#[test]
fn envelopes_deps_file_invalid() {
    let dir = TempDir::new("env-deps-invalid");
    let entry = dir.write("doc.saty", "let x = 1 in x");
    // Missing the required `test_dependencies` key.
    let deps = dir.write("rustyfi-deps.yaml", "envelopes: []\ndependencies: []\n");
    let opts = LoadOptions {
        version: RustyfiVersion::V0_1,
        mode: LoadMode::Envelopes { deps: Some(deps) },
        ..Default::default()
    };

    let err = load(&entry, &opts).expect_err("an invalid deps file must error");
    assert!(
        matches!(err, LoadError::DepsConfigDecode { .. }),
        "got {err:?}"
    );
}

#[test]
fn legacy_rejects_use_headers_with_mode_hint() {
    let dir = TempDir::new("legacy-use");
    // V0_1 grammar, but the DEFAULT (Legacy) mode: a `use` header is a typed
    // mode error, not the old parse error.
    let entry = dir.write("doc.saty", "use package X\nlet x = 1 in x");
    let opts = LoadOptions {
        version: RustyfiVersion::V0_1,
        mode: LoadMode::Legacy,
        ..Default::default()
    };

    let err = load(&entry, &opts).expect_err("use header under Legacy must fail");

    match err {
        LoadError::EnvelopeHeaderUnderLegacy { header, .. } => {
            assert!(header.contains("use package X"), "header: {header}");
        }
        other => panic!("expected LoadError::EnvelopeHeaderUnderLegacy, got {other:?}"),
    }
}

#[test]
fn envelopes_rejects_require_headers() {
    let dir = TempDir::new("env-require");
    let entry = dir.write("doc.saty", "@require: foo\nlet x = 1 in x");

    let err = load(&entry, &envelopes_v01()).expect_err("@require under Envelopes must fail");

    match err {
        LoadError::LegacyHeaderUnderEnvelopes { header, .. } => {
            assert!(header.contains("@require: foo"), "header: {header}");
        }
        other => panic!("expected LoadError::LegacyHeaderUnderEnvelopes, got {other:?}"),
    }
}

#[test]
fn default_mode_is_legacy() {
    assert_eq!(LoadOptions::default().mode, LoadMode::Legacy);
}

// ---- Ld3b-2: the deps-config (`--deps`) resolution path -----------------

/// Options with `deps: Some(path)`.
fn envelopes_v01_with_deps(deps_path: PathBuf) -> LoadOptions {
    LoadOptions {
        version: RustyfiVersion::V0_1,
        mode: LoadMode::Envelopes {
            deps: Some(deps_path),
        },
        ..Default::default()
    }
}

/// Write a `rustyfi-envelope.yaml` (library) + its `src/*.satyh` files under
/// `<root>/<pkg>/`, returning the absolute path to the envelope config file.
fn write_library_envelope(dir: &TempDir, pkg: &str, main_module: &str, sources: &[(&str, &str)]) -> PathBuf {
    let config = dir.write(
        &format!("{pkg}/rustyfi-envelope.yaml"),
        &format!(
            "library:\n  main_module: {main_module}\n  source_directories:\n  - ./src\n  test_directories: []\n"
        ),
    );
    for (name, src) in sources {
        dir.write(&format!("{pkg}/src/{name}"), src);
    }
    config
}

/// Render a `rustyfi-deps.yaml` string. `envelopes` = `(name, abs_config_path,
/// deps_names, test_only)`; `top_deps` = `(name, used_as)` for the document's
/// own `dependencies`.
fn deps_yaml(
    envelopes: &[(&str, &Path, &[&str], bool)],
    top_deps: &[(&str, &str)],
) -> String {
    let mut s = String::from("envelopes:\n");
    for (name, path, deps, test_only) in envelopes {
        s.push_str(&format!("- name: \"{name}\"\n"));
        s.push_str(&format!("  path: \"{}\"\n", path.display()));
        if deps.is_empty() {
            s.push_str("  dependencies: []\n");
        } else {
            s.push_str("  dependencies:\n");
            for d in *deps {
                s.push_str(&format!("  - name: \"{d}\"\n    used_as: \"Dep\"\n"));
            }
        }
        s.push_str(&format!("  test_only: {test_only}\n"));
    }
    if top_deps.is_empty() {
        s.push_str("dependencies: []\n");
    } else {
        s.push_str("dependencies:\n");
        for (name, used_as) in top_deps {
            s.push_str(&format!("- name: \"{name}\"\n  used_as: \"{used_as}\"\n"));
        }
    }
    s.push_str("test_dependencies: []\n");
    s
}

#[test]
fn envelopes_use_package_loads_envelope_modules() {
    let dir = TempDir::new("env-usepkg-ok");
    let config = write_library_envelope(
        &dir,
        "pkg",
        "Foo",
        &[("foo.satyh", "module Foo = struct\nval x = 1\nend")],
    );
    let deps = dir.write(
        "rustyfi-deps.yaml",
        &deps_yaml(&[("pkg", &config, &[], false)], &[("pkg", "Foo")]),
    );
    let entry = dir.write("doc.saty", "use package Foo\nlet x = 1 in x");

    let program = load(&entry, &envelopes_v01_with_deps(deps)).expect("use package load succeeds");

    assert_eq!(file_names(&program), vec!["foo.satyh", "doc.saty"]);
    assert_eq!(
        program.files[0].origin,
        FileOrigin::Envelope {
            envelope: "pkg".to_string(),
            module: "Foo".to_string(),
        }
    );
    assert_eq!(program.files[1].origin, FileOrigin::Local);
    assert!(program.files[1].cst.is_document(), "entry is the document");
}

#[test]
fn envelopes_bare_use_orders_modules_within_envelope() {
    let dir = TempDir::new("env-bareuse-order");
    // C uses B, B uses A — file names deliberately NOT in dependency order.
    let config = write_library_envelope(
        &dir,
        "pkg",
        "A",
        &[
            ("z_c.satyh", "use B\nmodule C = struct\nval c = 1\nend"),
            ("y_b.satyh", "use A\nmodule B = struct\nval b = 1\nend"),
            ("x_a.satyh", "module A = struct\nval a = 1\nend"),
        ],
    );
    let deps = dir.write(
        "rustyfi-deps.yaml",
        &deps_yaml(&[("pkg", &config, &[], false)], &[("pkg", "A")]),
    );
    let entry = dir.write("doc.saty", "use package A\nlet x = 1 in x");

    let program = load(&entry, &envelopes_v01_with_deps(deps)).expect("bare-use envelope loads");

    let modules: Vec<String> = program.files[..3]
        .iter()
        .map(|f| match &f.origin {
            FileOrigin::Envelope { module, .. } => module.clone(),
            FileOrigin::Local => panic!("expected envelope origin"),
        })
        .collect();
    assert_eq!(modules, vec!["A", "B", "C"], "closed dependency order");
    assert_eq!(file_names(&program).last().unwrap(), "doc.saty");
}

#[test]
fn envelopes_module_name_conflict_detected() {
    let dir = TempDir::new("env-modconflict");
    let config = write_library_envelope(
        &dir,
        "pkg",
        "Foo",
        &[
            ("a.satyh", "module Foo = struct\nval x = 1\nend"),
            ("b.satyh", "module Foo = struct\nval y = 2\nend"),
        ],
    );
    let deps = dir.write(
        "rustyfi-deps.yaml",
        &deps_yaml(&[("pkg", &config, &[], false)], &[("pkg", "Foo")]),
    );
    let entry = dir.write("doc.saty", "use package Foo\nlet x = 1 in x");

    let err = load(&entry, &envelopes_v01_with_deps(deps)).expect_err("duplicate module errors");
    assert!(
        matches!(err, LoadError::FileModuleNameConflict { .. }),
        "got {err:?}"
    );
}

#[test]
fn envelopes_bare_use_unknown_module() {
    let dir = TempDir::new("env-unknownmod");
    let config = write_library_envelope(
        &dir,
        "pkg",
        "Foo",
        &[("foo.satyh", "use Missing\nmodule Foo = struct\nval x = 1\nend")],
    );
    let deps = dir.write(
        "rustyfi-deps.yaml",
        &deps_yaml(&[("pkg", &config, &[], false)], &[("pkg", "Foo")]),
    );
    let entry = dir.write("doc.saty", "use package Foo\nlet x = 1 in x");

    let err = load(&entry, &envelopes_v01_with_deps(deps)).expect_err("unknown module errors");
    match err {
        LoadError::FileModuleNotFound { module, .. } => assert_eq!(module, "Missing"),
        other => panic!("expected FileModuleNotFound, got {other:?}"),
    }
}

#[test]
fn envelopes_use_of_inside_package_rejected() {
    let dir = TempDir::new("env-useof-inside");
    let config = write_library_envelope(
        &dir,
        "pkg",
        "Foo",
        &[("foo.satyh", "use B of `./b`\nmodule Foo = struct\nval x = 1\nend")],
    );
    let deps = dir.write(
        "rustyfi-deps.yaml",
        &deps_yaml(&[("pkg", &config, &[], false)], &[("pkg", "Foo")]),
    );
    let entry = dir.write("doc.saty", "use package Foo\nlet x = 1 in x");

    let err = load(&entry, &envelopes_v01_with_deps(deps)).expect_err("use…of inside pkg errors");
    assert!(
        matches!(err, LoadError::UseOfInsidePackage { .. }),
        "got {err:?}"
    );
}

#[test]
fn envelopes_envelope_graph_toposorted() {
    let dir = TempDir::new("env-envtopo");
    // Envelope B depends on A; listed B-first in the config → A files first.
    let config_a = write_library_envelope(
        &dir,
        "pkg-a",
        "A",
        &[("a.satyh", "module A = struct\nval a = 1\nend")],
    );
    let config_b = write_library_envelope(
        &dir,
        "pkg-b",
        "B",
        &[("b.satyh", "module B = struct\nval b = 2\nend")],
    );
    let deps = dir.write(
        "rustyfi-deps.yaml",
        &deps_yaml(
            &[
                ("envB", &config_b, &["envA"], false),
                ("envA", &config_a, &[], false),
            ],
            &[("envA", "A")],
        ),
    );
    let entry = dir.write("doc.saty", "use package A\nlet x = 1 in x");

    let program = load(&entry, &envelopes_v01_with_deps(deps)).expect("envelope graph loads");
    let names = file_names(&program);
    let a_pos = names.iter().position(|n| n == "a.satyh").unwrap();
    let b_pos = names.iter().position(|n| n == "b.satyh").unwrap();
    assert!(a_pos < b_pos, "envelope A before B: {names:?}");
}

#[test]
fn envelopes_use_package_unknown_alias() {
    let dir = TempDir::new("env-unknownalias");
    let config = write_library_envelope(
        &dir,
        "pkg",
        "Foo",
        &[("foo.satyh", "module Foo = struct\nval x = 1\nend")],
    );
    // The doc says `use package Bar`, but only `Foo` is an alias.
    let deps = dir.write(
        "rustyfi-deps.yaml",
        &deps_yaml(&[("pkg", &config, &[], false)], &[("pkg", "Foo")]),
    );
    let entry = dir.write("doc.saty", "use package Bar\nlet x = 1 in x");

    let err = load(&entry, &envelopes_v01_with_deps(deps)).expect_err("unknown alias errors");
    match err {
        LoadError::UnknownPackageDependency { module, .. } => assert_eq!(module, "Bar"),
        other => panic!("expected UnknownPackageDependency, got {other:?}"),
    }
}

#[test]
fn envelopes_test_only_envelope_skipped() {
    let dir = TempDir::new("env-testonly");
    let config = write_library_envelope(
        &dir,
        "pkg",
        "Foo",
        &[("foo.satyh", "module Foo = struct\nval x = 1\nend")],
    );
    // A test-only envelope whose config file DOES NOT EXIST — it must never be
    // read (upstream skips before reading, closedEnvelopeDependencyResolver.ml
    // :38-40).
    let deps = dir.write(
        "rustyfi-deps.yaml",
        &deps_yaml(
            &[
                ("pkg", &config, &[], false),
                ("testpkg", Path::new("/nonexistent/rustyfi-envelope.yaml"), &[], true),
            ],
            &[("pkg", "Foo")],
        ),
    );
    let entry = dir.write("doc.saty", "use package Foo\nlet x = 1 in x");

    let program =
        load(&entry, &envelopes_v01_with_deps(deps)).expect("test-only envelope is skipped");
    assert_eq!(file_names(&program), vec!["foo.satyh", "doc.saty"]);
}

#[test]
fn envelopes_font_envelope_contributes_no_files() {
    let dir = TempDir::new("env-font");
    // A font envelope: no sources parsed, but it still occupies a vertex, and
    // a library envelope depending on it loads fine.
    let font_config = dir.write(
        "font/rustyfi-envelope.yaml",
        "font:\n  main_module: FontFoo\n  files:\n  - path: ./foo.otf\n    opentype_single:\n      name: foo\n      math: false\n",
    );
    let lib_config = write_library_envelope(
        &dir,
        "pkg",
        "Foo",
        &[("foo.satyh", "module Foo = struct\nval x = 1\nend")],
    );
    let deps = dir.write(
        "rustyfi-deps.yaml",
        &deps_yaml(
            &[
                ("font", &font_config, &[], false),
                ("pkg", &lib_config, &["font"], false),
            ],
            &[("pkg", "Foo")],
        ),
    );
    let entry = dir.write("doc.saty", "use package Foo\nlet x = 1 in x");

    let program = load(&entry, &envelopes_v01_with_deps(deps)).expect("font envelope loads");
    // Only the library file (+ entry) appear; the font contributes nothing.
    assert_eq!(file_names(&program), vec!["foo.satyh", "doc.saty"]);
}
