"use client";

import Link from "next/link";
import { useRouter } from "next/navigation";
import { useState } from "react";
import { AuthShell } from "@/components/auth-shell";
import { authApi } from "@/lib/api";
import { useAuth } from "@/lib/auth";

export default function RegisterPage() {
  const router = useRouter();
  const { refresh } = useAuth();
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [displayName, setDisplayName] = useState("");
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);

  const onSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setLoading(true);
    setError("");
    try {
      await authApi.register({ email, password, display_name: displayName || undefined });
      await refresh();
      router.push("/studio");
    } catch (err) {
      setError(err instanceof Error ? err.message : "注册失败");
    } finally {
      setLoading(false);
    }
  };

  return (
    <AuthShell
      title="注册 TNexus"
      subtitle="创建你的创作账号"
      footer={
        <>
          已有账号？{" "}
          <Link href="/login" className="text-violet-300 hover:text-violet-200">
            登录
          </Link>
        </>
      }
    >
      <form onSubmit={onSubmit} className="space-y-5">
        <div className="space-y-2">
          <label htmlFor="displayName" className="auth-label">
            昵称
          </label>
          <input
            id="displayName"
            className="auth-input"
            placeholder="你的昵称"
            value={displayName}
            onChange={(e) => setDisplayName(e.target.value)}
          />
        </div>
        <div className="space-y-2">
          <label htmlFor="email" className="auth-label">
            账号 / 邮箱
          </label>
          <input
            id="email"
            className="auth-input"
            placeholder="username 或 email@example.com"
            value={email}
            onChange={(e) => setEmail(e.target.value)}
            required
            autoComplete="username"
          />
        </div>
        <div className="space-y-2">
          <label htmlFor="password" className="auth-label">
            密码（至少 8 位）
          </label>
          <input
            id="password"
            type="password"
            className="auth-input"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            required
            minLength={8}
            autoComplete="new-password"
          />
        </div>
        {error && <p className="text-sm text-red-400">{error}</p>}
        <button type="submit" className="auth-btn" disabled={loading}>
          {loading ? "注册中..." : "注册"}
        </button>
      </form>
    </AuthShell>
  );
}
