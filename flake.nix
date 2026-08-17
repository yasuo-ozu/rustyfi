{
  # Development environment for this repo's *comparison target*: the ORIGINAL
  # OCaml SATySFi, used to generate the reference PDFs the layout-fidelity test
  # measures the Rust port against (scripts/layout_fidelity.py,
  # docs/plans/design-layout-fidelity.md).
  #
  # Nixpkgs ships SATySFi 0.0.11 (a 0.0.x release matching the corpus's
  # 0.0.x documents). This flake pins it plus the tooling the layout test needs
  # — poppler `pdftotext` for word-box extraction and python3 for the harness —
  # so the baseline is reproducible rather than dependent on whatever SATySFi a
  # given machine happens to have.
  #
  #   nix develop                 # shell with satysfi + satyrographos + poppler + python3
  #   nix develop -c \
  #     python3 scripts/layout_fidelity.py --gen-refs --update
  #   nix run .#satysfi -- --version
  #
  # The Rust port itself is still built with the repo's pinned rustup toolchain
  # (not this flake) — this environment is only the SATySFi comparison target.

  description = "Original SATySFi (OCaml) + tooling to generate layout-fidelity baseline PDFs";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };
        # The original OCaml SATySFi — the reference typesetter the port is
        # compared against.
        satysfi = pkgs.satysfi;
      in
      {
        packages = {
          inherit satysfi;
          default = satysfi;
        };

        apps.satysfi = {
          type = "app";
          program = "${satysfi}/bin/satysfi";
        };
        apps.default = self.apps.${system}.satysfi;

        devShells.default = pkgs.mkShell {
          packages = [
            satysfi                 # the reference typesetter (0.0.11)
            pkgs.satyrographos      # its package manager (Satyrographos)
            pkgs.poppler-utils      # pdftotext / pdfinfo — layout extraction
            pkgs.python3            # the layout_fidelity.py harness
          ];
          shellHook = ''
            echo "SATySFi $(satysfi --version 2>/dev/null | grep -oE '[0-9.]+' | head -1) + pdftotext + python3"
            echo "generate baseline refs:  python3 scripts/layout_fidelity.py --gen-refs --update"
          '';
        };
      });
}
