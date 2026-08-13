import { test } from "node:test";
import assert from "node:assert/strict";

// ---- helpers inlined from web/src/lib/nav.ts ----------------------------------------

const NAV_ENTRIES = [
  { href: "/studio/",        label: "TNexus",   area: "体验端",   adminOnly: false, studioHome: true },
  { href: "/chat/",          label: "对话",      area: "体验端",   adminOnly: false },
  { href: "/grok/chat/",     label: "Grok 对话", area: "体验端",   adminOnly: false },
  { href: "/image-manager/", label: "图片管理",  area: "体验端",   adminOnly: false },
  { href: "/accounts/",      label: "号池管理",  area: "GPT管理",  adminOnly: true  },
  { href: "/ops/",           label: "运维",      area: "GPT管理",  adminOnly: true  },
  { href: "/grok/accounts/", label: "Grok 管理", area: "Grok管理", adminOnly: true  },
  { href: "/grok/dashboard/",label: "Grok 概览", area: "Grok管理", adminOnly: true, hiddenInNav: true },
  { href: "/grok/audits/",   label: "Grok 审计", area: "Grok管理", adminOnly: true, hiddenInNav: true },
  { href: "/grok/keys/",     label: "Grok 密钥", area: "Grok管理", adminOnly: true, hiddenInNav: true },
  { href: "/grok/models/",   label: "Grok 模型", area: "Grok管理", adminOnly: true, hiddenInNav: true },
  { href: "/grok/settings/", label: "Grok 设置", area: "Grok管理", adminOnly: true, hiddenInNav: true },
  { href: "/logs/",          label: "日志管理",  area: "共用",     adminOnly: true  },
  { href: "/settings/",      label: "设置",      area: "共用",     adminOnly: false },
];

function normPath(pathname) {
  if (!pathname || pathname === "/") return "/";
  return pathname.endsWith("/") ? pathname : `${pathname}/`;
}

function isNavActive(pathname, entry) {
  const p = normPath(pathname);
  if (entry.studioHome) {
    return p === "/studio/" || p.startsWith("/history/");
  }
  return p === entry.href || p.startsWith(entry.href);
}

function isAdminRoute(pathname) {
  const p = normPath(pathname);
  return NAV_ENTRIES.some(
    (e) => e.adminOnly && (p === e.href || p.startsWith(e.href)),
  );
}

function filterNavForRole(entries, isAdmin) {
  return entries.filter((e) => !e.hiddenInNav && (!e.adminOnly || isAdmin));
}

// ---- 未登记的 Grok 子页面会绕过门控（真实漏洞回归测试）--------------------------

test("所有 Grok 管理子页面都受门控，/grok/chat 除外", () => {
  for (const p of [
    "/grok/accounts/",
    "/grok/dashboard/",
    "/grok/audits/",
    "/grok/keys/",
    "/grok/models/",
    "/grok/settings/",
  ]) {
    assert.equal(isAdminRoute(p), true, `${p} 必须要求管理员`);
  }
  assert.equal(isAdminRoute("/grok/chat/"), false, "Grok 对话属体验端");
});

test("Grok 子页面不占顶栏位置，但管理员仍可访问", () => {
  const shown = filterNavForRole(NAV_ENTRIES, true).map((e) => e.href);
  assert.ok(!shown.includes("/grok/keys/"), "密钥页不应出现在导航");
  assert.ok(shown.includes("/grok/accounts/"), "Grok 管理入口应保留");
  assert.equal(isAdminRoute("/grok/keys/"), true);
});

test("体验端路由对普通用户开放", () => {
  for (const p of ["/studio/", "/history/detail/", "/chat/", "/image-manager/", "/settings/"]) {
    assert.equal(isAdminRoute(p), false, `${p} 不应要求管理员`);
  }
});

// ---- route → area mapping ------------------------------------------------------------

test("studio maps to 体验端", () => {
  const entry = NAV_ENTRIES.find((e) => e.href === "/studio/");
  assert.equal(entry?.area, "体验端");
});

