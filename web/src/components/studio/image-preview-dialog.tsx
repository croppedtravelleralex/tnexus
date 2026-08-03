"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import { Download, RotateCcw, X, ZoomIn, ZoomOut } from "lucide-react";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import { wheelZoomFactor, zoomAtPointer } from "@/lib/zoom-at-pointer";

type Props = {
  url: string | null;
  open: boolean;
  onClose: () => void;
  downloadUrl?: string | null;
  title?: string;
};

type Offset = { x: number; y: number };

export function downloadImage(url: string, filename = "tnexus-image.png") {
  const a = document.createElement("a");
  a.href = url;
  a.download = filename;
  a.rel = "noopener";
  document.body.appendChild(a);
  a.click();
  a.remove();
}

export function ImagePreviewDialog({ url, open, onClose, downloadUrl, title }: Props) {
  const [scale, setScale] = useState(1);
  const [offset, setOffset] = useState<Offset>({ x: 0, y: 0 });
  const [isDragging, setIsDragging] = useState(false);
  const viewportRef = useRef<HTMLDivElement>(null);
  const dragRef = useRef<{ x: number; y: number; offsetX: number; offsetY: number } | null>(null);

  const resetView = useCallback(() => {
    setScale(1);
    setOffset({ x: 0, y: 0 });
  }, []);

  useEffect(() => {
    if (open) resetView();
  }, [open, url, resetView]);

  useEffect(() => {
    if (scale <= 1 && (offset.x !== 0 || offset.y !== 0)) {
      setOffset({ x: 0, y: 0 });
    }
  }, [scale, offset.x, offset.y]);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  useEffect(() => {
    if (!open) return;

    const onMouseMove = (e: MouseEvent) => {
      if (!dragRef.current) return;
      setOffset({
        x: dragRef.current.offsetX + (e.clientX - dragRef.current.x),
        y: dragRef.current.offsetY + (e.clientY - dragRef.current.y),
      });
    };

    const stopDrag = () => {
      dragRef.current = null;
      setIsDragging(false);
    };

    window.addEventListener("mousemove", onMouseMove);
    window.addEventListener("mouseup", stopDrag);
    return () => {
      window.removeEventListener("mousemove", onMouseMove);
      window.removeEventListener("mouseup", stopDrag);
    };
  }, [open]);

  const onWheel = useCallback((e: React.WheelEvent) => {
    e.preventDefault();
    e.stopPropagation();
    const viewport = viewportRef.current;
    if (!viewport) return;
    const next = zoomAtPointer(
      e.clientX,
      e.clientY,
      viewport,
      scale,
      offset,
      wheelZoomFactor(e.deltaY, 1.12),
      0.25,
      5,
    );
    if (!next) return;
    setScale(next.scale <= 1 ? 1 : next.scale);
    setOffset(next.scale <= 1 ? { x: 0, y: 0 } : next.offset);
  }, [offset, scale]);

  const onMouseDown = useCallback(
    (e: React.MouseEvent) => {
      if (e.button !== 0 || scale <= 1) return;
      e.preventDefault();
      dragRef.current = {
        x: e.clientX,
        y: e.clientY,
        offsetX: offset.x,
        offsetY: offset.y,
      };
      setIsDragging(true);
    },
    [offset.x, offset.y, scale],
  );

  if (!open || !url) return null;

  const dl = downloadUrl || url;
  const canDrag = scale > 1;

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/80 p-4 backdrop-blur-sm"
      onClick={onClose}
      role="dialog"
      aria-modal="true"
    >
      <div
        className="relative flex h-[92vh] w-[92vw] flex-col overflow-hidden rounded-xl bg-zinc-950 shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex shrink-0 items-center justify-between gap-4 border-b border-zinc-800 px-4 py-2">
          <p className="truncate text-sm text-zinc-300">
            {title || "查看大图"} · 滚轮以指针为中心缩放
            {canDrag ? " · 按住拖拽" : ""} · {Math.round(scale * 100)}%
          </p>
          <div className="flex shrink-0 items-center gap-1">
            <Button
              variant="ghost"
              size="icon"
              className="text-zinc-300 hover:bg-zinc-800 hover:text-white"
              onClick={() => setScale((s) => Math.max(0.25, s - 0.25))}
            >
              <ZoomOut className="h-4 w-4" />
            </Button>
            <Button
              variant="ghost"
              size="icon"
              className="text-zinc-300 hover:bg-zinc-800 hover:text-white"
              onClick={resetView}
            >
              <RotateCcw className="h-4 w-4" />
            </Button>
            <Button
              variant="ghost"
              size="icon"
              className="text-zinc-300 hover:bg-zinc-800 hover:text-white"
              onClick={() => setScale((s) => Math.min(5, s + 0.25))}
            >
              <ZoomIn className="h-4 w-4" />
            </Button>
            <Button
              variant="ghost"
              size="sm"
              className="text-zinc-300 hover:bg-zinc-800 hover:text-white"
              onClick={() => downloadImage(dl)}
            >
              <Download className="mr-1 h-4 w-4" />
              下载
            </Button>
            <Button
              variant="ghost"
              size="icon"
              className="text-zinc-300 hover:bg-zinc-800 hover:text-white"
              onClick={onClose}
            >
              <X className="h-4 w-4" />
            </Button>
          </div>
        </div>
        <div
          ref={viewportRef}
          className={cn(
            "flex min-h-0 flex-1 items-center justify-center overflow-hidden",
            canDrag && (isDragging ? "cursor-grabbing" : "cursor-grab"),
          )}
          onWheel={onWheel}
          onMouseDown={onMouseDown}
        >
          <div
            className={cn("origin-center", !isDragging && "transition-transform duration-75")}
            style={{ transform: `translate(${offset.x}px, ${offset.y}px) scale(${scale})` }}
          >
            {/* eslint-disable-next-line @next/next/no-img-element */}
            <img
              src={url}
              alt=""
              draggable={false}
              className="max-h-[calc(92vh-3rem)] max-w-[92vw] select-none object-contain"
            />
          </div>
        </div>
      </div>
    </div>
  );
}
