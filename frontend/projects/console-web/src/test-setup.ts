// A real IndexedDB for the unit tests.
//
// Without this every storage test passes vacuously. jsdom has no
// IndexedDB, and [[Local]] is deliberately quiet when storage is refused — it
// answers `undefined` and swallows writes, because a private-browsing mode that
// refuses the database must not take the app down. So a test asserting that
// something was kept would read back nothing and agree with itself.
import 'fake-indexeddb/auto';
