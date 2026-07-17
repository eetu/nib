// Live project sync (connected mode). Opens a WebSocket to /ws/projects/{id}, streams the local
// editor's committed ops to the backend, and applies remote ops (from another browser or the LLM)
// back into the document — echo-guarded by a per-tab client id. Only imported behind `BACKEND`.
// A `.svelte.ts` module so the reactive `$state` connection status compiles.

import { base } from "$app/paths";
import { editor } from "$lib/stores/document.svelte";
import { settings } from "$lib/stores/settings.svelte";

const CLIENT_ID = crypto.randomUUID();

type SyncMsg = { clientId: string; ops: unknown[] };

function wsUrl(id: number): string {
  const token = encodeURIComponent(settings.backendToken);
  const path = `${base}/ws/projects/${id}?token=${token}`;
  if (settings.backendUrl) {
    const u = new URL(settings.backendUrl);
    const proto = u.protocol === "https:" ? "wss:" : "ws:";
    return `${proto}//${u.host}${path}`;
  }
  const proto = location.protocol === "https:" ? "wss:" : "ws:";
  return `${proto}//${location.host}${path}`;
}

class ProjectSync {
  #ws: WebSocket | null = null;
  status = $state<"disconnected" | "connecting" | "connected">("disconnected");
  projectId = $state<number | null>(null);

  connect(id: number): void {
    this.disconnect();
    this.projectId = id;
    this.status = "connecting";
    const ws = new WebSocket(wsUrl(id));
    this.#ws = ws;

    ws.addEventListener("open", () => {
      if (this.#ws !== ws) return;
      this.status = "connected";
      // Stream each committed op-batch to the backend, which persists + broadcasts it.
      editor.setSyncSink((ops) => this.#send(ops));
    });
    ws.addEventListener("message", (e) => {
      let msg: SyncMsg;
      try {
        msg = JSON.parse(e.data);
      } catch {
        return;
      }
      if (msg.clientId === CLIENT_ID) return; // our own echo
      editor.applyRemote(msg.ops);
    });
    const drop = () => {
      if (this.#ws === ws) {
        this.status = "disconnected";
        editor.setSyncSink(null);
      }
    };
    ws.addEventListener("close", drop);
    ws.addEventListener("error", drop);
  }

  #send(ops: unknown[]): void {
    if (this.#ws?.readyState === WebSocket.OPEN) {
      this.#ws.send(JSON.stringify({ clientId: CLIENT_ID, ops }));
    }
  }

  disconnect(): void {
    editor.setSyncSink(null);
    this.#ws?.close();
    this.#ws = null;
    this.projectId = null;
    this.status = "disconnected";
  }
}

export const sync = new ProjectSync();
