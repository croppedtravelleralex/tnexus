//! Live case catalog for grok-4.6 via `pi`.

use super::{
    assert_contains, assert_contains_ignore_case, assert_has_number, assert_json_array_len,
    assert_json_field_bool, assert_json_field_i64, assert_json_field_str, assert_json_object,
    assert_not_empty,
};

pub struct CaseSpec {
    pub id: &'static str,
    pub prompt: &'static str,
    pub no_tools: bool,
    pub check: fn(&str) -> bool,
}

// --- CONN (12) ---

fn conn_ping(text: &str) -> bool {
    assert_contains_ignore_case(text, "PING_OK")
}

fn conn_echo(text: &str) -> bool {
    assert_contains_ignore_case(text, "ECHO=HELLO")
}

fn conn_ready(text: &str) -> bool {
    assert_contains_ignore_case(text, "READY")
}

fn conn_alive(text: &str) -> bool {
    assert_contains_ignore_case(text, "ALIVE")
}

fn conn_ok(text: &str) -> bool {
    assert_contains_ignore_case(text, "OK")
}

fn conn_yes(text: &str) -> bool {
    assert_contains_ignore_case(text, "YES")
}

fn conn_one(text: &str) -> bool {
    assert_has_number(text, 1)
}

fn conn_hi(text: &str) -> bool {
    assert_contains_ignore_case(text, "HI")
}

fn conn_tag(text: &str) -> bool {
    assert_contains_ignore_case(text, "CONN_TAG")
}

fn conn_model(text: &str) -> bool {
    assert_contains_ignore_case(text, "grok") && text.contains("4.6")
}

fn conn_cn(text: &str) -> bool {
    text.contains('好')
}

fn conn_online(text: &str) -> bool {
    assert_contains_ignore_case(text, "ONLINE")
}

// --- MATH (12) ---

fn math_mul_17_23(text: &str) -> bool {
    assert_has_number(text, 391)
}

fn math_add_123_456(text: &str) -> bool {
    assert_has_number(text, 579)
}

fn math_sub_1000_357(text: &str) -> bool {
    assert_has_number(text, 643)
}

fn math_div_84_7(text: &str) -> bool {
    assert_has_number(text, 12)
}

fn math_sq_15(text: &str) -> bool {
    assert_has_number(text, 225)
}

fn math_pct_200_15(text: &str) -> bool {
    assert_has_number(text, 30)
}

fn math_fact_5(text: &str) -> bool {
    assert_has_number(text, 120)
}

fn math_sqrt_144(text: &str) -> bool {
    assert_has_number(text, 12)
}

fn math_pow_2_10(text: &str) -> bool {
    assert_has_number(text, 1024)
}

fn math_chain_3_7_5(text: &str) -> bool {
    assert_has_number(text, 50)
}

fn math_dec_3_5_2(text: &str) -> bool {
    assert_has_number(text, 7)
}

fn math_mod_17_5(text: &str) -> bool {
    assert_has_number(text, 2)
}

// --- JSON (12) ---

fn json_ok_true(text: &str) -> bool {
    assert_json_field_bool(text, "ok", true)
}

fn json_name_alice(text: &str) -> bool {
    assert_json_field_str(text, "name", "alice")
}

fn json_count_42(text: &str) -> bool {
    assert_json_field_i64(text, "count", 42)
}

fn json_items_3(text: &str) -> bool {
    assert_json_array_len(text, 3)
}

fn json_nested_b_1(text: &str) -> bool {
    super::extract_json_value(text)
        .and_then(|v| v.get("a").and_then(|a| a.get("b")).and_then(|b| b.as_i64()))
        == Some(1)
}

fn json_flag_false(text: &str) -> bool {
    assert_json_field_bool(text, "flag", false)
}

fn json_status_done(text: &str) -> bool {
    assert_json_field_str(text, "status", "done")
}

fn json_version_1_0(text: &str) -> bool {
    assert_json_field_str(text, "version", "1.0")
}

