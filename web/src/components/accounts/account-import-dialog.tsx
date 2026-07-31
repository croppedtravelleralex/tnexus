"use client";

import { Copy, ExternalLink, FileJson, FileText, KeyRound, LoaderCircle, Upload, X } from "lucide-react";
import { useRef, useState } from "react";
import { Button } from "@/components/ui/button";
import { Input, Textarea } from "@/components/ui/input";
import { accountsApi, type AccountImportPayload } from "@/lib/api";

type ImportMethod = "menu" | "token" | "session" | "account-json" | "oauth";

type Props = {
  disabled?: boolean;
  onImported: () => void;
};

const SESSION_URL = "https://chatgpt.com/api/auth/session";

function splitTokens(value: string) {
  return value
    .split(/\r?\n/)
    .map((item) => item.trim())
    .filter(Boolean);
}

function getSessionAccessToken(value: unknown) {
  const token = (value as { accessToken?: unknown })?.accessToken;
  return typeof token === "string" ? token.trim() : "";
}

function getAccountJsonAccount(value: unknown): AccountImportPayload | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) return null;
  const raw = value as Record<string, unknown>;
  const tokenValue = raw.access_token ?? raw.accessToken;
  const token = typeof tokenValue === "string" ? tokenValue.trim() : "";
  if (!token) return null;
  return { ...raw, access_token: token };
}

function getAccountJsonAccounts(value: unknown): AccountImportPayload[] {
  if (Array.isArray(value)) {
    return value.map(getAccountJsonAccount).filter((item): item is AccountImportPayload => Boolean(item));
  }
  const single = getAccountJsonAccount(value);
  if (single) return [single];
  if (value && typeof value === "object") {
    const nested = (value as Record<string, unknown>).accounts ?? (value as Record<string, unknown>).items;
    if (Array.isArray(nested)) {
      return nested.map(getAccountJsonAccount).filter((item): item is AccountImportPayload => Boolean(item));
    }
  }
  return [];
}

async function readFileAsText(file: File) {
  return new Promise<string>((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(typeof reader.result === "string" ? reader.result : "");
    reader.onerror = () => reject(reader.error ?? new Error(`读取文件失败: ${file.name}`));
    reader.readAsText(file);
  });
}

function MethodCard({
  title,
  description,
  icon: Icon,
  onClick,
}: {
  title: string;
  description: string;
  icon: typeof KeyRound;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="neo-card w-full p-0 text-left transition hover:brightness-[1.02]"
    >
      <div className="flex items-start gap-4 p-4">
        <div className="rounded-xl bg-[var(--neo-surface-muted)] p-3 text-[var(--neo-primary-deep)] shadow-bl-sm">
          <Icon className="size-5" />
        </div>
        <div className="space-y-1">
          <div className="text-sm font-semibold text-[var(--neo-ink)]">{title}</div>
          <div className="text-sm leading-6 text-[var(--neo-muted)]">{description}</div>
        </div>
      </div>
    </button>
  );
}

