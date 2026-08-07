"use client";

import { LoaderCircle } from "lucide-react";
import { useEffect, useState } from "react";

import { grokAdminApi, type GrokAccountSummary } from "@/lib/grok-admin";

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
  );
}