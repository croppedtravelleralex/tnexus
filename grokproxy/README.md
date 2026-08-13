# grokProxy

把注册机产出的 Grok 账号收成一个池子，对外暴露 OpenAI 兼容接口。
一个二进制 + 一个 SQLite 文件，没有外部数据库依赖。

## 它做四件事

| | |
|---|---|
| **入库** | `POST /api/v1/accounts` 接收注册机产出的 Build / Web 凭证 |
| **保鲜** | 派号前自动刷新 access token，**并把轮换后的 refresh token 写回** |
| **调度** | 按最久未使用挑号，失败按类型冷却或摘除，一次请求最多换 3 个号 |
| **服务** | `/v1/models`、`/v1/chat/completions`、`/v1/responses` |

## 两条硬约束（都是踩过坑换来的）

**1. refresh token 必须回写。** xAI 每次刷新都会轮换 refresh token 并**立刻吊销旧的**。
任何"刷新了但没保存新 token"的代码路径都会永久废掉那个账号。本项目只有
`Store::save_refreshed` 一个写入口，`Pool::acquire_build` 在使用新 token **之前**先落盘。

**2. 模型名不能硬编码。** 上游会不打招呼地改名（2026-08-13 把 `grok-4.5` 换成
`grok-4.6`），任何 `model == "grok-4.5"` 的等值判断都会把健康账号判死。
`pick_chat_model()` 从 `/models` 实际返回里选版本最高的 `grok-<major>.<minor>`；
客户端就算请求旧名字，也会被自动纠正。

## 账号状态机

| 状态 | 含义 | 会自愈吗 |
|------|------|---------|
| `active` | 可调度 | — |
| `cooling` | 限流 / 额度窗口 / 网络瞬断，带到期时间 | 会 |
| `needs_reauth` | refresh token 被吊销，必须重新导入凭证 | 不会 |
| `forbidden` | 上游拒绝该号的 chat 权益 | 不会 |
| `disabled` | 人工停用 | 不会 |

把 `cooling` 和 `needs_reauth` 分开是关键：混在一起会让"只是被限流"的池子看着像全死了。

## 本地跑

```bash
cargo test
GROKPROXY_ADMIN_KEY=dev GROKPROXY_DB=./grokproxy.db \
GROKPROXY_PROXY=http://127.0.0.1:7897 \
cargo run --release
```

端到端冒烟（会真的打一次上游）：

```bash
python scripts/smoke_import_and_chat.py \
  --admin-key dev \
  --auth-file /path/to/cpa_auths/xai-someone@yumail.co.json
```

## 配置

| 环境变量 | 默认 | 说明 |
|---------|------|------|
| `GROKPROXY_ADDR` | `0.0.0.0:8110` | 监听地址 |
| `GROKPROXY_DB` | `/data/grokproxy.db` | SQLite 路径 |
| `GROKPROXY_UPSTREAM` | `https://cli-chat-proxy.grok.com/v1` | 上游根 |
| `GROKPROXY_API_KEY` | 空 | `/v1/*` 的 Bearer；空=不校验 |
| `GROKPROXY_ADMIN_KEY` | 空 | `/api/v1/*` 与状态页的 Bearer；空=不校验（会告警） |
| `GROKPROXY_PROXY` | 空 | 账号无 `proxy_url` 时的默认出口 |
| `GROKPROXY_TIMEOUT_SECS` | `120` | 上游超时 |
| `GROKPROXY_MAX_ATTEMPTS` | `3` | 单请求最多换几个号 |

## 接口

```
GET  /healthz                     进程活着
GET  /readyz                      有可调度账号才 200，否则 503
GET  /admin                       状态页（浏览器打开，页内填 admin key）

GET  /v1/models                   OpenAI 兼容
POST /v1/chat/completions         OpenAI 兼容
POST /v1/responses                CLI 形态透传

GET  /api/v1/stats                按通道/状态计数
GET  /api/v1/accounts             账号列表（不含任何 token）
POST /api/v1/accounts             导入账号
POST /api/v1/accounts/{id}/health 人工改状态
```

导入格式（注册机上传用）：

```json
{
  "provider": "build",
  "accounts": [
    {
      "email": "someone@yumail.co",
      "access_token": "...",
      "refresh_token": "...",
      "expires_at": 1786600000,
      "proxy_url": "http://mail-someone:sticky@172.20.0.1:18100",
      "headers": {"X-XAI-Token-Auth": "xai-grok-cli"}
    }
  ]
}
```

同一邮箱重复导入是**更新**而不是新增；空字段不会覆盖已有值，所以只带
`proxy_url` 的补充提交不会把 token 抹掉。带新凭证的重新导入会把
`needs_reauth` 的号救活。

## 部署

铁律：**不在 Panda 上编译**。链路是 `git push → GitHub Actions → GHCR → panda pull`。

```bash
# panda 上首次
mkdir -p /opt/grokproxy/data
git clone <repo> /opt/grokproxy/repo
cp /opt/grokproxy/repo/deploy/panda/.env.example /opt/grokproxy/.env
# 编辑 .env 填两个 key

# 每次发布
cd /opt/grokproxy/repo && git pull
bash deploy/panda/deploy.sh          # pull + up，绝不 build
bash deploy/panda/deploy.sh rollback # 回滚上一版
```

## 还没做

- **Web SSO 账号只入库和调度，chat 路径未实现。** grok.com 的 Web 接口需要请求
  签名（statsig / 指纹），那是独立的一大块；当前 `provider=web` 的号可以存、可以
  在状态页看到，但 `/v1/chat/completions` 只走 Build 通道。
- 流式响应（`stream: true`）目前按非流式转发。
