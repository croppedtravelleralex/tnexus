//! Harness for live `pi` CLI runs against grok-4.6 providers.

mod cases;

use std::process::{Command, Output, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

pub use cases::{CaseSpec, ALL_CASES};

const DEFAULT_PI_BIN: &str = r"C:\software\nodejs\node_global\pi.cmd";
const DEFAULT_CWD: &str = r"D:\SelfMadeTool\piAgent";
const DEFAULT_MODEL: &str = "grok-4.6";
const DEFAULT_TIMEOUT_SECS: u64 = 120;

/// Options for a single non-interactive `pi` invocation.
#[derive(Debug, Clone)]
pub struct PiOpts {
    pub provider: String,
    pub prompt: String,
    pub no_tools: bool,
    pub timeout_secs: u64,
    pub pi_bin: String,
    pub cwd: String,
    pub model: String,
}

impl PiOpts {
    pub fn new(provider: impl Into<String>, prompt: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            prompt: prompt.into(),
            no_tools: true,
            timeout_secs: DEFAULT_TIMEOUT_SECS,
            pi_bin: pi_bin_from_env(),
            cwd: cwd_from_env(),
            model: DEFAULT_MODEL.to_string(),
        }
    }

    pub fn no_tools(mut self, no_tools: bool) -> Self {
        self.no_tools = no_tools;
        self
    }

    pub fn timeout_secs(mut self, secs: u64) -> Self {
        self.timeout_secs = secs;
        self
    }
}

/// Result of one `pi` run.
#[derive(Debug, Clone)]
pub struct PiRun {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
    pub elapsed_ms: u64,
    pub timed_out: bool,
    pub spawn_error: Option<String>,
}

impl PiRun {
    pub fn combined_output(&self) -> String {
        if self.stderr.is_empty() {
            self.stdout.clone()
        } else if self.stdout.is_empty() {
            self.stderr.clone()
        } else {
            format!("{}\n{}", self.stdout, self.stderr)
        }
    }

    pub fn text(&self) -> &str {
        self.combined_output().trim()
    }

    pub fn ok(&self) -> bool {
        self.spawn_error.is_none() && !self.timed_out && self.exit_code == Some(0)
    }
}

pub fn pi_bin_from_env() -> String {
    std::env::var("PI_BIN").unwrap_or_else(|_| DEFAULT_PI_BIN.to_string())
}

pub fn cwd_from_env() -> String {
    std::env::var("PI_CWD").unwrap_or_else(|_| DEFAULT_CWD.to_string())
}

/// Run `pi` non-interactively and wait up to `opts.timeout_secs`.
pub fn run_pi(opts: &PiOpts) -> PiRun {
    let started = Instant::now();
    let timeout = Duration::from_secs(opts.timeout_secs);
    let (tx, rx) = mpsc::channel();

    let opts = opts.clone();
    thread::spawn(move || {
        let result = build_command(&opts).output();
        let _ = tx.send(result);
    });

    match rx.recv_timeout(timeout) {
        Ok(Ok(output)) => output_to_run(output, started.elapsed(), false, None),
        Ok(Err(err)) => PiRun {
            stdout: String::new(),
            stderr: String::new(),
            exit_code: None,
            elapsed_ms: started.elapsed().as_millis() as u64,
            timed_out: false,
            spawn_error: Some(err.to_string()),
        },
        Err(mpsc::RecvTimeoutError::Timeout) => PiRun {
            stdout: String::new(),
            stderr: String::new(),
            exit_code: None,
            elapsed_ms: started.elapsed().as_millis() as u64,
            timed_out: true,
            spawn_error: Some(format!("timed out after {}s", opts.timeout_secs)),
        },
        Err(mpsc::RecvTimeoutError::Disconnected) => PiRun {
            stdout: String::new(),
            stderr: String::new(),
            exit_code: None,
            elapsed_ms: started.elapsed().as_millis() as u64,
            timed_out: false,
            spawn_error: Some("pi worker thread disconnected".to_string()),
        },
    }
}

