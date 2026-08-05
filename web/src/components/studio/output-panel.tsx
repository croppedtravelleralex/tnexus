"use client";

import { useEffect, useMemo, useRef, useState } from "react";
import type { JobResult } from "@/lib/api";
import { apiAssetUrl } from "@/lib/api";
import { formatBytes, formatResolution } from "@/lib/format-bytes";
import { formatDuration } from "@/lib/format-duration";
import { Download, Loader2, RotateCcw, ZoomIn } from "lucide-react";
import { downloadImage, ImagePreviewDialog } from "@/components/studio/image-preview-dialog";
import { cn } from "@/lib/utils";

export type OutputSlot = {
  id: string;
  status: "pending" | "success" | "failed";
  image?: JobResult;
  error?: string;
  label?: string;
  generationMs?: number;
};

function ImageMetaBadge({
  image,
  expectedWidth,
  expectedHeight,
  className,
}: {
  image?: JobResult;
  expectedWidth?: number;
  expectedHeight?: number;
  className?: string;
}) {
  const [probed, setProbed] = useState<{ width?: number; height?: number; sizeBytes?: number }>({});

  const width = image?.width ?? probed.width ?? expectedWidth;
  const height = image?.height ?? probed.height ?? expectedHeight;
  const sizeBytes = image?.size_bytes ?? probed.sizeBytes;

  useEffect(() => {
    if (!image) return;
    if (image.width && image.height && image.size_bytes) return;

    const url = apiAssetUrl(image.download_url || image.preview_url);
    if (!url || url.includes("/v1/images/assets/")) return;

    let cancelled = false;

    if (!image.size_bytes && !url.startsWith("data:")) {
      void fetch(url, { method: "HEAD", credentials: "include" })
        .then((res) => {
          const cl = res.headers.get("content-length");
          if (!cancelled && cl) {
            const parsed = Number.parseInt(cl, 10);
            if (Number.isFinite(parsed) && parsed > 0) {
              setProbed((prev) => ({ ...prev, sizeBytes: parsed }));
            }
          }
        })
        .catch(() => undefined);
    }

    if (!image.width || !image.height) {
      const img = new window.Image();
      img.onload = () => {
        if (!cancelled) {
          setProbed((prev) => ({
            ...prev,
            width: img.naturalWidth,
            height: img.naturalHeight,
          }));
        }
      };
      img.src = url;
    }

    return () => {
      cancelled = true;
    };
  }, [image]);

  const resolution = formatResolution(width, height);
  const sizeLabel = sizeBytes ? formatBytes(sizeBytes) : null;
  const label = [resolution, sizeLabel].filter(Boolean).join(" · ");
  if (!label) return null;

  return (
    <div
      className={cn(
        "rounded bg-black/55 px-1.5 py-0.5 text-[10px] font-medium leading-tight text-white backdrop-blur-sm",
        className,
      )}
    >
      {label}
    </div>
  );
}

function PendingTile({
  stageLabel,
  elapsedMs,
  progress,
  index,
  expectedWidth,
  expectedHeight,
}: {
  stageLabel: string;
  elapsedMs: number;
  progress: number;
  index: number;
  expectedWidth?: number;
  expectedHeight?: number;
}) {
  return (
    <div className="relative h-44 w-44 shrink-0 overflow-hidden rounded-lg border-2 border-dashed border-zinc-300 bg-zinc-50">
      {(expectedWidth || expectedHeight) && (
        <ImageMetaBadge
          expectedWidth={expectedWidth}
          expectedHeight={expectedHeight}
          className="absolute left-1.5 top-1.5 z-10"
        />
      )}
      <div
        className="absolute inset-0 opacity-50"
        style={{
          backgroundImage: "radial-gradient(circle, rgba(161,161,170,0.45) 1.4px, transparent 1.6px)",
          backgroundSize: "14px 14px",
        }}
      />
      <div className="absolute inset-0 flex flex-col items-center justify-center gap-2 p-3 text-center">
        <Loader2 className="h-6 w-6 animate-spin text-zinc-400" />
        <p className="text-base font-semibold tabular-nums text-zinc-700">{formatDuration(elapsedMs)}</p>
        <p className="text-[11px] text-zinc-500">
          {stageLabel || "生成中"}
          {index > 0 ? ` · #${index + 1}` : ""}
        </p>
        <div className="h-1 w-4/5 overflow-hidden rounded-full bg-zinc-200">
          <div
            className="h-full rounded-full bg-zinc-500 transition-all duration-500"
            style={{ width: `${Math.max(progress, 5)}%` }}
          />
        </div>
      </div>
    </div>
  );
}

