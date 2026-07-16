// The nib backend REST client (connected mode). Only imported behind the `BACKEND` flag. Talks to
// the token-authed projects API; the base URL defaults to same-origin (the dev Vite proxy / the
// backend-embedded build), or an explicit `settings.backendUrl`.

import { base } from "$app/paths";
import { settings } from "$lib/stores/settings.svelte";

export type ProjectMeta = { id: number; name: string; updated_at: string };
export type Project = { id: number; name: string; svg: string };

function apiBase(): string {
  return settings.backendUrl || base; // "" → same-origin (respecting the Pages base path)
}

function authHeaders(extra?: Record<string, string>): Record<string, string> {
  return { Authorization: `Bearer ${settings.backendToken}`, ...(extra ?? {}) };
}

async function json<T>(res: Response, what: string): Promise<T> {
  if (!res.ok) throw new Error(`${what}: ${res.status} ${await res.text().catch(() => "")}`.trim());
  return res.json() as Promise<T>;
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
export async function putProject(id: number, svg: string): Promise<void> {
  const res = await fetch(`${apiBase()}/api/projects/${id}`, {
    method: "PUT",
    headers: authHeaders({ "Content-Type": "image/svg+xml" }),
    body: svg,
  });
  if (!res.ok) throw new Error(`save project: ${res.status}`);
}
