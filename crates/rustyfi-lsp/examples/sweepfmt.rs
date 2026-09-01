//! Scratch: format every corpus file into <outdir>, mirroring the path.
use rustyfi_lsp::{format_cst_outcome, CstOptions, CstOutcome, RustyfiVersion};
use std::path::{Path, PathBuf};

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut e: Vec<_> = rd.flatten().map(|e| e.path()).collect();
    e.sort();
    for p in e {
        if p.is_dir() {
            collect(&p, out);
        } else if matches!(
            p.extension().and_then(|s| s.to_str()),
            Some("saty") | Some("satyh") | Some("satyg")
        ) {
            out.push(p);
        }
    }
}

fn main() {
    let outdir = PathBuf::from(std::env::args().nth(1).unwrap());
    for (roots, v) in [
        (
            vec!["lib-rustyfi/dist/packages", "layout-tests/corpus"],
            RustyfiVersion::V0_0,
        ),
        (vec!["lib-rustyfi/dist-v01/packages"], RustyfiVersion::V0_1),
    ] {
        let mut files = Vec::new();
        for r in roots {
            collect(Path::new(r), &mut files);
        }
        for f in files {
            let Ok(src) = std::fs::read_to_string(&f) else { continue };
            let text = match format_cst_outcome(&src, v, &CstOptions::default()) {
                CstOutcome::Formatted(s) => s,
                CstOutcome::AlreadyFormatted(s) => s,
                _ => continue,
            };
            let dst = outdir.join(&f);
            std::fs::create_dir_all(dst.parent().unwrap()).unwrap();
            std::fs::write(dst, text).unwrap();
        }
    }
}
