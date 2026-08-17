//! Math-command cross-module exposure: a `direct \cmd : .. math-cmd` item in
//! a `module M : sig .. end = struct .. end`'s signature must expose `\cmd`
//! UNQUALIFIED at the enclosing scope — the math-command analog of the
//! existing inline/block `direct`-exposure wave (`elaborate.rs`'s
//! `direct_cmd_name` + `TopBinding::Module` arm) — so that an importer's
//! `${..}` can reference `\cmd` without an `M.` prefix, exactly like
//! `bnf.satyh` (`@require: math`) does with `Math`'s `\mid`/`\mathrel`.
//!
//! This is a minimal, self-contained fixture (no `lib-satysfi` package
//! involved) proving the mechanism in isolation; `math_package.rs` covers
//! the real `math.satyh` gate.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use satysfi_backend::{FontKey, FontMetrics, Length};
use satysfi_lang::value::Value;
use satysfi_lang::{elaborate, eval, primitives, typecheck};
use satysfi_loader::{LoadOptions, LoadedProgram};

/// A uniquely-named temp directory (cleaned up on drop) holding one fixture
/// pair: a library file (`@import:`ed) and an entry document, so
/// `@import: lib` resolves relative to the entry's own directory (see
/// `satysfi-loader`'s `resolve_import`) without needing any `lib_root`.
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> TempDir {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "satysfi-lang-math-cmd-exposure-{tag}-{}-{}",
            std::process::id(),
            n
        ));
        fs::create_dir_all(&dir).expect("create temp fixture dir");
        TempDir(dir)
    }

    fn write(&self, name: &str, src: &str) -> PathBuf {
        let path = self.0.join(name);
        fs::write(&path, src).expect("write temp fixture file");
        path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn as_v006(cst: satysfi_loader::LoadedCst) -> satysfi_syntax::cst::File {
    match cst {
        satysfi_loader::LoadedCst::V0_0_6(f) => f,
        satysfi_loader::LoadedCst::V0_1(_) => {
            unreachable!("this test's merge_program is the V0_0_6-only path")
        }
    }
}

fn merge_program(program: LoadedProgram) -> satysfi_syntax::cst::File {
    let mut files = program.files;
    let entry = files.pop().expect("loader always yields the entry last");
    let entry_cst = as_v006(entry.cst);
    let mut prelude = Vec::new();
    for lib in files {
        prelude.extend(as_v006(lib.cst).prelude);
    }
    prelude.extend(entry_cst.prelude);
    satysfi_syntax::cst::File {
        headers: Vec::new(),
        prelude,
        in_kw: entry_cst.in_kw,
        body: entry_cst.body,
        eoi: entry_cst.eoi,
    }
}

struct NoFonts;

impl FontMetrics for NoFonts {
    fn advance(&self, _f: FontKey, _c: char, _size: Length) -> Option<Length> {
        None
    }
    fn ascender(&self, _f: FontKey, size: Length) -> Length {
        size
    }
    fn descender(&self, _f: FontKey, _size: Length) -> Length {
        Length::pt(0.0)
    }
}

/// An ASCII-only stub (mirrors `stdlib_tier0.rs`'s own `Mono`) — for the one
/// test below that forces glyph metrics via `embed-math` (`NoFonts`'s
/// always-`None` `advance` would make any character a dynamic error).
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

/// Load `entry_path` (already written to disk, `@import:`ing a sibling
/// library file) through the real loader, merge, elaborate, typecheck, and
/// evaluate — the full "compiles" bar, mirroring `math_package.rs`'s
/// `compile_via_loader` but keyed off a path rather than fresh source (no
/// `lib_root` is needed here: everything resolves via `@import:`).
fn compile_via_loader(entry_path: &PathBuf) -> Result<Value, String> {
    compile_via_loader_with_metrics(entry_path, &NoFonts)
}

fn compile_via_loader_with_metrics(
    entry_path: &PathBuf,
    metrics: &dyn FontMetrics,
) -> Result<Value, String> {
    let opts = LoadOptions::default();
    let program = satysfi_loader::load(entry_path, &opts).map_err(|e| format!("load: {e}"))?;
    let file = merge_program(program);

    let env = primitives::base_env();
    let scope = elaborate::Scope::new(env.names());
    let elaborated =
        elaborate::elaborate_program(&file, &scope).map_err(|e| format!("elaborate: {e}"))?;
    typecheck::typecheck(&elaborated).map_err(|e| format!("typecheck: {e}"))?;
    let mut interp = eval::Interp::new(metrics);
    interp
        .eval(&env, &elaborated.body)
        .map_err(|e| format!("eval: {e}"))
}

const LIB_SRC: &str = "\
module M : sig
  direct \\foo : [] math-cmd
  direct \\bar : [math] math-cmd
end = struct
  let-math \\foo = math-char MathOrd `x`
  let-math \\bar m = math-concat m (math-char MathOrd `y`)
end
";

/// A `direct`-exposed, zero-argument math command (`\foo`) is usable
/// UNQUALIFIED in an importer's `${..}` — proving both typecheck (its
/// `math-cmd` type threads through the alias) and eval (`read_math`'s
/// `MathElem::Cmd` applies the exposed binding, here to a `Value::MathText`).
#[test]
fn direct_math_cmd_is_usable_unqualified_across_a_module_boundary() {
    let dir = TempDir::new("nullary");
    dir.write("lib.satyh", LIB_SRC);
    let entry = dir.write(
        "entry.saty",
        "@import: lib
in
${\\foo}",
    );
    let v = compile_via_loader(&entry).expect("unqualified \\foo should compile and evaluate");
    assert!(matches!(v, Value::MathText { .. }), "expected a math value, got {v:?}");
}

/// The same, but for a math command that takes a `math` argument (`\bar`),
/// applied inside `${..}` exactly like `bnf.satyh`'s `\mathrel{..}` usage.
#[test]
fn direct_math_cmd_with_arg_is_usable_unqualified_across_a_module_boundary() {
    let dir = TempDir::new("unary");
    dir.write("lib.satyh", LIB_SRC);
    let entry = dir.write(
        "entry.saty",
        "@import: lib
in
${\\bar{\\foo}}",
    );
    let v = compile_via_loader(&entry).expect("unqualified \\bar{..} should compile and evaluate");
    assert!(matches!(v, Value::MathText { .. }), "expected a math value, got {v:?}");
}

/// `bnf.satyh` never uses a bare `${..}` — it always forces evaluation
/// through `embed-math ctx ${..}` (`as_math`/`reflect_math_elem`'s
/// `MathElem::Cmd` arm: `env.lookup(name)` then apply args), which is a
/// DIFFERENT code path from the two tests above (those only prove the
/// `Value::MathText` builds; they never force-apply `\foo`/`\bar`'s
/// closure). This is the one that actually matters for `bnf.satyh`.
#[test]
fn direct_math_cmd_survives_embed_math_forcing() {
    let dir = TempDir::new("embed-math");
    dir.write("lib.satyh", LIB_SRC);
    let entry = dir.write(
        "entry.saty",
        "@import: lib
let-inline ctx \\dummy m = inline-nil
in
let ctx = get-initial-context 100pt (command \\dummy) in
get-natural-metrics (embed-math ctx ${\\bar{\\foo}})",
    );
    let v = compile_via_loader_with_metrics(&entry, &Mono)
        .expect("embed-math ctx ${\\bar{\\foo}} should compile and evaluate");
    match v {
        Value::Tuple(vs) => assert_eq!(vs.len(), 3, "expected (width, height, depth)"),
        other => panic!("expected a tuple, got {other:?}"),
    }
}

/// Negative control: a `val \cmd : .. math-cmd` sig item (no `direct`) must
/// NOT be exposed unqualified — proving the exposure mechanism specifically
/// keys off `direct` (matching `direct_cmd_name`'s existing inline/block
/// behavior) rather than exposing every sig item indiscriminately.
#[test]
fn val_only_math_cmd_is_not_exposed_unqualified() {
    let dir = TempDir::new("val-only");
    dir.write(
        "lib.satyh",
        "module N : sig
  val \\foo : [] math-cmd
end = struct
  let-math \\foo = math-char MathOrd `x`
end
",
    );
    let entry = dir.write(
        "entry.saty",
        "@import: lib
in
${\\foo}",
    );
    let err = compile_via_loader(&entry).expect_err("un-`direct`-ed \\foo must stay unqualified-invisible");
    assert!(
        err.contains("unbound math command"),
        "expected an unbound-math-command error, got: {err}"
    );
}

/// Inverse of [`val_only_math_cmd_is_not_exposed_unqualified`]: even though a
/// `val`-only (no `direct`) sig item never exposes its command UNQUALIFIED,
/// the module's own qualified binding is always inserted (`TopBinding::Module`
/// binds `N.\foo` regardless of `direct` — see its doc comment in
/// `elaborate.rs`). So the fully QUALIFIED reference `${\N.foo}` must still
/// resolve, proving `AnyMathCmdTok::Mod`/`math_cmd_key` reach a module-scoped
/// math command even when it is not `direct`-exposed unqualified.
#[test]
fn val_only_math_cmd_is_reachable_when_qualified() {
    let dir = TempDir::new("val-only-qualified");
    dir.write(
        "lib.satyh",
        "module N : sig
  val \\foo : [] math-cmd
end = struct
  let-math \\foo = math-char MathOrd `x`
end
",
    );
    let entry = dir.write(
        "entry.saty",
        "@import: lib
in
${\\N.foo}",
    );
    let v = compile_via_loader(&entry).expect("qualified \\N.foo should compile and evaluate");
    assert!(matches!(v, Value::MathText { .. }), "expected a math value, got {v:?}");
}
