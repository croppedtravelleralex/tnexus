"use client";

import { LoaderCircle, Plus, RefreshCw } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { ElevatedCard, PageShell } from "@/components/admin/page-shell";
import { GrokTabs } from "@/components/grok/grok-tabs";
import { GrokTokenGate } from "@/components/grok/grok-token-gate";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input, Label } from "@/components/ui/input";
import {
  grokAdminApi,
  type GrokModelBinding,
  type GrokModelRoute,
} from "@/lib/grok-admin";

const PAGE_SIZE = 100;
const PROVIDER_OPTIONS = [
  { value: "grok_build", label: "Build" },
  { value: "grok_web", label: "Web" },
  { value: "grok_console", label: "Console" },
];

function providerLabel(provider: string) {
  return PROVIDER_OPTIONS.find((o) => o.value === provider)?.label ?? provider;
}

function fmtTime(value: string | null | undefined): string {
  if (!value) return "—";
  const d = new Date(value);
  if (Number.isNaN(d.getTime())) return value;
  return d.toLocaleString("zh-CN", { hour12: false });
}

function ModelsTable({
  items,
  bindingsByModel,
  onToggle,
  onDelete,
}: {
  items: GrokModelRoute[];
  bindingsByModel: Record<string, number>;
  onToggle: (route: GrokModelRoute) => void;
  onDelete: (route: GrokModelRoute) => void;
}) {
  if (items.length === 0) {
    return (
      <div className="flex flex-col items-center justify-center gap-2 py-16 text-sm text-[var(--neo-muted)]">
        <span>暂无模型路由</span>
        <span className="text-xs opacity-70">（grok-admin 返回空列表或未部署模型数据）</span>
      </div>
    );
  }
  return (
    <div className="neo-card overflow-x-auto">
      <table className="w-full min-w-[820px] border-collapse text-left text-sm">
        <thead>
          <tr className="border-b border-[var(--neo-border)] text-[11px] uppercase tracking-wide text-[var(--neo-muted)]">
            <th className="px-3 py-2 font-medium">ID</th>
            <th className="px-3 py-2 font-medium">Provider</th>
            <th className="px-3 py-2 font-medium">上游模型</th>
            <th className="px-3 py-2 font-medium">别名</th>
            <th className="px-3 py-2 font-medium">绑定账号</th>
            <th className="px-3 py-2 font-medium">状态</th>
            <th className="px-3 py-2 font-medium">创建于</th>
            <th className="px-3 py-2 font-medium">更新于</th>
            <th className="px-3 py-2 font-medium">操作</th>
          </tr>
        </thead>
        <tbody>
          {items.map((route) => (
            <tr
              key={route.id}
              className="border-b border-[var(--neo-border)] last:border-0 hover:bg-[var(--neo-surface-muted)]"
            >
              <td className="px-3 py-2 tabular-nums text-[var(--neo-muted)]">{route.id}</td>
              <td className="px-3 py-2">{providerLabel(route.provider)}</td>
              <td className="max-w-[220px] truncate px-3 py-2 font-medium text-[var(--neo-ink)]" title={route.upstream_model}>
                {route.upstream_model || "—"}
              </td>
              <td className="max-w-[240px] px-3 py-2">
                <div className="flex flex-wrap gap-1">
                  {(route.aliases ?? []).length > 0 ? (
                    route.aliases.map((alias) => (
                      <span
                        key={alias}
                        className="rounded-md bg-[var(--neo-surface-muted)] px-1.5 py-0.5 text-xs text-[var(--neo-muted)]"
                      >
                        {alias}
                      </span>
                    ))
                  ) : (
                    <span className="text-xs text-[var(--neo-muted)]">—</span>
                  )}
                </div>
              </td>
              <td className="px-3 py-2 tabular-nums">{bindingsByModel[route.upstream_model] ?? 0}</td>
              <td className="px-3 py-2">
                <Badge variant={route.enabled ? "success" : "muted"}>
                  {route.enabled ? "启用" : "停用"}
                </Badge>
              </td>
              <td className="px-3 py-2 whitespace-nowrap text-xs text-[var(--neo-muted)]">
                {fmtTime(route.created_at)}
              </td>
              <td className="px-3 py-2 whitespace-nowrap text-xs text-[var(--neo-muted)]">
                {fmtTime(route.updated_at)}
              </td>
              <td className="px-3 py-2">
                <div className="flex items-center gap-1.5">
                  <button
                    type="button"
                    role="switch"
                    aria-checked={route.enabled}
                    className={`relative h-5 w-9 rounded-full transition-colors ${
                      route.enabled ? "bg-pink-500" : "bg-stone-300"
                    }`}
                    onClick={() => onToggle(route)}
                    title={route.enabled ? "停用" : "启用"}
                  >
                    <span
                      className={`absolute top-0.5 size-4 rounded-full bg-white transition-all ${
                        route.enabled ? "left-[18px]" : "left-0.5"
                      }`}
                    />
                  </button>
                  <button
                    type="button"
                    className="rounded-md border border-[var(--neo-border)] px-2 py-1 text-xs text-[var(--neo-muted)] hover:bg-rose-50 hover:text-rose-700"
                    onClick={() => onDelete(route)}
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

function NewModelDialog({
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
  const [provider, setProvider] = useState("grok_build");
  const [upstreamModel, setUpstreamModel] = useState("");
  const [aliases, setAliases] = useState("");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");

  useEffect(() => {
    if (!open) return;
    // 微任务重置：避免 effect 内同步 setState（react-compiler 规则）。
    queueMicrotask(() => {
      setProvider("grok_build");
      setUpstreamModel("");
      setAliases("");
      setError("");
    });
  }, [open]);

  if (!open) return null;

  const handleSave = async () => {
    const model = upstreamModel.trim();
    if (!model) {
      setError("上游模型名不能为空");
      return;
    }
    setSaving(true);
    setError("");
    try {
      await grokAdminApi.createModel(token, {
        provider,
        upstream_model: model,
        aliases: aliases
          .split(",")
          .map((a) => a.trim())
          .filter(Boolean),
      });
      onSaved();
      onOpenChange(false);
    } catch (err) {
      setError(err instanceof Error ? err.message : "创建模型失败");
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="fixed inset-0 z-[100] flex items-center justify-center bg-black/30 p-4 backdrop-blur-sm">
      <div className="neo-card w-full max-w-md p-6">
        <h2 className="text-lg font-semibold text-[var(--neo-ink)]">新建模型路由</h2>
        <p className="mt-1 text-sm text-[var(--neo-muted)]">POST /admin/models</p>
        <div className="mt-4 space-y-4">
          <div className="space-y-2">
            <Label htmlFor="model-provider">Provider</Label>
            <select
              id="model-provider"
              value={provider}
              onChange={(e) => setProvider(e.target.value)}
              className="neo-input h-9 w-full rounded-md px-3 text-sm"
            >
              {PROVIDER_OPTIONS.map((option) => (
                <option key={option.value} value={option.value}>
                  {option.label}
                </option>
              ))}
            </select>
          </div>
          <div className="space-y-2">
            <Label htmlFor="model-upstream">上游模型名</Label>
            <Input
              id="model-upstream"
              value={upstreamModel}
              onChange={(e) => setUpstreamModel(e.target.value)}
              placeholder="如 grok-4 / grok-4.5-build-free"
            />
          </div>
          <div className="space-y-2">
            <Label htmlFor="model-aliases">别名（逗号分隔，可选）</Label>
            <Input
              id="model-aliases"
              value={aliases}
              onChange={(e) => setAliases(e.target.value)}
              placeholder="如 grok-4b, grok-latest"
            />
          </div>
          {error ? <p className="text-sm text-rose-600">{error}</p> : null}
        </div>
        <div className="mt-6 flex justify-end gap-2">
          <Button variant="ghost" size="sm" onClick={() => onOpenChange(false)} disabled={saving}>
            取消
          </Button>
          <Button size="sm" onClick={() => void handleSave()} disabled={saving}>
            {saving ? <LoaderCircle className="size-4 animate-spin" /> : null}
            创建
          </Button>
        </div>
      </div>
    </div>
  );
}

function ModelsContent({ token }: { token: string }) {
  const [items, setItems] = useState<GrokModelRoute[]>([]);
  const [bindings, setBindings] = useState<GrokModelBinding[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const [createOpen, setCreateOpen] = useState(false);

  const load = useCallback(async (currentToken: string) => {
    setLoading(true);
    setError("");
    try {
      const data = await grokAdminApi.listModels(currentToken, { page: 1, pageSize: PAGE_SIZE });
      setItems(data.items ?? []);
      const bindingList = await grokAdminApi.listModelBindings(currentToken).catch(() => []);
      setBindings(bindingList);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
      setItems([]);
    } finally {
      setLoading(false);
    }
  }, []);

  // 挂载/令牌变化时加载（setTimeout 0：避免 effect 内同步 setState）。
  useEffect(() => {
    const timer = setTimeout(() => void load(token), 0);
    return () => clearTimeout(timer);
  }, [token, load]);

  const handleToggle = useCallback(
    async (route: GrokModelRoute, token: string) => {
      try {
        await grokAdminApi.updateModel(token, route.id, { enabled: !route.enabled });
        setItems((prev) =>
          prev.map((r) => (r.id === route.id ? { ...r, enabled: !route.enabled } : r)),
        );
      } catch (err) {
        setError(err instanceof Error ? err.message : "更新模型失败");
      }
    },
    [],
  );

  const handleDelete = useCallback(
    async (route: GrokModelRoute, token: string) => {
      if (!window.confirm(`删除模型路由「${route.upstream_model}」？此操作不可撤销。`)) return;
      try {
        await grokAdminApi.deleteModel(token, route.id);
        setItems((prev) => prev.filter((r) => r.id !== route.id));
      } catch (err) {
        setError(err instanceof Error ? err.message : "删除模型失败");
      }
    },
    [],
  );

  const bindingsByModel: Record<string, number> = {};
  for (const binding of bindings) {
    bindingsByModel[binding.upstream_model] = (binding.account_ids ?? []).length;
  }

  return (
    <div className="flex flex-col gap-3">
            <div className="flex flex-wrap items-center justify-between gap-2 text-xs text-[var(--neo-muted)]">
              <span>共 {items.length} 个模型路由</span>
              <Button size="sm" onClick={() => setCreateOpen(true)} disabled={loading}>
                <Plus className="size-4" />
                新建模型
              </Button>
            </div>
            {error ? <p className="text-sm text-rose-600">{error}</p> : null}
            {loading && items.length === 0 ? (
              <div className="flex items-center justify-center gap-2 py-16 text-sm text-[var(--neo-muted)]">
                <LoaderCircle className="size-4 animate-spin" /> 加载中…
              </div>
            ) : (
              <ModelsTable
                items={items}
                bindingsByModel={bindingsByModel}
                onToggle={(route) => void handleToggle(route, token)}
                onDelete={(route) => void handleDelete(route, token)}
              />
            )}
            <div className="flex items-center gap-2 text-xs text-[var(--neo-muted)]">
              <Button variant="outline" size="sm" onClick={() => void load(token)} disabled={loading}>
                {loading ? <LoaderCircle className="size-4 animate-spin" /> : <RefreshCw className="size-4" />}
                刷新
              </Button>
            </div>
      <NewModelDialog
        open={createOpen}
        token={token}
        onOpenChange={setCreateOpen}
        onSaved={() => void load(token)}
      />
    </div>
  );
}

export default function GrokModelsPage() {
  return (
    <PageShell title="Grok 模型" subtitle="模型路由管理（列表 / 新建 / 启停 / 删除）" badge="G4-P2">
      <GrokTabs />
      <GrokTokenGate>{(token) => <ModelsContent token={token} />}</GrokTokenGate>
    </PageShell>
  );
}
