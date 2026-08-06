//! grok-shadow — Grok 移植 shadow compare（G6-P3）。
//!
//! Rust 移植（行为对齐）scripts/grok_shadow_compare.py：
//! - G6-A1 成功率 ≥ Go − 1%（--success-gap 0.01）
//! - G6-A2 P99 延迟 ≤ Go × 1.15（--p99-ratio 1.15）
//! - G6-A3 账号粒度额度 remaining 一致（--go-quota / --rust-quota，--quota-tol）
//!
//! 数据源：--file（默认，两份结果文件：`timestamp, model, status, latency_ms,
//! account_id` 逗号/空白分隔，表头自动跳过）或 --url（聚合端点 JSON 记录数组）。
//! 指标：成功率、P50/P95/P99（nearest-rank）。退出码 0=达标 / 2=阈值超限 /
//! 1=IO/解析/参数错误。--self-test 内置合成数据自测。

use serde_json::json;
use std::collections::HashMap;
use std::env;
use std::process::ExitCode;

#[derive(Debug, Clone, Default, PartialEq)]
struct Record {
    timestamp: String,
    model: String,
    status: String,
    latency_ms: f64,
    account_id: String,
}

#[derive(Debug, Clone, Default)]
#[allow(dead_code)] // name 保留与 Python 对齐（结构信息，未直接输出）
struct SideStats {
    name: String,
    n: usize,
    success: usize,
    success_rate: f64,
    latencies: Vec<f64>,
    p50: f64,
    p95: f64,
    p99: f64,
}

#[derive(Debug, Clone, Default)]
#[allow(dead_code)] // account_id/mode 保留与 Python 对齐（key 已含二者）
struct QuotaPoint {
    account_id: String,
    remaining: f64,
    mode: String,
}

#[derive(Debug, Default)]
struct CompareResult {
    go: SideStats,
    rust: SideStats,
    success_gap: f64,
    p99_ratio: f64,
    success_ok: bool,
    p99_ok: bool,
    quota_ok: bool,
    quota_total: usize,
    quota_match: usize,
    quota_diff: Vec<(String, f64, f64)>,
}

// ── 记录解析（对齐 Python load_records / _split_raw / _is_header）──────

fn split_raw(line: &str) -> Vec<String> {
    let by_comma: Vec<String> = line.split(',').map(|p| p.trim().to_string()).collect();
    if by_comma.len() == 1 {
        return line.split_whitespace().map(|p| p.to_string()).collect();
    }
    by_comma.into_iter().filter(|p| !p.is_empty()).collect()
}

fn is_header(line: &str) -> bool {
    let low = line.trim().to_lowercase();
    (low.contains("timestamp") && (low.contains("latency") || low.contains("status")))
        || low.starts_with("ts,")
        || low.starts_with("timestamp;")
}

fn is_success_status(status: &str) -> bool {
    let s = status.trim().to_lowercase();
    if ["success", "ok", "2xx"].contains(&s.as_str()) {
        return true;
    }
    if ["error", "fail", "failed", "failure", "1xx", "4xx", "5xx"].contains(&s.as_str()) {
        return false;
    }
    match s.parse::<f64>() {
        Ok(code) => (200.0..=399.0).contains(&code),
        Err(_) => false,
    }
}

fn load_records(lines: &[String]) -> Vec<Record> {
    let mut records = Vec::new();
    for line in lines {
        let line = line.trim();
        if line.is_empty() || is_header(line) {
            continue;
        }
        let parts = split_raw(line);
        if parts.len() < 3 {
            continue;
        }
        let latency = if parts.len() > 3 {
            parts[3].parse::<f64>().unwrap_or(0.0)
        } else {
            0.0
        };
        records.push(Record {
            timestamp: parts[0].clone(),
            model: parts[1].clone(),
            status: parts[2].clone(),
            latency_ms: latency,
            account_id: if parts.len() > 4 {
                parts[4].clone()
            } else {
                String::new()
            },
        });
    }
    records
}

fn load_file(path: &str) -> Result<Vec<Record>, String> {
    let content = std::fs::read_to_string(path).map_err(|e| format!("read {path}: {e}"))?;
    let lines: Vec<String> = content.lines().map(|l| l.to_string()).collect();
    Ok(load_records(&lines))
}

