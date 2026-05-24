use std::env;
use std::ffi::{CStr, CString, OsString};
use std::io::Write;
use std::os::raw::c_char;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use serde_json::Value as JsonValue;

#[repr(C)]
pub struct LozJson {
    value: JsonValue,
}

#[unsafe(no_mangle)]
pub extern "C" fn loz_json_parse(text: *const c_char) -> *mut LozJson {
    let Some(text) = c_str_argument(text, "loz_json_parse") else {
        return std::ptr::null_mut();
    };

    match serde_json::from_str::<JsonValue>(text) {
        Ok(value) => Box::into_raw(Box::new(LozJson { value })),
        Err(error) => {
            eprintln!("runtime error: invalid JSON in loz_json_parse(): {error}");
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn loz_json_stringify(json: *mut LozJson) -> *const c_char {
    let Some(json) = json_ref(json, "loz_json_stringify") else {
        return owned_c_string("", "loz_json_stringify");
    };

    match serde_json::to_string(&json.value) {
        Ok(text) => owned_c_string(&text, "loz_json_stringify"),
        Err(error) => {
            eprintln!("runtime error: failed to stringify Json in loz_json_stringify(): {error}");
            owned_c_string("", "loz_json_stringify")
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn loz_json_has(json: *mut LozJson, key: *const c_char) -> bool {
    let Some(json) = json_ref(json, "loz_json_has") else {
        return false;
    };
    let Some(key) = c_str_argument(key, "loz_json_has") else {
        return false;
    };
    let Some(object) = json.value.as_object() else {
        eprintln!("runtime error: Json value is not an object in loz_json_has()");
        return false;
    };

    object.contains_key(key)
}

#[unsafe(no_mangle)]
pub extern "C" fn loz_json_get_text(json: *mut LozJson, key: *const c_char) -> *const c_char {
    let Some(value) = object_field(json, key, "loz_json_get_text") else {
        return owned_c_string("", "loz_json_get_text");
    };

    match value.as_str() {
        Some(text) => owned_c_string(text, "loz_json_get_text"),
        None => {
            eprintln!("runtime error: key has wrong type in loz_json_get_text()");
            owned_c_string("", "loz_json_get_text")
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn loz_json_get_i32(json: *mut LozJson, key: *const c_char) -> i32 {
    let Some(value) = object_field(json, key, "loz_json_get_i32") else {
        return 0;
    };

    match value.as_i64().and_then(|value| i32::try_from(value).ok()) {
        Some(value) => value,
        None => {
            eprintln!("runtime error: key has wrong type in loz_json_get_i32()");
            0
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn loz_json_get_i64(json: *mut LozJson, key: *const c_char) -> i64 {
    let Some(value) = object_field(json, key, "loz_json_get_i64") else {
        return 0;
    };

    match value.as_i64() {
        Some(value) => value,
        None => {
            eprintln!("runtime error: key has wrong type in loz_json_get_i64()");
            0
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn loz_json_get_f64(json: *mut LozJson, key: *const c_char) -> f64 {
    let Some(value) = object_field(json, key, "loz_json_get_f64") else {
        return 0.0;
    };

    match value.as_f64() {
        Some(value) => value,
        None => {
            eprintln!("runtime error: key has wrong type in loz_json_get_f64()");
            0.0
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn loz_json_get_bool(json: *mut LozJson, key: *const c_char) -> bool {
    let Some(value) = object_field(json, key, "loz_json_get_bool") else {
        return false;
    };

    match value.as_bool() {
        Some(value) => value,
        None => {
            eprintln!("runtime error: key has wrong type in loz_json_get_bool()");
            false
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn loz_json_free(json: *mut LozJson) {
    if json.is_null() {
        return;
    }

    unsafe {
        drop(Box::from_raw(json));
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn loz_schema_validate(
    schema_descriptor: *const c_char,
    json: *mut LozJson,
) -> bool {
    let Some(schema) = parse_schema_descriptor(schema_descriptor, "loz_schema_validate") else {
        return false;
    };
    let Some(json) = json_ref(json, "loz_schema_validate") else {
        return false;
    };

    validate_json_against_schema(&schema, &json.value).is_ok()
}

#[unsafe(no_mangle)]
pub extern "C" fn loz_schema_require(
    schema_descriptor: *const c_char,
    json: *mut LozJson,
) -> *mut LozJson {
    let Some(schema) = parse_schema_descriptor(schema_descriptor, "loz_schema_require") else {
        return std::ptr::null_mut();
    };
    let Some(json) = json_ref(json, "loz_schema_require") else {
        return std::ptr::null_mut();
    };

    match validate_json_against_schema(&schema, &json.value) {
        Ok(()) => json as *const LozJson as *mut LozJson,
        Err(error) => {
            eprintln!(
                "runtime error: schema validation failed in loz_schema_require() for '{}': {}",
                schema.name, error
            );
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn loz_python_call(path: *const c_char, input_json: *mut LozJson) -> *mut LozJson {
    let Some(function_path) = c_str_argument(path, "loz_python_call") else {
        return std::ptr::null_mut();
    };
    let Some(input_json) = json_ref(input_json, "loz_python_call") else {
        return std::ptr::null_mut();
    };

    match run_python_bridge(function_path, &input_json.value) {
        Ok(value) => Box::into_raw(Box::new(LozJson { value })),
        Err(error) => {
            eprintln!("runtime error: {error}");
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn loz_llm_ask(prompt: *const c_char) -> *const c_char {
    let Some(prompt) = c_str_argument(prompt, "loz_llm_ask") else {
        return owned_c_string("", "loz_llm_ask");
    };

    match run_llm_request(prompt) {
        Ok(response) => owned_c_string(&response, "loz_llm_ask"),
        Err(error) => {
            eprintln!("{error}");
            owned_c_string("", "loz_llm_ask")
        }
    }
}

fn c_str_argument<'a>(pointer: *const c_char, context: &str) -> Option<&'a str> {
    if pointer.is_null() {
        eprintln!("runtime error: null Text argument in {context}()");
        return None;
    }

    let c_str = unsafe { CStr::from_ptr(pointer) };
    match c_str.to_str() {
        Ok(value) => Some(value),
        Err(error) => {
            eprintln!("runtime error: invalid UTF-8 Text argument in {context}(): {error}");
            None
        }
    }
}

fn json_ref<'a>(json: *mut LozJson, context: &str) -> Option<&'a LozJson> {
    match unsafe { json.as_ref() } {
        Some(value) => Some(value),
        None => {
            eprintln!("runtime error: null Json value in {context}()");
            None
        }
    }
}

fn object_field<'a>(
    json: *mut LozJson,
    key: *const c_char,
    context: &str,
) -> Option<&'a JsonValue> {
    let json = json_ref(json, context)?;
    let key = c_str_argument(key, context)?;
    let Some(object) = json.value.as_object() else {
        eprintln!("runtime error: Json value is not an object in {context}()");
        return None;
    };

    match object.get(key) {
        Some(value) => Some(value),
        None => {
            eprintln!("runtime error: missing key '{key}' in {context}()");
            None
        }
    }
}

fn owned_c_string(text: &str, context: &str) -> *const c_char {
    match CString::new(text) {
        Ok(value) => value.into_raw(),
        Err(_) => {
            eprintln!("runtime error: {context}() cannot return Text containing null bytes");
            CString::new("").unwrap().into_raw()
        }
    }
}

struct ParsedSchema {
    name: String,
    fields: Vec<ParsedSchemaField>,
}

struct ParsedSchemaField {
    name: String,
    type_name: String,
}

fn parse_schema_descriptor(pointer: *const c_char, context: &str) -> Option<ParsedSchema> {
    let descriptor = c_str_argument(pointer, context)?;
    let (name, fields_text) = descriptor.split_once('|').unwrap_or((descriptor, ""));
    let mut fields = Vec::new();

    if !fields_text.is_empty() {
        for field in fields_text.split(';') {
            if field.is_empty() {
                continue;
            }

            let Some((field_name, type_name)) = field.split_once(':') else {
                eprintln!(
                    "runtime error: invalid schema descriptor field '{}' in {}()",
                    field, context
                );
                return None;
            };

            fields.push(ParsedSchemaField {
                name: field_name.to_string(),
                type_name: type_name.to_string(),
            });
        }
    }

    Some(ParsedSchema {
        name: name.to_string(),
        fields,
    })
}

fn validate_json_against_schema(schema: &ParsedSchema, value: &JsonValue) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| format!("Json value is not an object for schema '{}'", schema.name))?;

    for field in &schema.fields {
        let Some(field_value) = object.get(&field.name) else {
            return Err(format!(
                "missing key '{}' for schema '{}'",
                field.name, schema.name
            ));
        };

        let matches_type = match field.type_name.as_str() {
            "Text" => field_value.is_string(),
            "Bool" => field_value.is_boolean(),
            "i32" => field_value
                .as_i64()
                .and_then(|value| i32::try_from(value).ok())
                .is_some(),
            "i64" => field_value.as_i64().is_some(),
            "f64" => field_value.as_f64().is_some(),
            "Json" => true,
            other => {
                return Err(format!(
                    "unsupported schema field type '{}' in schema '{}'",
                    other, schema.name
                ));
            }
        };

        if !matches_type {
            return Err(format!(
                "key '{}' has wrong type for schema '{}'",
                field.name, schema.name
            ));
        }
    }

    Ok(())
}

fn run_python_bridge(function_path: &str, input_json: &JsonValue) -> Result<JsonValue, String> {
    let python_executable = python_executable();
    let bridge_script = python_bridge_script_path()?;
    let input_text = serde_json::to_string(input_json).map_err(|error| {
        format!("failed to serialize Json payload for loz_python_call(): {error}")
    })?;

    let mut child = Command::new(&python_executable)
        .arg(&bridge_script)
        .arg(function_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| {
            format!(
                "failed to launch Python executable '{}' for loz_python_call(): {error}",
                python_executable.to_string_lossy()
            )
        })?;

    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "failed to open stdin for Python bridge in loz_python_call()".to_string())?;
    stdin
        .write_all(input_text.as_bytes())
        .map_err(|error| format!("failed to write Json payload to Python bridge stdin: {error}"))?;
    drop(stdin);

    let output = child.wait_with_output().map_err(|error| {
        format!("failed to wait for Python bridge in loz_python_call(): {error}")
    })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let detail = if stderr.is_empty() {
            format!("Python bridge exited with status {}", output.status)
        } else {
            stderr
        };
        return Err(format!(
            "python.call() failed for '{function_path}': {detail}"
        ));
    }

    serde_json::from_slice(&output.stdout).map_err(|error| {
        format!("python.call() returned invalid JSON for '{function_path}': {error}")
    })
}

fn run_llm_request(prompt: &str) -> Result<String, String> {
    let provider = env::var("LOZ_LLM_PROVIDER").unwrap_or_else(|_| "mock".to_string());

    match provider.as_str() {
        "mock" => Ok(mock_llm_response(prompt)),
        "ollama" => run_ollama_request(prompt),
        "github" => run_github_models_request(prompt),
        other => Err(format!("runtime error: unknown LLM provider '{}'", other)),
    }
}

fn mock_llm_response(prompt: &str) -> String {
    env::var("LOZ_LLM_MOCK_RESPONSE").unwrap_or_else(|_| format!("[mock] {prompt}"))
}

fn run_ollama_request(prompt: &str) -> Result<String, String> {
    let base_url =
        env::var("LOZ_OLLAMA_BASE_URL").unwrap_or_else(|_| "http://localhost:11434".to_string());
    let model = env::var("LOZ_MODEL").unwrap_or_else(|_| "qwen2.5:0.5b".to_string());
    let endpoint = format!("{}/api/generate", base_url.trim_end_matches('/'));
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|error| format!("runtime error: failed to create HTTP client: {error}"))?;

    let response = client
        .post(&endpoint)
        .json(&serde_json::json!({
            "model": model,
            "prompt": prompt,
            "stream": false
        }))
        .send()
        .map_err(|error| format!("runtime error: failed to call Ollama at {base_url}: {error}"))?;

    let status = response.status();
    let body_text = response
        .text()
        .map_err(|error| format!("runtime error: failed to call Ollama at {base_url}: {error}"))?;
    let body: JsonValue = serde_json::from_str(&body_text)
        .map_err(|_| "runtime error: invalid LLM provider response".to_string())?;

    if !status.is_success() {
        let detail = body
            .get("error")
            .and_then(JsonValue::as_str)
            .unwrap_or("request failed");
        return Err(format!(
            "runtime error: failed to call Ollama at {base_url}: HTTP {status} {detail}"
        ));
    }

    body.get("response")
        .and_then(JsonValue::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| "runtime error: invalid LLM provider response".to_string())
}

fn run_github_models_request(prompt: &str) -> Result<String, String> {
    let token_env_name = github_token_env_name();
    let token = env::var(&token_env_name).map_err(|_| {
        format!(
            "runtime error: {} is required for LOZ_LLM_PROVIDER=github",
            token_env_name
        )
    })?;
    let model = env::var("LOZ_MODEL").map_err(|_| {
        "runtime error: LOZ_MODEL is required for LOZ_LLM_PROVIDER=github".to_string()
    })?;
    let base_url = env::var("LOZ_GITHUB_MODELS_BASE_URL")
        .unwrap_or_else(|_| "https://models.github.ai/inference".to_string());
    let endpoint = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|error| format!("runtime error: failed to create HTTP client: {error}"))?;

    let response = client
        .post(&endpoint)
        .bearer_auth(token)
        .json(&serde_json::json!({
            "model": model,
            "messages": [
                {
                    "role": "user",
                    "content": prompt
                }
            ]
        }))
        .send()
        .map_err(|error| {
            format!("runtime error: failed to call GitHub Models at {base_url}: {error}")
        })?;

    let status = response.status();
    let body_text = response.text().map_err(|error| {
        format!("runtime error: failed to call GitHub Models at {base_url}: {error}")
    })?;
    let body: JsonValue = serde_json::from_str(&body_text)
        .map_err(|_| "runtime error: invalid LLM provider response".to_string())?;

    if !status.is_success() {
        let detail = body
            .get("error")
            .and_then(|value| value.get("message"))
            .and_then(JsonValue::as_str)
            .or_else(|| body.get("message").and_then(JsonValue::as_str))
            .unwrap_or("request failed");
        return Err(format!(
            "runtime error: failed to call GitHub Models at {base_url}: HTTP {status} {detail}"
        ));
    }

    body.get("choices")
        .and_then(JsonValue::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("content"))
        .and_then(JsonValue::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| "runtime error: invalid LLM provider response".to_string())
}

fn github_token_env_name() -> String {
    env::var("LOZ_GITHUB_TOKEN_ENV").unwrap_or_else(|_| "GITHUB_TOKEN".to_string())
}

fn python_executable() -> OsString {
    env::var_os("LOZ_PYTHON_PATH")
        .filter(|value| !value.is_empty())
        .or_else(|| find_command_on_path("python3"))
        .or_else(|| find_command_on_path("python"))
        .unwrap_or_else(|| OsString::from("python3"))
}

fn python_bridge_script_path() -> Result<PathBuf, String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../runtime/python_bridge.py");
    if path.is_file() {
        Ok(path)
    } else {
        Err(format!(
            "Python bridge script is missing at '{}'",
            path.display()
        ))
    }
}

fn find_command_on_path(command: &str) -> Option<OsString> {
    let path_value = env::var_os("PATH")?;
    let windows_exts = windows_path_extensions();
    let has_extension = Path::new(command).extension().is_some();

    for directory in env::split_paths(&path_value) {
        let direct = directory.join(command);
        if direct.is_file() {
            return Some(direct.into_os_string());
        }

        if cfg!(target_os = "windows") && !has_extension {
            for extension in &windows_exts {
                let candidate = directory.join(format!("{command}{extension}"));
                if candidate.is_file() {
                    return Some(candidate.into_os_string());
                }
            }
        }
    }

    None
}

fn windows_path_extensions() -> Vec<String> {
    env::var("PATHEXT")
        .ok()
        .map(|value| {
            value
                .split(';')
                .filter(|part| !part.is_empty())
                .map(|part| part.to_string())
                .collect::<Vec<_>>()
        })
        .filter(|extensions| !extensions.is_empty())
        .unwrap_or_else(|| {
            vec![
                ".exe".to_string(),
                ".cmd".to_string(),
                ".bat".to_string(),
                ".com".to_string(),
            ]
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Mutex, OnceLock};
    use std::time::{SystemTime, UNIX_EPOCH};

    static TEST_MODULE_COUNTER: AtomicU64 = AtomicU64::new(0);
    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    fn c_string(text: &str) -> CString {
        CString::new(text).unwrap()
    }

    fn take_text(pointer: *const c_char) -> String {
        let owned = unsafe { CString::from_raw(pointer as *mut c_char) };
        owned.into_string().unwrap()
    }

    fn python_is_available() -> bool {
        Command::new(python_executable())
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }

    fn write_test_module(module_source: &str) -> (String, PathBuf) {
        let current_dir = env::current_dir().unwrap();
        let unique = format!(
            "{}_{}_{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            TEST_MODULE_COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let module_name = format!("loz_runtime_python_test_{unique}");
        let module_path = current_dir.join(format!("{module_name}.py"));
        fs::write(&module_path, module_source).unwrap();
        (module_name, module_path)
    }

    fn with_env_vars<T>(updates: &[(&str, Option<&str>)], test: impl FnOnce() -> T) -> T {
        let _guard = ENV_LOCK.get_or_init(|| Mutex::new(())).lock().unwrap();
        let previous = updates
            .iter()
            .map(|(key, _)| ((*key).to_string(), env::var(key).ok()))
            .collect::<Vec<_>>();

        for (key, value) in updates {
            unsafe {
                match value {
                    Some(value) => env::set_var(key, value),
                    None => env::remove_var(key),
                }
            }
        }

        let result = test();

        for (key, value) in previous {
            unsafe {
                match value {
                    Some(value) => env::set_var(&key, value),
                    None => env::remove_var(&key),
                }
            }
        }

        result
    }

    #[test]
    fn parses_and_reads_json_values() {
        let source = c_string(r#"{"id":1,"name":"Ahmed","active":true,"score":98.5}"#);
        let user = loz_json_parse(source.as_ptr());
        assert!(!user.is_null());

        let id_key = c_string("id");
        let name_key = c_string("name");
        let active_key = c_string("active");
        let score_key = c_string("score");
        let missing_key = c_string("email");

        assert_eq!(loz_json_get_i32(user, id_key.as_ptr()), 1);
        assert_eq!(loz_json_get_i64(user, id_key.as_ptr()), 1);
        assert_eq!(
            take_text(loz_json_get_text(user, name_key.as_ptr())),
            "Ahmed"
        );
        assert!(loz_json_get_bool(user, active_key.as_ptr()));
        assert_eq!(loz_json_get_f64(user, score_key.as_ptr()), 98.5);
        assert!(!loz_json_has(user, missing_key.as_ptr()));

        loz_json_free(user);
    }

    #[test]
    fn stringifies_json_values() {
        let source = c_string(r#"{"id":1,"name":"Ahmed"}"#);
        let user = loz_json_parse(source.as_ptr());
        assert!(!user.is_null());

        let text = take_text(loz_json_stringify(user));
        assert_eq!(text, r#"{"id":1,"name":"Ahmed"}"#);

        loz_json_free(user);
    }

    #[test]
    fn returns_defaults_for_missing_keys() {
        let source = c_string(r#"{"id":1}"#);
        let user = loz_json_parse(source.as_ptr());
        assert!(!user.is_null());

        let missing = c_string("missing");
        assert_eq!(loz_json_get_i32(user, missing.as_ptr()), 0);
        assert_eq!(take_text(loz_json_get_text(user, missing.as_ptr())), "");

        loz_json_free(user);
    }

    #[test]
    fn validates_schema_descriptors() {
        let descriptor = c_string("User|id:i32;name:Text;active:Bool");
        let source = c_string(r#"{"id":1,"name":"Ahmed","active":true}"#);
        let user = loz_json_parse(source.as_ptr());

        assert!(loz_schema_validate(descriptor.as_ptr(), user));
        assert_eq!(loz_schema_require(descriptor.as_ptr(), user), user);

        loz_json_free(user);
    }

    #[test]
    fn rejects_invalid_schema_matches() {
        let descriptor = c_string("User|id:i32;name:Text");
        let source = c_string(r#"{"id":"wrong","name":"Ahmed"}"#);
        let user = loz_json_parse(source.as_ptr());

        assert!(!loz_schema_validate(descriptor.as_ptr(), user));
        assert!(loz_schema_require(descriptor.as_ptr(), user).is_null());

        loz_json_free(user);
    }

    #[test]
    fn calls_python_bridge_and_returns_json() {
        if !python_is_available() {
            return;
        }

        let (module_name, module_path) = write_test_module(
            r#"def analyze_text(payload):
    text = payload["text"]
    return {"length": len(text), "label": "ok"}
"#,
        );

        let input = c_string(r#"{"text":"hello"}"#);
        let path = c_string(&format!("{module_name}.analyze_text"));
        let result_input = loz_json_parse(input.as_ptr());
        assert!(!result_input.is_null());

        let result = loz_python_call(path.as_ptr(), result_input);
        assert!(!result.is_null());

        let length_key = c_string("length");
        let label_key = c_string("label");
        assert_eq!(loz_json_get_i32(result, length_key.as_ptr()), 5);
        assert_eq!(
            take_text(loz_json_get_text(result, label_key.as_ptr())),
            "ok"
        );

        loz_json_free(result_input);
        loz_json_free(result);
        let _ = fs::remove_file(module_path);
    }

    #[test]
    fn returns_null_for_python_bridge_errors() {
        if !python_is_available() {
            return;
        }

        let (module_name, module_path) = write_test_module(
            r#"def fail(payload):
    raise RuntimeError("boom")
"#,
        );

        let input = c_string(r#"{"text":"hello"}"#);
        let path = c_string(&format!("{module_name}.fail"));
        let result_input = loz_json_parse(input.as_ptr());
        assert!(!result_input.is_null());

        let result = loz_python_call(path.as_ptr(), result_input);
        assert!(result.is_null());

        loz_json_free(result_input);
        let _ = fs::remove_file(module_path);
    }

    #[test]
    fn returns_mock_llm_response() {
        with_env_vars(
            &[
                ("LOZ_LLM_PROVIDER", Some("mock")),
                ("LOZ_LLM_MOCK_RESPONSE", None),
                ("LOZ_MODEL", None),
                ("GITHUB_TOKEN", None),
            ],
            || {
                let prompt = c_string("hello");
                let response = loz_llm_ask(prompt.as_ptr());

                assert_eq!(take_text(response), "[mock] hello");
            },
        );
    }

    #[test]
    fn returns_empty_text_for_unknown_llm_provider() {
        with_env_vars(
            &[
                ("LOZ_LLM_PROVIDER", Some("bad-provider")),
                ("LOZ_LLM_MOCK_RESPONSE", None),
                ("LOZ_MODEL", None),
                ("GITHUB_TOKEN", None),
            ],
            || {
                let prompt = c_string("hello");
                let response = loz_llm_ask(prompt.as_ptr());

                assert_eq!(take_text(response), "");
            },
        );
    }

    #[test]
    fn returns_empty_text_when_github_token_is_missing() {
        with_env_vars(
            &[
                ("LOZ_LLM_PROVIDER", Some("github")),
                ("LOZ_MODEL", Some("openai/gpt-4.1-mini")),
                ("GITHUB_TOKEN", None),
                ("LOZ_LLM_MOCK_RESPONSE", None),
            ],
            || {
                let prompt = c_string("hello");
                let response = loz_llm_ask(prompt.as_ptr());

                assert_eq!(take_text(response), "");
            },
        );
    }
}
