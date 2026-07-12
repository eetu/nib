import { sveltekit } from "@sveltejs/kit/vite";
import { defineConfig } from "vite";

// Vite dev server (:5173). Output goes to dist/ (wired in svelte.config.js via
// adapter-static). nib is a fully client-side tool today, so there is no
// backend to talk to — the /api + /status proxy below is dormant, kept ready
// for the day the rust-axum serve-shell lands (see the plan). Harmless now:
// nothing hits those paths.
export default defineConfig({
  plugins: [sveltekit()],
  server: {
    proxy: {
      "/api": "http://localhost:3010",
      "/status": "http://localhost:3010",
    },
  },
});