fn records_from_json(payload: &[serde_json::Value]) -> Vec<Record> {
    let mut out = Vec::new();
    for it in payload {
        let obj = match it.as_object() {
            Some(o) => o,
            None => continue,
        };
        let lat = obj
            .get("latency_ms")
            .or_else(|| obj.get("latency"))
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        out.push(Record {
            timestamp: obj
                .get("timestamp")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            model: obj
                .get("model")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            status: obj
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("200")
                .to_string(),
            latency_ms: lat,
            account_id: obj
                .get("account_id")
                .or_else(|| obj.get("accountId"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        });
    }
    out
}

async fn fetch_json(url: &str) -> Result<Vec<serde_json::Value>, String> {
    let resp = reqwest::Client::new()
        .get(url)
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| format!("fetch {url}: {e}"))?;
    let payload: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("decode {url}: {e}"))?;
    match payload {
        serde_json::Value::Array(arr) => Ok(arr),
        serde_json::Value::Object(map) => match map.get("records") {
            Some(serde_json::Value::Array(arr)) => Ok(arr.clone()),
            _ => Err(format!(
                "unexpected JSON shape from {url} (want list of records)"
            )),
        },
        _ => Err(format!(
            "unexpected JSON shape from {url} (want list of records)"
        )),
    }
}

// ── 统计（对齐 Python percentile / compute_stats）──────────────────────

fn percentile(sorted_values: &[f64], p: f64) -> f64 {
    let n = sorted_values.len();
    if n == 0 {
        return 0.0;
    }
    if p <= 0.0 {
        return sorted_values[0];
    }
    if p >= 100.0 {
        return *sorted_values.last().unwrap();
    }
    let mut idx = (p / 100.0 * n as f64).ceil() as usize;
    idx = idx.saturating_sub(1);
    idx = idx.min(n - 1);
    sorted_values[idx]
}

fn compute_stats(name: &str, records: &[Record], latency_of: &str) -> SideStats {
    let mut st = SideStats {
        name: name.to_string(),
        ..Default::default()
    };
    if records.is_empty() {
        return st;
    }
    st.n = records.len();
    st.success = records
        .iter()
        .filter(|r| is_success_status(&r.status))
        .count();
    st.success_rate = st.success as f64 / st.n as f64;
    let pool: Vec<&Record> = if latency_of == "all" {
        records.iter().collect()
    } else {
        records
            .iter()
            .filter(|r| is_success_status(&r.status))
            .collect()
    };
    let mut lat: Vec<f64> = pool.iter().map(|r| r.latency_ms).collect();
    lat.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    st.latencies = lat;
    st.p50 = percentile(&st.latencies, 50.0);
    st.p95 = percentile(&st.latencies, 95.0);
    st.p99 = percentile(&st.latencies, 99.0);
    st
}

// ── 额度（对齐 Python load_quota）──────────────────────────────────────

fn load_quota(path: &str) -> Result<HashMap<String, QuotaPoint>, String> {
    let content = std::fs::read_to_string(path).map_err(|e| format!("read quota {path}: {e}"))?;
    let mut out = HashMap::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || is_header(line) {
            continue;
        }
        let parts: Vec<String> = line
            .replace(',', ":")
            .split(':')
            .map(|s| s.to_string())
            .collect();
        if parts.len() < 2 {
            continue;
        }
        let remaining = match parts[1].trim().parse::<f64>() {
            Ok(r) => r,
            Err(_) => continue,
        };
        let acc = parts[0].trim().to_string();
        let mode = if parts.len() > 2 {
            parts[2].trim().to_string()
        } else {
            String::new()
        };
        out.insert(
            format!("{acc}:{mode}"),
            QuotaPoint {
                account_id: acc,
                remaining,
                mode,
            },
        );
    }
    Ok(out)
}

// ── 对比（对齐 Python compare_sides / compare_quota）───────────────────

fn compare_sides(
    go: &SideStats,
    rust: &SideStats,
    success_gap: f64,
    p99_ratio: f64,
) -> CompareResult {
    let mut res = CompareResult {
        go: go.clone(),
        rust: rust.clone(),
        ..Default::default()
    };
    res.success_gap = rust.success_rate - go.success_rate;
    // G6-A1：rust ≥ go − gap（浮点容差 1e-9）
    res.success_ok = rust.success_rate >= (go.success_rate - success_gap) - 1e-9;
    // G6-A2：rust.P99 ≤ go.P99 × ratio（go 无样本 → 达标）
    if go.p99 > 0.0 {
        res.p99_ratio = rust.p99 / go.p99;
        res.p99_ok = rust.p99 <= go.p99 * p99_ratio + 1e-9;
    } else {
        res.p99_ratio = 0.0;
        res.p99_ok = true;
    }
    res
}

