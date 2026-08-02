<script lang="ts">
  // The connected-mode account block inside Settings: who you're signed in as, your personal
  // bearer token (copy / rotate), and the backend URL.
  //
  // It lives in its own component so SettingsDialog can import it *dynamically* behind the
  // BACKEND flag — a static import would pull the backend client into the standalone
  // GitHub-Pages bundle, which is meant to ship zero backend code.
  import { account } from "$lib/backend/account.svelte";
  import { signIn, signOut } from "$lib/backend/client";
  import { setBackendUrl, settings } from "$lib/stores/settings.svelte";

  let copied = $state(false);
  const token = $derived(account.me?.token ?? settings.backendToken);

  async function copyToken() {
    await navigator.clipboard.writeText(token);
    copied = true;
    setTimeout(() => (copied = false), 1200);
  }

  async function rotate() {
    if (!confirm("Rotate your token? Any MCP client using the current one stops working.")) return;
    await account.rotate();
  }
</script>

<div class="setting">
  <span class="setting-label">account</span>
  {#if account.me}
    <div class="row">
      <span class="who">{account.me.email ?? account.me.name}</span>
      <button class="ghost" onclick={() => void signOut()}>log out</button>
    </div>
  {:else if account.error}
    <span class="setting-hint">backend unreachable — {account.error}</span>
  {:else}
    <button class="ghost" onclick={() => signIn()}>sign in</button>
  {/if}

  <span class="setting-label">token (paste into an MCP client)</span>
  <div class="row">
    <input class="field" type="text" readonly spellcheck="false" value={token} />
    <button class="ghost" disabled={!token} onclick={copyToken}>
      {copied ? "copied" : "copy"}
    </button>
    <button class="ghost" disabled={!account.me || account.rotating} onclick={rotate}>
      {account.rotating ? "…" : "rotate"}
    </button>
  </div>
  <span class="setting-hint">
    connect an MCP client with <code>Authorization: Bearer &lt;token&gt;</code> to co-edit. rotating breaks
    any client still using the old one.
  </span>

  <span class="setting-label">backend url</span>
  <input
    class="field"
    type="text"
    placeholder="blank = same origin"
    value={settings.backendUrl}
    onchange={(e) => setBackendUrl(e.currentTarget.value)}
  />
</div>

<style>
  /* Mirrors SettingsDialog's own .setting/.field/.setting-label rules — scoped styles don't cross
     the component boundary, so the shared look is restated rather than inherited. */
  .setting {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .setting-label,
  .setting-hint {
    font-size: 12px;
    color: var(--halo-text-muted);
  }

  .field {
    width: 100%;
    font-size: 12px;
  }

  .field[readonly] {
    color: var(--halo-text-muted);
  }

  code {
    font-family: ui-monospace, "SF Mono", monospace;
    font-size: 11px;
  }

  .row {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .who {
    flex: 1;
    font-size: 12px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .ghost {
    flex: none;
    height: 26px;
    padding: 0 10px;
    border: 1px solid var(--halo-border);
    border-radius: var(--halo-radius);
    background: var(--halo-bg-main);
    color: var(--halo-text-main);
    font-size: 12px;
  }

  .ghost:hover:not(:disabled) {
    border-color: var(--halo-accent);
    color: var(--halo-accent);
  }

  .ghost:disabled {
    opacity: 0.5;
  }
</style>
