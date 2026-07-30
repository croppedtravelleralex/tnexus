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
    if (allSelected) {
      setSelected(new Set());
    } else {
      setSelected(new Set(items.map((i) => i.id)));
    }
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
    <div className="panel-card flex h-full flex-col border-r border-zinc-200">
      <div className="panel-header flex items-center justify-between border-b border-zinc-100 px-4 py-3">
        <div className="flex items-center gap-2">
          <span className="text-sm font-semibold text-zinc-900">生成记录</span>
          <span className="rounded-md bg-zinc-100 px-1.5 py-0.5 text-xs text-zinc-500">{items.length}</span>
        </div>
      </div>

      <div className="flex items-center gap-2 border-b border-zinc-100 px-3 py-2">
        <Button variant="outline" size="sm" className="h-8 gap-1 text-xs" onClick={onNew}>
          <Plus className="h-3.5 w-3.5" />
          新建
        </Button>
        <Button
          variant="outline"
          size="sm"
          className="h-8 gap-1 text-xs"
          onClick={toggleAll}
          disabled={items.length === 0}
        >
          <CheckSquare className="h-3.5 w-3.5" />
          全选
        </Button>
        <Button
          variant="outline"
          size="sm"
          className="h-8 gap-1 text-xs"
          onClick={() => void onDelete()}
          disabled={selected.size === 0 || deleting}
        >
          <Trash2 className="h-3.5 w-3.5" />
          删除
        </Button>
      </div>

      <div className="panel-body scrollbar-hide flex-1 space-y-2 overflow-y-auto p-2">
        {!apiOnline ? (
          <div className="rounded-lg border border-amber-200 bg-amber-50 px-3 py-3 text-xs leading-relaxed text-amber-900">
            后端未连接：请在 WSL 中启动{" "}
            <code className="rounded bg-amber-100 px-1">tnexus-api</code> 与{" "}
            <code className="rounded bg-amber-100 px-1">tnexus-worker</code>
            （或运行 <code className="rounded bg-amber-100 px-1">scripts/local-dev.sh</code>）
          </div>
        ) : loading && items.length === 0 ? (
          <p className="px-2 py-4 text-center text-sm text-zinc-400">加载中…</p>
        ) : items.length === 0 ? (
          <p className="px-2 py-4 text-center text-sm text-zinc-400">暂无生成记录，点击「新建」开始创作</p>
        ) : (
          items.map((job) => {
            const isDone = job.status === "done";
            const isFailed = job.status === "failed";
            const isRunning = !isDone && !isFailed;
            const successCount = isDone ? job.result_count : 0;
            const failCount = isFailed ? 1 : 0;
            const imageCount = isDone ? job.result_count : 0;

            return (
              <div
                key={job.id}
                className={cn(
                  "group relative rounded-xl border bg-white p-3 transition-colors",
                  activeId === job.id ? "border-zinc-400 ring-1 ring-zinc-200" : "border-zinc-200 hover:border-zinc-300",
                )}
              >
                <div className="flex gap-2">
                  <input
                    type="checkbox"
                    checked={selected.has(job.id)}
                    onChange={() => toggleOne(job.id)}
                    className="mt-1 h-4 w-4 shrink-0 rounded border-zinc-300"
                    onClick={(e) => e.stopPropagation()}
                  />
                  <button
                    type="button"
                    className="min-w-0 flex-1 text-left"
                    onClick={() => onSelect(job)}
                  >
                    <div className="flex items-start justify-between gap-2">
                      <p className="line-clamp-2 text-sm font-medium text-zinc-800">{job.input_prompt}</p>
                      <div className="flex shrink-0 gap-1">
                        {isDone && successCount > 0 && (
                          <span className="rounded-md bg-sky-50 px-1.5 py-0.5 text-[11px] font-medium text-sky-600">
                            成功 {successCount}
                          </span>
                        )}
                        {isFailed && (
                          <span className="rounded-md bg-rose-50 px-1.5 py-0.5 text-[11px] font-medium text-rose-500">
                            失败 {failCount}
                          </span>
                        )}
                        {isRunning && (
                          <span className="rounded-md bg-zinc-100 px-1.5 py-0.5 text-[11px] font-medium text-zinc-500">
                            进行中
                          </span>
                        )}
                      </div>
                    </div>

                    <div className="mt-2 flex items-end justify-between gap-2">
                      <div className="flex items-center gap-2">
                        <div className="h-12 w-12 shrink-0 overflow-hidden rounded-md border border-zinc-200 bg-zinc-50">
                          {job.thumb_url ? (
                            // eslint-disable-next-line @next/next/no-img-element
                            <img src={job.thumb_url} alt="" className="h-full w-full object-cover" />
                          ) : (
                            <div className="flex h-full items-center justify-center text-[10px] text-zinc-300">
                              {isRunning ? "…" : "无图"}
                            </div>
                          )}
                        </div>
                        <div className="text-xs text-zinc-500">
                          {imageCount > 0 && <p>{imageCount} 张</p>}
                          {!isRunning && (
                            <p className="font-medium text-emerald-600">
                              {formatDuration(
                                new Date(job.updated_at).getTime() - new Date(job.created_at).getTime(),
                              )}
                            </p>
                          )}
                          {isRunning && <p className="text-zinc-400">进行中…</p>}
                        </div>
                      </div>
                      <span className="rounded-full bg-zinc-100 px-2 py-0.5 text-[10px] text-zinc-500">
                        {formatTimestamp(job.created_at)}
                      </span>
                    </div>
                  </button>
                </div>
              </div>
            );
          })
        )}
      </div>
    </div>
  );
}
