"use client";

import { Loader2, Sparkles } from "lucide-react";
import { FactorPlane } from "@/components/factor-plane";
import { Button } from "@/components/ui/button";
import { ChoiceButton, SegmentGroup } from "@/components/ui/choice-button";
import { Label, Textarea } from "@/components/ui/input";
import type { FactorPoint } from "@/lib/api";
import {
  ASPECT_PRESETS,
  DEFAULT_GEN_CONFIG,
  QUALITY_OPTIONS,
  snap16,
  type GenConfig,
  type ImageQuality,
} from "@/lib/gen-config";
import { IMAGE_ENGINES, TEXT_MODELS, textModelLabel, type TextModelId } from "@/lib/models";
import { STYLE_PRESETS } from "@/lib/presets";

type Props = {
  prompt: string;
  onPromptChange: (v: string) => void;
  mode: "director" | "casting";
  onModeChange: (v: "director" | "casting") => void;
  workflow: "full_agent" | "keyword_ps";
  onWorkflowChange: (v: "full_agent" | "keyword_ps") => void;
  textModel: TextModelId;
  onTextModelChange: (v: TextModelId) => void;
  castingModels: TextModelId[];
  onToggleCastingModel: (id: TextModelId) => void;
  actorImageCounts: Record<TextModelId, number>;
  onActorImageCountChange: (id: TextModelId, count: number) => void;
  imageEngine: "chatgpt" | "grok";
  onImageEngineChange: (v: "chatgpt" | "grok") => void;
  enhanceEnabled: boolean;
  enhanceLocked: boolean;
  onEnhanceChange: (v: boolean) => void;
  directorFactors: FactorPoint;
  onDirectorFactorsChange: (v: FactorPoint) => void;
  renderFactors: FactorPoint;
  onRenderFactorsChange: (v: FactorPoint) => void;
  genConfig: GenConfig;
  onGenConfigChange: (v: GenConfig) => void;
  activeAspect: string;
  onAspectChange: (id: string, w: number, h: number) => void;
  busy: boolean;
  stageLabel: string;
  progress: number;
  error: string;
  onGenerate: () => void;
};

