import { expect, test } from "@playwright/test";

/** Real queue row incl. the editor preload field (routes.rs get_label_queue). */
const FRAME_ID = "0190-ab12-cd34";
const ROW = {
  frame_id: FRAME_ID,
  zone_id: "dock_gate",
  ts: "2026-10-18T09:00:00Z",
  camera_id: "camD",
  divergence: 0.42,
  uncertainty_mask: 1,
  promotion: "Amber",
  coco_annotations: null,
};

/** Minimal valid JPEG served as the frame bytes (image route). */
const JPEG_1PX = Buffer.from(
  "/9j/4AAQSkZJRgABAQAAAQABAAD/2wBDAAMCAgICAgMCAgIDAwMDBAYEBAQEBAgGBgUGCQgKCgkICAkKDA8MCgsOCwkJDRENDg8QEBEQCgwSExIQEw8QEBD/wAALCAABAAEBAREA/8QAFAABAQAAAAAAAAAAAAAAAAAAAAAv/2gAIAQEAAD8A0s8g/9k=",
  "base64",
);

test("review viewer: frame bytes, drag box, approve posts the COCO body", async ({ page }) => {
  test.setTimeout(60_000);
  const approves: unknown[] = [];
  // Server-side model: the approved frame leaves the queue (LabeledOk).
  const queue = [{ ...ROW }];
  void page.route(/^.*\/api\/v1\/labeling\/queue.*$/, (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ queue: [...queue] }),
    }),
  );
  void page.route(new RegExp(`/api/v1/snapshots/image/${FRAME_ID}$`), (route) =>
    route.fulfill({ status: 200, contentType: "image/jpeg", body: JPEG_1PX.toString("binary") }),
  );
  void page.route(new RegExp(`/api/v1/labeling/${FRAME_ID}/approve$`), async (route) => {
    approves.push(route.request().postDataJSON());
    queue.length = 0;
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ approved: true }),
    });
  });

  await page.goto("/review");
  const row = page.getByTestId(`label-${FRAME_ID}`);
  await expect(row).toContainText("divergence 0.42");

  // Open the inspector; the image route serves real bytes.
  await page.getByTestId(`inspect-${FRAME_ID}`).click();
  await expect(row.locator("img")).toBeVisible();

  // Draw a box over the frame, trying the package category.
  const canvas = page.getByTestId("box-canvas");
  const box = await canvas.boundingBox();
  const x0 = box!.x + box!.width * 0.2;
  const y0 = box!.y + box!.height * 0.2;
  await page.mouse.move(x0, y0);
  await page.mouse.down();
  await page.mouse.move(x0 + box!.width * 0.3, y0 + box!.height * 0.3, { steps: 4 });
  await page.mouse.up();
  await expect(page.getByTestId("box-0")).toBeVisible();

  // Approve gates on the operator id, then POSTs frame + annotations.
  const btn = page.getByTestId(`approve-${FRAME_ID}`);
  await expect(btn).toBeDisabled();
  await page.getByTestId("operator-input").fill("op_1");
  await btn.click();

  await expect(row).toHaveCount(0);
  await expect(page.getByTestId("review-list")).toContainText("queue clear");

  const body = approves[0] as {
    operator_id: string;
    coco_annotations: {
      images: { id: number; width: number; height: number }[];
      annotations: { id: number; image_id: number; category_id: number; bbox: number[]; area: number; iscrowd: number }[];
      categories: string[];
    };
  };
  expect(body.operator_id).toBe("op_1");
  expect(body.coco_annotations.images[0]).toEqual({
    id: 1,
    file_name: `${FRAME_ID}.jpg`,
    height: 480,
    width: 640,
  });
  expect(body.coco_annotations.annotations[0].bbox.length).toBe(4);
  expect(body.coco_annotations.annotations[0].iscrowd).toBe(0);
  expect(body.coco_annotations.categories.length).toBeGreaterThan(0);

  await page.screenshot({ path: "e2e/__screenshots__/review.png", fullPage: true });
});
