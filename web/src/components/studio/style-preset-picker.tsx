"use client";

import { useMemo, useState } from "react";
import { ChevronDown, ChevronRight } from "lucide-react";
import { ChoiceButton } from "@/components/ui/choice-button";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/input";
import type { FactorPoint } from "@/lib/api";
import { STYLE_CATEGORIES, stylePresetLabel } from "@/lib/presets";

type Props = {
  activeStylePreset: string;
  defaultExpanded?: boolean;
  onSelect: (label: string, director: FactorPoint, render: FactorPoint, promptHint: string) => void;
};

export function StylePresetPicker({ activeStylePreset, defaultExpanded = false, onSelect }: Props) {
  const [expanded, setExpanded] = useState<Record<string, boolean>>(() => {
    const init: Record<string, boolean> = {};
    for (const cat of STYLE_CATEGORIES) {
      init[cat.id] = defaultExpanded;
    }
    return init;
  });

  const allExpanded = useMemo(
    () => STYLE_CATEGORIES.every((c) => expanded[c.id]),
    [expanded],
  );

  const toggleAll = () => {
    const next = !allExpanded;
    const map: Record<string, boolean> = {};
    for (const cat of STYLE_CATEGORIES) map[cat.id] = next;
    setExpanded(map);
  };

  return (
    <div className="space-y-2">
      <div className="flex items-center justify-between gap-2">
        <Label>风格预设</Label>
        <Button type="button" variant="ghost" size="sm" className="h-7 text-xs" onClick={toggleAll}>
          {allExpanded ? "一键收起" : "一键展开"}
        </Button>
      </div>
      <div className="space-y-1.5">
        {STYLE_CATEGORIES.map((cat) => {
          const open = expanded[cat.id];
          return (
            <div key={cat.id} className="rounded-lg border border-zinc-200 bg-zinc-50/80">
              <button
                type="button"
                className="flex w-full items-center gap-1.5 px-2.5 py-2 text-left text-xs font-semibold text-zinc-800"
                onClick={() => setExpanded((s) => ({ ...s, [cat.id]: !open }))}
              >
                {open ? <ChevronDown className="size-3.5 shrink-0" /> : <ChevronRight className="size-3.5 shrink-0" />}
                {cat.name}
                <span className="ml-auto font-normal text-zinc-400">{cat.variants.length}</span>
              </button>
              {open ? (
                <div className="flex flex-wrap gap-1.5 border-t border-zinc-200/80 px-2.5 pb-2.5 pt-1.5">
                  {cat.variants.map((v) => {
                    const label = stylePresetLabel(cat.name, v.name);
                    return (
                      <ChoiceButton
                        key={label}
                        variant="chip"
                        active={activeStylePreset === label}
                        className="text-[11px]"
                        onClick={() => onSelect(label, v.director, v.render, v.promptHint)}
                      >
                        {v.name}
                      </ChoiceButton>
                    );
                  })}
                </div>
              ) : null}
            </div>
          );
        })}
      </div>
    </div>
  );
}