fn compare_quota(
    go: &HashMap<String, QuotaPoint>,
    rust: &HashMap<String, QuotaPoint>,
    tol: f64,
) -> CompareResult {
    let mut res = CompareResult {
        go: SideStats {
            name: "go".into(),
            ..Default::default()
        },
        rust: SideStats {
            name: "rust".into(),
            ..Default::default()
        },
        ..Default::default()
    };
    let mut keys: Vec<&String> = go.keys().chain(rust.keys()).collect();
    keys.sort();
    keys.dedup();
    res.quota_total = keys.len();
    for key in keys {
        let g = go.get(key);
        let r = rust.get(key);
        if let (Some(g), Some(r)) = (g, r) {
            if (r.remaining - g.remaining).abs() <= tol {
                res.quota_match += 1;
                continue;
            }
        }
        let gv = g.map(|p| p.remaining).unwrap_or(f64::NAN);
        let rv = r.map(|p| p.remaining).unwrap_or(f64::NAN);
        res.quota_diff.push((key.clone(), gv, rv));
    }
    res.quota_ok = res.quota_match == res.quota_total;
    res
}

// ── 输出（对齐 Python render_table / json payload）────────────────────

fn render_table(res: &CompareResult, p95: bool) -> Vec<String> {
    let mut rows = Vec::new();
    rows.push(format!(
        "{:<14}{:>12}{:>12}{:>12}",
        "metric", "go", "rust", "delta"
    ));
    rows.push(format!(
        "{:<14}{:>12}{:>12}{:>12}",
        "requests",
        res.go.n,
        res.rust.n,
        res.rust.n as i64 - res.go.n as i64
    ));
    rows.push(format!(
        "{:<14}{:>12.4}{:>12.4}{:>+8.4}",
        "success_rate", res.go.success_rate, res.rust.success_rate, res.success_gap
    ));
    rows.push(format!(
        "{:<14}{:>12.1}{:>12.1}{:>+8.1}",
        "p50_ms",
        res.go.p50,
        res.rust.p50,
        res.rust.p50 - res.go.p50
    ));
    if p95 {
        rows.push(format!(
            "{:<14}{:>12.1}{:>12.1}{:>+8.1}",
            "p95_ms",
            res.go.p95,
            res.rust.p95,
            res.rust.p95 - res.go.p95
        ));
    }
    rows.push(format!(
        "{:<14}{:>12.1}{:>12.1}{:>12.3}x",
        "p99_ms", res.go.p99, res.rust.p99, res.p99_ratio
    ));
    rows
}

fn json_payload(res: &CompareResult) -> serde_json::Value {
    let quota_diff: Vec<serde_json::Value> = res
        .quota_diff
        .iter()
        .take(50)
        .map(|(k, a, b)| json!({ "account_id": k, "go": a, "rust": b }))
        .collect();
    json!({
        "go": {
            "n": res.go.n, "success": res.go.success, "success_rate": res.go.success_rate,
            "latency_ms": {"p50": res.go.p50, "p95": res.go.p95, "p99": res.go.p99},
        },
        "rust": {
            "n": res.rust.n, "success": res.rust.success, "success_rate": res.rust.success_rate,
            "latency_ms": {"p50": res.rust.p50, "p95": res.rust.p95, "p99": res.rust.p99},
        },
        "diff": {
            "success_gap": res.success_gap,
            "p99_ratio": res.p99_ratio,
            "success_ok": res.success_ok,
            "p99_ok": res.p99_ok,
        },
        "quota": {
            "total": res.quota_total, "match": res.quota_match, "ok": res.quota_ok,
            "diff": quota_diff,
        },
        "pass": res.success_ok && res.p99_ok && res.quota_ok,
    })
}

// ── 主流程（对齐 Python run / self_test）──────────────────────────────

struct RunArgs {
    url: bool,
    go: String,
    rust: String,
    go_url: String,
    rust_url: String,
    go_quota: String,
    rust_quota: String,
    quota_tol: f64,
    success_gap: f64,
    p99_ratio: f64,
    p95: bool,
    json: bool,
    self_test: bool,
}

