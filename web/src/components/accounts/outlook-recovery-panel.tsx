"use client";

import { LoaderCircle, Mail, RefreshCw } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { ElevatedCard } from "@/components/admin/page-shell";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { accountsApi, type Account } from "@/lib/api";

type Props = {
  selectedAccount?: Account | null;
  onCompleted?: () => void;
};

export function OutlookRecoveryPanel({ selectedAccount, onCompleted }: Props) {
  const [status, setStatus] = useState<Record<string, unknown> | null>(null);
  const [loading, setLoading] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [progressId, setProgressId] = useState<string | null>(null);
  const [progress, setProgress] = useState<Record<string, unknown> | null>(null);

  const loadStatus = useCallback(async () => {
    setLoading(true);
    setError("");
    try {
      const data = await accountsApi.outlookRecoveryStatus();
      setStatus(data);
    } catch (e) {
      setError(e instanceof Error ? e.message : "加载失败");
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void loadStatus();
  }, [loadStatus]);

  useEffect(() => {
    if (!progressId) return;
    let cancelled = false;
    const tick = async () => {
      try {
        const row = await accountsApi.recoverOutlookProgress(progressId);
        if (cancelled) return;
        setProgress(row);
        if (row.done === true || row.finished === true) {
          setProgressId(null);
          onCompleted?.();
          void loadStatus();
        }
      } catch {
        if (!cancelled) setProgressId(null);
      }
    };
    void tick();
    const id = window.setInterval(() => void tick(), 1500);
    return () => {
      cancelled = true;
      window.clearInterval(id);
    };
  }, [progressId, loadStatus, onCompleted]);

  const toggleAuto = async () => {
    const enabled = !(status?.enabled === true);
    setBusy(true);
    try {
      const data = await accountsApi.outlookRecoveryEnable(enabled);
      setStatus(data);
    } catch (e) {
      setError(e instanceof Error ? e.message : "切换失败");
    } finally {
      setBusy(false);
    }
  };

  const recoverSelected = async () => {
    const token = selectedAccount?.access_token?.trim();
    if (!token) {
      setError("请先在表格中选择一个 Outlook 账号");
      return;
    }
    setBusy(true);
    setError("");
    try {
      const { progress_id } = await accountsApi.recoverOutlook(token);
      setProgressId(progress_id);
      setProgress({ done: false });
    } catch (e) {
      setError(e instanceof Error ? e.message : "恢复失败");
    } finally {
      setBusy(false);
    }
  };

  const autoEnabled = status?.enabled === true;
  const running = status?.running === true || Boolean(progressId);

  return (
    <ElevatedCard className="mt-4 p-4">
      <div className="mb-3 flex flex-wrap items-center justify-between gap-2">
        <div className="flex items-center gap-2">
          <Mail className="size-4 text-[var(--neo-primary-deep)]" />
          <h3 className="text-sm font-semibold text-[var(--neo-ink)]">Outlook 自动恢复</h3>
          {autoEnabled ? <Badge variant="default">已启用</Badge> : <Badge variant="muted">未启用</Badge>}
          {running ? <Badge variant="muted">运行中</Badge> : null}
        </div>
        <div className="flex flex-wrap gap-2">
          <Button size="sm" variant="outline" className="h-8" disabled={loading || busy} onClick={() => void loadStatus()}>
            {loading ? <LoaderCircle className="size-3.5 animate-spin" /> : <RefreshCw className="size-3.5" />}
            刷新
          </Button>
          <Button size="sm" variant="outline" className="h-8" disabled={busy} onClick={() => void toggleAuto()}>
            {autoEnabled ? "关闭自动恢复" : "开启自动恢复"}
          </Button>
          <Button size="sm" className="h-8" disabled={busy || !selectedAccount} onClick={() => void recoverSelected()}>
            恢复所选账号
          </Button>
        </div>
      </div>
      {error ? <p className="mb-2 text-xs text-red-600">{error}</p> : null}
      {status?.message ? (
        <p className="text-xs text-[var(--neo-muted)]">{String(status.message)}</p>
      ) : (
        <p className="text-xs text-[var(--neo-muted)]">
          通过 account-ops 本地执行 Outlook 恢复环；需 GPTIMAGE_ROOT 与邮箱凭据配置。
        </p>
      )}
      {progress ? (
        <pre className="mt-2 max-h-32 overflow-auto rounded bg-[var(--neo-surface-muted)] p-2 text-[10px] text-[var(--neo-muted)]">
          {JSON.stringify(progress, null, 2)}
        </pre>
      ) : null}
    </ElevatedCard>
  );
}
