import type { SystemLog } from "@/lib/api";

export type PhaseTiming = {
  key: string;
  label: string;
  hint?: string;
  ms: number;
  derived?: boolean;
};

const phaseMeta: Record<string, { label: string; hint?: string }> = {
  task_queue_ms: { label: "任务排队", hint: "HTTP 提交 → worker 开始执行" },
  admit_queue_ms: { label: "准入排队", hint: "等 pipeline 全局 in_flight 名额" },
  upload_queue_ms: { label: "上传排队", hint: "等参考图上传并发槽" },
  ps_queue_ms: { label: "pS 槽排队", hint: "等 prompt 增强槽" },
  ss_queue_ms: { label: "sS 槽排队", hint: "等 sS pool FIFO 槽位" },
  account_queue_ms: { label: "取号", hint: "等可用账号 / binding 并发" },
  upload_ms: { label: "参考图上传", hint: "参考图实际上传耗时" },
  ps_ms: { label: "pS 增强", hint: "prompt 增强阶段" },
  sse_stream_ms: { label: "开票+SSE", hint: "取号后 → SSE resolve" },
  poll_resolve_ms: { label: "轮询收图", hint: "SSE 结束后 poll 直到出图" },
  download_ms: { label: "下载落盘", hint: "从 CDN 拉取并写入存储" },
  download_queue_ms: { label: "下载排队", hint: "等下载并发槽" },
  ss_ms: { label: "sS 槽占用", hint: "持有 sS 槽总时长" },
  wall_clock_ms: { label: "Pipeline 墙钟", hint: "pipeline begin → finish" },
};

export const callLogPhaseOrder = [
  "task_queue_ms",
  "admit_queue_ms",
  "upload_queue_ms",
  "ps_queue_ms",
  "account_queue_ms",
  "ss_queue_ms",
  "upload_ms",
  "ps_ms",
  "sse_stream_ms",
  "poll_resolve_ms",
  "download_queue_ms",
  "download_ms",
  "wall_clock_ms",
] as const;

function readMs(detail: Record<string, unknown> | undefined, key: string): number {
  if (!detail) return 0;
  const phases = detail.phase_timings_ms;
  if (phases && typeof phases === "object" && !Array.isArray(phases)) {
    const raw = (phases as Record<string, unknown>)[key];
    const n = typeof raw === "number" ? raw : Number(raw);
    if (Number.isFinite(n) && n > 0) return n;
  }
  const flat = detail[`phase_${key}`];
  const flatN = typeof flat === "number" ? flat : Number(flat);
  if (Number.isFinite(flatN) && flatN > 0) return flatN;
  if (key === "task_queue_ms") {
    const tq = detail.task_queue_ms;
    const tqN = typeof tq === "number" ? tq : Number(tq);
    if (Number.isFinite(tqN) && tqN > 0) return tqN;
  }
  return 0;
}

export function buildCallLogPhases(detail: Record<string, unknown> | undefined): PhaseTiming[] {
  if (!detail) return [];

  const accountQueue = readMs(detail, "account_queue_ms");
  const sseStream = readMs(detail, "sse_stream_ms");
  const ssMs = readMs(detail, "ss_ms");
  const pollDerived = Math.max(0, ssMs - accountQueue - sseStream);

  const values: Record<string, number> = {};
  for (const key of callLogPhaseOrder) {
    if (key === "poll_resolve_ms") {
      if (pollDerived > 0) values[key] = pollDerived;
      continue;
    }
    const ms = readMs(detail, key);
    if (ms > 0) values[key] = ms;
  }

  const result: PhaseTiming[] = [];
  for (const key of callLogPhaseOrder) {
    const ms = values[key];
    if (!ms || ms <= 0) continue;
    const meta = phaseMeta[key] || { label: key.replace(/_ms$/, "") };
    result.push({
      key,
      label: meta.label,
      hint: meta.hint,
      ms,
      derived: key === "poll_resolve_ms",
    });
  }
  return result;
}

export function getInlinePhases(detail: Record<string, unknown> | undefined): PhaseTiming[] {
  const all = buildCallLogPhases(detail);
  if (!all.length) return [];
  const prefer = [
    "task_queue_ms",
    "account_queue_ms",
    "ss_queue_ms",
    "sse_stream_ms",
    "poll_resolve_ms",
    "download_ms",
  ];
  const picked = prefer
    .map((key) => all.find((p) => p.key === key))
    .filter((p): p is PhaseTiming => Boolean(p));
  return picked.length ? picked : all.slice(0, 6);
}

export function formatDurationMs(detail: Record<string, unknown> | undefined): string {
  const wallFromPhase = readMs(detail, "wall_clock_ms");
  const ms =
    (typeof detail?.total_wall_ms === "number" && detail.total_wall_ms > 0
      ? detail.total_wall_ms
      : undefined) ??
    (wallFromPhase > 0 ? wallFromPhase : undefined) ??
    (typeof detail?.worker_duration_ms === "number" && detail.worker_duration_ms > 0
      ? detail.worker_duration_ms
      : undefined) ??
    (typeof detail?.duration_ms === "number" && detail.duration_ms > 0 ? detail.duration_ms : undefined);
  return typeof ms === "number" ? `${(ms / 1000).toFixed(2)} s` : "-";
}

export function formatTokensPerSec(detail: Record<string, unknown> | undefined): string | null {
  const tps = detail?.tokens_per_sec;
  if (typeof tps === "number" && Number.isFinite(tps) && tps > 0) {
    return `${tps.toFixed(1)} t/s`;
  }
  return null;
}

export function dedupeCallLogs(items: SystemLog[]): SystemLog[] {
  const completedTaskIds = new Set<string>();
  for (const item of items) {
    const taskId = item.detail?.task_id;
    const summary = String(item.summary || "");
    const isComplete = summary.includes("调用完成") || summary.includes("调用失败");
    if (typeof taskId === "string" && taskId && isComplete) {
      completedTaskIds.add(taskId);
    }
  }
  return items.filter((item) => {
    const summary = String(item.summary || "");
    if (summary.includes("阶段耗时")) {
      const taskId = item.detail?.task_id;
      if (typeof taskId === "string" && taskId && completedTaskIds.has(taskId)) {
        return false;
      }
    }
    return true;
  });
}
