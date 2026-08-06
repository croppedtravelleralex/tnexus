"use client";

import { LoaderCircle, X } from "lucide-react";
import { useEffect, useState } from "react";

import { Button } from "@/components/ui/button";
import { Input, Label } from "@/components/ui/input";
import {
  grokAdminApi,
  type GrokAccountView,
  type GrokUpdateAccountInput,
} from "@/lib/grok-admin";

/** 后端 parse_auth_status 可解析的取值（active/restricted/banned/reauth_required）。 */
const AUTH_STATUS_OPTIONS = [
  { value: "active", label: "Active" },
  { value: "restricted", label: "受限" },
  { value: "banned", label: "封禁" },
  { value: "reauth_required", label: "需重登" },
];

type Props = {
  open: boolean;
  account: GrokAccountView | null;
  token: string;
  onOpenChange: (open: boolean) => void;
  onSaved: () => void;
};

/** 日期字符串 → datetime-local 输入值（本地时区，无日期返回空串）。 */
function toLocalInput(value: string | null): string {
  if (!value) return "";
  const d = new Date(value);
  if (Number.isNaN(d.getTime())) return "";
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(
    d.getHours(),
  )}:${pad(d.getMinutes())}`;
}

/** 编辑 grok 账号（enabled / auth_status / priority / cooldown_until → PATCH /admin/accounts/:id）。 */
export function GrokAccountEditDialog({ open, account, token, onOpenChange, onSaved }: Props) {
  const [enabled, setEnabled] = useState(true);
  const [authStatus, setAuthStatus] = useState("active");
  const [priority, setPriority] = useState("");
  const [cooldown, setCooldown] = useState("");
  const [clearCooldown, setClearCooldown] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");

  useEffect(() => {
    if (!open || !account) return;
    let cancelled = false;
    // 微任务重置：避免 effect 内同步 setState（react-compiler 规则）。
    queueMicrotask(() => {
      if (cancelled) return;
      setEnabled(account.enabled);
      setAuthStatus(account.auth_status);
      setPriority(String(account.priority));
      setCooldown(toLocalInput(account.cooldown_until));
      setClearCooldown(false);
      setError("");
    });
    return () => {
      cancelled = true;
    };
  }, [open, account]);

  if (!open || !account) return null;

  const handleSave = async () => {
    setSaving(true);
    setError("");
    try {
      const input: GrokUpdateAccountInput = { enabled, auth_status: authStatus };
      const prio = Number(priority);
      if (Number.isFinite(prio)) input.priority = Math.trunc(prio);
      if (clearCooldown) {
        input.cooldown_until = null;
      } else if (cooldown) {
        const value = new Date(cooldown);
        if (Number.isNaN(value.getTime())) {
          throw new Error("冷却时间格式无效");
        }
        input.cooldown_until = value.toISOString();
      }
      await grokAdminApi.updateAccount(token, account.id, input);
      onSaved();
      onOpenChange(false);
      alert("账号已更新");
    } catch (err) {
      setError(err instanceof Error ? err.message : "更新账号失败");
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="fixed inset-0 z-[100] flex items-center justify-center bg-black/30 p-4 backdrop-blur-sm">
      <div className="neo-card w-full max-w-md p-6">
        <div className="mb-4 flex items-start justify-between gap-3">
          <div>
            <h2 className="text-lg font-semibold text-[var(--neo-ink)]">编辑 Grok 账号</h2>
            <p className="mt-1 truncate text-sm text-[var(--neo-muted)]">
              #{account.id} · {account.name || "—"} · {account.provider}
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
        <div className="space-y-4">
          <div className="flex items-center justify-between rounded-lg border border-[var(--neo-border)] px-3 py-2">
            <Label htmlFor="edit-enabled" className="text-sm font-medium">
              启用
            </Label>
            <button
              id="edit-enabled"
              type="button"
              role="switch"
              aria-checked={enabled}
              className={`relative h-5 w-9 rounded-full transition-colors ${
                enabled ? "bg-pink-500" : "bg-stone-300"
              }`}
              onClick={() => setEnabled((v) => !v)}
            >
              <span
                className={`absolute top-0.5 size-4 rounded-full bg-white transition-all ${
                  enabled ? "left-[18px]" : "left-0.5"
                }`}
              />
            </button>
          </div>

          <div className="space-y-2">
            <Label htmlFor="edit-auth-status">认证状态</Label>
            <select
              id="edit-auth-status"
              value={authStatus}
              onChange={(e) => setAuthStatus(e.target.value)}
              className="neo-input h-9 w-full rounded-md px-3 text-sm"
            >
              {AUTH_STATUS_OPTIONS.map((option) => (
                <option key={option.value} value={option.value}>
                  {option.label}
                </option>
              ))}
            </select>
          </div>

          <div className="space-y-2">
            <Label htmlFor="edit-priority">优先级</Label>
            <Input
              id="edit-priority"
              type="number"
              value={priority}
              onChange={(e) => setPriority(e.target.value)}
              placeholder="数字，越大越优先"
            />
          </div>

          <div className="space-y-2">
            <Label htmlFor="edit-cooldown">冷却至</Label>
            <Input
              id="edit-cooldown"
              type="datetime-local"
              value={cooldown}
              disabled={clearCooldown}
              onChange={(e) => {
                setCooldown(e.target.value);
                setClearCooldown(false);
              }}
            />
            <label className="flex items-center gap-2 text-xs text-[var(--neo-muted)]">
              <input
                type="checkbox"
                checked={clearCooldown}
                onChange={(e) => setClearCooldown(e.target.checked)}
              />
              清除冷却（保存时置 null）
            </label>
          </div>

          {error ? <p className="text-sm text-rose-600">{error}</p> : null}
        </div>
        <div className="mt-6 flex justify-end gap-2">
          <Button variant="ghost" size="sm" onClick={() => onOpenChange(false)} disabled={saving}>
            取消
          </Button>
          <Button size="sm" onClick={() => void handleSave()} disabled={saving}>
            {saving ? <LoaderCircle className="size-4 animate-spin" /> : null}
            保存
          </Button>
        </div>
      </div>
    </div>
  );
}