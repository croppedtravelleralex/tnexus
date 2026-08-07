//! x-statsig-id 本地生成：1645e3 模块算法骨架的参考实现（RFC 级，常量待拟合）。
//!
//! ## 侦察结论（2026-08-07，全证据链）
//!
//! **grok 的 x-statsig-id 不是标准 statsig SDK 产物**——标准路径已证伪：
//! - 101 个前端 chunk 中无 statsig client key（`client-*` 命中均为无关字符串）、无 GrainHash/generateHash 实现；
//! - 签名器是 Turbopack 模块 `4629918` 的 wrapper，真实实现模块 `1645e3`：
//!   **javascript-obfuscator 重度混淆**（RC4 加密字符串表 314 元素 + rotate + 多层转发 + 自我防卫），
//!   与 statsig 开源 SDK 无代码关联；
//! - 唯一相关的是模块名/参数形状（`signer(path, method) -> Promise<id>`）。
//!
//! ## 从 1645e3 尾部未混淆调用逻辑解出的算法骨架
//!
//! ```text
//! signer(path, method):                       // async
//!   u = timeBucket(now)                        // 两次 /1e3 运算 → 秒/千秒桶
//!   e = new A([u]).join("r")                   // 时间桶序列化
//!   o = _ || defaultState                      // 状态/盐
//!   f = lookup(o)
//!   x = resolve(L)                            // 解析某数据 → 浮点数组
//!   hex = String(x1 + x2).match(/([\d.-]+)/g)  // 提取全部数字
//!         .map(v => Number(Number(v).toFixed(2)).toString(16))  // 定点 hex 化
//!         .join("").replace(/[.-]/g, "")       // 去分隔符 → 输入串
//!   input = [path, method, u].join("!")        // 显式可见的 join("!")
//!   return new A(await hash(input + ...))      // async hash（WebCrypto 推测）→ 输出 id
//! ```
//!
//! 未确定常量（需真实 id 样本或字符串表解码拟合）：时间桶粒度、join("!") 的字段序、
//! hash 算法（SHA-256/HMAC/自研）、`hex` 与 `input` 的拼接方式、输出编码（base36?）。
//!
//! 本模块是**参数化参考实现**：常量经 [`GrainConfig`] 可调，`fit_from_sample` 提供
//! 用真实样本验证/拟合的入口。**当前配置下输出的 id 是否被 grok 接受未经证实**——
//! 需要 1 个真实签名样本（浏览器/Panda 运行时抓取）对齐常量。

use sha2::{Digest, Sha256};

/// 时间桶粒度（秒）：statsig 风格 10s；1645e3 疑似两次 /1e3 运算，具体粒度待拟合。
pub const DEFAULT_BUCKET_SECONDS: u64 = 10;

/// 输入字段序（join("!")）：默认 [path, method, bucket]——按 1645e3 可见的 `[n, W, u]` 顺序。
pub const DEFAULT_JOIN_ORDER: [JoinField; 3] =
    [JoinField::Path, JoinField::Method, JoinField::Bucket];

/// join("!") 输入字段。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinField {
    Path,
    Method,
    Bucket,
}

/// 签名配置（常量全部参数化，供样本拟合）。
#[derive(Debug, Clone)]
pub struct GrainConfig {
    pub bucket_seconds: u64,
    pub join_order: Vec<JoinField>,
    pub separator: &'static str,
    /// 是否把 hex 序列（定点 hex 化浮点串）拼进 hash 输入。
    pub include_hex_series: bool,
}

impl Default for GrainConfig {
    fn default() -> Self {
        Self {
            bucket_seconds: DEFAULT_BUCKET_SECONDS,
            join_order: DEFAULT_JOIN_ORDER.to_vec(),
            separator: "!",
            include_hex_series: true,
        }
    }
}

impl GrainConfig {
    /// 时间桶：`now_unix_secs / bucket_seconds`。
    pub fn bucket(&self, now_unix_secs: u64) -> u64 {
        now_unix_secs / self.bucket_seconds
    }

    /// 拼接输入（对齐 1645e3 的 join("!") 骨架）。
    pub fn join_input(&self, path: &str, method: &str, bucket: u64) -> String {
        let mut parts = Vec::with_capacity(self.join_order.len());
        for f in &self.join_order {
            let v = match f {
                JoinField::Path => path.to_string(),
                JoinField::Method => method.to_string(),
                JoinField::Bucket => bucket.to_string(),
            };
            parts.push(v);
        }
        parts.join(self.separator)
    }

    /// 定点 hex 序列化：`Number(v.toFixed(2)).toString(16)` 后去 `.`/`-` 再拼接。
    /// 对齐 1645e3 的 `match(/([\d.-]+)/g) → toFixed(2) → toString(16) → 去分隔` 链。
    pub fn hex_series(values: &[f64]) -> String {
        let mut out = String::new();
        for v in values {
            let fixed = format!("{v:.2}");
            if let Ok(num) = fixed.parse::<f64>() {
                let hex = format_hex_no_dot(num);
                out.push_str(&hex);
            }
        }
        out
    }
}

