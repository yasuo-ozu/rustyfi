//! The real `@require: math` gate (`docs/plans/math-engine.md` §A + §G):
//! `lib-satysfi/dist/packages/math.satyh` (ported byte-for-byte from
//! upstream) must PARSE, ELABORATE, TYPECHECK, and EVALUATE through the
//! production multi-file loader — the same "compiles" bar
//! `stdlib_tier0.rs`'s own tests hold `list`/`option`/`pervasives`/`gr` to
//! (that file is owned by a concurrent sibling agent porting off-path
//! packages, so this is a NEW file rather than an addition there; the
//! helper shapes below are deliberately copied, not shared, per that
//! constraint).
//!
//! `math.satyh` transitively pulls in `pervasives`, `list` (hence
//! `option`), and `gr` (hence `geom`) — all five already ported and proven
//! to load on their own (`stdlib_tier0.rs`). This test is the first proof
//! that the whole graph loads TOGETHER, and specifically that every one of
//! `math.satyh`'s ~200 `let`/`let-math` bindings (`\alpha`..`\Omega`,
//! `\to`/`\pm`/…, `\sum`/`\int`/…, `\paren`/`\brace`/…, `\frac`, `\sqrt`,
//! …) both type-checks against its `sig` and evaluates without error — most
//! of them via a FULLY-APPLIED `math-*` primitive call at binding time (see
//! `primitives.rs`'s new §A + §G section), not merely a deferred closure.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use satysfi_backend::{FontKey, FontMetrics, Length};
use satysfi_lang::value::Value;
use satysfi_lang::{elaborate, eval, primitives, typecheck};
use satysfi_loader::{LoadOptions, LoadedProgram};

/// This repo's `lib-satysfi/` (`math.satyh`'s real home), resolved relative
/// to this crate's manifest directory — the same convention
/// `stdlib_tier0.rs`/`compile.rs`'s test helpers already use.
fn lib_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lib-satysfi")
}

/// A uniquely-named temp `.saty` file, cleaned up on drop (mirrors
/// `stdlib_tier0.rs`'s own `TempDoc`, copied rather than shared — see this
/// module's doc comment).
struct TempDoc(PathBuf);

impl TempDoc {
    fn new(tag: &str, src: &str) -> TempDoc {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "satysfi-lang-math-package-{tag}-{}-{}.saty",
            std::process::id(),
            n
        ));
        fs::write(&path, src).expect("write temp fixture");
        TempDoc(path)
    }
}

impl Drop for TempDoc {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// Merge a loader-resolved program's preludes into one synthetic
/// `cst::File`, exactly like `satysfi-cli`'s `merge_program` /
/// `stdlib_tier0.rs`'s own copy: the loader guarantees dependency-first
/// order with the entry document last, so every library's prelude is
/// spliced ahead of the entry's own, in that order.
fn merge_program(program: LoadedProgram) -> satysfi_syntax::cst::File {
    let mut files = program.files;
    let entry = files.pop().expect("loader always yields the entry last");
    let mut prelude = Vec::new();
    for lib in files {
        prelude.extend(lib.cst.prelude);
    }
    prelude.extend(entry.cst.prelude);
    satysfi_syntax::cst::File {
        headers: Vec::new(),
        prelude,
        in_kw: entry.cst.in_kw,
        body: entry.cst.body,
        eoi: entry.cst.eoi,
    }
}

/// `FontMetrics` stub: `math.satyh`'s own top-level evaluation never
/// actually measures a glyph (see this module's doc comment — every
/// `math-*` primitive call at binding time just builds a `Value::Math`
/// tree; nothing here calls `embed-math`/renders a page), so this is never
/// consulted, mirroring `stdlib_tier0.rs`'s `NoFonts`.
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

/// Load `src` (a document `@require:`ing packages resolved against
/// `lib_root()`) through the real loader, merge, elaborate, typecheck, and
/// evaluate — returning the final `Value`. The full "compiles" bar, not
/// merely a parse or a typecheck (mirrors `stdlib_tier0.rs`'s
/// `compile_via_loader`).
fn compile_via_loader(tag: &str, src: &str) -> Result<Value, String> {
    let doc = TempDoc::new(tag, src);
    let opts = LoadOptions {
        lib_root: Some(lib_root()),
        ..Default::default()
    };
    let program = satysfi_loader::load(&doc.0, &opts).map_err(|e| format!("load: {e}"))?;
    let file = merge_program(program);

    let env = primitives::base_env();
    let scope = elaborate::Scope::new(env.names());
    let elaborated =
        elaborate::elaborate_program(&file, &scope).map_err(|e| format!("elaborate: {e}"))?;
    typecheck::typecheck(&elaborated).map_err(|e| format!("typecheck: {e}"))?;
    let mut interp = eval::Interp::new(&NoFonts);
    interp
        .eval(&env, &elaborated.body)
        .map_err(|e| format!("eval: {e}"))
}

/// Run `f` on a thread with a generously large stack. `math.satyh` (1571
/// lines — the largest bundled package by far, and now merged alongside
/// `pervasives`/`list`/`option`/`gr`/`geom`'s own preludes) needs more depth
/// than the default stack allows through syan's recursive-descent parser
/// and this port's tree-walking elaborator/typechecker/evaluator, the same
/// reason `stdlib_tier0.rs`'s own `run_with_big_stack` exists for `gr.satyh`
/// (205 lines). `Value` holds `Rc`s (not `Send`), so the compile call AND
/// every assertion on its result must run entirely *inside* `f`.
fn run_with_big_stack(f: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(f)
        .expect("spawn big-stack thread")
        .join()
        .expect("big-stack thread panicked (see assertion above)");
}

fn as_int(v: Value) -> i64 {
    match v {
        Value::Int(n) => n,
        other => panic!("expected an int, got {other:?}"),
    }
}

// ============================================================================
// `@require: math` — the real gate.
// ============================================================================

#[test]
fn require_math_compiles_and_evaluates() {
    run_with_big_stack(|| {
        // The cheapest whole-module proof (same rationale as
        // `stdlib_tier0.rs`'s `require_gr_rectangle_compiles_and_evaluates`):
        // a trivial body still forces the loader to resolve the FULL
        // transitive graph (`pervasives`, `list` -> `option`, `gr` ->
        // `geom`) and forces elaboration/typecheck/evaluation of every one
        // of `math.satyh`'s top-level bindings (`Ast::LetIn`/`Ast::
        // LetMathIn` always infers + evaluates its value eagerly,
        // regardless of whether the entry body ever references it) — so
        // success here means the ENTIRE file compiled, not just whatever
        // the body names.
        let src = "@require: math
in
0";
        let v = compile_via_loader("require-math", src).expect("math.satyh should compile");
        assert_eq!(as_int(v), 0);
    });
}

#[test]
fn require_math_join_is_reachable_across_the_module_boundary() {
    run_with_big_stack(|| {
        // A slightly stronger proof than the trivial-body test above:
        // `Math.join` (a plain, non-`let-math` `val`, built out of
        // `List.fold-left`/`math-concat`) called from OUTSIDE the module —
        // proving the module's own qualified-export machinery
        // (`elaborate.rs`'s `export_alias`, producing the `"Math.join"`
        // binding) resolves from a caller's perspective, not just that
        // `math.satyh`'s bindings evaluate internally to themselves.
        let src = "@require: math
in
match Math.join (math-char MathOrd `,`) [] with
| _ -> 1";
        let v = compile_via_loader("require-math-join", src).expect("math.satyh should compile");
        assert_eq!(as_int(v), 1);
    });
}
