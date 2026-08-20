#!/usr/bin/env python3
"""Cross-version import sweep: real Satyrographos packages, `@require:`d from a
0.1 document against a lib root that holds ONLY the 0.0.6 corpus.

Why this exists
---------------
The cross-version bridge (`CLAUDE.md` §1) was built against hand-written
fixtures, each of which is one file, declares no `let-math`, and keeps its
decorations at the export boundary. Measuring it against packages people
actually publish found four bugs those fixtures structurally could not see —
and then a fifth (X3c: a 0.0.6 package consuming ANOTHER 0.0.6 package's
crossed `deco`/`paren`), which needs two packages in one document to appear at
all. That measurement had no re-runnable form, so its headline number could not
be re-checked. This is that form.

What it measures
----------------
For each package, two compiles:

* the CROSSING case — a minimal **0.1** document (`cases/NAME.saty`) that
  `@require:`s the package. The lib root contains `dist/` only, never
  `dist-v01/`, so `v006::resolve::resolve_require`'s same-generation preference
  has nothing to prefer and every `@require:` genuinely crosses.
* the **0.0.6 CONTROL** (`v006/NAME.saty`) — the same package exercised the
  same way from an ordinary 0.0.6 document. This is what separates a BRIDGE
  failure from a pre-existing 0.0.6-side gap, and it reclassified 5 of the 22
  cases in the original audit. Never read a `FAIL` without its control.

Usage
-----
    python3 scripts/xver_sweep.py                 # assemble, install, sweep, check
    python3 scripts/xver_sweep.py --root DIR      # keep the assembled root around
    python3 scripts/xver_sweep.py --offline       # no network (needs a warm cache)
    python3 scripts/xver_sweep.py --only siunitx --only latexcmds
    python3 scripts/xver_sweep.py --update-baseline

Exit status is 0 when every case matches `xver_sweep_baseline.json`, 1 on any
drift (in either direction — an unexpected PASS is reported too, so the
baseline stays honest about what works).
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parent
FIXTURES = HERE / "xver_sweep"
BASELINE = HERE / "xver_sweep_baseline.json"
LIB_RUSTYFI = REPO / "lib-rustyfi"


# Registry package names to install, in dependency-friendly order. `math`,
# `stdjabook`, `gr`, ... are NOT here: they are the port's own bundled 0.0.6
# corpus, copied from `lib-rustyfi/dist/packages` before anything is fetched.
PACKAGES = [
    "base",
    "fss",
    "algorithm",
    "arrows",
    "azmath",
    "chemfml",
    "code-printer",
    "colorbox",
    "derive",
    "easytable",
    "enumitem",
    "figbox",
    "latexcmds",
    "lipsum",
    "matrixcd",
    "pagenumber",
    "quotation",
    "railway",
    "ruby",
    "siunitx",
    "texlogo",
    "uline",
]


def default_bin() -> Path:
    return REPO / "target" / "debug" / "rustyfi"


def find_font() -> str | None:
    """A real TTF to set text in, so a failure is never a font-discovery one.

    Any serif face does; the sweep only ever asks whether a document COMPILES.
    Falls back to the built-in base-14 fonts when nothing is found.
    """
    env = os.environ.get("RUSTYFI_SWEEP_FONT")
    if env and Path(env).exists():
        return env
    candidates = [
        "/usr/share/fonts/truetype/dejavu/DejaVuSerif.ttf",
        "/usr/share/fonts/dejavu/DejaVuSerif.ttf",
        "/usr/share/fonts/TTF/DejaVuSerif.ttf",
    ]
    for c in candidates:
        if Path(c).exists():
            return c
    for root in (Path("/nix/store"), Path("/usr/share/fonts")):
        if not root.exists():
            continue
        try:
            hit = next(root.glob("**/DejaVuSerif.ttf"), None)
        except OSError:
            hit = None
        if hit:
            return str(hit)
    return None


def assemble_root(root: Path, bin_path: Path, offline: bool, quiet: bool) -> None:
    """`<root>/dist/packages/` = the port's bundled 0.0.6 corpus + registry installs.

    `dist-v01/` is deliberately absent. That is the whole point: with both
    corpora present, `resolve_require` prefers the requesting file's own
    generation and a 0.1 document quietly stops crossing while still passing
    (the trap `xver_capstone.rs` documents).
    """
    pkg = root / "dist" / "packages"
    pkg.mkdir(parents=True, exist_ok=True)
    src_pkg = LIB_RUSTYFI / "dist" / "packages"
    for entry in src_pkg.iterdir():
        target = pkg / entry.name
        if entry.is_dir():
            shutil.copytree(entry, target, dirs_exist_ok=True)
        else:
            shutil.copy2(entry, target)
    for name in PACKAGES:
        cmd = [
            str(bin_path),
            "install",
            "--config",
            str(REPO / "config.toml"),
            name,
            "--dest",
            str(root),
            "--force",
        ]
        if offline:
            cmd.append("--offline")
        res = subprocess.run(cmd, capture_output=True, text=True, timeout=900)
        if res.returncode != 0:
            print(f"install {name}: FAILED\n{res.stdout}{res.stderr}", file=sys.stderr)
            raise SystemExit(2)
        if not quiet:
            print(f"  installed {name}")


def compile_one(bin_path: Path, doc: Path, root: Path, lang: str, font: str | None) -> tuple[bool, str]:
    cmd = [str(bin_path), str(doc), "--lang", lang, "--lib-root", str(root), "--no-cache"]
    if font:
        cmd += ["--font", font]
    try:
        res = subprocess.run(cmd, capture_output=True, text=True, timeout=600)
    except subprocess.TimeoutExpired:
        return False, "timed out"
    out = (res.stdout or "") + (res.stderr or "")
    if "output written on" in out:
        return True, ""
    # The first `Error:`/`error:` line is the whole diagnosis for every failure
    # this sweep has ever seen; keep it, drop the render log above it.
    for line in out.splitlines():
        s = line.strip()
        if s.lower().startswith("error"):
            return False, s
    return False, out.strip().splitlines()[-1] if out.strip() else "no output"


def short(msg: str, root: Path | None = None, limit: int = 220) -> str:
    """One line, with the (long, temp-dir-flavoured) document and lib-root paths
    stripped, so two runs' output diffs cleanly."""
    msg = " ".join(msg.split())
    msg = msg.replace("Error: ", "", 1)
    if ".saty: " in msg:
        msg = msg.split(".saty: ", 1)[1]
    if root is not None:
        msg = msg.replace(str(root) + "/", "<root>/")
    return msg if len(msg) <= limit else msg[: limit - 1] + "…"


