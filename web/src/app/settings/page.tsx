"use client";

import Image from "next/image";
import Link from "next/link";
import { useRouter } from "next/navigation";
import { useCallback, useEffect, useState } from "react";
import { SiteHeader } from "@/components/site-header";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
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
  const { user, loading, logout } = useAuth();
  const router = useRouter();
  const [users, setUsers] = useState<User[]>([]);
  const [adminBusy, setAdminBusy] = useState<string | null>(null);

  useEffect(() => {
    if (!loading && !user) router.replace("/login");
  }, [loading, user, router]);

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

  if (loading || !user) return null;

  const activeCount = users.filter((u) => !u.disabled).length;
  const disabledCount = users.filter((u) => u.disabled).length;
  const adminCount = users.filter((u) => u.role === "admin").length;

  return (
    <div className="min-h-screen bg-zinc-50">
      <SiteHeader />
      <main className="mx-auto max-w-4xl space-y-6 px-6 py-8">
        <Card className="p-6">
          <div className="flex items-start gap-4">
            <Image src="/logo.png" alt="" width={48} height={48} className="rounded-xl" />
            <div className="flex-1">
              <h1 className="text-xl font-semibold text-zinc-900">账户设置</h1>
              <p className="mt-1 text-sm text-zinc-500">管理你的 TNexus 账号</p>
            </div>
          </div>
          <dl className="mt-6 grid gap-3 text-sm sm:grid-cols-2">
            <div className="rounded-lg border border-zinc-200 bg-zinc-50 px-4 py-3">
              <dt className="text-zinc-500">昵称</dt>
              <dd className="mt-1 font-medium text-zinc-900">{user.display_name || user.email}</dd>
            </div>
            <div className="rounded-lg border border-zinc-200 bg-zinc-50 px-4 py-3">
              <dt className="text-zinc-500">账号</dt>
              <dd className="mt-1 font-medium text-zinc-900">{user.email}</dd>
            </div>
            <div className="rounded-lg border border-zinc-200 bg-zinc-50 px-4 py-3">
              <dt className="text-zinc-500">角色</dt>
              <dd className="mt-1">
                <span
                  className={`inline-flex rounded-full px-2 py-0.5 text-xs font-medium ${
                    user.role === "admin"
                      ? "bg-violet-100 text-violet-700"
                      : "bg-zinc-200 text-zinc-700"
                  }`}
                >
                  {roleLabel(user.role)}
                </span>
              </dd>
            </div>
            <div className="rounded-lg border border-zinc-200 bg-zinc-50 px-4 py-3">
              <dt className="text-zinc-500">状态</dt>
              <dd className="mt-1 font-medium text-emerald-600">正常</dd>
            </div>
          </dl>
          <div className="mt-6 flex flex-wrap gap-3">
            <Button variant="outline" onClick={() => void logout().then(() => router.push("/"))}>
              退出登录
            </Button>
            <Link href="/studio">
              <Button>进入工作台</Button>
            </Link>
          </div>
        </Card>

        {user.role === "admin" && (
          <Card className="p-6">
            <div className="mb-6 flex flex-wrap items-end justify-between gap-4">
              <div>
                <h2 className="text-lg font-semibold text-zinc-900">用户管理</h2>
                <p className="mt-1 text-sm text-zinc-500">启用 / 禁用账号，查看成员信息</p>
              </div>
              <div className="flex flex-wrap gap-2 text-xs">
                <span className="rounded-full bg-zinc-100 px-3 py-1 text-zinc-600">
                  共 {users.length} 人
                </span>
                <span className="rounded-full bg-emerald-50 px-3 py-1 text-emerald-700">
                  正常 {activeCount}
                </span>
                <span className="rounded-full bg-amber-50 px-3 py-1 text-amber-700">
                  禁用 {disabledCount}
                </span>
                <span className="rounded-full bg-violet-50 px-3 py-1 text-violet-700">
                  管理员 {adminCount}
                </span>
              </div>
            </div>

            <div className="overflow-hidden rounded-lg border border-zinc-200">
              <table className="w-full text-left text-sm">
                <thead className="bg-zinc-50 text-zinc-500">
                  <tr>
                    <th className="px-4 py-3 font-medium">用户</th>
                    <th className="px-4 py-3 font-medium">角色</th>
                    <th className="px-4 py-3 font-medium">状态</th>
                    <th className="px-4 py-3 font-medium">注册时间</th>
                    <th className="px-4 py-3 font-medium">操作</th>
                  </tr>
                </thead>
                <tbody>
                  {users.map((u) => {
                    const isSelf = u.id === user.id;
                    return (
                      <tr
                        key={u.id}
                        className={`border-t border-zinc-100 ${isSelf ? "bg-violet-50/50" : ""}`}
                      >
                        <td className="px-4 py-3">
                          <p className="font-medium text-zinc-900">
                            {u.display_name}
                            {isSelf && (
                              <span className="ml-2 text-xs text-violet-600">（当前）</span>
                            )}
                          </p>
                          <p className="text-xs text-zinc-500">{u.email}</p>
                        </td>
                        <td className="px-4 py-3">
                          <span
                            className={`inline-flex rounded-full px-2 py-0.5 text-xs font-medium ${
                              u.role === "admin"
                                ? "bg-violet-100 text-violet-700"
                                : "bg-zinc-100 text-zinc-600"
                            }`}
                          >
                            {roleLabel(u.role)}
                          </span>
                        </td>
                        <td className="px-4 py-3">
                          {u.disabled ? (
                            <span className="text-amber-600">已禁用</span>
                          ) : (
                            <span className="text-emerald-600">正常</span>
                          )}
                        </td>
                        <td className="px-4 py-3 text-zinc-500">{formatDate(u.created_at)}</td>
                        <td className="px-4 py-3">
                          {isSelf ? (
                            <span className="text-xs text-zinc-400">—</span>
                          ) : (
                            <Button
                              size="sm"
                              variant={u.disabled ? "default" : "outline"}
                              disabled={adminBusy === u.id}
                              onClick={() => void toggleDisabled(u)}
                            >
                              {adminBusy === u.id
                                ? "处理中..."
                                : u.disabled
                                  ? "启用"
                                  : "禁用"}
                            </Button>
                          )}
                        </td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            </div>
          </Card>
        )}
      </main>
    </div>
  );
}
