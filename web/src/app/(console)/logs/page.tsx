"use client";

import { useCallback, useEffect, useMemo, useState } from "react";
import { ChevronLeft, ChevronRight, LoaderCircle, RefreshCw, Search, Trash2, X } from "lucide-react";
import { DateRangeFilter } from "@/components/date-range-filter";
import { ElevatedCard, PageShell } from "@/components/admin/page-shell";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { logsApi, type SystemLog } from "@/lib/api";
import { fetchWithCache, invalidateCache } from "@/lib/api-cache";
import {
  buildCallLogPhases,
  dedupeCallLogs,
  formatDurationMs,
  formatTokensPerSec,
  getInlinePhases,
  type PhaseTiming,
} from "@/lib/image-log-phases";

const LogType = {
  Call: "call",
  Account: "account",
  LlmOps: "llm_ops",
} as const;

const typeLabels: Record<string, string> = {
  [LogType.Call]: "调用日志",
  [LogType.Account]: "账号管理日志",
  [LogType.LlmOps]: "LLM 操作日志",
};

function PhaseChip({ phase }: { phase: PhaseTiming }) {
  return (
    <span
      className="inline-flex rounded-md bg-[var(--neo-surface-muted)] px-1.5 py-0.5 text-[10px] leading-tight text-[var(--neo-muted)]"
      title={phase.hint ? `${phase.label}: ${phase.hint}` : phase.label}
    >
      {phase.label} {(phase.ms / 1000).toFixed(1)}s
    </span>
  );
}

function DurationCell({ item }: { item: SystemLog }) {
  const detail = item.detail;
  const phases = getInlinePhases(detail);
  const tokensPerSec = formatTokensPerSec(detail);
  return (
    <div className="min-w-[200px] space-y-1">
      <div className="font-medium text-[var(--neo-ink)]">{formatDurationMs(detail)}</div>
      {phases.length ? (
        <div className="flex flex-wrap gap-1">
          {phases.map((phase) => (
            <PhaseChip key={phase.key} phase={phase} />
          ))}
        </div>
      ) : null}
      {tokensPerSec ? <div className="text-[10px] text-[var(--neo-muted)]">{tokensPerSec}</div> : null}
    </div>
  );
}

function getLogImageUrls(item: SystemLog): string[] {
  const urls = item.detail?.urls;
  return Array.isArray(urls) ? urls.filter((u): u is string => typeof u === "string") : [];
}

function getStatus(item: SystemLog) {
  const status = item.detail?.status;
  if (status === "success") return "成功";
  if (status === "failed") return "失败";
  return "-";
}

function getDetailText(item: SystemLog, key: string) {
  const value = item.detail?.[key];
  return typeof value === "string" || typeof value === "number" ? String(value) : "-";
}

