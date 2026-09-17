<script lang="ts">
  // The connected-mode account block inside Settings: who you're signed in as, your personal
  // bearer token (rotate / copy-once), and the backend URL.
  //
  // It lives in its own component so SettingsDialog can import it *dynamically* behind the
  // BACKEND flag — a static import would pull the backend client into the standalone
  // GitHub-Pages bundle, which is meant to ship zero backend code.
  import { account } from "$lib/backend/account.svelte";
  import { signIn, signOut } from "$lib/backend/client";
  import { setBackendToken, setBackendUrl, settings } from "$lib/stores/settings.svelte";

  let copied = $state(false);
  // The server stores only a hash, so it can show which token you have but never the token. The
  // hint identifies it; the value itself exists on screen exactly once, after a rotate.
  const hint = $derived(account.me?.tokenHint ?? "");

  async function copyToken() {
    if (!account.freshToken) return;
    await navigator.clipboard.writeText(account.freshToken);
    copied = true;
    setTimeout(() => (copied = false), 1200);
  }

  async function rotate() {
    if (
      !confirm(
        "Rotate your token? Any MCP client using the current one stops working, and the new one is shown once.",
      )
    )
      return;
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
  {#if account.freshToken}
    <!-- The one moment the token exists outside the database's hash. It is shown until the dialog
         closes rather than behind a timer: a value you cannot ask for again must not vanish while
         you are still reaching for the paste target. -->
    <div class="row">
      <input
        class="field fresh"
        type="text"
        readonly
        spellcheck="false"
        value={account.freshToken}
      />
      <button class="ghost" onclick={copyToken}>{copied ? "copied" : "copy"}</button>
    </div>
    <span class="setting-hint">
      copy it now — this is the only time it is shown. connect an MCP client with
      <code>Authorization: Bearer &lt;token&gt;</code> to co-edit.
    </span>
  {:else}
    <div class="row">
      <input
        class="field"
        type="text"
        readonly
        spellcheck="false"
        value={hint ? `${hint}…` : ""}
        placeholder="none yet — rotate to mint one"
      />
      <button class="ghost" disabled={!account.me || account.rotating} onclick={rotate}>
        {account.rotating ? "…" : "rotate"}
      </button>
    </div>
    <span class="setting-hint">
      only the token's hash is stored, so it can't be shown again — rotate to mint a new one.
      rotating breaks any client still using the old one.
    </span>
  {/if}

  <span class="setting-label">token this browser sends</span>
  <input
    class="field"
    type="password"
    autocomplete="off"
    placeholder="not needed same-origin — the session cookie authenticates"
    value={settings.backendToken}
    onchange={(e) => setBackendToken(e.currentTarget.value)}
  />
  <span class="setting-hint">
    only needed when <b>backend url</b> is set: cross-origin the session cookie doesn't ride along, so
    the live-sync socket has to carry a token instead.
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

  /* The one-time reveal reads as a value to act on, not as the greyed-out state next to it. */
  .fresh {
    color: var(--halo-text-main);
    font-family: ui-monospace, "SF Mono", monospace;
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
