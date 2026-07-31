"use client";

import { cn } from "@/lib/utils";

export type CfDayPoint = {
  date?: string;
  ok?: number;
  cf?: number;
  image_fail?: number;
};

type Props = {
  days?: CfDayPoint[] | null;
  className?: string;
};

export type CfLightStatus = "none" | "ok" | "warn" | "error";

export function summarizeCfDay(row?: CfDayPoint | null): {
  ok: number;
  cf: number;
  image_fail: number;
  status: CfLightStatus;
} {
  if (!row || typeof row !== "object") {
    return { ok: 0, cf: 0, image_fail: 0, status: "none" };
  }
  const ok = Math.max(0, Number(row.ok) || 0);
  const cf = Math.max(0, Number(row.cf) || 0);
  const image_fail = Math.max(0, Number(row.image_fail) || 0);
  const total = ok + cf + image_fail;
  let status: CfLightStatus = "none";
  if (total > 0) {
    if (cf + image_fail > ok) status = "error";
    else if (cf > 0) status = "warn";
    else status = "ok";
  }
  return { ok, cf, image_fail, status };
}

const STATUS_CLASS: Record<CfLightStatus, string> = {
  ok: "bg-emerald-500",
  warn: "bg-amber-400",
  error: "bg-rose-500",
  none: "bg-stone-300",
};

const STATUS_LABEL: Record<CfLightStatus, string> = {
  ok: "无 CF 403",
  warn: "有 CF 但少于成功",
  error: "CF/生图失败多于成功",
  none: "无业务样本",
};

export function CfStatusLight({ days, className }: Props) {
  const points =
    Array.isArray(days) && days.length ? days : Array.from({ length: 7 }, (_, i) => ({ date: `d${i}` }));
  const seven = points.slice(-7);
  while (seven.length < 7) {
    seven.unshift({ date: `pad-${seven.length}` });
  }

  return (
    <div className={cn("flex items-center gap-0.5", className)} aria-label="近7日CF状态">
      {seven.map((row, i) => {
        const { ok, cf, image_fail, status } = summarizeCfDay(row);
        const label = `${row.date || "—"} · ${STATUS_LABEL[status]} · ok=${ok} cf=${cf} fail=${image_fail}`;
        return (
          <span
            key={`${row.date}-${i}`}
            title={label}
            className={cn("inline-block size-1.5 rounded-full", STATUS_CLASS[status])}
          />
        );
      })}
      <span className="ml-0.5 text-[10px] text-stone-400">CF</span>
    </div>
  );
}

export function cfDaysForAccount(account: {
  cf_daily?: CfDayPoint[] | null;
}): CfDayPoint[] {
  const today = new Date();
  const dates: string[] = [];
  for (let i = 6; i >= 0; i -= 1) {
    const d = new Date(today);
    d.setDate(today.getDate() - i);
    dates.push(d.toISOString().slice(0, 10));
  }
  const byDate = new Map<string, CfDayPoint>();
  for (const row of account.cf_daily || []) {
    if (!row || typeof row !== "object") continue;
    const date = String(row.date || "").slice(0, 10);
    if (!date) continue;
    byDate.set(date, {
      date,
      ok: Math.max(0, Number(row.ok) || 0),
      cf: Math.max(0, Number(row.cf) || 0),
      image_fail: Math.max(0, Number(row.image_fail) || 0),
    });
  }
  return dates.map((date) => byDate.get(date) || { date, ok: 0, cf: 0, image_fail: 0 });
}
