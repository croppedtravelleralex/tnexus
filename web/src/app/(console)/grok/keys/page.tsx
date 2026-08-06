"use client";

import { Copy, LoaderCircle, Plus, RefreshCw } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { ElevatedCard, PageShell } from "@/components/admin/page-shell";
import { GrokTabs } from "@/components/grok/grok-tabs";
import { GrokTokenGate } from "@/components/grok/grok-token-gate";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input, Label } from "@/components/ui/input";
import { grokAdminApi, type GrokClientKey } from "@/lib/grok-admin";

const PAGE_SIZE = 100;

function fmtTime(value: string | null | undefined): string {
  if (!value) return "—";
  const d = new Date(value);
  if (Number.isNaN(d.getTime())) return value;
  return d.toLocaleString("zh-CN", { hour12: false });
}

function KeysTable({
  items,
  onToggle,
  onDelete,
}: {
  items: GrokClientKey[];
  onToggle: (key: GrokClientKey) => void;
  onDelete: (key: GrokClientKey) => void;
}) {
  if (items.length === 0) {
    return (
      <div className="flex flex-col items-center justify-center gap-2 py-16 text-sm text-[var(--neo-muted)]">
        <span>暂无客户端密钥</span>
        <span className="text-xs opacity-70">（grok-admin 返回空列表或未部署密钥数据）</span>
      </div>
    );
  }
  return (
    <div className="neo-card overflow-x-auto">
      <table className="w-full min-w-[720px] border-collapse text-left text-sm">
        <thead>
          <tr className="border-b border-[var(--neo-border)] text-[11px] uppercase tracking-wide text-[var(--neo-muted)]">
            <th className="px-3 py-2 font-medium">ID</th>
            <th className="px-3 py-2 font-medium">名称</th>
            <th className="px-3 py-2 font-medium">前缀</th>
            <th className="px-3 py-2 font-medium">状态</th>
            <th className="px-3 py-2 font-medium">创建于</th>
            <th className="px-3 py-2 font-medium">最近使用</th>
            <th className="px-3 py-2 font-medium">操作</th>
          </tr>
        </thead>
        <tbody>
          {items.map((key) => (
            <tr
              key={key.id}
              className="border-b border-[var(--neo-border)] last:border-0 hover:bg-[var(--neo-surface-muted)]"
            >
              <td className="px-3 py-2 tabular-nums text-[var(--neo-muted)]">{key.id}</td>
              <td className="max-w-[200px] truncate px-3 py-2 font-medium text-[var(--neo-ink)]" title={key.name}>
                {key.name || "—"}
              </td>
              <td className="px-3 py-2">
                <code className="rounded-md bg-[var(--neo-surface-muted)] px-1.5 py-0.5 font-mono text-xs text-[var(--neo-muted)]">
                  {key.prefix}…
                </code>
              </td>
              <td className="px-3 py-2">
                <Badge variant={key.enabled ? "success" : "muted"}>
                  {key.enabled ? "启用" : "停用"}
                </Badge>
              </td>
              <td className="px-3 py-2 whitespace-nowrap text-xs text-[var(--neo-muted)]">
                {fmtTime(key.created_at)}
              </td>
              <td className="px-3 py-2 whitespace-nowrap text-xs text-[var(--neo-muted)]">
                {fmtTime(key.last_used_at)}
              </td>
              <td className="px-3 py-2">
                <div className="flex items-center gap-1.5">
                  <button
                    type="button"
                    role="switch"
                    aria-checked={key.enabled}
                    className={`relative h-5 w-9 rounded-full transition-colors ${
                      key.enabled ? "bg-pink-500" : "bg-stone-300"
                    }`}
                    onClick={() => onToggle(key)}
                    title={key.enabled ? "停用" : "启用"}
                  >
                    <span
                      className={`absolute top-0.5 size-4 rounded-full bg-white transition-all ${
                        key.enabled ? "left-[18px]" : "left-0.5"
                      }`}
                    />
                  </button>
                  <button
                    type="button"
                    className="rounded-md border border-[var(--neo-border)] px-2 py-1 text-xs text-[var(--neo-muted)] hover:bg-rose-50 hover:text-rose-700"
                    onClick={() => onDelete(key)}
                  >
                    删除
                  </button>
                </div>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function CreateKeyDialog({
  open,
  token,
  onOpenChange,
  onSaved,
}: {
  open: boolean;
  token: string;
  onOpenChange: (open: boolean) => void;
  onSaved: () => void;
}) {
  const [name, setName] = useState("");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");
  /** 创建成功后一次性显示的明文 secret。 */
  const [createdSecret, setCreatedSecret] = useState<string | null>(null);

  useEffect(() => {
    if (!open) return;
    // 微任务重置：避免 effect 内同步 setState（react-compiler 规则）。
    queueMicrotask(() => {
      setName("");
      setError("");
      setCreatedSecret(null);
    });
  }, [open]);

  if (!open) return null;

  const handleCreate = async () => {
    if (!name.trim()) {
      setError("名称不能为空");
      return;
    }
    setSaving(true);
    setError("");
    try {
      const result = await grokAdminApi.createKey(token, { name: name.trim() });
      setCreatedSecret(result.secret);
      onSaved();
    } catch (err) {
      setError(err instanceof Error ? err.message : "创建密钥失败");
    } finally {
      setSaving(false);
    }
  };

  const copySecret = () => {
    if (!createdSecret) return;
    void navigator.clipboard?.writeText(createdSecret);
  };

  return (
    <div className="fixed inset-0 z-[100] flex items-center justify-center bg-black/30 p-4 backdrop-blur-sm">
      <div className="neo-card w-full max-w-md p-6">
        <h2 className="text-lg font-semibold text-[var(--neo-ink)]">创建客户端密钥</h2>
        <p className="mt-1 text-sm text-[var(--neo-muted)]">POST /admin/client-keys</p>
        <div className="mt-4 space-y-4">
          <div className="space-y-2">
            <Label htmlFor="key-name">名称</Label>
            <Input
              id="key-name"
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="如 web-gateway"
              disabled={createdSecret !== null}
            />
          </div>
          {createdSecret ? (
            <div className="rounded-lg border border-emerald-200 bg-emerald-50 p-3">
              <p className="text-xs font-medium text-emerald-800">
                密钥仅显示一次，请立即保存：
              </p>
              <div className="mt-2 flex items-center gap-2">
                <code className="min-w-0 flex-1 truncate rounded-md bg-white px-2 py-1 font-mono text-xs text-[var(--neo-ink)]">
                  {createdSecret}
                </code>
                <Button size="sm" variant="outline" onClick={copySecret}>
                  <Copy className="size-3.5" />
                  复制
                </Button>
              </div>
            </div>
          ) : null}
          {error ? <p className="text-sm text-rose-600">{error}</p> : null}
        </div>
        <div className="mt-6 flex justify-end gap-2">
          <Button variant="ghost" size="sm" onClick={() => onOpenChange(false)} disabled={saving}>
            {createdSecret ? "关闭" : "取消"}
          </Button>
          {createdSecret ? null : (
            <Button size="sm" onClick={() => void handleCreate()} disabled={saving || !name.trim()}>
              {saving ? <LoaderCircle className="size-4 animate-spin" /> : null}
              创建
            </Button>
          )}
        </div>
      </div>
    </div>
  );
}

function KeysContent({ token }: { token: string }) {
  const [items, setItems] = useState<GrokClientKey[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const [createOpen, setCreateOpen] = useState(false);

  const load = useCallback(async (currentToken: string) => {
    setLoading(true);
    setError("");
    try {
      const data = await grokAdminApi.listKeys(currentToken, { page: 1, pageSize: PAGE_SIZE });
      setItems(data.items ?? []);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
      setItems([]);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    const timer = setTimeout(() => void load(token), 0);
    return () => clearTimeout(timer);
  }, [token, load]);

  const handleToggle = useCallback(async (key: GrokClientKey, token: string) => {
    try {
      await grokAdminApi.updateKey(token, key.id, { enabled: !key.enabled });
      setItems((prev) =>
        prev.map((k) => (k.id === key.id ? { ...k, enabled: !key.enabled } : k)),
      );
    } catch (err) {
      setError(err instanceof Error ? err.message : "更新密钥失败");
    }
  }, []);

  const handleDelete = useCallback(async (key: GrokClientKey, token: string) => {
    if (!window.confirm(`删除密钥「${key.name || key.id}」？此操作不可撤销。`)) return;
    try {
      await grokAdminApi.deleteKey(token, key.id);
      setItems((prev) => prev.filter((k) => k.id !== key.id));
    } catch (err) {
      setError(err instanceof Error ? err.message : "删除密钥失败");
    }
  }, []);

  return (
    <div className="flex flex-col gap-3">
            <div className="flex flex-wrap items-center justify-between gap-2 text-xs text-[var(--neo-muted)]">
              <span>共 {items.length} 个密钥</span>
              <Button size="sm" onClick={() => setCreateOpen(true)} disabled={loading}>
                <Plus className="size-4" />
                创建密钥
              </Button>
            </div>
            {error ? <p className="text-sm text-rose-600">{error}</p> : null}
            {loading && items.length === 0 ? (
              <div className="flex items-center justify-center gap-2 py-16 text-sm text-[var(--neo-muted)]">
                <LoaderCircle className="size-4 animate-spin" /> 加载中…
              </div>
            ) : (
              <KeysTable
                items={items}
                onToggle={(key) => void handleToggle(key, token)}
                onDelete={(key) => void handleDelete(key, token)}
              />
            )}
            <div className="flex items-center gap-2 text-xs text-[var(--neo-muted)]">
              <Button variant="outline" size="sm" onClick={() => void load(token)} disabled={loading}>
                {loading ? <LoaderCircle className="size-4 animate-spin" /> : <RefreshCw className="size-4" />}
                刷新
              </Button>
            </div>
      <CreateKeyDialog
        open={createOpen}
        token={token}
        onOpenChange={setCreateOpen}
        onSaved={() => void load(token)}
      />
    </div>
  );
}

export default function GrokKeysPage() {
  return (
    <PageShell title="Grok 密钥" subtitle="客户端密钥管理（列表 / 创建 / 启停 / 删除）" badge="G4-P2">
      <GrokTabs />
      <GrokTokenGate>{(token) => <KeysContent token={token} />}</GrokTokenGate>
    </PageShell>
  );
}
