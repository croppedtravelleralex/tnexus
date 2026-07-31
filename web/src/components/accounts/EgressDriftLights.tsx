"use client";

import { cn } from "@/lib/utils";

export type EgressDayPoint = {
  date: string;
  status?: "ok" | "warn" | "error" | "none" | string;
  ip?: string;
};

const STATUS_CLASS: Record<string, string> = {
  ok: "bg-emerald-500",
  warn: "bg-amber-400",
  error: "bg-rose-500",
  none: "bg-stone-300",
};

export function EgressDriftLights({ days, className }: { days?: EgressDayPoint[] | null; className?: string }) {
  const points = Array.isArray(days) && days.length ? days : Array.from({ length: 7 }, (_, i) => ({ date: `d${i}`, status: "none" }));
  const seven = points.slice(-7);
  while (seven.length < 7) seven.unshift({ date: `pad-${seven.length}`, status: "none" });

  return (
    <div className={cn("flex items-center gap-0.5", className)} aria-label="近7日IP漂移监测">
      {seven.map((d, i) => {
        const st = String(d.status || "none").toLowerCase();
        return (
          <span
            key={`${d.date}-${i}`}
            title={`${d.date || "—"} · ${st}${"ip" in d && d.ip ? ` · ${d.ip}` : ""}`}
            className={cn("inline-block size-1.5 rounded-full", STATUS_CLASS[st] || STATUS_CLASS.none)}
          />
        );
      })}
    </div>
  );
}
