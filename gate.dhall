{-
memview/gate.dhall — this repository's commit gate.

Was `scripts/verify.sh`. Two things it asserted turned out not to be true, and
both are now checked by a tool rather than by a line of shell.

**The build check was reading a bundle the gate had never built.** The script
ran `pnpm run build || true` and then `test -s dist/console-web/browser/
index.html`. But `pnpm run build` is `ng build && ng build console-web`, and
console-web's `outputPath` is `dist/console-build` — nothing in that chain
writes `dist/console-web`, which is a copy `pnpm run build:console` rsyncs by
hand. Measured rather than reasoned: backdate both index.html files to 2020, run
the gate's own build, and `dist/memview-web` and `dist/console-build` come back
stamped today while `dist/console-web` is still 2020. So for the console the
gate's authoritative build assertion had been passing against whatever a
previous by-hand `build:console` had left, indefinitely.

`console-web/e2e/harness.mjs` had already worked this out for the harness it
serves — "they are a build behind whenever `ng build` is run on its own, and a
layout harness pointed at a stale bundle passes for a page nobody is looking at"
— and points at `dist/console-build/browser`. The gate now points where the
harness does, and `ng-build` additionally requires that this run *rewrote* the
file, so pointing it somewhere stale fails instead of passing.

**`|| true` is gone**, and with it the whole question of what the exit status
means. `ng-build` decides from the artifact: index.html present, non-empty,
rewritten by this run, and every script and stylesheet it names on disk and
non-empty. That is the strongest of the five versions of this workaround the
fleet was carrying, rather than this repository's — which was among the weakest.

**The corpus guard is gone.** The script ran memory-lint only `if [[ -f
$MEMORY_DIR/MEMORY.md ]]`, justified by CI having no memory directory. But CI
never ran this script: `.github/workflows/build.yml` writes the steps out in
YAML itself, deliberately without a corpus. So the guard was protecting a path
that does not take this gate, while on the machine that does take it the corpus
is always there. Unguarded, a moved corpus now fails loudly — `memory-lint`
already defaults to the corpus path and errors when it cannot load it. The `|
tail -20` went with it: it trimmed noise for a human, and the gate prints a
check's output only when it fails.

**The conditional `pnpm install` is gone**, for the reason gamepads' and coach's
were: its own comment justified it on correctness, and running unconditionally
serves that better. An up-to-date `--frozen-lockfile` install measured 455 ms.

**The `&&` chain is gone.** One `nix develop -c bash -c '…'` with eleven
commands in it reported one name when eleven things could be wrong.

Not changed, deliberately: the co-use artefact is still not regenerated here.
`couse` reads every byte of every session transcript (~3 GB, about two minutes);
`memory-lint` uses whatever is already on disk, and the mining is run by hand:

    nix develop -c cargo run --release --bin couse

The generated `gate.json` is committed; `the table matches its Dhall` re-renders
and diffs it, so running the gate needs no `dhall`.

**The vocabulary moved into the schema.** `inDevShell`, the clippy target
directory, the Angular worker cap, and the `ng-build` / `dev-lint` /
`check-table` rows were spelled out here and in a dozen other tables
identically — the duplication the shared tools were built to remove, recreated
one level up. They are `G.` values now. Two consequences the rendered JSON
shows: every dev-shell row gains `--no-warn-dirty`, because a gate that prints
"Git tree is dirty" on every row of every run has trained everyone to ignore a
warning; and dev-lint is pinned to its committed HEAD rather than run out of its
worktree, which is what stops a neighbour's half-finished edit failing this gate
for a reason no commit anywhere explains.

-}

let G = ../dev-lint/gate/schema.dhall

