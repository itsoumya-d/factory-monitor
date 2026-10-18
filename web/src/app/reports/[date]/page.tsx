import Link from "next/link";
import { Suspense } from "react";
import ReportFor from "./ReportFor";

export default async function ReportPage({
  params,
}: {
  params: Promise<{ date: string }>;
}) {
  const { date } = await params;
  return (
    <div className="space-y-4">
      <h1 className="text-lg font-semibold text-zinc-100">
        Visual business report · {date}
      </h1>
      <Suspense fallback={<p className="text-sm text-zinc-500">loading…</p>}>
        <ReportFor date={date} />
      </Suspense>
      <p className="text-xs text-zinc-500">
        Route always accepts YYYY-MM-DD; try{" "}
        <Link className="underline" href="/reports/2026-01-01">
          /reports/2026-01-01
        </Link>
      </p>
    </div>
  );
}
