//! Turnstile token VM — port of `gptimage-panda/utils/turnstile.py`.

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::time::{SystemTime, UNIX_EPOCH};

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use rand::Rng;
use serde_json::{json, Map, Value};

type Program = Vec<Value>;
type SlotMap = HashMap<String, Value>;

fn map_key(v: &Value) -> String {
    match v {
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Null => "null".into(),
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

fn handler_marker(op: i64) -> Value {
    Value::String(format!("__fn:{op}"))
}

fn as_handler_op(v: &Value) -> Option<i64> {
    v.as_str()
        .and_then(|s| s.strip_prefix("__fn:"))
        .and_then(|s| s.parse().ok())
}

#[derive(Default)]
struct OrderedMap {
    keys: Vec<String>,
    values: Map<String, Value>,
}

impl OrderedMap {
    fn add(&mut self, key: impl Into<String>, value: Value) {
        let key = key.into();
        if !self.values.contains_key(&key) {
            self.keys.push(key.clone());
        }
        self.values.insert(key, value);
    }

    fn to_dict(&self) -> Map<String, Value> {
        self.keys
            .iter()
            .filter_map(|k| self.values.get(k).map(|v| (k.clone(), v.clone())))
            .collect()
    }
}

fn turnstile_json_value(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            if let Some(Value::Object(rect)) = map.get("__rect") {
                let mut out = map.clone();
                out.remove("__rect");
                if let Some(Value::String(_)) = out.get("getBoundingClientRect") {
                    for (k, v) in rect {
                        out.insert(k.clone(), v.clone());
                    }
                }
                Value::Object(out)
            } else {
                Value::Object(map.clone())
            }
        }
        Value::Array(items) => Value::Array(items.iter().map(turnstile_json_value).collect()),
        other => other.clone(),
    }
}

fn turnstile_to_str(value: &Value) -> String {
    match value {
        Value::Null => "undefined".into(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => {
            let special = [
                ("window.Math", "[object Math]"),
                ("window.Reflect", "[object Reflect]"),
                ("window.performance", "[object Performance]"),
                ("window.localStorage", "[object Storage]"),
                ("window.Object", "function Object() { [native code] }"),
                ("window.Reflect.set", "function set() { [native code] }"),
                ("window.performance.now", "function () { [native code] }"),
                (
                    "window.Object.create",
                    "function create() { [native code] }",
                ),
                ("window.Object.keys", "function keys() { [native code] }"),
                ("window.Math.random", "function random() { [native code] }"),
            ];
            for (k, v) in special {
                if s == k {
                    return v.into();
                }
            }
            s.clone()
        }
        Value::Array(items) if items.iter().all(|i| i.is_string()) => items
            .iter()
            .filter_map(|i| i.as_str())
            .collect::<Vec<_>>()
            .join(","),
        other => other.to_string(),
    }
}

fn xor_string(text: &str, key: &str) -> String {
    if key.is_empty() {
        return text.to_string();
    }
    text.chars()
        .enumerate()
        .map(|(i, ch)| {
            let k = key.as_bytes()[i % key.len()] as char;
            char::from_u32((ch as u32) ^ (k as u32)).unwrap_or(ch)
        })
        .collect()
}

fn read_property(target: &Value, key: &Value) -> Value {
    let key_text = turnstile_to_str(key);
    match target {
        Value::Object(map) => map.get(&key_text).cloned().unwrap_or(Value::Null),
        Value::String(s) => {
            let value = format!("{s}.{key_text}");
            if value == "window.document.location" {
                Value::String("https://chatgpt.com/".into())
            } else {
                Value::String(value)
            }
        }
        _ => Value::Null,
    }
}

fn make_pseudo_element() -> Value {
    let mut rect = OrderedMap::default();
    for (k, v) in [
        ("x", 0.0),
        ("y", 1129.0),
        ("width", 28.300003051757812),
        ("height", 27.0),
        ("top", 1129.0),
        ("right", 28.300003051757812),
        ("bottom", 1156.0),
        ("left", 0.0),
    ] {
        rect.add(k, json!(v));
    }
    let mut element = OrderedMap::default();
    element.add("style", Value::Object(Map::new()));
    element.add(
        "getBoundingClientRect",
        Value::String("getBoundingClientRect".into()),
    );
    let mut map = element.to_dict();
    map.insert("__rect".into(), Value::Object(rect.to_dict()));
    Value::Object(map)
}

struct Vm<'a> {
    process_map: SlotMap,
    result: &'a mut String,
    start_time: f64,
    rng: &'a mut rand::rngs::ThreadRng,
    instruction_count: usize,
}

impl<'a> Vm<'a> {
    fn get_slot(&self, key: &Value) -> Value {
        self.process_map
            .get(&map_key(key))
            .cloned()
            .unwrap_or(Value::Null)
    }

    fn set_slot(&mut self, key: &Value, value: Value) {
        self.process_map.insert(map_key(key), value);
    }

    fn slot_defined_not_null(&self, key: &Value) -> bool {
        match self.process_map.get(&map_key(key)) {
            Some(Value::Null) | None => false,
            Some(_) => true,
        }
    }

