"use client";

import { CheckCircle2, Upload, X } from "lucide-react";
import { useState } from "react";

import { Button } from "@/components/ui/button";
import { grokAdminApi, type GrokImportAccountInput, type GrokImportError, type GrokImportResult } from "@/lib/grok-admin";

type Props = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** 已登录的管理员 Bearer token（导入端点鉴权用）。 */
  token: string;
  /** 导入成功后回调（父页刷新列表）。 */
  onImported?: () => void;
};

/** 解析粘贴文本：JSON 数组 或 每行一个 JSON 对象 → 逐条输入。 */
function parseRows(text: string): unknown[] {
  const trimmed = text.trim();
  const parsed = JSON.parse(trimmed);
  if (Array.isArray(parsed)) return parsed;
  return trimmed
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean)
    .map((line) => JSON.parse(line));
}

/** 粗校验单条输入字段（identity_key/provider 必填；非法行由后端逐条报错）。 */
function toImportItem(row: unknown): { ok: true; item: unknown } | { ok: false; reason: string } {
  if (typeof row !== "object" || row === null) return { ok: false, reason: "非 JSON 对象" };
  const record = row as Record<string, unknown>;
  if (typeof record.identity_key !== "string" || !record.identity_key.trim()) {
    return { ok: false, reason: "缺少 identity_key" };
  }
  if (typeof record.provider !== "string" || !record.provider.trim()) {
    return { ok: false, reason: "缺少 provider" };
  }
  return { ok: true, item: row };
}

/**
 * Grok 账号导入对话框：粘贴 JSON/JSONL → 前端解析/粗校验 → POST /admin/accounts/import
 * （原始 JSON 数组）→ 展示 imported/failed + 失败行明细 → 成功后回调父页刷新。
 */
export function GrokImportDialog({ open, onOpenChange, token, onImported }: Props) {
  const [raw, setRaw] = useState("");
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<GrokImportResult | null>(null);

  if (!open) return null;

  const reset = () => {
    setRaw("");
    setError("");
    setResult(null);
  };

  const close = () => {
    reset();
    onOpenChange(false);
  };

  const handleImport = async () => {
    setError("");
    setResult(null);
    const text = raw.trim();
    if (!text) {
      setError("请先粘贴账号数据（JSON 数组 或 每行一个 JSON 对象）");
      return;
    }
    let rows: unknown[];
    try {
      rows = parseRows(text);
    } catch (err) {
      setError(`解析失败：${err instanceof Error ? err.message : String(err)}`);
      return;
    }
    if (rows.length === 0) {
      setError("未解析到任何账号");
      return;
    }
    const invalidIndex = rows.findIndex((row) => !toImportItem(row).ok);
    if (invalidIndex !== -1) {
      const reason = (toImportItem(rows[invalidIndex]) as { reason: string }).reason;
      setError(`第 ${invalidIndex + 1} 条非法：${reason}`);
      return;
    }
    setBusy(true);
    try {
      const res = await grokAdminApi.importAccounts(
        token,
        rows as GrokImportAccountInput[],
      );
      setResult(res);
      if (res.failed === 0) {
        onImported?.();
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="fixed inset-0 z-[100] flex items-center justify-center bg-black/30 p-4 backdrop-blur-sm">
      <div className="neo-card w-full max-w-lg p-6">
        <div className="mb-4 flex items-start justify-between gap-3">
          <div>
            <h2 className="text-lg font-semibold text-[var(--neo-ink)]">导入 Grok 账号</h2>
            <p className="mt-1 text-sm text-[var(--neo-muted)]">
              支持 JSON 数组或每行一个 JSON 对象（
              {`{ "identity_key": "acc-1", "provider": "grok_web", "name": "可选", "priority": 0, "max_concurrent": 1 }`}
              ）
            </p>
          </div>
          <button
            type="button"
            className="rounded-lg p-1 text-[var(--neo-muted)] hover:bg-stone-100"
            onClick={close}
            aria-label="关闭"
          >
            <X className="size-5" />
          </button>
        </div>
        <textarea
          value={raw}
          onChange={(e) => {
            setRaw(e.target.value);
            setError("");
            setResult(null);
          }}
          rows={10}
          placeholder={`[\n  { "identity_key": "acc-1", "provider": "grok_web", "priority": 0, "max_concurrent": 1 }\n]`}
          className="neo-input w-full rounded-md px-3 py-2 font-mono text-xs leading-relaxed"
        />
        {error ? <p className="mt-2 text-sm text-rose-600">{error}</p> : null}
        {result ? (
          <div className="mt-4 rounded-lg border border-[var(--neo-border)] bg-[var(--neo-surface-muted)] px-3 py-2 text-xs text-[var(--neo-muted)]">
            <p className="flex items-center gap-1.5 font-medium text-[var(--neo-ink)]">
              <CheckCircle2 className="size-4 text-emerald-600" />
              导入完成：成功 {result.imported} 条，失败 {result.failed} 条
            </p>
            {result.errors.length > 0 ? (
              <ul className="mt-2 max-h-32 space-y-1 overflow-y-auto">
                {result.errors.map((e: GrokImportError) => (
                  <li key={e.index} className="text-rose-600">
                    第 {e.index + 1} 条：{e.reason}
                  </li>
                ))}
              </ul>
            ) : null}
          </div>
        ) : (
          <div className="mt-4 rounded-lg border border-[var(--neo-border)] bg-[var(--neo-surface-muted)] px-3 py-2 text-xs text-[var(--neo-muted)]">
            <p className="leading-relaxed">
              提交到
              <code className="mx-1 rounded bg-stone-200 px-1">POST /admin/accounts/import</code>
              （原始 JSON 数组）；identity_key 与 provider 必填，其余字段可选。
            </p>
          </div>
        )}
        <div className="mt-6 flex justify-end gap-2">
          <Button variant="ghost" size="sm" onClick={close} disabled={busy}>
            关闭
          </Button>
          <Button size="sm" onClick={() => void handleImport()} disabled={busy || !raw.trim()}>
            <Upload className="size-4" />
            {busy ? "导入中…" : "导入"}
          </Button>
        </div>
      </div>
    </div>
  );
}
