import { expect, test } from "@playwright/test";

/** Wire-true shapes for the new surfaces (docs/12 §3 + §5). */
const ROW = {
  frame_id: "0190-abc-01",
  zone_id: "dock_gate",
  ts: "2026-10-18T10:00:00Z",
  camera_id: "camD",
  divergence: 0.5,
  uncertainty_mask: 1,
  promotion: "Amber",
  coco_annotations: null,
};

const WINDOW = {
  zone_id: "dock_gate",
  ts: "2026-10-18T10:00:00Z",
  focal_frame_id: "0190-abc-01",
  frames: [
    { frame_id: "0190-abc-01", ts: "2026-10-18T10:00:00Z", camera_id: "camD", uncertainty_mask: 1, divergence: 0.5, promotion: "Amber" },
    { frame_id: "0190-abc-02", ts: "2026-10-18T10:05:00Z", camera_id: "camD", uncertainty_mask: 0, divergence: null, promotion: "Amber" },
  ],
};

test("review: window ±30m opens the sampled context scrubber", async ({ page }) => {
  test.setTimeout(60_000);
  let windowCalls = 0;
  void page.route(/^.*\/api\/v1\/labeling\/queue.*$/, (route) =>
    route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify({ queue: [ROW] }) }),
  );
  void page.route(/^.*\/api\/v1\/snapshots\/window.*$/, async (route) => {
    const url = route.request().url();
    expect(url).toContain("zone_id=dock_gate");
    expect(url).toContain("span=1800");
    windowCalls += 1;
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(WINDOW),
    });
  });
  void page.route(/^.*\/api\/v1\/snapshots\/image\/0190-abc.*$/, (route) =>
    route.fulfill({ status: 200, contentType: "image/jpeg", body: Buffer.from([0xff, 0xd8]) }),
  );

  await page.goto("/review");
  await page.getByTestId("window-0190-abc-01").click();
  await expect(page.getByTestId("suspect-strip")).toBeVisible();
  expect(windowCalls).toBe(1);
  await expect(page.getByTestId("suspect-tile-0")).toContainText("10:00:00");
  await expect(page.getByTestId("suspect-tile-1")).toContainText("10:05:00");
  // focal tile active with its OCCLUDED_ESTIMATE chip
  await expect(page.getByTestId("suspect-flags")).toContainText("OCCLUDED_ESTIMATE");
  await page.getByTestId("suspect-tile-1").click();
  await expect(page.getByTestId("suspect-flags")).toContainText("no uncertainty flags");
  await page.getByTestId("suspect-close").click();
  await expect(page.getByTestId("suspect-window")).toHaveCount(0);
  expect(windowCalls).toBe(1);
});

test("policy: pause preset posts the override, revert posts last-safe", async ({ page }) => {
  test.setTimeout(60_000);
  const overrides: { until?: string; note?: string }[] = [];
  void page.route(/^.*\/api\/v1\/rl\/state.*$/, (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ paused_until: null, override_note: null, rejected_24h: 0 }),
    }),
  );
  void page.route(/^.*\/api\/v1\/rl\/history.*$/, (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        date: "2026-10-18",
        transitions: [
          {
            transition_id: "t1",
            ts: "2026-10-18T09:00:00Z",
            zone_id: null,
            before: { conf_threshold: 0.45, iou_threshold: 0.45, vision_weight: 0.7, track_buffer_frames: 45 },
            after: { conf_threshold: 0.51, iou_threshold: 0.5, vision_weight: 0.72, track_buffer_frames: 60 },
            reward: -0.65,
            was_clamped: false,
            rejected: false,
            reason: "eod_correction",
          },
        ],
      }),
    }),
  );
  void page.route(/^.*\/api\/v1\/rl\/override$/, async (route) => {
    overrides.push(route.request().postDataJSON() as never);
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ ok: true, paused_until: "2026-10-18T12:00:00Z", note: "x" }),
    });
  });

  await page.goto("/settings/policy");
  await expect(page.getByTestId("rl-history")).toContainText("eod_correction");

  await page.getByTestId("pause-2-hours").click();
  await expect(overrides[0]).toEqual({ note: "manual pause (2 hours)", until: expect.any(String) });
  expect(new Date(overrides[0].until as string).getTime()).toBeGreaterThan(Date.now());

  await page.getByTestId("revert-button").click();
  await expect(page.locator("data-testid=paused-chip")).toBeTruthy();
});