    fn call_handler(&mut self, op: i64, args: &[Value]) {
        match op {
            1 if args.len() >= 2 => {
                let left = turnstile_to_str(&self.get_slot(&args[0]));
                let right = turnstile_to_str(&self.get_slot(&args[1]));
                self.set_slot(&args[0], Value::String(xor_string(&left, &right)));
            }
            2 if args.len() >= 2 => self.set_slot(&args[0], args[1].clone()),
            3 if !args.is_empty() => {
                let bytes = match &args[0] {
                    Value::String(s) => s.as_bytes().to_vec(),
                    other => turnstile_to_str(other).into_bytes(),
                };
                *self.result = B64.encode(bytes);
            }
            5 if args.len() >= 2 => {
                let current = self.get_slot(&args[0]);
                let incoming = self.get_slot(&args[1]);
                if let Value::Array(mut arr) = current {
                    arr.push(incoming);
                    self.set_slot(&args[0], Value::Array(arr));
                } else if current.is_string()
                    || current.is_number()
                    || incoming.is_string()
                    || incoming.is_number()
                {
                    let merged = format!(
                        "{}{}",
                        turnstile_to_str(&current),
                        turnstile_to_str(&incoming)
                    );
                    self.set_slot(&args[0], Value::String(merged));
                } else {
                    self.set_slot(&args[0], Value::String("NaN".into()));
                }
            }
            6 if args.len() >= 3 => {
                let target = self.get_slot(&args[1]);
                let key = self.get_slot(&args[2]);
                let val = read_property(&target, &key);
                self.set_slot(&args[0], val);
            }
            7 if !args.is_empty() => {
                let target = self.get_slot(&args[0]);
                let value_keys = &args[1..];
                let values: Vec<Value> = value_keys.iter().map(|a| self.get_slot(a)).collect();
                if target == json!("window.Reflect.set") && values.len() >= 3 {
                    if let Some(obj_key) = value_keys.first() {
                        if let Value::Object(mut map) = values[0].clone() {
                            let key = turnstile_to_str(&values[1]);
                            map.insert(key, values[2].clone());
                            self.set_slot(obj_key, Value::Object(map));
                        }
                    }
                } else if let Some(nested) = as_handler_op(&target) {
                    self.call_handler(nested, &values);
                }
            }
            8 if args.len() >= 2 => {
                let val = self.get_slot(&args[1]);
                self.set_slot(&args[0], val);
            }
            13 | 23 => {
                if args.len() >= 2 && self.slot_defined_not_null(&args[0]) {
                    if let Some(nested) = as_handler_op(&self.get_slot(&args[1])) {
                        self.call_handler(nested, &args[2..]);
                    }
                }
            }
            14 if args.len() >= 2 => {
                if let Value::String(s) = self.get_slot(&args[1]) {
                    if let Ok(v) = serde_json::from_str::<Value>(&s) {
                        self.set_slot(&args[0], v);
                    }
                }
            }
            15 if args.len() >= 2 => {
                let v = turnstile_json_value(&self.get_slot(&args[1]));
                let s = serde_json::to_string(&v).unwrap_or_default();
                self.set_slot(&args[0], Value::String(s));
            }
            17 if args.len() >= 2 => {
                let call_args: Vec<Value> = args[2..].iter().map(|a| self.get_slot(a)).collect();
                let target = self.get_slot(&args[1]);
                if target == json!("window.performance.now") {
                    let elapsed_ns = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map(|d| d.as_nanos() as f64)
                        .unwrap_or(0.0)
                        - self.start_time * 1e9;
                    let r = self.rng.random::<f64>();
                    let val = (elapsed_ns + r) / 1e6;
                    self.set_slot(&args[0], json!(val));
                } else if target == json!("window.Object.create") {
                    self.set_slot(&args[0], Value::Object(Map::new()));
                } else if target == json!("window.document.createElement") {
                    self.set_slot(&args[0], make_pseudo_element());
                } else if target == json!("window.navigator.storage.estimate") {
                    self.set_slot(
                        &args[0],
                        json!({"quota": 10_i64 * 1024 * 1024 * 1024, "usage": 16 * 1024}),
                    );
                } else if target == json!("window.Object.keys") {
                    if call_args.first() == Some(&json!("window.localStorage")) {
                        self.set_slot(
                            &args[0],
                            json!([
                                "STATSIG_LOCAL_STORAGE_INTERNAL_STORE_V4",
                                "STATSIG_LOCAL_STORAGE_STABLE_ID",
                                "client-correlated-secret",
                                "oai/apps/capExpiresAt",
                                "oai-did",
                                "STATSIG_LOCAL_STORAGE_LOGGING_REQUEST",
                                "UiState.isNavigationCollapsed.1"
                            ]),
                        );
                    } else if let Some(Value::Object(map)) = call_args.first() {
                        self.set_slot(
                            &args[0],
                            Value::Array(map.keys().cloned().map(Value::String).collect()),
                        );
                    }
                } else if target == json!("window.Math.random") {
                    let r = self.rng.random::<f64>();
                    self.set_slot(&args[0], json!(r));
                } else if let Some(nested) = as_handler_op(&target) {
                    self.call_handler(nested, &call_args);
                }
            }
            18 if !args.is_empty() => {
                if let Ok(bytes) = B64.decode(turnstile_to_str(&self.get_slot(&args[0]))) {
                    if let Ok(s) = String::from_utf8(bytes) {
                        self.set_slot(&args[0], Value::String(s));
                    }
                }
            }
            19 if !args.is_empty() => {
                self.set_slot(
                    &args[0],
                    Value::String(
                        B64.encode(turnstile_to_str(&self.get_slot(&args[0])).as_bytes()),
                    ),
                );
            }
            20 if args.len() >= 3 && self.get_slot(&args[0]) == self.get_slot(&args[1]) => {
                let target = self.get_slot(&args[2]);
                if target == json!("getBoundingClientRect") {
                    if let Value::Object(obj) = self.get_slot(&args[0]) {
                        if let Some(Value::Object(rect)) = obj.get("__rect") {
                            self.set_slot(&args[0], Value::Object(rect.clone()));
                        }
                    }
                } else if let Some(nested) = as_handler_op(&target) {
                    self.call_handler(nested, &args[3..]);
                }
            }
            21 => {}
            22 if args.len() >= 2 => {
                self.set_slot(&args[0], Value::Null);
                if let Value::Array(program) = &args[1] {
                    self.execute_program(program);
                }
            }
            24 if args.len() >= 3 => {
                let target = self.get_slot(&args[1]);
                let key = self.get_slot(&args[2]);
                let val = read_property(&target, &key);
                self.set_slot(&args[0], val);
            }
            34 if args.len() >= 2 => self.set_slot(&args[0], self.get_slot(&args[1])),
            _ => {}
        }
    }

