"use client";

import { useEffect, useMemo, useState } from "react";
import { LoaderCircle } from "lucide-react";
import { cn } from "@/lib/utils";
import {
  grokAdminApi,
  grokAdminListAuditsUpTo,
  type GrokAuditEntry,
  type GrokAuditSummary,
} from "@/lib/grok-admin";

const SAMPLE_LIMIT = 100;

function fmtTime(iso: string | null | undefined): string {
  if (!iso) return "—";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  return d.toLocaleString("zh-CN", { hour12: false });
}

function latencyColor(ms: number): string {
  if (ms <= 0) return "text-[var(--neo-muted)]";
  if (ms < 1_000) return "text-emerald-600";
  if (ms < 5_000) return "text-amber-600";
  return "text-rose-600";
}

function fmtLatency(ms: number): string {
  if (ms <= 0) return "—";
  return ms >= 1000 ? `${(ms / 1000).toFixed(1)}s` : `${ms}ms`;
}

/** 活动流水面板：审计列表 + 成功率/平均延迟/按模型分布条形（数据走 /admin/request-audits）。 */
export function GrokActivityPanels({ token, reloadKey = 0 }: { token: string; reloadKey?: number }) {
  const [audits, setAudits] = useState<GrokAuditEntry[]>([]);
  const [summary, setSummary] = useState<GrokAuditSummary | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");

  useEffect(() => {
    let active = true;
    grokAdminListAuditsUpTo(token, SAMPLE_LIMIT)
      .then(async (items) => {
        const sum = await grokAdminApi.getAuditSummary(token);
        if (!active) return;
        setAudits(items);
        setSummary(sum);
      })
      .catch((err) => {
        if (active) setError(err instanceof Error ? err.message : String(err));
      })
      .finally(() => {
        if (active) setLoading(false);
      });
    return () => {
      active = false;
    };
  }, [token, reloadKey]);

  const distribution = useMemo(() => {
    const byModel = new Map<string, { total: number; ok: number; latencies: number[] }>();
    for (const item of audits) {
      const model = item.upstream_model || item.provider || "未标注";
      const entry = byModel.get(model) ?? { total: 0, ok: 0, latencies: [] as number[] };
      entry.total += 1;
      if (item.outcome === "success") entry.ok += 1;
      if (item.latency_ms > 0) entry.latencies.push(item.latency_ms);
      byModel.set(model, entry);
    }
    const rows = [...byModel.entries()].map(([model, v]) => ({
      model,
      total: v.total,
      ok: v.ok,
      avgLatency: v.latencies.length
        ? Math.round(v.latencies.reduce((a, b) => a + b, 0) / v.latencies.length)
        : 0,
    }));
    rows.sort((a, b) => b.total - a.total);
    return rows.slice(0, 6);
  }, [audits]);

  const totals = useMemo(() => {
    const ok = audits.filter((a) => a.outcome === "success").length;
    const err = audits.length - ok;
    const latencies = audits.filter((a) => a.latency_ms > 0).map((a) => a.latency_ms);
    const avg = latencies.length
      ? Math.round(latencies.reduce((a, b) => a + b, 0) / latencies.length)
      : 0;
    return { ok, err, avg };
  }, [audits]);

  if (loading && audits.length === 0) {
    return (
      <div className="flex min-h-[180px] items-center justify-center gap-2 rounded-2xl border border-[var(--neo-border)] bg-white/90 text-sm text-[var(--neo-muted)]">
        <LoaderCircle className="size-4 animate-spin" /> 加载活动流水…
      </div>
    );
  }

  if (audits.length === 0) {
    return (
      <div className="flex flex-col gap-2 rounded-2xl border border-[var(--neo-border)] bg-white/90 p-5 text-sm text-[var(--neo-muted)]">
        <div>暂无活动流水（/admin/request-audits 无记录）</div>
        <div className="text-xs opacity-70">
          数据源：request-audits 由 grok-audit 写入、grok-admin 读侧提供；接入真实 DB 后自动出现。
        </div>
        {error ? <div className="text-xs text-rose-600">{error}</div> : null}
      </div>
    );
  }

  const maxModelTotal = Math.max(1, ...distribution.map((d) => d.total));

  return (
    <div className="grid gap-4 lg:grid-cols-2">
      {/* 审计列表 */}
      <div className="flex flex-col gap-2 rounded-2xl border border-[var(--neo-border)] bg-white/90 p-4">
        <div className="flex flex-wrap items-baseline justify-between gap-2">
          <div className="text-sm font-medium text-[var(--neo-ink)]">活动流水 · 最近 {audits.length} 条</div>
          <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-[11px] text-[var(--neo-muted)]">
            <span>
              成功 <span className="font-semibold text-emerald-600">{totals.ok}</span>
            </span>
            <span>
              失败 <span className="font-semibold text-rose-600">{totals.err}</span>
            </span>
            <span>
              平均延迟 <span className={cn("font-semibold", latencyColor(totals.avg))}>{fmtLatency(totals.avg)}</span>
            </span>
            {summary ? (
              <span>
                24h 成功率{" "}
                <span className="font-semibold">
                  {(summary.success_rate_24h * 100).toFixed(0)}%
                </span>
              </span>
            ) : null}
          </div>
        </div>
        <ul className="flex max-h-72 flex-col divide-y divide-[var(--neo-border)]/60 overflow-y-auto text-xs">
          {audits.slice(0, 14).map((item) => (
            <li key={item.id} className="flex items-center justify-between gap-2 py-1.5">
              <div className="flex min-w-0 items-center gap-2">
                <span
                  className={cn(
                    "inline-flex size-1.5 shrink-0 rounded-full",
                    item.outcome === "success" ? "bg-emerald-500" : "bg-rose-500",
                  )}
                />
                <span className="truncate text-[var(--neo-ink)]">
                  {item.account_id != null ? `#${item.account_id}` : "—"}
                </span>
                <span className="truncate text-[var(--neo-muted)]">
                  {item.upstream_model || item.provider || "未标注"}
                </span>
                <span className={cn("shrink-0 font-medium", latencyColor(item.latency_ms))}>
                  {fmtLatency(item.latency_ms)}
                </span>
              </div>
              <span className="shrink-0 text-[10px] text-[var(--neo-muted)]">{fmtTime(item.created_at)}</span>
            </li>
          ))}
        </ul>
        <div className="text-[10px] text-[var(--neo-muted)]">
          数据源：GET /admin/request-audits（分页聚合 {SAMPLE_LIMIT} 条，后端无记录时为空态）
        </div>
      </div>

      {/* 按模型分布条形 */}
      <div className="flex flex-col gap-2 rounded-2xl border border-[var(--neo-border)] bg-white/90 p-4">
        <div className="text-sm font-medium text-[var(--neo-ink)]">按上游模型分布 · 请求量</div>
        <div className="flex flex-col gap-2">
          {distribution.map((row) => (
            <div key={row.model} className="flex items-center gap-2">
              <span className="w-32 shrink-0 truncate text-xs text-[var(--neo-ink)]" title={row.model}>
                {row.model}
              </span>
              <div className="relative h-4 flex-1 overflow-hidden rounded bg-[var(--neo-surface-muted)]">
                <div
                  className="absolute inset-y-0 left-0 bg-[var(--neo-primary)]/80"
                  style={{ width: `${((row.ok / maxModelTotal) * 100).toFixed(1)}%` }}
                />
                <div
                  className="absolute inset-y-0 bg-rose-400/80"
                  style={{
                    left: `${((row.ok / maxModelTotal) * 100).toFixed(1)}%`,
                    width: `${(((row.total - row.ok) / maxModelTotal) * 100).toFixed(1)}%`,
                  }}
                />
              </div>
              <span className="w-20 shrink-0 text-right text-[10px] text-[var(--neo-muted)]">
                {row.total} · {row.total ? Math.round((row.ok / row.total) * 100) : 0}%
              </span>
              <span className={cn("w-14 shrink-0 text-right text-[10px]", latencyColor(row.avgLatency))}>
                {fmtLatency(row.avgLatency)}
              </span>
            </div>
          ))}
        </div>
        <div className="text-[10px] text-[var(--neo-muted)]">
          <span className="inline-block size-2 rounded-sm bg-[var(--neo-primary)]/80" /> 成功
          <span className="ml-3 inline-block size-2 rounded-sm bg-rose-400/80" /> 失败
          {error ? <span className="ml-3 text-rose-600">{error}</span> : null}
        </div>
      </div>
    </div>
  );
}
