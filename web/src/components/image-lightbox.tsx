"use client";

import { useCallback, useEffect, useState } from "react";
import { ChevronLeft, ChevronRight, Download, X } from "lucide-react";
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

export function ImageLightbox({ images, currentIndex, open, onOpenChange, onIndexChange }: Props) {
  const current = images[currentIndex];

  const goPrev = useCallback(() => {
    if (currentIndex > 0) onIndexChange(currentIndex - 1);
  }, [currentIndex, onIndexChange]);

  const goNext = useCallback(() => {
    if (currentIndex < images.length - 1) onIndexChange(currentIndex + 1);
  }, [currentIndex, images.length, onIndexChange]);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onOpenChange(false);
      if (e.key === "ArrowLeft") goPrev();
      if (e.key === "ArrowRight") goNext();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onOpenChange, goPrev, goNext]);

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
        </div>
        <div className="flex items-center gap-2">
          <Button size="sm" variant="toolbar" className="h-8 text-white hover:bg-white/10" onClick={() => void onDownload()}>
            <Download className="size-4" />
            下载
          </Button>
          <button type="button" className="rounded-lg p-2 hover:bg-white/10" onClick={() => onOpenChange(false)}>
            <X className="size-5" />
          </button>
        </div>
      </div>
      <div className="relative flex min-h-0 flex-1 items-center justify-center px-12" onClick={(e) => e.stopPropagation()}>
        {currentIndex > 0 ? (
          <button type="button" className="absolute left-4 rounded-full bg-white/10 p-2 text-white hover:bg-white/20" onClick={goPrev}>
            <ChevronLeft className="size-6" />
          </button>
        ) : null}
        {/* eslint-disable-next-line @next/next/no-img-element */}
        <img src={current.src} alt="" className="max-h-[calc(100vh-8rem)] max-w-full object-contain" />
        {currentIndex < images.length - 1 ? (
          <button type="button" className="absolute right-4 rounded-full bg-white/10 p-2 text-white hover:bg-white/20" onClick={goNext}>
            <ChevronRight className="size-6" />
          </button>
        ) : null}
      </div>
    </div>
  );
}
