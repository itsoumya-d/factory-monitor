import type { Page } from "@playwright/test";

/** Real /audit/pending rows (AuditCorrectionRow serde shape — repo.rs:127). */
export const PENDING_CORRECTION = {
  testId: "c1-e2e",
  body: (id: string) => ({
    correction_id: id,
    feedback: {
      zone_id: "line_1",
      reported_count: 100,
      actual_count: 250,
      ts_from: "2026-10-18T14:00:00Z",
      ts_to: "2026-10-18T15:00:00Z",
      failure_tags: 1,
      notes: "missed crates",
      operator_id: "op_1",
      wants_retrain: true,
    },
    outcome: "submitted",
    reward_computed: -1.5,
    root_cause_report: null,
    created_at: "2026-10-18T15:05:00Z",
  }),
};

/** Install /api/v1 interception with REAL backend JSON shapes. Returns the
 *  list of seen commit bodies so tests assert the request contract. */
export function interceptBackend(page: Page) {
  const commits: unknown[] = [];
  const queue = [PENDING_CORRECTION.body(PENDING_CORRECTION.testId)];

  void page.route("**/api/v1/audit/pending", async (route) => {
    const url = route.request().url();
    if (new URL(url).pathname === "/api/v1/audit/pending") {
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({ pending: queue }),
      });
    }
  });
  void page.route(/^.*\/api\/v1\/labeling\/queue.*$/, (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ queue: [] }),
    }),
  );
  void page.route(/^.*\/api\/v1\/labeling\/[^/]+\/approve$/, (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ frame_id: "0190", approved: true, reviewed_by: "op_1" }),
    }),
  );
  void page.route(/^.*\/api\/v1\/audit\/corrections\/[^/]+\/commit$/, async (route) => {
    commits.push(route.request().postDataJSON());
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ outcome: "committed", idempotent: false }),
    });
    queue.length = 0;
  });

  return { commits };
}
