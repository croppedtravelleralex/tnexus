"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import { Button } from "@/components/ui/button";
import {
  DEFAULT_COLUMN_WIDTHS,
  MIN_COLUMN_WIDTH,
  normalizeWidths,
  type StudioLayoutPrefs,
} from "@/lib/studio-layout";
import { cn } from "@/lib/utils";

type Props = {
  widths: [number, number, number];
  onWidthsChange: (w: [number, number, number]) => void;
  onSaveDefault: () => void | Promise<void>;
  saving?: boolean;
  saved?: boolean;
  children: [React.ReactNode, React.ReactNode, React.ReactNode];
};

export function ResizableStudioLayout({
  widths,
  onWidthsChange,
  onSaveDefault,
  saving,
  saved,
  children,
}: Props) {
  const containerRef = useRef<HTMLDivElement>(null);
  const [dragging, setDragging] = useState<0 | 1 | null>(null);

  const onDrag = useCallback(
    (index: 0 | 1, clientX: number) => {
      const el = containerRef.current;
      if (!el) return;
      const rect = el.getBoundingClientRect();
      const total = rect.width - 8;
      const x = clientX - rect.left;
      if (index === 0) {
        const left = Math.max(MIN_COLUMN_WIDTH, Math.min(x - 4, total - MIN_COLUMN_WIDTH * 2));
        const rest = total - left;
        const mid = Math.max(MIN_COLUMN_WIDTH, Math.min(widths[1], rest - MIN_COLUMN_WIDTH));
        onWidthsChange([left, mid, rest - mid]);
      } else {
        const leftPlusMid = Math.max(widths[0] + MIN_COLUMN_WIDTH, Math.min(x - 4, total - MIN_COLUMN_WIDTH));
        const left = widths[0];
        const mid = leftPlusMid - left;
        const right = total - leftPlusMid;
        onWidthsChange([left, Math.max(MIN_COLUMN_WIDTH, mid), Math.max(MIN_COLUMN_WIDTH, right)]);
      }
    },
    [onWidthsChange, widths]
  );

  useEffect(() => {
    if (dragging === null) return;
    const move = (e: PointerEvent) => onDrag(dragging, e.clientX);
    const up = () => setDragging(null);
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    return () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
    };
  }, [dragging, onDrag]);

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex shrink-0 items-center justify-end gap-2 border-b border-zinc-100 px-3 py-1.5">
        <span className="mr-auto text-xs text-zinc-400">拖拽列边界调整宽度</span>
        <Button variant="outline" size="sm" onClick={() => void onSaveDefault()} disabled={saving}>
          {saving ? "保存中…" : saved ? "已保存" : "设为默认布局"}
        </Button>
      </div>
      <div ref={containerRef} className="flex min-h-0 flex-1">
        <div style={{ width: widths[0] }} className="min-h-0 shrink-0 overflow-hidden">
          {children[0]}
        </div>
        <ResizeHandle active={dragging === 0} onPointerDown={() => setDragging(0)} />
        <div style={{ width: widths[1] }} className="min-h-0 shrink-0 overflow-hidden">
          {children[1]}
        </div>
        <ResizeHandle active={dragging === 1} onPointerDown={() => setDragging(1)} />
        <div style={{ width: widths[2], minWidth: MIN_COLUMN_WIDTH }} className="min-h-0 shrink-0 overflow-hidden">
          {children[2]}
        </div>
      </div>
    </div>
  );
}

function ResizeHandle({ onPointerDown, active }: { onPointerDown: () => void; active: boolean }) {
  return (
    <div
      role="separator"
      onPointerDown={(e) => {
        e.preventDefault();
        onPointerDown();
      }}
      className={cn(
        "w-1 shrink-0 cursor-col-resize bg-zinc-200 transition-colors hover:bg-zinc-400",
        active && "bg-zinc-500"
      )}
    />
  );
}

export { DEFAULT_COLUMN_WIDTHS };
