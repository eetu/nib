import { fileURLToPath } from "node:url";

import { defineConfig } from "vitest/config";

// Node unit tests for the pure-TS core (model, snap, geometry). jsdom gives us
// DOMParser/XMLSerializer for the document parse/serialize round-trip tests.
// Kept separate from vite.config.ts so the SvelteKit plugin stays out of the
// test run. Component (browser) tests are Phase-0-deferred — see the plan.
export default defineConfig({
  resolve: {
    // Mirror SvelteKit's $lib alias so tests can import it like app code does.
    alias: { $lib: fileURLToPath(new URL("./src/lib", import.meta.url)) },
  },
  test: {
    environment: "jsdom",
    include: ["src/**/*.test.ts"],
  },
});
