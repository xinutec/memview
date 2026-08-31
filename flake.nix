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
      # The console binary, built the way the machine it runs on is built.
      #
      # ⚠ **This is not "run the console from a store path".** Signing INTO the
      # store does not work — a build runs as `nixbld`, whose session cannot
      # reach the signing key's ACL in ~/Library/Keychains, and signing a
      # realised path as root afterwards is a lie nix silently undoes at the next
      # GC. So the installed copy stays at ~/.local/libexec/agent-console and the
      # launchd plist goes on naming that path; this output is only what the
      # copy is made FROM. scripts/console-upgrade.sh does the install, by atomic
      # rename, because macOS refuses to write to a running executable.
      #
      # Only the Rust half. Packaging the Angular build invites esbuild's macOS
      # teardown abort, which lands before index.html is flushed and leaves a
      # directory that exists and is empty — thoth shipped exactly that and
      # crash-looped 33 times. The frontend is published by `publish:console`.
      packages = forAll (pkgs:
      let
        # Shared by both packages below, because the reasoning under it is about
        # the REPOSITORY rather than about either binary.
        workspace = pkgs.lib.fileset.toSource {
          root = ./.;
          fileset = pkgs.lib.fileset.unions [
            ./Cargo.toml
            ./Cargo.lock
            ./bash-oracle
            ./console
            ./reader
            ./src
          ];
        };
      in {
        console = pkgs.rustPlatform.buildRustPackage {
          pname = "agent-console";
          version = "0.1.0";

          # Named rather than `./.`: the repository root also holds the
          # frontend's node_modules and a 141 MB dist, and a source filter that
          # relies on gitignore would drop a Rust file nobody had staged yet —
          # silently, which is the failure mode worth spending five lines on.
          # `src/` is memview's own, and is here because cargo resolves every
          # workspace member's targets even when asked for one package.
          #
          # ⚠ **Every workspace member belongs here, including ones the console
          # never links.** Cargo loads the manifest of each member named in the
          # root `Cargo.toml` before it builds anything, so an absent directory
          # is not a smaller build — it is `failed to load manifest for
          # workspace member`. Adding `bash-oracle` to the workspace failed this
          # build and nothing else.
          src = workspace;

          cargoLock.lockFile = ./Cargo.lock;
          cargoBuildFlags = [ "--package" "console" ];

          # The gate runs the tests, against the whole workspace, with the corpus
          # and the transcripts this sandbox has none of. A second run here would
          # be a slower way to learn less.
          doCheck = false;

          meta.mainProgram = "console";
        };

        # The desk-side CLI, and ONLY it.
        #
        # ⚠ **`--bin sessions` leaves the server out, and that is correctness
        # rather than size.** The console runner is installed to
        # `~/.local/libexec/agent-console` by `scripts/console-upgrade.sh`,
        # deliberately outside the store so signing into the keychain survives a
        # GC. Shipping it a second time through home-manager would put a second
        # copy on PATH with different upgrade rules — the same argument
        # `tasks` makes for splitting its CLI from its server.
        #
        # ⚠ **This exists so the word a reader types is the program that runs.**
        # Until it is installed the only invocation is a `cargo run` from a
        # checkout, so every doc writes one thing and every shell does another
        # (memview#1298).
        sessions = pkgs.rustPlatform.buildRustPackage {
          pname = "sessions";
          version = "0.1.0";
          src = workspace;
          cargoLock.lockFile = ./Cargo.lock;
          cargoBuildFlags = [ "--package" "console" "--bin" "sessions" ];
          # As above: the gate runs the tests, with a corpus this sandbox has not.
          doCheck = false;
          meta.mainProgram = "sessions";
        };
        default = self.packages.${pkgs.stdenv.hostPlatform.system}.console;
      });

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
