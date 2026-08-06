"use client";

import { useEffect, useMemo, useState } from "react";
import { LoaderCircle } from "lucide-react";
import { cn } from "@/lib/utils";
import {
  grokAdminListAuditsUpTo,
  type GrokAuditEntry,
} from "@/lib/grok-admin";

const HEATMAP_AUDIT_LIMIT = 400;
const HEATMAP_DAYS = 7;
const MAX_ACCOUNT_ROWS = 15;

function dayIndex(date: Date, today: Date): number {
  // 近 7 天：today 为最后一列（index 6），越早越靠左。
  const diff = Math.round((today.getTime() - date.getTime()) / 86_400_000);
  return HEATMAP_DAYS - 1 - Math.max(0, Math.min(HEATMAP_DAYS - 1, diff));
}

function cellStyle(count: number, max: number) {
  if (count <= 0 || max <= 0) return { backgroundColor: "#f5f5f4" };
  const ratio = Math.min(1, count / max);
  const alpha = 0.15 + ratio * 0.85;
  return { backgroundColor: `rgba(168, 85, 247, ${alpha.toFixed(3)})` };
}

/** 账号活跃热力图：账号 × 近 7 天 请求量（对照 gptimage BindingActivityHeatmaps 渲染）。
 *
 * 数据源：/admin/request-audits 分页聚合后客户端按 account_id×created_at 计数；
 * 后端无按账号时间序列端点（数据源 TODO：/admin/analytics/accounts-timeseries），
 * 条目不足 7 天时显示已有数据 + 空态说明。
 */
export function GrokAccountHeatmap({ token }: { token: string }) {
  const [audits, setAudits] = useState<GrokAuditEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");

  useEffect(() => {
    let active = true;
    grokAdminListAuditsUpTo(token, HEATMAP_AUDIT_LIMIT)
      .then((items) => {
        if (active) setAudits(items);
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
  }, [token]);

  const { rows, labels, max, total } = useMemo(() => {
    const today = new Date();
    today.setHours(0, 0, 0, 0);
    const counts = new Map<number, number[]>();
    const accountName = new Map<number, string>();
    let total = 0;
    for (const item of audits) {
      if (item.account_id == null) continue;
      const date = new Date(item.created_at);
      if (Number.isNaN(date.getTime())) continue;
      const idx = dayIndex(date, today);
      const row = counts.get(item.account_id) ?? Array.from({ length: HEATMAP_DAYS }, () => 0);
      row[idx] += 1;
      counts.set(item.account_id, row);
      accountName.set(item.account_id, `#${item.account_id}`);
      total += 1;
    }
    const sorted = [...counts.entries()].sort(
      (a, b) => b[1].reduce((s, v) => s + v, 0) - a[1].reduce((s, v) => s + v, 0),
    );
    const rows = sorted.slice(0, MAX_ACCOUNT_ROWS).map(([accountId, values]) => ({
      accountId,
      label: accountName.get(accountId) ?? `#${accountId}`,
      values,
    }));
    const labels = Array.from({ length: HEATMAP_DAYS }, (_, i) => {
      const d = new Date(today);
      d.setDate(d.getDate() - (HEATMAP_DAYS - 1 - i));
      return `${d.getMonth() + 1}/${d.getDate()}`;
    });
    const max = Math.max(1, ...rows.flatMap((r) => r.values));
    return { rows, labels, max, total };
  }, [audits]);

  if (loading && audits.length === 0) {
    return (
      <div className="flex min-h-[150px] items-center justify-center gap-2 rounded-2xl border border-[var(--neo-border)] bg-white/90 text-sm text-[var(--neo-muted)]">
        <LoaderCircle className="size-4 animate-spin" /> 加载活跃度…
      </div>
    );
  }

  if (rows.length === 0) {
    return (
      <div className="flex flex-col gap-2 rounded-2xl border border-[var(--neo-border)] bg-white/90 p-5 text-sm text-[var(--neo-muted)]">
        <div>暂无账号活跃数据（近 7 天 request-audits 无记录或缺失 account_id）</div>
        <div className="text-xs opacity-70">
          数据源：客户端按 /admin/request-audits 聚合；
          后端接入真实 DB 后自动出现，亦可由 /admin/analytics/accounts-timeseries 端点替代（TODO）。
        </div>
        {error ? <div className="text-xs text-rose-600">{error}</div> : null}
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-2 rounded-2xl border border-[var(--neo-border)] bg-white/90 p-4">
      <div className="flex flex-wrap items-baseline justify-between gap-2">
        <div className="text-sm font-medium text-[var(--neo-ink)]">账号活跃热力图 · 近 {HEATMAP_DAYS} 天</div>
        <div className="text-[11px] text-[var(--neo-muted)]">
          Σ{total} 请求 · 客户端聚合 /admin/request-audits
        </div>
      </div>
      <div className="overflow-x-auto">
        <table className="border-separate border-spacing-0.5">
          <thead>
            <tr>
              <th className="w-16 pr-2 text-left text-[10px] font-medium text-[var(--neo-muted)]">账号</th>
              {labels.map((label, i) => (
                <th key={label} className={cn("text-center text-[9px] font-medium", i === HEATMAP_DAYS - 1 ? "text-[var(--neo-primary-deep)]" : "text-[var(--neo-muted)]")}>
                  {label}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {rows.map((row) => (
              <tr key={row.accountId}>
                <td className="max-w-24 truncate pr-2 text-[10px] text-[var(--neo-ink)]" title={row.label}>
                  {row.label}
                </td>
                {row.values.map((count, i) => (
                  <td key={i}>
                    <div
                      title={`${row.label} ${labels[i]} · ${count} 请求`}
                      style={cellStyle(count, max)}
                      className={cn(
                        "flex size-4 items-center justify-center rounded-[2px] border border-stone-200/40 text-[8px] font-semibold leading-none",
                        count > 0 ? "text-white/90" : "text-transparent",
                      )}
                    >
                      {count > 0 ? (count > 9 ? "9+" : count) : ""}
                    </div>
                  </td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <div className="flex items-center gap-2 text-[10px] text-[var(--neo-muted)]">
        <span>低</span>
        <div className="flex h-2 w-24 overflow-hidden rounded-full">
          <div style={{ backgroundColor: "#f5f5f4" }} className="flex-1" />
          <div style={{ backgroundColor: "rgba(168, 85, 247, 0.25)" }} className="flex-1" />
          <div style={{ backgroundColor: "rgba(168, 85, 247, 0.55)" }} className="flex-1" />
          <div style={{ backgroundColor: "rgba(168, 85, 247, 0.9)" }} className="flex-1" />
        </div>
        <span>高</span>
        {rows.length === MAX_ACCOUNT_ROWS ? <span>· 仅显示请求量前 {MAX_ACCOUNT_ROWS} 个账号</span> : null}
      </div>
    </div>
  );
}