test("chat maps to 体验端", () => {
  const entry = NAV_ENTRIES.find((e) => e.href === "/chat/");
  assert.equal(entry?.area, "体验端");
});

test("grok/chat maps to 体验端 and is NOT adminOnly", () => {
  const entry = NAV_ENTRIES.find((e) => e.href === "/grok/chat/");
  assert.equal(entry?.area, "体验端");
  assert.equal(entry?.adminOnly, false);
});

test("image-manager maps to 体验端", () => {
  const entry = NAV_ENTRIES.find((e) => e.href === "/image-manager/");
  assert.equal(entry?.area, "体验端");
});

test("accounts maps to GPT管理 and is adminOnly", () => {
  const entry = NAV_ENTRIES.find((e) => e.href === "/accounts/");
  assert.equal(entry?.area, "GPT管理");
  assert.equal(entry?.adminOnly, true);
});

test("ops maps to GPT管理 and is adminOnly", () => {
  const entry = NAV_ENTRIES.find((e) => e.href === "/ops/");
  assert.equal(entry?.area, "GPT管理");
  assert.equal(entry?.adminOnly, true);
});

test("grok/accounts maps to Grok管理 and is adminOnly", () => {
  const entry = NAV_ENTRIES.find((e) => e.href === "/grok/accounts/");
  assert.equal(entry?.area, "Grok管理");
  assert.equal(entry?.adminOnly, true);
});

test("logs maps to 共用 and is adminOnly", () => {
  const entry = NAV_ENTRIES.find((e) => e.href === "/logs/");
  assert.equal(entry?.area, "共用");
  assert.equal(entry?.adminOnly, true);
});

test("settings maps to 共用 and is NOT adminOnly", () => {
  const entry = NAV_ENTRIES.find((e) => e.href === "/settings/");
  assert.equal(entry?.area, "共用");
  assert.equal(entry?.adminOnly, false);
});

test("路由表登记了全部页面：9 个上导航 + 5 个仅门控的 Grok 子页", () => {
  assert.equal(NAV_ENTRIES.length, 14);
  assert.equal(NAV_ENTRIES.filter((e) => e.hiddenInNav).length, 5);
});

// ---- admin-only filtering ------------------------------------------------------------

test("admin sees all 9 nav entries", () => {
  const visible = filterNavForRole(NAV_ENTRIES, true);
  assert.equal(visible.length, 9);
});

test("non-admin sees only non-adminOnly entries (5 items)", () => {
  const visible = filterNavForRole(NAV_ENTRIES, false);
  assert.equal(visible.length, 5);
  assert.ok(visible.every((e) => !e.adminOnly), "non-admin must not see adminOnly entries");
});

test("non-admin sees studio, chat, grok/chat, image-manager, settings", () => {
  const visible = filterNavForRole(NAV_ENTRIES, false);
  const hrefs = visible.map((e) => e.href);
  for (const expected of ["/studio/", "/chat/", "/grok/chat/", "/image-manager/", "/settings/"]) {
    assert.ok(hrefs.includes(expected), `missing ${expected}`);
  }
});

test("non-admin does NOT see accounts, ops, grok/accounts, logs", () => {
  const visible = filterNavForRole(NAV_ENTRIES, false);
  const hrefs = visible.map((e) => e.href);
  for (const forbidden of ["/accounts/", "/ops/", "/grok/accounts/", "/logs/"]) {
    assert.ok(!hrefs.includes(forbidden), `non-admin should not see ${forbidden}`);
  }
});

test("admin sees accounts, ops, grok/accounts, logs", () => {
  const visible = filterNavForRole(NAV_ENTRIES, true);
  const hrefs = visible.map((e) => e.href);
  for (const expected of ["/accounts/", "/ops/", "/grok/accounts/", "/logs/"]) {
    assert.ok(hrefs.includes(expected), `admin should see ${expected}`);
  }
});

// ---- active-route matching -----------------------------------------------------------

