//! grok-etl — grok2api SQLite backend.db → TNexus PostgreSQL grok_* tables.
//!
//! Rust 移植（行为对齐）scripts/grok_etl_sqlite_to_pg.py，实现 docs/39b §4
//! （full-table COPY in dependency order）。
//!
//! 设计（与 Python 版一致）：
//! - Schema-driven：读 SQLite 源表列（PRAGMA table_info）与 PG 目标列
//!   （information_schema），按列交集逐行复制到映射的 grok_* 目标表。
//! - 防御性：任一侧缺表即 skip 并打印，永不 fatal。
//! - 保留 id 显式插入（shadow diff 可用）；密文列（identity_key/encrypted_*）逐字复制。
//! - 可重跑：先 TRUNCATE CASCADE RESTART IDENTITY 现存 grok 目标表再插。
//!
//! Env:
//!   GROK_ETL_SOURCE     SQLite backend.db 路径（缺 → exit 2）
//!   GROK_ETL_PG_DSN     postgres:// URL（缺 → 只出 plan）
//!   GROK_CREDENTIAL_KEY base64 32B AES-256 key（仅 decrypt smoke；Rust 版未 vendored
//!                       aes-gcm → 默认跳过并打印说明，exit 0）
//!
//! 用法：
//!   grok-etl --dry-run        # 不连 PG 出 plan
//!   grok-etl                  # 全量复制
//!   grok-etl --limit 10       # 每表前 N 行冒烟

use rusqlite::{Connection, OpenFlags};
use sqlx::postgres::{PgArguments, PgConnection};
use sqlx::{Arguments, Connection as _, Row as _};
use std::collections::{HashMap, HashSet};
use std::env;
use std::process::ExitCode;

// ── Go 表名 → TNexus grok_* PG 表名（docs/39b §3）─────────────────────
// 顺序即依赖顺序：父表（FK 源）在子表前，显式 id 插入满足引用完整性。
const TABLE_MAP: &[(&str, &str)] = &[
    ("admins", "grok_admins"),
    ("admin_sessions", "grok_admin_sessions"),
    ("provider_accounts", "grok_accounts"),
    ("account_credentials", "grok_credentials"),
    ("account_provider_links", "grok_account_provider_links"),
    ("web_account_profiles", "grok_web_profiles"),
    ("account_quota_windows", "grok_quota_windows"),
    ("account_billing_snapshots", "grok_billing_snapshots"),
    ("account_pool_snapshots", "grok_pool_snapshots"),
    ("account_quota_recovery", "grok_quota_recovery"),
    ("client_keys", "grok_client_keys"),
    ("billing_reservations", "grok_billing_reservations"),
    ("model_routes", "grok_model_routes"),
    ("model_route_aliases", "grok_model_route_aliases"),
    ("model_route_accounts", "grok_model_route_accounts"),
    ("client_key_models", "grok_client_key_models"),
    ("account_model_capabilities", "grok_model_capabilities"),
    ("account_model_sync_states", "grok_model_sync_states"),
    ("account_model_quota_blocks", "grok_model_quota_blocks"),
    ("account_model_states", "grok_model_states"),
    ("egress_nodes", "grok_egress_nodes"),
    ("egress_traffic_hops", "grok_egress_traffic_hops"),
    ("request_audits", "grok_request_audits"),
    ("response_ownership", "grok_response_ownership"),
    ("web_response_states", "grok_web_response_states"),
    ("image_pipeline_traces", "grok_pipeline_traces"),
    ("image_pipeline_segments", "grok_pipeline_segments"),
    ("chrome_tickets", "grok_chrome_tickets"),
    ("media_jobs", "grok_media_jobs"),
    ("media_assets", "grok_media_assets"),
    ("runtime_settings", "grok_runtime_settings"),
];

/// 双侧存在时始终逐字复制的列（密文/标识，永不触碰）。
const KEEP_RAW: &[&str] = &[
    "identity_key",
    "encrypted_primary",
    "encrypted_refresh",
    "encrypted_access_token",
    "encrypted_refresh_token",
    "encrypted_secret",
    "encrypted_proxy_url",
    "encrypted_cloudflare_cookie",
];

