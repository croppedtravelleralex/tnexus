"use client";

import { LoaderCircle, Plus, RefreshCw, X } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { PageShell } from "@/components/admin/page-shell";
import { GrokTabs } from "@/components/grok/grok-tabs";
import { GrokTokenGate } from "@/components/grok/grok-token-gate";
import { Button } from "@/components/ui/button";
import { Input, Label } from "@/components/ui/input";
import { grokAdminApi, type GrokSettingsView } from "@/lib/grok-admin";

function fmtTime(value: string): string {
  const d = new Date(value);
  if (Number.isNaN(d.getTime())) return value;
  return d.toLocaleString("zh-CN", { hour12: false });
}

function SettingsContent({ token }: { token: string }) {
  const [view, setView] = useState<GrokSettingsView | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  // 编辑状态：key → 编辑中的 value（未编辑的条目不提交，避免误改）。
  const [edits, setEdits] = useState<Record<string, string>>({});
  const [addKey, setAddKey] = useState("");
  const [addValue, setAddValue] = useState("");
  const [saving, setSaving] = useState(false);

  const load = useCallback(async (currentToken: string) => {
    setLoading(true);
    setError("");
    try {
      const data = await grokAdminApi.getSettings(currentToken);
      setView(data);
      setEdits({});
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
      setView(null);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    const timer = setTimeout(() => void load(token), 0);
    return () => clearTimeout(timer);
  }, [token, load]);

  const handleSave = async () => {
    if (!view) return;
    // 只提交有编辑值的条目 + 新加条目。
    const values: Record<string, string> = { ...view.values };
    for (const [key, value] of Object.entries(edits)) {
      values[key] = value;
    }
    if (addKey.trim()) {
      values[addKey.trim()] = addValue;
    }
    setSaving(true);
    setError("");
    try {
      const updated = await grokAdminApi.putSettings(token, { values });
      setView(updated);
      setEdits({});
      setAddKey("");
      setAddValue("");
    } catch (err) {
      setError(err instanceof Error ? err.message : "保存设置失败");
    } finally {
      setSaving(false);
    }
  };

  const keys = Object.keys(view?.values ?? {});
  const mergedKeys = Array.from(new Set([...keys, ...Object.keys(edits)]));
  const dirtyCount =
    Object.keys(edits).length + (addKey.trim() ? 1 : 0);

  return (
    <div className="flex flex-col gap-3">
      <div className="flex flex-wrap items-center justify-between gap-2 text-xs text-[var(--neo-muted)]">
        <span>
          {view
            ? `版本 v${view.version} · 更新于 ${fmtTime(view.updated_at)}`
            : "设置未加载"}
          {dirtyCount > 0 ? ` · ${dirtyCount} 项待保存` : ""}
        </span>
        <div className="flex items-center gap-2">
          <Button variant="outline" size="sm" onClick={() => void load(token)} disabled={loading}>
            {loading ? <LoaderCircle className="size-4 animate-spin" /> : <RefreshCw className="size-4" />}
            刷新
          </Button>
          <Button size="sm" onClick={() => void handleSave()} disabled={saving || dirtyCount === 0}>
            {saving ? <LoaderCircle className="size-4 animate-spin" /> : null}
            保存
          </Button>
        </div>
      </div>
      {error ? <p className="text-sm text-rose-600">{error}</p> : null}
      {loading && !view ? (
        <div className="flex items-center justify-center gap-2 py-16 text-sm text-[var(--neo-muted)]">
          <LoaderCircle className="size-4 animate-spin" /> 加载中…
        </div>
      ) : view && mergedKeys.length === 0 ? (
        <div className="flex flex-col items-center justify-center gap-2 py-16 text-sm text-[var(--neo-muted)]">
          <span>暂无配置项</span>
          <span className="text-xs opacity-70">（grok-admin 返回空设置；可用下方表单新增）</span>
        </div>
      ) : (
        <div className="neo-card overflow-x-auto">
          <table className="w-full min-w-[560px] border-collapse text-left text-sm">
            <thead>
              <tr className="border-b border-[var(--neo-border)] text-[11px] uppercase tracking-wide text-[var(--neo-muted)]">
                <th className="px-3 py-2 font-medium">键</th>
                <th className="px-3 py-2 font-medium">值</th>
                <th className="px-3 py-2 font-medium">操作</th>
              </tr>
            </thead>
            <tbody>
              {mergedKeys.map((key) => {
                const original = view?.values[key];
                const edited = edits[key];
                return (
                  <tr key={key} className="border-b border-[var(--neo-border)] last:border-0">
                    <td className="max-w-[240px] truncate px-3 py-2 font-medium text-[var(--neo-ink)]" title={key}>
                      {key}
                    </td>
                    <td className="px-3 py-2">
                      <Input
                        value={edited ?? original ?? ""}
                        placeholder={original ?? "（新键）"}
                        onChange={(e) => setEdits((prev) => ({ ...prev, [key]: e.target.value }))}
                        className="h-8 text-sm"
                      />
                    </td>
                    <td className="px-3 py-2">
                      <button
                        type="button"
                        className="rounded-md p-1 text-[var(--neo-muted)] hover:bg-stone-100 hover:text-rose-600"
                        aria-label={`移除编辑项 ${key}`}
                        onClick={() => {
                          setEdits((prev) => {
                            const next = { ...prev };
                            delete next[key];
                            return next;
                          });
                          // 若为新增键（不在原值中），一并移除。
                          if (original === undefined) {
                            setView((prev) =>
                              prev
                                ? { ...prev, values: { ...prev.values, [key]: "" } }
                                : prev,
                            );
                          }
                        }}
                      >
                        <X className="size-4" />
                      </button>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}
      <div className="neo-card flex flex-col gap-3 p-4">
        <h3 className="text-sm font-semibold text-[var(--neo-ink)]">新增配置项</h3>
        <div className="flex flex-wrap items-end gap-2">
          <div className="space-y-1">
            <Label htmlFor="settings-new-key">键</Label>
            <Input
              id="settings-new-key"
              value={addKey}
              onChange={(e) => setAddKey(e.target.value)}
              placeholder="如 max_concurrent"
              className="w-56"
            />
          </div>
          <div className="flex-1 space-y-1">
            <Label htmlFor="settings-new-value">值</Label>
            <Input
              id="settings-new-value"
              value={addValue}
              onChange={(e) => setAddValue(e.target.value)}
              placeholder="如 8"
            />
          </div>
          <Button
            variant="outline"
            size="sm"
            onClick={() => {
              setEdits((prev) =>
                addKey.trim() ? { ...prev, [addKey.trim()]: addValue } : prev,
              );
              setAddKey("");
              setAddValue("");
            }}
            disabled={!addKey.trim()}
          >
            <Plus className="size-4" />
            加入列表
          </Button>
        </div>
      </div>
    </div>
  );
}

export default function GrokSettingsPage() {
  return (
    <PageShell title="Grok 设置" subtitle="全局配置 KV（读取 / 编辑 / 保存）" badge="G4-P2">
      <GrokTabs />
      <GrokTokenGate>{(token) => <SettingsContent token={token} />}</GrokTokenGate>
    </PageShell>
  );
}
