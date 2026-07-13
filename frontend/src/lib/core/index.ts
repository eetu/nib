// Thin wrapper around the nib-core WASM engine (the Rust document / geometry / op core).
// The rest of the app imports the core from here — never from the generated `nib-core`
// package directly — so the WASM boundary and its one-time init live in exactly one place.
//
// Phase A1: this only proves the Rust → WASM → Svelte pipeline. A2+ grows it into the
// store-facing surface (the `Editor` handle the rune stores wrap).

import init, { core_version, Editor } from "nib-core";

let ready: Promise<void> | null = null;

/**
 * Instantiate the WASM module exactly once (idempotent across callers). The app is
 * `ssr = false`, so every call site already runs in the browser; still, keep init explicit
 * so nothing touches the core before the module is live.
 */
export function initCore(): Promise<void> {
  ready ??= init().then(() => undefined);
  return ready;
}

export { core_version, Editor };
