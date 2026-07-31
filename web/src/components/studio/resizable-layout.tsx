"use client";

import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import {
  DEFAULT_COLUMN_WIDTHS,
  MIN_COLUMN_WIDTH,
  columnWidthsFromRatios,
  defaultColumnWidths,
  loadSavedColumnRatios,
} from "@/lib/studio-layout";
import { cn } from "@/lib/utils";

type Props = {
  widths: [number, number, number] | null;
  onWidthsChange: (w: [number, number, number]) => void;
  children: [React.ReactNode, React.ReactNode, React.ReactNode];
};

export function ResizableStudioLayout({ widths, onWidthsChange, children }: Props) {
  const containerRef = useRef<HTMLDivElement>(null);
  const [dragging, setDragging] = useState<0 | 1 | null>(null);
  const hydrated = useRef(false);

  useLayoutEffect(() => {
    if (hydrated.current || !containerRef.current) return;
    const total = containerRef.current.getBoundingClientRect().width - 8;
    if (total <= 0) return;
    const saved = loadSavedColumnRatios();
    onWidthsChange(saved ? columnWidthsFromRatios(total, saved) : defaultColumnWidths(total));
    hydrated.current = true;
  }, [onWidthsChange]);

  const activeWidths = widths ?? DEFAULT_COLUMN_WIDTHS;

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
        const mid = Math.max(MIN_COLUMN_WIDTH, Math.min(activeWidths[1], rest - MIN_COLUMN_WIDTH));
        onWidthsChange([left, mid, rest - mid]);
      } else {
        const leftPlusMid = Math.max(activeWidths[0] + MIN_COLUMN_WIDTH, Math.min(x - 4, total - MIN_COLUMN_WIDTH));
        const left = activeWidths[0];
        const mid = leftPlusMid - left;
        const right = total - leftPlusMid;
        onWidthsChange([left, Math.max(MIN_COLUMN_WIDTH, mid), Math.max(MIN_COLUMN_WIDTH, right)]);
      }
    },
    [onWidthsChange, activeWidths],
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
      <div ref={containerRef} className="flex min-h-0 flex-1">
        <div style={{ width: activeWidths[0] }} className="min-h-0 shrink-0 overflow-hidden">
          {children[0]}
        </div>
        <ResizeHandle active={dragging === 0} onPointerDown={() => setDragging(0)} />
        <div style={{ width: activeWidths[1] }} className="min-h-0 shrink-0 overflow-hidden">
          {children[1]}
        </div>
        <ResizeHandle active={dragging === 1} onPointerDown={() => setDragging(1)} />
        <div style={{ width: activeWidths[2], minWidth: MIN_COLUMN_WIDTH }} className="min-h-0 shrink-0 overflow-hidden">
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
        "w-1 shrink-0 cursor-col-resize bg-[var(--neo-border)] transition-colors hover:bg-[var(--neo-primary)]",
        active && "bg-[var(--neo-primary-deep)]",
      )}
    />
  );
}

export { DEFAULT_COLUMN_WIDTHS };