/// SQLite 系统表 / GORM 元数据永不复制。
const SKIP_SQLITE: &[&str] = &["sqlite_sequence", "sqlite_master"];

/// 批量页大小（对齐 Python execute_values page_size=200）。
const PAGE_SIZE: usize = 200;

// ── CLI ────────────────────────────────────────────────────────────────

struct Args {
    dry_run: bool,
    limit: Option<usize>,
    schema: String,
    identity_smoke: usize,
    decrypt_smoke: usize,
}

fn usage() -> String {
    "grok-etl — grok2api SQLite -> TNexus PG ETL

Usage: grok-etl [--dry-run] [--limit N] [--schema S] [--identity-smoke N] [--decrypt-smoke N]

Options:
  --dry-run             read SQLite, print plan; do not require/contact PG
  --limit N             only copy first N rows per table (smoke)
  --schema S            PG schema for grok tables (default: public)
  --identity-smoke N    accounts to sample for identity_key compare (default 10, 0=skip)
  --decrypt-smoke N     credentials to attempt AES-GCM decrypt (default 10, 0=skip; not vendored)
  -h, --help            show this help

Env:
  GROK_ETL_SOURCE       path to backend.db (required)
  GROK_ETL_PG_DSN       postgres:// URL (optional; missing -> plan only)
  GROK_CREDENTIAL_KEY   base64 32-byte AES-256 key (decrypt smoke; unused in Rust build)"
        .to_string()
}

/// 手写参数解析（对齐 grok 系无 clap 风格；行为近似 argparse：未知/缺值 → exit 2）。
fn parse_args(argv: &[String]) -> Result<Args, String> {
    let mut dry_run = false;
    let mut limit: Option<usize> = None;
    let mut schema = "public".to_string();
    let mut identity_smoke = 10usize;
    let mut decrypt_smoke = 10usize;

    let mut i = 0;
    while i < argv.len() {
        let a = argv[i].as_str();
        match a {
            "--dry-run" => dry_run = true,
            "--limit" => {
                i += 1;
                let v = argv.get(i).ok_or("--limit requires a value")?;
                limit = Some(
                    v.parse()
                        .map_err(|_| format!("--limit requires an integer, got {v}"))?,
                );
            }
            "--schema" => {
                i += 1;
                schema = argv.get(i).cloned().ok_or("--schema requires a value")?;
            }
            "--identity-smoke" => {
                i += 1;
                let v = argv.get(i).ok_or("--identity-smoke requires a value")?;
                identity_smoke = v
                    .parse()
                    .map_err(|_| format!("--identity-smoke requires an integer, got {v}"))?;
            }
            "--decrypt-smoke" => {
                i += 1;
                let v = argv.get(i).ok_or("--decrypt-smoke requires a value")?;
                decrypt_smoke = v
                    .parse()
                    .map_err(|_| format!("--decrypt-smoke requires an integer, got {v}"))?;
            }
            "-h" | "--help" => return Err("__help__".to_string()),
            other => return Err(format!("unrecognized argument: {other}")),
        }
        i += 1;
    }
    Ok(Args {
        dry_run,
        limit,
        schema,
        identity_smoke,
        decrypt_smoke,
    })
}

// ── 表规划 ────────────────────────────────────────────────────────────

#[derive(Debug)]
struct TablePlan {
    source: String,
    target: String,
    /// 列交集（KEEP_RAW 优先 + 源序），有序。
    columns: Vec<String>,
    src_exists: bool,
    dst_exists: bool,
}

/// 列交集构建（对齐 Python build_plans：KEEP_RAW 列在前，其余按源序）。
fn plan_columns(src_cols: &[String], dst_cols: &HashSet<String>) -> Vec<String> {
    let mut ordered: Vec<String> = src_cols
        .iter()
        .filter(|c| KEEP_RAW.contains(&c.as_str()) && dst_cols.contains(*c))
        .cloned()
        .collect();
    ordered.extend(
        src_cols
            .iter()
            .filter(|c| !KEEP_RAW.contains(&c.as_str()) && dst_cols.contains(*c))
            .cloned(),
    );
    ordered.retain(|c| c != "rowid");
    ordered
}

