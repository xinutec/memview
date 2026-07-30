// The app-specific half of the shared phone-width harness (@xinutec/ui-harness).
// Read by BOTH playwright.config.ts and the harness's static server, so there is
// one place to say what this app is and no port to keep in step — the port is
// allocated from `app`.

/** @type {import('@xinutec/ui-harness/config').HarnessSpec} */
export default {
  app: 'memview',
  dist: 'dist/memview-web/browser',
  // Fallback stub only — the specs page.route everything. Real prod is the Rust
  // backend. Signed-in owner + an empty corpus, so an un-mocked run still
  // renders.
  api: {
    '/api/me': { user_id: 'test', display_name: 'Test', shared: false, auth_enabled: false },
    '/api/index': { html: '<p>index</p>', count: 0 },
    '/api/memories': [],
  },
};
