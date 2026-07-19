import { defineConfig, devices } from "@playwright/test";

// Browser smoke tests that drive the real app (WASM core + Svelte UI). Run via
// `just test-e2e` (builds first), or `yarn test:e2e` against an existing dist/.
export default defineConfig({
  testDir: "./e2e",
  fullyParallel: true,
  timeout: 30_000,
  use: {
    baseURL: "http://localhost:4319",
    trace: "on-first-retry",
  },
  webServer: {
    command: "node .yarn/releases/yarn-4.16.0.cjs preview --port 4319 --strictPort",
    url: "http://localhost:4319",
    reuseExistingServer: false,
    timeout: 60_000,
  },
  // Chromium runs the full suite (incl. Chromium-only paths: File System Access, clipboard).
  // WebKit + Firefox run only the `@cross` smoke subset — enough to prove the WASM core boots,
  // renders, takes pointer/keyboard edits, and undoes on all three engines, without tripping on
  // engine-gated APIs the editor already degrades gracefully around.
  projects: [
    { name: "chromium", use: { ...devices["Desktop Chrome"] } },
    { name: "webkit", use: { ...devices["Desktop Safari"] }, grep: /@cross/ },
    { name: "firefox", use: { ...devices["Desktop Firefox"] }, grep: /@cross/ },
  ],
});