fn usage() -> String {
    "grok-shadow — Grok 移植 shadow compare（G6-A1/A2/A3）

Usage: grok-shadow [--file] [--go F] [--rust F] [--go-quota F] [--rust-quota F] [options]
       grok-shadow --url --go-url U --rust-url U [options]
       grok-shadow --self-test

Options:
  --file                local file replay (default; needs --go/--rust)
  --url                 URL aggregate endpoints (needs --go-url/--rust-url)
  --go F                Go result file
  --rust F              Rust result file
  --go-url U            Go aggregate endpoint
  --rust-url U          Rust aggregate endpoint
  --go-quota F          Go quota file (account_id: remaining[:mode])
  --rust-quota F        Rust quota file
  --quota-tol T         quota tolerance (default 0)
  --success-gap G       success-rate lower bound gap (default 0.01)
  --p99-ratio R         Rust/Go P99 upper ratio (default 1.15)
  --p95                 additionally print P95
  --json                JSON output
  --self-test           built-in synthetic-data self test
  -h, --help            show this help"
        .to_string()
}

fn parse_args(argv: &[String]) -> Result<RunArgs, String> {
    let mut url = false;
    let mut go = String::new();
    let mut rust = String::new();
    let mut go_url = String::new();
    let mut rust_url = String::new();
    let mut go_quota = String::new();
    let mut rust_quota = String::new();
    let mut quota_tol = 0.0f64;
    let mut success_gap = 0.01f64;
    let mut p99_ratio = 1.15f64;
    let mut p95 = false;
    let mut json = false;
    let mut self_test = false;

    let mut i = 0;
    while i < argv.len() {
        let a = argv[i].as_str();
        match a {
            "--file" => url = false,
            "--url" => url = true,
            "--go" => {
                i += 1;
                go = argv.get(i).cloned().ok_or("--go requires a value")?;
            }
            "--rust" => {
                i += 1;
                rust = argv.get(i).cloned().ok_or("--rust requires a value")?;
            }
            "--go-url" => {
                i += 1;
                go_url = argv.get(i).cloned().ok_or("--go-url requires a value")?;
            }
            "--rust-url" => {
                i += 1;
                rust_url = argv.get(i).cloned().ok_or("--rust-url requires a value")?;
            }
            "--go-quota" => {
                i += 1;
                go_quota = argv.get(i).cloned().ok_or("--go-quota requires a value")?;
            }
            "--rust-quota" => {
                i += 1;
                rust_quota = argv
                    .get(i)
                    .cloned()
                    .ok_or("--rust-quota requires a value")?;
            }
            "--quota-tol" => {
                i += 1;
                quota_tol = argv
                    .get(i)
                    .and_then(|v| v.parse().ok())
                    .ok_or("--quota-tol requires a float")?;
            }
            "--success-gap" => {
                i += 1;
                success_gap = argv
                    .get(i)
                    .and_then(|v| v.parse().ok())
                    .ok_or("--success-gap requires a float")?;
            }
            "--p99-ratio" => {
                i += 1;
                p99_ratio = argv
                    .get(i)
                    .and_then(|v| v.parse().ok())
                    .ok_or("--p99-ratio requires a float")?;
            }
            "--p95" => p95 = true,
            "--json" => json = true,
            "--self-test" => self_test = true,
            "-h" | "--help" => return Err("__help__".to_string()),
            other => return Err(format!("unrecognized argument: {other}")),
        }
        i += 1;
    }
    Ok(RunArgs {
        url,
        go,
        rust,
        go_url,
        rust_url,
        go_quota,
        rust_quota,
        quota_tol,
        success_gap,
        p99_ratio,
        p95,
        json,
        self_test,
    })
}

async fn load_records_or_fetch(args: &RunArgs) -> Result<(Vec<Record>, Vec<Record>), String> {
    if args.url {
        let go = records_from_json(&fetch_json(&args.go_url).await?);
        let rust = records_from_json(&fetch_json(&args.rust_url).await?);
        Ok((go, rust))
    } else {
        if args.go.is_empty() || args.rust.is_empty() {
            return Err("need --go and --rust result files (or --url mode)".to_string());
        }
        Ok((load_file(&args.go)?, load_file(&args.rust)?))
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let argv: Vec<String> = env::args().skip(1).collect();
    let args = match parse_args(&argv) {
        Ok(a) => a,
        Err(msg) => {
            if msg == "__help__" {
                println!("{}", usage());
                return ExitCode::SUCCESS;
            }
            eprintln!("error: {msg}\n\n{}", usage());
            return ExitCode::from(2);
        }
    };

    if args.self_test {
        match self_test() {
            Ok(code) => return ExitCode::from(code),
            Err(e) => {
                eprintln!("error: {e}");
                return ExitCode::from(1);
            }
        }
    }

    let outcome = run(&args).await;
    match outcome {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(1)
        }
    }
}