export default function LogsPage() {
  const [items, setItems] = useState<SystemLog[]>([]);
  const [type, setType] = useState<string>(LogType.Call);
  const [startDate, setStartDate] = useState("");
  const [endDate, setEndDate] = useState("");
  const [source, setSource] = useState("");
  const [outcome, setOutcome] = useState("");
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const [selectedIds, setSelectedIds] = useState<string[]>([]);
  const [deleting, setDeleting] = useState(false);
  const [page, setPage] = useState(1);
  const [pageSize, setPageSize] = useState(50);
  const [detailLog, setDetailLog] = useState<SystemLog | null>(null);

  const isCallLog = type === LogType.Call;
  const isLlmOps = type === LogType.LlmOps;

  const loadLogs = useCallback(
    async (options?: { force?: boolean; background?: boolean }) => {
      if (!options?.background) setLoading(true);
      setError("");
      const cacheKey = `logs:${type}:${startDate}:${endDate}:${source}:${outcome}`;
      try {
        const { data } = await fetchWithCache(
          cacheKey,
          () =>
            logsApi.list({
              type,
              start_date: startDate || undefined,
              end_date: endDate || undefined,
              source: source || undefined,
              outcome: outcome || undefined,
              limit: 500,
            }),
          30_000,
          { force: options?.force },
        );
        setItems(data.items);
        setSelectedIds([]);
        setPage(1);
      } catch (err) {
        setError(err instanceof Error ? err.message : "加载日志失败");
        setItems([]);
      } finally {
        setLoading(false);
      }
    },
    [type, startDate, endDate, source, outcome],
  );

  useEffect(() => {
    void loadLogs({ background: items.length > 0 });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [loadLogs]);

  const visibleItems = useMemo(() => (isCallLog ? dedupeCallLogs(items) : items), [items, isCallLog]);
  const pageCount = Math.max(1, Math.ceil(visibleItems.length / pageSize));
  const safePage = Math.min(page, pageCount);
  const currentRows = visibleItems.slice((safePage - 1) * pageSize, safePage * pageSize);
  const selectedSet = useMemo(() => new Set(selectedIds), [selectedIds]);
  const currentPageSelected = currentRows.length > 0 && currentRows.every((item) => selectedSet.has(item.id));
  const allSelected = visibleItems.length > 0 && visibleItems.every((item) => selectedSet.has(item.id));

  const toggleIds = (ids: string[], checked: boolean) => {
    setSelectedIds((prev) => {
      const next = new Set(prev);
      for (const id of ids) {
        if (checked) next.add(id);
        else next.delete(id);
      }
      return [...next];
    });
  };

  const onDeleteSelected = async () => {
    if (selectedIds.length === 0) return;
    setDeleting(true);
    try {
      await logsApi.delete(selectedIds);
      invalidateCache("logs:");
      await loadLogs({ force: true });
    } catch (err) {
      setError(err instanceof Error ? err.message : "删除失败");
    } finally {
      setDeleting(false);
    }
  };

  const clearFilters = () => {
    setStartDate("");
    setEndDate("");
    setSource("");
    setOutcome("");
  };

  const detailPhases = detailLog ? buildCallLogPhases(detailLog.detail) : [];

  return (
    <PageShell
      title="日志管理"
      actions={
        <div className="flex flex-wrap items-center gap-2">
          <select
            value={type}
            onChange={(e) => setType(e.target.value)}
            className="neo-input h-8 rounded-lg px-2 text-sm"
          >
            <option value={LogType.Call}>调用日志</option>
            <option value={LogType.Account}>账号管理日志</option>
            <option value={LogType.LlmOps}>LLM 操作日志</option>
          </select>
          {isLlmOps ? (
            <>
              <select
                value={source}
                onChange={(e) => setSource(e.target.value)}
                className="neo-input h-8 rounded-lg px-2 text-sm"
              >
                <option value="">全部 source</option>
                <option value="L0">L0</option>
                <option value="L1">L1</option>
                <option value="L2">L2</option>
              </select>
              <select
                value={outcome}
                onChange={(e) => setOutcome(e.target.value)}
                className="neo-input h-8 rounded-lg px-2 text-sm"
              >
                <option value="">全部 outcome</option>
                <option value="ok">ok</option>
                <option value="reject">reject</option>
                <option value="error">error</option>
              </select>
            </>
          ) : null}
          <DateRangeFilter startDate={startDate} endDate={endDate} onChange={(s, e) => { setStartDate(s); setEndDate(e); }} />
          <Button size="sm" variant="toolbar" className="h-8" onClick={clearFilters}>
            清除筛选
          </Button>
          <Button size="sm" className="h-8 gap-1" onClick={() => { invalidateCache("logs:"); void loadLogs({ force: true }); }} disabled={loading}>
            {loading ? <LoaderCircle className="size-3.5 animate-spin" /> : <Search className="size-3.5" />}
            查询
          </Button>
        </div>
      }
    >
      {error ? (
        <ElevatedCard className="mb-4 border-red-200 bg-red-50 p-3 text-sm text-red-700">{error}</ElevatedCard>
      ) : null}

      <ElevatedCard className="overflow-hidden">
        <div className="flex flex-wrap items-center justify-between gap-3 border-b border-[var(--neo-border)] px-4 py-3">
          <div className="flex flex-wrap items-center gap-3 text-sm text-[var(--neo-muted)]">
            <span>
              共 {visibleItems.length} 条
              {isCallLog && visibleItems.length !== items.length ? `（去重前 ${items.length}）` : ""}
            </span>
            <label className="flex items-center gap-2">
              <input
                type="checkbox"
                checked={currentPageSelected}
                onChange={(e) => toggleIds(currentRows.map((i) => i.id), e.target.checked)}
              />
              本页全选
            </label>
            <label className="flex items-center gap-2">
              <input
                type="checkbox"
                checked={allSelected}
                onChange={(e) => toggleIds(visibleItems.map((i) => i.id), e.target.checked)}
              />
              全选结果
            </label>
            {selectedIds.length > 0 ? <span>已选 {selectedIds.length} 条</span> : null}
            <select
              value={pageSize}
              onChange={(e) => {
                setPageSize(Number(e.target.value));
                setPage(1);
              }}
              className="neo-input h-7 rounded-lg px-2 text-xs"
            >
              <option value={10}>10 条/页</option>
              <option value={50}>50 条/页</option>
              <option value={200}>200 条/页</option>
            </select>
          </div>
          <div className="flex items-center gap-2">
            <Button size="sm" variant="toolbar" className="h-8" onClick={() => { invalidateCache("logs:"); void loadLogs({ force: true }); }} disabled={loading}>
              <RefreshCw className={`size-3.5 ${loading ? "animate-spin" : ""}`} />
            </Button>
            <Button
              size="sm"
              variant="toolbar"
              className="h-8 text-rose-600"
              disabled={selectedIds.length === 0 || deleting}
              onClick={() => void onDeleteSelected()}
            >
              <Trash2 className="size-3.5" />
              删除所选
            </Button>
          </div>
        </div>

        <div className="overflow-x-auto">
          <table className="w-full min-w-[900px] text-left text-sm">
            <thead className="neo-table-head">
              <tr>
                <th className="w-10 px-4 py-2.5" />
                <th className="px-4 py-2.5 font-medium">时间</th>
                <th className="px-4 py-2.5 font-medium">类型</th>
                {isCallLog ? <th className="px-4 py-2.5 font-medium">调用耗时</th> : null}
                {isCallLog ? <th className="px-4 py-2.5 font-medium">状态</th> : null}
                {isCallLog ? <th className="px-4 py-2.5 font-medium">图片</th> : null}
                {isLlmOps ? <th className="px-4 py-2.5 font-medium">source</th> : null}
                {isLlmOps ? <th className="px-4 py-2.5 font-medium">outcome</th> : null}
                <th className="px-4 py-2.5 font-medium">简述</th>
                <th className="w-28 px-4 py-2.5 font-medium">操作</th>
              </tr>
            </thead>
            <tbody>
              {currentRows.map((item) => (
                <tr key={item.id} className="border-t border-[var(--neo-border)] neo-row-hover">
                  <td className="px-4 py-3">
                    <input
                      type="checkbox"
                      checked={selectedSet.has(item.id)}
                      onChange={(e) => toggleIds([item.id], e.target.checked)}
                    />
                  </td>
                  <td className="whitespace-nowrap px-4 py-3 text-[var(--neo-muted)]">{item.time}</td>
                  <td className="px-4 py-3">
                    <Badge variant="muted">{typeLabels[item.type] || item.type}</Badge>
                  </td>
                  {isCallLog ? (
                    <td className="px-4 py-3">
                      <DurationCell item={item} />
                    </td>
                  ) : null}
                  {isCallLog ? (
                    <td className="px-4 py-3">
                      <Badge variant={item.detail?.status === "failed" ? "default" : "success"}>{getStatus(item)}</Badge>
                    </td>
                  ) : null}
                  {isCallLog ? (
                    <td className="px-4 py-3">
                      {getLogImageUrls(item).length ? (
                        <div className="flex gap-1">
                          {getLogImageUrls(item).slice(0, 2).map((url) => (
                            // eslint-disable-next-line @next/next/no-img-element
                            <img key={url} src={url} alt="" className="size-9 rounded-md border border-[var(--neo-border)] object-cover" />
                          ))}
                        </div>
                      ) : (
                        <span className="text-xs text-[var(--neo-muted)]">—</span>
                      )}
                    </td>
                  ) : null}
                  {isLlmOps ? <td className="px-4 py-3">{getDetailText(item, "source")}</td> : null}
                  {isLlmOps ? <td className="px-4 py-3">{getDetailText(item, "outcome")}</td> : null}
                  <td className="max-w-[360px] truncate px-4 py-3 text-[var(--neo-muted)]">{item.summary || "-"}</td>
                  <td className="px-4 py-3">
                    <Button size="sm" variant="toolbar" className="h-7 text-xs" onClick={() => setDetailLog(item)}>
                      详情
                    </Button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>

        <div className="flex items-center justify-end gap-2 border-t border-[var(--neo-border)] px-4 py-3 text-sm text-[var(--neo-muted)]">
          <span>
            第 {safePage} / {pageCount} 页
          </span>
          <Button size="sm" variant="toolbar" className="h-8 w-8 p-0" disabled={safePage <= 1} onClick={() => setPage((p) => Math.max(1, p - 1))}>
            <ChevronLeft className="size-4" />
          </Button>
          <Button
            size="sm"
            variant="toolbar"
            className="h-8 w-8 p-0"
            disabled={safePage >= pageCount}
            onClick={() => setPage((p) => Math.min(pageCount, p + 1))}
          >
            <ChevronRight className="size-4" />
          </Button>
        </div>

        {!loading && visibleItems.length === 0 ? (
          <div className="px-6 py-14 text-center text-sm text-[var(--neo-muted)]">没有找到日志</div>
        ) : null}
      </ElevatedCard>

      {detailLog ? (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-4" onClick={() => setDetailLog(null)}>
          <div
            className="neo-card max-h-[85vh] w-full max-w-2xl overflow-y-auto p-5"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="mb-4 flex items-start justify-between gap-3">
              <div>
                <h2 className="text-lg font-semibold text-[var(--neo-ink)]">日志详情</h2>
                <p className="mt-1 text-sm text-[var(--neo-muted)]">{detailLog.time}</p>
              </div>
              <button type="button" className="rounded-lg p-1 text-[var(--neo-muted)] hover:bg-[var(--neo-surface-muted)]" onClick={() => setDetailLog(null)}>
                <X className="size-5" />
              </button>
            </div>
            <p className="text-sm text-[var(--neo-ink)]">{detailLog.summary || "—"}</p>
            {isCallLog && detailPhases.length ? (
              <div className="mt-4 space-y-2">
                <h3 className="text-sm font-medium text-[var(--neo-ink)]">阶段耗时（端到端分解）</h3>
                {detailPhases.map((phase) => (
                  <div
                    key={phase.key}
                    className="flex items-center justify-between rounded-lg bg-[var(--neo-surface-muted)] px-3 py-2 text-sm"
                  >
                    <span className="text-[var(--neo-ink)]">
                      {phase.label}
                      {phase.derived ? <span className="ml-1 text-[10px] text-[var(--neo-muted)]">（推导）</span> : null}
                    </span>
                    <span className="font-mono font-medium">{(phase.ms / 1000).toFixed(2)} s</span>
                  </div>
                ))}
              </div>
            ) : null}
            <pre className="mt-4 max-h-48 overflow-auto rounded-lg bg-[var(--neo-surface-muted)] p-3 text-xs text-[var(--neo-muted)]">
              {JSON.stringify(detailLog.detail ?? {}, null, 2)}
            </pre>
          </div>
        </div>
      ) : null}
    </PageShell>
  );
}