fn sqlite_columns(con: &Connection, table: &str) -> Vec<String> {
    let mut stmt = con
        .prepare(&format!(r#"PRAGMA table_info("{table}")"#))
        .unwrap_or_else(|e| panic!("PRAGMA table_info {table}: {e}"));
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .unwrap_or_else(|e| panic!("query {table} info: {e}"));
    rows.filter_map(Result::ok).collect()
}

fn sqlite_tables(con: &Connection) -> HashSet<String> {
    let mut stmt = con
        .prepare("SELECT name FROM sqlite_master WHERE type='table'")
        .expect("sqlite_master query");
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .expect("sqlite_master rows");
    rows.filter_map(Result::ok).collect()
}

async fn pg_column_types(
    conn: &mut PgConnection,
    schema: &str,
) -> Result<HashMap<String, HashMap<String, String>>, sqlx::Error> {
    let mut out: HashMap<String, HashMap<String, String>> = HashMap::new();
    let rows = sqlx::query(
        "SELECT table_name, column_name, data_type
         FROM information_schema.columns
         WHERE table_schema = $1
         ORDER BY table_name, ordinal_position",
    )
    .bind(schema)
    .fetch_all(&mut *conn)
    .await?;
    for r in rows {
        let tname: String = r.get(0);
        let cname: String = r.get(1);
        let dtype: String = r.get(2);
        out.entry(tname).or_default().insert(cname, dtype);
    }
    Ok(out)
}

// ── 类型转换（对齐 Python _coerce / _parse_ts）────────────────────────

/// SQLite 值 → PG 目标列类型的转换结果（对齐 Python 语义）。
enum Coerced {
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(String),
    Bytes(Vec<u8>),
    Json(String),
}

fn coerce(pg_type: &str, raw: Option<&rusqlite::types::Value>) -> Option<Coerced> {
    let raw = raw?;
    let t = pg_type.to_lowercase();
    if t.contains("bool") {
        return match raw {
            rusqlite::types::Value::Text(s) => {
                let low = s.trim().to_lowercase();
                if ["1", "true", "t", "yes", "on"].contains(&low.as_str()) {
                    Some(Coerced::Bool(true))
                } else if ["0", "false", "f", "no", "off", "", "0.0"].contains(&low.as_str()) {
                    Some(Coerced::Bool(false))
                } else {
                    None
                }
            }
            rusqlite::types::Value::Integer(i) => Some(Coerced::Bool(*i != 0)),
            rusqlite::types::Value::Real(f) => Some(Coerced::Bool(*f != 0.0)),
            _ => None,
        };
    }
    if t.contains("int") {
        return match raw {
            rusqlite::types::Value::Integer(i) => Some(Coerced::Int(*i)),
            rusqlite::types::Value::Text(s) => s.trim().parse::<i64>().ok().map(Coerced::Int),
            rusqlite::types::Value::Real(f) => Some(Coerced::Int(*f as i64)),
            _ => None,
        };
    }
    if t.contains("numeric") || t.contains("real") || t.contains("double") || t.contains("float") {
        return match raw {
            rusqlite::types::Value::Real(f) => Some(Coerced::Float(*f)),
            rusqlite::types::Value::Integer(i) => Some(Coerced::Float(*i as f64)),
            rusqlite::types::Value::Text(s) => s.trim().parse::<f64>().ok().map(Coerced::Float),
            _ => None,
        };
    }
    if t.contains("json") {
        return match raw {
            rusqlite::types::Value::Text(s) => Some(Coerced::Json(s.clone())),
            rusqlite::types::Value::Null => None,
            other => Some(Coerced::Json(format!("{other:?}"))),
        };
    }
    if t.contains("time") || t.contains("date") {
        return match raw {
            rusqlite::types::Value::Text(s) => {
                // ISO 归一化（空格→T）后原样交 PG 尝试 cast；解析失败也交原文。
                let s = s.trim();
                if s.is_empty() {
                    None
                } else {
                    Some(Coerced::Text(s.replace(' ', "T")))
                }
            }
            rusqlite::types::Value::Null => None,
            other => Some(Coerced::Text(format!("{other:?}"))),
        };
    }
    if t.contains("bytea") {
        return match raw {
            rusqlite::types::Value::Blob(b) => Some(Coerced::Bytes(b.clone())),
            rusqlite::types::Value::Text(s) => Some(Coerced::Bytes(s.as_bytes().to_vec())),
            rusqlite::types::Value::Null => None,
            other => Some(Coerced::Bytes(format!("{other:?}").into_bytes())),
        };
    }
    // text/varchar 等：原样透传
    match raw {
        rusqlite::types::Value::Text(s) => Some(Coerced::Text(s.clone())),
        rusqlite::types::Value::Integer(i) => Some(Coerced::Text(i.to_string())),
        rusqlite::types::Value::Real(f) => Some(Coerced::Text(f.to_string())),
        rusqlite::types::Value::Blob(b) => {
            Some(Coerced::Text(String::from_utf8_lossy(b).into_owned()))
        }
        rusqlite::types::Value::Null => None,
    }
}

// ── 复制执行 ──────────────────────────────────────────────────────────

async fn truncate_present(
    conn: &mut PgConnection,
    schema: &str,
    targets: &[&str],
    existing: &HashMap<String, HashMap<String, String>>,
) -> Result<usize, sqlx::Error> {
    let present: Vec<String> = targets
        .iter()
        .filter(|t| existing.contains_key(**t))
        .map(|t| format!(r#""{schema}"."{t}""#))
        .collect();
    if present.is_empty() {
        return Ok(0);
    }
    let sql = format!("TRUNCATE {} CASCADE RESTART IDENTITY", present.join(", "));
    sqlx::query(&sql).execute(&mut *conn).await?;
    println!("[truncate] {} grok target table(s) reset", present.len());
    Ok(present.len())
}

/// 批量插入（200 行/批，对齐 execute_values page_size=200 的语义边界）。
async fn insert_batch(
    conn: &mut PgConnection,
    schema: &str,
    target: &str,
    cols: &[String],
    rows: &[Vec<Option<Coerced>>],
) -> Result<(), sqlx::Error> {
    let col_list = cols
        .iter()
        .map(|c| format!(r#""{c}""#))
        .collect::<Vec<_>>()
        .join(", ");
    let placeholders = (1..=cols.len())
        .map(|i| format!("${i}"))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(r#"INSERT INTO "{schema}"."{target}" ({col_list}) VALUES ({placeholders})"#);

    for row in rows {
        let mut args = PgArguments::default();
        for v in row.iter() {
            match v {
                Some(Coerced::Bool(b)) => {
                    let _ = args.add(*b);
                }
                Some(Coerced::Int(i64v)) => {
                    let _ = args.add(*i64v);
                }
                Some(Coerced::Float(f)) => {
                    let _ = args.add(*f);
                }
                Some(Coerced::Text(s)) => {
                    let _ = args.add(s.clone());
                }
                Some(Coerced::Json(s)) => {
                    let _ = args.add(s.clone());
                }
                Some(Coerced::Bytes(b)) => {
                    let _ = args.add(b.clone());
                }
                None => {
                    // 目标列类型由 PG 推断；sqlx 对未类型化 null 用 Unknown。
                    let _ = args.add::<Option<String>>(None);
                }
            }
        }
        sqlx::query_with(&sql, args).execute(&mut *conn).await?;
    }
    Ok(())
}

async fn run_copy(
    conn: &mut PgConnection,
    schema: &str,
    plans: &[TablePlan],
    existing: &HashMap<String, HashMap<String, String>>,
    limit: Option<usize>,
    con: &Connection,
) -> Result<HashMap<String, usize>, sqlx::Error> {
    let mut copied = HashMap::new();
    for plan in plans {
        if !(plan.src_exists && plan.dst_exists) {
            println!(
                "[skip]   {} -> {}: missing source or target",
                plan.source, plan.target
            );
            continue;
        }
        let pg_types = existing.get(&plan.target).cloned().unwrap_or_default();
        let cols: Vec<String> = plan
            .columns
            .iter()
            .filter(|c| pg_types.contains_key(c.as_str()))
            .cloned()
            .collect();
        if cols.is_empty() {
            println!(
                "[skip]   {} -> {}: no intersecting columns",
                plan.source, plan.target
            );
            continue;
        }

        // 全量读取（limit 截断）→ 分批插入
        let mut batch: Vec<Vec<Option<Coerced>>> = Vec::new();
        let mut rows_total = 0usize;
        {
            let col_list = cols
                .iter()
                .map(|c| format!(r#""{c}""#))
                .collect::<Vec<_>>()
                .join(", ");
            let sel = format!(r#"SELECT {col_list} FROM "{}""#, plan.source);
            let mut stmt = con.prepare(&sel).map_err(|e| {
                sqlx::Error::Io(std::io::Error::other(format!(
                    "select {}: {e}",
                    plan.source
                )))
            })?;
            let mut rows = stmt.query([]).map_err(|e| {
                sqlx::Error::Io(std::io::Error::other(format!("query {}: {e}", plan.source)))
            })?;
            while let Some(row) = rows.next().map_err(|e| {
                sqlx::Error::Io(std::io::Error::other(format!("next {}: {e}", plan.source)))
            })? {
                let mut out = Vec::with_capacity(cols.len());
                for (ci, col) in cols.iter().enumerate() {
                    let pg_type = pg_types.get(col).map(|s| s.as_str()).unwrap_or("");
                    // rusqlite Row::get 按列索引取 Value
                    let v: rusqlite::types::Value = row.get(ci).map_err(|e| {
                        sqlx::Error::Io(std::io::Error::other(format!("get {}: {e}", plan.source)))
                    })?;
                    out.push(coerce(pg_type, Some(&v)));
                }
                batch.push(out);
                rows_total += 1;
                if batch.len() >= PAGE_SIZE {
                    insert_batch(conn, schema, &plan.target, &cols, &batch).await?;
                    batch.clear();
                }
                if let Some(l) = limit {
                    if rows_total >= l {
                        break;
                    }
                }
            }
        }
        if !batch.is_empty() {
            insert_batch(conn, schema, &plan.target, &cols, &batch).await?;
        }
        copied.insert(plan.target.clone(), rows_total);
        println!(
            "[copy]    {} -> {}: {} rows ({} cols)",
            plan.source,
            plan.target,
            rows_total,
            cols.len()
        );
    }
    Ok(copied)
}

async fn safe_counts(
    con: &Connection,
    conn: &mut PgConnection,
    schema: &str,
    src: &str,
    dst: &str,
) -> (i64, i64) {
    let sc: i64 = con
        .query_row(&format!(r#"SELECT COUNT(*) FROM "{src}""#), [], |r| {
            r.get(0)
        })
        .unwrap_or(-1);
    let pc: i64 = sqlx::query_scalar(&format!(r#"SELECT COUNT(*) FROM "{schema}"."{dst}""#))
        .fetch_one(&mut *conn)
        .await
        .unwrap_or(-1);
    (sc, pc)
}

async fn identity_key_smoke(
    con: &Connection,
    conn: &mut PgConnection,
    schema: &str,
    limit: usize,
) -> (usize, usize) {
    let mut src = HashMap::new();
    {
        let mut stmt = con
            .prepare("SELECT id, identity_key FROM provider_accounts ORDER BY id LIMIT ?")
            .expect("identity src prepare");
        let rows = stmt
            .query_map([rusqlite::types::Value::Integer(limit as i64)], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })
            .expect("identity src rows");
        for r in rows.flatten() {
            src.insert(r.0, r.1);
        }
    }
    let mut dst = HashMap::new();
    {
        let sql = format!(
            r#"SELECT id, identity_key FROM "{schema}"."grok_accounts" ORDER BY id LIMIT {limit}"#
        );
        let rows = sqlx::query(&sql)
            .fetch_all(&mut *conn)
            .await
            .unwrap_or_default();
        for r in rows {
            let id: i64 = r.get(0);
            let key: Option<String> = r.get(1);
            dst.insert(id, key);
        }
    }
    if src.is_empty() {
        return (0, 0);
    }
    let matched = src
        .iter()
        .filter(|(k, v)| dst.get(k).and_then(|o| o.as_deref()) == Some(v.as_str()))
        .count();
    (src.len(), matched)
}

// ── 主流程 ────────────────────────────────────────────────────────────

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

    let source = env::var("GROK_ETL_SOURCE").unwrap_or_default();
    let dsn = env::var("GROK_ETL_PG_DSN").unwrap_or_default();
    let key = env::var("GROK_CREDENTIAL_KEY").unwrap_or_default();

    if source.is_empty() {
        eprintln!("Set GROK_ETL_SOURCE (path to backend.db)");
        return ExitCode::from(2);
    }
    if !std::path::Path::new(&source).is_file() {
        eprintln!("Missing SQLite: {source}");
        return ExitCode::from(1);
    }

    let con = match Connection::open_with_flags(
        format!("file:{source}?mode=ro"),
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    ) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to open SQLite {source}: {e}");
            return ExitCode::from(1);
        }
    };

    let src_tables = sqlite_tables(&con);
    println!("[sqlite] {} tables in {source}", src_tables.len());
    for (go_name, _) in TABLE_MAP {
        if !src_tables.contains(*go_name) {
            println!("  [missing-source] {go_name}");
        }
    }

    // PG 连接（dry-run 不连；无 DSN 或失败降级出 plan）
    let mut pg_ok = false;
    let mut conn: Option<PgConnection> = None;
    if args.dry_run {
        println!("[mode] dry-run — PG not contacted");
    } else if dsn.is_empty() {
        eprintln!("[warn] GROK_ETL_PG_DSN not set — drawing plan only (no copy)");
    } else {
        match PgConnection::connect(&dsn).await {
            Ok(c) => {
                pg_ok = true;
                conn = Some(c);
            }
            Err(e) => {
                eprintln!("[warn] PG connect failed: {e}");
            }
        }
    }

    // 规划
    let (existing, plans) = if pg_ok {
        let mut pg = conn.take().expect("pg connection");
        let existing = match pg_column_types(&mut pg, &args.schema).await {
            Ok(e) => e,
            Err(e) => {
                eprintln!("[warn] PG information_schema query failed: {e}");
                HashMap::new()
            }
        };
        let plans = build_plans(&con, &existing);
        conn = Some(pg);
        (existing, plans)
    } else {
        (HashMap::new(), build_plans(&con, &HashMap::new()))
    };
    let copyable = plans
        .iter()
        .filter(|p| p.src_exists && p.dst_exists)
        .count();
    let total_cols: usize = plans.iter().map(|p| p.columns.len()).sum();
    println!(
        "\n[plan] {copyable}/{} table families copyable ({total_cols} intersecting columns)",
        TABLE_MAP.len()
    );

    let mut copied: HashMap<String, usize> = HashMap::new();
    if pg_ok {
        let mut pg = conn.take().expect("pg connection");
        let targets: Vec<&str> = plans.iter().map(|p| p.target.as_str()).collect();
        let _ = truncate_present(&mut pg, &args.schema, &targets, &existing).await;
        copied = match run_copy(&mut pg, &args.schema, &plans, &existing, args.limit, &con).await {
            Ok(c) => c,
            Err(e) => {
                eprintln!("[warn] copy failed: {e}");
                HashMap::new()
            }
        };
        conn = Some(pg);
    }

    // 校验
    if pg_ok {
        let mut pg = conn.take().expect("pg connection");
        println!("\n[validate] per-table count compare (sqlite vs pg):");
        for p in &plans {
            if !(p.src_exists && p.dst_exists) {
                continue;
            }
            let (sc, pc) = safe_counts(&con, &mut pg, &args.schema, &p.source, &p.target).await;
            let mark = if sc == pc { "OK " } else { "DIFF" };
            println!("  [{mark}] {:<28} sqlite={:<6} pg={pc}", p.source, sc);
        }

        if args.identity_smoke > 0 {
            let (n_src, n_match) =
                identity_key_smoke(&con, &mut pg, &args.schema, args.identity_smoke).await;
            println!("[validate] identity_key smoke: {n_match}/{n_src} sampled accounts match");
        }

        if args.decrypt_smoke > 0 && !key.is_empty() {
            // aes-gcm 未 vendored：打印说明并跳过（与 Python 的 decrypt_smoke 语义对齐，
            // 不失败）。接入 aes-gcm 后按 Go infra/security/cipher.go 语义实现。
            println!(
                "[validate] decrypt smoke skipped (aes-gcm not vendored in this build; \
                 GROK_CREDENTIAL_KEY set but no decrypt performed)"
            );
        } else {
            println!(
                "[validate] decrypt smoke skipped (GROK_CREDENTIAL_KEY unset or --decrypt-smoke 0)"
            );
        }
        if !copied.is_empty() && args.limit.is_some() {
            println!(
                "[note] --limit {} used (smoke only; re-run without to load all rows)",
                args.limit.unwrap_or(0)
            );
        }
    } else {
        println!("\n[validate] source-only counts (PG not connected):");
        for (go_name, _) in TABLE_MAP {
            if src_tables.contains(*go_name) && !SKIP_SQLITE.contains(go_name) {
                let row: i64 = con
                    .query_row(&format!(r#"SELECT COUNT(*) FROM "{go_name}""#), [], |r| {
                        r.get(0)
                    })
                    .unwrap_or(-1);
                println!("  [src] {:<28} sqlite={row}", *go_name);
            }
        }
    }

    let provider: i64 = con
        .query_row(r#"SELECT COUNT(*) FROM provider_accounts"#, [], |r| {
            r.get(0)
        })
        .unwrap_or(0);
    println!("\nprovider_accounts rows: {provider}");
    drop(con);

    if !pg_ok && !args.dry_run {
        println!(
            "[result] PG copy not performed (no DSN / connect) — plan + source validation only"
        );
        return ExitCode::SUCCESS;
    }
    println!("[result] ETL done");
    ExitCode::SUCCESS
}

/// 构建表规划（对齐 Python build_plans：src 存在性 + dst 列交集）。
fn build_plans(
    con: &Connection,
    dst_types: &HashMap<String, HashMap<String, String>>,
) -> Vec<TablePlan> {
    let src_tables = sqlite_tables(con);
    let mut plans = Vec::new();
    for (go_name, pg_name) in TABLE_MAP {
        let mut p = TablePlan {
            source: (*go_name).to_string(),
            target: (*pg_name).to_string(),
            columns: Vec::new(),
            src_exists: src_tables.contains(*go_name) && !SKIP_SQLITE.contains(go_name),
            dst_exists: dst_types.contains_key(*pg_name),
        };
        if p.src_exists && p.dst_exists {
            let dst_cols: HashSet<String> = dst_types
                .get(*pg_name)
                .map(|m| m.keys().cloned().collect())
                .unwrap_or_default();
            p.columns = plan_columns(&sqlite_columns(con, go_name), &dst_cols);
        }
        plans.push(p);
    }
    plans
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_map_31_unique() {
        assert_eq!(TABLE_MAP.len(), 31, "31 表映射");
        let mut names = HashSet::new();
        for (src, dst) in TABLE_MAP {
            assert!(names.insert(*src), "重复源表: {src}");
            assert!(names.insert(*dst), "重复目标表: {dst}");
        }
        // 父表在前抽查：provider_accounts 在 account_credentials 前
        let idx = |n: &str| TABLE_MAP.iter().position(|(s, _)| *s == n).unwrap();
        assert!(idx("provider_accounts") < idx("account_credentials"));
        assert!(idx("model_routes") < idx("model_route_aliases"));
    }

    #[test]
    fn coerce_bool_variants() {
        for s in ["1", "true", "t", "yes", "on", " TRUE "] {
            assert!(
                matches!(
                    coerce("boolean", Some(&rusqlite::types::Value::Text(s.into()))),
                    Some(Coerced::Bool(true))
                ),
                "{s}"
            );
        }
        for s in ["0", "false", "f", "no", "off", "", "0.0"] {
            assert!(
                matches!(
                    coerce("boolean", Some(&rusqlite::types::Value::Text(s.into()))),
                    Some(Coerced::Bool(false))
                ),
                "{s}"
            );
        }
        assert!(coerce("boolean", Some(&rusqlite::types::Value::Text("zzz".into()))).is_none());
        assert!(matches!(
            coerce("boolean", Some(&rusqlite::types::Value::Integer(1))),
            Some(Coerced::Bool(true))
        ));
        assert!(coerce("boolean", None).is_none());
    }

    #[test]
    fn coerce_numeric_and_bytea() {
        assert!(matches!(
            coerce("bigint", Some(&rusqlite::types::Value::Text("42".into()))),
            Some(Coerced::Int(42))
        ));
        assert!(coerce("bigint", Some(&rusqlite::types::Value::Text("abc".into()))).is_none());
        assert!(
            matches!(coerce("numeric", Some(&rusqlite::types::Value::Text("1.5".into()))), Some(Coerced::Float(f)) if f == 1.5)
        );
        assert!(
            matches!(coerce("double precision", Some(&rusqlite::types::Value::Integer(7))), Some(Coerced::Float(f)) if f == 7.0)
        );
        assert!(
            matches!(coerce("bytea", Some(&rusqlite::types::Value::Blob(vec![1, 2, 3]))), Some(Coerced::Bytes(b)) if b == vec![1, 2, 3])
        );
        assert!(
            matches!(coerce("bytea", Some(&rusqlite::types::Value::Text("hi".into()))), Some(Coerced::Bytes(b)) if b == b"hi".to_vec())
        );
    }

    #[test]
    fn coerce_json_and_time() {
        assert!(matches!(
            coerce(
                "jsonb",
                Some(&rusqlite::types::Value::Text(r#"{"a":1}"#.into()))
            ),
            Some(Coerced::Json(_))
        ));
        // ISO 归一化：空格 → T
        assert!(
            matches!(coerce("timestamp", Some(&rusqlite::types::Value::Text("2026-08-06 10:00:00".into()))), Some(Coerced::Text(s)) if s == "2026-08-06T10:00:00")
        );
        // 空串 → None
        assert!(coerce("timestamp", Some(&rusqlite::types::Value::Text("".into()))).is_none());
    }

    #[test]
    fn plan_columns_keep_raw_first() {
        let src = vec![
            "encrypted_primary".to_string(),
            "name".to_string(),
            "identity_key".to_string(),
            "age".to_string(),
        ];
        let dst: HashSet<String> = ["encrypted_primary", "name", "identity_key", "age", "extra"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let cols = plan_columns(&src, &dst);
        // KEEP_RAW 优先且按源序（Python：`[c for c in src_cols if c in _KEEP_RAW]`），再其余按源序
        assert_eq!(
            cols,
            vec!["encrypted_primary", "identity_key", "name", "age"]
        );
        // rowid 排除
        let src2 = vec!["rowid".to_string(), "name".to_string()];
        let dst2: HashSet<String> = ["rowid", "name"].iter().map(|s| s.to_string()).collect();
        assert_eq!(plan_columns(&src2, &dst2), vec!["name"]);
    }

    #[test]
    fn parse_args_ok_and_errors() {
        let argv: Vec<String> = vec![
            "--dry-run",
            "--limit",
            "10",
            "--schema",
            "grok",
            "--identity-smoke",
            "0",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        let args = parse_args(&argv).unwrap();
        assert!(args.dry_run);
        assert_eq!(args.limit, Some(10));
        assert_eq!(args.schema, "grok");
        assert_eq!(args.identity_smoke, 0);
        let bad: Vec<String> = vec!["--nope".into()];
        assert!(parse_args(&bad).is_err());
        let missing: Vec<String> = vec!["--limit".into()];
        assert!(parse_args(&missing).is_err());
        let badval: Vec<String> = vec!["--limit".into(), "abc".into()];
        assert!(parse_args(&badval).is_err());
    }

    #[test]
    fn parse_args_defaults() {
        let args = parse_args(&[]).unwrap();
        assert!(!args.dry_run);
        assert_eq!(args.limit, None);
        assert_eq!(args.schema, "public");
        assert_eq!(args.identity_smoke, 10);
        assert_eq!(args.decrypt_smoke, 10);
    }
}
