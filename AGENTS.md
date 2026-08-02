# TNexus — Agent 必读

## 生产部署（最高优先级）

**禁止在 Panda 生产机上构建。** 详见 [.cursor/rules/panda-no-remote-build.mdc](.cursor/rules/panda-no-remote-build.mdc)。

发布链路：`本地/CI 构建 → git push → GHCR Actions → Panda 仅 `deploy.sh`（pull + up）`。

Panda 上执行 `docker build` / `cargo build` / `npm run build` 属 **重大事故**（曾导致 CPU/内存爆满）。

## 文档入口

- [HANDOFF.md](HANDOFF.md) — 当前状态与读序
- [plan.md](plan.md) — 施工总控与红线
- [web/AGENTS.md](web/AGENTS.md) — Next.js 前端约定
