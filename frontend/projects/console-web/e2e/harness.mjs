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
  dist: '../../dist/console-web/browser',
  // Fallback stub only — the specs page.route everything. Real prod is the Rust
  // runner on loopback. An empty roster, so an un-mocked run still renders.
  api: {
    '/api/state': { dirs: ['/home/example/Code'], repos: [], sessions: [] },
  },
};