fn json_x_y(text: &str) -> bool {
    assert_json_field_i64(text, "x", 1) && assert_json_field_i64(text, "y", 2)
}

fn json_msg_hi(text: &str) -> bool {
    assert_json_field_str(text, "msg", "hi")
}

fn json_score_99(text: &str) -> bool {
    assert_json_field_i64(text, "score", 99)
}

fn json_valid_object(text: &str) -> bool {
    assert_json_object(text)
}

// --- BASH (8) ---

fn bash_echo_ok(text: &str) -> bool {
    assert_contains_ignore_case(text, "BASH_OK")
}

fn bash_true_exit(text: &str) -> bool {
    assert_contains_ignore_case(text, "BASH_TRUE")
}

fn bash_hostname(text: &str) -> bool {
    assert_not_empty(text) && text.len() < 200
}

fn bash_pwd(text: &str) -> bool {
    text.contains("piAgent") || text.contains('/') || text.contains('\\')
}

fn bash_date_year(text: &str) -> bool {
    assert_has_number(text, 2026) || assert_has_number(text, 2025)
}

fn bash_seq_3(text: &str) -> bool {
    assert_has_number(text, 3)
}

fn bash_wc_hello(text: &str) -> bool {
    assert_has_number(text, 6) || assert_has_number(text, 5)
}

fn bash_uname(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("windows")
        || lower.contains("linux")
        || lower.contains("darwin")
        || lower.contains("mingw")
}

// --- READ (8) ---

fn read_settings(text: &str) -> bool {
    assert_contains(text, "settings") || text.contains('{')
}

fn read_models(text: &str) -> bool {
    assert_contains_ignore_case(text, "grok") || assert_contains(text, "provider")
}

fn read_e2e_script(text: &str) -> bool {
    assert_contains_ignore_case(text, "grok") || assert_contains(text, "CASES")
}

fn read_auth_json(text: &str) -> bool {
    assert_contains(text, "auth") || assert_json_object(text)
}

fn read_newapi_sites(text: &str) -> bool {
    assert_contains_ignore_case(text, "newapi") || assert_contains(text, "http")
}

fn read_launch_script(text: &str) -> bool {
    assert_contains_ignore_case(text, "pi") || assert_contains(text, "grok")
}

fn read_patch_script(text: &str) -> bool {
    assert_contains(text, "patch") || assert_contains(text, "apply")
}

fn read_reports_dir(text: &str) -> bool {
    assert_contains_ignore_case(text, "grok46") || assert_contains(text, "report")
}

// --- GREP (8) ---

fn grep_grok_in_models(text: &str) -> bool {
    assert_contains_ignore_case(text, "grok")
}

fn grep_provider_in_models(text: &str) -> bool {
    assert_contains_ignore_case(text, "provider")
}

fn grep_e2e_in_script(text: &str) -> bool {
    assert_contains_ignore_case(text, "e2e") || assert_contains(text, "CASES")
}

fn grep_openai_in_script(text: &str) -> bool {
    assert_contains_ignore_case(text, "openai")
}

fn grep_anthropic_in_script(text: &str) -> bool {
    assert_contains_ignore_case(text, "anthropic")
}

fn grep_pi_in_launch(text: &str) -> bool {
    assert_contains_ignore_case(text, "pi")
}

fn grep_patch_apply(text: &str) -> bool {
    assert_contains(text, "apply") || assert_contains(text, "patch")
}

fn grep_reports_grok46(text: &str) -> bool {
    assert_contains_ignore_case(text, "grok46")
}

