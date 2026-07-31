"use client";

import { LoaderCircle, X } from "lucide-react";
import { useEffect, useState } from "react";

import { Button } from "@/components/ui/button";
import { Input, Label } from "@/components/ui/input";
import { accountsApi, type Account, type AccountStatus } from "@/lib/api";

const STATUS_OPTIONS: AccountStatus[] = ["正常", "限流", "异常", "禁用"];

type Props = {
  open: boolean;
  account: Account | null;
  onOpenChange: (open: boolean) => void;
  onSaved: () => void;
};

export function AccountEditDialog({ open, account, onOpenChange, onSaved }: Props) {
  const [type, setType] = useState("");
  const [status, setStatus] = useState<AccountStatus>("正常");
  const [proxy, setProxy] = useState("");
  const [quota, setQuota] = useState("");
  const [softBand, setSoftBand] = useState("");
  const [clearSoftBand, setClearSoftBand] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");

  useEffect(() => {
    if (!open || !account) return;
    setType(account.type ?? "");
    setStatus(account.status);
    setProxy(account.proxy ?? "");
    setQuota(String(account.quota ?? 0));
    setSoftBand("");
    setClearSoftBand(false);
    setError("");
  }, [open, account]);

  if (!open || !account) return null;

  const handleSave = async () => {
    setSaving(true);
    setError("");
    try {
      const quotaNum = Number(quota);
      await accountsApi.update({
        access_token: account.access_token,
        type: type.trim() || undefined,
        status,
        proxy: proxy.trim(),
        quota: Number.isFinite(quotaNum) ? quotaNum : undefined,
      });
      if (clearSoftBand) {
        await accountsApi.softBand(account.access_token, null);
      } else if (softBand.trim()) {
        const percent = Number(softBand);
        if (!Number.isFinite(percent) || percent < 0 || percent > 100) {
          throw new Error("软带宽百分比需在 0–100 之间");
        }
        await accountsApi.softBand(account.access_token, percent);
      }
      onSaved();
      onOpenChange(false);
      alert("账号信息已更新");
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
            <h2 className="text-lg font-semibold text-[var(--neo-ink)]">编辑账户</h2>
            <p className="mt-1 text-sm text-[var(--neo-muted)]">{account.email ?? account.access_token.slice(0, 12)}</p>
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
          <div className="space-y-2">
            <Label htmlFor="edit-type">类型</Label>
            <Input id="edit-type" value={type} onChange={(e) => setType(e.target.value)} placeholder="例如 free / plus" />
          </div>
          <div className="space-y-2">
            <Label htmlFor="edit-status">状态</Label>
            <select
              id="edit-status"
              value={status}
              onChange={(e) => setStatus(e.target.value as AccountStatus)}
              className="neo-input h-9 w-full rounded-md px-3 text-sm"
            >
              {STATUS_OPTIONS.map((option) => (
                <option key={option} value={option}>
                  {option}
                </option>
              ))}
            </select>
          </div>
          <div className="space-y-2">
            <Label htmlFor="edit-proxy">账号代理</Label>
            <Input
              id="edit-proxy"
              value={proxy}
              onChange={(e) => setProxy(e.target.value)}
              placeholder="留空走全局代理，例如 http://127.0.0.1:7890"
            />
          </div>
          <div className="space-y-2">
            <Label htmlFor="edit-quota">额度</Label>
            <Input
              id="edit-quota"
              type="number"
              value={quota}
              onChange={(e) => setQuota(e.target.value)}
              placeholder="手动覆盖额度"
            />
          </div>
          <div className="space-y-2">
            <Label htmlFor="edit-soft-band">软带宽 (%)</Label>
            <Input
              id="edit-soft-band"
              type="number"
              min={0}
              max={100}
              value={softBand}
              onChange={(e) => {
                setSoftBand(e.target.value);
                setClearSoftBand(false);
              }}
              placeholder="留空不修改；0–100"
              disabled={clearSoftBand}
            />
            <label className="inline-flex items-center gap-2 text-xs text-[var(--neo-muted)]">
              <input
                type="checkbox"
                checked={clearSoftBand}
                onChange={(e) => {
                  setClearSoftBand(e.target.checked);
                  if (e.target.checked) setSoftBand("");
                }}
              />
              清除软带宽设置
            </label>
          </div>
          {error ? <p className="text-sm text-red-600">{error}</p> : null}
        </div>
        <div className="mt-6 flex justify-end gap-2">
          <Button type="button" variant="outline" onClick={() => onOpenChange(false)} disabled={saving}>
            取消
          </Button>
          <Button type="button" onClick={() => void handleSave()} disabled={saving}>
            {saving ? <LoaderCircle className="size-4 animate-spin" /> : null}
            保存修改
          </Button>
        </div>
      </div>
    </div>
  );
}
