# Dev shell for the memview backend (Rust) + Angular frontend. Enter with: nix develop
# Pure-Rust TLS (rustls) so there's no openssl/pkg-config native dep.
{
  description = "memview — web viewer for the Claude memory markdown corpus";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      systems = [ "aarch64-darwin" "x86_64-linux" ];
      forAll = f: nixpkgs.lib.genAttrs systems (s: f nixpkgs.legacyPackages.${s});
    in {
      devShells = forAll (pkgs: {
        default = pkgs.mkShell {
          packages = [
            pkgs.cargo
            pkgs.rustc
            pkgs.rust-analyzer
            pkgs.rustfmt
            pkgs.clippy
            pkgs.nodejs_24 # Angular 22 frontend (frontend/)
            pkgs.pnpm # the frontend's installer; node ships npm too, ignore it
            # Re-renders gate.dhall into the committed gate.json. The gate can
            # only tell you the table is stale, and names this command when it
            # does — so the command has to be here, or editing the gate means
            # fetching 46 MB from the channel to run the one thing the error
            # message just told you to run.
            pkgs.dhall-json
          ];
        };
      });
    };
}
