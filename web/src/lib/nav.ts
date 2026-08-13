/**
 * 单一权威路由表：导航栏 + 管理员访问门控均从此读取。
 * 未来拆分体验端独立部署时，只需从此表读取 area 字段即可。
 */

export type NavArea = "体验端" | "GPT管理" | "Grok管理" | "共用";

export interface NavEntry {
  readonly href: string;
  readonly label: string;
  readonly area: NavArea;
  /** 仅管理员（role === "admin"）可见、可访问 */
  readonly adminOnly: boolean;
  /** /studio/ 条目同时高亮 /history/ 下所有子页面 */
  readonly studioHome?: boolean;
  /**
   * 不在顶栏渲染，但仍参与访问门控。
   * Grok 控制台的子页面从 /grok/accounts/ 内部跳转进入，不占顶栏位置；
   * 漏登记会让它们绕过 isAdminRoute（/grok/keys 会直接暴露 API 密钥页）。
   */
  readonly hiddenInNav?: boolean;
}

export const NAV_ENTRIES: readonly NavEntry[] = [
  { href: "/studio/",        label: "TNexus",   area: "体验端",  adminOnly: false, studioHome: true },
  { href: "/chat/",          label: "对话",      area: "体验端",  adminOnly: false },
  { href: "/grok/chat/",     label: "Grok 对话", area: "体验端",  adminOnly: false },
  { href: "/image-manager/", label: "图片管理",  area: "体验端",  adminOnly: false },
  { href: "/accounts/",      label: "号池管理",  area: "GPT管理", adminOnly: true  },
  { href: "/ops/",           label: "运维",      area: "GPT管理", adminOnly: true  },
  { href: "/grok/accounts/", label: "Grok 管理", area: "Grok管理",adminOnly: true  },
  { href: "/grok/dashboard/",label: "Grok 概览", area: "Grok管理",adminOnly: true, hiddenInNav: true },
  { href: "/grok/audits/",   label: "Grok 审计", area: "Grok管理",adminOnly: true, hiddenInNav: true },
  { href: "/grok/keys/",     label: "Grok 密钥", area: "Grok管理",adminOnly: true, hiddenInNav: true },
  { href: "/grok/models/",   label: "Grok 模型", area: "Grok管理",adminOnly: true, hiddenInNav: true },
  { href: "/grok/settings/", label: "Grok 设置", area: "Grok管理",adminOnly: true, hiddenInNav: true },
  { href: "/logs/",          label: "日志管理",  area: "共用",    adminOnly: true  },
  { href: "/settings/",      label: "设置",      area: "共用",    adminOnly: false },
];

/** 桌面端和移动端导航中各区域的展示顺序 */
export const NAV_AREA_ORDER: readonly NavArea[] = [
  "体验端",
  "GPT管理",
  "Grok管理",
  "共用",
];

/** 移动端菜单中各区域的中文标签 */
export const NAV_AREA_LABELS: Record<NavArea, string> = {
  "体验端":  "体验",
  "GPT管理": "GPT 管理",
  "Grok管理":"Grok 管理",
  "共用":    "通用",
};

export function normPath(pathname: string): string {
  if (!pathname || pathname === "/") return "/";
  return pathname.endsWith("/") ? pathname : `${pathname}/`;
}

export function isNavActive(
  pathname: string,
  entry: Pick<NavEntry, "href" | "studioHome">,
): boolean {
  const p = normPath(pathname);
  if (entry.studioHome) {
    return p === "/studio/" || p.startsWith("/history/");
  }
  return p === entry.href || p.startsWith(entry.href);
}

/**
 * 判断当前路径是否落在任意仅管理员路由下，用于客户端门控。
 * 注意：这是纵深防御与 UX，安全边界由服务端 API 强制执行。
 */
export function isAdminRoute(pathname: string): boolean {
  const p = normPath(pathname);
  return NAV_ENTRIES.some(
    (e) => e.adminOnly && (p === e.href || p.startsWith(e.href)),
  );
}

/** 按角色过滤导航条目；hiddenInNav 的条目只参与门控，不渲染。 */
export function filterNavForRole(
  entries: readonly NavEntry[],
  isAdmin: boolean,
): NavEntry[] {
  return entries.filter((e) => !e.hiddenInNav && (!e.adminOnly || isAdmin));
}
