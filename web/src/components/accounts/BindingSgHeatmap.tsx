"use client";

import { useMemo } from "react";
import { cn } from "@/lib/utils";

const DAY_LABELS = ["一", "二", "三", "四", "五", "六", "日"] as const;
const SLOT_LABELS = [
  "00-02",
  "02-04",
  "04-06",
  "06-08",
  "08-10",
  "10-12",
  "12-14",
  "14-16",
  "16-18",
  "18-20",
  "20-22",
  "22-24",
] as const;

type Props = {
  weights: number[][];
  label?: string;
  className?: string;
};

function clampWeight(value: number) {
  if (!Number.isFinite(value)) return 0;
  return Math.max(0, Math.min(1, value));
}

function weightColor(weight: number) {
  const w = clampWeight(weight);
  if (w <= 0) return "bg-stone-100";
  if (w < 0.25) return "bg-emerald-200/90";
  if (w < 0.5) return "bg-emerald-400/90";
  if (w < 0.75) return "bg-emerald-600/90";
  return "bg-emerald-800/90";
}

function normalizeWeights(weights: number[][]) {
  return Array.from({ length: 7 }, (_, day) =>
    Array.from({ length: 12 }, (_, slot) => clampWeight(Number(weights?.[day]?.[slot] ?? 0))),
  );
}

function matrixToWeights(matrix: number[][] | undefined, max: number) {
  if (!matrix?.length) return normalizeWeights([]);
  const peak = Math.max(1, max);
  return matrix.map((row) => row.map((cell) => clampWeight(Number(cell) / peak)));
}

export function bindingMatrixPeak(matrix: number[][] | undefined) {
  let peak = 0;
  for (const row of matrix || []) {
    for (const cell of row || []) {
      peak = Math.max(peak, Number(cell) || 0);
    }
  }
  return peak;
}

export function BindingSgHeatmap({ weights, label, className }: Props) {
  const matrix = useMemo(() => normalizeWeights(weights), [weights]);

  return (
    <div className={cn("inline-flex flex-col gap-0.5", className)}>
      {label ? <div className="text-[10px] font-medium text-stone-600">{label}</div> : null}
      <div className="flex items-start gap-1">
        <div className="flex flex-col gap-px pt-3">
          {SLOT_LABELS.map((slot) => (
            <div key={slot} className="flex h-2.5 items-center text-[7px] leading-none text-stone-400">
              {slot}
            </div>
          ))}
        </div>
        <div>
          <div className="mb-0.5 grid grid-cols-7 gap-px text-center text-[8px] text-stone-500">
            {DAY_LABELS.map((day) => (
              <span key={day} className="w-2.5">
                {day}
              </span>
            ))}
          </div>
          <div className="grid grid-cols-7 gap-px">
            {matrix.map((row, day) =>
              row.map((weight, slot) => (
                <span
                  key={`${day}-${slot}`}
                  title={`${DAY_LABELS[day]} ${SLOT_LABELS[slot]} · ${clampWeight(weight).toFixed(2)}`}
                  className={cn("size-2.5 rounded-[2px]", weightColor(weight))}
                />
              )),
            )}
          </div>
        </div>
      </div>
      <div className="text-[9px] text-stone-400">时区 Asia/Shanghai</div>
    </div>
  );
}

export function activityMatrixToWeights(matrix: number[][] | undefined) {
  return matrixToWeights(matrix, bindingMatrixPeak(matrix));
}
