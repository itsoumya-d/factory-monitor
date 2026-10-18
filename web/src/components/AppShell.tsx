"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import { useEffect } from "react";
import { useLive } from "@/lib/live";

const NAV = [
  { href: "/dashboard", label: "Live ops" },
  { href: "/audit", label: "Corrections" },
  { href: "/reports", label: "Reports" },
  { href: "/settings/rois", label: "ROI editor" },
  { href: "/settings/policy", label: "RL knobs" },
] as const;

/** Top nav + stale-data banner + WS boot (docs/12 §/4). */
export default function AppShell({ children }: { children: React.ReactNode }) {
  const connected = useLive((s) => s.connected);
  const lastMessageAt = useLive((s) => s.lastMessageAt);
  const now = useLive((s) => s.now);
  const pathname = usePathname();

  // Boot the feed exactly once per document.
  useEffect(() => {
    useLive.api.connect(`/ws/live`);
    return () => useLive.api.close();
  }, []);

  const stale = lastMessageAt !== 0 && now - lastMessageAt > 10_000;

  return (
    <div className="min-h-screen bg-zinc-950 text-zinc-100">
      <header className="flex items-center gap-6 border-b border-zinc-800 px-4 py-2">
        <Link href="/dashboard" className="font-semibold tracking-tight">
          Shado
        </Link>
        <nav className="flex gap-4 text-sm">
          {NAV.map((n) => (
            <Link
              key={n.href}
              href={n.href}
              className={`px-2 py-1 rounded ${
                pathname === n.href ? "bg-lime-500 text-zinc-950 font-medium" : "text-zinc-300 hover:bg-zinc-900"
              }`}
            >
              {n.label}
            </Link>
          ))}
        </nav>
        <span
          data-testid="conn-state"
          className={`ml-auto font-mono text-xs ${
            connected ? "text-lime-300" : "text-red-400"
          }`}
        >
          {connected ? "live" : "reconnecting"}
        </span>
      </header>
      {stale && (
        <div role="status" data-testid="stale-banner" className="bg-amber-500/90 px-4 py-1.5 text-sm font-medium text-zinc-950">
          Stale feed — last message over 10s old.
        </div>
      )}
      <main className="mx-auto max-w-6xl p-4">{children}</main>
    </div>
  );
}
