import svelte from "@anarkisti/eslint-config/svelte";

import svelteConfig from "./svelte.config.js";

// Shared house preset (node base + eslint-plugin-svelte + TS parser wiring).
// Factory: it threads svelte.config.js into the parser for Svelte-aware rules.
// See coding-style:svelte / the eslint-config repo.
export default [
  ...svelte(svelteConfig),
  // e2e specs + the playwright config live outside the app's tsconfig project; lint them
  // through their own tooling, not the type-aware app rules.
  { ignores: ["dist/", ".svelte-kit/", "e2e/", "playwright.config.ts", "test-results/"] },
];
