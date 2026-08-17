//! Slice X1 (`docs/plans/design-cross-version-import.md`): a `V0_1` document
//! `@require:`-ing a version-neutral `V0_0_6` package end-to-end.
//!
//! Two cases, both driven through the REAL loader (`satysfi_loader::load`,
//! `LoadOptions { version: V0_1, .. }`) so the per-file detection rule
//! (`load_legacy`'s Q4 worklist logic) and the forked-name guard
//! (`compile_document_v1_with_trials`'s dep loop, `collect_free_globals`)
//! are both exercised for real, not bypassed:
//!
//! - [`positive_case_list_and_option_render`]: a `V0_1` entry requiring the
//!   REAL vendored `list.satyg`/`option.satyg` (copied byte-for-byte from
//!   `lib-satysfi/dist/packages/` into a temp `lib_root` — see
//!   [`TempDir::copy_real_package`]) — `List.map`/`List.fold-left`/
//!   `Option.from` all get used, and the resulting document actually
//!   renders (a real page with real placed content), proving the splice +
//!   guard pass a genuinely version-neutral dependency through untouched.
//! - [`negative_case_forked_primitive_is_rejected`]: a `V0_0_6` package
//!   that references `page-break` (a version-forked primitive —
//!   `primitives::forked_prim_names`) unqualified — asserts
//!   `CompileError::CrossVersionUnsupportedName`, *not* a wrong render.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use satysfi_backend::{FontKey, FontMetrics, Length};
use satysfi_lang::CompileError;
use satysfi_loader::{LoadOptions, SatysfiVersion};

/// This repo's root, resolved relative to this crate's own manifest
/// directory (`crates/satysfi-lang/../..`) — same helper every other
/// `v01_*`/`xver_*` integration test in this crate reproduces locally (no
/// shared test-support library target exists here).
fn repo(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(rel)
}

