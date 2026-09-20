import { Injectable } from '@angular/core';

/**
 * This device's own storage: one IndexedDB object store, keyed by string.
 *
 * ⚠ **Every read and write can fail, and none of them may throw at the caller.**
 * IndexedDB is refused outright in some private-browsing modes, and the things
 * kept here — an unsent draft, a picture, the last page of a transcript — are all
 * conveniences. Losing them costs a reader the offline copy; taking the app down
 * with them would cost the reader everything.
 *
 * Deliberately not a database library. What is wanted is `get`, `set` and
 * `delete` over a handful of keys, and an RxDB collection with a replication
 * protocol was what this replaced — see the note at the top of [[Drafts]].
 */

const DATABASE = 'console-local';
const STORE = 'kept';

@Injectable({ providedIn: 'root' })
export class Local {
  private opening?: Promise<IDBDatabase | undefined>;
  /** Overridden by a test that wants a database of its own. */
  private name = DATABASE;

  /** Use a database of this name. Called before anything else, or not at all. */
  under(name: string): void {
    this.name = name;
    this.opening = undefined;
  }

  async get(key: string): Promise<unknown> {
    const db = await this.open();
    if (!db) return undefined;
    return new Promise((done) => {
      try {
        const ask = db.transaction(STORE, 'readonly').objectStore(STORE).get(key);
        ask.onsuccess = () => done(ask.result);
        ask.onerror = () => done(undefined);
      } catch {
        done(undefined);
      }
    });
  }

  async set(key: string, value: unknown): Promise<void> {
    const db = await this.open();
    if (!db) return;
    return new Promise((done) => {
      try {
        const put = db.transaction(STORE, 'readwrite').objectStore(STORE).put(value, key);
        put.onsuccess = () => done();
        put.onerror = () => done();
      } catch {
        done();
      }
    });
  }

  async delete(key: string): Promise<void> {
    const db = await this.open();
    if (!db) return;
    return new Promise((done) => {
      try {
        const gone = db.transaction(STORE, 'readwrite').objectStore(STORE).delete(key);
        gone.onsuccess = () => done();
        gone.onerror = () => done();
      } catch {
        done();
      }
    });
  }

  /** Let go of the handle, so a test can open another database. */
  close(): void {
    const opening = this.opening;
    this.opening = undefined;
    void opening?.then((db) => db?.close());
  }

  /**
   * The database, opened once. `undefined` for ever after a refusal — retrying
   * per read would put a rejected permission prompt in front of every keystroke.
   */
  private open(): Promise<IDBDatabase | undefined> {
    this.opening ??= new Promise<IDBDatabase | undefined>((done) => {
      if (typeof indexedDB === 'undefined') {
        done(undefined);
        return;
      }
      const ask = indexedDB.open(this.name, 1);
      ask.onupgradeneeded = () => {
        if (!ask.result.objectStoreNames.contains(STORE)) ask.result.createObjectStore(STORE);
      };
      ask.onsuccess = () => done(ask.result);
      ask.onerror = () => {
        console.warn('local storage is not available; nothing will be kept on this device');
        done(undefined);
      };
      ask.onblocked = () => done(undefined);
    });
    return this.opening;
  }
}
