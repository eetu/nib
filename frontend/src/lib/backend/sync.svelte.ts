// Live project sync (connected mode). Opens a WebSocket to /ws/projects/{id}, streams the local
// editor's committed ops to the backend, and applies remote ops (from another browser or the LLM)
// back into the document — echo-guarded by a per-tab client id. Only imported behind `BACKEND`.
// A `.svelte.ts` module so the reactive `$state` connection status compiles.

import { base } from "$app/paths";
import { getProject, putProject } from "$lib/backend/client";
import { loadState, removeState, saveState } from "$lib/persistence";
import { type DocumentReplacement, editor } from "$lib/stores/document.svelte";
import { settings } from "$lib/stores/settings.svelte";

const CLIENT_ID = crypto.randomUUID();

/** Which project this browser was editing — so a reload comes back to it instead of detaching. */
const PROJECT_KEY = "project";

// `reload` marks a whole-document replacement (an import). Ops describe edits *to* a document, so
// there's nothing to replay — the peer re-fetches instead.
type SyncMsg = { clientId: string; ops: unknown[]; reload?: boolean };

function wsUrl(id: number): string {
  if (settings.backendUrl) {
    // Cross-origin: the session cookie won't ride along, so the token has to.
    const u = new URL(settings.backendUrl);
    const proto = u.protocol === "https:" ? "wss:" : "ws:";
    const token = encodeURIComponent(settings.backendToken);
    return `${proto}//${u.host}${base}/ws/projects/${id}?token=${token}`;
  }
  // Same-origin: the browser sends `nib_session` on the handshake. Keeping the token out of the
  // URL keeps it out of proxy access logs and browser history.
  const proto = location.protocol === "https:" ? "wss:" : "ws:";
  return `${proto}//${location.host}${base}/ws/projects/${id}`;
}

class ProjectSync {
  #ws: WebSocket | null = null;
  status = $state<"disconnected" | "connecting" | "connected">("disconnected");
  projectId = $state<number | null>(null);
  /** Surfaced by the projects panel — an import that couldn't reach the project must not be silent. */
  error = $state<string | null>(null);

  /**
   * Open a project: load the server's model, then attach.
   *
   * The model, not the SVG — it carries the node uids every client and the LLM address, so loading
   * it is what keeps identity shared. (A project from before the model column has none; the
   * backend migrates it on first open, so the svg fallback is a one-time path.)
   */
  async open(id: number): Promise<void> {
    const p = await getProject(id);
    if (p.model) editor.loadModel(JSON.parse(p.model), p.name);
    else editor.load(p.svg, p.name);
    this.connect(id);
  }

  /**
   * Re-open whatever was being edited before the page reloaded.
   *
   * Without this, a reload left the canvas showing the project — restored from localStorage — with
   * nothing attached to it. Every edit went nowhere, and the only cue was an unhighlighted row in
   * a panel you might not have open. That's the same silent divergence an unsynced import used to
   * cause, reached through a different door.
   *
   * The server's copy wins, deliberately: it's what every other client and the LLM are working
   * from, and a local copy that drifted while detached is exactly what must not be pushed over it.
   * Bounded by the caller's own project list, so a deleted or foreign id quietly does nothing.
   */
  async restore(known: readonly number[]): Promise<void> {
    const id = loadState<number>(PROJECT_KEY);
    if (id === null || !known.includes(id)) {
      removeState(PROJECT_KEY);
      return;
    }
    try {
      await this.open(id);
    } catch {
      removeState(PROJECT_KEY); // gone, or unreachable — start detached rather than pretend
    }
  }

  connect(id: number): void {
    this.disconnect();
    this.projectId = id;
    saveState(PROJECT_KEY, id);
    this.status = "connecting";
    const ws = new WebSocket(wsUrl(id));
    this.#ws = ws;

    ws.addEventListener("open", () => {
      if (this.#ws !== ws) return;
      this.status = "connected";
      // Stream each committed op-batch to the backend, which persists + broadcasts it.
      editor.setSyncSink((ops) => this.#send(ops));
      // …and catch whole-document swaps, which ops can't carry.
      editor.setReplaceSink((r) => void this.#onReplace(r));
    });
    ws.addEventListener("message", (e) => {
      let msg: SyncMsg;
      try {
        msg = JSON.parse(e.data);
      } catch {
        return;
      }
      if (msg.reload) {
        void this.#reload();
        return;
      }
      if (msg.clientId === CLIENT_ID) return; // our own echo
      editor.applyRemote(msg.ops);
    });
    const drop = () => {
      if (this.#ws === ws) {
        this.status = "disconnected";
        editor.setSyncSink(null);
        editor.setReplaceSink(null);
      }
    };
    ws.addEventListener("close", drop);
    ws.addEventListener("error", drop);
  }

  /** The document was replaced wholesale while a project was open. */
  async #onReplace(r: DocumentReplacement): Promise<void> {
    const id = this.projectId;
    if (id === null) return;
    // Starting a blank document means you've left the project behind — detaching is what stops
    // subsequent edits streaming into a project that no longer matches the canvas.
    if (r.kind === "new") {
      this.disconnect();
      return;
    }
    try {
      await putProject(id, r.svg);
      // Adopt the server's parse: it mints the node uids every client (and the LLM) addresses, so
      // re-loading its model is what keeps identity shared. Skipping this is how ops start
      // referring to nodes the backend has never heard of.
      await this.#reload();
      this.error = null;
    } catch (e) {
      // Don't keep streaming edits into a project that didn't receive the import.
      this.error = `import into project failed: ${e instanceof Error ? e.message : String(e)}`;
      this.disconnect();
    }
  }

  /** Re-fetch the open project and load its model (after an import, here or from a peer). */
  async #reload(): Promise<void> {
    const id = this.projectId;
    if (id === null) return;
    const p = await getProject(id);
    if (this.projectId !== id) return; // switched projects mid-flight
    // Loaded through `load`/`loadModel`, never `importDocument` — this came *from* the project, so
    // announcing it as an import would push it straight back.
    if (p.model) editor.loadModel(JSON.parse(p.model), p.name);
    else editor.load(p.svg, p.name);
  }

  #send(ops: unknown[]): void {
    if (this.#ws?.readyState === WebSocket.OPEN) {
      this.#ws.send(JSON.stringify({ clientId: CLIENT_ID, ops }));
    }
  }

  disconnect(): void {
    editor.setSyncSink(null);
    editor.setReplaceSink(null);
    this.#ws?.close();
    this.#ws = null;
    this.projectId = null;
    this.status = "disconnected";
    // Forget it too: New detaches on purpose, and a reload that re-opened the project you had
    // just left would undo that decision for you.
    removeState(PROJECT_KEY);
  }
}

export const sync = new ProjectSync();
