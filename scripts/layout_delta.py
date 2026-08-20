#!/usr/bin/env python3
"""Where does a corpus document's vertical space actually diverge?

`layout_fidelity.py` reports one number per document; `layout_probe.py` isolates
one construct in a hand-written probe. This is the middle: given the port's and
upstream's PDFs of the SAME document, it groups words into LINES, aligns the two
line sequences by text, and reports the per-gap difference in baseline advance
(`dy_port - dy_upstream`) for every consecutive aligned pair that lands on the
same page in both files.

Only same-page pairs are usable — a pair straddling a page break measures the
page geometry, not the construct — but that is enough: page breaks are where the
two files disagree, and every gap on either side of one is still measured.

    scripts/layout_delta.py a.port.pdf a.satysfi.pdf            # top offenders
    scripts/layout_delta.py a.port.pdf a.satysfi.pdf --all      # every gap

The `dy` here is a DIFFERENCE of one engine's own numbers, so the font-descriptor
trap that invalidates comparing a latin word's absolute `yMin` across engines
does not apply to the aggregate — but it DOES still apply per row when the two
lines use different faces, so rows are tagged with the y-extent of the words
involved and a persistent per-face offset shows up as a constant, not a spike.
"""

from __future__ import annotations

import argparse
import difflib
import re
import shutil
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path

PAGE_RE = re.compile(r'<page width="([\d.]+)" height="([\d.]+)"')
WORD_RE = re.compile(
    r'<word xMin="([\d.]+)" yMin="([\d.]+)" xMax="([\d.]+)" yMax="([\d.]+)">(.*?)</word>'
)


CJK_RE = re.compile(r"[　-ヿ㐀-鿿＀-￯]")
FOLIO = re.compile(r"^[—\-—–\s\d]+$")


def is_cjk(text: str) -> bool:
    """A line typeset wholly in the japanese face. Mixed lines are excluded on
    purpose: a latin run inside them re-introduces the descriptor offset."""
    core = [c for c in text if not c.isspace()]
    return bool(core) and all(CJK_RE.match(c) or c in "、。，．・「」（）" for c in core)


@dataclass
class Line:
    page: int
    y: float          # representative yMin (the min over the line's words)
    ymax: float
    text: str


def lines_of(pdf: Path) -> list[Line]:
    pdftotext = shutil.which("pdftotext") or sys.exit("pdftotext not found")
    out = subprocess.run(
        [pdftotext, "-bbox", str(pdf), "-"], capture_output=True, text=True, timeout=300
    ).stdout
    page = 0
    cur: list[tuple[float, float, float, str]] = []
    res: list[Line] = []

    def flush():
        nonlocal cur
        if cur:
            cur.sort(key=lambda w: w[0])
            res.append(
                Line(page, min(w[1] for w in cur), max(w[2] for w in cur),
                     "".join(w[3] for w in cur))
            )
            cur = []

    rows: list[tuple[int, float, float, float, str]] = []
    for ln in out.splitlines():
        if PAGE_RE.search(ln):
            page += 1
            continue
        m = WORD_RE.search(ln)
        if m:
            x0, y0, _x1, y1, txt = m.groups()
            rows.append((page, float(x0), float(y0), float(y1), txt))
    # group by (page, y) with a small tolerance: words on one baseline share yMin
    rows.sort(key=lambda r: (r[0], round(r[2], 1), r[1]))
    key = None
    for pg, x0, y0, y1, txt in rows:
        k = (pg, round(y0, 1))
        if k != key:
            flush()
            page = pg
            key = k
        cur.append((x0, y0, y1, txt))
    flush()
    res.sort(key=lambda l: (l.page, l.y))
    return res


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("port", type=Path)
    ap.add_argument("upstream", type=Path)
    ap.add_argument("--all", action="store_true")
    ap.add_argument("--eps", type=float, default=0.05, help="ignore |ddy| below this")
    ap.add_argument(
        "--cjk",
        action="store_true",
        help="only gaps whose BOTH endpoints are CJK lines — the one comparison "
        "the differing font descriptors cannot contaminate",
    )
    ap.add_argument("--no-folio", action="store_true", help="drop gaps touching a page number")
    args = ap.parse_args()

    p = lines_of(args.port)
    u = lines_of(args.upstream)
    print(f"port lines={len(p)} pages={p[-1].page}   upstream lines={len(u)} pages={u[-1].page}")

    sm = difflib.SequenceMatcher(None, [l.text for l in p], [l.text for l in u], autojunk=False)
    pairs: list[tuple[int, int]] = []
    for a, b, n in sm.get_matching_blocks():
        for k in range(n):
            pairs.append((a + k, b + k))
    print(f"aligned lines: {len(pairs)}")

    # Whole-document median |dy|: for every aligned line that landed on the SAME
    # page in both files, how far apart are the two baselines? One number for
    # "how well do the two documents line up", complementing `text_match` (which
    # is text-order based and can stay flat while geometry moves).
    same_page = [abs(p[a].y - u[b].y) for a, b in pairs if p[a].page == u[b].page]
    if same_page:
        same_page.sort()
        med = same_page[len(same_page) // 2]
        print(
            f"same-page aligned lines: {len(same_page)}/{len(pairs)}   "
            f"median |dy| = {med:.2f}pt   mean = {sum(same_page) / len(same_page):.2f}pt"
        )

    rows = []
    for (pa, ua), (pb, ub) in zip(pairs, pairs[1:]):
        lpa, lpb = p[pa], p[pb]
        lua, lub = u[ua], u[ub]
        if lpa.page != lpb.page or lua.page != lub.page:
            continue
        if args.cjk and not (is_cjk(lpa.text) and is_cjk(lpb.text)):
            continue
        if args.no_folio and any(FOLIO.match(t) for t in (lpa.text, lpb.text)):
            continue
        ddy = (lpb.y - lpa.y) - (lub.y - lua.y)
        rows.append((ddy, lpa.page, lpa.text, lpb.text, lpb.y - lpa.y, lub.y - lua.y))

    tot = sum(r[0] for r in rows)
    big = [r for r in rows if abs(r[0]) >= args.eps]
    print(f"same-page gaps: {len(rows)}   sum(ddy)={tot:+.2f}pt   |ddy|>={args.eps}: {len(big)}")

    show = rows if args.all else sorted(big, key=lambda r: -abs(r[0]))[:60]
    print(f"{'ddy':>8} {'pg':>3}  {'port dy':>8} {'up dy':>8}  from -> to")
    for ddy, pg, ta, tb, dp, du in show:
        print(f"{ddy:>8.2f} {pg:>3}  {dp:>8.2f} {du:>8.2f}  {ta[:34]!r} -> {tb[:34]!r}")

    # Bucket by rounded value: a systematic per-construct offset shows up as a
    # tall bucket, a one-off as a lone row.
    from collections import Counter
    c = Counter(round(r[0], 2) for r in big)
    print("\ntop recurring ddy values (value: count, total):")
    for val, n in c.most_common(15):
        print(f"  {val:+7.2f} x{n:<4} = {val * n:+8.2f}pt")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
