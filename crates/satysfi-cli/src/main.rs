use anyhow::Context as _;
use clap::Parser;
use std::path::PathBuf;

/// Compile a SATySFi (.saty) document to PDF.
#[derive(Parser)]
#[command(name = "satysfi-rust", version)]
struct Args {
    /// Input .saty file.
    input: PathBuf,

    /// Output PDF path (defaults to the input with a .pdf extension).
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Library root for `@require:` resolution (packages under
    /// `<lib-root>/dist/packages/`). Falls back to $SATYSFI_LIB_ROOT.
    #[arg(long)]
    lib_root: Option<PathBuf>,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let output = args
        .output
        .unwrap_or_else(|| args.input.with_extension("pdf"));
    let lib_root = args
        .lib_root
        .or_else(|| std::env::var_os("SATYSFI_LIB_ROOT").map(PathBuf::from));

    let program = satysfi_loader::load(&args.input, &satysfi_loader::LoadOptions { lib_root })
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let merged = merge_program(program);

    let metrics = satysfi_pdf::Base14Metrics;
    let doc = satysfi_lang::compile_document_cst(&merged, &metrics)
        .map_err(|e| anyhow::anyhow!("{}: {e}", args.input.display()))?;

    let bytes = satysfi_pdf::render_pdf(&doc.geometry, &doc.pages)?;
    std::fs::write(&output, bytes)
        .with_context(|| format!("cannot write {}", output.display()))?;

    let line_count: usize = doc.pages.iter().map(|p| p.lines.len()).sum();
    eprintln!(
        " ---- ---- ---- ----\n  output written on {} ({} page(s), {} line(s)).",
        output.display(),
        doc.pages.len(),
        line_count
    );
    Ok(())
}

/// Concatenate the dependency-ordered library preludes ahead of the entry
/// document's own prelude, producing one synthetic file for elaboration.
/// (The v0.0.6 analog type-checks each library into a shared environment in
/// dependency order; untyped elaboration gets the same scoping by prelude
/// concatenation.)
fn merge_program(program: satysfi_loader::LoadedProgram) -> satysfi_syntax::cst::File {
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
