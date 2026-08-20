#!/usr/bin/env python3
"""Render one probe `.saty` with BOTH engines and print their text lines
side by side.

  scripts/vspace_probe/run.py FIXTURE.saty [--page N] [--bin PATH]

The port gets the same assembled lib-root `scripts/layout_fidelity.py` builds
(port packages + full satysfi-base + the corpus `enumitem`/`easytable` sources);
the original SATySFi gets a `-C` root holding only the non-stdlib packages, so
it resolves its own stdlib from its default config path — exactly as the
fidelity harness does. Run under `nix develop` so `satysfi` is on PATH.

Only lines from ONE engine are ever compared to lines of the SAME engine unless
you look at pure-CJK rows: the two PDF writers emit different font descriptors
for identical latin glyphs, so a latin word's reported yMin is NOT comparable
across engines (see CLAUDE.md's measurement trap).
"""

from __future__ import annotations

import argparse
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
CORPUS = REPO / "scripts" / "layout_fidelity_corpus"
LIB = REPO / "lib-rustyfi"

# Corpus packages staged under a published prefix, mirroring layout_fidelity.py.
STAGE = {
    "enumitem": "enumitem/src",
    "easytable": "easytable/src",
    "figbox": "figbox/src",
}


def assemble_port_root(dst: Path) -> Path:
    pkg = dst / "dist" / "packages"
    pkg.mkdir(parents=True, exist_ok=True)
    for entry in (LIB / "dist" / "packages").iterdir():
        target = pkg / entry.name
        if entry.is_dir():
            shutil.copytree(entry, target, dirs_exist_ok=True)
        else:
            shutil.copy2(entry, target)
    base = CORPUS / "satysfi-base" / "src"
    if base.exists():
        shutil.copytree(base, pkg / "base", dirs_exist_ok=True)
    for prefix, src in STAGE.items():
        p = CORPUS / src
        if p.exists():
            shutil.copytree(p, pkg / prefix, dirs_exist_ok=True)
    return dst


def assemble_saty_root(dst: Path) -> Path:
    pkg = dst / "dist" / "packages"
    pkg.mkdir(parents=True, exist_ok=True)
    base = CORPUS / "satysfi-base" / "src"
    if base.exists():
        shutil.copytree(base, pkg / "base", dirs_exist_ok=True)
    for prefix, src in STAGE.items():
        p = CORPUS / src
        if p.exists():
            shutil.copytree(p, pkg / prefix, dirs_exist_ok=True)
    return dst


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("fixture", type=Path)
    ap.add_argument("--bin", type=Path, default=REPO / "target" / "debug" / "rustyfi")
    ap.add_argument("--page", type=int, action="append", default=[])
    ap.add_argument("--keep", type=Path, default=None, help="copy both PDFs here")
    ap.add_argument("--port-only", action="store_true")
    args = ap.parse_args()

    src = args.fixture.resolve()
    with tempfile.TemporaryDirectory(prefix="vspace-") as tmp:
        t = Path(tmp)
        port_pdf = t / "port.pdf"
        saty_pdf = t / "saty.pdf"

        cmd = [
            str(args.bin), "--no-cache", "--no-aux",
            "--lib-root", str(assemble_port_root(t / "libroot")),
            "--font-dir", str(LIB),
            "-o", str(port_pdf), src.name,
        ]
        p = subprocess.run(cmd, cwd=src.parent, capture_output=True, text=True, timeout=600)
        if p.returncode != 0 or not port_pdf.exists():
            print("PORT FAILED:\n" + (p.stdout + p.stderr)[-3000:])
            return 1

        ok_saty = False
        if not args.port_only:
            q = subprocess.run(
                ["satysfi", src.name, "-o", str(saty_pdf), "-C", str(assemble_saty_root(t / "satyroot"))],
                cwd=src.parent, capture_output=True, text=True, timeout=600,
            )
            if q.returncode != 0 or not saty_pdf.exists():
                print("SATYSFI FAILED:\n" + (q.stdout + q.stderr)[-3000:])
            else:
                ok_saty = True

        if args.keep:
            args.keep.mkdir(parents=True, exist_ok=True)
            shutil.copy2(port_pdf, args.keep / (src.stem + ".port.pdf"))
            if ok_saty:
                shutil.copy2(saty_pdf, args.keep / (src.stem + ".saty.pdf"))

        pages = []
        for pg in args.page:
            pages += ["--page", str(pg)]
        print("######## PORT ########")
        subprocess.run([sys.executable, str(HERE / "lines.py"), str(port_pdf)] + pages)
        if ok_saty:
            print("######## SATySFi 0.0.11 ########")
            subprocess.run([sys.executable, str(HERE / "lines.py"), str(saty_pdf)] + pages)
    return 0


if __name__ == "__main__":
    sys.exit(main())
