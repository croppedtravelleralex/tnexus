"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import { ChevronLeft, ChevronRight, Download, RotateCcw, X, ZoomIn, ZoomOut } from "lucide-react";
import { Button } from "@/components/ui/button";

export type LightboxImage = {
  id: string;
  src: string;
  sizeLabel?: string;
  dimensions?: string;
};

type Props = {
  images: LightboxImage[];
  currentIndex: number;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onIndexChange: (index: number) => void;
};

const MIN_SCALE = 0.05;
const MAX_SCALE = 20;

export function ImageLightbox({ images, currentIndex, open, onOpenChange, onIndexChange }: Props) {
  const current = images[currentIndex];
  const [scale, setScale] = useState(1);
  const [offset, setOffset] = useState({ x: 0, y: 0 });
  const dragging = useRef(false);
  const dragStart = useRef({ x: 0, y: 0, ox: 0, oy: 0 });
  const viewportRef = useRef<HTMLDivElement>(null);

  const resetTransform = useCallback(() => {
    setScale(1);
    setOffset({ x: 0, y: 0 });
  }, []);

  useEffect(() => {
    if (open) resetTransform();
  }, [open, currentIndex, resetTransform]);

  const goPrev = useCallback(() => {
    if (currentIndex > 0) onIndexChange(currentIndex - 1);
  }, [currentIndex, onIndexChange]);

  const goNext = useCallback(() => {
    if (currentIndex < images.length - 1) onIndexChange(currentIndex + 1);
  }, [currentIndex, images.length, onIndexChange]);

  const clampScale = (v: number) => Math.min(MAX_SCALE, Math.max(MIN_SCALE, v));

  const onWheel = useCallback((e: WheelEvent) => {
    e.preventDefault();
    const factor = e.deltaY < 0 ? 1.12 : 1 / 1.12;
    setScale((s) => clampScale(s * factor));
  }, []);

  useEffect(() => {
    const el = viewportRef.current;
    if (!open || !el) return;
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  }, [open, onWheel]);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onOpenChange(false);
      if (e.key === "ArrowLeft") goPrev();
      if (e.key === "ArrowRight") goNext();
      if (e.key === "+" || e.key === "=") setScale((s) => clampScale(s * 1.2));
      if (e.key === "-") setScale((s) => clampScale(s / 1.2));
      if (e.key === "0") resetTransform();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onOpenChange, goPrev, goNext, resetTransform]);

  const onPointerDown = (e: React.PointerEvent) => {
    if (e.button !== 0) return;
    dragging.current = true;
    dragStart.current = { x: e.clientX, y: e.clientY, ox: offset.x, oy: offset.y };
    (e.target as HTMLElement).setPointerCapture(e.pointerId);
  };

  const onPointerMove = (e: React.PointerEvent) => {
    if (!dragging.current) return;
    setOffset({
      x: dragStart.current.ox + (e.clientX - dragStart.current.x),
      y: dragStart.current.oy + (e.clientY - dragStart.current.y),
    });
  };

  const onPointerUp = (e: React.PointerEvent) => {
    dragging.current = false;
    try {
      (e.target as HTMLElement).releasePointerCapture(e.pointerId);
    } catch {
      /* ignore */
    }
  };

  if (!open || !current) return null;

  const onDownload = async () => {
    try {
      const res = await fetch(current.src);
      const blob = await res.blob();
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = `${current.id}.png`;
      a.click();
      URL.revokeObjectURL(url);
    } catch {
      window.open(current.src, "_blank");
    }
  };

  return (
    <div className="fixed inset-0 z-[70] flex flex-col bg-black/90" onClick={() => onOpenChange(false)}>
      <div className="flex items-center justify-between px-4 py-3 text-white" onClick={(e) => e.stopPropagation()}>
        <div className="text-sm text-white/80">
          {currentIndex + 1} / {images.length}
          {current.dimensions ? ` · ${current.dimensions}` : ""}
          {current.sizeLabel ? ` · ${current.sizeLabel}` : ""}
          {` · ${Math.round(scale * 100)}%`}
        </div>
        <div className="flex items-center gap-2">
          <Button
            size="sm"
            variant="toolbar"
            className="h-8 text-white hover:bg-white/10"
            onClick={() => setScale((s) => clampScale(s / 1.25))}
            title="缩小"
          >
            <ZoomOut className="size-4" />
          </Button>
          <Button
            size="sm"
            variant="toolbar"
            className="h-8 text-white hover:bg-white/10"
            onClick={() => setScale((s) => clampScale(s * 1.25))}
            title="放大"
          >
            <ZoomIn className="size-4" />
          </Button>
          <Button size="sm" variant="toolbar" className="h-8 text-white hover:bg-white/10" onClick={resetTransform} title="重置">
            <RotateCcw className="size-4" />
          </Button>
          <Button size="sm" variant="toolbar" className="h-8 text-white hover:bg-white/10" onClick={() => void onDownload()}>
            <Download className="size-4" />
            下载
          </Button>
          <button type="button" className="rounded-lg p-2 hover:bg-white/10" onClick={() => onOpenChange(false)}>
            <X className="size-5" />
          </button>
        </div>
      </div>
      <div
        ref={viewportRef}
        className="relative min-h-0 flex-1 cursor-grab overflow-hidden active:cursor-grabbing"
        onClick={(e) => e.stopPropagation()}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerCancel={onPointerUp}
      >
        {currentIndex > 0 ? (
          <button
            type="button"
            className="absolute left-4 top-1/2 z-10 -translate-y-1/2 rounded-full bg-white/10 p-2 text-white hover:bg-white/20"
            onClick={goPrev}
          >
            <ChevronLeft className="size-6" />
          </button>
        ) : null}
        <div className="flex h-full w-full items-center justify-center">
          {/* eslint-disable-next-line @next/next/no-img-element */}
          <img
            src={current.src}
            alt=""
            draggable={false}
            className="max-h-[calc(100vh-8rem)] max-w-none select-none object-contain"
            style={{
              transform: `translate(${offset.x}px, ${offset.y}px) scale(${scale})`,
              transformOrigin: "center center",
              transition: dragging.current ? "none" : "transform 0.05s ease-out",
            }}
          />
        </div>
        {currentIndex < images.length - 1 ? (
          <button
            type="button"
            className="absolute right-4 top-1/2 z-10 -translate-y-1/2 rounded-full bg-white/10 p-2 text-white hover:bg-white/20"
            onClick={goNext}
          >
            <ChevronRight className="size-6" />
          </button>
        ) : null}
      </div>
      <div className="px-4 py-2 text-center text-xs text-white/60" onClick={(e) => e.stopPropagation()}>
        滚轮缩放（0.05×–20×）· 左键拖拽平移 · 按 0 重置
      </div>
    </div>
  );
}
