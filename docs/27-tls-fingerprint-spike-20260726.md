# 27 — Phase B′ TLS 指纹等价性实测（2026-07-26）

结论先行：**`wreq` 能复现 curl_cffi 的 TLS 指纹。判据 1 通过（chrome124 / chrome131），chrome120 不稳定应弃用。**

这是本项目最要紧的技术未知 —— 它决定剩余 87% 的数据面重写是否可能。
在此之前 [26](26-perf-measured-20260726.md) §9 把它列为「最大不确定项」。

---

## 0. 判定

`plan.md` §2 的四条出门判据，本次只回答第 1 条：

| # | 判据 | 结果 |
|---|------|------|
| 1 | JA3 / JA4 与 curl_cffi 实测一致 | ✅ **通过**（chrome124 / chrome131） |
| 2 | CF 通过率不劣于 Python 基线 | ☐ 未测（需 CF 可测窗 + 号池账号） |
| 3 | SSE 长连接跑通一轮完整生图 | ☐ 未测 |
| 4 | `self=0` | ☐ 未测（但 `error_class` 的 `client` 不可达已修，指标本身现在可信） |

**硬阻塞性质由「技术未知」正式降级为「工作量」。**

---

## 1. 实测数据

复现方式见 §5。检测服务 `https://tls.browserleaks.com/json`，经本地代理 `127.0.0.1:7897`。

### 1.1 JA4（排序后哈希，扩展顺序不敏感）

| profile | curl_cffi 0.15.0 | wreq 6.0.0-rc | 一致 |
|---------|-----------------|---------------|------|
| chrome120 | `t13d1516h2_8daaf6152771_02713d6af862` | `t13d1516h2_8daaf6152771_02713d6af862` | ⚠️ 见 §2 |
| chrome124 | `t13d1516h2_8daaf6152771_02713d6af862` | `t13d1516h2_8daaf6152771_02713d6af862` | ✅ |
| chrome131 | `t13d1516h2_8daaf6152771_02713d6af862` | `t13d1516h2_8daaf6152771_02713d6af862` | ✅ |

三个 profile 的 JA4 **两侧完全相同，且彼此也相同** —— 说明 JA4 对这三个 Chrome 版本不做区分。

### 1.2 JA3N（归一化 JA3，扩展排序后）

| profile | curl_cffi | wreq | 一致 |
|---------|-----------|------|------|
| chrome120 | `473f0e7c0b6a0f7b049072f4e683068b` | `473f0e7c0b6a0f7b049072f4e683068b` | ⚠️ 不稳定 |
| chrome124 | `4c9ce26028c11d7544da00d3f7e4f45c` | `4c9ce26028c11d7544da00d3f7e4f45c` | ✅ **逐字节相同** |
| chrome131 | `dee19b855b658c6aa0f575eda2525e19` | `dee19b855b658c6aa0f575eda2525e19` | ✅ **逐字节相同** |

**这是本次最强的证据** —— JA3N 对 cipher suite 列表、扩展集合、椭圆曲线、点格式全部敏感，
两侧独立实现却算出同一个 MD5，意味着 ClientHello 的实质内容一致。

### 1.3 JA4_r（含 cipher / 扩展 / 签名算法全展开）

两侧完全相同：

```text
t13d1516h2_002f,0035,009c,009d,1301,1302,1303,c013,c014,c02b,c02c,c02f,c030,cca8,cca9
_0005,000a,000b,000d,0012,0017,001b,0023,002b,002d,0033,4469,fe0d,ff01
_0403,0804,0401,0503,0805,0501,0806,0601
```

15 个 cipher suite、16 个扩展、8 个签名算法，逐项对齐。

### 1.4 HTTP/2 指纹（Akamai）

| 侧 | akamai_hash |
|----|-------------|
| curl_cffi（全部 profile） | `52d84b11737d980aef856699f885ca86` |
| wreq（`http2(true)`，全部 profile） | `52d84b11737d980aef856699f885ca86` |

**完全一致。** SETTINGS 帧、窗口大小、优先级、伪头顺序（`m,a,s,p`）全部对齐：

```text
1:65536;2:0;4:6291456;6:262144|15663105|0|m,a,s,p
```

⚠️ 注意：`Emulation::builder().http2(false)` 会导致 Akamai 指纹变为 `787b789948...`（不匹配）。
**生产必须保持 `http2(true)`（默认值）。**

### 1.5 JA3（未排序，扩展顺序敏感）

**两侧不一致，且这是预期的。**

| profile | curl_cffi | wreq |
|---------|-----------|------|
| chrome120 | `b02bec06b52a1a67f7143f015245277d` | `3e22184726f3bf75620451b70e8e435a` |
| chrome124 | `3c490ff4dc712b10bf6d871278a25048` | `5fbe855f0907efddf3dc6d0ab9458d97` |
| chrome131 | `c97e2c7f081860e516f2cdd046772ad6` | `dbb3e0843052407e8681edb5ba37bc57` |

**为什么这不构成失败**：真实 Chrome 自 2023 年起对 TLS 扩展顺序做**随机化**（extension shuffling），
所以原始 JA3 对 Chrome 本来就不是稳定标识 —— 同一个真实浏览器每次握手的 JA3 都不同。
CF 这类风控用的是 JA3N / JA4 这种排序后指纹。curl_cffi 自己的 JA3 也逐 profile 不同而 JA4 相同，
正是同一现象。**用 JA3 判等价是错误的判据。**