function FailedTile({ error, onRetry }: { error?: string; onRetry?: () => void }) {
  return (
    <div className="flex h-44 w-44 shrink-0 flex-col overflow-hidden rounded-lg border-2 border-red-200 bg-red-50">
      <div className="flex flex-1 items-center justify-center p-4 text-center text-sm leading-relaxed text-red-600">
        {error || "生成失败"}
      </div>
      {onRetry && (
        <button
          type="button"
          onClick={onRetry}
          className="flex items-center justify-center gap-1 border-t border-red-200 py-2.5 text-sm font-medium text-red-600 hover:bg-red-100"
        >
          <RotateCcw className="h-4 w-4" />
          重试
        </button>
      )}
    </div>
  );
}

function SuccessTile({
  image,
  generationMs,
  onPreview,
}: {
  image: JobResult;
  generationMs?: number;
  onPreview: (url: string, downloadUrl?: string | null) => void;
}) {
  const preview = apiAssetUrl(image.preview_url || image.thumb_url);
  const lightbox = apiAssetUrl(image.download_url || image.preview_url || image.thumb_url);
  const download = apiAssetUrl(image.download_url || image.preview_url);

  return (
    <div className="group relative h-44 w-44 shrink-0 overflow-hidden rounded-lg border border-zinc-200 bg-zinc-100 shadow-sm">
      <ImageMetaBadge image={image} className="absolute left-1.5 top-1.5 z-10" />
      {generationMs != null && generationMs > 0 && (
        <div className="absolute bottom-1.5 left-1.5 z-10 rounded bg-black/55 px-1.5 py-0.5 text-[10px] font-medium tabular-nums text-white backdrop-blur-sm">
          {formatDuration(generationMs)}
        </div>
      )}
      {preview ? (
        <button
          type="button"
          className="h-full w-full cursor-zoom-in"
          onClick={() => onPreview(lightbox ?? preview, download)}
        >
          {/* eslint-disable-next-line @next/next/no-img-element */}
          <img src={preview} alt="" className="h-full w-full object-cover transition group-hover:scale-[1.02]" />
        </button>
      ) : (
        <div className="flex h-full items-center justify-center text-sm text-zinc-400">无预览</div>
      )}
      {preview && (
        <div className="absolute inset-x-0 bottom-0 flex justify-end gap-1 bg-gradient-to-t from-black/50 to-transparent p-2 opacity-0 transition group-hover:opacity-100">
          <button
            type="button"
            title="查看大图"
            onClick={() => onPreview(lightbox ?? preview, download)}
            className="rounded-md bg-white/90 p-1.5 text-zinc-800 shadow hover:bg-white"
          >
            <ZoomIn className="h-4 w-4" />
          </button>
          {download && (
            <button
              type="button"
              title="下载"
              onClick={(e) => {
                e.stopPropagation();
                downloadImage(download);
              }}
              className="rounded-md bg-white/90 p-1.5 text-zinc-800 shadow hover:bg-white"
            >
              <Download className="h-4 w-4" />
            </button>
          )}
        </div>
      )}
    </div>
  );
}

function ThumbnailRow({ children, count }: { children: React.ReactNode; count: number }) {
  const ref = useRef<HTMLDivElement>(null);

  return (
    <div
      ref={ref}
      className="scrollbar-hide flex gap-3 overflow-x-auto pb-1"
      onWheel={(e) => {
        if (!ref.current || count <= 1) return;
        e.preventDefault();
        ref.current.scrollLeft += e.deltaY;
      }}
    >
      {children}
    </div>
  );
}

type Props = {
  slots: OutputSlot[];
  results: JobResult[];
  busy: boolean;
  stageLabel: string;
  progress: number;
  elapsedMs: number;
  jobStatus: "idle" | "running" | "done" | "failed";
  error?: string;
  onRetry?: () => void;
  expectedWidth?: number;
  expectedHeight?: number;
};

