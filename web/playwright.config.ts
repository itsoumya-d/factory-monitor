import { defineConfig, devices } from "@playwright/test";

/**
 * E2E against the Next dev server running the standalone bundle. REST is
 * intercepted at the browser layer (page.route) with REAL wire shapes — the
 * same JSON the Rust api-server emits — so no backend/Docker is needed.
 * WS /ws/live fails against nothing (proxy ECONNREFUSED): exactly what the
 * stale-connection banner is for, so we assert it.
 */
export default defineConfig({
  testDir: "./e2e",
  fullyParallel: true,
  forbidOnly: Boolean(process.env.CI),
  retries: process.env.CI ? 2 : 0,
  reporter: [["list"], ["junit", { outputFile: "test-results/junit.xml" }]],
  use: {
    ...devices["Desktop Chrome"],
    baseURL: "http://127.0.0.1:3000",
    screenshot: "only-on-failure",
  },
  outputDir: "test-results",
  webServer: {
    command: "npm run dev",
    url: "http://127.0.0.1:3000",
    reuseExistingServer: !process.env.CI,
    timeout: 120_000,
  },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"] } }],
});