pub const ALL_CASES: &[CaseSpec] = &[
    // CONN
    CaseSpec {
        id: "conn_ping",
        prompt: "只回复一行：PING_OK",
        no_tools: true,
        check: conn_ping,
    },
    CaseSpec {
        id: "conn_echo",
        prompt: "只回复一行：ECHO=HELLO",
        no_tools: true,
        check: conn_echo,
    },
    CaseSpec {
        id: "conn_ready",
        prompt: "只回复一行：READY",
        no_tools: true,
        check: conn_ready,
    },
    CaseSpec {
        id: "conn_alive",
        prompt: "只回复一行：ALIVE",
        no_tools: true,
        check: conn_alive,
    },
    CaseSpec {
        id: "conn_ok",
        prompt: "只回复一行：OK",
        no_tools: true,
        check: conn_ok,
    },
    CaseSpec {
        id: "conn_yes",
        prompt: "只回复一行：YES",
        no_tools: true,
        check: conn_yes,
    },
    CaseSpec {
        id: "conn_one",
        prompt: "只回复数字：1",
        no_tools: true,
        check: conn_one,
    },
    CaseSpec {
        id: "conn_hi",
        prompt: "只回复：HI",
        no_tools: true,
        check: conn_hi,
    },
    CaseSpec {
        id: "conn_tag",
        prompt: "只回复一行，必须包含标记 CONN_TAG",
        no_tools: true,
        check: conn_tag,
    },
    CaseSpec {
        id: "conn_model",
        prompt: "只回复一行，包含模型名 grok-4.6",
        no_tools: true,
        check: conn_model,
    },
    CaseSpec {
        id: "conn_cn",
        prompt: "只回复一个字：好",
        no_tools: true,
        check: conn_cn,
    },
    CaseSpec {
        id: "conn_online",
        prompt: "Reply with only: ONLINE",
        no_tools: true,
        check: conn_online,
    },
    // MATH
    CaseSpec {
        id: "math_mul_17_23",
        prompt: "17乘23等于多少？只回复数字",
        no_tools: true,
        check: math_mul_17_23,
    },
    CaseSpec {
        id: "math_add_123_456",
        prompt: "123+456等于多少？只回复数字",
        no_tools: true,
        check: math_add_123_456,
    },
    CaseSpec {
        id: "math_sub_1000_357",
        prompt: "1000减357等于多少？只回复数字",
        no_tools: true,
        check: math_sub_1000_357,
    },
    CaseSpec {
        id: "math_div_84_7",
        prompt: "84除以7等于多少？只回复数字",
        no_tools: true,
        check: math_div_84_7,
    },
    CaseSpec {
        id: "math_sq_15",
        prompt: "15的平方是多少？只回复数字",
        no_tools: true,
        check: math_sq_15,
    },
    CaseSpec {
        id: "math_pct_200_15",
        prompt: "200的15%是多少？只回复数字",
        no_tools: true,
        check: math_pct_200_15,
    },
    CaseSpec {
        id: "math_fact_5",
        prompt: "5的阶乘是多少？只回复数字",
        no_tools: true,
        check: math_fact_5,
    },
    CaseSpec {
        id: "math_sqrt_144",
        prompt: "144的平方根是多少？只回复数字",
        no_tools: true,
        check: math_sqrt_144,
    },
    CaseSpec {
        id: "math_pow_2_10",
        prompt: "2的10次方是多少？只回复数字",
        no_tools: true,
        check: math_pow_2_10,
    },
    CaseSpec {
        id: "math_chain_3_7_5",
        prompt: "(3+7)*5等于多少？只回复数字",
        no_tools: true,
        check: math_chain_3_7_5,
    },
    CaseSpec {
        id: "math_dec_3_5_2",
        prompt: "3.5乘2等于多少？只回复数字",
        no_tools: true,
        check: math_dec_3_5_2,
    },
    CaseSpec {
        id: "math_mod_17_5",
        prompt: "17除以5的余数是多少？只回复数字",
        no_tools: true,
        check: math_mod_17_5,
    },
    // JSON
    CaseSpec {
        id: "json_ok_true",
        prompt: r#"只输出合法 JSON：{"ok":true}"#,
        no_tools: true,
        check: json_ok_true,
    },
    CaseSpec {
        id: "json_name_alice",
        prompt: r#"只输出合法 JSON：{"name":"alice"}"#,
        no_tools: true,
        check: json_name_alice,
    },
    CaseSpec {
        id: "json_count_42",
        prompt: r#"只输出合法 JSON：{"count":42}"#,
        no_tools: true,
        check: json_count_42,
    },
    CaseSpec {
        id: "json_items_3",
        prompt: r#"只输出合法 JSON 数组：[1,2,3]"#,
        no_tools: true,
        check: json_items_3,
    },
    CaseSpec {
        id: "json_nested_b_1",
        prompt: r#"只输出合法 JSON：{"a":{"b":1}}"#,
        no_tools: true,
        check: json_nested_b_1,
    },
    CaseSpec {
        id: "json_flag_false",
        prompt: r#"只输出合法 JSON：{"flag":false}"#,
        no_tools: true,
        check: json_flag_false,
    },
    CaseSpec {
        id: "json_status_done",
        prompt: r#"只输出合法 JSON：{"status":"done"}"#,
        no_tools: true,
        check: json_status_done,
    },
    CaseSpec {
        id: "json_version_1_0",
        prompt: r#"只输出合法 JSON：{"version":"1.0"}"#,
        no_tools: true,
        check: json_version_1_0,
    },
    CaseSpec {
        id: "json_x_y",
        prompt: r#"只输出合法 JSON：{"x":1,"y":2}"#,
        no_tools: true,
        check: json_x_y,
    },
    CaseSpec {
        id: "json_msg_hi",
        prompt: r#"只输出合法 JSON：{"msg":"hi"}"#,
        no_tools: true,
        check: json_msg_hi,
    },
    CaseSpec {
        id: "json_score_99",
        prompt: r#"只输出合法 JSON：{"score":99}"#,
        no_tools: true,
        check: json_score_99,
    },
    CaseSpec {
        id: "json_valid_object",
        prompt: r#"只输出合法 JSON 对象，包含字段 "ready": true"#,
        no_tools: true,
        check: json_valid_object,
    },
    // BASH
    CaseSpec {
        id: "bash_echo_ok",
        prompt: "用 bash 执行 `echo BASH_OK`，只回复命令输出",
        no_tools: false,
        check: bash_echo_ok,
    },
    CaseSpec {
        id: "bash_true_exit",
        prompt: "用 bash 执行 `echo BASH_TRUE && true`，只回复输出",
        no_tools: false,
        check: bash_true_exit,
    },
    CaseSpec {
        id: "bash_hostname",
        prompt: "用 bash 执行 hostname 或等效命令，只回复主机名",
        no_tools: false,
        check: bash_hostname,
    },
    CaseSpec {
        id: "bash_pwd",
        prompt: "用 bash 执行 pwd，只回复当前目录路径",
        no_tools: false,
        check: bash_pwd,
    },
    CaseSpec {
        id: "bash_date_year",
        prompt: "用 bash 执行 date +%Y，只回复四位年份",
        no_tools: false,
        check: bash_date_year,
    },
    CaseSpec {
        id: "bash_seq_3",
        prompt: "用 bash 执行 `seq 1 3 | tail -1`，只回复最后一行数字",
        no_tools: false,
        check: bash_seq_3,
    },
    CaseSpec {
        id: "bash_wc_hello",
        prompt: "用 bash 执行 `echo -n hello | wc -c`，只回复字符数",
        no_tools: false,
        check: bash_wc_hello,
    },
    CaseSpec {
        id: "bash_uname",
        prompt: "用 bash 执行 uname -s 或等效命令，只回复系统名称",
        no_tools: false,
        check: bash_uname,
    },
    // READ
    CaseSpec {
        id: "read_settings",
        prompt: "读取 .pi-agent/settings.json 并回复是否包含 settings 或 JSON 结构",
        no_tools: false,
        check: read_settings,
    },
    CaseSpec {
        id: "read_models",
        prompt: "读取 .pi-agent/models.json 并回复是否包含 grok 或 provider",
        no_tools: false,
        check: read_models,
    },
    CaseSpec {
        id: "read_e2e_script",
        prompt: "读取 scripts/e2e_grok_score.mjs 并回复是否提到 grok 或 CASES",
        no_tools: false,
        check: read_e2e_script,
    },
    CaseSpec {
        id: "read_auth_json",
        prompt: "读取 .pi-agent/auth.json 并回复是否包含 auth 或 JSON",
        no_tools: false,
        check: read_auth_json,
    },
    CaseSpec {
        id: "read_newapi_sites",
        prompt: "读取 .pi-agent/newapi-sites.json 并回复是否包含 newapi 或 http",
        no_tools: false,
        check: read_newapi_sites,
    },
    CaseSpec {
        id: "read_launch_script",
        prompt: "读取 scripts/pi-grok-openai.ps1 并回复是否提到 pi 或 grok",
        no_tools: false,
        check: read_launch_script,
    },
    CaseSpec {
        id: "read_patch_script",
        prompt: "读取 patches/apply-openai-toolcalls.mjs 并回复是否提到 patch 或 apply",
        no_tools: false,
        check: read_patch_script,
    },
    CaseSpec {
        id: "read_reports_dir",
        prompt: "查看 reports 目录下任一 grok46 报告文件名或内容摘要",
        no_tools: false,
        check: read_reports_dir,
    },
    // GREP
    CaseSpec {
        id: "grep_grok_in_models",
        prompt: "在 .pi-agent/models.json 中搜索 grok，只回复匹配行或摘要",
        no_tools: false,
        check: grep_grok_in_models,
    },
    CaseSpec {
        id: "grep_provider_in_models",
        prompt: "在 .pi-agent/models.json 中搜索 provider，只回复匹配摘要",
        no_tools: false,
        check: grep_provider_in_models,
    },
    CaseSpec {
        id: "grep_e2e_in_script",
        prompt: "在 scripts/e2e_grok_score.mjs 中搜索 e2e 或 CASES，只回复摘要",
        no_tools: false,
        check: grep_e2e_in_script,
    },
    CaseSpec {
        id: "grep_openai_in_script",
        prompt: "在 scripts/e2e_grok_score.mjs 中搜索 OPENAI，只回复摘要",
        no_tools: false,
        check: grep_openai_in_script,
    },
    CaseSpec {
        id: "grep_anthropic_in_script",
        prompt: "在 scripts/e2e_grok_score.mjs 中搜索 ANTHROPIC，只回复摘要",
        no_tools: false,
        check: grep_anthropic_in_script,
    },
    CaseSpec {
        id: "grep_pi_in_launch",
        prompt: "在 scripts/pi-grok-openai.ps1 中搜索 pi，只回复摘要",
        no_tools: false,
        check: grep_pi_in_launch,
    },
    CaseSpec {
        id: "grep_patch_apply",
        prompt: "在 patches 目录中搜索 apply，只回复摘要",
        no_tools: false,
        check: grep_patch_apply,
    },
    CaseSpec {
        id: "grep_reports_grok46",
        prompt: "在 reports 目录中搜索 grok46，只回复摘要",
        no_tools: false,
        check: grep_reports_grok46,
    },
    // MATH extra
    CaseSpec { id: "math_12x11", prompt: "只回复一行：MATH=132（12乘11）", no_tools: true, check: |t| t.contains("132") },
    CaseSpec { id: "math_13x7", prompt: "只回复一行：MATH=91（13乘7）", no_tools: true, check: |t| t.contains("91") },
    CaseSpec { id: "math_15x6", prompt: "只回复一行：MATH=90", no_tools: true, check: |t| t.contains("90") },
    CaseSpec { id: "math_99plus1", prompt: "只回复一行：99+1=", no_tools: true, check: |t| t.contains("100") },
    CaseSpec { id: "math_50x2", prompt: "只回复一行：50*2=", no_tools: true, check: |t| t.contains("100") },
    CaseSpec { id: "math_144div12", prompt: "只回复一行：144/12=", no_tools: true, check: |t| t.contains("12") },
    CaseSpec { id: "math_2pow10", prompt: "只回复一行：2的10次方=", no_tools: true, check: |t| t.contains("1024") },
    CaseSpec { id: "math_sqrt81", prompt: "只回复一行：81的平方根=", no_tools: true, check: |t| t.contains("9") },
    CaseSpec { id: "math_1000minus7", prompt: "只回复一行：1000-7=", no_tools: true, check: |t| t.contains("993") },
    CaseSpec { id: "math_7x8", prompt: "只回复一行：7*8=", no_tools: true, check: |t| t.contains("56") },
    CaseSpec { id: "math_11x11", prompt: "只回复一行：11*11=", no_tools: true, check: |t| t.contains("121") },
    CaseSpec { id: "math_25x4", prompt: "只回复一行：25*4=", no_tools: true, check: |t| t.contains("100") },
    // JSON extra
    CaseSpec { id: "json_ok_false", prompt: "只输出JSON：{\"ok\":false}", no_tools: true, check: |t| t.contains("\"ok\":false") || t.contains("\"ok\": false") },
    CaseSpec { id: "json_arr", prompt: "只输出JSON数组：[1,2,3]", no_tools: true, check: |t| t.contains("[1") && t.contains("3]") },
    CaseSpec { id: "json_name", prompt: "只输出JSON：{\"name\":\"pi\"}", no_tools: true, check: |t| t.contains("pi") },
    CaseSpec { id: "json_count3", prompt: "只输出JSON：{\"count\":3}", no_tools: true, check: |t| t.contains("3") },
    CaseSpec { id: "json_flag", prompt: "只输出JSON：{\"enabled\":true}", no_tools: true, check: |t| t.contains("true") },
    CaseSpec { id: "json_empty_obj", prompt: "只输出JSON：{}", no_tools: true, check: |t| t.contains("{}") },
    CaseSpec { id: "json_nested", prompt: "只输出JSON：{\"a\":{\"b\":1}}", no_tools: true, check: |t| t.contains("\"b\"") },
    CaseSpec { id: "json_list2", prompt: "只输出JSON：{\"items\":[\"x\",\"y\"]}", no_tools: true, check: |t| t.contains("x") && t.contains("y") },
    CaseSpec { id: "json_version", prompt: "只输出JSON：{\"version\":1}", no_tools: true, check: |t| t.contains("version") },
    CaseSpec { id: "json_status", prompt: "只输出JSON：{\"status\":\"ok\"}", no_tools: true, check: |t| t.contains("ok") },
    CaseSpec { id: "json_code", prompt: "只输出JSON：{\"code\":0}", no_tools: true, check: |t| t.contains("0") },
    CaseSpec { id: "json_id", prompt: "只输出JSON：{\"id\":\"abc\"}", no_tools: true, check: |t| t.contains("abc") },
    // TOOL bash extra
    CaseSpec { id: "tool_echo_a", prompt: "用bash执行 echo A，只回复输出", no_tools: false, check: |t| t.contains('A') },
    CaseSpec { id: "tool_echo_b", prompt: "用bash执行 echo B，只回复输出", no_tools: false, check: |t| t.contains('B') },
    CaseSpec { id: "tool_echo_42", prompt: "用bash执行 echo 42，只回复数字", no_tools: false, check: |t| t.contains("42") },
    CaseSpec { id: "tool_echo_ok", prompt: "用bash执行 echo OK，只回复OK", no_tools: false, check: |t| t.contains("OK") },
    CaseSpec { id: "tool_echo_test", prompt: "用bash执行 echo TEST，只回复TEST", no_tools: false, check: |t| t.contains("TEST") },
    CaseSpec { id: "tool_echo_pi", prompt: "用bash执行 echo PI，只回复PI", no_tools: false, check: |t| t.contains("PI") },
    CaseSpec { id: "tool_echo_rust", prompt: "用bash执行 echo RUST，只回复RUST", no_tools: false, check: |t| t.contains("RUST") },
    CaseSpec { id: "tool_echo_grok", prompt: "用bash执行 echo GROK，只回复GROK", no_tools: false, check: |t| t.contains("GROK") },
    CaseSpec { id: "tool_echo_done", prompt: "用bash执行 echo DONE，只回复DONE", no_tools: false, check: |t| t.contains("DONE") },
    CaseSpec { id: "tool_echo_pass", prompt: "用bash执行 echo PASS，只回复PASS", no_tools: false, check: |t| t.contains("PASS") },
    CaseSpec { id: "tool_echo_zero", prompt: "用bash执行 echo 0，只回复0", no_tools: false, check: |t| t.contains('0') },
    CaseSpec { id: "tool_echo_one", prompt: "用bash执行 echo 1，只回复1", no_tools: false, check: |t| t.contains('1') },
    // CONN extra
    CaseSpec { id: "conn_go", prompt: "只回复：GO", no_tools: true, check: |t| t.contains("GO") },
    CaseSpec { id: "conn_run", prompt: "只回复：RUN", no_tools: true, check: |t| t.contains("RUN") },
    CaseSpec { id: "conn_up", prompt: "只回复：UP", no_tools: true, check: |t| t.contains("UP") },
    CaseSpec { id: "conn_on", prompt: "只回复：ON", no_tools: true, check: |t| t.contains("ON") },
    CaseSpec { id: "conn_zero", prompt: "只回复数字0", no_tools: true, check: |t| t.contains('0') },
    CaseSpec { id: "conn_two", prompt: "只回复数字2", no_tools: true, check: |t| t.contains('2') },
    CaseSpec { id: "conn_three", prompt: "只回复数字3", no_tools: true, check: |t| t.contains('3') },
    CaseSpec { id: "conn_four", prompt: "只回复数字4", no_tools: true, check: |t| t.contains('4') },
    CaseSpec { id: "conn_five", prompt: "只回复数字5", no_tools: true, check: |t| t.contains('5') },
    CaseSpec { id: "conn_six", prompt: "只回复数字6", no_tools: true, check: |t| t.contains('6') },
    CaseSpec { id: "conn_seven", prompt: "只回复数字7", no_tools: true, check: |t| t.contains('7') },
    CaseSpec { id: "conn_eight", prompt: "只回复数字8", no_tools: true, check: |t| t.contains('8') },
    // REG / format smoke
    CaseSpec { id: "reg_grok46_name", prompt: "只回复模型名 grok-4.6", no_tools: true, check: |t| t.contains("grok") && t.contains("4.6") },
    CaseSpec { id: "reg_no_markdown", prompt: "只回复单词PLAIN，不要markdown", no_tools: true, check: |t| t.contains("PLAIN") },
    CaseSpec { id: "reg_short_en", prompt: "Reply with exactly: OK", no_tools: true, check: |t| t.contains("OK") },
    CaseSpec { id: "reg_cn_ok", prompt: "只回复两个字：好的", no_tools: true, check: |t| t.contains("好") },
    CaseSpec { id: "reg_bool_yes", prompt: "只回复：是", no_tools: true, check: |t| t.contains("是") },
    CaseSpec { id: "reg_bool_no", prompt: "只回复：否", no_tools: true, check: |t| t.contains("否") },
    CaseSpec { id: "reg_hex_ff", prompt: "只回复十六进制FF", no_tools: true, check: |t| t.to_uppercase().contains("FF") },
    CaseSpec { id: "reg_uuid_shape", prompt: "只回复一个UUID格式的示例（可虚构）", no_tools: true, check: |t| t.contains('-') },
    CaseSpec { id: "reg_email_shape", prompt: "只回复邮箱格式示例 a@b.co", no_tools: true, check: |t| t.contains('@') },
    CaseSpec { id: "reg_path_win", prompt: "只回复Windows路径示例 C:\\\\temp", no_tools: true, check: |t| t.contains("C:") || t.contains("c:") },
    CaseSpec { id: "reg_json_bool", prompt: "只输出{\"flag\":true}", no_tools: true, check: |t| t.contains("true") },
    CaseSpec { id: "reg_json_num", prompt: "只输出{\"n\":42}", no_tools: true, check: |t| t.contains("42") },
];