export function OutputPanel({
  slots,
  results,
  busy,
  stageLabel,
  progress,
  elapsedMs,
  jobStatus,
  error,
  onRetry,
  expectedWidth,
  expectedHeight,
}: Props) {
  const [preview, setPreview] = useState<{ url: string; downloadUrl?: string | null } | null>(null);

  const effectiveSlots: OutputSlot[] = useMemo(() => {
    if (slots.length > 0) return slots;
    if (busy) return [{ id: "pending-0", status: "pending" }];
    if (results.length > 0) {
      return results.map((img) => ({ id: img.id, status: "success" as const, image: img }));
    }
    return [];
  }, [slots, busy, results]);

  const showGrid = effectiveSlots.length > 0;

  const statusBadge = () => {
    if (jobStatus === "running" || busy) {
      return (
        <span className="rounded-full bg-amber-50 px-2.5 py-0.5 text-xs font-medium text-amber-700">
          等待 {formatDuration(elapsedMs)}
        </span>
      );
    }
    if (jobStatus === "done") {
      return (
        <span className="rounded-full bg-sky-50 px-2.5 py-0.5 text-xs font-medium text-sky-600">
          成功 · {formatDuration(elapsedMs)}
        </span>
      );
    }
    if (jobStatus === "failed") {
      return (
        <span className="rounded-full bg-rose-50 px-2.5 py-0.5 text-xs font-medium text-rose-600">
          失败 · {formatDuration(elapsedMs)}
        </span>
      );
    }
    return null;
  };

  return (
    <>
      <div className="panel-card flex h-full min-h-0 flex-col">
        <div className="panel-header flex items-center justify-between text-zinc-900">
          <span>出图效果</span>
          <div className="flex items-center gap-2">
            {statusBadge()}
            {(busy || jobStatus === "running") && (
              <span className="text-xs font-normal text-zinc-400">
                {stageLabel} {progress}%
              </span>
            )}
          </div>
        </div>

        <div className="panel-body scrollbar-hide min-h-0 flex-1 overflow-y-auto">
          {jobStatus === "failed" && error && (
            <div className="mb-4 rounded-lg border border-red-200 bg-red-50 px-3 py-2 text-sm text-red-700">
              {error}
            </div>
          )}

          {showGrid ? (
            <div className="space-y-2">
              {!busy && effectiveSlots.some((s) => s.status === "success") && (
                <p className="text-xs text-zinc-500">点击缩略图放大查看，滚轮可缩放</p>
              )}
              <ThumbnailRow count={effectiveSlots.length}>
                {effectiveSlots.map((slot, index) => {
                  if (slot.status === "pending") {
                    return (
                      <PendingTile
                        key={slot.id}
                        index={index}
                        stageLabel={stageLabel}
                        elapsedMs={elapsedMs}
                        progress={progress}
                        expectedWidth={expectedWidth}
                        expectedHeight={expectedHeight}
                      />
                    );
                  }
                  if (slot.status === "failed") {
                    return (
                      <FailedTile key={slot.id} error={slot.error ?? error} onRetry={onRetry} />
                    );
                  }
                  if (slot.image) {
                    return (
                      <SuccessTile
                        key={slot.id}
                        image={slot.image}
                        generationMs={slot.generationMs ?? slot.image?.generation_ms ?? undefined}
                        onPreview={(url, downloadUrl) => setPreview({ url, downloadUrl })}
                      />
                    );
                  }
                  return (
                    <PendingTile
                      key={slot.id}
                      index={index}
                      stageLabel={stageLabel}
                      elapsedMs={elapsedMs}
                      progress={progress}
                      expectedWidth={expectedWidth}
                      expectedHeight={expectedHeight}
                    />
                  );
                })}
              </ThumbnailRow>
            </div>
          ) : (
            <div
              className={cn(
                "flex min-h-[280px] flex-col items-center justify-center rounded-xl border-2 border-dashed text-sm",
                busy ? "border-zinc-300 bg-zinc-50 text-zinc-600" : "border-zinc-200 text-zinc-400",
              )}
            >
              {busy ? (
                <>
                  <Loader2 className="mb-3 h-10 w-10 animate-spin text-zinc-400" />
                  <p className="text-base font-medium">{stageLabel || "生成中"}…</p>
                  <p className="mt-2 text-lg font-semibold tabular-nums text-zinc-700">
                    {formatDuration(elapsedMs)}
                  </p>
                  <p className="mt-1 text-xs text-[var(--neo-muted)]">生成结果将显示在此处</p>
                </>
              ) : (
                <p>生成完成后预览图将在此展示</p>
              )}
            </div>
          )}
        </div>
      </div>

      <ImagePreviewDialog
        open={!!preview}
        url={preview?.url ?? null}
        downloadUrl={preview?.downloadUrl}
        onClose={() => setPreview(null)}
      />
    </>
  );
}
