"use client";

import { memo, useMemo } from "react";
import { SLOT_LABELS } from "@/components/accounts/BindingSgHeatmap";
import { cn } from "@/lib/utils";

export type ActivityMetric = "images_api" | "images_chat" | "dialogues_nurture" | "dialogues_real";

const METRIC_LABELS: Record<ActivityMetric, string> = {
  images_api: "api生图",
  images_chat: "对话生图",
  dialogues_nurture: "拟人对话",
  dialogues_real: "真实对话",
};

const METRIC_RGB: Record<ActivityMetric, [number, number, number]> = {
  images_api: [139, 92, 246],
  images_chat: [217, 70, 239],
  dialogues_nurture: [245, 158, 11],
  dialogues_real: [14, 165, 233],
};

const ALL_METRICS = Object.keys(METRIC_LABELS) as ActivityMetric[];

function normalizeMatrix(matrix?: number[][]) {
  return Array.from({ length: 7 }, (_, day) =>
    Array.from({ length: 12 }, (_, slot) => Math.max(0, Number(matrix?.[day]?.[slot] ?? 0))),
  );
}

function cellStyle(count: number, max: number, metric: ActivityMetric) {
  if (count <= 0 || max <= 0) return { backgroundColor: "#f5f5f4" };
  const ratio = Math.min(1, count / max);
  const alpha = 0.2 + ratio * 0.8;
  const [r, g, b] = METRIC_RGB[metric];
  return { backgroundColor: `rgba(${r}, ${g}, ${b}, ${alpha.toFixed(3)})` };
}

export const BindingActivityHeatmaps = memo(function BindingActivityHeatmaps({
  matrices,
  weekLabel = "",
  weekdayLabels = ["一", "二", "三", "四", "五", "六", "日"],
  dayLabels = [],
  timezoneLabel = "",
  compact = true,
  className,
}: {
  matrices: Partial<Record<ActivityMetric, number[][]>>;
  weekLabel?: string;
  weekdayLabels?: string[];
  dayLabels?: string[];
  timezoneLabel?: string;
  compact?: boolean;
  className?: string;
}) {
  const normalized = useMemo(() => {
    const items = ALL_METRICS.map((metric) => {
      const matrix = normalizeMatrix(matrices[metric]);
      const total = matrix.flat().reduce((sum, value) => sum + value, 0);
      return { metric, matrix, total };
    });
    return items.filter((item) => item.total > 0).length > 0 ? items.filter((i) => i.total > 0) : items;
  }, [matrices]);

  const labels = dayLabels.length === 7 ? dayLabels : Array.from({ length: 7 }, () => "");
  const cellSize = compact ? "size-3" : "size-3.5";

  return (
    <div className={cn("flex flex-col gap-1", className)}>
      <div className="grid grid-cols-2 gap-1.5 xl:grid-cols-4">
        {normalized.map((item) => {
          const max = Math.max(0, ...item.matrix.flat());
          return (
            <div key={item.metric} className="inline-flex min-w-0 flex-col gap-1 rounded-lg border border-[var(--neo-border)] bg-white/70 p-1.5">
              <div className="flex items-baseline justify-between gap-1">
                <div className="truncate text-[10px] font-semibold">{METRIC_LABELS[item.metric]}</div>
                <div className="text-[9px] font-medium text-[var(--neo-muted)]">Σ{item.total}</div>
              </div>
              <div className="grid grid-cols-7 gap-px">
                {item.matrix.map((row, day) =>
                  row.map((count, slot) => (
                    <div
                      key={`${day}-${slot}`}
                      title={`${weekLabel} ${weekdayLabels[day] || ""} ${SLOT_LABELS[slot]} ${count}`}
                      style={cellStyle(count, max, item.metric)}
                      className={cn("rounded-[2px] border border-stone-200/40 text-[7px] font-semibold leading-none", cellSize, count > 0 ? "text-stone-800" : "text-transparent")}
                    >
                      {count > 0 ? (count > 9 ? "9+" : count) : ""}
                    </div>
                  )),
                )}
              </div>
            </div>
          );
        })}
      </div>
      <div className="text-[9px] text-[var(--neo-muted)]">{weekLabel || "本周"}{timezoneLabel ? ` · ${timezoneLabel}` : ""}</div>
    </div>
  );
});
