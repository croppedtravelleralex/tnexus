"use client";

import { LoaderCircle, RefreshCw } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { ElevatedCard, PageShell } from "@/components/admin/page-shell";
import { GrokTabs } from "@/components/grok/grok-tabs";
import { GrokTokenGate } from "@/components/grok/grok-token-gate";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  grokAdminApi,
  type GrokAuditEntry,
  type GrokAuditSummary,
} from "@/lib/grok-admin";

const PAGE_SIZE = 50;
/** 客户端分页上限（后端无过滤参数，基础过滤在前端做）。 */
const FETCH_LIMIT = 200;

function fmtTime(value: string): string {
  const d = new Date(value);
  if (Number.isNaN(d.getTime())) return value;
  return d.toLocaleString("zh-CN", { hour12: false });
}

function providerLabel(provider: string | null) {
  if (!provider) return "—";
  return (
    {
      grok_build: "Build",
      grok_web: "Web",
      grok_console: "Console",
    }[provider] ?? provider
  );
}

function SummaryCards({ summary }: { summary: GrokAuditSummary | null }) {
  if (!summary) return null;
  const rate = summary.success_rate_24h ?? 0;
  const cards: Array<{ label: string; value: string; accent?: boolean }> = [
    { label: "总数", value: String(summary.total ?? 0) },
    { label: "24h 请求", value: String(summary.requests_24h ?? 0) },
    { label: "24h 成功", value: String(summary.succeeded_24h ?? 0) },
    { label: "24h 失败", value: String(summary.failed_24h ?? 0) },
    {
      label: "24h 成功率",
      value: `${(rate * 100).toFixed(1)}%`,
      accent: rate >= 0.95,
    },
  ];
  return (
    <div className="grid grid-cols-2 gap-2 sm:grid-cols-3 lg:grid-cols-5">
      {cards.map((card) => (
        <div key={card.label} className="neo-card px-3 py-2.5">
          <div className="text-[11px] uppercase tracking-wide text-[var(--neo-muted)]">{card.label}</div>
          <div
            className={`mt-1 text-xl font-semibold tabular-nums ${
              card.accent ? "text-pink-500" : "text-[var(--neo-ink)]"
            }`}
          >
            {card.value}
          </div>
        </div>
      ))}
    </div>
  );
}

