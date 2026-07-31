"use client";

import Image from "next/image";
import { useRouter } from "next/navigation";
import { useCallback, useEffect, useState } from "react";
import { ElevatedCard, PageShell } from "@/components/admin/page-shell";
import { Button } from "@/components/ui/button";
import { ChoiceButton, SegmentGroup } from "@/components/ui/choice-button";
import { authApi, type User } from "@/lib/api";
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
            <h2 className="text-sm font-semibold text-[var(--neo-ink)]">代理与运行时</h2>
            <p className="mt-3 text-sm text-[var(--neo-muted)]">代理与 FlareSolverr 运行时配置将在此保存。</p>
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
