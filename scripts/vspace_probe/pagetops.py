#!/usr/bin/env python3
"""Per-page FIRST and LAST text baseline of two PDFs, side by side.

Baselines come from the content stream (see baselines.py), so they are exact
and directly comparable between the two engines — unlike `pdftotext -bbox`
glyph boxes. The first baseline of a page is where the page's text area
effectively starts; comparing it engine-to-engine isolates page-TOP handling
from everything that happens further down.

Usage: pagetops.py PORT.pdf REF.pdf [--skip-x X]   (X = running-head/footer x)
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from baselines import parse_objects, page_objects, runs_of, stream_bytes  # noqa: E402


def page_rows(pdf: Path) -> list[list[float]]:
    data = pdf.read_bytes()
    objs = parse_objects(data)
    out = []
    for h, cs in page_objects(data, objs):
        content = b"".join(stream_bytes(objs.get(n, b"")) or b"" for n in cs)
        ys = sorted({round(h - y, 3) for _, y, _ in runs_of(content) if 0 < h - y < h})
        out.append(ys)
    return out


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("port", type=Path)
    ap.add_argument("ref", type=Path)
    ap.add_argument("--head-below", type=float, default=60.0, help="ignore baselines above this (running head)")
    ap.add_argument("--foot-above", type=float, default=760.0, help="ignore baselines below this (folio)")
    args = ap.parse_args()

    p = page_rows(args.port)
    r = page_rows(args.ref)
    print(f"port pages={len(p)}  ref pages={len(r)}")
    print(" pg   port_first  ref_first   d_first   port_last   ref_last    d_last  port_n ref_n")
    for i in range(max(len(p), len(r))):
        pa = [y for y in (p[i] if i < len(p) else []) if args.head_below < y < args.foot_above]
        ra = [y for y in (r[i] if i < len(r) else []) if args.head_below < y < args.foot_above]
        if not pa or not ra:
            print(f"{i:>3}   (missing)")
            continue
        print(
            f"{i:>3} {pa[0]:>11.3f} {ra[0]:>10.3f} {pa[0]-ra[0]:>9.3f} "
            f"{pa[-1]:>11.3f} {ra[-1]:>10.3f} {pa[-1]-ra[-1]:>9.3f} {len(pa):>7} {len(ra):>5}"
        )
    return 0


if __name__ == "__main__":
    sys.exit(main())
