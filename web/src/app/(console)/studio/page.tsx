"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { GenerationRecordsPanel } from "@/components/studio/generation-records-panel";
import { GenConfigPanel } from "@/components/studio/gen-config-panel";
import { OcrPanel } from "@/components/studio/ocr-panel";
import { OutputPanel, type OutputSlot } from "@/components/studio/output-panel";
import { ResizableStudioLayout, DEFAULT_COLUMN_WIDTHS } from "@/components/studio/resizable-layout";
import { conversationsApi, jobsApi, apiAssetUrl, type FactorPoint, type JobDetail, type JobListItem, type JobResult } from "@/lib/api";
import { getJobDetailCached } from "@/lib/job-detail-cache";
import { isClientCacheReady, saveImageToClientCache } from "@/lib/client-image-cache";
import { useAuth } from "@/lib/auth";
import {
  conversationTitle,
  EMPTY_CONVERSATION_STATE,
  type ConversationState,
} from "@/lib/conversations";
import { isChatConversationState } from "@/lib/chat-conversations";
import { DEFAULT_GEN_CONFIG, snappedGenConfig, type GenConfig } from "@/lib/gen-config";
import { saveColumnRatios } from "@/lib/studio-layout";
import type { TextModelId } from "@/lib/models";
import { Loader2, ScanText } from "lucide-react";
import { Button } from "@/components/ui/button";

const STAGE_LABELS: Record<string, string> = {
  queued: "排队中",
  directing: "构思中",
  generating: "绘图中",
  uploading: "保存中",
  done: "完成",
  failed: "失败",
};

function formatJobError(msg: string): string {
  if (msg.includes("empty prompt from director")) {
    return "构思模型未返回有效英文提示词，请重试或更换构思模型";
  }
  if (msg.includes("invalid director json") || msg.includes("no json in director")) {
    return "构思模型返回格式异常，请重试";
  }
  if (msg.includes("无法连接服务器")) return msg;
  return msg;
}

