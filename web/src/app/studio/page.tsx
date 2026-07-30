"use client";

import { useRouter } from "next/navigation";
import { useCallback, useEffect, useRef, useState } from "react";
import { GenerationRecordsPanel } from "@/components/studio/generation-records-panel";
import { GenConfigPanel } from "@/components/studio/gen-config-panel";
import { OutputPanel, type OutputSlot } from "@/components/studio/output-panel";
import { ResizableStudioLayout, DEFAULT_COLUMN_WIDTHS } from "@/components/studio/resizable-layout";
import { SiteHeader } from "@/components/site-header";
import { authApi, conversationsApi, jobsApi, type FactorPoint, type JobDetail, type JobListItem } from "@/lib/api";
import { useAuth } from "@/lib/auth";
import {
  conversationTitle,
  EMPTY_CONVERSATION_STATE,
  type ConversationState,
} from "@/lib/conversations";
import { DEFAULT_GEN_CONFIG, type GenConfig } from "@/lib/gen-config";
import type { TextModelId } from "@/lib/models";
import { LAYOUT_STORAGE_KEY, normalizeWidths } from "@/lib/studio-layout";
import { Loader2 } from "lucide-react";

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

function loadStoredLayout(): [number, number, number] | null {
  try {
    const raw = localStorage.getItem(LAYOUT_STORAGE_KEY);
    if (!raw) return null;
    const w = JSON.parse(raw) as number[];
    if (Array.isArray(w) && w.length === 3 && w.every((n) => typeof n === "number" && n >= 180)) {
      return [w[0], w[1], w[2]];
    }
  } catch {
    // ignore
  }
  return null;
}

