import { expect, test } from "@playwright/test";
import { interceptBackend } from "./fixtures";

/** GET /api/v1/zones/{z}/hourly response body — real agg.rs shape. */
const HOURLY = {
  zone_id: "line_1",
  date: "2026-10-18",
  hours: [
    { hour: "2026-10-18T08:00:00Z", packages: 14, avg_conf: 0.93, occluded_events: 0 },
    { hour: "2026-10-18T09:00:00Z", packages: 3, avg_conf: 0.4, occluded_events: 1 },
  ],
};

test("report: chart, heatmap and table render the per-zone hourly wire", async ({ page }) => {
  test.setTimeout(60_000);
  interceptBackend(page);
  void page.route(/^.*\/api\/v1\/zones\/line_1\/hourly.*$/, (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(HOURLY),
    }),
  );

  const today = new Date().toISOString().slice(0, 10);
  await page.goto(`/reports/${today}`);

  // Recharts root mounts as soon as the series shape arrives.
  await expect(page.locator(".recharts-wrapper").first()).toBeVisible({ timeout: 45_000 });

  const table = page.getByTestId("hourly-table");
  await expect(table).toContainText("08:00");
  await expect(table).toContainText("14");

  // Heatmap grid: storm-hued hottest cell + busiest callout.
  const stormCell = page.getByTestId("heat-line_1-08");
  await expect(stormCell).toHaveAttribute("data-ramp", "storm");
  await expect(page.getByTestId("heatmap-busiest")).toContainText("08:00");

  await page.screenshot({ path: "e2e/__screenshots__/report.png", fullPage: true });
});