def run(args: argparse.Namespace) -> int:
    bin_path = Path(args.bin) if args.bin else default_bin()
    if not bin_path.exists():
        print(f"port binary not built at {bin_path} (run `cargo build -p rustyfi`)", file=sys.stderr)
        return 2

    tmp: tempfile.TemporaryDirectory | None = None
    if args.root:
        root = Path(args.root).resolve()
    else:
        tmp = tempfile.TemporaryDirectory(prefix="rustyfi-xver-sweep-")
        root = Path(tmp.name) / "root"

    names = sorted(p.stem for p in (FIXTURES / "cases").glob("*.saty"))
    if args.only:
        names = [n for n in names if n in set(args.only)]
        if not names:
            print(f"no case matches {args.only}", file=sys.stderr)
            return 2

    if not (root / "dist" / "packages").exists() or args.reinstall:
        if not args.quiet:
            print(f"assembling lib root at {root}")
        assemble_root(root, bin_path, args.offline, args.quiet)
    elif not args.quiet:
        print(f"reusing lib root at {root} (pass --reinstall to rebuild)")

    font = find_font()
    if not args.quiet and font:
        print(f"font: {font}")

    # Compile in a scratch copy so the committed fixtures never collect
    # `.pdf`/`.satysfi-aux` siblings.
    work = Path(tempfile.mkdtemp(prefix="rustyfi-xver-docs-"))
    shutil.copytree(FIXTURES, work / "fx", dirs_exist_ok=True)

    results: dict[str, dict[str, object]] = {}
    for name in names:
        case = work / "fx" / "cases" / f"{name}.saty"
        ctl = work / "fx" / "v006" / f"{name}.saty"
        ok, msg = compile_one(bin_path, case, root, "0.1", font)
        cok, cmsg = compile_one(bin_path, ctl, root, "0.0", font) if ctl.exists() else (None, "")
        results[name] = {"cross": ok, "cross_err": msg, "control": cok, "control_err": cmsg}

    shutil.rmtree(work, ignore_errors=True)

    if args.update_baseline:
        BASELINE.write_text(
            json.dumps({k: {"cross": v["cross"], "control": v["control"]} for k, v in results.items()}, indent=2, sort_keys=True)
            + "\n"
        )
        print(f"wrote {BASELINE}")

    base = json.loads(BASELINE.read_text()) if BASELINE.exists() else {}
    drift: list[str] = []
    crossing = sum(1 for v in results.values() if v["cross"])
    controls = sum(1 for v in results.values() if v["control"])

    width = max(len(n) for n in results) if results else 0
    for name, v in sorted(results.items()):
        cross = "CROSS " if v["cross"] else "refuse"
        control = "ok  " if v["control"] else "FAIL"
        print(f"{name:<{width}}  xver={cross}  v0.0.6={control}")
        if not v["cross"]:
            print(f"{'':<{width}}    xver : {short(str(v['cross_err']), root)}")
        if v["control"] is False:
            # Printed for every failing control, not only when the crossing
            # also failed: a package whose plain-0.0.6 compile is broken was
            # never the boundary's to fix, and that is the single most
            # misread line in the original audit.
            print(f"{'':<{width}}    0.0.6: {short(str(v['control_err']), root)}")
        want = base.get(name)
        if want is not None:
            if bool(want.get("cross")) != bool(v["cross"]):
                drift.append(
                    f"{name}: crossing {'REGRESSED' if want.get('cross') else 'now PASSES'} "
                    f"(baseline {want.get('cross')}, got {v['cross']})"
                )
            if bool(want.get("control")) != bool(v["control"]):
                drift.append(
                    f"{name}: 0.0.6 control {'REGRESSED' if want.get('control') else 'now PASSES'} "
                    f"(baseline {want.get('control')}, got {v['control']})"
                )
        elif base:
            drift.append(f"{name}: not in the baseline")

    print(f"\ncrossing {crossing}/{len(results)}   0.0.6 controls {controls}/{len(results)}")
    if drift:
        print("\nDRIFT vs " + str(BASELINE) + ":")
        for d in drift:
            print("  " + d)
        print("\n(re-record with --update-baseline once the change is intended)")
        return 1
    if base:
        print(f"matches {BASELINE.name}")
    if tmp:
        tmp.cleanup()
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--bin", help="rustyfi binary (default target/debug/rustyfi)")
    ap.add_argument("--root", help="lib root to assemble into and reuse (default: a temp dir)")
    ap.add_argument("--reinstall", action="store_true", help="re-run the registry installs even if --root already has them")
    ap.add_argument("--offline", action="store_true", help="pass --offline to every install (needs a warm archive cache)")
    ap.add_argument("--only", action="append", help="restrict to this package (repeatable)")
    ap.add_argument("--update-baseline", action="store_true", help="rewrite xver_sweep_baseline.json from this run")
    ap.add_argument("--quiet", action="store_true")
    return run(ap.parse_args())


if __name__ == "__main__":
    sys.exit(main())
