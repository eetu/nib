import { sveltekit } from "@sveltejs/kit/vite";
import { defineConfig } from "vite";

// Vite dev server (:5173). Output goes to dist/ (wired in svelte.config.js via
// adapter-static). nib is a fully client-side tool today, so there is no
// backend to talk to — the /api + /status proxy below is dormant, kept ready
// for the day the rust-axum serve-shell lands (see the plan). Harmless now:
// nothing hits those paths.
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
      "/api": "http://localhost:3010",
      "/status": "http://localhost:3010",
    },
  },
});
