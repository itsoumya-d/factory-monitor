"use client";

import { useEffect } from "react";
import { useRouter } from "next/navigation";

/** /reports → today's EOD report (docs/12 §5). */
export default function ReportsIndex() {
  const router = useRouter();
  useEffect(() => {
    router.replace(`/reports/${new Date().toISOString().slice(0, 10)}`);
  }, [router]);
  return <p className="text-sm text-zinc-500">opening today&apos;s report…</p>;
}
