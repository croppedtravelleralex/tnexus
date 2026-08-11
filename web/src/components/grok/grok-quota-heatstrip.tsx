"use client";

import { cn } from "@/lib/utils";
import type { GrokQuotaWindow } from "@/lib/grok-admin";
import { labelQuotaMode } from "@/lib/grok-labels";

function fmtNum(value: number): string {
  if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(1)}m`;
  if (value >= 1_000) return `${(value / 1_000).toFixed(1)}k`;
  return String(Math.round(value));
}

/** 额度使用热条：remaining/total 比例条；无窗口显示「未知」。
 *
 * 颜色：比例充足 = 主紫，濒临耗尽 = 红/琥珀（与号池 gptimage 版 account-usage-heatstrip 风格一致）。
 */
export function GrokQuotaHeatstrip({
  window,
  className,
}: {
  window?: GrokQuotaWindow | null;
  className?: string;
}) {
  if (!window || window.total <= 0) {
    return (
      <span className={cn("text-xs text-[var(--neo-muted)]", className)}>未知</span>
    );
  }
  const remaining = Math.max(0, window.remaining);
  const ratio = Math.min(1, remaining / window.total);
  const color =
    ratio > 0.3 ? "bg-[var(--neo-primary)]/85" : ratio > 0.05 ? "bg-amber-500/90" : "bg-rose-500/90";
  return (
    <div className={cn("flex min-w-28 items-center gap-1.5", className)} title={`${labelQuotaMode(window.mode)} · 剩 ${fmtNum(remaining)} / 总 ${fmtNum(window.total)}`}>
      <div className="relative h-2 w-16 overflow-hidden rounded-full bg-[var(--neo-surface-muted)]">
        <div className={cn("absolute inset-y-0 left-0 rounded-full", color)} style={{ width: `${(ratio * 100).toFixed(1)}%` }} />
      </div>
      <span className="text-[10px] leading-none text-[var(--neo-muted)]">
        {fmtNum(remaining)}/{fmtNum(window.total)}
      </span>
    </div>
  );
}
