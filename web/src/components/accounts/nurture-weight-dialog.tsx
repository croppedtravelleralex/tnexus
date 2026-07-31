"use client";

import { X } from "lucide-react";
import { useEffect, useState } from "react";

import { BindingSgHeatmap } from "@/components/accounts/BindingSgHeatmap";
import { Button } from "@/components/ui/button";
import type { IpNurturePreset } from "@/lib/api";

type Props = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  bindingKey: string;
  presets: IpNurturePreset[];
  initialWeights: number[][];
  onSave: (customMatrix: number[][]) => void | Promise<void>;
};

export function NurtureWeightDialog({
  open,
  onOpenChange,
  bindingKey,
  presets,
  initialWeights,
  onSave,
}: Props) {
  const [matrix, setMatrix] = useState<number[][]>(initialWeights);
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    if (open) {
      setMatrix(initialWeights);
    }
  }, [open, initialWeights]);

  if (!open) return null;

  const presetLabel = presets.find((p) => p.id === bindingKey)?.label;

  return (
    <div className="fixed inset-0 z-[100] flex items-center justify-center bg-black/30 p-4 backdrop-blur-sm">
      <div className="neo-card max-h-[90vh] w-full max-w-3xl overflow-y-auto p-6">
        <div className="mb-4 flex items-start justify-between gap-3">
          <div>
            <h2 className="text-lg font-semibold text-[var(--neo-ink)]">养号时段权重</h2>
            <p className="mt-1 text-sm text-[var(--neo-muted)]">
              点击格子循环调整 0 → 0.25 → 0.5 → 0.75 → 1（Asia/Singapore）
              {bindingKey ? ` · ${bindingKey.slice(0, 12)}${bindingKey.length > 12 ? "…" : ""}` : ""}
            </p>
          </div>
          <button
            type="button"
            className="rounded-lg p-1 text-[var(--neo-muted)] hover:bg-stone-100"
            onClick={() => onOpenChange(false)}
            aria-label="关闭"
          >
            <X className="size-5" />
          </button>
        </div>
        <div className="py-2">
          <BindingSgHeatmap weights={matrix} editable onChange={setMatrix} label={presetLabel} />
        </div>
        <div className="mt-4 flex justify-end gap-2">
          <Button type="button" variant="outline" onClick={() => onOpenChange(false)} disabled={saving}>
            取消
          </Button>
          <Button
            type="button"
            disabled={saving}
            onClick={() => {
              setSaving(true);
              void Promise.resolve(onSave(matrix))
                .then(() => onOpenChange(false))
                .finally(() => setSaving(false));
            }}
          >
            保存
          </Button>
        </div>
      </div>
    </div>
  );
}
