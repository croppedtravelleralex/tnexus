"use client";

import { LoaderCircle } from "lucide-react";
import { useEffect, useState } from "react";

import { grokAdminApi, type GrokAccountSummary, type GrokQuotaModeSummary } from "@/lib/grok-admin";
import { labelQuotaMode } from "@/lib/grok-labels";
import { QUOTA_UNLIMITED_THRESHOLD } from "@/components/grok/grok-quota-heatstrip";

type Props = {
  token: string;
  onError?: (message: string) => void;
  /** 自增后强制重新拉取（页面「刷新」按钮用） */
  reloadKey?: number;
};

const CARD_META: Array<{
  key: keyof Pick<
    GrokAccountSummary,
    "total" | "available" | "cooldown" | "reauth_required" | "disabled" | "probing" | "quota_exhausted"
  >;
  label: string;
  accent?: boolean;
}> = [
  { key: "total", label: "总数" },
  { key: "available", label: "可用", accent: true },
  { key: "cooldown", label: "冷却" },
  { key: "reauth_required", label: "需重登" },
  { key: "probing", label: "探针中" },
  { key: "quota_exhausted", label: "额度耗尽" },
  { key: "disabled", label: "禁用" },
];

function fmtNum(value: number): string {
  if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(1)}m`;
  if (value >= 1_000) return `${(value / 1_000).toFixed(1)}k`;
  return String(Math.round(value));
}

function QuotaModeBar({ q }: { q: GrokQuotaModeSummary }) {
  const unlimited = q.total >= QUOTA_UNLIMITED_THRESHOLD;
  const ratio = !unlimited && q.total > 0 ? Math.min(1, q.remaining / q.total) : 1;
  const barColor =
    ratio > 0.3 ? "bg-[var(--neo-primary)]/80" : ratio > 0.05 ? "bg-amber-500/80" : "bg-rose-500/80";

  return (
    <div className="flex flex-col gap-0.5">
      <div className="flex items-center justify-between gap-2 text-[11px]">
        <span className="font-medium text-[var(--neo-ink)]">{labelQuotaMode(q.mode)}</span>
        <span className="tabular-nums text-[var(--neo-muted)]">
          {unlimited ? "不限" : `${fmtNum(q.remaining)} / ${fmtNum(q.total)}`}
        </span>
      </div>

      {!unlimited && (
        <div className="relative h-1.5 w-full overflow-hidden rounded-full bg-[var(--neo-surface-muted)]">
          <div
            className={`absolute inset-y-0 left-0 rounded-full transition-all ${barColor}`}
            style={{ width: `${(ratio * 100).toFixed(1)}%` }}
          />
        </div>
      )}

      <div className="flex flex-wrap gap-x-2 text-[10px] text-[var(--neo-muted)]">
        {q.exhausted > 0 && (
          <span className="text-rose-500">{q.exhausted} 个耗尽</span>
        )}
        {q.stale > 0 && (
          <span className="text-amber-500">{q.stale} 个窗口超 24h 未刷新</span>
        )}
      </div>
    </div>
  );
}

/** 总额度面板（quota 字段可选；旧服务端不返回则不渲染） */
function QuotaPanel({ quota }: { quota: GrokQuotaModeSummary[] | undefined }) {
  if (!quota || quota.length === 0) return null;

  return (
    <div className="neo-card px-4 py-3">
      <div className="mb-2.5 text-[11px] uppercase tracking-wide text-[var(--neo-muted)]">总额度</div>
      <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
        {quota.map((q) => (
          <QuotaModeBar key={q.mode} q={q} />
        ))}
      </div>
    </div>
  );
}

/** 池规模统计卡片（GET /admin/accounts/summary）。 */
export function GrokSummaryCards({ token, onError, reloadKey = 0 }: Props) {
  const [summary, setSummary] = useState<GrokAccountSummary | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let cancelled = false;
    grokAdminApi
      .getSummary(token)
      .then((data) => {
        if (!cancelled) setSummary(data);
      })
      .catch((err) => {
        if (!cancelled) onError?.(err instanceof Error ? err.message : String(err));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [token, onError, reloadKey]);

  if (loading && !summary) {
    return (
      <div className="flex items-center gap-2 text-xs text-[var(--neo-muted)]">
        <LoaderCircle className="size-3.5 animate-spin" /> 统计加载中…
      </div>
    );
  }
  if (!summary) return null;

  return (
    <div className="flex flex-col gap-2">
      <div className="grid grid-cols-2 gap-2 sm:grid-cols-4 lg:grid-cols-7">
        {CARD_META.map(({ key, label, accent }) => (
          <div key={key} className="neo-card px-3 py-2.5">
            <div className="text-[11px] uppercase tracking-wide text-[var(--neo-muted)]">{label}</div>
            <div
              className={`mt-1 text-xl font-semibold tabular-nums ${
                accent ? "text-pink-500" : "text-[var(--neo-ink)]"
              }`}
            >
              {summary[key] ?? 0}
            </div>
          </div>
        ))}
      </div>
      <QuotaPanel quota={summary.quota} />
    </div>
  );
}
