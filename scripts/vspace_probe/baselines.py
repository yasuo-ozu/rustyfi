#!/usr/bin/env python3
"""Exact text BASELINES straight out of a PDF's content stream.

`pdftotext -bbox` reports a word's GLYPH BOX, whose top/bottom come from the
font descriptor — and the two engines emit different descriptors for the same
latin glyphs, so those boxes are not comparable across engines (CLAUDE.md's
measurement trap). The text-positioning operators are: `BT ... Tm/Td ... Tj`
places a run at an exact baseline in PDF user space, identically for both
writers.

This walks each page's (possibly Flate-compressed) content stream with a tiny
tokenizer, tracks the text matrix, and prints one row per distinct baseline:

    y_from_top   dy_from_previous   x_of_first_run   fonts   text-ish

`y_from_top` = page_height - baseline_y, so it reads the same way as
`pdftotext -bbox` output (increasing downward).

Usage: baselines.py FILE.pdf [--page N]... [--min-dy 0.01]
"""

from __future__ import annotations

import argparse
import re
import sys
import zlib
from pathlib import Path


def parse_objects(data: bytes) -> dict[int, bytes]:
    """Map object number -> raw object body (between `obj` and `endobj`)."""
    objs: dict[int, bytes] = {}
    for m in re.finditer(rb"(\d+)\s+(\d+)\s+obj\b", data):
        num = int(m.group(1))
        start = m.end()
        end = data.find(b"endobj", start)
        if end < 0:
            continue
        objs[num] = data[start:end]
    return objs


def stream_bytes(body: bytes) -> bytes | None:
    i = body.find(b"stream")
    if i < 0:
        return None
    j = i + len(b"stream")
    if body[j : j + 2] == b"\r\n":
        j += 2
    elif body[j : j + 1] in (b"\n", b"\r"):
        j += 1
    k = body.rfind(b"endstream")
    raw = body[j:k] if k > 0 else body[j:]
    if b"FlateDecode" in body[:i]:
        try:
            return zlib.decompress(raw)
        except Exception:
            try:
                return zlib.decompressobj().decompress(raw)
            except Exception:
                return None
    return raw


def page_order(objs: dict[int, bytes]) -> list[int]:
    """Page object numbers in DOCUMENT order, by walking /Type /Pages /Kids.
    Object-number order is NOT document order (upstream SATySFi emits them
    out of order), and reading the wrong page is a silent, total wrong answer."""
    kids: dict[int, list[int]] = {}
    roots: list[int] = []
    is_page: set[int] = set()
    for num, body in objs.items():
        if re.search(rb"/Type\s*/Pages\b", body):
            m = re.search(rb"/Kids\s*\[(.*?)\]", body, re.S)
            kids[num] = [int(x) for x in re.findall(rb"(\d+)\s+\d+\s+R", m.group(1))] if m else []
            roots.append(num)
        elif re.search(rb"/Type\s*/Page\b(?!s)", body):
            is_page.add(num)
    if not kids:
        return sorted(is_page)
    # The root Pages node is the one nobody lists as a kid.
    child_of_someone = {c for v in kids.values() for c in v}
    tops = [n for n in roots if n not in child_of_someone] or roots[:1]
    out: list[int] = []
    seen: set[int] = set()

    def walk(n: int) -> None:
        if n in seen:
            return
        seen.add(n)
        if n in kids:
            for c in kids[n]:
                walk(c)
        elif n in is_page:
            out.append(n)

    for t in tops:
        walk(t)
    for n in sorted(is_page):
        if n not in out:
            out.append(n)
    return out


