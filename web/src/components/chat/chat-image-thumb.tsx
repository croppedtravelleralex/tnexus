"use client";

import { useEffect, useState } from "react";
import { Download, Expand } from "lucide-react";
import { Button } from "@/components/ui/button";
import { estimateBase64Bytes, formatBytes } from "@/lib/chat-conversations";

type Props = {
  b64: string;
  onOpen: () => void;
  /** 图片 MIME 类型（grok 生图可能返回 JPEG/WEBP；缺省 png 保持 gpt 面板兼容）。 */
  mime?: "png" | "jpeg" | "webp";
};

export function ChatImageThumb({ b64, onOpen, mime = "png" }: Props) {
  const src = `data:image/${mime};base64,${b64}`;
  const [dimensions, setDimensions] = useState<string | null>(null);
  const sizeLabel = formatBytes(estimateBase64Bytes(b64));

  useEffect(() => {
    const img = new Image();
    img.onload = () => setDimensions(`${img.naturalWidth}×${img.naturalHeight}`);
    img.src = src;
  }, [src]);

  const onDownload = async () => {
    try {
      const res = await fetch(src);
      const blob = await res.blob();
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = `chat-image-${Date.now()}.${mime}`;
      a.click();
      URL.revokeObjectURL(url);
    } catch {
      window.open(src, "_blank");
    }
  };

  return (
    <div className="group relative mt-2 inline-block max-w-full">
      <button
        type="button"
        className="block overflow-hidden rounded-lg border border-[var(--neo-border)] bg-white/50"
        onClick={onOpen}
        title="点击查看大图"
      >
        <img src={src} alt="生成图" className="max-h-80 max-w-full object-contain" />
      </button>
      <div className="mt-1 flex flex-wrap items-center gap-2 text-[10px] text-[var(--neo-muted)]">
        {dimensions ? <span>{dimensions}</span> : null}
        <span>{sizeLabel}</span>
        <Button
          type="button"
          size="sm"
          variant="ghost"
          className="h-6 gap-1 px-1.5 text-[10px]"
          onClick={onOpen}
        >
          <Expand className="size-3" />
          查看
        </Button>
        <Button
          type="button"
          size="sm"
          variant="ghost"
          className="h-6 gap-1 px-1.5 text-[10px]"
          onClick={() => void onDownload()}
        >
          <Download className="size-3" />
          下载
        </Button>
      </div>
    </div>
  );
}
