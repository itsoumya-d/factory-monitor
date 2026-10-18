"use client";

import { useState } from "react";
import { apiFetch } from "@/lib/live";
import type { CorrectionResult } from "@/lib/types";
import { FAILURE_TAGS, encodeFailureTags, needsBigSwingConfirmation, rewardPreview } from "@/lib/formats";

/**
 * Correction submission (docs/08 §3): operator types the zone window and the
 * true count. The bad-delta guard (>25% swing) shows the confirmation
 * question and makes the retrain checkbox mandatory before submit unlocks.
 * POST body matches the Rust CorrectionSubmit extractor exactly.
 */
export default function CorrectionForm() {
  const [zone, setZone] = useState("");
  const [reported, setReported] = useState<number | "">("");
  const [actual, setActual] = useState<number | "">("");
  const [tsFrom, setTsFrom] = useState("");
  const [tsTo, setTsTo] = useState("");
  const [tags, setTags] = useState<string[]>([]);
  const [notes, setNotes] = useState("");
  const [wantsRetrain, setWantsRetrain] = useState(false);
  const [confirmSwing, setConfirmSwing] = useState(false);
  const [last, setLast] = useState<CorrectionResult | null>(null);
  const [err, setErr] = useState<string | null>(null);

  const swing = needsBigSwingConfirmation(Number(reported), Number(actual));
  const reward = rewardPreview(Number(reported), Number(actual));

  const toggleTag = (t: string) =>
    setTags((existing) =>
      existing.includes(t) ? existing.filter((x) => x !== t) : [...existing, t],
    );

  const countsReady = reported !== "" && actual !== "" && zone.trim() !== "";
  const blocked = !countsReady || (swing && (!confirmSwing || !wantsRetrain));

  /** datetime-local value → wire ISO (RFC3339 with Z). */
  const toIso = (local: string): string | null => {
    if (!local) return null;
    const d = new Date(local);
    return Number.isNaN(d.getTime()) ? null : d.toISOString();
  };

  async function submit() {
    const from = toIso(tsFrom);
    const to = toIso(tsTo);
    try {
      const res = await apiFetch<CorrectionResult>("/api/v1/audit/corrections", {
        method: "POST",
        body: JSON.stringify({
          zone_id: zone.trim(),
          reported_count: Number(reported),
          actual_count: Number(actual),
          ts_from: from,
          ts_to: to,
          failure_tags: encodeFailureTags(tags),
          notes: notes || null,
          operator_id: "operator", // replaced by SSO identity at auth merge
          wants_retrain: wantsRetrain,
        }),
      });
      setLast(res);
      setErr(null);
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
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

      <label htmlFor="zone-select" className="block text-xs text-zinc-400">Zone id</label>
      <input
        id="zone-select"
        data-testid="zone-select"
        className="w-56 rounded bg-zinc-800 px-2 py-1.5 font-mono text-zinc-100"
        value={zone}
        onChange={(e) => setZone(e.target.value)}
        placeholder="line_1"
      />

      <div className="flex gap-3">
        <div>
          <label htmlFor="ts-from" className="block text-xs text-zinc-400">Window from</label>
          <input
            id="ts-from"
            data-testid="ts-from"
            type="datetime-local"
            className="rounded bg-zinc-800 px-2 py-1.5 text-zinc-100"
            value={tsFrom}
            onChange={(e) => setTsFrom(e.target.value)}
          />
        </div>
        <div>
          <label htmlFor="ts-to" className="block text-xs text-zinc-400">Window to</label>
          <input
            id="ts-to"
            data-testid="ts-to"
            type="datetime-local"
            className="rounded bg-zinc-800 px-2 py-1.5 text-zinc-100"
            value={tsTo}
            onChange={(e) => setTsTo(e.target.value)}
          />
        </div>
      </div>

      <div className="flex gap-3">
        <div>
          <label htmlFor="reported-input" className="block text-xs text-zinc-400">Reported count</label>
          <input
            id="reported-input"
            data-testid="reported-input"
            type="number"
            className="w-32 rounded bg-zinc-800 px-2 py-1.5 text-right font-mono text-zinc-100"
            value={reported}
            onChange={(e) => setReported(e.target.value === "" ? "" : Number(e.target.value))}
          />
        </div>
        <div>
          <label htmlFor="actual-input" className="block text-xs text-zinc-400">Actual count</label>
          <input
            id="actual-input"
            data-testid="actual-input"
            type="number"
            className="w-32 rounded bg-zinc-800 px-2 py-1.5 text-right font-mono text-zinc-100"
            value={actual}
            onChange={(e) => setActual(e.target.value === "" ? "" : Number(e.target.value))}
          />
        </div>
      </div>

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
          Bad-delta guard: |actual − reported| &gt; 25% of {reported}. Confirm
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
