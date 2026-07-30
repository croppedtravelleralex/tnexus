"use client";

import { useCallback, useRef, useState } from "react";
import type { FactorPoint } from "@/lib/api";
import { cn } from "@/lib/utils";

type AxisLabels = {
  xLow: string;
  xHigh: string;
  yLow: string;
  yHigh: string;
};

type Props = {
  title: string;
  value: FactorPoint;
  onChange: (v: FactorPoint) => void;
  labels: AxisLabels;
};

const TOOLTIP_EST_H = 52;

function pointFromEvent(rect: DOMRect, clientX: number, clientY: number): FactorPoint {
  const x = Math.min(1, Math.max(0, (clientX - rect.left) / rect.width));
  const y = Math.min(1, Math.max(0, 1 - (clientY - rect.top) / rect.height));
  return { x, y };
}

export function FactorPlane({ title, value, onChange, labels }: Props) {
  const planeRef = useRef<HTMLDivElement>(null);
  const [dragging, setDragging] = useState(false);
  const [hover, setHover] = useState<FactorPoint | null>(null);
  const [cursor, setCursor] = useState<{ x: number; y: number } | null>(null);

  const active = dragging ? value : (hover ?? value);
  const quadrant = `${active.x < 0.5 ? labels.xLow : labels.xHigh} · ${active.y < 0.5 ? labels.yLow : labels.yHigh}`;
  const showTooltip = cursor && (dragging || hover);
  const flipBelow = cursor ? cursor.y < TOOLTIP_EST_H + 12 : false;

  const updateFromClient = useCallback(
    (clientX: number, clientY: number, commit: boolean) => {
      const rect = planeRef.current?.getBoundingClientRect();
      if (!rect) return;
      const next = pointFromEvent(rect, clientX, clientY);
      setCursor({ x: clientX - rect.left, y: clientY - rect.top });
      if (commit) onChange(next);
      else setHover(next);
    },
    [onChange]
  );

  return (
    <div className="space-y-2">
      <div className="flex items-center justify-between gap-2">
        <p className="text-xs font-medium text-zinc-700">{title}</p>
        <span className="rounded-full border border-green-200 bg-green-50 px-2 py-0.5 text-[10px] text-green-700">
          {quadrant}
        </span>
      </div>

      <div className="relative">
        <div
          ref={planeRef}
          className={cn(
            "neo-plane relative h-36 cursor-crosshair touch-none select-none overflow-hidden rounded-lg",
            dragging && "ring-2 ring-green-500/30"
          )}
          onPointerDown={(e) => {
            e.preventDefault();
            e.currentTarget.setPointerCapture(e.pointerId);
            setDragging(true);
            updateFromClient(e.clientX, e.clientY, true);
          }}
          onPointerMove={(e) => {
            const rect = planeRef.current?.getBoundingClientRect();
            if (!rect) return;
            setCursor({ x: e.clientX - rect.left, y: e.clientY - rect.top });
            if (dragging || e.buttons === 1) updateFromClient(e.clientX, e.clientY, true);
            else setHover(pointFromEvent(rect, e.clientX, e.clientY));
          }}
          onPointerUp={(e) => {
            e.currentTarget.releasePointerCapture(e.pointerId);
            setDragging(false);
          }}
          onPointerLeave={() => {
            if (!dragging) {
              setHover(null);
              setCursor(null);
            }
          }}
        >
          <div className="pointer-events-none absolute inset-0 grid grid-cols-2 grid-rows-2">
            <div className="border-r border-b border-zinc-200/80" />
            <div className="border-b border-zinc-200/80" />
            <div className="border-r border-zinc-200/80" />
            <div />
          </div>
          <span className="pointer-events-none absolute left-2 top-1.5 text-[9px] text-zinc-400">{labels.yHigh}</span>
          <span className="pointer-events-none absolute bottom-1.5 left-2 text-[9px] text-zinc-400">{labels.yLow}</span>
          <span className="pointer-events-none absolute bottom-1.5 left-1/2 -translate-x-1/2 text-[9px] text-zinc-400">{labels.xLow}</span>
          <span className="pointer-events-none absolute bottom-1.5 right-2 text-[9px] text-zinc-400">{labels.xHigh}</span>
          <div
            className="pointer-events-none absolute z-10 h-4 w-4 -translate-x-1/2 -translate-y-1/2 rounded-full border-2 border-white bg-green-600 shadow-md"
            style={{ left: `${value.x * 100}%`, top: `${(1 - value.y) * 100}%` }}
          />
        </div>

        {showTooltip && cursor && (
          <div
            className="pointer-events-none absolute z-30 min-w-[8rem] rounded-md border border-zinc-200 bg-white px-2 py-1 text-[10px] shadow-md"
            style={{
              left: cursor.x,
              top: flipBelow ? cursor.y + 12 : cursor.y - 6,
              transform: flipBelow ? "translate(-50%, 0)" : "translate(-50%, -100%)",
            }}
          >
            <div className="font-mono text-zinc-400">
              X {(dragging ? value : active).x.toFixed(2)} · Y {(dragging ? value : active).y.toFixed(2)}
            </div>
            <div className="text-zinc-700">{quadrant}</div>
          </div>
        )}
      </div>
    </div>
  );
}
