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
  const freshRemaining = q.remaining_fresh ?? 0;
  const freshTotal = q.total_fresh ?? 0;
  const freshAccounts = q.accounts_fresh ?? 0;
  const hasFresh = freshAccounts > 0 && freshTotal > 0;
  const ratio = !unlimited && hasFresh ? Math.min(1, freshRemaining / freshTotal) : hasFresh ? 1 : 0;
  const barColor =
    ratio > 0.3 ? "bg-[var(--neo-primary)]/80" : ratio > 0.05 ? "bg-amber-500/80" : "bg-rose-500/80";

  return (
    <div className="flex flex-col gap-0.5">
      <div className="flex items-center justify-between gap-2 text-[11px]">
        <span className="font-medium text-[var(--neo-ink)]">{labelQuotaMode(q.mode)}</span>
        <span className="tabular-nums text-[var(--neo-muted)]">
          {unlimited && !hasFresh
            ? "不限（陈旧）"
            : hasFresh
              ? `${fmtNum(freshRemaining)} / ${fmtNum(freshTotal)}`
              : "无新鲜窗口"}
        </span>
      </div>

      {hasFresh && !unlimited && (
        <div className="relative h-1.5 w-full overflow-hidden rounded-full bg-[var(--neo-surface-muted)]">
          <div
            className={`absolute inset-y-0 left-0 rounded-full transition-all ${barColor}`}
            style={{ width: `${(ratio * 100).toFixed(1)}%` }}
          />
        </div>
      )}

      <div className="flex flex-wrap gap-x-2 text-[10px] text-[var(--neo-muted)]">
        {hasFresh && <span>{freshAccounts} 个号 24h 内同步</span>}
        {q.stale > 0 && (
          <span className="text-amber-500">{q.stale} 个窗口超 24h</span>
        )}
        {q.exhausted > 0 && (
          <span className="text-rose-500">{q.exhausted} 个耗尽</span>
        )}
      </div>
    </div>
  );
}

/** 总额度面板：主数字用 24h 内同步的 fast 窗口。 */
function QuotaPanel({ quota }: { quota: GrokQuotaModeSummary[] | undefined }) {
  if (!quota || quota.length === 0) return null;
  const fast = quota.find((q) => q.mode === "fast");
  const remaining = fast?.remaining_fresh ?? 0;
  const total = fast?.total_fresh ?? 0;
  const accounts = fast?.accounts_fresh ?? 0;

  return (
    <div className="neo-card px-4 py-3">
      <div className="mb-2.5 flex flex-wrap items-end justify-between gap-2">
        <div>
          <div className="text-[11px] uppercase tracking-wide text-[var(--neo-muted)]">
            可用对话额度
          </div>
          <div className="mt-0.5 text-2xl font-semibold tabular-nums text-pink-500">
            {fmtNum(remaining)}
            <span className="ml-1 text-base font-medium text-[var(--neo-muted)]">
              / {fmtNum(total)}
            </span>
          </div>
          <div className="text-[10px] text-[var(--neo-muted)]">
            {accounts} 个启用号 · 仅统计 24h 内同步的 fast（/rest/rate-limits）
          </div>
        </div>
      </div>
      <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
        {quota.map((q) => (
          <QuotaModeBar key={q.mode} q={q} />
        ))}
      </div>
      <p className="mt-2 text-[10px] text-[var(--neo-muted)]">
        auto / imagine 窗口目前不会被刷新，数字为 ETL 冻结值时显示「无新鲜窗口」。
      </p>
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
