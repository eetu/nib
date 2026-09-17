// The signed-in account (connected mode only). Holds what `/api/me` returns — identity plus the
// bearer token's hint — so Settings can say which token is in play without re-fetching. The
// token itself is never returned: only its hash is stored (see backend migration 0006).
//
// `error` is reserved for *real* failures: an unauthenticated call redirects to the login flow
// from inside the client, so reaching the catch here means the backend is actually unreachable.

import { type Me, me as fetchMe, rotateToken } from "$lib/backend/client";
import { setBackendToken } from "$lib/stores/settings.svelte";

class Account {
  me = $state<Me | null>(null);
  error = $state<string | null>(null);
  rotating = $state(false);

  /** The token minted by the most recent rotate, held only in memory for the user to copy. The
   *  server cannot show it again, so this is the one moment it exists outside the database's
   *  hash — it is deliberately not persisted anywhere but the field the user pastes it into. */
  freshToken = $state<string | null>(null);

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
    } catch (e) {
      this.error = e instanceof Error ? e.message : String(e);
    }
  }

  async rotate(): Promise<void> {
    this.rotating = true;
    try {
      const token = await rotateToken();
      this.freshToken = token;
      if (this.me) this.me = { ...this.me, tokenHint: token.slice(0, 12) };
      // Same-origin the browser authenticates by cookie, so it doesn't need this — but the
      // cross-origin dev split does, and the user has the token in hand exactly now.
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