export function GenConfigPanel(props: Props) {
  const {
    prompt,
    onPromptChange,
    mode,
    onModeChange,
    workflow,
    onWorkflowChange,
    textModel,
    onTextModelChange,
    castingModels,
    onToggleCastingModel,
    actorImageCounts,
    onActorImageCountChange,
    imageEngine,
    onImageEngineChange,
    enhanceEnabled,
    enhanceLocked,
    onEnhanceChange,
    directorFactors,
    onDirectorFactorsChange,
    renderFactors,
    onRenderFactorsChange,
    genConfig,
    onGenConfigChange,
    activeAspect,
    onAspectChange,
    busy,
    stageLabel,
    progress,
    error,
    onGenerate,
  } = props;

  const activeActors: TextModelId[] = mode === "casting" ? castingModels : [textModel];

  return (
    <div className="flex h-full min-h-0 flex-col border-r border-[var(--neo-border)] bg-[var(--neo-surface)]">
      <div className="panel-header text-zinc-900">生图配置</div>
      <div className="panel-body scrollbar-hide space-y-5">
        <Textarea
          placeholder="描述你想生成的画面..."
          value={prompt}
          onChange={(e) => onPromptChange(e.target.value)}
          className="min-h-[220px] resize-y text-[15px] leading-relaxed"
          rows={10}
        />

        <div className="grid gap-4 sm:grid-cols-2">
          <Segment label="模式" value={mode} options={[
            { value: "director", label: "导演模式" },
            { value: "casting", label: "竞演模式" },
          ]} onChange={(v) => onModeChange(v as typeof mode)} />
          <Segment label="工作流" value={workflow} options={[
            { value: "full_agent", label: "完整方案" },
            { value: "keyword_ps", label: "风格锚点" },
          ]} onChange={(v) => onWorkflowChange(v as typeof workflow)} />
        </div>

        {mode === "director" ? (
          <Segment label="构思模型" value={textModel} options={TEXT_MODELS.map((m) => ({ value: m.id, label: m.label }))} onChange={(v) => onTextModelChange(v as TextModelId)} />
        ) : (
          <div className="space-y-2">
            <Label>构思模型（演员，可多选）</Label>
            <div className="flex flex-wrap gap-2">
              {TEXT_MODELS.map((m) => (
                <ChoiceButton
                  key={m.id}
                  variant="chip"
                  active={castingModels.includes(m.id)}
                  onClick={() => onToggleCastingModel(m.id)}
                >
                  {m.label}
                </ChoiceButton>
              ))}
            </div>
          </div>
        )}

        <div className="space-y-2">
          <Label>{mode === "casting" ? "每位演员出图张数" : "导演出图张数"}</Label>
          <div className="space-y-2">
            {activeActors.map((actorId) => (
              <div key={actorId} className="flex items-center gap-2 rounded-lg border border-zinc-200 bg-zinc-50 px-3 py-2">
                <span className="w-16 shrink-0 text-xs font-medium text-zinc-700">{textModelLabel(actorId)}</span>
                <div className="flex flex-wrap gap-1">
                  {Array.from({ length: 10 }, (_, i) => i + 1).map((n) => (
                    <ChoiceButton
                      key={n}
                      variant="pill"
                      active={(actorImageCounts[actorId] ?? 1) === n}
                      onClick={() => onActorImageCountChange(actorId, n)}
                    >
                      {n}
                    </ChoiceButton>
                  ))}
                </div>
              </div>
            ))}
          </div>
        </div>

        <Segment label="绘图引擎" value={imageEngine} options={IMAGE_ENGINES.map((e) => ({ value: e.id, label: e.label }))} onChange={(v) => onImageEngineChange(v as typeof imageEngine)} />

        <label className="flex items-center gap-2 text-sm text-zinc-600">
          <input type="checkbox" checked={enhanceLocked ? true : enhanceEnabled} disabled={enhanceLocked} onChange={(e) => onEnhanceChange(e.target.checked)} />
          智能润色{enhanceLocked ? "（风格锚点自动开启）" : ""}
        </label>

        <div className="space-y-2">
          <Label>质量</Label>
          <div className="flex flex-wrap gap-2">
            {QUALITY_OPTIONS.map((q) => (
              <ChoiceButton
                key={q.id}
                variant="chip"
                active={genConfig.quality === q.id}
                onClick={() => onGenConfigChange({ ...genConfig, quality: q.id })}
              >
                {q.label}
              </ChoiceButton>
            ))}
          </div>
        </div>

        <div className="space-y-2">
          <div className="flex items-center justify-between">
            <Label>尺寸</Label>
            <label className="flex items-center gap-1.5 text-xs text-zinc-500">
              <input
                type="checkbox"
                checked={genConfig.align_16}
                onChange={(e) => onGenConfigChange({ ...genConfig, align_16: e.target.checked })}
              />
              16 倍数对齐
            </label>
          </div>
          <div className="flex items-center gap-2">
            <div className="flex-1">
              <span className="text-xs text-zinc-400">W</span>
              <input
                type="number"
                className="mt-0.5 flex h-9 w-full rounded-md border border-zinc-200 px-2 text-sm"
                value={genConfig.width}
                onChange={(e) => {
                  const w = Number(e.target.value);
                  onGenConfigChange({ ...genConfig, width: genConfig.align_16 ? snap16(w) : w });
                }}
              />
            </div>
            <span className="pt-4 text-zinc-300">↔</span>
            <div className="flex-1">
              <span className="text-xs text-zinc-400">H</span>
              <input
                type="number"
                className="mt-0.5 flex h-9 w-full rounded-md border border-zinc-200 px-2 text-sm"
                value={genConfig.height}
                onChange={(e) => {
                  const h = Number(e.target.value);
                  onGenConfigChange({ ...genConfig, height: genConfig.align_16 ? snap16(h) : h });
                }}
              />
            </div>
          </div>
        </div>

        <div className="space-y-2">
          <Label>宽高比</Label>
          <div className="grid grid-cols-4 gap-2 sm:grid-cols-6">
            {ASPECT_PRESETS.map((a) => (
              <ChoiceButton
                key={a.id}
                variant="chip"
                active={activeAspect === a.id}
                className="w-full py-2 text-center text-[10px] sm:text-xs"
                onClick={() => onAspectChange(a.id, a.w, a.h)}
              >
                {a.label}
              </ChoiceButton>
            ))}
          </div>
        </div>

        <label className="flex items-center justify-between text-sm">
          <span className="text-zinc-700">透明背景</span>
          <input
            type="checkbox"
            checked={genConfig.transparent_bg}
            onChange={(e) => onGenConfigChange({ ...genConfig, transparent_bg: e.target.checked })}
          />
        </label>

        <div className="space-y-2">
          <Label>风格预设</Label>
          <div className="flex flex-wrap gap-1.5">
            {STYLE_PRESETS.map((p) => (
              <ChoiceButton
                key={p.name}
                variant="chip"
                onClick={() => {
                  onDirectorFactorsChange(p.director);
                  onRenderFactorsChange(p.render);
                }}
              >
                {p.name}
              </ChoiceButton>
            ))}
          </div>
        </div>

        <div className="grid gap-4 sm:grid-cols-2">
          <FactorPlane
            title="导演因子"
            value={directorFactors}
            onChange={onDirectorFactorsChange}
            labels={{ xLow: "描述具体", xHigh: "思维发散", yLow: "技术细节", yHigh: "情绪氛围" }}
          />
          <FactorPlane
            title="画面质感"
            value={renderFactors}
            onChange={onRenderFactorsChange}
            labels={{ xLow: "留白简约", xHigh: "细节密度", yLow: "平光自然", yHigh: "光影戏剧" }}
          />
        </div>

        {error && <p className="text-sm text-red-600">{error}</p>}
      </div>

      <div className="border-t border-zinc-200 p-4">
        <Button onClick={onGenerate} disabled={busy} className="w-full" size="lg">
          {busy ? (
            <span className="flex items-center gap-2">
              <Loader2 className="h-4 w-4 animate-spin" />
              {stageLabel} {progress}%
            </span>
          ) : (
            <span className="flex items-center gap-2">
              <Sparkles className="h-4 w-4" />
              开始生成
            </span>
          )}
        </Button>
      </div>
    </div>
  );
}

function Segment({
  label,
  value,
  options,
  onChange,
}: {
  label: string;
  value: string;
  options: { value: string; label: string }[];
  onChange: (v: string) => void;
}) {
  return (
    <div className="space-y-2">
      <Label>{label}</Label>
      <SegmentGroup>
        {options.map((o) => (
          <ChoiceButton
            key={o.value}
            variant="segment"
            active={value === o.value}
            onClick={() => onChange(o.value)}
          >
            {o.label}
          </ChoiceButton>
        ))}
      </SegmentGroup>
    </div>
  );
}