function AuditsTable({ items }: { items: GrokAuditEntry[] }) {
  if (items.length === 0) {
    return (
      <div className="flex flex-col items-center justify-center gap-2 py-16 text-sm text-[var(--neo-muted)]">
        <span>暂无审计记录</span>
        <span className="text-xs opacity-70">（grok-admin 返回空列表或未部署审计数据）</span>
      </div>
    );
  }
  return (
    <div className="neo-card overflow-x-auto">
      <table className="w-full min-w-[880px] border-collapse text-left text-sm">
        <thead>
          <tr className="border-b border-[var(--neo-border)] text-[11px] uppercase tracking-wide text-[var(--neo-muted)]">
            <th className="px-3 py-2 font-medium">时间</th>
            <th className="px-3 py-2 font-medium">账号</th>
            <th className="px-3 py-2 font-medium">Provider</th>
            <th className="px-3 py-2 font-medium">模型</th>
            <th className="px-3 py-2 font-medium">状态</th>
            <th className="px-3 py-2 font-medium">结果</th>
            <th className="px-3 py-2 font-medium">延迟</th>
          </tr>
        </thead>
        <tbody>
          {items.map((entry) => (
            <tr
              key={entry.id}
              className="border-b border-[var(--neo-border)] last:border-0 hover:bg-[var(--neo-surface-muted)]"
            >
              <td className="whitespace-nowrap px-3 py-2 text-xs text-[var(--neo-muted)]">
                {fmtTime(entry.created_at)}
              </td>
              <td className="px-3 py-2 tabular-nums">{entry.account_id ?? "—"}</td>
              <td className="px-3 py-2">{providerLabel(entry.provider)}</td>
              <td className="max-w-[200px] truncate px-3 py-2" title={entry.upstream_model ?? ""}>
                {entry.upstream_model || "—"}
              </td>
              <td className="px-3 py-2 tabular-nums">{entry.status}</td>
              <td className="px-3 py-2">
                <Badge variant={entry.outcome === "success" ? "success" : "danger"}>
                  {entry.outcome === "success" ? "成功" : "失败"}
                </Badge>
              </td>
              <td className="px-3 py-2 tabular-nums text-[var(--neo-muted)]">
                {entry.latency_ms != null ? `${entry.latency_ms}ms` : "—"}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function AuditsContent({ token }: { token: string }) {
  const [entries, setEntries] = useState<GrokAuditEntry[]>([]);
  const [summary, setSummary] = useState<GrokAuditSummary | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  // 基础过滤：成功 / 失败 / 全部（客户端过滤；后端无过滤参数）。
  const [outcomeFilter, setOutcomeFilter] = useState<"all" | "success" | "error">("all");

  const load = useCallback(async (currentToken: string) => {
    setLoading(true);
    setError("");
    try {
      const [page, summaryData] = await Promise.all([
        grokAdminApi.listAudits(currentToken, { page: 1, pageSize: PAGE_SIZE }),
        grokAdminApi.getAuditSummary(currentToken),
      ]);
      const items = [...(page.items ?? [])];
      const total = page.total ?? items.length;
      // 拉取后续页直到达到 FETCH_LIMIT（供前端过滤用）。
      let pageNum = 2;
      while (items.length < Math.min(FETCH_LIMIT, total) && pageNum <= 5) {
        const next = await grokAdminApi.listAudits(currentToken, { page: pageNum, pageSize: PAGE_SIZE });
        items.push(...(next.items ?? []));
        if ((next.items ?? []).length === 0) break;
        pageNum += 1;
      }
      setEntries(items.slice(0, FETCH_LIMIT));
      setSummary(summaryData);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
      setEntries([]);
      setSummary(null);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    const timer = setTimeout(() => void load(token), 0);
    return () => clearTimeout(timer);
  }, [token, load]);

  const filtered =
    outcomeFilter === "all"
      ? entries
      : entries.filter((e) => e.outcome === outcomeFilter);

  return (
    <div className="flex flex-col gap-3">
            <SummaryCards summary={summary} />
            <div className="flex flex-wrap items-center justify-between gap-2 text-xs text-[var(--neo-muted)]">
              <div className="flex items-center gap-1 rounded-lg border border-[var(--neo-border)] bg-[var(--neo-surface)] p-1">
                {(
                  [
                    { value: "all", label: "全部" },
                    { value: "success", label: "成功" },
                    { value: "error", label: "失败" },
                  ] as const
                ).map((option) => (
                  <button
                    key={option.value}
                    type="button"
                    className={`rounded-md px-2.5 py-1 text-xs font-medium transition ${
                      outcomeFilter === option.value
                        ? "bg-white text-[var(--neo-ink)] ring-1 ring-[var(--neo-border)]"
                        : "text-[var(--neo-muted)] hover:text-[var(--neo-ink)]"
                    }`}
                    onClick={() => setOutcomeFilter(option.value)}
                  >
                    {option.label}
                  </button>
                ))}
              </div>
              <Button variant="outline" size="sm" onClick={() => void load(token)} disabled={loading}>
                {loading ? <LoaderCircle className="size-4 animate-spin" /> : <RefreshCw className="size-4" />}
                刷新
              </Button>
            </div>
            {error ? <p className="text-sm text-rose-600">{error}</p> : null}
            {loading && entries.length === 0 ? (
              <div className="flex items-center justify-center gap-2 py-16 text-sm text-[var(--neo-muted)]">
                <LoaderCircle className="size-4 animate-spin" /> 加载中…
              </div>
            ) : (
              <AuditsTable items={filtered} />
            )}
      <div className="text-[10px] text-[var(--neo-muted)]">
        前端过滤展示最多 {FETCH_LIMIT} 条（分页拉取；后端过滤参数 TODO）
      </div>
    </div>
  );
}

export default function GrokAuditsPage() {
  return (
    <PageShell title="Grok 审计" subtitle="请求审计流水（分页 + 24h 统计 + 结果过滤）" badge="G4-P2">
      <GrokTabs />
      <GrokTokenGate>{(token) => <AuditsContent token={token} />}</GrokTokenGate>
    </PageShell>
  );
}
