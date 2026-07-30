"use client";

import Link from "next/link";
import { useRouter } from "next/navigation";
import { useState } from "react";
import { AuthShell } from "@/components/auth-shell";
import { authApi } from "@/lib/api";
import { useAuth } from "@/lib/auth";

export default function LoginPage() {
  const router = useRouter();
  const { refresh } = useAuth();
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);

  const onSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setLoading(true);
    setError("");
    try {
      await authApi.login({ email, password });
      await refresh();
      router.push("/studio");
    } catch (err) {
      setError(err instanceof Error ? err.message : "登录失败");
    } finally {
      setLoading(false);
    }
  };

  return (
    <AuthShell
      title="登录 TNexus"
      subtitle="进入 AI 生图工作台"
      footer={
        <>
          没有账号？{" "}
          <Link href="/register" className="text-violet-300 hover:text-violet-200">
            注册
          </Link>
        </>
      }
    >
      <form onSubmit={onSubmit} className="space-y-5">
        <div className="space-y-2">
          <label htmlFor="email" className="auth-label">
            账号
          </label>
          <input
            id="email"
            className="auth-input"
            placeholder="admin 或 user"
            value={email}
            onChange={(e) => setEmail(e.target.value)}
            required
            autoComplete="username"
          />
        </div>
        <div className="space-y-2">
          <label htmlFor="password" className="auth-label">
            密码
          </label>
          <input
            id="password"
            type="password"
            className="auth-input"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            required
            autoComplete="current-password"
          />
        </div>
        {error && <p className="text-sm text-red-400">{error}</p>}
        <button type="submit" className="auth-btn" disabled={loading}>
          {loading ? "登录中..." : "登录"}
        </button>
      </form>

      <div className="auth-hint">
        <p className="font-medium text-zinc-300">Mock 测试账号</p>
        <p className="mt-1">
          管理员：<span className="text-zinc-200">admin</span> /{" "}
          <span className="text-zinc-200">123456</span>
        </p>
        <p>
          普通用户：<span className="text-zinc-200">demo</span> /{" "}
          <span className="text-zinc-200">demo1234</span>
        </p>
        <p className="mt-2 text-zinc-500">
          若提示无法连接服务器，请先启动 API（localhost:9000）
        </p>
      </div>
    </AuthShell>
  );
}