/// `Number.toFixed(2) → Number → toString(16)` 后去除 `.`/`-`。
fn format_hex_no_dot(num: f64) -> String {
    let mut s = format!("{:x}", num.to_bits()); // 占位：真正 JS 语义是 (num).toString(16)
    s.retain(|c| c != '.' && c != '-');
    s
}

/// 骨架签名器：时间桶 + join("!") + hex 序列 + SHA-256 派生。
/// **实验性**：常量未经真实样本拟合，输出可能不被 grok 接受。
#[derive(Default)]
pub struct GrainSigner {
    pub config: GrainConfig,
}

impl GrainSigner {
    pub fn new(config: GrainConfig) -> Self {
        Self { config }
    }

    /// 产出 id（实验性）：`base36(SHA-256(join_input + ":" + hex_series))`。
    /// 长度与 looksGood 约束（>20、非 x0:/eDA6 前缀）对齐。
    pub fn sign(&self, path: &str, method: &str, now_unix_secs: u64) -> String {
        let bucket = self.config.bucket(now_unix_secs);
        let mut input = self.config.join_input(path, method, bucket);
        if self.config.include_hex_series {
            // 骨架中的浮点数组源未确定（L 的解析产物）；用 bucket 派生两个占位浮点。
            let placeholders = [bucket as f64 / 1000.0, (bucket % 1000) as f64];
            input.push(':');
            input.push_str(&GrainConfig::hex_series(&placeholders));
        }
        let digest = Sha256::digest(input.as_bytes());
        // base36 编码摘要前 14 字节 → 约 21 字符（>20 满足 looksGood）。
        let mut n = 0u128;
        for b in digest.iter().take(14) {
            n = (n << 8) | u128::from(*b);
        }
        let mut out = String::new();
        let mut v = n;
        if v == 0 {
            out.push('0');
        }
        while v > 0 {
            let d = (v % 36) as u8;
            out.push(if d < 10 { b'0' + d } else { b'a' + (d - 10) } as char);
            v /= 36;
        }
        out.chars().rev().collect()
    }

    /// 用真实样本验证当前配置：`sign(path, method, now)` 与样本 id 对齐则 `Ok(())`。
    /// 对齐失败返回误差提示（供拟合调整常量）。
    pub fn fit_from_sample(
        &self,
        path: &str,
        method: &str,
        now_unix_secs: u64,
        sample_id: &str,
    ) -> Result<(), String> {
        let got = self.sign(path, method, now_unix_secs);
        if got == sample_id {
            Ok(())
        } else {
            Err(format!(
                "样本不匹配：got {got:?} want {sample_id:?}（bucket_seconds={} join_order 需拟合）",
                self.config.bucket_seconds
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bucket_is_stable_within_window() {
        let cfg = GrainConfig::default();
        let t = 1_752_000_000u64;
        assert_eq!(cfg.bucket(t), cfg.bucket(t + 9)); // 同窗
        assert_ne!(cfg.bucket(t), cfg.bucket(t + cfg.bucket_seconds)); // 跨窗
    }

    #[test]
    fn join_input_uses_exclamation() {
        let cfg = GrainConfig::default();
        let s = cfg.join_input("/v1/chat", "POST", 42);
        assert_eq!(s, "/v1/chat!POST!42");
    }

    #[test]
    fn hex_series_drops_dots() {
        // 对齐 1645e3 的 toFixed(2)→toString(16)→去 [.-] 链（语义近似）。
        let s = GrainConfig::hex_series(&[1.5, 0.25]);
        assert!(!s.contains('.'));
        assert!(!s.contains('-'));
        assert!(!s.is_empty());
    }

    #[test]
    fn sign_output_meets_looks_good_constraints() {
        // js.rs looksGood：len > 20、不以 x0:/eDA6 开头。
        let signer = GrainSigner::default();
        let id = signer.sign("/rest/app-chat/conversations/new", "POST", 1_752_000_000);
        assert!(id.len() > 20, "len={}", id.len());
        assert!(!id.starts_with("x0:"));
        assert!(!id.starts_with("eDA6"));
    }

    #[test]
    fn same_input_same_id() {
        let signer = GrainSigner::default();
        let a = signer.sign("/p", "POST", 1_752_000_000);
        let b = signer.sign("/p", "POST", 1_752_000_000);
        assert_eq!(a, b);
    }

    #[test]
    fn fit_from_sample_matches_self() {
        let signer = GrainSigner::default();
        let t = 1_752_000_000u64;
        let id = signer.sign("/p", "GET", t);
        assert!(signer.fit_from_sample("/p", "GET", t, &id).is_ok());
        assert!(signer.fit_from_sample("/p", "GET", t, "wrong-id").is_err());
    }
}