export default function StudioPage() {
  const { user, loading } = useAuth();
  const router = useRouter();

  const [conversationId, setConversationId] = useState<string | null>(null);
  const [activeJobId, setActiveJobId] = useState<string | null>(null);
  const [prompt, setPrompt] = useState("");
  const [mode, setMode] = useState<"director" | "casting">("director");
  const [workflow, setWorkflow] = useState<"full_agent" | "keyword_ps">("full_agent");
  const [enhanceEnabled, setEnhanceEnabled] = useState(false);
  const [textModel, setTextModel] = useState<TextModelId>("gpt");
  const [castingModels, setCastingModels] = useState<TextModelId[]>(["gpt", "grok"]);
  const [actorImageCounts, setActorImageCounts] = useState<Record<TextModelId, number>>(
    EMPTY_CONVERSATION_STATE.actorImageCounts
  );
  const [imageEngine, setImageEngine] = useState<"chatgpt" | "grok">("chatgpt");
  const [directorFactors, setDirectorFactors] = useState<FactorPoint>({ x: 0.5, y: 0.5 });
  const [renderFactors, setRenderFactors] = useState<FactorPoint>({ x: 0.5, y: 0.5 });
  const [genConfig, setGenConfig] = useState<GenConfig>(DEFAULT_GEN_CONFIG);
  const [activeAspect, setActiveAspect] = useState("1:1");
  const [columnWidths, setColumnWidths] = useState<[number, number, number]>(DEFAULT_COLUMN_WIDTHS);
  const [savingLayout, setSavingLayout] = useState(false);
  const [layoutSaved, setLayoutSaved] = useState(false);
  const [busy, setBusy] = useState(false);
  const [progress, setProgress] = useState(0);
  const [stage, setStage] = useState("");
  const [result, setResult] = useState<JobDetail | null>(null);
  const [outputSlots, setOutputSlots] = useState<OutputSlot[]>([]);
  const [jobStatus, setJobStatus] = useState<"idle" | "running" | "done" | "failed">("idle");
  const [startedAt, setStartedAt] = useState(0);
  const [elapsedMs, setElapsedMs] = useState(0);
  const [refreshKey, setRefreshKey] = useState(0);
  const [error, setError] = useState("");

  const saveTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    if (!loading && !user) router.replace("/login");
  }, [loading, user, router]);

  useEffect(() => {
    if (!busy || !startedAt) return;
    const timer = setInterval(() => setElapsedMs(Date.now() - startedAt), 1000);
    return () => clearInterval(timer);
  }, [busy, startedAt]);

  useEffect(() => {
    if (!user) return;
    const applyLayout = (widths: [number, number, number]) => {
      const total = typeof window !== "undefined" ? window.innerWidth : 1200;
      setColumnWidths(normalizeWidths(widths, total));
    };

    void authApi
      .getPreferences()
      .then((prefs) => {
        const w = prefs.studio_layout?.columnWidths;
        if (Array.isArray(w) && w.length === 3) {
          applyLayout([w[0], w[1], w[2]]);
          return;
        }
        const stored = loadStoredLayout();
        if (stored) applyLayout(stored);
      })
      .catch(() => {
        const stored = loadStoredLayout();
        if (stored) applyLayout(stored);
      });
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
    ]
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
    setGenConfig(s.genConfig ?? DEFAULT_GEN_CONFIG);
    setActiveAspect(s.activeAspect ?? "1:1");
    if (s.lastJobId) {
      void jobsApi.get(s.lastJobId).then(setResult).catch(() => setResult(null));
    } else {
      setResult(null);
    }
  };

  const ensureConversation = async () => {
    const list = await conversationsApi.list();
    if (list.length > 0) {
      const c = list[0];
      setConversationId(c.id);
      applyState(c.state ?? EMPTY_CONVERSATION_STATE);
      return;
    }
    const created = await conversationsApi.create({ state: EMPTY_CONVERSATION_STATE });
    setConversationId(created.id);
    setRefreshKey((k) => k + 1);
  };

  const persistConversation = useCallback(
    (id: string, state: ConversationState, title?: string) => {
      if (saveTimer.current) clearTimeout(saveTimer.current);
      saveTimer.current = setTimeout(() => {
        void conversationsApi.patch(id, {
          title: title ?? conversationTitle(state.prompt),
          state,
        });
      }, 600);
    },
    []
  );

  useEffect(() => {
    if (!conversationId) return;
    persistConversation(conversationId, buildState());
  }, [conversationId, buildState, persistConversation]);

  const onSelectJob = (job: JobListItem) => {
    setActiveJobId(job.id);
    setPrompt(job.input_prompt);
    setError("");
    setOutputSlots([]);
    if (job.status === "done") {
      setJobStatus("done");
      setElapsedMs(
        new Date(job.updated_at).getTime() - new Date(job.created_at).getTime(),
      );
    } else if (job.status === "failed") {
      setJobStatus("failed");
      setElapsedMs(
        new Date(job.updated_at).getTime() - new Date(job.created_at).getTime(),
      );
    } else {
      setJobStatus("idle");
    }
    void jobsApi.get(job.id).then((d) => {
      setResult(d);
      if (d.status === "done" && d.results.length > 0) {
        setOutputSlots(
          d.results.map((img) => ({ id: img.id, status: "success" as const, image: img })),
        );
      }
    }).catch(() => setResult(null));
  };

  const onNewConversation = async () => {
    const created = await conversationsApi.create({ state: EMPTY_CONVERSATION_STATE });
    setConversationId(created.id);
    applyState(EMPTY_CONVERSATION_STATE);
    setActiveJobId(null);
    setResult(null);
    setOutputSlots([]);
    setJobStatus("idle");
    setElapsedMs(0);
    setError("");
    setRefreshKey((k) => k + 1);
  };

  const onSaveDefaultLayout = async () => {
    setSavingLayout(true);
    setLayoutSaved(false);
    try {
      localStorage.setItem(LAYOUT_STORAGE_KEY, JSON.stringify(columnWidths));
      await authApi.savePreferences({ studio_layout: { columnWidths } });
      setLayoutSaved(true);
      setTimeout(() => setLayoutSaved(false), 2000);
    } catch (err) {
      setError(err instanceof Error ? err.message : "保存布局失败");
    } finally {
      setSavingLayout(false);
    }
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

  const onGenerate = async () => {
    if (!prompt.trim() || !conversationId) return;
    const directorModels = mode === "casting" ? castingModels : [textModel];
    const counts: Record<string, number> = {};
    for (const m of directorModels) counts[m] = actorImageCounts[m] ?? 1;
    const totalImages = Object.values(counts).reduce((a, b) => a + b, 0);

    setBusy(true);
    setError("");
    setResult(null);
    setProgress(5);
    setStage("queued");
    setJobStatus("running");
    const startTime = Date.now();
    setStartedAt(startTime);
    setElapsedMs(0);
    setOutputSlots(
      Array.from({ length: Math.max(totalImages, 1) }, (_, i) => ({
        id: `slot-${i}`,
        status: "pending" as const,
      })),
    );

    let pollTimer: ReturnType<typeof setInterval> | null = null;
    let queuedTimer: ReturnType<typeof setTimeout> | null = null;
    let es: EventSource | null = null;

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
          setOutputSlots(
            d.results.map((img, i) => ({
              id: img.id,
              status: "success" as const,
              image: img,
              label: `slot-${i}`,
            })),
          );
        } else if (d.status === "failed") {
          setJobStatus("failed");
          const errMsg = formatJobError(failedError ?? d.error_message ?? "生成失败");
          setError(errMsg);
          setOutputSlots((prev) =>
            prev.map((s) => ({ ...s, status: "failed" as const, error: errMsg })),
          );
        }

        if (conversationId) {
          persistConversation(conversationId, { ...buildState(), lastJobId: jobId });
        }
      } catch (err) {
        setJobStatus("failed");
        const errMsg = formatJobError(err instanceof Error ? err.message : "获取结果失败");
        setError(errMsg);
        setOutputSlots((prev) =>
          prev.map((s) => ({ ...s, status: "failed" as const, error: errMsg })),
        );
      } finally {
        setBusy(false);
      }
    };

    try {
      const { job_id } = await jobsApi.create({
        mode,
        workflow_path: workflow,
        ps_enabled: enhanceLocked ? true : enhanceEnabled,
        provider: imageEngine,
        director_models: directorModels,
        director_factors: directorFactors,
        ps_factors: renderFactors,
        input_prompt: prompt.trim(),
        gen_config: genConfig,
        conversation_id: conversationId,
        actor_image_counts: counts,
      });

      queuedTimer = setTimeout(() => {
        setError(
          "任务仍在排队：请确认 tnexus-worker 已启动（WSL 运行 ./target/debug/tnexus-worker）",
        );
      }, 15000);

      pollTimer = setInterval(() => {
        void jobsApi
          .get(job_id)
          .then((d) => {
            setStage(d.status);
            setProgress(
              d.status === "done"
                ? 100
                : d.status === "failed"
                  ? 0
                  : d.status === "generating"
                    ? 55
                    : d.status === "directing"
                      ? 25
                      : d.status === "uploading"
                        ? 85
                        : 5,
            );
            if (d.status === "done" || d.status === "failed") {
              void finishJob(job_id, d.error_message ?? undefined);
            }
          })
          .catch(() => undefined);
      }, 2000);

      es = new EventSource(jobsApi.eventsUrl(job_id), { withCredentials: true });
      es.onmessage = (ev) => {
        const data = JSON.parse(ev.data) as { stage: string; progress: number; error?: string };
        setStage(data.stage);
        setProgress(data.progress);
        if (data.stage === "done" || data.stage === "failed") {
          void finishJob(job_id, data.error);
        }
      };
      es.onerror = () => {
        void jobsApi.get(job_id).then((d) => {
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
      setOutputSlots((prev) =>
        prev.map((s) => ({ ...s, status: "failed" as const, error: errMsg })),
      );
      setBusy(false);
    }
  };

  if (loading || !user) {
    return (
      <div className="flex min-h-screen items-center justify-center bg-white">
        <Loader2 className="h-8 w-8 animate-spin text-zinc-400" />
      </div>
    );
  }

  const stageLabel = STAGE_LABELS[stage] ?? stage;

  return (
    <div className="flex h-screen flex-col bg-white">
      <SiteHeader />
      <ResizableStudioLayout
        widths={columnWidths}
        onWidthsChange={setColumnWidths}
        onSaveDefault={onSaveDefaultLayout}
        saving={savingLayout}
        saved={layoutSaved}
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
          onActorImageCountChange={(id, count) =>
            setActorImageCounts((prev) => ({ ...prev, [id]: count }))
          }
          imageEngine={imageEngine}
          onImageEngineChange={setImageEngine}
          enhanceEnabled={enhanceEnabled}
          enhanceLocked={enhanceLocked}
          onEnhanceChange={setEnhanceEnabled}
          directorFactors={directorFactors}
          onDirectorFactorsChange={setDirectorFactors}
          renderFactors={renderFactors}
          onRenderFactorsChange={setRenderFactors}
          genConfig={genConfig}
          onGenConfigChange={setGenConfig}
          activeAspect={activeAspect}
          onAspectChange={onAspectChange}
          busy={busy}
          stageLabel={stageLabel}
          progress={progress}
          error={error}
          onGenerate={onGenerate}
        />
        <OutputPanel
          slots={outputSlots}
          results={result?.results ?? []}
          busy={busy}
          stageLabel={stageLabel}
          progress={progress}
          elapsedMs={elapsedMs}
          jobStatus={jobStatus}
          error={error}
          onRetry={() => void onGenerate()}
        />
      </ResizableStudioLayout>
    </div>
  );
}
