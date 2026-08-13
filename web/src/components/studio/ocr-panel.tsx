"use client";

import { useCallback, useRef, useState } from "react";
import { Copy, FileImage, Loader2, ScanText, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { grokApi } from "@/lib/grok-api";
import { estimateBase64Bytes, formatBytes } from "@/lib/chat-conversations";
import { cn } from "@/lib/utils";

/** OCR 最大图片原始字节（G1 网关 8 图/64MiB 上限，单图也按其总量校验）。 */
const OCR_MAX_IMAGE_BYTES = 64 << 20;

type Props = {
  /** 关闭回调（宿主对话框使用）。 */
  onClose?: () => void;
};

/**
 * Grok OCR 面板（G7-P2）：图片选择/粘贴 → 「提取文字」→ 文本展示（loading/error 态）。
 * 经 TNexus `/api/grok/v1` 代理到 grok2api-rs 的 `/v1/chat/completions`（带图附件走 OCR 路径）。
 */
export function OcrPanel({ onClose }: Props) {
  const [dataUrl, setDataUrl] = useState<string | null>(null);
  const [fileName, setFileName] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const [text, setText] = useState("");
  const [copied, setCopied] = useState(false);
  const fileRef = useRef<HTMLInputElement>(null);

  const readImage = useCallback((file: File) => {
    if (!file.type.startsWith("image/")) {
      setError("请选择图片文件");
      return;
    }
    const reader = new FileReader();
    reader.onload = () => {
      const url = reader.result as string;
      if (estimateBase64Bytes(url) > OCR_MAX_IMAGE_BYTES) {
        setError(
          `图片过大（${file.name}，> ${formatBytes(OCR_MAX_IMAGE_BYTES)}），请压缩后重试`,
        );
        return;
      }
      setDataUrl(url);
      setFileName(file.name);
      setError("");
      setText("");
    };
    reader.onerror = () => setError("读取图片失败");
    reader.readAsDataURL(file);
  }, []);

  const onPaste = useCallback(
    (e: React.ClipboardEvent) => {
      const item = Array.from(e.clipboardData.items).find((i) => i.type.startsWith("image/"));
      const file = item?.getAsFile();
      if (file) {
        e.preventDefault();
        readImage(file);
      }
    },
    [readImage],
  );

  const extract = useCallback(async () => {
    if (!dataUrl || loading) return;
    setLoading(true);
    setError("");
    setCopied(false);
    try {
      const result = await grokApi.extractText(dataUrl);
      setText(result.trim() || "（空）");
    } catch (err) {
      setError(err instanceof Error ? err.message : "识别失败");
    } finally {
      setLoading(false);
    }
  }, [dataUrl, loading]);

  const copyText = useCallback(async () => {
    if (!text) return;
    try {
      await navigator.clipboard.writeText(text);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      setCopied(false);
    }
  }, [text]);

  return (
    <div
      className="flex w-full max-w-xl flex-col gap-4 rounded-xl border border-[var(--neo-border)] bg-[var(--neo-surface-raised)] p-5 shadow-lg"
      onPaste={onPaste}
    >
      <div className="flex items-center justify-between">
        <h2 className="flex items-center gap-2 text-sm font-medium text-[var(--neo-foreground)]">
          <ScanText className="h-4 w-4" aria-hidden />
          Grok OCR 提取文字
        </h2>
        {onClose && (
          <Button variant="ghost" size="sm" onClick={onClose} aria-label="关闭">
            ×
          </Button>
        )}
      </div>

      {/* 图片预览 / 空态 */}
      <div className="flex min-h-36 items-center justify-center rounded-lg border border-dashed border-[var(--neo-border)] bg-[var(--neo-surface)]">
        {dataUrl ? (
          <div className="flex flex-col items-center gap-2 p-2">
            <img
              src={dataUrl}
              alt="待识别图片"
              className="max-h-56 max-w-full rounded-md object-contain"
            />
            <span className="text-xs text-[var(--neo-muted-foreground)]">
              {fileName ?? "粘贴的图片"}
              {dataUrl ? ` · ${formatBytes(estimateBase64Bytes(dataUrl))}` : ""}
            </span>
          </div>
        ) : (
          <div className="flex flex-col items-center gap-2 p-6 text-center text-sm text-[var(--neo-muted-foreground)]">
            <FileImage className="h-6 w-6" aria-hidden />
            <span>选择图片文件，或在面板内<strong>粘贴</strong>（Ctrl+V）</span>
          </div>
        )}
      </div>

      {/* 操作行 */}
      <div className="flex flex-wrap items-center gap-2">
        <input
          ref={fileRef}
          type="file"
          accept="image/*"
          className="hidden"
          onChange={(e) => {
            const f = e.target.files?.[0];
            if (f) readImage(f);
            e.target.value = "";
          }}
        />
        <Button
          variant="outline"
          size="sm"
          onClick={() => fileRef.current?.click()}
          disabled={loading}
        >
          选择图片
        </Button>
        {dataUrl && (
          <Button
            variant="ghost"
            size="sm"
            onClick={() => {
              setDataUrl(null);
              setFileName(null);
              setText("");
              setError("");
            }}
            disabled={loading}
          >
            <Trash2 className="h-4 w-4" aria-hidden />
            清除
          </Button>
        )}
        <Button
          size="sm"
          className="ml-auto"
          onClick={() => void extract()}
          disabled={!dataUrl || loading}
        >
          {loading && <Loader2 className="mr-2 h-4 w-4 animate-spin" aria-hidden />}
          {loading ? "识别中…" : "提取文字"}
        </Button>
      </div>

      {error && (
        <p className="text-xs text-red-500" role="alert">
          {error}
        </p>
      )}

      {/* 结果文本 */}
      {text && (
        <div className="flex flex-col gap-2">
          <div className="flex items-center justify-between">
            <span className="text-xs text-[var(--neo-muted-foreground)]">识别结果</span>
            <Button
              variant="ghost"
              size="sm"
              onClick={() => void copyText()}
              className="h-6 px-2 text-xs"
            >
              <Copy className="mr-1 h-3 w-3" aria-hidden />
              {copied ? "已复制" : "复制"}
            </Button>
          </div>
          <pre
            className={cn(
              "max-h-72 overflow-auto whitespace-pre-wrap rounded-md border border-[var(--neo-border)]",
              "bg-[var(--neo-surface)] p-3 text-sm leading-relaxed text-[var(--neo-foreground)]",
            )}
          >
            {text}
          </pre>
        </div>
      )}
    </div>
  );
}