    fn execute_program(&mut self, program: &Program) {
        for token in program {
            self.instruction_count += 1;
            if self.instruction_count > 10_000 {
                return;
            }
            let Value::Array(parts) = token else { continue };
            if parts.is_empty() {
                continue;
            }
            if let Some(op) = as_handler_op(&self.get_slot(&parts[0])) {
                self.call_handler(op, &parts[1..]);
            }
        }
    }
}

fn init_process_map(token_list: &Program, p: &str) -> SlotMap {
    let mut map = SlotMap::new();
    for op in [
        1, 2, 3, 5, 6, 7, 8, 13, 14, 15, 17, 18, 19, 20, 21, 22, 23, 24, 34,
    ] {
        map.insert(op.to_string(), handler_marker(op));
    }
    map.insert("9".into(), Value::Array(token_list.clone()));
    map.insert("10".into(), Value::String("window".into()));
    map.insert("16".into(), Value::String(p.to_string()));
    map
}

fn program_hash(program: &Program) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    serde_json::to_string(program)
        .unwrap_or_default()
        .hash(&mut hasher);
    hasher.finish()
}

/// Solve Turnstile dx blob with prepare token `p`. Returns None on failure.
pub fn solve_turnstile_token(dx: &str, p: &str) -> Option<String> {
    if dx.is_empty() || p.is_empty() || dx.len() > 512_000 {
        return None;
    }
    let decoded = B64.decode(dx).ok()?;
    let decoded_text = String::from_utf8(decoded).ok()?;
    let token_list: Value = serde_json::from_str(&xor_string(&decoded_text, p)).ok()?;
    let Value::Array(token_list) = token_list else {
        return None;
    };

    let start_time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);
    let mut result = String::new();
    let mut rng = rand::rng();
    let mut process_map = init_process_map(&token_list, p);

    let mut programs: Vec<Program> = vec![token_list];
    let mut seen: HashSet<u64> = HashSet::new();

    let mut instruction_count = 0usize;

    for _round in 0..8 {
        let program = programs.last()?;
        let ph = program_hash(program);
        if seen.contains(&ph) {
            break;
        }
        seen.insert(ph);

        {
            let mut vm = Vm {
                process_map: std::mem::take(&mut process_map),
                result: &mut result,
                start_time,
                rng: &mut rng,
                instruction_count: 0,
            };
            vm.instruction_count = instruction_count;
            vm.execute_program(program);
            instruction_count = vm.instruction_count;
            process_map = vm.process_map;
        }

        let next = process_map.get("9").cloned();
        let Value::Array(next_program) = next? else {
            break;
        };
        let next_hash = program_hash(&next_program);
        if seen.contains(&next_hash) {
            break;
        }
        programs.push(next_program);
    }

    if result.len() < 512 {
        return None;
    }
    Some(result)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn spa_fixture_yields_long_token() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/upstream/turnstile_dx_20260721.json");
        let fixture: Value = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
        let token = solve_turnstile_token(
            fixture["dx"].as_str().unwrap(),
            fixture["p"].as_str().unwrap(),
        )
        .expect("fixture should solve");
        assert!(token.len() > 1000);
        let decoded = B64.decode(token).unwrap();
        let decoded_text = String::from_utf8(decoded).unwrap();
        assert!(decoded_text.len() > 750);
    }

    #[test]
    fn invalid_payload_fails_closed() {
        assert!(solve_turnstile_token("not-base64", "fixture-p").is_none());
    }
}