/// A small fixture tree under a unique temp directory (cleaned up on drop) —
/// mirrors `crates/satysfi-loader/tests/loader.rs`'s own `TempDir` helper.
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "satysfi-lang-xver-test-{tag}-{}-{}-{}",
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
    /// root), creating any needed parent directories.
    fn write(&self, rel: &str, content: &str) {
        let p = self.0.join(rel);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).expect("create parent dir");
        }
        fs::write(&p, content).expect("write fixture file");
    }

    /// Copy the REAL vendored `lib-satysfi/dist/packages/<name>` byte-for-
    /// byte into this temp tree's own `dist/packages/<name>` — so
    /// `@require: <name>` against `self.path()` as `lib_root` resolves to
    /// the actual frozen 0.0.6 corpus content, without needing a SECOND
    /// `lib_root` alongside this test's own V0_1 helper library (which must
    /// live in the SAME tree so `@import:` can reach it — see this file's
    /// module doc comment).
    fn copy_real_package(&self, name: &str) {
        let real = repo(&format!("lib-satysfi/dist/packages/{name}"));
        let content = fs::read_to_string(&real).unwrap_or_else(|e| panic!("read {real:?}: {e}"));
        self.write(&format!("dist/packages/{name}"), &content);
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A real (ASCII-only) `FontMetrics` — the positive-case fixture renders
/// actual digits (`arabic`'s output), so `advance` must return `Some` for
/// ASCII — same shape as every other `v01_*` test's `Mono` stub.
struct Mono;

impl FontMetrics for Mono {
    fn advance(&self, _f: FontKey, c: char, size: Length) -> Option<Length> {
        if c.is_ascii() {
            Some(size * 0.5)
        } else {
            None
        }
    }
    fn ascender(&self, _f: FontKey, size: Length) -> Length {
        size * 0.75
    }
    fn descender(&self, _f: FontKey, size: Length) -> Length {
        size * 0.25
    }
}

/// A tiny V0_1 helper library providing `get-initial-context`'s mandatory
/// `[math] inline-cmd` argument — `V0_1`'s `FileV1::Document` has no prelude
/// of its own (every `let` chains its own `in`), so a document-scaffolding
/// command like `\math` must come from a required/imported library, exactly
/// like the real `v01-mini.satyh`/`stdja-mini.satyh` fixtures do. Reached via
/// `@import:` (a same-directory SIBLING of the entry), NOT `@require:` — so
/// it is NOT a `@require:`-resolved corpus target and correctly keeps
/// `opts.version` (`V0_1`) under the X1 Q4 per-file detection rule, even
/// though (like every 0.0.6-style `module … = struct … end` head) plain
/// content sniffing alone would be ambiguous (`version.rs`'s own doc
/// comment: a bare `module` head is deliberately not a signal).
const XVER_HELPER_SRC: &str =
    "module XverHelper = struct\n  val inline ctx \\math m = embed-math ctx (read-math ctx m)\nend\n";

/// The positive-case V0_1 entry: `@require:`s the real vendored
/// `list`/`option` (version-neutral, X1's target subset), `@import:`s the
/// local math-command helper above, and actually calls `List.map`/
/// `List.fold-left`/`Option.from` — end-to-end use, not just a parse probe.
fn positive_entry_src() -> String {
    "@require: list\n\
     @require: option\n\
     @import: xver-helper\n\
     \n\
     let mapped = List.map (fun x -> x + 1) [1, 2, 3] in\n\
     let summed = List.fold-left (fun a b -> a + b) 0 mapped in\n\
     let combined = Option.from 0 (Some summed) in\n\
     let ctx = get-initial-context 440pt (command \\XverHelper.math) in\n\
     let content pbinfo = (| text-origin = (72pt, 100pt), text-height = 640pt |) in\n\
     let parts pbinfo =\n\
       (| header-origin = (72pt, 72pt),  header-content = block-nil,\n\
          footer-origin = (72pt, 800pt), footer-content = block-nil |)\n\
     in\n\
     let body =\n\
       line-break true true ctx\n\
         (inline-fil ++ (read-inline ctx (embed-string (arabic combined))) ++ inline-fil)\n\
     in\n\
     page-break (210mm, 297mm) content parts body\n"
        .to_string()
}

#[test]
fn positive_case_list_and_option_render() {
    let dir = TempDir::new("positive");
    dir.copy_real_package("list.satyg");
    dir.copy_real_package("option.satyg");
    dir.write("xver-helper.satyh", XVER_HELPER_SRC);
    let entry_src = positive_entry_src();
    dir.write("entry.saty", &entry_src);

    let opts = LoadOptions {
        lib_root: Some(dir.path().to_path_buf()),
        version: SatysfiVersion::V0_1,
        ..Default::default()
    };
    let program = satysfi_loader::load(&dir.path().join("entry.saty"), &opts)
        .unwrap_or_else(|e| panic!("loading the positive xver fixture should succeed: {e}"));

    // The two REQUIRE'd corpus packages (list, option — option is also
    // pulled in transitively by list.satyg's own `@require: option`, so
    // both are deduplicated to ONE node each) must have been detected as
    // `V0_0_6` (Q4's provenance rule), while the `@import:`-reached helper
    // and the entry itself stay `V0_1`.
    let mut saw_v006 = 0;
    for f in &program.files[..program.files.len() - 1] {
        match f.version {
            SatysfiVersion::V0_0_6 => {
                saw_v006 += 1;
                assert!(
                    matches!(f.cst, satysfi_loader::LoadedCst::V0_0_6(_)),
                    "a V0_0_6-tagged file must carry a V0_0_6-parsed cst: {:?}",
                    f.path
                );
            }
            SatysfiVersion::V0_1 => {
                assert!(
                    matches!(f.cst, satysfi_loader::LoadedCst::V0_1(_)),
                    "a V0_1-tagged file must carry a V0_1-parsed cst: {:?}",
                    f.path
                );
            }
            other => panic!("unexpected version tag {other} on {:?}", f.path),
        }
    }
    assert_eq!(saw_v006, 2, "list.satyg + option.satyg should both be V0_0_6-tagged deps");
    let entry_file = program.files.last().expect("loader always yields the entry last");
    assert!(
        matches!(entry_file.version, SatysfiVersion::V0_1),
        "the entry must always stay V0_1"
    );

    let mono = Mono;
    let (doc, _trials) = satysfi_lang::compile_document_v1_with_trials(&program.files, &mono)
        .unwrap_or_else(|e| panic!("compile_document_v1_with_trials should succeed: {e}"));

    // `mapped = [2;3;4]`, `summed = 9` (List.map/List.fold-left, from the
    // spliced list.satyg), `combined = Option.from 0 (Some 9) = 9` (from the
    // spliced option.satyg) — a real page with real placed content proves
    // the whole pipeline (splice -> elaborate -> typecheck -> eval ->
    // layout) actually ran on the cross-version-linked bindings, not merely
    // that compilation returned `Ok`.
    assert_eq!(doc.pages.len(), 1, "one A4 page");
    assert!(
        !doc.pages[0].lines.is_empty(),
        "the footer line (rendering `combined` = 9) must have been placed"
    );
}

/// The X1 forked-name guard's negative proof: a `V0_0_6` package that
/// references `page-break` (version-forked — bound to a DIFFERENT arity
/// under `V0_0_6` vs `V0_1`, `primitives::forked_prim_names`) unqualified,
/// at top level. Must be rejected with `CrossVersionUnsupportedName`
/// *before* any elaboration/typecheck/eval runs on the spliced dependency —
/// silently mis-resolving `page-break` to the V0_1 primitive (§3.2's R1)
/// would be strictly worse than refusing to compile at all.
#[test]
fn negative_case_forked_primitive_is_rejected() {
    let dir = TempDir::new("negative");
    dir.write(
        "dist/packages/xver-forked.satyg",
        "@stage: persistent\n\nlet trigger-forked-name = page-break\n",
    );
    dir.write("entry.saty", "@require: xver-forked\n\n0\n");

    let opts = LoadOptions {
        lib_root: Some(dir.path().to_path_buf()),
        version: SatysfiVersion::V0_1,
        ..Default::default()
    };
    let program = satysfi_loader::load(&dir.path().join("entry.saty"), &opts)
        .unwrap_or_else(|e| panic!("loading the negative xver fixture should succeed: {e}"));
    assert!(
        matches!(
            program.files[0].cst,
            satysfi_loader::LoadedCst::V0_0_6(_)
        ),
        "xver-forked.satyg should have been detected as V0_0_6 (it's a @require: corpus target)"
    );

    let mono = Mono;
    let err = satysfi_lang::compile_document_v1(&program.files, &mono)
        .expect_err("a dependency referencing a version-forked primitive must be rejected");
    match err {
        CompileError::CrossVersionUnsupportedName { name, slice, .. } => {
            assert_eq!(name, "page-break");
            assert_eq!(slice, "X1");
        }
        other => panic!("expected CrossVersionUnsupportedName, got: {other}"),
    }
}
