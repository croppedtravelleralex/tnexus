"use client";

import { cn } from "@/lib/utils";
import type { GrokQuotaWindow } from "@/lib/grok-admin";
import { labelQuotaMode } from "@/lib/grok-labels";
import { isQuotaStale } from "@/lib/grok-quota";

export { isQuotaStale };

/** total >= 1B 视为哨兵"不限"值（imagine 模式专用）。 */
export const QUOTA_UNLIMITED_THRESHOLD = 1_000_000_000;

function fmtNum(value: number): string {
  if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(1)}m`;
  if (value >= 1_000) return `${(value / 1_000).toFixed(1)}k`;
  return String(Math.round(value));
}

function fmtSyncTime(value: string | null | undefined): string {
  if (!value) return "从未同步";
  const d = new Date(value);
  if (Number.isNaN(d.getTime())) return "同步时间无效";
  return `同步于 ${d.toLocaleString("zh-CN", { hour12: false })}`;
}

/** 额度使用热条：remaining/total 比例条；无窗口显示「未知」。
 *
 * - total >= 1B：渲染「不限」而非无意义的大数字
 * - source === "default"：渲染「未探测」（从未真实同步过）
 * - synced_at 缺失或超 24h：视觉标为陈旧（虚线边框 + 灰色文字）
 */
export function GrokQuotaHeatstrip({
  window,
  className,
}: {
  window?: GrokQuotaWindow | null;
  className?: string;
}) {
  if (!window) {
    return (
      <span className={cn("text-xs text-[var(--neo-muted)]", className)}>未知</span>
    );
  }

  const modeLabel = labelQuotaMode(window.mode);
  const syncLabel = fmtSyncTime(window.synced_at);

  if (window.source === "default") {
    return (
      <span
        className={cn("text-xs text-[var(--neo-muted)] italic", className)}
        title={`${modeLabel} · 未实际探测（source=default） · ${syncLabel}`}
      >
        未探测
      </span>
    );
  }

  const isUnlimited = window.total >= QUOTA_UNLIMITED_THRESHOLD;
  if (isUnlimited) {
    const stale = isQuotaStale(window);
    return (
      <span
        className={cn("text-xs", stale ? "text-[var(--neo-muted)]" : "text-emerald-600", className)}
        title={`${modeLabel} · 不限额 · ${syncLabel}`}
      >
        {stale ? (
          <span className="border-b border-dashed border-[var(--neo-muted)]">不限</span>
        ) : (
          "不限"
        )}
      </span>
    );
  }

  const remaining = Math.max(0, window.remaining);
  const total = Math.max(0, window.total);
  const ratio = total > 0 ? Math.min(1, remaining / total) : remaining > 0 ? 1 : 0;
  const stale = isQuotaStale(window);

  const barColor =
    ratio > 0.3 ? "bg-[var(--neo-primary)]/85" : ratio > 0.05 ? "bg-amber-500/90" : "bg-rose-500/90";

  const tooltipText = `${modeLabel} · 剩 ${fmtNum(remaining)} / 总 ${fmtNum(total)} · ${syncLabel}`;

  return (
    <div
      className={cn(
        "flex min-w-28 items-center gap-1.5",
        stale && "opacity-60",
        className,
      )}
      title={tooltipText}
    >
      <div
        className={cn(
          "relative h-2 w-16 overflow-hidden rounded-full bg-[var(--neo-surface-muted)]",
          stale && "border border-dashed border-[var(--neo-muted)]",
        )}
      >
        <div
          className={cn("absolute inset-y-0 left-0 rounded-full", barColor)}
          style={{ width: `${(ratio * 100).toFixed(1)}%` }}
        />
      </div>
      <span className="text-[10px] leading-none text-[var(--neo-muted)]">
        {fmtNum(remaining)}/{fmtNum(total)}
      </span>
    </div>
  );
}
