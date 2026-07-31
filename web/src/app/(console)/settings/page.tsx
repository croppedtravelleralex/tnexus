"use client";

import Image from "next/image";
import { useRouter } from "next/navigation";
import { useCallback, useEffect, useState } from "react";
import { LoaderCircle, RefreshCw } from "lucide-react";
import { ElevatedCard, PageShell } from "@/components/admin/page-shell";
import { Button } from "@/components/ui/button";
import { ChoiceButton, SegmentGroup } from "@/components/ui/choice-button";
import { Input } from "@/components/ui/input";
import { authApi, proxyApi, type User } from "@/lib/api";
import { useAuth } from "@/lib/auth";

function roleLabel(role: string) {
  return role === "admin" ? "管理员" : "成员";
}

function formatDate(iso?: string) {
  if (!iso) return "—";
  return new Date(iso).toLocaleString("zh-CN");
}

export default function SettingsPage() {
  const { user, logout } = useAuth();
  const router = useRouter();
  const [users, setUsers] = useState<User[]>([]);
  const [adminBusy, setAdminBusy] = useState<string | null>(null);
  const [tab, setTab] = useState<"system" | "account">("system");
  const [proxyRuntime, setProxyRuntime] = useState<Record<string, unknown>>({});
  const [proxyUrl, setProxyUrl] = useState("");
  const [proxyBusy, setProxyBusy] = useState(false);
  const [proxyError, setProxyError] = useState("");
  const [webshareStatus, setWebshareStatus] = useState<Record<string, unknown> | null>(null);

  const loadProxy = useCallback(async () => {
    if (user?.role !== "admin") return;
    try {
      const data = await proxyApi.runtime();
      const runtime = (data.runtime as Record<string, unknown>) ?? {};
      setProxyRuntime(runtime);
      setProxyUrl(String(runtime.proxy_url ?? ""));
      const ws = await proxyApi.webshareStatus();
      setWebshareStatus(ws);
    } catch (e) {
      setProxyError(e instanceof Error ? e.message : "加载代理配置失败");
    }
  }, [user?.role]);

  useEffect(() => {
    if (user?.role === "admin" && tab === "system") {
      void loadProxy();
    }
  }, [user, tab, loadProxy]);

  const saveProxy = async () => {
    setProxyBusy(true);
    setProxyError("");
    try {
      const next = { ...proxyRuntime, proxy_url: proxyUrl.trim(), enabled: true };
      const data = await proxyApi.saveRuntime(next);
      const runtime = (data.runtime as Record<string, unknown>) ?? next;
      setProxyRuntime(runtime);
      setProxyUrl(String(runtime.proxy_url ?? ""));
    } catch (e) {
      setProxyError(e instanceof Error ? e.message : "保存失败");
    } finally {
      setProxyBusy(false);
    }
  };

  const runWebshareScan = async () => {
    setProxyBusy(true);
    setProxyError("");
    try {
      await proxyApi.webshareRunOnce();
      const ws = await proxyApi.webshareStatus();
      setWebshareStatus(ws);
    } catch (e) {
      setProxyError(e instanceof Error ? e.message : "扫描失败");
    } finally {
      setProxyBusy(false);
    }
  };

  const loadUsers = useCallback(async () => {
    const list = await authApi.listUsers();
    setUsers(list);
  }, []);

  useEffect(() => {
    if (user?.role === "admin") {
      void loadUsers().catch(() => undefined);
    }
  }, [user, loadUsers]);

  const toggleDisabled = async (target: User) => {
    if (target.id === user?.id) return;
    setAdminBusy(target.id);
    try {
      await authApi.setDisabled(target.id, !target.disabled);
      await loadUsers();
    } finally {
      setAdminBusy(null);
    }
  };

  if (!user) return null;

  return (
    <PageShell
      title="设置"
      actions={
        <SegmentGroup>
          <ChoiceButton variant="segment" active={tab === "system"} onClick={() => setTab("system")}>
            系统
          </ChoiceButton>
          <ChoiceButton variant="segment" active={tab === "account"} onClick={() => setTab("account")}>
            账户
          </ChoiceButton>
        </SegmentGroup>
      }
    >
      {tab === "system" ? (
        <div className="grid gap-4 lg:grid-cols-2">
          <ElevatedCard className="p-5">
            <h2 className="text-sm font-semibold text-[var(--neo-ink)]">Gateway 公网地址</h2>
            <div className="mt-3 rounded-lg border border-[var(--neo-border)] bg-[var(--neo-surface-muted)] px-3 py-2 font-mono text-xs text-[var(--neo-muted)]">
              https://tnexus.relai.asia/v1
            </div>
          </ElevatedCard>
          <ElevatedCard className="p-5">
            <div className="flex items-center justify-between gap-2">
              <h2 className="text-sm font-semibold text-[var(--neo-ink)]">Webshare 代理（TNexus 托管）</h2>
              <Button size="sm" variant="outline" className="h-8" disabled={proxyBusy} onClick={() => void loadProxy()}>
                {proxyBusy ? <LoaderCircle className="size-3.5 animate-spin" /> : <RefreshCw className="size-3.5" />}
              </Button>
            </div>
            <p className="mt-2 text-xs text-[var(--neo-muted)]">
              读写 gptimage 本地 config（account-ops + GPTIMAGE_ROOT），不经过生产 :8012 HTTP。
            </p>
            {proxyError ? <p className="mt-2 text-xs text-red-600">{proxyError}</p> : null}
            <label className="mt-3 block text-xs font-medium text-[var(--neo-muted)]">出口代理 URL</label>
            <Input
              className="mt-1 font-mono text-xs"
              value={proxyUrl}
              onChange={(e) => setProxyUrl(e.target.value)}
              placeholder="http://user:pass@host:port"
            />
            <div className="mt-3 flex flex-wrap gap-2">
              <Button size="sm" disabled={proxyBusy} onClick={() => void saveProxy()}>
                保存代理
              </Button>
              <Button size="sm" variant="outline" disabled={proxyBusy} onClick={() => void runWebshareScan()}>
                立即 CF 扫描
              </Button>
            </div>
            {webshareStatus ? (
              <pre className="mt-3 max-h-40 overflow-auto rounded bg-[var(--neo-surface-muted)] p-2 text-[10px] text-[var(--neo-muted)]">
                {JSON.stringify(webshareStatus, null, 2)}
              </pre>
            ) : null}
          </ElevatedCard>
        </div>
      ) : (
        <div className="mx-auto max-w-3xl space-y-4">
          <ElevatedCard className="p-6">
            <div className="flex items-start gap-4">
              <Image src="/logo.png" alt="" width={48} height={48} className="rounded-xl shadow-sm" />
              <div>
                <h2 className="text-lg font-semibold text-[var(--neo-ink)]">当前账户</h2>
                <p className="text-sm text-[var(--neo-muted)]">{user.display_name || user.email}</p>
              </div>
            </div>
            <dl className="mt-6 grid gap-3 text-sm sm:grid-cols-2">
              <div className="neo-stat-cell px-4 py-3">
                <dt className="text-[var(--neo-muted)]">角色</dt>
                <dd className="mt-1 font-medium">{roleLabel(user.role)}</dd>
              </div>
              <div className="neo-stat-cell px-4 py-3">
                <dt className="text-[var(--neo-muted)]">邮箱</dt>
                <dd className="mt-1 font-medium">{user.email}</dd>
              </div>
            </dl>
            <div className="mt-6">
              <Button variant="outline" onClick={() => void logout().then(() => router.push("/login"))}>
                退出登录
              </Button>
            </div>
          </ElevatedCard>

          {user.role === "admin" && users.length > 0 ? (
            <ElevatedCard className="overflow-hidden">
              <div className="border-b border-[var(--neo-border)] px-4 py-3">
                <h3 className="font-semibold text-[var(--neo-ink)]">用户管理</h3>
              </div>
              <table className="w-full text-left text-sm">
                <thead className="neo-table-head">
                  <tr>
                    <th className="px-4 py-2.5">用户</th>
                    <th className="px-4 py-2.5">角色</th>
                    <th className="px-4 py-2.5">状态</th>
                    <th className="px-4 py-2.5">注册</th>
                    <th className="px-4 py-2.5">操作</th>
                  </tr>
                </thead>
                <tbody>
                  {users.map((u) => (
                    <tr key={u.id} className="border-t border-[var(--neo-border)] neo-row-hover">
                      <td className="px-4 py-3">
                        <p className="font-medium">{u.display_name}</p>
                        <p className="text-xs text-[var(--neo-muted)]">{u.email}</p>
                      </td>
                      <td className="px-4 py-3">{roleLabel(u.role)}</td>
                      <td className="px-4 py-3">{u.disabled ? "已禁用" : "正常"}</td>
                      <td className="px-4 py-3 text-[var(--neo-muted)]">{formatDate(u.created_at)}</td>
                      <td className="px-4 py-3">
                        {u.id !== user.id ? (
                          <Button
                            size="sm"
                            variant="outline"
                            disabled={adminBusy === u.id}
                            onClick={() => void toggleDisabled(u)}
                          >
                            {u.disabled ? "启用" : "禁用"}
                          </Button>
                        ) : (
                          "—"
                        )}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </ElevatedCard>
          ) : null}
        </div>
      )}
    </PageShell>
  );
}
