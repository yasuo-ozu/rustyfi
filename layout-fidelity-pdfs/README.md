# Layout-fidelity comparison renders

Side-by-side PDF renders of each corpus document, for eyeballing how the Rust
port's layout compares to the **original OCaml SATySFi** (the fidelity the
`layout-fidelity` test measures — see `docs/plans/design-layout-fidelity.md`).

For each document:

- `<doc>.port.pdf` — rendered by this Rust port.
- `<doc>.satysfi.pdf` — rendered by the original SATySFi 0.0.11 (from `flake.nix`).

`gakushin` has no `.satysfi.pdf`: official SATySFi needs the `fonts-junicode`
Satyrographos font package, which the test's `-C` package staging can't provide,
so it is compared only against the port's own baseline (self-snapshot).

Regenerate (needs `nix develop` for the original SATySFi):

```
nix develop -c python3 scripts/layout_fidelity.py --gen-refs --keep-going \
    --out-dir layout-fidelity-pdfs
```

These are committed via a `.gitignore` exception (the repo otherwise ignores
`*.pdf`). The biggest is `figbox.port.pdf` (~20.5 MB against SATySFi's 6.4 MB),
because **the port writes non-JPEG images completely uncompressed**: the
`else` branch of the image-XObject writer (`crates/rustyfi-pdf/src/lib.rs:209`)
emits `im.samples` as raw 8-bit `DeviceRGB` with no `/Filter` at all. figbox
holds 13 `DCTDecode` streams (the JPEG passthrough, working) and 13 unfiltered
ones; that single uncompressed image, embedded 13×, is 16 MB of the 20.5.

Duplicate embedding is NOT the cause and not a port bug — SATySFi duplicates
harder (44 image streams over 2 unique, vs the port's 26 over 2). The whole
difference is per-copy size: 0.71 MB each for the port, 0.115 MB for SATySFi.
Deflating those samples (`flate2` is already a workspace dependency) would be
pixel-lossless and bring the file to rough parity.