test("/studio/ is active for studio entry", () => {
  const entry = NAV_ENTRIES.find((e) => e.href === "/studio/");
  assert.equal(isNavActive("/studio/", entry), true);
});

// studioHome 匹配逻辑：仅精确匹配 /studio/ 或前缀匹配 /history/，不做通用 /studio/* 前缀匹配
test("/studio/detail is NOT active for studio entry (studioHome exact-only)", () => {
  const entry = NAV_ENTRIES.find((e) => e.href === "/studio/");
  assert.equal(isNavActive("/studio/detail", entry), false);
});

test("/history/ is active for studio entry (studioHome special case)", () => {
  const entry = NAV_ENTRIES.find((e) => e.href === "/studio/");
  assert.equal(isNavActive("/history/", entry), true);
});

test("/history/abc/123 is active for studio entry", () => {
  const entry = NAV_ENTRIES.find((e) => e.href === "/studio/");
  assert.equal(isNavActive("/history/abc/123", entry), true);
});

test("/accounts/ is NOT active for studio entry", () => {
  const entry = NAV_ENTRIES.find((e) => e.href === "/studio/");
  assert.equal(isNavActive("/accounts/", entry), false);
});

test("/grok/chat/ is active for grok/chat entry", () => {
  const entry = NAV_ENTRIES.find((e) => e.href === "/grok/chat/");
  assert.equal(isNavActive("/grok/chat/", entry), true);
});

test("/grok/chat/ is NOT active for grok/accounts entry", () => {
  const entry = NAV_ENTRIES.find((e) => e.href === "/grok/accounts/");
  assert.equal(isNavActive("/grok/chat/", entry), false);
});

test("/grok/accounts/123 is active for grok/accounts entry (prefix)", () => {
  const entry = NAV_ENTRIES.find((e) => e.href === "/grok/accounts/");
  assert.equal(isNavActive("/grok/accounts/123", entry), true);
});

test("/settings/ is active for settings entry", () => {
  const entry = NAV_ENTRIES.find((e) => e.href === "/settings/");
  assert.equal(isNavActive("/settings/", entry), true);
});

test("/settings without trailing slash is active (normPath fix)", () => {
  const entry = NAV_ENTRIES.find((e) => e.href === "/settings/");
  assert.equal(isNavActive("/settings", entry), true);
});

// ---- isAdminRoute --------------------------------------------------------------------

test("/accounts/ is an admin route", () => assert.equal(isAdminRoute("/accounts/"), true));
test("/ops/ is an admin route",      () => assert.equal(isAdminRoute("/ops/"), true));
test("/logs/ is an admin route",     () => assert.equal(isAdminRoute("/logs/"), true));
test("/grok/accounts/ is an admin route", () => assert.equal(isAdminRoute("/grok/accounts/"), true));

test("/grok/accounts/keys is an admin route (sub-path)", () => {
  assert.equal(isAdminRoute("/grok/accounts/keys"), true);
});

test("/studio/ is NOT an admin route",        () => assert.equal(isAdminRoute("/studio/"), false));
test("/chat/ is NOT an admin route",          () => assert.equal(isAdminRoute("/chat/"), false));
test("/grok/chat/ is NOT an admin route",     () => assert.equal(isAdminRoute("/grok/chat/"), false));
test("/image-manager/ is NOT an admin route", () => assert.equal(isAdminRoute("/image-manager/"), false));
test("/settings/ is NOT an admin route",      () => assert.equal(isAdminRoute("/settings/"), false));

// ---- normPath ------------------------------------------------------------------------

test("normPath adds trailing slash when missing", () => {
  assert.equal(normPath("/accounts"), "/accounts/");
});

test("normPath preserves existing trailing slash", () => {
  assert.equal(normPath("/accounts/"), "/accounts/");
});

test("normPath handles '/' root", () => {
  assert.equal(normPath("/"), "/");
});

test("normPath handles empty string", () => {
  assert.equal(normPath(""), "/");
});