---

## 2. ⚠️ chrome120 不稳定，应弃用

连续两轮运行，chrome124 / chrome131 的指纹**完全稳定**，但 chrome120 变了：

| 轮次 | chrome120 JA4 | chrome120 JA3N |
|------|--------------|----------------|
| 1 | `t13d1516h2_8daaf6152771_02713d6af862` | `473f0e7c0b6a0f7b049072f4e683068b` |
| 2 | `t13d1517h2_8daaf6152771_b1ff8ab2d16f` | `8a9ee1d3c6f0f892b4d43cabcf554150` |

`1516` → `1517` 是**扩展数量**从 16 变 17，说明该 preset 存在条件性扩展（很可能是 GREASE 或
`application_settings` 的条件注入）。

**处置**：`gptimage-panda/services/account_fingerprint.py` 的 `FP_PROFILES` 里
chrome120 出现 2 次（共 6 个 profile）。迁移时应**只用 chrome124 / chrome131**，
或先查清 chrome120 的浮动来源。这条在 Python 侧同样存在（curl_cffi 的 chrome120 也需复测），
不是 Rust 引入的问题。

---

## 3. 选型现状

| 项 | 值 |
|----|-----|
| `wreq` | `6.0.0-rc`（本次实测版本） |
| `wreq-util` | `3.0.0-rc.14` |
| TLS 后端 | BoringSSL，经 `btls-sys 0.5.6`（自带 patch 集） |
| 指纹预设 | `Profile::Chrome120/124/131` 等；`Platform::{MacOS,Windows,Linux,Android,IOS}` |
| 构建依赖 | cmake + **libclang**（bindgen 需要） |

`wreq-util` 的 `Emulation` 是 `TypedBuilder`，可分别指定 `profile` / `platform` / `http2` / `headers`。
`Platform` 必须显式设为 `MacOS` 才与 curl_cffi 的 UA 对齐（curl_cffi 的 chrome* profile 默认报 macOS UA）。

### 构建成本（不可忽视）

BoringSSL 从源码编译，首次构建在 16 核机器上约 **10-15 分钟**，且需要：

- `cmake`（Windows 上走 Visual Studio generator）
- `libclang.dll` —— 本机原本**没有**，通过 `pip download libclang` 取 wheel 里的 DLL 解决
  （`LIBCLANG_PATH` 指向解压目录）

CI 和 panda 构建镜像都要预置这两项。Linux 上通常 `apt install clang cmake` 即可。

---

## 4. 对项目的影响

### 4.1 Phase B′ 判据 1 通过 → 继续，不改定位

`plan.md` §2 写明「若 B′ 判定失败，把数据面重写从 §0 目标删除，本项目定位改为 face + 鉴权层，
`docs/13` 的并发收益预估同步作废」。**判据 1 通过，这条不触发。**

[26](26-perf-measured-20260726.md) §8 的完全重写收益预估（RSS −82~93% / CPU −85~95% /
并发 ×5~15）**前提成立**，从「悬空」变为「待施工」。

### 4.2 但仍有三条判据未测

判据 2（CF 通过率）是真正的业务验收，需要 CF 可测窗 + 真实号池账号。
指纹一致 ≠ CF 一定放行 —— CF 还看 IP 信誉、行为特征、Turnstile。
**本次结论只能说「TLS 层不再是阻塞」，不能说「Rust 能过 CF」。**

### 4.3 下一步

1. 用 chrome124 / chrome131 复跑判据 2（CF 通过率 A/B）
2. `wreq` 从 `-rc` 升到正式版后复测（rc 版本不宜直接上生产）
3. 查清 chrome120 的指纹浮动来源，或从 `FP_PROFILES` 移除

---

## 5. 复现

spike 位于 `spike/tls-fingerprint/`，**独立 cargo 项目，不是主 workspace 的 member**
（避免 BoringSSL 编译拖慢主项目 CI）。

```bash
# Python 基线
cd spike
SPIKE_PROXY=http://127.0.0.1:7897 python curl_cffi_baseline.py

# Rust 侧
cd spike/tls-fingerprint
export LIBCLANG_PATH=/path/to/dir/containing/libclang.dll
SPIKE_PROXY=http://127.0.0.1:7897 cargo run
# 结果写入 out-chrome{120,124,131}.json

# 对比
python -c "
import json
for p in ['chrome120','chrome124','chrome131']:
    d = json.load(open(f'out-{p}.json'))
    print(p, d['ja3n_hash'], d['ja4'], d['akamai_hash'])
"
```

libclang 获取（本机无 LLVM 时）：

```bash
pip download libclang --no-deps -d /tmp/lc
unzip -o /tmp/lc/libclang-*.whl -d /tmp/lc/x
# DLL 在 /tmp/lc/x/libclang-*.data/platlib/clang/native/libclang.dll
```

⚠️ `cargo run` 会重跑 build script，`LIBCLANG_PATH` 必须 `export`（不能只写在命令前缀）。

---

## 6. 原始数据

`spike/tls-fingerprint/out-*.json`（wreq 侧）为本次运行产物。
Python 侧输出未落盘，复跑 `curl_cffi_baseline.py` 即可重现（§1 表格已记录全部关键字段）。

> spike 目录含 BoringSSL 编译产物（`target/` 数百 MB）与 84MB 的 `libclang.dll`，
> 已在 `.gitignore` 中排除，**不要提交**。
