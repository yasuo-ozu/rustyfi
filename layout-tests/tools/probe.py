#!/usr/bin/env python3
"""Render ONE minimal `.saty` probe with BOTH engines and print the word boxes
side by side.

`layout-tests/fidelity.py` measures whole corpus documents; that is the right
granularity for a regression gate and the wrong one for a diagnosis, because a
whole document composes dozens of constructs and any one of them can absorb a
few points. This driver is the diagnostic counterpart: point it at a probe that
exercises a SINGLE construct and it reports, per word, where the port put it and
where upstream SATySFi put it.

    layout-tests/tools/probe.py layout-tests/probes/code_block.saty
    layout-tests/tools/probe.py --port-only ...       # skip the `satysfi` run
    layout-tests/tools/probe.py --keep out/           # keep both PDFs

MEASUREMENT TRAP, and it has already produced one false finding: the two writers
emit DIFFERENT font descriptors for identical glyphs at identical size, so a
LATIN word's reported `yMin`/`yMax` can differ by ~7.7pt with the baselines
identical (figbox p1's "1." reports height 22.02pt in the port and 32.19pt
upstream). Never compare a latin word's absolute yMin across engines. Compare
either CJK words (the descriptors agree there: 19.36 vs 19.34) or DIFFERENCES
of the same engine's own numbers — which is what the `dy` column below is:
each row's advance from the PREVIOUS row, per engine. That quantity is
box-independent, so a mismatch in it is a real layout divergence.
"""

from __future__ import annotations

import argparse
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from fidelity import (  # noqa: E402
    DOCS,
    LIB_RUSTYFI,
    assemble_lib_root,
    assemble_satysfi_lib_root,
    extract_pages,
)

REPO = Path(__file__).resolve().parent.parent.parent


def find_pdftotext() -> str:
    for name in ("pdftotext",):
        if shutil.which(name):
            return name
    sys.exit("pdftotext not found (poppler)")


def build_port(src: Path, out_pdf: Path, lib_root: Path, bin_path: Path) -> None:
    cmd = [
        str(bin_path),
        "--no-cache",
        "--no-aux",
        "--lib-root",
        str(lib_root),
        "--font-dir",
        str(LIB_RUSTYFI),
        "-o",
        str(out_pdf),
        src.name,
    ]
    proc = subprocess.run(cmd, cwd=src.parent, capture_output=True, text=True, timeout=300)
    if proc.returncode != 0 or not out_pdf.exists():
        sys.exit("port failed:\n" + (proc.stdout + proc.stderr)[-3000:])


def build_satysfi(src: Path, out_pdf: Path, lib_root: Path) -> None:
    """Run the ORIGINAL SATySFi, via `nix develop` when it is not already on
    PATH (the flake pins 0.0.11)."""
    inner = ["satysfi", src.name, "-o", str(out_pdf), "-C", str(lib_root)]
    if shutil.which("satysfi"):
        cmd = inner
    else:
        cmd = ["nix", "develop", str(REPO), "--command", *inner]
    proc = subprocess.run(cmd, cwd=src.parent, capture_output=True, text=True, timeout=900)
    if proc.returncode != 0 or not out_pdf.exists():
        sys.exit("satysfi failed:\n" + (proc.stdout + proc.stderr)[-3000:])


CJK = re.compile(r"[　-鿿＀-￯]")


def rows(pages) -> list[tuple[int, float, float, float, float, str]]:
    """One row per WORD, in reading order: (page, xMin, yMin, xMax, yMax, text)."""
    out = []
    for pi, pg in enumerate(pages, 1):
        for w in pg.words:
            out.append((pi, w.x0, w.y0, w.x1, w.y1, w.text))
    return out


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("probe", type=Path)
    ap.add_argument("--bin", default=str(REPO / "target" / "debug" / "rustyfi"))
    ap.add_argument("--port-only", action="store_true")
    ap.add_argument("--keep", type=Path, default=None, help="dir to keep both PDFs in")
    ap.add_argument("--cjk-only", action="store_true", help="only rows whose text is CJK")
    ap.add_argument(
        "--stage",
        action="append",
        default=[],
        help="also stage this corpus doc's packages into both lib roots "
        "(e.g. --stage figbox), so a probe can exercise a corpus package",
    )
    args = ap.parse_args()

    pdftotext = find_pdftotext()
    probe = args.probe.resolve()
    staged = [d for d in DOCS if d.name in args.stage]

    with tempfile.TemporaryDirectory(prefix="rustyfi-probe-") as tmp:
        tmpd = Path(tmp)
        lib_root = assemble_lib_root(tmpd / "libroot", staged)
        port_pdf = tmpd / "port.pdf"
        build_port(probe, port_pdf, lib_root, Path(args.bin))
        port = rows(extract_pages(port_pdf, pdftotext))

        up = None
        if not args.port_only:
            saty_root = assemble_satysfi_lib_root(tmpd / "satyroot", staged)
            up_pdf = tmpd / "upstream.pdf"
            build_satysfi(probe, up_pdf, saty_root)
            up = rows(extract_pages(up_pdf, pdftotext))

        if args.keep:
            args.keep.mkdir(parents=True, exist_ok=True)
            shutil.copy2(port_pdf, args.keep / (probe.stem + ".port.pdf"))
            if up is not None:
                shutil.copy2(tmpd / "upstream.pdf", args.keep / (probe.stem + ".satysfi.pdf"))

        print(f"probe: {probe}")
        print(f"pages: port={len(set(r[0] for r in port))}", end="")
        if up is not None:
            print(f" upstream={len(set(r[0] for r in up))}")
        else:
            print()

        hdr = f"{'#':>4} {'pg':>2} {'text':<22} {'p.xMin':>8} {'p.yMin':>8} {'p.dy':>7}"
        if up is not None:
            hdr += f" | {'u.xMin':>8} {'u.yMin':>8} {'u.dy':>7} {'ddy':>7}"
        print(hdr)

        n = max(len(port), len(up or []))
        pprev = uprev = None
        for i in range(n):
            p = port[i] if i < len(port) else None
            u = up[i] if up is not None and i < len(up) else None
            if args.cjk_only and p and not CJK.search(p[5]):
                pprev, uprev = (p[2] if p else pprev), (u[2] if u else uprev)
                continue
            pdy = (p[2] - pprev) if (p and pprev is not None) else float("nan")
            udy = (u[2] - uprev) if (u and uprev is not None) else float("nan")
            txt = (p[5] if p else (u[5] if u else ""))[:22]
            line = f"{i:>4} {p[0] if p else 0:>2} {txt:<22}"
            line += f" {p[1]:>8.2f} {p[2]:>8.2f} {pdy:>7.2f}" if p else f" {'-':>8} {'-':>8} {'-':>7}"
            if up is not None:
                if u:
                    line += f" | {u[1]:>8.2f} {u[2]:>8.2f} {udy:>7.2f}"
                    line += f" {pdy - udy:>7.2f}" if p else f" {'-':>7}"
                else:
                    line += f" | {'-':>8} {'-':>8} {'-':>7} {'-':>7}"
            print(line)
            if p:
                pprev = p[2]
            if u:
                uprev = u[2]
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
