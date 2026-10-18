"use client";

import { useMemo, useState } from "react";
import { apiFetch } from "@/lib/live";
import type { CorrectionResult, PendingWindow } from "@/lib/types";
import { FAILURE_TAGS, encodeFailureTags, needsBigSwingConfirmation, rewardPreview } from "@/lib/formats";

/**
 * Multi-step correction flow (docs/12 §3): zone → window → counts → tags →
 * notes → submit. The bad-delta guard (>25% swing) shows the confirmation
 * question and makes the retrain checkbox mandatory before submit unlocks.
 */
export default function CorrectionForm({ windows }: { windows: PendingWindow[] }) {
  const zones = useMemo(
    () => [...new Set(windows.map((w) => w.zone_id))],
    [windows],
  );
  const [zone, setZone] = useState<string>("");
  const [win, setWin] = useState<PendingWindow | null>(null);
  const [actual, setActual] = useState<number | "">("");
  const [tags, setTags] = useState<string[]>([]);
  const [notes, setNotes] = useState("");
  const [wantsRetrain, setWantsRetrain] = useState(false);
  const [confirmSwing, setConfirmSwing] = useState(false);
  const [last, setLast] = useState<CorrectionResult | null>(null);
  const [err, setErr] = useState<string | null>(null);

  const zoneWindows = windows.filter((w) => w.zone_id === zone);
  const reported = win?.predicted_count ?? 0;
  const swing = needsBigSwingConfirmation(reported, Number(actual));
  const reward = rewardPreview(reported, Number(actual));

  const toggleTag = (t: string) =>
    setTags((existing) =>
      existing.includes(t) ? existing.filter((x) => x !== t) : [...existing, t],
    );

  const ready = Number.isFinite(Number(actual)) && actual !== "";
  const blocked = !win || !ready || (swing && (!confirmSwing || !wantsRetrain));

  async function submit() {
    if (!win) return;
    try {
      const body = {
        zone_id: win.zone_id,
        reported_count: win.predicted_count,
        actual_count: Number(actual),
        ts_from: win.ts_from,
        ts_to: win.ts_to,
        failure_tags: encodeFailureTags(tags),
        notes: notes || null,
        operator_id: "operator", // replaced by SSO identity at auth merge
        wants_retrain: wantsRetrain,
      };
      const res = await apiFetch<CorrectionResult>("/api/v1/audit/corrections", {
        method: "POST",
        body: JSON.stringify(body),
      });
      setLast(res);
      setErr(null);
    } catch (e) {
      setErr(String(e).replace(/^Error:\s*/, ""));
    }
  }

  return (
    <form
      data-testid="correction-form"
      className="space-y-4 rounded border border-zinc-800 bg-zinc-900 p-4"
      onSubmit={(e) => {
        e.preventDefault();
        void submit();
      }}
    >
      <h3 className="text-sm font-semibold text-zinc-200">Submit a correction</h3>

      <label htmlFor="zone-select" className="block text-xs text-zinc-400">Zone</label>
      <select
        id="zone-select"
        data-testid="zone-select"
        className="w-full rounded bg-zinc-800 px-2 py-1.5 text-zinc-100"
        value={zone}
        onChange={(e) => {
          setZone(e.target.value);
          setWin(null);
        }}
      >
        <option value="" />
        {zones.map((z) => (
          <option key={z} value={z}>{z}</option>
        ))}
      </select>

      <label htmlFor="window-select" className="block text-xs text-zinc-400">Window</label>
      <select
        id="window-select"
        data-testid="window-select"
        className="w-full rounded bg-zinc-800 px-2 py-1.5 text-zinc-100"
        value={win?.window_id ?? ""}
        onChange={(e) =>
          setWin(zoneWindows.find((w) => w.window_id === e.target.value) ?? null)
        }
      >
        <option value="" />
        {zoneWindows.map((w) => (
          <option key={w.window_id} value={w.window_id}>
            {w.ts_from} → {w.ts_to} · predicted {w.predicted_count}
          </option>
        ))}
      </select>

      <label htmlFor="actual-input" className="block text-xs text-zinc-400">
        Actual count (predicted {reported})
      </label>
      <input
        id="actual-input"
        data-testid="actual-input"
        type="number"
        className="w-32 rounded bg-zinc-800 px-2 py-1.5 text-right font-mono text-zinc-100"
        value={actual}
        onChange={(e) => setActual(e.target.value === "" ? "" : Number(e.target.value))}
      />

      <fieldset className="flex flex-wrap gap-2">
        <legend className="mb-1 text-xs text-zinc-400">Failure tags</legend>
        {FAILURE_TAGS.map((t) => (
          <button
            type="button"
            key={t}
            onClick={() => toggleTag(t)}
            aria-pressed={tags.includes(t)}
            className={`rounded px-2 py-1 text-xs ${
              tags.includes(t) ? "bg-lime-600 text-lime-50" : "bg-zinc-800 text-zinc-300"
            }`}
          >
            {t.toLowerCase().replace(/_/g, " ")}
          </button>
        ))}
      </fieldset>

      <textarea
        data-testid="notes"
        placeholder="notes"
        className="w-full rounded bg-zinc-800 px-2 py-1.5 text-sm text-zinc-100"
        value={notes}
        onChange={(e) => setNotes(e.target.value)}
      />

      <div className="flex flex-col gap-1 text-xs text-zinc-300">
        <label className="flex items-center gap-2">
          <input
            data-testid="wants-retrain"
            type="checkbox"
            checked={wantsRetrain}
            onChange={(e) => setWantsRetrain(e.target.checked)}
          />
          wants retrain
        </label>
        <label className="flex items-center gap-2">
          <input
            data-testid="confirm-swing"
            type="checkbox"
            checked={confirmSwing}
            onChange={(e) => setConfirmSwing(e.target.checked)}
          />
          confirm big swing
        </label>
        <span className="font-mono text-zinc-500" data-testid="reward-preview">
          reward preview: {reward.toFixed(2)}
        </span>
      </div>

      {swing && (
        <p data-testid="swing-warning" className="text-xs font-semibold text-amber-400">
          Bad-delta guard: |actual − predicted| &gt; 25% of {reported}. Confirm
          the swing and tick “wants retrain”.
        </p>
      )}

      <button
        data-testid="submit-correction"
        className="w-44 rounded bg-lime-500 px-3 py-2 text-sm font-semibold text-zinc-950 disabled:opacity-40"
        disabled={blocked}
        type="submit"
      >
        Submit correction
      </button>

      {last && (
        <p data-testid="submit-result" className="font-mono text-xs text-lime-300">
          {last.correction_id} · reward {last.reward_computed} · retrain: {String(last.retrain)}
        </p>
      )}
      {err && <p className="text-xs text-red-400">{err}</p>}
    </form>
  );
}
