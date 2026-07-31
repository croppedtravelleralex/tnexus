"use client";

import { ChevronLeft, ChevronRight } from "lucide-react";
import { cn } from "@/lib/utils";

export type HeatmapTimezone = "Asia/Shanghai" | "Asia/Singapore";

type Props = {
  weekOffset: number;
  weekLabel: string;
  timezone: HeatmapTimezone;
  timezoneLabel: string;
  loading?: boolean;
  onWeekOffsetChange: (offset: number) => void;
  onTimezoneChange: (tz: HeatmapTimezone) => void;
  className?: string;
};

export function BindingActivityHeatmapToolbar({
  weekOffset,
  weekLabel,
  timezone,
  timezoneLabel,
  loading = false,
  onWeekOffsetChange,
  onTimezoneChange,
  className,
}: Props) {
  const canGoNext = weekOffset < 0;
  return (
    <div className={cn("flex flex-wrap items-center gap-2 rounded-lg border border-[var(--neo-border)] bg-white/80 px-2.5 py-1.5", className)}>
      <span className="text-[10px] font-medium text-[var(--neo-muted)]">活动热力</span>
      <div className="flex items-center gap-0.5">
        <button type="button" className="inline-flex h-6 items-center gap-0.5 rounded-md border px-1.5 text-[10px] disabled:opacity-40" disabled={loading} onClick={() => onWeekOffsetChange(weekOffset - 1)}>
          <ChevronLeft className="size-3" />上一周
        </button>
        <button type="button" className={cn("inline-flex h-6 items-center rounded-md border px-2 text-[10px] font-medium", weekOffset === 0 ? "border-sky-300 bg-sky-50 text-sky-800" : "")} disabled={loading || weekOffset === 0} onClick={() => onWeekOffsetChange(0)}>本周</button>
        <button type="button" className="inline-flex h-6 items-center gap-0.5 rounded-md border px-1.5 text-[10px] disabled:opacity-40" disabled={loading || !canGoNext} onClick={() => onWeekOffsetChange(weekOffset + 1)}>
          下一周<ChevronRight className="size-3" />
        </button>
      </div>
      <span className="text-[11px] font-semibold tabular-nums">{weekLabel}</span>
      <div className="ml-auto flex items-center gap-1">
        <button type="button" className={cn("h-6 rounded-md border px-2 text-[10px]", timezone === "Asia/Shanghai" ? "border-sky-300 bg-sky-50" : "")} onClick={() => onTimezoneChange("Asia/Shanghai")}>北京时间</button>
        <button type="button" className={cn("h-6 rounded-md border px-2 text-[10px]", timezone === "Asia/Singapore" ? "border-sky-300 bg-sky-50" : "")} onClick={() => onTimezoneChange("Asia/Singapore")}>新加坡</button>
        <span className="text-[10px] text-[var(--neo-muted)]">{loading ? "加载中…" : timezoneLabel}</span>
      </div>
    </div>
  );
}