export default function StudioPage() {
  const { user } = useAuth();

  const [conversationId, setConversationId] = useState<string | null>(null);
  const [activeJobId, setActiveJobId] = useState<string | null>(null);
  const [prompt, setPrompt] = useState("");
  const [mode, setMode] = useState<"director" | "casting">("director");
  const [workflow, setWorkflow] = useState<"full_agent" | "keyword_ps">("full_agent");
  const [enhanceEnabled, setEnhanceEnabled] = useState(false);
  const [activeStyleHint, setActiveStyleHint] = useState("");
  const [queueHint, setQueueHint] = useState("");
  const [textModel, setTextModel] = useState<TextModelId>("gpt");
  const [castingModels, setCastingModels] = useState<TextModelId[]>(["gpt", "grok"]);
  const [actorImageCounts, setActorImageCounts] = useState<Record<TextModelId, number>>(
    EMPTY_CONVERSATION_STATE.actorImageCounts,
  );
  const [imageEngine, setImageEngine] = useState<"chatgpt" | "grok">("chatgpt");
  const [directorFactors, setDirectorFactors] = useState<FactorPoint>({ x: 0.5, y: 0.5 });
  const [renderFactors, setRenderFactors] = useState<FactorPoint>({ x: 0.5, y: 0.5 });
  const [genConfig, setGenConfig] = useState<GenConfig>(DEFAULT_GEN_CONFIG);
  const [activeAspect, setActiveAspect] = useState("1:1");
  const [activeStylePreset, setActiveStylePreset] = useState("");
  const [columnWidths, setColumnWidths] = useState<[number, number, number] | null>(null);
  const [busy, setBusy] = useState(false);
  const [progress, setProgress] = useState(0);
  const [stage, setStage] = useState("");
  const [result, setResult] = useState<JobDetail | null>(null);
  const [completedSlots, setCompletedSlots] = useState<OutputSlot[]>([]);
  const [totalSlotCount, setTotalSlotCount] = useState(0);
  const [jobStatus, setJobStatus] = useState<"idle" | "running" | "done" | "failed">("idle");
  const [startedAt, setStartedAt] = useState(0);
  const [elapsedMs, setElapsedMs] = useState(0);
  const [refreshKey, setRefreshKey] = useState(0);
  const [ocrOpen, setOcrOpen] = useState(false);
  const displaySlots = useMemo(() => {
    const pending = Math.max(totalSlotCount - completedSlots.length, 0);
    const pendingTiles: OutputSlot[] = Array.from({ length: pending }, (_, i) => ({
      id: `pending-${completedSlots.length + i}`,
      status: "pending" as const,
    }));
    return [...completedSlots, ...pendingTiles];
  }, [completedSlots, totalSlotCount]);

  const mergePartialResults = useCallback((results: JobResult[]) => {
    setCompletedSlots((prev) => {
      const seen = new Set(prev.map((s) => s.id));
      const next = [...prev];
      for (const img of results) {
        if (seen.has(img.id)) continue;
        seen.add(img.id);
        next.push({
          id: img.id,
          status: "success" as const,
          image: img,
          generationMs: img.generation_ms ?? undefined,
        });
      }
      return next;
    });
  }, []);
  const [error, setError] = useState("");

  const saveTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const jobDetailCacheRef = useRef<Map<string, JobDetail>>(new Map());

  const applyJobDetail = useCallback((d: JobDetail) => {
    setResult(d);
    if (d.status === "done" && d.results.length > 0) {
      setTotalSlotCount(d.results.length);
      setCompletedSlots(
        d.results.map((img) => ({
          id: img.id,
          status: "success" as const,
          image: img,
          generationMs: img.generation_ms ?? undefined,
        })),
      );
      void (async () => {
        if (!(await isClientCacheReady())) return;
        for (const img of d.results) {
          const url = img.download_url || img.preview_url;
          if (!url) continue;
          const full = apiAssetUrl(url);
          if (!full) continue;
          try {
            await saveImageToClientCache({
              jobId: d.id,
              resultId: img.id,
              downloadUrl: full,
            });
          } catch {
            // 单张失败不阻断展示
          }
        }
      })();
    } else if (d.status === "failed") {
      setJobStatus("failed");
    }
  }, []);

  const onSelectJob = (job: JobListItem) => {
    setActiveJobId(job.id);
    setPrompt(job.input_prompt);
    setError("");
    if (job.status === "done") {
      setJobStatus("done");
      setElapsedMs(new Date(job.updated_at).getTime() - new Date(job.created_at).getTime());
    } else if (job.status === "failed") {
      setJobStatus("failed");
      setElapsedMs(new Date(job.updated_at).getTime() - new Date(job.created_at).getTime());
    } else {
      setJobStatus("idle");
    }

    // 乐观渲染：列表已有 thumb_url / result_count，右侧立即出占位+首图
    if (job.status === "done" && job.result_count > 0) {
      setTotalSlotCount(job.result_count);
      const optimistic: OutputSlot[] = Array.from({ length: job.result_count }, (_, i) => {
        if (i === 0 && job.thumb_url) {
          return {
            id: `${job.id}-opt-${i}`,
            status: "success" as const,
            image: {
              id: `${job.id}-opt-0`,
              provider: "",
              thumb_url: job.thumb_url,
              preview_url: job.thumb_url,
            },
          };
        }
        return { id: `${job.id}-opt-${i}`, status: "pending" as const };
      });
      setCompletedSlots(optimistic);
    } else {
      setCompletedSlots([]);
      setTotalSlotCount(0);
    }

    const cached = jobDetailCacheRef.current.get(job.id);
    if (cached) {
      applyJobDetail(cached);
      return;
    }

    void getJobDetailCached(job.id)
      .then(({ data }) => {
        jobDetailCacheRef.current.set(job.id, data);
        applyJobDetail(data);
      })
      .catch(() => setResult(null));
  };

  useEffect(() => {
    // 用户切换时清空组件级 job 详情缓存，防止后续用户短暂看到前用户已加载的数据。
    jobDetailCacheRef.current.clear();
  }, [user?.id]);

  useEffect(() => {
    if (!user) return;
    void ensureConversation().catch((err) => {
      setError(err instanceof Error ? err.message : "加载对话失败");
    });
  }, [user]);

  const buildState = useCallback(
    (): ConversationState => ({
      prompt,
      mode,
      workflow,
      enhanceEnabled,
      textModel,
      castingModels,
      actorImageCounts,
      imageEngine,
      directorFactors,
      renderFactors,
      genConfig,
      activeAspect,
      lastJobId: result?.id ?? null,
    }),
    [
      prompt,
      mode,
      workflow,
      enhanceEnabled,
      textModel,
      castingModels,
      actorImageCounts,
      imageEngine,
      directorFactors,
      renderFactors,
      genConfig,
      activeAspect,
      result?.id,
    ],
  );

  const applyState = (s: ConversationState) => {
    setPrompt(s.prompt ?? "");
    setMode(s.mode ?? "director");
    setWorkflow(s.workflow ?? "full_agent");
    setEnhanceEnabled(s.enhanceEnabled ?? false);
    setTextModel(s.textModel ?? "gpt");
    setCastingModels(s.castingModels ?? ["gpt", "grok"]);
    setActorImageCounts(s.actorImageCounts ?? EMPTY_CONVERSATION_STATE.actorImageCounts);
    setImageEngine(s.imageEngine ?? "chatgpt");
    setDirectorFactors(s.directorFactors ?? { x: 0.5, y: 0.5 });
    setRenderFactors(s.renderFactors ?? { x: 0.5, y: 0.5 });
    setGenConfig({ ...DEFAULT_GEN_CONFIG, ...(s.genConfig ?? {}) });
    setActiveAspect(s.activeAspect ?? "1:1");
    if (s.lastJobId) {
      void jobsApi.get(s.lastJobId).then(setResult).catch(() => setResult(null));
    } else {
      setResult(null);
    }
  };

  const ensureConversation = async () => {
    const list = await conversationsApi.list();
    const studioList = list.filter((c) => !isChatConversationState(c.state));
    if (studioList.length > 0) {
      const c = studioList[0];
      setConversationId(c.id);
      applyState((c.state as ConversationState) ?? EMPTY_CONVERSATION_STATE);
      return;
    }
    const created = await conversationsApi.create({ state: EMPTY_CONVERSATION_STATE });
    setConversationId(created.id);
    setRefreshKey((k) => k + 1);
  };

  const persistConversation = useCallback((id: string, state: ConversationState, title?: string) => {
    if (saveTimer.current) clearTimeout(saveTimer.current);
    saveTimer.current = setTimeout(() => {
      void conversationsApi.patch(id, {
        title: title ?? conversationTitle(state.prompt),
        state,
      });
    }, 600);
  }, []);

  useEffect(() => {
    if (!conversationId) return;
    persistConversation(conversationId, buildState());
  }, [conversationId, buildState, persistConversation]);

  useEffect(() => {
    if (!busy || startedAt <= 0) return;
    const tick = () => setElapsedMs(Date.now() - startedAt);
    tick();
    const timer = setInterval(tick, 500);
    return () => clearInterval(timer);
  }, [busy, startedAt]);

  const onNewConversation = async () => {
    const created = await conversationsApi.create({ state: EMPTY_CONVERSATION_STATE });
    setConversationId(created.id);
    applyState(EMPTY_CONVERSATION_STATE);
    setActiveJobId(null);
    setResult(null);
    setCompletedSlots([]);
    setTotalSlotCount(0);
    setJobStatus("idle");
    setElapsedMs(0);
    setError("");
    setRefreshKey((k) => k + 1);
  };

  const enhanceLocked = workflow === "keyword_ps";

  const toggleCastingModel = (id: TextModelId) => {
    setCastingModels((prev) => {
      if (prev.includes(id)) return prev.length > 1 ? prev.filter((m) => m !== id) : prev;
      return [...prev, id];
    });
  };

  const onAspectChange = (id: string, w: number, h: number) => {
    setActiveAspect(id);
    setGenConfig((c) => ({ ...c, width: w, height: h }));
  };

  const onStylePresetChange = (name: string, director: FactorPoint, render: FactorPoint, promptHint: string) => {
    setActiveStylePreset(name);
    setActiveStyleHint(promptHint);
    setDirectorFactors(director);
    setRenderFactors(render);
  };

  const onGenerate = async () => {
    if (!prompt.trim() || !conversationId) return;
    // 端侧缓存只是本地副本，图片本身存在服务端，output-panel 缓存缺失时会回退远程预览。
    // 这里曾经是硬门槛，直接 return 导致请求根本发不出去；而目录句柄的授权不跨浏览器
    // 会话保留，重开浏览器就失效，等于把生图整个堵死。
    const cacheReady = await isClientCacheReady();
    const directorModels = mode === "casting" ? castingModels : [textModel];
    const counts: Record<string, number> = {};
    for (const m of directorModels) counts[m] = actorImageCounts[m] ?? 1;
    const totalImages = Object.values(counts).reduce((a, b) => a + b, 0);

    setBusy(true);
    setError("");
    setQueueHint(
      cacheReady ? "" : "端侧缓存未就绪，本次只保留服务端图片；如需本地副本请在「设置 → 端侧缓存」选择目录",
    );
    setResult(null);
    setProgress(5);
    setStage("queued");
    setJobStatus("running");
    const startTime = Date.now();
    setStartedAt(startTime);
    setElapsedMs(0);
    setTotalSlotCount(totalImages);
    setCompletedSlots([]);

    let pollTimer: ReturnType<typeof setInterval> | null = null;
    let queuedTimer: ReturnType<typeof setTimeout> | null = null;
    let es: EventSource | null = null;
    let jobStarted = false;

    const markStarted = () => {
      if (jobStarted) return;
      jobStarted = true;
      setQueueHint("");
    };

    const cleanup = () => {
      if (pollTimer) clearInterval(pollTimer);
      if (queuedTimer) clearTimeout(queuedTimer);
      es?.close();
    };

    const finishJob = async (jobId: string, failedError?: string) => {
      cleanup();
      const elapsed = Date.now() - startTime;
      try {
        const d = await jobsApi.get(jobId);
        setResult(d);
        setStage(d.status);
        setProgress(d.status === "done" ? 100 : d.status === "failed" ? 0 : 5);
        setActiveJobId(jobId);
        setElapsedMs(elapsed);
        setRefreshKey((k) => k + 1);

        if (d.status === "done" && d.results.length > 0) {
          setJobStatus("done");
          setTotalSlotCount(d.results.length);
          setCompletedSlots(
            d.results.map((img, i) => ({
              id: img.id,
              status: "success" as const,
              image: img,
              label: `slot-${i}`,
              generationMs: img.generation_ms ?? undefined,
            })),
          );
        } else if (d.status === "failed") {
          setJobStatus("failed");
          const errMsg = formatJobError(failedError ?? d.error_message ?? "生成失败");
          setError(errMsg);
        }

        if (conversationId) {
          persistConversation(conversationId, { ...buildState(), lastJobId: jobId });
        }
      } catch (err) {
        setJobStatus("failed");
        const errMsg = formatJobError(err instanceof Error ? err.message : "获取结果失败");
        setError(errMsg);
      } finally {
        setBusy(false);
      }
    };

    try {
      const styleParts = [
        activeStylePreset ? `[风格预设: ${activeStylePreset}]` : "",
        activeStyleHint ? `Style reference: ${activeStyleHint}.` : "",
      ].filter(Boolean);
      const stylePrefix = styleParts.length ? `${styleParts.join(" ")} ` : "";
      const polishFactor = enhanceLocked ? 1 : genConfig.polish_factor;
      const { job_id } = await jobsApi.create({
        mode,
        workflow_path: workflow,
        ps_enabled: enhanceLocked || polishFactor >= 0.35,
        provider: imageEngine,
        director_models: directorModels,
        director_factors: directorFactors,
        ps_factors: renderFactors,
        input_prompt: `${stylePrefix}${prompt.trim()}`,
        gen_config: { ...snappedGenConfig(genConfig), polish_factor: polishFactor },
        conversation_id: conversationId,
        actor_image_counts: counts,
      });

      setActiveJobId(job_id);
      setRefreshKey((k) => k + 1);

      queuedTimer = setTimeout(() => {
        if (!jobStarted) {
          setQueueHint("任务仍在排队，请稍候…（若超过 30 秒仍未开始，请联系管理员检查 worker 服务）");
        }
      }, 30000);

      pollTimer = setInterval(() => {
        void jobsApi
          .getStatus(job_id)
          .then((d) => {
            setStage(d.status);
            setProgress(d.progress);
            if (d.status !== "queued") markStarted();
            if (d.status === "generating" || d.status === "uploading") {
              void jobsApi
                .get(job_id)
                .then((j) => mergePartialResults(j.results))
                .catch(() => undefined);
            }
            if (d.status === "done" || d.status === "failed") {
              void finishJob(job_id, d.error_message ?? undefined);
            }
          })
          .catch(() => undefined);
      }, 2000);

      es = new EventSource(jobsApi.eventsUrl(job_id), { withCredentials: true });
      es.onmessage = (ev) => {
        const data = JSON.parse(ev.data) as {
          event?: string;
          stage: string;
          progress: number;
          error?: string;
          result_id?: string;
          generation_ms?: number;
          preview_url?: string;
          thumb_url?: string;
          download_url?: string;
        };
        setStage(data.stage);
        setProgress(data.progress);
        if (data.stage !== "queued") markStarted();
        if (data.event === "slot_done" && data.result_id) {
          const partial: JobResult = {
            id: data.result_id,
            provider: "",
            preview_url: data.preview_url,
            thumb_url: data.thumb_url,
            download_url: data.download_url,
            generation_ms: data.generation_ms,
          };
          setCompletedSlots((prev) => {
            if (prev.some((s) => s.id === data.result_id)) return prev;
            return [
              ...prev,
              {
                id: data.result_id!,
                status: "success" as const,
                image: partial,
                generationMs: data.generation_ms,
              },
            ];
          });
          void jobsApi
            .get(job_id)
            .then((j) => mergePartialResults(j.results))
            .catch(() => undefined);
        }
        if (data.stage === "done" || data.stage === "failed") {
          void finishJob(job_id, data.error);
        }
      };
      es.onerror = () => {
        void jobsApi.getStatus(job_id).then((d) => {
          if (d.status === "done" || d.status === "failed") {
            void finishJob(job_id, d.error_message ?? undefined);
          }
        });
      };
    } catch (err) {
      cleanup();
      const errMsg = formatJobError(err instanceof Error ? err.message : "生成失败");
      setError(errMsg);
      setJobStatus("failed");
      setBusy(false);
    }
  };

  const stageLabel = STAGE_LABELS[stage] ?? stage;

  return (
    <div className="flex h-[calc(100dvh-3rem)] min-h-0 flex-col bg-[var(--neo-surface-raised)]">
      {/* Grok OCR 快捷入口（G7-P2，图片 → 文字提取） */}
      <Button
        variant="outline"
        size="sm"
        onClick={() => setOcrOpen(true)}
        className="absolute right-3 top-3 z-30 gap-1.5"
      >
        <ScanText className="h-3.5 w-3.5" aria-hidden />
        Grok OCR
      </Button>
      {ocrOpen && (
        <div
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4 backdrop-blur-sm"
          onMouseDown={(e) => {
            if (e.target === e.currentTarget) setOcrOpen(false);
          }}
        >
          <OcrPanel onClose={() => setOcrOpen(false)} />
        </div>
      )}
      <ResizableStudioLayout
        widths={columnWidths}
        onWidthsChange={(w) => {
          setColumnWidths(w);
          saveColumnRatios(w);
        }}
      >
        <GenerationRecordsPanel
          activeId={activeJobId}
          onSelect={onSelectJob}
          onNew={onNewConversation}
          refreshKey={refreshKey}
        />
        <GenConfigPanel
          prompt={prompt}
          onPromptChange={setPrompt}
          mode={mode}
          onModeChange={setMode}
          workflow={workflow}
          onWorkflowChange={setWorkflow}
          textModel={textModel}
          onTextModelChange={setTextModel}
          castingModels={castingModels}
          onToggleCastingModel={toggleCastingModel}
          actorImageCounts={actorImageCounts}
          onActorImageCountChange={(id, count) => setActorImageCounts((prev) => ({ ...prev, [id]: count }))}
          imageEngine={imageEngine}
          onImageEngineChange={setImageEngine}
          polishLocked={enhanceLocked}
          directorFactors={directorFactors}
          onDirectorFactorsChange={setDirectorFactors}
          renderFactors={renderFactors}
          onRenderFactorsChange={setRenderFactors}
          genConfig={genConfig}
          onGenConfigChange={setGenConfig}
          activeAspect={activeAspect}
          onAspectChange={onAspectChange}
          activeStylePreset={activeStylePreset}
          onStylePresetChange={onStylePresetChange}
          queueHint={queueHint}
          busy={busy}
          stageLabel={stageLabel}
          progress={progress}
          error={error}
          onGenerate={onGenerate}
        />
        <OutputPanel
          slots={displaySlots}
          results={result?.results ?? []}
          busy={busy}
          stageLabel={stageLabel}
          progress={progress}
          elapsedMs={elapsedMs}
          jobStatus={jobStatus}
          error={error}
          onRetry={() => void onGenerate()}
          expectedWidth={genConfig.width}
          expectedHeight={genConfig.height}
        />
      </ResizableStudioLayout>
    </div>
  );
}
