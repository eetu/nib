// The signed-in account (connected mode only). Holds what `/api/me` returns — identity plus the
// personal bearer token an MCP client needs — so Settings can show it without re-fetching.
//
// `error` is reserved for *real* failures: an unauthenticated call redirects to the login flow
// from inside the client, so reaching the catch here means the backend is actually unreachable.

import { type Me, me as fetchMe, rotateToken } from "$lib/backend/client";
import { setBackendToken } from "$lib/stores/settings.svelte";

class Account {
  me = $state<Me | null>(null);
  error = $state<string | null>(null);
  rotating = $state(false);

  /** In-flight `/api/me`, shared by concurrent callers. Two mount together on a fresh load — the
   *  header's project link and the projects panel — and `/api/me` is the call that bounces an
   *  expired session to the login, so firing it twice means two redirects racing each other. */
  #loading: Promise<void> | null = null;

  /** Fetch the account. A later call re-fetches (the panel's refresh wants fresh data); only
   *  overlapping calls share one request. */
  load(): Promise<void> {
    this.#loading ??= this.#fetch().finally(() => (this.#loading = null));
    return this.#loading;
  }

  async #fetch(): Promise<void> {
    try {
      this.me = await fetchMe();
      this.error = null;
      // Keep the stored token in step with the server's, so the WebSocket and a cross-origin SPA
      // both authenticate with the current one after a rotate elsewhere.
      setBackendToken(this.me.token);
    } catch (e) {
      this.error = e instanceof Error ? e.message : String(e);
    }
  }

  async rotate(): Promise<void> {
    this.rotating = true;
    try {
      const token = await rotateToken();
      if (this.me) this.me = { ...this.me, token };
      setBackendToken(token);
      this.error = null;
    } catch (e) {
      this.error = e instanceof Error ? e.message : String(e);
    } finally {
      this.rotating = false;
    }
  }
}

export const account = new Account();