fn build_command(opts: &PiOpts) -> Command {
    let mut args = vec![
        "--provider".to_string(),
        opts.provider.clone(),
        "--model".to_string(),
        opts.model.clone(),
        "--thinking".to_string(),
        "off".to_string(),
        "--no-session".to_string(),
        "--no-skills".to_string(),
        "--no-prompt-templates".to_string(),
        "--no-themes".to_string(),
        "--no-context-files".to_string(),
        "--print".to_string(),
        opts.prompt.clone(),
    ];

    if opts.no_tools {
        args.push("--no-tools".to_string());
    }

    let mut cmd = pi_command(&opts.pi_bin);
    cmd.current_dir(&opts.cwd)
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    cmd
}

fn pi_command(pi_bin: &str) -> Command {
    #[cfg(windows)]
    {
        let mut cmd = Command::new("cmd");
        cmd.arg("/C").arg(pi_bin);
        cmd
    }
    #[cfg(not(windows))]
    {
        Command::new(pi_bin)
    }
}

fn output_to_run(
    output: Output,
    elapsed: Duration,
    timed_out: bool,
    spawn_error: Option<String>,
) -> PiRun {
    PiRun {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        exit_code: output.status.code(),
        elapsed_ms: elapsed.as_millis() as u64,
        timed_out,
        spawn_error,
    }
}

// --- assert helpers ---

pub fn assert_not_empty(text: &str) -> bool {
    !text.trim().is_empty()
}

pub fn assert_contains(text: &str, needle: &str) -> bool {
    text.contains(needle)
}

pub fn assert_contains_ignore_case(text: &str, needle: &str) -> bool {
    text.to_ascii_lowercase().contains(&needle.to_ascii_lowercase())
}

pub fn assert_has_number(text: &str, n: i64) -> bool {
    let needle = n.to_string();
    text.split(|c: char| !c.is_ascii_digit() && c != '-' && c != '.')
        .any(|part| part == needle)
}

pub fn assert_json_object(text: &str) -> bool {
    extract_json_value(text).is_some()
}

pub fn assert_json_field_bool(text: &str, key: &str, expected: bool) -> bool {
    let Some(value) = extract_json_value(text) else {
        return false;
    };
    value.get(key).and_then(|v| v.as_bool()) == Some(expected)
}

pub fn assert_json_field_i64(text: &str, key: &str, expected: i64) -> bool {
    let Some(value) = extract_json_value(text) else {
        return false;
    };
    value.get(key).and_then(|v| v.as_i64()) == Some(expected)
}

pub fn assert_json_field_str(text: &str, key: &str, expected: &str) -> bool {
    let Some(value) = extract_json_value(text) else {
        return false;
    };
    value.get(key).and_then(|v| v.as_str()) == Some(expected)
}

pub fn assert_json_array_len(text: &str, len: usize) -> bool {
    let Some(value) = extract_json_value(text) else {
        return false;
    };
    value.as_array().is_some_and(|arr| arr.len() == len)
}

pub fn extract_json_value(text: &str) -> Option<serde_json::Value> {
    let trimmed = text.trim();
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) {
        return Some(v);
    }
    let start = trimmed.find('{').or_else(|| trimmed.find('['))?;
    let end = trimmed.rfind('}').or_else(|| trimmed.rfind(']'))?;
    if end < start {
        return None;
    }
    serde_json::from_str(&trimmed[start..=end]).ok()
}

pub fn run_case(provider: &str, case: &CaseSpec) -> (PiRun, bool) {
    let opts = PiOpts::new(provider, case.prompt).no_tools(case.no_tools);
    let run = run_pi(&opts);
    let text = run.text();
    let pass = !text.is_empty() && !run.timed_out && (case.check)(text);
    (run, pass)
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    #[test]
    fn extract_json_from_wrapped_text() {
        let text = r"Here is JSON: {"ok":true,"n":7} done";
        let v = extract_json_value(text).expect("json");
        assert_eq!(v["ok"], true);
        assert_eq!(v["n"], 7);
    }

    #[test]
    fn assert_has_number_matches_token() {
        assert!(assert_has_number("answer: 391", 391));
        assert!(!assert_has_number("answer: 392", 391));
    }
}
