"use client";

import { CheckCircle2, CircleOff, LoaderCircle, RefreshCw } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";

import { ElevatedCard } from "@/components/admin/page-shell";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { accountsApi, type AccountRefreshAllStatus } from "@/lib/api";

const REFRESH_ALL_STATE_TEXT: Record<string, string> = {
  idle: "空闲",
  running: "运行中",
  paused: "资源保护暂停",
  stopping: "停止中",
  stopped: "已停止",
  completed: "已完成",
};

function formatRefreshAllState(state?: string) {
  const key = String(state || "idle");
  return REFRESH_ALL_STATE_TEXT[key] ?? key;
}

function formatRefreshAllOption(status: AccountRefreshAllStatus | null, key: string) {
  const value = status?.options?.[key];
  if (typeof value === "number" || typeof value === "string" || typeof value === "boolean") {
    return String(value);
  }
  return "-";
}

function formatRefreshAllResource(status: AccountRefreshAllStatus | null) {
  const resource = status?.resource ?? {};
  const parts: string[] = [];
  if (typeof resource.available_memory_mb === "number") {
    parts.push(`可用内存 ${resource.available_memory_mb}MB`);
  }
  if (typeof resource.memory_current_mb === "number" && typeof resource.memory_limit_mb === "number") {
    parts.push(`容器 ${resource.memory_current_mb}/${resource.memory_limit_mb}MB`);
  }
  if (typeof resource.load_1m === "number") {
    parts.push(`负载 ${resource.load_1m}`);
  }
  return parts.join(" · ");
}

type Props = {
  onCompleted?: () => void;
};

