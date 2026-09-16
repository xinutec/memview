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
      # ⚠ NOT run from the store. Signing into the store cannot work — a build runs
      # as `nixbld`, which cannot reach the signing key's ACL, and signing a realised
      # path afterwards is undone at the next GC. The installed copy lives at
      # ~/.local/libexec/agent-console, put there by scripts/console-upgrade.sh with
      # an atomic rename, because macOS refuses to write to a running executable.
      #
      # ⚠ Only the Rust half: packaging the Angular build invites esbuild's macOS
      # teardown abort, which lands before index.html is flushed and leaves a
      # directory that exists and is empty. The frontend is published by
      # `publish:console`.
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
            # ⚠ One FILE, not the frontend. `console/tests/parse.rs` compares the
            # runner's output against this golden, and without it the read
            # returns "" and the test reports a diff against nothing — which
            # reads as a parser regression rather than a missing fixture.
            ./frontend/projects/console-web/e2e/parsed.fixture.json
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

          # ⚠ **Scoped, or the check runs NOTHING.** `cargoCheckHook` does not
          # inherit `cargoBuildFlags`: left to itself it runs the workspace's lib
          # and bin unittests, every one empty, and reports success having executed
          # 0 tests. `doCheck = true` alone therefore satisfies
          # `nix-rust-package-docheck-false` while testing nothing, which is the
          # defect that rule exists to catch.
          cargoTestFlags = [ "--package" "console" ];
          # `console/tests/orphan.rs` shells out to `ps` to prove a child was
          # reaped; the sandbox has no `ps`, and the failure reads as a broken
          # test rather than a missing tool.
          nativeCheckInputs = [ pkgs.procps ];
          # `past.rs` asks `ps -u $USER` which processes are running a
          # conversation, and holds everything BUSY when it cannot ask — a
          # deliberate fail-safe. The sandbox sets no `USER`, so two tests meet
          # that fallback rather than the thing they test.
          preCheck = "export USER=nixbld";
          doCheck = true;

          meta.mainProgram = "console";
        };

        # The desk-side CLI, and ONLY it.
        #
        # ⚠ `--bin sessions` leaves the server OUT: it is installed outside the
        # store by `scripts/console-upgrade.sh`, and shipping it again here would
        # put a second copy on PATH with different upgrade rules.
        sessions = pkgs.rustPlatform.buildRustPackage {
          pname = "sessions";
          version = "0.1.0";
          src = workspace;
          cargoLock.lockFile = ./Cargo.lock;
          cargoBuildFlags = [ "--package" "console" "--bin" "sessions" ];

          # ⚠ **The tests are NOT run here, and that is not a gap.** `.#console`
          # above is the same crate from the same source and runs the identical
          # suite, so a second run is two minutes of every gate spent on
          # byte-identical code. What this derivation checks is that the packaged
          # binary builds and installs — `cargoBuildFlags` above. If the two ever
          # stop being the same crate, the tests belong back here.
          # dev-lint: allow-docheck-false the same suite runs in `.#console`
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