export function AccountImportDialog({ disabled, onImported }: Props) {
  const [open, setOpen] = useState(false);
  const [method, setMethod] = useState<ImportMethod>("menu");
  const [tokenInput, setTokenInput] = useState("");
  const [sessionInput, setSessionInput] = useState("");
  const [oauthEmailHint, setOauthEmailHint] = useState("");
  const [oauthSession, setOauthSession] = useState<{ session_id: string; authorize_url: string } | null>(null);
  const [oauthCallbackInput, setOauthCallbackInput] = useState("");
  const [oauthStarting, setOauthStarting] = useState(false);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState("");
  const [error, setError] = useState("");
  const txtRef = useRef<HTMLInputElement>(null);
  const jsonRef = useRef<HTMLInputElement>(null);

  const reset = () => {
    setMethod("menu");
    setTokenInput("");
    setSessionInput("");
    setOauthEmailHint("");
    setOauthSession(null);
    setOauthCallbackInput("");
    setMessage("");
    setError("");
  };

  const close = () => {
    setOpen(false);
    reset();
  };

  const submitImport = async (tokens: string[], accounts: AccountImportPayload[] = []) => {
    const normalized = tokens.map((t) => t.trim()).filter(Boolean);
    if (normalized.length === 0) {
      setError("请先提供至少一个可用 Token");
      return;
    }
    setBusy(true);
    setError("");
    setMessage("");
    try {
      const data = await accountsApi.create(normalized, accounts);
      onImported();
      close();
      setMessage(
        `导入完成：新增 ${data.added ?? 0}，跳过 ${data.skipped ?? 0}，更新 ${data.updated ?? 0}`,
      );
    } catch (err) {
      setError(err instanceof Error ? err.message : "导入失败");
    } finally {
      setBusy(false);
    }
  };

  const handleTokenImport = () => void submitImport(splitTokens(tokenInput));

  const handleSessionImport = () => {
    try {
      const payload = JSON.parse(sessionInput) as unknown;
      const token = getSessionAccessToken(payload);
      if (!token) {
        setError("未从 Session JSON 中提取到 accessToken");
        return;
      }
      void submitImport([token]);
    } catch {
      setError("Session JSON 解析失败");
    }
  };

  const handleTxtFile = async (file: File | undefined) => {
    if (!file) return;
    try {
      const content = await readFileAsText(file);
      const tokens = splitTokens(content);
      if (tokens.length === 0) {
        setError("TXT 文件里没有有效 Token");
        return;
      }
      setTokenInput((prev) => [...splitTokens(prev), ...tokens].join("\n"));
      setError("");
    } catch (err) {
      setError(err instanceof Error ? err.message : "读取 TXT 失败");
    }
  };

  const handleJsonFiles = async (files: FileList | null) => {
    if (!files?.length) return;
    try {
      const accounts = (
        await Promise.all(
          Array.from(files).map(async (file) => {
            const raw = await readFileAsText(file);
            return getAccountJsonAccounts(JSON.parse(raw) as unknown);
          }),
        )
      ).flat();
      if (accounts.length === 0) {
        setError("JSON 文件中没有可用 access_token");
        return;
      }
      void submitImport(
        accounts.map((a) => a.access_token),
        accounts,
      );
    } catch (err) {
      setError(err instanceof Error ? err.message : "读取 JSON 失败");
    }
  };

  const handleStartOAuth = async () => {
    setOauthStarting(true);
    setError("");
    try {
      const data = await accountsApi.oauthStart(oauthEmailHint.trim());
      setOauthSession({ session_id: data.session_id, authorize_url: data.authorize_url });
      window.open(data.authorize_url, "_blank", "noopener,noreferrer");
    } catch (err) {
      setError(err instanceof Error ? err.message : "OAuth 起始失败");
    } finally {
      setOauthStarting(false);
    }
  };

  const handleFinishOAuth = async () => {
    if (!oauthSession) {
      setError("请先打开授权页面");
      return;
    }
    const trimmed = oauthCallbackInput.trim();
    if (!trimmed) {
      setError("请粘贴 callback URL");
      return;
    }
    setBusy(true);
    setError("");
    try {
      const data = await accountsApi.oauthFinish(oauthSession.session_id, trimmed);
      onImported();
      close();
      setMessage(
        `OAuth 完成：新增 ${data.added ?? 0}，更新 ${data.updated ?? 0}，刷新 ${data.refreshed ?? 0}`,
      );
    } catch (err) {
      setError(err instanceof Error ? err.message : "OAuth 换 token 失败");
    } finally {
      setBusy(false);
    }
  };

  const handleCopyAuthorizeUrl = async () => {
    if (!oauthSession?.authorize_url) return;
    try {
      await navigator.clipboard.writeText(oauthSession.authorize_url);
      setMessage("授权 URL 已复制");
    } catch {
      setError("复制失败，请手动复制");
    }
  };

  const title =
    method === "menu"
      ? "导入账户"
      : method === "token"
        ? "导入 Access Token"
        : method === "session"
          ? "导入 Session JSON"
          : method === "oauth"
            ? "OAuth 登录导入"
            : "导入账号 JSON";

  return (
    <>
      <Button size="sm" className="h-8 gap-1.5" onClick={() => setOpen(true)} disabled={disabled}>
        <Upload className="size-3.5" /> 导入
      </Button>

      {message ? (
        <span className="text-xs text-emerald-600">{message}</span>
      ) : null}

      {open ? (
        <div className="fixed inset-0 z-[100] flex items-center justify-center bg-black/30 p-4 backdrop-blur-sm">
          <div className="neo-card max-h-[90vh] w-full max-w-lg overflow-y-auto p-5 shadow-bl-lg">
            <div className="mb-4 flex items-start justify-between gap-3">
              <div>
                <h2 className="text-base font-semibold text-[var(--neo-ink)]">{title}</h2>
                <p className="mt-1 text-sm text-[var(--neo-muted)]">
                  {method === "menu"
                    ? "选择导入方式，成功后会写入本地号池文件。"
                    : method === "token"
                      ? "一行一个 Token，也支持从 TXT 读取。"
                      : method === "session"
                        ? "粘贴 chatgpt.com session 接口返回的 JSON。"
                        : method === "oauth"
                          ? "通过 TNexus 独立 OAuth 服务授权导入（不依赖 gptimage HTTP）。"
                          : "支持单账号对象或账号数组 JSON 文件。"}
                </p>
              </div>
              <button
                type="button"
                className="rounded-md p-1 text-[var(--neo-muted)] hover:bg-[var(--neo-surface-muted)]"
                onClick={close}
                aria-label="关闭"
              >
                <X className="size-4" />
              </button>
            </div>

            {error ? (
              <div className="mb-3 rounded-lg border border-red-200 bg-red-50 px-3 py-2 text-sm text-red-700">
                {error}
              </div>
            ) : null}

            {method === "menu" ? (
              <div className="space-y-2">
                <MethodCard
                  title="导入 Access Token"
                  description="粘贴或从 TXT 读取，一行一个。"
                  icon={KeyRound}
                  onClick={() => setMethod("token")}
                />
                <MethodCard
                  title="导入 Session JSON"
                  description="从 session 接口复制 JSON，自动提取 accessToken。"
                  icon={FileJson}
                  onClick={() => setMethod("session")}
                />
                <MethodCard
                  title="OAuth 登录导入"
                  description="浏览器授权后粘贴 callback URL，自动换 token 入库。"
                  icon={ExternalLink}
                  onClick={() => setMethod("oauth")}
                />
                <MethodCard
                  title="导入账号 JSON 文件"
                  description="支持导出的单账号或数组格式。"
                  icon={FileText}
                  onClick={() => setMethod("account-json")}
                />
              </div>
            ) : null}

            {method === "token" ? (
              <div className="space-y-3">
                <button
                  type="button"
                  className="text-sm text-[var(--neo-muted)] hover:text-[var(--neo-ink)]"
                  onClick={() => setMethod("menu")}
                >
                  ← 返回
                </button>
                <Textarea
                  placeholder="每行一个 Access Token…"
                  value={tokenInput}
                  onChange={(e) => setTokenInput(e.target.value)}
                  className="min-h-48 font-mono text-xs"
                />
                <Button variant="outline" size="sm" onClick={() => txtRef.current?.click()} disabled={busy}>
                  选择 TXT 文件
                </Button>
                <input
                  ref={txtRef}
                  type="file"
                  accept=".txt,text/plain"
                  className="hidden"
                  onChange={(e) => void handleTxtFile(e.target.files?.[0])}
                />
              </div>
            ) : null}

            {method === "session" ? (
              <div className="space-y-3">
                <button
                  type="button"
                  className="text-sm text-[var(--neo-muted)] hover:text-[var(--neo-ink)]"
                  onClick={() => setMethod("menu")}
                >
                  ← 返回
                </button>
                <p className="text-xs leading-6 text-[var(--neo-muted)]">
                  打开{" "}
                  <a href={SESSION_URL} target="_blank" rel="noreferrer" className="underline">
                    {SESSION_URL}
                  </a>{" "}
                  复制完整 JSON。
                </p>
                <Textarea
                  placeholder='粘贴包含 "accessToken" 的 JSON…'
                  value={sessionInput}
                  onChange={(e) => setSessionInput(e.target.value)}
                  className="min-h-48 font-mono text-xs"
                />
              </div>
            ) : null}

            {method === "oauth" ? (
              <div className="space-y-3">
                <button
                  type="button"
                  className="text-sm text-[var(--neo-muted)] hover:text-[var(--neo-ink)]"
                  onClick={() => setMethod("menu")}
                >
                  ← 返回
                </button>
                <Input
                  placeholder="邮箱（可选预填）"
                  value={oauthEmailHint}
                  onChange={(e) => setOauthEmailHint(e.target.value)}
                  disabled={Boolean(oauthSession) || oauthStarting}
                />
                {!oauthSession ? (
                  <Button size="sm" onClick={() => void handleStartOAuth()} disabled={oauthStarting}>
                    {oauthStarting ? <LoaderCircle className="size-4 animate-spin" /> : <ExternalLink className="size-3.5" />}
                    打开授权页面
                  </Button>
                ) : (
                  <div className="space-y-2">
                    <div className="rounded-lg border border-[var(--neo-border)] bg-[var(--neo-surface-muted)] p-2 text-xs break-all font-mono">
                      {oauthSession.authorize_url}
                    </div>
                    <div className="flex flex-wrap gap-2">
                      <Button size="sm" variant="outline" onClick={() => void handleCopyAuthorizeUrl()}>
                        <Copy className="size-3.5" /> 复制 URL
                      </Button>
                      <Button
                        size="sm"
                        variant="outline"
                        onClick={() => window.open(oauthSession.authorize_url, "_blank", "noopener,noreferrer")}
                      >
                        再次打开
                      </Button>
                    </div>
                    <Textarea
                      placeholder="粘贴 callback URL（含 code=...）"
                      value={oauthCallbackInput}
                      onChange={(e) => setOauthCallbackInput(e.target.value)}
                      className="min-h-24 font-mono text-xs"
                    />
                  </div>
                )}
              </div>
            ) : null}

            {method === "account-json" ? (
              <div className="space-y-3">
                <button
                  type="button"
                  className="text-sm text-[var(--neo-muted)] hover:text-[var(--neo-ink)]"
                  onClick={() => setMethod("menu")}
                >
                  ← 返回
                </button>
                <Button size="sm" onClick={() => jsonRef.current?.click()} disabled={busy}>
                  选择 JSON 文件
                </Button>
                <input
                  ref={jsonRef}
                  type="file"
                  accept=".json,application/json"
                  multiple
                  className="hidden"
                  onChange={(e) => void handleJsonFiles(e.target.files)}
                />
              </div>
            ) : null}

            <div className="mt-5 flex justify-end gap-2">
              <Button variant="outline" onClick={close} disabled={busy}>
                取消
              </Button>
              {method === "token" ? (
                <Button onClick={handleTokenImport} disabled={busy}>
                  {busy ? <LoaderCircle className="size-4 animate-spin" /> : null}
                  导入 Token
                </Button>
              ) : null}
              {method === "session" ? (
                <Button onClick={handleSessionImport} disabled={busy}>
                  {busy ? <LoaderCircle className="size-4 animate-spin" /> : null}
                  导入 JSON
                </Button>
              ) : null}
              {method === "oauth" && oauthSession ? (
                <Button onClick={() => void handleFinishOAuth()} disabled={busy}>
                  {busy ? <LoaderCircle className="size-4 animate-spin" /> : null}
                  完成 OAuth 导入
                </Button>
              ) : null}
            </div>
          </div>
        </div>
      ) : null}
    </>
  );
}
