// The app-specific half of the shared phone-width harness (@xinutec/ui-harness).
// Read by BOTH the Playwright config and the harness's static server, so there
// is one place to say what this app is and no port to keep in step — the port is
// allocated from `app`.

/** @type {import('@xinutec/ui-harness/config').HarnessSpec} */
export default {
  app: 'console',
  // Relative to THIS project, because Playwright runs the harness's static
  // server with the config's directory as its cwd — and the config lives here,
  // beside the e2e/ it owns, while the bundle lands in the workspace's shared
  // dist/.
  // ng's own output path. NOT one of the copies the runner serves: those are
  // rsynced from here and exist so that no build ever deletes a directory
  // somebody is being served from (docs/agent-console.md), which also means they
  // are a build behind whenever `ng build` is run on its own — and a layout
  // harness pointed at a stale bundle passes for a page nobody is looking at.
  dist: '../../dist/console-build/browser',
  // Fallback stub only — the specs page.route everything. Real prod is the Rust
  // runner on loopback. An empty roster, so an un-mocked run still renders.
  api: {
    '/api/state': { dirs: ['/home/example/Code'], repos: [], sessions: [] },
  },
};
