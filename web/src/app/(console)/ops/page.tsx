"use client";

import { useCallback, useEffect, useState } from "react";
import { Activity, LoaderCircle, RefreshCw, Server, Sprout } from "lucide-react";
import { ElevatedCard, PageShell } from "@/components/admin/page-shell";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { ChoiceButton, SegmentGroup } from "@/components/ui/choice-button";
import { Input } from "@/components/ui/input";
import { healthApi, opsApi } from "@/lib/api";

const TABS = [
  { id: "health", label: "健康概览" },
  { id: "pipeline", label: "生图流水线" },
  { id: "nurture", label: "养号" },
  { id: "risk", label: "监控埋点" },
] as const;

export default function OpsPage() {
  const [tab, setTab] = useState<(typeof TABS)[number]["id"]>("health");
  const [health, setHealth] = useState<Record<string, unknown> | null>(null);
  const [summary, setSummary] = useState<Record<string, unknown> | null>(null);
  const [pipeline, setPipeline] = useState<Record<string, unknown> | null>(null);
  const [nurture, setNurture] = useState<Record<string, unknown> | null>(null);
  const [risk, setRisk] = useState<Record<string, unknown> | null>(null);
  const [nurturePrompt, setNurturePrompt] = useState("");
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");

  const load = useCallback(async () => {
    setLoading(true);
    setError("");
    try {
      const [h, s, p, n, r] = await Promise.all([
        healthApi.ping(),
        opsApi.summary(),
        opsApi.pipelineSnapshot(),
        opsApi.nurtureStatus(),
        opsApi.riskMetrics(),
      ]);
      setHealth(h as Record<string, unknown>);
      setSummary(s);
      setPipeline(p);
      setNurture(n);
      setRisk(r);
    } catch (err) {
      setError(err instanceof Error ? err.message : "加载运维快照失败");
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const topology = (pipeline?.slot_topology || {}) as Record<string, number>;
  const phaseAvg = (pipeline?.phase_avg_ms || {}) as Record<string, number>;
  const nurtureQueue = (nurture?.queue || {}) as { depth?: number; oldest_age_sec?: number };

  const onEnqueueNurture = async () => {
    setLoading(true);
    setError("");
    try {
      await opsApi.nurtureEnqueue({ prompt: nurturePrompt.trim() || undefined, source: "tnexus_ops" });
      const n = await opsApi.nurtureStatus();
      setNurture(n);
    } catch (err) {
      setError(err instanceof Error ? err.message : "养号入队失败");
    } finally {
      setLoading(false);
    }
  };

  return (
    <PageShell
      title="运维"
      actions={
        <Button size="sm" variant="toolbar" className="h-8 gap-1.5" onClick={() => void load()} disabled={loading}>
          {loading ? <LoaderCircle className="size-3.5 animate-spin" /> : <RefreshCw className="size-3.5" />}
          刷新快照
        </Button>
      }
    >
      {error ? (
        <ElevatedCard className="mb-4 border-red-200 bg-red-50 p-3 text-sm text-red-700">{error}</ElevatedCard>
      ) : null}

      <SegmentGroup className="mb-4">
        {TABS.map((t) => (
          <ChoiceButton key={t.id} variant="segment" active={tab === t.id} onClick={() => setTab(t.id)}>
            {t.label}
          </ChoiceButton>
        ))}
      </SegmentGroup>

      {tab === "health" ? (
        <div className="grid gap-4 md:grid-cols-2">
          <ElevatedCard className="p-4">
            <div className="flex items-center gap-2 text-sm font-medium text-[var(--neo-ink)]">
              <Server className="size-4 text-[var(--neo-muted)]" /> TNexus API
            </div>
            <pre className="mt-3 max-h-48 overflow-auto rounded-lg border border-[var(--neo-border)] bg-[var(--neo-surface-muted)] p-3 text-xs text-[var(--neo-muted)]">
              {JSON.stringify(health ?? { status: "loading" }, null, 2)}
            </pre>
          </ElevatedCard>
          <ElevatedCard className="p-4">
            <div className="flex items-center gap-2 text-sm font-medium text-[var(--neo-ink)]">
              <Activity className="size-4 text-[var(--neo-muted)]" /> 任务与号池
            </div>
            <div className="mt-3 space-y-2 text-sm">
              <div className="flex justify-between">
                <span className="text-[var(--neo-muted)]">任务总数</span>
                <span className="font-medium">{String(summary?.jobs_total ?? "—")}</span>
              </div>
              <div className="flex justify-between">
                <span className="text-[var(--neo-muted)]">进行中</span>
                <Badge variant="success">{String(summary?.jobs_running ?? 0)}</Badge>
              </div>
              <div className="flex justify-between">
                <span className="text-[var(--neo-muted)]">已出图</span>
                <span>{String(summary?.results_total ?? "—")}</span>
              </div>
              <div className="flex justify-between">
                <span className="text-[var(--neo-muted)]">号池账户</span>
                <span>{String(summary?.accounts_total ?? "—")}</span>
              </div>
            </div>
          </ElevatedCard>
        </div>
      ) : null}

      {tab === "pipeline" ? (
        <ElevatedCard className="p-6">
          <p className="text-sm text-[var(--neo-muted)]">双槽流水线（pS / sS）</p>
          <div className="mt-4 grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
            <div className="rounded-lg border border-[var(--neo-border)] p-3 text-sm">
              <div className="text-[var(--neo-muted)]">pS 槽</div>
              <div className="mt-1 text-lg font-semibold">
                {topology.ps_inflight ?? 0}/{topology.ps_capacity ?? 0}
              </div>
              <div className="text-xs text-[var(--neo-muted)]">排队 {topology.ps_queued ?? 0}</div>
            </div>
            <div className="rounded-lg border border-[var(--neo-border)] p-3 text-sm">
              <div className="text-[var(--neo-muted)]">sS 槽</div>
              <div className="mt-1 text-lg font-semibold">
                {topology.ss_inflight ?? 0}/{topology.ss_capacity ?? 0}
              </div>
              <div className="text-xs text-[var(--neo-muted)]">排队 {topology.ss_queued ?? 0}</div>
            </div>
            <div className="rounded-lg border border-[var(--neo-border)] p-3 text-sm">
              <div className="text-[var(--neo-muted)]">全局 in-flight</div>
              <div className="mt-1 text-lg font-semibold">{topology.pipeline_in_flight ?? 0}</div>
            </div>
            <div className="rounded-lg border border-[var(--neo-border)] p-3 text-sm">
              <div className="text-[var(--neo-muted)]">数据源</div>
              <div className="mt-1 text-sm">{String(pipeline?.source ?? "—")}</div>
            </div>
          </div>
          <div className="mt-6 flex h-32 items-end gap-2">
            {[
              { key: "ps_ms", label: "pS" },
              { key: "sse_stream_ms", label: "开票+SSE" },
              { key: "download_ms", label: "落盘" },
            ].map((phase) => {
              const ms = Number(phaseAvg[phase.key] ?? 0);
              const h = Math.max(12, Math.min(120, ms / 40));
              return (
                <div key={phase.key} className="flex flex-1 flex-col items-center gap-1">
                  <div
                    className="w-full rounded-t-md bg-gradient-to-t from-[var(--neo-primary-deep)] to-[var(--neo-primary)]"
                    style={{ height: `${h}px` }}
                  />
                  <span className="text-[10px] text-[var(--neo-muted)]">{phase.label}</span>
                  <span className="text-[10px] font-medium">{(ms / 1000).toFixed(1)}s</span>
                </div>
              );
            })}
          </div>
          <p className="mt-4 text-xs text-[var(--neo-muted)]">
            阶段均值来自任务 phase_timings_ms（样本 {String(phaseAvg.samples ?? 0)}）。
          </p>
        </ElevatedCard>
      ) : null}

      {tab === "nurture" ? (
        <ElevatedCard className="p-6 space-y-4">
          <div className="flex items-center gap-2 text-sm font-medium">
            <Sprout className="size-4" /> 养号队列
          </div>
          <div className="grid gap-2 text-sm sm:grid-cols-2">
            <div>已开启: {String(nurture?.enabled ?? false)}</div>
            <div>运行中: {String(nurture?.running ?? false)}</div>
            <div>队列深度: {String(nurtureQueue.depth ?? 0)}</div>
            <div>最老任务: {String(nurtureQueue.oldest_age_sec ?? 0)} 秒</div>
          </div>
          <div className="flex flex-wrap gap-2">
            <Input
              placeholder="养号提示词（可选）"
              value={nurturePrompt}
              onChange={(e) => setNurturePrompt(e.target.value)}
              className="max-w-md"
            />
            <Button size="sm" onClick={() => void onEnqueueNurture()} disabled={loading}>
              入队一条
            </Button>
          </div>
          {nurture?.last_error ? (
            <p className="text-xs text-red-600">{String(nurture.last_error)}</p>
          ) : null}
        </ElevatedCard>
      ) : null}

      {tab === "risk" ? (
        <ElevatedCard className="p-6">
          <p className="text-sm font-medium text-[var(--neo-ink)]">监控埋点（24h）</p>
          <div className="mt-4 grid gap-3 sm:grid-cols-3 text-sm">
            <div className="rounded-lg border border-[var(--neo-border)] p-3">
              <div className="text-[var(--neo-muted)]">成功任务</div>
              <div className="mt-1 text-xl font-semibold">{String(risk?.jobs_done_24h ?? "—")}</div>
            </div>
            <div className="rounded-lg border border-[var(--neo-border)] p-3">
              <div className="text-[var(--neo-muted)]">失败任务</div>
              <div className="mt-1 text-xl font-semibold">{String(risk?.jobs_failed_24h ?? "—")}</div>
            </div>
            <div className="rounded-lg border border-[var(--neo-border)] p-3">
              <div className="text-[var(--neo-muted)]">失败率</div>
              <div className="mt-1 text-xl font-semibold">
                {typeof risk?.failure_rate_24h === "number"
                  ? `${(risk.failure_rate_24h * 100).toFixed(1)}%`
                  : "—"}
              </div>
            </div>
          </div>
          <pre className="mt-4 max-h-48 overflow-auto rounded-lg border border-[var(--neo-border)] bg-[var(--neo-surface-muted)] p-3 text-xs">
            {JSON.stringify(risk ?? {}, null, 2)}
          </pre>
        </ElevatedCard>
      ) : null}
    </PageShell>
  );
}