def page_objects(data: bytes, objs: dict[int, bytes]) -> list[tuple[float, list[int]]]:
    """[(page_height, [content-stream obj nums])] in document order."""
    out = []
    for num in page_order(objs):
        body = objs[num]
        mb = re.search(rb"/MediaBox\s*\[\s*([\d.\-]+)\s+([\d.\-]+)\s+([\d.\-]+)\s+([\d.\-]+)", body)
        h = float(mb.group(4)) - float(mb.group(2)) if mb else 841.89
        cm = re.search(rb"/Contents\s+(\d+)\s+\d+\s+R", body)
        cs: list[int] = []
        if cm:
            cs = [int(cm.group(1))]
        else:
            cm2 = re.search(rb"/Contents\s*\[([^\]]*)\]", body)
            if cm2:
                cs = [int(x) for x in re.findall(rb"(\d+)\s+\d+\s+R", cm2.group(1))]
        out.append((h, cs))
    return out


TOKEN = re.compile(rb"(\([^)]*\)|<[0-9A-Fa-f\s]*>|/[^\s/\[\]<>()]+|\[|\]|[-+]?[\d.]+|[A-Za-z'\"*]+)")


def runs_of(content: bytes) -> list[tuple[float, float, str]]:
    """[(x, y, font)] for every text-showing operator, in stream order."""
    toks = [m.group(1) for m in TOKEN.finditer(content)]
    out: list[tuple[float, float, str]] = []
    stack: list[bytes] = []
    tm = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]
    tlm = list(tm)
    font = "?"

    def num(b: bytes) -> float:
        try:
            return float(b)
        except ValueError:
            return 0.0

    for t in toks:
        if t in (b"Tj", b"TJ", b"'", b'"'):
            out.append((tm[4], tm[5], font))
            stack.clear()
            continue
        if t == b"Tf":
            if len(stack) >= 2 and stack[-2].startswith(b"/"):
                font = stack[-2].decode("latin1")
            stack.clear()
            continue
        if t == b"Tm":
            if len(stack) >= 6:
                tm = [num(x) for x in stack[-6:]]
                tlm = list(tm)
            stack.clear()
            continue
        if t in (b"Td", b"TD"):
            if len(stack) >= 2:
                dx, dy = num(stack[-2]), num(stack[-1])
                tlm = [tlm[0], tlm[1], tlm[2], tlm[3], tlm[4] + dx, tlm[5] + dy]
                tm = list(tlm)
            stack.clear()
            continue
        if t == b"T*":
            tlm = [tlm[0], tlm[1], tlm[2], tlm[3], tlm[4], tlm[5]]
            tm = list(tlm)
            stack.clear()
            continue
        if t in (b"BT",):
            tm = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]
            tlm = list(tm)
            stack.clear()
            continue
        stack.append(t)
        if len(stack) > 8:
            del stack[0]
    return out


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("pdf", type=Path)
    ap.add_argument("--page", type=int, action="append", default=[])
    ap.add_argument("--tol", type=float, default=0.05, help="baselines within this are one row")
    args = ap.parse_args()

    data = args.pdf.read_bytes()
    objs = parse_objects(data)
    pages = page_objects(data, objs)
    want = args.page or list(range(len(pages)))
    for pi in want:
        if pi >= len(pages):
            continue
        h, cs = pages[pi]
        content = b"".join(stream_bytes(objs.get(n, b"")) or b"" for n in cs)
        rs = runs_of(content)
        rows: list[tuple[float, float, list[str]]] = []
        for x, y, f in rs:
            yt = h - y
            if rows and abs(rows[-1][0] - yt) <= args.tol:
                if f not in rows[-1][2]:
                    rows[-1][2].append(f)
                continue
            rows.append((yt, x, [f]))
        rows.sort(key=lambda r: r[0])
        print(f"=== page {pi} (h={h}) — {len(rows)} baselines ===")
        prev = None
        for yt, x, fs in rows:
            d = "" if prev is None else f"{yt - prev:+8.3f}"
            print(f"  y={yt:9.3f} d={d:>9} x={x:8.2f}  {','.join(fs)}")
            prev = yt
    return 0


if __name__ == "__main__":
    sys.exit(main())
