"use client";

import { Upload, X } from "lucide-react";
import { useState } from "react";

import { Button } from "@/components/ui/button";

type Props = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
};

/**
 * Grok 账号导入对话框。
 *
 * 现状：grok-admin 后端尚未提供批量创建/导入端点（G4-A1 对照表中
 * accounts 域导入为缺失项），因此本对话框只做粘贴/解析 UI 与格式校验，
 * 提交按钮在无端点时置灰并提示。待后端补 `POST /admin/accounts/import`
 * 后，在 `handleImport` 中接入即可（见 TODO）。
 */
export function GrokImportDialog({ open, onOpenChange }: Props) {
  const [raw, setRaw] = useState("");
  const [error, setError] = useState("");

  if (!open) return null;

  const handleParse = (): boolean => {
    setError("");
    const text = raw.trim();
    if (!text) {
      setError("请先粘贴账号数据（JSON 数组 或 每行一个 JSON 对象）");
      return false;
    }
    try {
      const parsed = JSON.parse(text);
      const rows = Array.isArray(parsed)
        ? parsed
        : text.split("\n").map((line) => line.trim()).filter(Boolean).map((line) => JSON.parse(line));
      if (rows.length === 0) {
        setError("未解析到任何账号");
        return false;
      }
      return true;
    } catch (err) {
      setError(`解析失败：${err instanceof Error ? err.message : String(err)}`);
      return false;
    }
  };

  return (
    <div className="fixed inset-0 z-[100] flex items-center justify-center bg-black/30 p-4 backdrop-blur-sm">
      <div className="neo-card w-full max-w-lg p-6">
        <div className="mb-4 flex items-start justify-between gap-3">
          <div>
            <h2 className="text-lg font-semibold text-[var(--neo-ink)]">导入 Grok 账号</h2>
            <p className="mt-1 text-sm text-[var(--neo-muted)]">
              支持 JSON 数组或每行一个 JSON 对象（{`{ "identity_key": "...", "provider": "grok_web", "encrypted_token": "..." }`}）
            </p>
          </div>
          <button
            type="button"
            className="rounded-lg p-1 text-[var(--neo-muted)] hover:bg-stone-100"
            onClick={() => onOpenChange(false)}
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
          }}
          rows={10}
          placeholder={`[\n  { "identity_key": "acc-1", "provider": "grok_web", "encrypted_token": "..." }\n]`}
          className="neo-input w-full rounded-md px-3 py-2 font-mono text-xs leading-relaxed"
        />
        {error ? <p className="mt-2 text-sm text-rose-600">{error}</p> : null}
        <div className="mt-4 rounded-lg border border-[var(--neo-border)] bg-[var(--neo-surface-muted)] px-3 py-2 text-xs text-[var(--neo-muted)]">
          <p className="font-medium text-[var(--neo-ink)]">TODO（后端未实现）</p>
          <p className="mt-1 leading-relaxed">
            grok-admin 尚无批量导入端点（G4-A1 缺失项）。实现
            <code className="mx-1 rounded bg-stone-200 px-1">POST /admin/accounts/import</code>
            后，在本对话框接入：解析结果 POST 到该端点并刷新列表。
          </p>
        </div>
        <div className="mt-6 flex justify-end gap-2">
          <Button variant="ghost" size="sm" onClick={() => onOpenChange(false)}>
            取消
          </Button>
          <Button
            size="sm"
            disabled
            title="后端导入端点尚未实现"
            onClick={() => {
              if (handleParse()) {
                // TODO: 接入 POST /admin/accounts/import 后启用提交
              }
            }}
          >
            <Upload className="size-4" />
            导入（待后端支持）
          </Button>
        </div>
      </div>
    </div>
  );
}