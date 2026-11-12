{-
memview/acting.dhall — every operational script in this repository (`*.sh`,
`*.py`), classified, with a ceiling per class; a test needs no row when its path
says it is one. What the classes mean and when a ceiling may rise:
dev-lint/tables/acting_schema.dhall. The gate runs `dev-lint-fleet --acting .`:
a script with no row, a row for a file that is gone, or a class over its
ceiling fails the commit that causes it.
-}
let A = ../dev-lint/tables/acting_schema.dhall

let subject = A.subject

let Class = A.Class

in  { ceilings =
        -- acts: each row names what it becomes (automation-model items 13-15)
        -- bootstrap: one-off imports and enrolments
      { acts = 4
      , bootstrap = 1
      , by_design = 0
      , observes = 0
      , observer_checks = 0
      }
    , subjects =
      [ subject
              "android/deploy.sh"
              Class.Acts
              "builds the APK and installs it on a phone over adb: the phone-shells plan's target (automation-model item 14)"
          , subject
              "console/android/deploy.sh"
              Class.Acts
              "builds the APK and installs it on a phone over adb: the phone-shells plan's target (automation-model item 14)"
          , subject "reader/probes/ansi-quote.sh" Class.OutOfScope ""
          , subject "reader/probes/arithmetic.sh" Class.OutOfScope ""
          , subject "reader/probes/array.sh" Class.OutOfScope ""
          , subject "reader/probes/bash-printer.sh" Class.OutOfScope ""
          , subject "reader/probes/brace.sh" Class.OutOfScope ""
          , subject "reader/probes/bracket.sh" Class.OutOfScope ""
          , subject "reader/probes/case.sh" Class.OutOfScope ""
          , subject "reader/probes/conditional.sh" Class.OutOfScope ""
          , subject "reader/probes/grouping.sh" Class.OutOfScope ""
          , subject "reader/probes/heredoc.sh" Class.OutOfScope ""
          , subject "reader/probes/parameter-op.sh" Class.OutOfScope ""
          , subject "reader/probes/process-substitution.sh" Class.OutOfScope ""
          , subject "reader/probes/quoting.sh" Class.OutOfScope ""
          , subject "reader/probes/substitution-heredoc.sh" Class.OutOfScope ""
          , subject
              "scripts/awake-watch.sh"
              Class.OutOfScope
              "watches the phone's screen state over adb; changes nothing"
          , subject "scripts/comment-blocks.py" Class.OutOfScope ""
          , subject "scripts/console-identity.sh" Class.OutOfScope ""
          , subject "scripts/console-service.sh" Class.OutOfScope ""
          , subject "scripts/console-serving.sh" Class.OutOfScope ""
          , subject
              "scripts/console-upgrade.sh"
              Class.Acts
              "rebuilds the agent console and hands the running one over to it"
          , subject "scripts/console.sh" Class.OutOfScope ""
          , subject "scripts/dev.sh" Class.OutOfScope ""
          , subject
              "scripts/enrol.sh"
              Class.Bootstrap
              "enrols one phone: it makes a key and the server records it"
          , subject "scripts/gate-changed.sh" Class.OutOfScope ""
          , subject "scripts/gen-types.sh" Class.OutOfScope ""
          , subject "scripts/gist-strays.py" Class.OutOfScope ""
          , subject "scripts/manifests-parse-from-stubs.sh" Class.OutOfScope ""
          , subject "scripts/memory-role-hook.sh" Class.OutOfScope ""
          , subject "scripts/mine-resume.sh" Class.OutOfScope ""
          , subject "scripts/reparent-transcript.py" Class.OutOfScope ""
          , subject "scripts/setup-hooks.sh" Class.OutOfScope ""
          , subject "scripts/sync-check.sh" Class.OutOfScope ""
          , subject
              "scripts/sync.sh"
              Class.Acts
              "pushes the Mac's memory corpus into the memview pod"
          , subject "scripts/target-silt.sh" Class.OutOfScope ""
          , subject "scripts/tests-live-in-tests.sh" Class.OutOfScope ""
          , subject "scripts/workspace-members.sh" Class.OutOfScope ""
      ]
    }
