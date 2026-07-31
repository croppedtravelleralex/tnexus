"use client";

import { CheckSquare, Plus, Trash2 } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import { jobsApi, type JobListItem } from "@/lib/api";
import { useAuth } from "@/lib/auth";
import { formatDuration } from "@/lib/format-duration";
import { cn } from "@/lib/utils";

type Props = {
  activeId: string | null;
  onSelect: (job: JobListItem) => void;
  onNew: () => void;
  refreshKey: number;
};

function formatTimestamp(iso: string): string {
  return new Date(iso).toLocaleString("zh-CN", {
    year: "numeric",
    month: "numeric",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false,
  });
}

export function GenerationRecordsPanel({ activeId, onSelect, onNew, refreshKey }: Props) {
  const { apiOnline } = useAuth();
  const [items, setItems] = useState<JobListItem[]>([]);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [loading, setLoading] = useState(false);
  const [deleting, setDeleting] = useState(false);

  const load = useCallback(async () => {
    if (!apiOnline) return;
    setLoading(true);
    try {
      const list = await jobsApi.listSummaries();
      setItems(list);
    } catch {
      setItems([]);
    } finally {
      setLoading(false);
    }
  }, [apiOnline]);

  useEffect(() => {
    void load();
  }, [load, refreshKey]);

  const allSelected = items.length > 0 && selected.size === items.length;

  const toggleAll = () => {
    if (allSelected) setSelected(new Set());
    else setSelected(new Set(items.map((i) => i.id)));
  };

  const toggleOne = (id: string) => {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  const onDelete = async () => {
    if (selected.size === 0) return;
    setDeleting(true);
    try {
      await jobsApi.deleteMany([...selected]);
      setSelected(new Set());
      await load();
    } finally {
      setDeleting(false);
    }
  };

  return (
    <div className="flex h-full min-h-0 flex-col border-r border-[var(--neo-border)] bg-[var(--neo-surface)]">
      <div className="flex shrink-0 items-center justify-between border-b border-[var(--neo-border)] px-3 py-2.5">
        <div className="flex items-center gap-2">
          <span className="text-sm font-semibold text-[var(--neo-ink)]">生成记录</span>
          <span className="rounded-full bg-[var(--neo-surface-muted)] px-1.5 py-0.5 text-[11px] text-[var(--neo-muted)]">
            {items.length}
          </span>
        </div>
      </div>

      <div className="flex shrink-0 items-center gap-1 border-b border-[var(--neo-border)] px-2 py-1.5">
        <Button variant="toolbar" size="sm" className="h-7 gap-1 text-xs" onClick={onNew}>
          <Plus className="h-3.5 w-3.5" />
          新建
        </Button>
        <Button variant="toolbar" size="sm" className="h-7 gap-1 text-xs" onClick={toggleAll} disabled={items.length === 0}>
          <CheckSquare className="h-3.5 w-3.5" />
          全选
        </Button>
        <Button
          variant="toolbar"
          size="sm"
          className="h-7 gap-1 text-xs text-rose-600 hover:text-rose-700"
          onClick={() => void onDelete()}
          disabled={selected.size === 0 || deleting}
        >
          <Trash2 className="h-3.5 w-3.5" />
          删除
        </Button>
      </div>

      <div className="scrollbar-hide flex-1 space-y-1 overflow-y-auto p-2">
        {!apiOnline ? (
          <div className="rounded-lg bg-amber-50 px-3 py-3 text-xs leading-relaxed text-amber-900">
            后端未连接：请启动 <code className="rounded bg-amber-100 px-1">tnexus-api</code>
          </div>
        ) : loading && items.length === 0 ? (
          <p className="px-2 py-4 text-center text-sm text-[var(--neo-muted)]">加载中…</p>
        ) : items.length === 0 ? (
          <p className="px-2 py-4 text-center text-sm text-[var(--neo-muted)]">暂无生成记录，点击「新建」开始创作</p>
        ) : (
          items.map((job) => {
            const isDone = job.status === "done";
            const isFailed = job.status === "failed";
            const isRunning = !isDone && !isFailed;
            const active = activeId === job.id;
            const durationMs = new Date(job.updated_at).getTime() - new Date(job.created_at).getTime();

            return (
              <div
                key={job.id}
                className={cn(
                  "flex gap-2 rounded-lg px-2 py-2 transition-colors",
                  active ? "bg-[var(--neo-surface-muted)] ring-1 ring-[var(--neo-primary)]/35" : "hover:bg-[var(--neo-surface-muted)]/70",
                )}
              >
                <input
                  type="checkbox"
                  checked={selected.has(job.id)}
                  onChange={() => toggleOne(job.id)}
                  className="mt-1 h-4 w-4 shrink-0 rounded border-[var(--neo-border)]"
                  onClick={(e) => e.stopPropagation()}
                />
                <button type="button" className="min-w-0 flex-1 text-left" onClick={() => onSelect(job)}>
                  <div className="flex items-start justify-between gap-2">
                    <p className="line-clamp-2 text-sm font-medium text-[var(--neo-ink)]">{job.input_prompt}</p>
                    <div className="flex shrink-0 gap-1">
                      {isDone && job.result_count > 0 ? (
                        <span className="rounded-md bg-sky-50 px-1.5 py-0.5 text-[11px] font-medium text-sky-600">
                          成功 {job.result_count}
                        </span>
                      ) : null}
                      {isFailed ? (
                        <span className="rounded-md bg-rose-50 px-1.5 py-0.5 text-[11px] font-medium text-rose-500">
                          失败
                        </span>
                      ) : null}
                      {isRunning ? (
                        <span className="rounded-md bg-[var(--neo-surface-muted)] px-1.5 py-0.5 text-[11px] font-medium text-[var(--neo-muted)]">
                          进行中
                        </span>
                      ) : null}
                    </div>
                  </div>

                  <div className="mt-2 flex items-end justify-between gap-2">
                    <div className="flex items-center gap-2">
                      <div className="h-11 w-11 shrink-0 overflow-hidden rounded-lg bg-[var(--neo-surface-muted)]">
                        {job.thumb_url ? (
                          // eslint-disable-next-line @next/next/no-img-element
                          <img src={job.thumb_url} alt="" className="h-full w-full object-cover" />
                        ) : (
                          <div className="flex h-full items-center justify-center text-[10px] text-[var(--neo-muted)]">
                            {isRunning ? "…" : "无图"}
                          </div>
                        )}
                      </div>
                      <div className="text-xs text-[var(--neo-muted)]">
                        {job.result_count > 0 ? <p>{job.result_count} 张</p> : null}
                        {!isRunning && durationMs > 0 ? (
                          <p className="font-medium text-emerald-600">{formatDuration(durationMs)}</p>
                        ) : null}
                        {isRunning ? <p>进行中…</p> : null}
                      </div>
                    </div>
                    <time className="shrink-0 text-[10px] text-[var(--neo-muted)]">{formatTimestamp(job.created_at)}</time>
                  </div>
                </button>
              </div>
            );
          })
        )}
      </div>
    </div>
  );
}