export function RefreshAllPanel({ onCompleted }: Props) {
  const [status, setStatus] = useState<AccountRefreshAllStatus | null>(null);
  const [concurrency, setConcurrency] = useState("4");
  const [batchSize, setBatchSize] = useState("25");
  const [delaySec, setDelaySec] = useState("0.2");
  const [starting, setStarting] = useState(false);
  const [stopping, setStopping] = useState(false);
  const lastStateRef = useRef("");

  const active = Boolean(
    status?.state === "running" || status?.state === "paused" || status?.state === "stopping",
  );

  const loadStatus = useCallback(async () => {
    try {
      const next = await accountsApi.refreshAllStatus();
      setStatus(next);
      return next;
    } catch {
      return null;
    }
  }, []);

  useEffect(() => {
    void loadStatus();
  }, [loadStatus]);

  useEffect(() => {
    if (status?.state === "stopping") {
      setStopping(true);
    }
    if (status?.state === "stopped" || status?.state === "completed" || status?.state === "idle") {
      setStopping(false);
    }
  }, [status?.state]);

  useEffect(() => {
    if (!active) return;
    const timer = window.setInterval(() => {
      void (async () => {
        const next = await loadStatus();
        if (!next) return;
        const previous = lastStateRef.current;
        lastStateRef.current = next.state;
        if ((next.state === "completed" || next.state === "stopped") && previous && previous !== next.state) {
          setStopping(false);
          onCompleted?.();
        }
      })();
    }, 2000);
    return () => window.clearInterval(timer);
  }, [active, loadStatus, onCompleted]);

  const handleStart = async () => {
    setStarting(true);
    lastStateRef.current = "";
    try {
      const next = await accountsApi.refreshAllStart({
        concurrency: Number(concurrency) || undefined,
        max_concurrency: Number(concurrency) || undefined,
        batch_size: Number(batchSize) || undefined,
        delay_between_accounts_sec: Number(delaySec) || undefined,
        stale_after_hours: 0,
        include_recent: true,
        resource_pause_enabled: false,
        delete_invalid: false,
        delete_after_failures: 1,
      });
      setStatus(next);
      if (next.state === "completed" && next.total === 0) {
        alert(`没有需要慢刷的账号，已跳过 ${next.skipped} 个近期刷新账号`);
      } else {
        alert(`慢速刷新已启动：队列 ${next.total} 个账号`);
      }
    } catch (err) {
      alert(err instanceof Error ? err.message : "启动慢速刷新失败");
    } finally {
      setStarting(false);
    }
  };

  const handleStop = async () => {
    setStopping(true);
    try {
      const next = await accountsApi.refreshAllStop();
      setStatus(next);
      alert("已请求停止慢速刷新，正在等待当前请求结束");
    } catch (err) {
      alert(err instanceof Error ? err.message : "停止慢速刷新失败");
      setStopping(false);
    }
  };

  return (
    <ElevatedCard className="mt-4 p-4">
      <div className="flex flex-wrap items-center gap-2">
        <Button
          size="sm"
          variant="toolbar"
          className="h-8 gap-1.5"
          disabled={starting || active}
          onClick={() => void handleStart()}
        >
          {starting || active ? <LoaderCircle className="size-3.5 animate-spin" /> : <RefreshCw className="size-3.5" />}
          全量慢刷额度
        </Button>
        <div className="flex items-center gap-1 rounded-xl border border-[var(--neo-border)] bg-white/80 px-2 py-1">
          <Input
            value={concurrency}
            onChange={(e) => setConcurrency(e.target.value)}
            className="h-7 w-14 border-0 bg-transparent px-1 text-xs shadow-none"
            placeholder="并发"
            disabled={active}
            title="慢刷并发"
          />
          <Input
            value={batchSize}
            onChange={(e) => setBatchSize(e.target.value)}
            className="h-7 w-14 border-0 bg-transparent px-1 text-xs shadow-none"
            placeholder="批量"
            disabled={active}
            title="批大小"
          />
          <Input
            value={delaySec}
            onChange={(e) => setDelaySec(e.target.value)}
            className="h-7 w-14 border-0 bg-transparent px-1 text-xs shadow-none"
            placeholder="间隔"
            disabled={active}
            title="账号间隔秒"
          />
        </div>
        {active ? (
          <Button
            size="sm"
            variant="outline"
            className="h-8 gap-1.5 text-rose-600"
            disabled={stopping}
            onClick={() => void handleStop()}
          >
            {stopping ? <LoaderCircle className="size-3.5 animate-spin" /> : <CircleOff className="size-3.5" />}
            {stopping || status?.state === "stopping" ? "停止中" : "停止慢刷"}
          </Button>
        ) : null}
      </div>

      {status && status.state !== "idle" ? (
        <div className="mt-3 border-t border-[var(--neo-border)] pt-3">
          <div className="flex flex-wrap items-center justify-between gap-2 text-sm">
            <div className="flex min-w-0 items-center gap-2 text-stone-700">
              {active ? (
                <LoaderCircle className="size-4 animate-spin text-amber-500" />
              ) : (
                <CheckCircle2 className="size-4 text-emerald-500" />
              )}
              <span className="font-medium">慢速刷新全部额度</span>
              <Badge variant={status.state === "paused" ? "warning" : status.state === "completed" ? "success" : "muted"}>
                {formatRefreshAllState(status.state)}
              </Badge>
            </div>
            <div className="text-[var(--neo-muted)]">
              {status.processed}/{status.total}
            </div>
          </div>
          <div className="mt-2 h-2 w-full overflow-hidden rounded-full bg-stone-100">
            <div
              className="h-full rounded-full bg-gradient-to-r from-emerald-400 to-blue-500 transition-all duration-300 ease-out"
              style={{
                width: `${status.total > 0 ? Math.min(100, (status.processed / status.total) * 100) : 100}%`,
              }}
            />
          </div>
          <div className="mt-2 flex flex-wrap gap-x-4 gap-y-1 text-xs text-[var(--neo-muted)]">
            <span>刷新成功 {status.refreshed}</span>
            <span>可调度 {status.available}</span>
            <span>新增可调度 {status.became_available}</span>
            <span>未知额度 {status.unknown_quota ?? 0}</span>
            <span>失败 {status.failed}</span>
            <span>跳过 {status.skipped}</span>
            <span>并发 {formatRefreshAllOption(status, "concurrency")}</span>
            <span>批量 {formatRefreshAllOption(status, "batch_size")}</span>
            <span>间隔 {formatRefreshAllOption(status, "delay_between_accounts_sec")}s</span>
            {formatRefreshAllResource(status) ? <span>{formatRefreshAllResource(status)}</span> : null}
          </div>
          {status.pause_reason ? <div className="mt-2 text-xs text-amber-700">{status.pause_reason}</div> : null}
        </div>
      ) : null}
    </ElevatedCard>
  );
}
