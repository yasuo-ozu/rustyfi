//! Integration tests for the multi-file loader, driven against small
//! fixture trees built under a unique temp directory per test (cleaned up on
//! drop; no extra dev-dependencies needed beyond `std`).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use satysfi_loader::{load, LoadError, LoadOptions, LoadedProgram, SatysfiVersion};

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "satysfi-loader-test-{tag}-{}-{}-{}",
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
    assert!(program.files[0].cst.body.is_some());
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

#[test]
fn unimplemented_version_is_rejected_before_touching_disk() {
    let dir = TempDir::new("unsupported-version");
    // Deliberately do NOT create `missing.saty`: an `UnsupportedVersion`
    // error must be returned before `load` ever reads the entry file.
    let entry = dir.path().join("missing.saty");

    let opts = LoadOptions {
        lib_root: None,
        version: SatysfiVersion::V0_1,
    };
    let err = load(&entry, &opts).expect_err("0.1 must be rejected");
    let msg = err.to_string();

    match &err {
        LoadError::UnsupportedVersion {
            requested,
            supported,
        } => {
            assert_eq!(*requested, SatysfiVersion::V0_1);
            assert_eq!(supported, &vec![SatysfiVersion::V0_0_6]);
        }
        other => panic!("expected LoadError::UnsupportedVersion, got {other:?}"),
    }
    assert!(msg.contains("0.1"), "message should name the requested version: {msg}");
    assert!(
        msg.contains("0.0.6"),
        "message should name the supported version: {msg}"
    );
}

#[test]
fn default_load_options_targets_v0_0_6() {
    assert_eq!(LoadOptions::default().version, SatysfiVersion::V0_0_6);
}
