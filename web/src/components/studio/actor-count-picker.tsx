"use client";

import { ChoiceButton } from "@/components/ui/choice-button";

const QUICK_COUNTS = [1, 2, 3, 4, 5, 6, 7, 8, 9] as const;
const MAX_COUNT = 40;

type Props = {
  value: number;
  onChange: (n: number) => void;
};

export function ActorCountPicker({ value, onChange }: Props) {
  const clamp = (n: number) => Math.min(MAX_COUNT, Math.max(1, Math.floor(n) || 1));
  const showCustom = value >= 10;

  return (
    <div className="flex flex-wrap items-center gap-1">
      {QUICK_COUNTS.map((n) => (
        <ChoiceButton key={n} variant="pill" active={value === n} onClick={() => onChange(n)}>
          {n}
        </ChoiceButton>
      ))}
      <label className="ml-1 inline-flex items-center gap-1 text-xs text-zinc-500">
        <span>≥10</span>
        <input
          type="number"
          min={10}
          max={MAX_COUNT}
          step={1}
          className="h-7 w-14 rounded-md border border-zinc-200 px-1.5 text-center text-xs"
          placeholder="10"
          value={showCustom ? value : ""}
          onChange={(e) => {
            const raw = e.target.value;
            if (!raw) return;
            onChange(clamp(Number(raw)));
          }}
          onFocus={() => {
            if (value < 10) onChange(10);
          }}
        />
      </label>
    </div>
  );
}
