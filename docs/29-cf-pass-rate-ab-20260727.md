# 29 — Phase B′ 判据 2：CF 通过率 A/B（2026-07-27）

**状态**：脚本就位；**上游 CF 探针需 CF 可测窗 + 号池账号** 后执行。

## 1. 范围

| 项 | 说明 |
|----|------|
| 判据 | 同号池、同出口下，wreq 路径 CF 通过率 **不劣于** curl_cffi 基线 |
| Profile | **chrome124 / chrome131**（弃用 chrome120，见 [27](27-tls-fingerprint-spike-20260726.md)） |
| 代理 | 默认 `SPIKE_PROXY=http://127.0.0.1:7897`（本机 egress） |

## 2. 运行

```bash
# TLS 指纹层 A/B（已实现）
SPIKE_PROXY=http://127.0.0.1:7897 python3 scripts/cf_pass_rate_ab.py --rounds 2

# 输出：data/runlogs/cf-pass-rate-ab-latest.json
```

脚本当前对比 **TLS 探测**（browserleaks）两侧是否一致；**业务 CF403 探针**需在 CF 可测窗追加：

1. 从号池取 pin 账号 + 代理（`secrets/pin_account.json`）
2. 对 `chatgpt.com` 或生产同等探针 URL 发 N 轮 prepare/start
3. 统计 `upstream` 中 `cf_edge_block` / HTTP 403 比例

## 3. 出门

- [ ] curl_cffi 基线：≥1 轮成功过 CF（非 self 失败）
- [ ] wreq 同条件通过率 ≥ 基线
- [ ] 记录 runlog 路径进 `docs/18-test-matrix.md` 签字栏

## 4. 关联

- [27-tls-fingerprint-spike-20260726.md](27-tls-fingerprint-spike-20260727.md) — 判据 1
- `scripts/cf_pass_rate_ab.py`
