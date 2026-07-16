import { sveltekit } from "@sveltejs/kit/vite";
import { defineConfig } from "vite";

// Vite dev server (:5173). Output goes to dist/ (wired in svelte.config.js via
// adapter-static). In connected mode (`PUBLIC_NIB_BACKEND=1`, e.g. `just dev`) the SPA talks to
// the rust-axum backend on :4321 via the same-origin proxy below (REST + the sync WebSocket).
// Standalone builds never hit these paths.
export default defineConfig({
  plugins: [sveltekit()],
  // The nib-core WASM engine is a `link:` dep resolving to ../core/pkg (a sibling of
  // frontend/, outside the Vite root). Two knobs make it load cleanly:
  //  - fs.allow ".." so the dev server may serve the .wasm from the repo root.
  //  - optimizeDeps.exclude so esbuild's pre-bundler doesn't rewrite the glue's
  //    `new URL('…_bg.wasm', import.meta.url)` and break wasm resolution.
  optimizeDeps: {
    exclude: ["nib-core"],
  },
  server: {
    fs: {
      allow: [".."],
    },
    proxy: {
      "/api": "http://localhost:4321",
      "/mcp": "http://localhost:4321",
      "/ws": { target: "ws://localhost:4321", ws: true },
    },
  },
});
