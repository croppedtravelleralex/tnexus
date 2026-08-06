"use client";

import { LoaderCircle, RefreshCw } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { PageShell } from "@/components/admin/page-shell";
import { GrokTabs } from "@/components/grok/grok-tabs";
import { GrokTokenGate } from "@/components/grok/grok-token-gate";
import { Button } from "@/components/ui/button";
import {
  grokAdminApi,
  type GrokDashboardView,
  type GrokTimeseriesPoint,
  type GrokTopAccount,
} from "@/lib/grok-admin";

function fmtTime(value: string | null): string {
  if (!value) return "—";
  const d = new Date(value);
  if (Number.isNaN(d.getTime())) return value;
  return d.toLocaleString("zh-CN", { hour12: false });
}

function fmtRate(value: number | null | undefined): string {
  if (value == null || Number.isNaN(value)) return "—";
  return `${(value * 100).toFixed(1)}%`;
}

function DashboardCards({ view }: { view: GrokDashboardView | null }) {
  if (!view) {
    return (
      <div className="neo-card px-3 py-6 text-center text-sm text-[var(--neo-muted)]">
        暂无仪表盘数据（grok-admin 未部署 dashboard 数据源）
      </div>
    );
  }
  const cards: Array<{ label: string; value: string; accent?: boolean }> = [
    { label: "账号总数", value: String(view.total_accounts ?? 0) },
    { label: "可用", value: String(view.available_accounts ?? 0), accent: true },
    { label: "冷却", value: String(view.cooldown_accounts ?? 0) },
    { label: "需重登", value: String(view.reauth_accounts ?? 0) },
    { label: "额度耗尽", value: String(view.quota_exhausted_accounts ?? 0) },
    { label: "24h 请求", value: String(view.requests_24h ?? 0) },
    { label: "24h 成功率", value: fmtRate(view.success_rate_24h) },
    { label: "模型路由", value: String(view.model_routes ?? 0) },
    { label: "活跃密钥", value: String(view.active_client_keys ?? 0) },
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

function TimeseriesChart({ points }: { points: GrokTimeseriesPoint[] }) {
  if (!points || points.length === 0) {
    return (
      <div className="neo-card px-3 py-6 text-center text-sm text-[var(--neo-muted)]">
        暂无时间序列数据（/admin/analytics/timeseries 返回空）
      </div>
    );
  }
  const max = Math.max(1, ...points.map((p) => p.requests ?? 0));
  return (
    <div className="neo-card overflow-x-auto p-4">
      <h3 className="mb-3 text-sm font-semibold text-[var(--neo-ink)]">每日请求量（近 {points.length} 天）</h3>
      <table className="w-full min-w-[560px] border-collapse text-left text-sm">
        <thead>
          <tr className="border-b border-[var(--neo-border)] text-[11px] uppercase tracking-wide text-[var(--neo-muted)]">
            <th className="px-2 py-1.5 font-medium">日期</th>
            <th className="px-2 py-1.5 font-medium">请求量</th>
            <th className="px-2 py-1.5 font-medium">成功 / 失败</th>
            <th className="px-2 py-1.5 font-medium">P50 延迟</th>
          </tr>
        </thead>
        <tbody>
          {points.map((point) => {
            const width = Math.max(2, Math.round(((point.requests ?? 0) / max) * 100));
            return (
              <tr key={point.date} className="border-b border-[var(--neo-border)] last:border-0">
                <td className="whitespace-nowrap px-2 py-2 text-xs text-[var(--neo-muted)]">{point.date}</td>
                <td className="px-2 py-2">
                  <div className="flex items-center gap-2">
                    <div className="h-2 min-w-[2px] rounded-full bg-pink-400" style={{ width: `${width}%` }} />
                    <span className="tabular-nums text-xs text-[var(--neo-ink)]">{point.requests ?? 0}</span>
                  </div>
                </td>
                <td className="px-2 py-2 tabular-nums text-xs">
                  <span className="text-emerald-600">{point.succeeded ?? 0}</span>
                  <span className="text-[var(--neo-muted)]"> / </span>
                  <span className="text-rose-500">{point.failed ?? 0}</span>
                </td>
                <td className="px-2 py-2 tabular-nums text-xs text-[var(--neo-muted)]">
                  {point.latency_p50_ms != null ? `${point.latency_p50_ms}ms` : "—"}
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}

function TopAccountsTable({ items }: { items: GrokTopAccount[] }) {
  if (!items || items.length === 0) {
    return (
      <div className="neo-card px-3 py-6 text-center text-sm text-[var(--neo-muted)]">
        暂无 Top 账号（/admin/analytics/top-accounts 返回空）
      </div>
    );
  }
  return (
    <div className="neo-card overflow-x-auto p-4">
      <h3 className="mb-3 text-sm font-semibold text-[var(--neo-ink)]">Top 账号（按请求量）</h3>
      <table className="w-full border-collapse text-left text-sm">
        <thead>
          <tr className="border-b border-[var(--neo-border)] text-[11px] uppercase tracking-wide text-[var(--neo-muted)]">
            <th className="px-2 py-1.5 font-medium">账号</th>
            <th className="px-2 py-1.5 font-medium">请求量</th>
            <th className="px-2 py-1.5 font-medium">失败</th>
            <th className="px-2 py-1.5 font-medium">失败率</th>
          </tr>
        </thead>
        <tbody>
          {items.map((account) => (
            <tr key={account.account_id} className="border-b border-[var(--neo-border)] last:border-0">
              <td className="px-2 py-2">
                <span className="font-medium text-[var(--neo-ink)]">{account.name || "—"}</span>
                <span className="ml-1.5 text-xs text-[var(--neo-muted)]">#{account.account_id}</span>
              </td>
              <td className="px-2 py-2 tabular-nums">{account.requests}</td>
              <td className="px-2 py-2 tabular-nums text-rose-500">{account.failed}</td>
              <td className="px-2 py-2 tabular-nums text-[var(--neo-muted)]">{fmtRate(account.failure_rate)}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function DashboardContent({ token }: { token: string }) {
  const [view, setView] = useState<GrokDashboardView | null>(null);
  const [points, setPoints] = useState<GrokTimeseriesPoint[]>([]);
  const [topAccounts, setTopAccounts] = useState<GrokTopAccount[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");

  const load = useCallback(async (currentToken: string) => {
    setLoading(true);
    setError("");
    try {
      const [dashboard, timeseries, top] = await Promise.all([
        grokAdminApi.getDashboard(currentToken),
        grokAdminApi.getTimeseries(currentToken, 7).catch(() => []),
        grokAdminApi.getTopAccounts(currentToken, 10).catch(() => []),
      ]);
      setView(dashboard);
      setPoints(timeseries);
      setTopAccounts(top);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
      setView(null);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    const timer = setTimeout(() => void load(token), 0);
    return () => clearTimeout(timer);
  }, [token, load]);

  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-center justify-between text-xs text-[var(--neo-muted)]">
        <span>最近请求：{view?.last_request_at ? fmtTime(view.last_request_at) : "—"}</span>
        <Button variant="outline" size="sm" onClick={() => void load(token)} disabled={loading}>
          {loading ? <LoaderCircle className="size-4 animate-spin" /> : <RefreshCw className="size-4" />}
          刷新
        </Button>
      </div>
      {error ? <p className="text-sm text-rose-600">{error}</p> : null}
      {loading && !view ? (
        <div className="flex items-center justify-center gap-2 py-16 text-sm text-[var(--neo-muted)]">
          <LoaderCircle className="size-4 animate-spin" /> 加载中…
        </div>
      ) : (
        <>
          <DashboardCards view={view} />
          <TimeseriesChart points={points} />
          <TopAccountsTable items={topAccounts} />
        </>
      )}
    </div>
  );
}

export default function GrokDashboardPage() {
  return (
    <PageShell title="Grok 总览" subtitle="聚合面板（账号 / 请求 / 成功率 / Top 账号）" badge="G4-P2">
      <GrokTabs />
      <GrokTokenGate>{(token) => <DashboardContent token={token} />}</GrokTokenGate>
    </PageShell>
  );
}
