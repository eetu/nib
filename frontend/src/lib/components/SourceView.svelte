<script lang="ts">
  import Check from "@lucide/svelte/icons/check";
  import ChevronDown from "@lucide/svelte/icons/chevron-down";
  import ChevronUp from "@lucide/svelte/icons/chevron-up";
  import Copy from "@lucide/svelte/icons/copy";
  import Wand2 from "@lucide/svelte/icons/wand-2";

  import { editor } from "$lib/stores/document.svelte";

  let expanded = $state(false);
  let copied = $state(false);
  let draft = $state("");
  let dirty = $state(false);
  let error = $state<string | null>(null);
  let ta = $state<HTMLTextAreaElement>();

  const source = $derived(editor.hasDocument ? editor.toSvg() : "");

  // Mirror the live source into the editor until the user starts typing; once
  // dirty, their draft is preserved until they apply or revert.
  $effect(() => {
    if (!dirty) draft = source;
  });

  async function copy() {
    await navigator.clipboard.writeText(draft);
    copied = true;
    setTimeout(() => (copied = false), 1200);
  }

  function onInput(e: Event) {
    draft = (e.currentTarget as HTMLTextAreaElement).value;
    dirty = true;
    error = null;
  }

  // Re-parse the edited source. On failure the document is left untouched
  // (parseSvg throws before load mutates anything) and the error is shown.
  function apply() {
    try {
      editor.load(draft, editor.fileName);
      editor.dirty = true;
      dirty = false;
      error = null;
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
  }

  function revert() {
    dirty = false;
    error = null; // the effect reseeds draft from the live source
  }

  // Pretty-print the SVG (an opt-in reformat — the document is byte-preserved otherwise).
  // Leaves unparseable markup untouched. Marks the draft dirty so it applies on demand.
  function prettify() {
    const doc = new DOMParser().parseFromString(draft, "image/svg+xml");
    if (doc.querySelector("parsererror") || !doc.documentElement) return;
    draft = printEl(doc.documentElement, 0);
    dirty = true;
    error = null;
  }

  function printEl(el: Element, depth: number): string {
    const pad = "  ".repeat(depth);
    const attrs = Array.from(el.attributes)
      .map((a) => `${a.name}="${a.value}"`)
      .join(" ");
    const open = attrs ? `${el.tagName} ${attrs}` : el.tagName;
    const kids = Array.from(el.children);
    if (kids.length === 0) {
      const text = el.textContent?.trim();
      return text ? `${pad}<${open}>${text}</${el.tagName}>` : `${pad}<${open} />`;
    }
    const inner = kids.map((c) => printEl(c, depth + 1)).join("\n");
    return `${pad}<${open}>\n${inner}\n${pad}</${el.tagName}>`;
  }

  // Reveal the selected path's tag in the source — scroll to it + select it — when the
  // source is open and not being edited. Passive: it doesn't steal keyboard focus.
  $effect(() => {
    const i = editor.selectedPathIndex;
    const paths = editor.doc?.paths;
    if (!expanded || dirty || !ta || i == null || !paths || paths[i]?.deleted) return;
    let k = 0;
    for (let j = 0; j < i; j++) if (!paths[j].deleted) k++;
    const hay = draft.toLowerCase();
    let at = -1;
    for (let n = 0; n <= k; n++) {
      at = hay.indexOf("<path", at + 1);
      if (at === -1) return;
    }
    const end = draft.indexOf(">", at);
    if (end === -1) return;
    ta.setSelectionRange(at, end + 1);
    const line = draft.slice(0, at).split("\n").length;
    ta.scrollTop = Math.max(0, (line - 2) * 18); // ≈ line-height
  });
</script>

<div class="sourceview" class:expanded>
  <div class="bar">
    <button class="toggle" onclick={() => (expanded = !expanded)}>
      {#if expanded}<ChevronDown size={15} />{:else}<ChevronUp size={15} />{/if}
      source
    </button>
    {#if expanded && dirty}
      <button class="apply" onclick={apply}>apply</button>
      <button class="ghost" onclick={revert}>revert</button>
    {/if}
    {#if expanded}
      <button
        class="copy"
        onclick={prettify}
        disabled={!draft}
        title="Prettify"
        aria-label="Prettify"
      >
        <Wand2 size={14} />
      </button>
    {/if}
    <button class="copy" onclick={copy} disabled={!draft} class:ok={copied} aria-label="Copy SVG">
      {#if copied}<Check size={14} />{:else}<Copy size={14} />{/if}
    </button>
  </div>
  {#if expanded}
    <textarea class="src" bind:this={ta} value={draft} oninput={onInput} spellcheck="false"
    ></textarea>
    {#if error}<p class="error">{error}</p>{/if}
  {/if}
</div>

<style>
  .sourceview {
    border-top: 1px solid var(--halo-border);
    background: var(--halo-bg-main);
  }

  .bar {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 30px;
    padding: 0 8px;
  }

  .toggle {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    border: none;
    background: transparent;
    color: var(--halo-text-muted);
    font-family: var(--halo-font-heading);
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.08em;
  }

  .apply,
  .ghost {
    height: 22px;
    padding: 0 10px;
    border: 1px solid var(--halo-border);
    border-radius: var(--halo-radius-pill);
    background: var(--halo-bg-light);
    color: var(--halo-text-main);
    font-size: 12px;
  }

  .apply {
    border-color: var(--halo-accent);
    color: var(--halo-accent);
  }

  .copy {
    margin-left: auto;
    display: inline-flex;
    align-items: center;
    padding: 4px;
    border: none;
    background: transparent;
    color: var(--halo-text-muted);
  }

  .copy.ok {
    color: var(--halo-connected);
  }

  .src {
    display: block;
    width: 100%;
    height: 26vh;
    resize: vertical;
    padding: 8px 12px;
    border: none;
    border-top: 1px solid var(--halo-border);
    background: var(--halo-bg-light);
    color: var(--halo-text-main);
    font-family: ui-monospace, "SF Mono", monospace;
    font-size: 12px;
    line-height: 1.5;
  }

  .src:focus {
    outline: none;
  }

  .error {
    margin: 0;
    padding: 6px 12px;
    background: var(--halo-accent-soft);
    color: var(--halo-error);
    font-size: 12px;
  }
</style>
