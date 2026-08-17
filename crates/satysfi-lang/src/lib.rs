//! Abstract syntax tree, elaboration, evaluator, and primitives — the
//! language core of the SATySFi port.

pub mod ast;
pub(crate) mod compile;
pub mod elaborate;
pub mod eval;
pub mod prim_types;
pub mod primitives;
pub mod typecheck;
pub mod types;
pub mod unify;
pub mod value;

use satysfi_backend::FontMetrics;
use value::{DocumentValue, Value};

#[derive(Debug, thiserror::Error)]
pub enum CompileError {
    #[error(transparent)]
    Parse(#[from] satysfi_syntax::ParseFileError),
    #[error(transparent)]
    Elaborate(#[from] elaborate::ElabError),
    #[error(transparent)]
    Type(#[from] typecheck::TypeError),
    #[error(transparent)]
    Eval(#[from] eval::EvalError),
    #[error("the file's expression evaluated to {0}, not a document")]
    NotADocument(&'static str),
}

/// Compile a `.saty` source string down to a typeset document:
/// lex → parse → elaborate → evaluate.
pub fn compile_document(
    src: &str,
    metrics: &dyn FontMetrics,
) -> Result<std::rc::Rc<DocumentValue>, CompileError> {
    let file = satysfi_syntax::parse_file(src)?;
    compile_document_cst(&file, metrics)
}

/// Compile an already-parsed (possibly loader-merged) file. The multi-file
/// loader concatenates library preludes into one synthetic `cst::File` and
/// enters here.
pub fn compile_document_cst(
    file: &satysfi_syntax::cst::File,
    metrics: &dyn FontMetrics,
) -> Result<std::rc::Rc<DocumentValue>, CompileError> {
    let env = primitives::base_env();
    let scope = elaborate::Scope::new(env.names());
    let program = elaborate::elaborate_program(file, &scope)?;
    typecheck::typecheck(&program)?;
    let mut interp = eval::Interp::new(metrics);
    // Compile the elaborated body into a closure tree once, then run it. This
    // is the fast path; `eval::Interp::eval` remains the reference tree-walker
    // (used, e.g., in unit tests to cross-check that both produce identical
    // values). See `compile.rs` for the design and correctness argument.
    let compiled = compile::compile_program(&program.body, &env);
    match compiled.run(&env, &mut interp)? {
        Value::Document(doc) => Ok(doc),
        other => Err(CompileError::NotADocument(other.type_name())),
    }
}
