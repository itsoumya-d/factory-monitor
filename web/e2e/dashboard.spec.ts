import { expect, test } from "@playwright/test";
import { interceptBackend } from "./fixtures";

test("dashboard shell: nav, headline counters and dead-WS honest chip", async ({ page }) => {
  interceptBackend(page);
  await page.goto("/dashboard");

  // App chrome.
  await expect(page.locator("header")).toContainText("Shado");
  await expect(page.locator("header")).toContainText("Live ops");
  await expect(page.locator("header")).toContainText("Review");
  await expect(page.locator("header")).toContainText("ROI editor");

  // No WS backend → the connection chip must say reconnecting, not live.
  await expect(page.getByTestId("conn-state")).toContainText("reconnecting", {
    timeout: 30_000,
  });

  await page.screenshot({ path: "e2e/__screenshots__/dashboard.png", fullPage: true });
});
