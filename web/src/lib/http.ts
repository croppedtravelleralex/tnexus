/**
 * 各 API 客户端共用的 HTTP 工具函数。
 * 基础 URL 解析统一走 api-base.ts；禁止在此直读 process.env.NEXT_PUBLIC_API_BASE。
 */

/**
 * 统一 unwrap 列表接口响应（裸数组 / {items:[...]} 均可；其余返回空数组）。
 * 修复根因：grok-admin GET /admin/accounts/:id/quota 返回 { items:[...] }，
 * 旧客户端误当成裸数组取 value[0]，导致页面显示「未知」。
 */
export function unwrapItems<T>(raw: unknown): T[] {
  if (Array.isArray(raw)) return raw as T[];
  if (raw && typeof raw === "object" && Array.isArray((raw as { items?: unknown }).items)) {
    return (raw as { items: T[] }).items;
  }
  return [];
}

/**
 * 从 HTTP 错误响应体文本中提取可读消息。
 * 依次尝试：`error`（字符串 / 嵌套 `{message}`）→ `message` → 原始文本 → statusText。
 * 覆盖四处已知重复提取块（api.ts × 2、grok-api.ts × 1、已删除的 chatApi.generateImage × 1）。
 */
export function extractErrorMessage(text: string, statusText: string): string {
  const fallback = text || statusText;
  try {
    const json = JSON.parse(text) as {
      error?: string | { message?: string };
      message?: string;
    };
    if (json.error) {
      if (typeof json.error === "string") return json.error;
      if (typeof json.error === "object" && json.error.message) return json.error.message;
    }
    if (json.message) return json.message;
  } catch {
    // 保留原始文本
  }
  return fallback;
}
