import { expect, test } from "@playwright/test";
import { interceptBackend, PENDING_CORRECTION } from "./fixtures";

test("audit desk: pending commits render from the real wire and commit", async ({ page }) => {
  const { commits } = interceptBackend(page);
  await page.goto("/audit");

  const row = page.getByTestId(`commit-row-${PENDING_CORRECTION.testId}`);
  await expect(row).toBeVisible();
  await expect(row).toContainText("line_1");
  await expect(row).toContainText("100→250");
  await expect(row).toContainText("reward -1.50");

  // Delta 150 > BIG_DELTA_CONFIRM: commit locked until the confirm checkbox.
  const btn = page.getByTestId(`commit-btn-${PENDING_CORRECTION.testId}`);
  await expect(btn).toBeDisabled();
  await page.getByTestId(`commit-confirm-${PENDING_CORRECTION.testId}`).check();
  await btn.click();

  // Row leaves the queue once committed (server no longer lists it).
  await expect(row).toHaveCount(0);
  await expect(page.getByTestId("audit-clear")).toBeVisible();
  expect(commits).toEqual([{ confirm: true }]);

  await page.screenshot({ path: "e2e/__screenshots__/audit.png", fullPage: true });
});

test("audit desk offline: reconnecting chip shows on dead WS, form still usable", async ({ page }) => {
  // No interception: REST fails through the dead proxy, WS can't upgrade.
  await page.goto("/audit");
  const form = page.getByTestId("correction-form");
  await expect(form).toBeVisible({ timeout: 30_000 });

  // With no proxy target the pending fetch errors into the offline notice.
  await expect(page.getByTestId("audit-offline")).toContainText("backend unreachable", {
    timeout: 30_000,
  });
  // WS /ws/live has no server → the chip says reconnecting, never "live".
  await expect(page.getByTestId("conn-state")).toContainText("reconnecting");

  await page.screenshot({ path: "e2e/__screenshots__/audit-offline.png", fullPage: true });
});