async fn run(args: &RunArgs) -> Result<u8, String> {
    let (go, rust) = load_records_or_fetch(args).await?;
    let go_s = compute_stats("go", &go, "success");
    let rust_s = compute_stats("rust", &rust, "success");
    let mut res = compare_sides(&go_s, &rust_s, args.success_gap, args.p99_ratio);

    // 额度对比（可选；未提供 → 视为达标，对齐 Python CompareResult 默认 True）
    if !args.go_quota.is_empty() && !args.rust_quota.is_empty() {
        let qgo = load_quota(&args.go_quota)?;
        let qrust = load_quota(&args.rust_quota)?;
        let qres = compare_quota(&qgo, &qrust, args.quota_tol);
        res.quota_ok = qres.quota_ok;
        res.quota_total = qres.quota_total;
        res.quota_match = qres.quota_match;
        res.quota_diff = qres.quota_diff;
    } else {
        res.quota_ok = true;
    }

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json_payload(&res)).unwrap_or_default()
        );
    } else {
        for line in render_table(&res, args.p95) {
            println!("{line}");
        }
        if !args.go_quota.is_empty() && !args.rust_quota.is_empty() {
            println!(
                "\nquota: {}/{} 账号一致 (tol={})",
                res.quota_match, res.quota_total, args.quota_tol
            );
            for (key, a, b) in res.quota_diff.iter().take(10) {
                println!("  diff {key}: go={a} rust={b}");
            }
        }
        let mut status = Vec::new();
        if !res.success_ok {
            status.push("G6-A1 成功率不达标");
        }
        if !res.p99_ok {
            status.push("G6-A2 P99 不达标");
        }
        if !res.quota_ok {
            status.push("G6-A3 额度不一致");
        }
        println!();
        if status.is_empty() {
            println!("ALL PASS: G6-A1/A2/A3 全部达标");
        } else {
            println!("{}", status.join("; "));
        }
    }

    if !(res.success_ok && res.p99_ok && res.quota_ok) {
        return Ok(2);
    }
    Ok(0)
}

