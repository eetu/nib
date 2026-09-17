// The nib backend REST client (connected mode). Only imported behind the `BACKEND` flag. The base
// URL defaults to same-origin (the dev Vite proxy / the backend-embedded build), or an explicit
// `settings.backendUrl`.
//
// Two credentials, by design. Same-origin the browser sends the `nib_session` cookie the OIDC
// login set, and that's what authorizes the account endpoints. A bearer token is sent too when one
// is configured, which is how a *cross-origin* SPA (an explicit `backendUrl`) authenticates, since
// the production server serves no CORS credentials.

import { base } from "$app/paths";
import { settings } from "$lib/stores/settings.svelte";

export type ProjectMeta = { id: number; name: string; updated_at: string };
// `model` is the native document-model JSON (the source of truth); `svg` is a cached export. A
// brand-new project has an empty `model` until first opened (then the backend imports svg → model).
export type Project = {
  id: number;
  name: string;
  model: string;
  svg: string;
  /** How many times this project's document has been replaced wholesale. Echoed back as
   *  `If-Match` on the next import, so one that would land on top of a replacement this
   *  client never saw is refused rather than silently winning. */
  generation: number;
};

/** A `PUT` refused because the project moved on — the server's message says where it is. */
export class ProjectConflict extends Error {}
/** Identity + the personal bearer token, from `/api/me` (session-authenticated). */
export type Me = {
  id: number;
  name: string;
  email: string | null;
  token: string;
  projects: ProjectMeta[];
};

function apiBase(): string {
  return settings.backendUrl || base; // "" → same-origin (respecting the Pages base path)
}

function authHeaders(extra?: Record<string, string>): Record<string, string> {
  const auth: Record<string, string> = settings.backendToken
    ? { Authorization: `Bearer ${settings.backendToken}` }
    : {};
  return { ...auth, ...(extra ?? {}) };
}

/** Bounce to the OIDC login and come back where we were. */
export function signIn(): void {
  const here = location.pathname + location.search + location.hash;
  location.assign(`${apiBase()}/auth/login?next=${encodeURIComponent(here)}`);
}

async function json<T>(res: Response, what: string): Promise<T> {
  if (!res.ok) {
    // A 401 means "not signed in", not "broken" — send them through the login flow. Anything else
    // is a real failure and must surface as one, or a down backend becomes a redirect loop.
    if (res.status === 401) {
      signIn();
      throw new Error("not signed in");
    }
    throw new Error(`${what}: ${res.status} ${await res.text().catch(() => "")}`.trim());
  }
  return res.json() as Promise<T>;
}

/** Who am I, my token, and my projects — one call on connect. Session cookie only. */
export async function me(): Promise<Me> {
  return json(await fetch(`${apiBase()}/api/me`, { headers: authHeaders() }), "me");
}

/** Mint a replacement bearer token. Invalidates the previous one immediately. */
export async function rotateToken(): Promise<string> {
  const { token } = await json<{ token: string }>(
    await fetch(`${apiBase()}/api/token/rotate`, { method: "POST", headers: authHeaders() }),
    "rotate token",
  );
  return token;
}

/** Drop the session cookie. The bearer token survives, so a configured MCP client keeps working. */
export async function signOut(): Promise<void> {
  await fetch(`${apiBase()}/auth/logout`, { method: "POST", headers: authHeaders() });
  location.assign(`${apiBase()}/auth/login?next=/`);
}

export async function listProjects(): Promise<ProjectMeta[]> {
  return json(
    await fetch(`${apiBase()}/api/projects`, { headers: authHeaders() }),
    "list projects",
  );
}

export async function getProject(id: number): Promise<Project> {
  return json(
    await fetch(`${apiBase()}/api/projects/${id}`, { headers: authHeaders() }),
    "get project",
  );
}

export async function createProject(name: string): Promise<{ id: number; name: string }> {
  return json(
    await fetch(`${apiBase()}/api/projects`, {
      method: "POST",
      headers: authHeaders({ "Content-Type": "application/json" }),
      body: JSON.stringify({ name }),
    }),
    "create project",
  );
}

/** Persist a project's SVG (an explicit save; live edits also stream via the WebSocket). */
/**
 * Replace a project's document.
 *
 * `generation` is the one the caller believes it is replacing; the server refuses (409) if the
 * project has moved on since. Omitting it forces the write, which is what a deliberate overwrite
 * looks like — so it is never omitted by accident here.
 */
export async function putProject(id: number, svg: string, generation?: number): Promise<void> {
  const headers = authHeaders({ "Content-Type": "image/svg+xml" });
  if (generation !== undefined) headers["If-Match"] = String(generation);
  const res = await fetch(`${apiBase()}/api/projects/${id}`, {
    method: "PUT",
    headers,
    body: svg,
  });
  if (res.status === 409) {
    throw new ProjectConflict((await res.text()) || "the project moved on");
  }
  if (!res.ok) throw new Error(`save project: ${res.status}`);
}

/** Rename a project. PATCH, not PUT — PUT replaces the *document*. */
export async function renameProject(id: number, name: string): Promise<void> {
  const res = await fetch(`${apiBase()}/api/projects/${id}`, {
    method: "PATCH",
    headers: authHeaders({ "Content-Type": "application/json" }),
    body: JSON.stringify({ name }),
  });
  if (!res.ok) throw new Error(`rename project: ${res.status}`);
}

/** Delete a project and its stored document. Irreversible — callers confirm first. */
export async function deleteProject(id: number): Promise<void> {
  const res = await fetch(`${apiBase()}/api/projects/${id}`, {
    method: "DELETE",
    headers: authHeaders(),
  });
  if (!res.ok) throw new Error(`delete project: ${res.status}`);
}
