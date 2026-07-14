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
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"] } }],
});
