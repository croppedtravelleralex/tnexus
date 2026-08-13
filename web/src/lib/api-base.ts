/**
 * 浏览器里的 API / Gateway 根地址。
 *
 * 生产静态页若把 NEXT_PUBLIC_API_BASE 打成 http://localhost:9000，
 * 用户打开 https://tnexus.relai.asia 时会打到自己电脑。
 * 非本机 hostname 一律走同域（nginx 反代 :9000 / :8014）。
 */
function isBrowserRemoteHost(): boolean {
  if (typeof window === "undefined") return false;
  const host = window.location.hostname;
  return host !== "localhost" && host !== "127.0.0.1" && host !== "[::1]";
}

function trimSlash(value: string): string {
  return value.replace(/\/$/, "");
}

export function apiBase(): string {
  if (isBrowserRemoteHost()) return "";
  const baked = process.env.NEXT_PUBLIC_API_BASE;
  if (baked && baked.length > 0) return trimSlash(baked);
  return "http://localhost:9000";
}

export function gatewayBase(): string {
  if (isBrowserRemoteHost()) return "";
  const baked = process.env.NEXT_PUBLIC_GATEWAY_BASE;
  if (baked && baked.length > 0) return trimSlash(baked);
  return "http://localhost:8014";
}

export function apiBaseLabel(): string {
  const base = apiBase();
  if (base.length > 0) return base;
  if (typeof window !== "undefined") return window.location.origin;
  return "same-origin";
}
