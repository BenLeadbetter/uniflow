{
  description = "uniflow — a unidirectional data flow state management library";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    { nixpkgs, rust-overlay, ... }:
    let
      systems = [
        "aarch64-darwin"
        "x86_64-darwin"
        "aarch64-linux"
        "x86_64-linux"
      ];
      forAllSystems =
        f:
        nixpkgs.lib.genAttrs systems (
          system:
          f (
            import nixpkgs {
              inherit system;
              overlays = [ rust-overlay.overlays.default ];
            }
          )
        );
    in
    {
      devShells = forAllSystems (pkgs: {
        default = pkgs.mkShell {
          packages = [
            # Latest stable toolchain; `nix flake update` moves it forward.
            (pkgs.rust-bin.stable.latest.default.override {
              extensions = [
                "rust-src"
                "rust-analyzer"
              ];
            })
            # Runs the development commands in the justfile.
            pkgs.just
            # Manages the Python environment for the git hooks (pyproject.toml).
            pkgs.uv
            pkgs.python3
          ];

          # NOTE: uv uses the Nix-provided Python rather than downloading its own,
          # so the whole toolchain stays pinned by flake.lock.
          env = {
            UV_PYTHON = "${pkgs.python3}/bin/python3";
            UV_PYTHON_DOWNLOADS = "never";
          };

          shellHook = ''
            # Create or update .venv from uv.lock, and put its tools (pre-commit) on PATH.
            uv sync --frozen --quiet
            export PATH="$PWD/.venv/bin:$PATH"

            # Use the repo's commit message template. NOTE: only when none is set
            # for this clone, so a personal template set with `git config --local` wins.
            git config --local --get commit.template >/dev/null \
              || git config --local commit.template COMMIT_MESSAGE_TEMPLATE

            # Install the git hooks if they are missing, so a clone is checked from its
            # first commit. NOTE: the hook environments are built on first use, not here,
            # to keep entering the shell fast. Set UNIFLOW_SKIP_HOOKS=1 (in .envrc.local,
            # say) to manage the hooks yourself with `just install-hooks`.
            if [ -z "''${UNIFLOW_SKIP_HOOKS:-}" ]; then
              for hook in pre-commit commit-msg; do
                [ -f "$(git rev-parse --git-path hooks/$hook)" ] \
                  || pre-commit install --hook-type "$hook" >/dev/null
              done
            fi
          '';
        };
      });

      formatter = forAllSystems (pkgs: pkgs.nixfmt-rfc-style);
    };
}