in  { name = "memview"
    , checks =
      [ G.Check::{
        , name = "formatting"
        , argv = G.inDevShell [ "cargo", "fmt", "--all", "--check" ]
        , timeout_s = 180
        }
      , {-  `--workspace`: the console is a member crate, and without this its
            clippy and its tests are simply not run.
        -}
        G.Check::{
        , name = "clippy"
        , argv =
            G.inDevShell
              [ "cargo"
              , "clippy"
              , "--workspace"
              , "--all-targets"
              , "--"
              , "-D"
              , "warnings"
              ]
        , timeout_s = 1800
        }
      , G.Check::{
        , name = "tests"
        , argv = G.inDevShell [ "cargo", "test", "--workspace" ]
        , timeout_s = 1800
        }
      , {-  `--frozen-lockfile` is pnpm ci: install exactly pnpm-lock.yaml, or
            fail. The gate has to run from a clean checkout — a fresh clone, or
            the tree the fleetwatch collector runs in — not just a warm dev
            machine.
        -}
        G.Check::{
        , name = "frontend deps match the lockfile"
        , cwd = "frontend"
        , argv = G.inDevShell [ "pnpm", "install", "--frozen-lockfile" ]
        , timeout_s = 900
        }
      , G.Check::{
        , name = "frontend lint"
        , cwd = "frontend"
        , argv = G.inDevShell [ "pnpm", "run", "lint" ]
        , timeout_s = 900
        }
      , {-  The layout harnesses and the Playwright configs. `ng build` compiles
            only what `src/main.ts` imports and Playwright strips types with
            esbuild rather than checking them, so without this the code that
            guards how the console behaves on a phone is the least-checked in the
            repository — proven with a planted type error that passed lint,
            `ng build` and a Playwright run.
        -}
        G.Check::{
        , name = "frontend typecheck (e2e)"
        , cwd = "frontend"
        , argv = G.inDevShell [ "pnpm", "run", "typecheck" ]
        , timeout_s = 900
        }
      , G.Check::{
        , name = "frontend unit tests"
        , cwd = "frontend"
        , argv = G.inDevShell [ "pnpm", "test" ]
        , env = G.oneAngularWorker
        , timeout_s = 1800
        }
      , {-  A row of its own because it cannot be one of the above. The Angular
            unit-test builder runs specs against a browser build of an
            application; this is a node script that deletes files out of the
            directory the phone is served from, so it is tested with node's own
            runner against a real temporary tree.
        -}
        G.Check::{
        , name = "the live-bundle pruner"
        , cwd = "frontend"
        , argv = G.inDevShell [ "node", "--test", "scripts/prune-live.test.mjs" ]
        , timeout_s = 300
        }
      , {-  Both applications, and both output directories asserted on. The
            `--expect` paths are ng's own output paths — `dist/console-live` is
            an rsynced copy that exists so no build ever deletes a directory
            somebody is being served from, and it is a build behind whenever
            `ng build` runs on its own. Since 2026-08-11 it is a publish behind
            as well: only `pnpm run publish:console` writes it, so a build for a
            test cannot reach the phone.

            `../../dev-lint`, not `../dev-lint`: cwd is `memview/frontend`.
        -}
        G.Check::{
        , name = "frontend build (both applications)"
        , cwd = "frontend"
        , argv =
            G.ngBuild
              "../../"
              [ "dist/memview-web/browser", "dist/console-build/browser" ]
              [ "pnpm", "run", "build" ]
        , timeout_s = 1800
        }
      , {-  Serves the freshly-built dist and asserts no overlap or overflow at
            Pixel width, for both applications. Runs after the build, which is
            why it is placed here — though placement is presentation only, and it
            would run regardless.
        -}
        G.Check::{
        , name = "frontend ui-check (phone-width layout harness)"
        , cwd = "frontend"
        , argv = G.inDevShell [ "pnpm", "run", "ui-check" ]
        , env = G.oneAngularWorker
        , timeout_s = 1800
        }
      , {-  The graph layout, measured rather than looked at. Every bug this view
            has had was a picture that looked plausible — a zoom that was a silent
            no-op, labels that overprinted, sections that smeared into one ball —
            and none of them threw or failed a lint. These thresholds are the only
            gate that can catch the next one before it ships.
        -}
        G.Check::{
        , name = "graph layout report"
        , argv = G.inDevShell [ "node", "scripts/graph-report.mjs" ]
        , timeout_s = 900
        }
      , {-  The corpus itself. Locally this is the check that matters most: the
            app can be perfect and the document set still be falling apart.
        -}
        G.Check::{
        , name = "memory-lint (the corpus)"
        , argv =
            G.inDevShell
              [ "cargo", "run", "--quiet", "--bin", "memory-lint" ]
        , timeout_s = 900
        }
      , {-  The conversations the corpus is mined from. Same argument as
            memory-lint one row up, one layer down: the documents can be
            well-formed and the evidence under them broken.

            It earns a row rather than living in the tests because the tests
            check the rules against fixtures, and this checks the rules against
            what is actually on the disk — which is where the three severed
            links were, unnoticed for three weeks, while every reader in the
            workspace rendered them as though nothing were missing.

            Unguarded, like memory-lint: CI writes its own steps in YAML and
            never takes this gate, so the only machine that runs it is the one
            where the transcripts always are. A file being appended to right now
            reports `incomplete-tail`, which is not damage and does not fail.
        -}
        G.Check::{
        , name = "transcript-lint (the conversations)"
        , argv =
            G.inDevShell
              [ "cargo"
              , "run"
              , "--quiet"
              , {-  `-p` is required and memory-lint's row does without it. That
                    one is a bin of the ROOT package, and `default-run =
                    "memview"` makes a bare `--bin` search only there — so
                    naming a bin that lives in `reader` fails with "no bin
                    target named transcript-lint in default-run packages"
                    while listing that exact name as available.
                -}
                "-p"
              , "reader"
              , "--bin"
              , "transcript-lint"
              , "--"
              , "--quiet"
              ]
        , timeout_s = 900
        }
      , G.checkTable "../dev-lint"
      , G.devLint "../"
      ]
    }