fn self_test() -> Result<u8, String> {
    let go: Vec<Record> = (0..100)
        .map(|i| Record {
            status: "200".into(),
            latency_ms: 100.0 + (i % 5) as f64 * 10.0,
            ..Default::default()
        })
        .collect();
    let rust_ok: Vec<Record> = (0..100)
        .map(|i| Record {
            status: "200".into(),
            latency_ms: 110.0 + (i % 5) as f64 * 10.0,
            ..Default::default()
        })
        .collect();

    let mut go_mixed = Vec::new();
    for _ in 0..90 {
        go_mixed.push(Record {
            status: "200".into(),
            latency_ms: 100.0,
            ..Default::default()
        });
    }
    for _ in 0..10 {
        go_mixed.push(Record {
            status: "500".into(),
            latency_ms: 50.0,
            ..Default::default()
        });
    }
    let mut rust_low = Vec::new();
    for _ in 0..85 {
        rust_low.push(Record {
            status: "200".into(),
            latency_ms: 100.0,
            ..Default::default()
        });
    }
    for _ in 0..15 {
        rust_low.push(Record {
            status: "500".into(),
            latency_ms: 50.0,
            ..Default::default()
        });
    }

    let g = compute_stats("go", &go, "success");
    let r = compute_stats("rust", &rust_ok, "success");
    let base = compare_sides(&g, &r, 0.01, 1.15);
    assert!(base.success_ok && base.p99_ok, "基准(达标)应通过");

    let gm = compute_stats("go", &go_mixed, "success");
    let rl = compute_stats("rust", &rust_low, "success");
    let fail_succ = compare_sides(&gm, &rl, 0.01, 1.15);
    assert!(!fail_succ.success_ok, "成功率差距>1% 应不达标");

    let slow: Vec<Record> = (0..100)
        .map(|_| Record {
            status: "200".into(),
            latency_ms: 200.0,
            ..Default::default()
        })
        .collect();
    let slow_s = compute_stats("rust", &slow, "success");
    let fail_p99 = compare_sides(&g, &slow_s, 0.01, 1.15);
    assert!(!fail_p99.p99_ok, "P99 超 1.15x 应不达标");

    let loose = compare_sides(&gm, &rl, 0.05, 1.15);
    assert!(loose.success_ok, "放宽阈值后应达标");

    // 配额
    let mut qgo = HashMap::new();
    qgo.insert(
        "1:fast".into(),
        QuotaPoint {
            account_id: "1".into(),
            remaining: 100.0,
            mode: "fast".into(),
        },
    );
    qgo.insert(
        "2:fast".into(),
        QuotaPoint {
            account_id: "2".into(),
            remaining: 0.0,
            mode: "fast".into(),
        },
    );
    let mut qrust = HashMap::new();
    qrust.insert(
        "1:fast".into(),
        QuotaPoint {
            account_id: "1".into(),
            remaining: 100.0,
            mode: "fast".into(),
        },
    );
    qrust.insert(
        "2:fast".into(),
        QuotaPoint {
            account_id: "2".into(),
            remaining: 5.0,
            mode: "fast".into(),
        },
    );
    let q = compare_quota(&qgo, &qrust, 0.0);
    assert!(!q.quota_ok && !q.quota_diff.is_empty(), "额度不一致应检出");
    let mut qrust2 = HashMap::new();
    qrust2.insert(
        "1:fast".into(),
        QuotaPoint {
            account_id: "1".into(),
            remaining: 100.0,
            mode: "fast".into(),
        },
    );
    let q2 = compare_quota(&qgo, &qrust2, 0.0);
    assert!(!q2.quota_ok, "缺账号应检出");

    // 百分位（nearest-rank：p=99, n=100 → 第 99 个（1-based）→ index 98 = 98）
    let vals: Vec<f64> = (0..100).map(|i| i as f64).collect();
    assert_eq!(percentile(&vals, 99.0), 98.0);
    assert_eq!(percentile(&[], 99.0), 0.0);

    println!("self-test OK: 达标/劣化/配额/百分位 全部断言通过");
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn success_status_variants() {
        assert!(is_success_status("200"));
        assert!(is_success_status("204"));
        assert!(is_success_status("399"));
        assert!(is_success_status("success"));
        assert!(is_success_status("ok"));
        assert!(is_success_status("2xx"));
        assert!(!is_success_status("400"));
        assert!(!is_success_status("500"));
        assert!(!is_success_status("error"));
        assert!(!is_success_status("failed"));
        assert!(!is_success_status("5xx"));
        assert!(!is_success_status("unknown"));
    }

    #[test]
    fn percentile_nearest_rank() {
        let vals: Vec<f64> = (0..100).map(|i| i as f64).collect();
        assert_eq!(percentile(&vals, 99.0), 98.0);
        assert_eq!(percentile(&vals, 50.0), 49.0);
        assert_eq!(percentile(&vals, 95.0), 94.0);
        assert_eq!(percentile(&vals, 0.0), 0.0);
        assert_eq!(percentile(&vals, 100.0), 99.0);
        assert_eq!(percentile(&[], 50.0), 0.0);
        // clamp 上界：p=99, n=5 → ceil(4.95)=5-1=4 → 最后
        let small = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        assert_eq!(percentile(&small, 99.0), 5.0);
    }

    #[test]
    fn load_records_formats() {
        // 逗号分隔 + 表头跳过
        let lines = vec![
            "timestamp, model, status, latency_ms, account_id".to_string(),
            "2026-08-06T10:00:00, grok-3, 200, 123.5, acc1".to_string(),
            "2026-08-06T10:00:01, grok-3, success, 99, acc2".to_string(),
            "".to_string(),
            "bad-line".to_string(),
        ];
        let recs = load_records(&lines);
        assert_eq!(recs.len(), 2);
        assert_eq!(recs[0].model, "grok-3");
        assert_eq!(recs[0].status, "200");
        assert!((recs[0].latency_ms - 123.5).abs() < 1e-9);
        assert_eq!(recs[0].account_id, "acc1");
        assert_eq!(recs[1].status, "success");
        // 空白分隔
        let ws = vec!["ts model 200 10 acc".to_string()];
        let r2 = load_records(&ws);
        assert_eq!(r2.len(), 1);
        assert_eq!(r2[0].timestamp, "ts");
    }

    #[test]
    fn compute_stats_success_pool() {
        let recs = vec![
            Record {
                status: "200".into(),
                latency_ms: 100.0,
                ..Default::default()
            },
            Record {
                status: "200".into(),
                latency_ms: 200.0,
                ..Default::default()
            },
            Record {
                status: "500".into(),
                latency_ms: 50.0,
                ..Default::default()
            },
        ];
        let st = compute_stats("t", &recs, "success");
        assert_eq!(st.n, 3);
        assert_eq!(st.success, 2);
        assert!((st.success_rate - 2.0 / 3.0).abs() < 1e-9);
        // 延迟池只含成功请求（100, 200）
        assert_eq!(st.latencies, vec![100.0, 200.0]);
        assert_eq!(st.p50, 100.0);
        // all 池含失败
        let all = compute_stats("t", &recs, "all");
        assert_eq!(all.latencies, vec![50.0, 100.0, 200.0]);
    }

    #[test]
    fn compare_thresholds() {
        let go = SideStats {
            name: "go".into(),
            success_rate: 0.95,
            p99: 100.0,
            ..Default::default()
        };
        let rust_ok = SideStats {
            name: "rust".into(),
            success_rate: 0.96,
            p99: 110.0,
            ..Default::default()
        };
        let r = compare_sides(&go, &rust_ok, 0.01, 1.15);
        assert!(r.success_ok && r.p99_ok);
        // rust 成功率过低
        let rust_bad = SideStats {
            name: "rust".into(),
            success_rate: 0.93,
            p99: 100.0,
            ..Default::default()
        };
        assert!(!compare_sides(&go, &rust_bad, 0.01, 1.15).success_ok);
        // go 无 P99 样本 → 达标
        let go_empty = SideStats {
            name: "go".into(),
            success_rate: 0.0,
            p99: 0.0,
            ..Default::default()
        };
        assert!(compare_sides(&go_empty, &rust_ok, 0.01, 1.15).p99_ok);
    }

    #[test]
    fn quota_compare() {
        let mut go = HashMap::new();
        go.insert(
            "1:fast".into(),
            QuotaPoint {
                account_id: "1".into(),
                remaining: 100.0,
                mode: "fast".into(),
            },
        );
        let mut rust = HashMap::new();
        rust.insert(
            "1:fast".into(),
            QuotaPoint {
                account_id: "1".into(),
                remaining: 100.5,
                mode: "fast".into(),
            },
        );
        // 容差 1.0 → 一致
        let q = compare_quota(&go, &rust, 1.0);
        assert!(q.quota_ok && q.quota_match == 1);
        // 容差 0 → 不一致
        let q2 = compare_quota(&go, &rust, 0.0);
        assert!(!q2.quota_ok && q2.quota_diff.len() == 1);
    }

    #[test]
    fn load_quota_format() {
        use std::io::Write;
        let dir = std::env::temp_dir();
        let path = dir.join("grok_shadow_test_quota.txt");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, "account_id: remaining").unwrap();
        writeln!(f, "1, 100").unwrap();
        writeln!(f, "2: 50: fast").unwrap();
        writeln!(f, "bad").unwrap();
        let q = load_quota(path.to_str().unwrap()).unwrap();
        assert_eq!(q.len(), 2);
        assert!((q["1:"].remaining - 100.0).abs() < 1e-9);
        assert!((q["2:fast"].remaining - 50.0).abs() < 1e-9);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn parse_args_cli() {
        let argv: Vec<String> = vec!["--go", "a", "--rust", "b", "--p95"]
            .into_iter()
            .map(String::from)
            .collect();
        let a = parse_args(&argv).unwrap();
        assert_eq!(a.go, "a");
        assert_eq!(a.rust, "b");
        assert!(a.p95);
        assert!(!a.json && !a.url);
        let bad: Vec<String> = vec!["--nope".into()];
        assert!(parse_args(&bad).is_err());
        let missing: Vec<String> = vec!["--go".into()];
        assert!(parse_args(&missing).is_err());
    }

    #[test]
    fn self_test_passes() {
        assert_eq!(self_test().unwrap(), 0);
    }
}
