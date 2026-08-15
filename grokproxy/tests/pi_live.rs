//! Live integration tests: `pi` CLI against grok-4.6 providers.
//!
//! Run:
//!   cargo test --test pi_live -- --ignored
//!
//! Env:
//!   GROK46_PROVIDER_OPENAI    — pi provider id for OpenAI format
//!   GROK46_PROVIDER_ANTHROPIC — pi provider id for Anthropic format
//!   PI_BIN                    — default C:\software\nodejs\node_global\pi.cmd
//!   PI_CWD                    — default D:\SelfMadeTool\piAgent

mod pi_harness;

use pi_harness::{run_case, ALL_CASES};

fn provider_from_env(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|s| !s.trim().is_empty())
}

fn run_all_cases(provider_env: &str) {
    let provider = provider_from_env(provider_env).unwrap_or_else(|| {
        panic!("{provider_env} must be set for live pi tests");
    });

    let mut failures = Vec::new();

    for case in ALL_CASES {
        let (run, pass) = run_case(&provider, case);
        if pass {
            eprintln!(
                "[PASS] {} ({provider}) {}ms",
                case.id, run.elapsed_ms
            );
            continue;
        }

        let preview: String = run.text().chars().take(200).collect();
        eprintln!(
            "[FAIL] {} ({provider}) {}ms | exit={:?} timeout={} | {}",
            case.id,
            run.elapsed_ms,
            run.exit_code,
            run.timed_out,
            preview.replace('\n', " ")
        );
        failures.push(case.id);
    }

    assert!(
        failures.is_empty(),
        "{provider_env}={provider}: {} case(s) failed: {}",
        failures.len(),
        failures.join(", ")
    );
}

#[test]
#[ignore = "live pi + grok-4.6; set GROK46_PROVIDER_OPENAI"]
fn pi_live_openai_all_cases() {
    run_all_cases("GROK46_PROVIDER_OPENAI");
}

#[test]
#[ignore = "live pi + grok-4.6; set GROK46_PROVIDER_ANTHROPIC"]
fn pi_live_anthropic_all_cases() {
    run_all_cases("GROK46_PROVIDER_ANTHROPIC");
}

#[test]
fn pi_harness_case_catalog_size() {
    assert!(
        ALL_CASES.len() >= 100,
        "expected at least 100 cases, got {}",
        ALL_CASES.len()
    );
}
