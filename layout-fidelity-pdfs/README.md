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
`*.pdf`). The biggest is `figbox.port.pdf` (~14 MB) — the port currently embeds
that manual's images far less compactly than SATySFi (6.4 MB), itself a
fidelity lead.
