//! Answer-model provider catalog, legacy settings migration, and model-list requests.
//!
//! Chat completion stays on the OpenAI-compatible path used by tool calling.
//! Listing and connection tests follow each provider's own API.

use crate::types::ModelConfig;
use anyhow::{anyhow, bail, Context, Result};
use reqwest::Client;
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Debug, Clone, Deserialize)]
pub struct ProviderSpec {
    pub id: String,
    pub label: String,
    pub base_url: String,
    pub api: String,
    pub hosts: Vec<String>,
    #[serde(default)]
    pub match_port: Option<u16>,
    pub presets: Vec<String>,
}

#[derive(Deserialize)]
struct Catalog {
    providers: Vec<ProviderSpec>,
}

pub fn providers() -> &'static [ProviderSpec] {
    static CATALOG: std::sync::OnceLock<Vec<ProviderSpec>> = std::sync::OnceLock::new();
    CATALOG.get_or_init(|| {
        let catalog: Catalog = serde_json::from_str(include_str!("../../src/lib/llm-providers.json"))
            .expect("bundled provider catalog must be valid");
        catalog.providers
    })
}

pub fn provider(id: &str) -> Option<&'static ProviderSpec> {
    providers().iter().find(|provider| provider.id == id)
}

pub fn presets_for(id: &str) -> &'static [String] {
    provider(id).map(|provider| provider.presets.as_slice()).unwrap_or(&[])
}

pub fn infer_provider(kind: &str, url: &str) -> String {
    if kind == "jev" {
        return infer_jev(url);
    }
    let Ok(parsed) = reqwest::Url::parse(url.trim()) else {
        return "custom".into();
    };
    let host = parsed.host_str().unwrap_or("");
    for provider in providers() {
        if provider.id == "custom" || !provider.hosts.iter().any(|item| item == host) {
            continue;
        }
        if provider.match_port.is_some() && parsed.port() != provider.match_port {
            continue;
        }
        return provider.id.clone();
    }
    "custom".into()
}

fn infer_jev(url: &str) -> String {
    let Ok(parsed) = reqwest::Url::parse(url.trim()) else {
        return "custom".into();
    };
    match parsed.host_str().unwrap_or("") {
        "api.zleap.com" => "zleap".into(),
        "api.302.ai" | "api.302ai.cn" => "302".into(),
        "openrouter.ai" => "openrouter".into(),
        "api.typesafe.ai" => "official".into(),
        _ => "custom".into(),
    }
}

pub fn normalize_provider(kind: &str, config: &mut ModelConfig) {
    if config.provider.trim().is_empty() {
        config.provider = infer_provider(kind, &config.url);
    }
}

pub fn request_style(config: &ModelConfig) -> String {
    let id = if config.provider.trim().is_empty() {
        infer_provider("answer", &config.url)
    } else {
        config.provider.clone()
    };
    provider(&id).map(|item| item.api.clone()).unwrap_or_else(|| "openai".into())
}

pub fn allows_missing_key(config: &ModelConfig) -> bool {
    request_style(config) == "ollama"
}

pub fn uses_native_api(config: &ModelConfig) -> bool {
    // Anthropic 的连接测试走问答实际使用的 Chat Completions，不走 Messages API。
    matches!(request_style(config).as_str(), "gemini" | "ollama")
}

pub fn redact_secret(message: &str, key: &str) -> String {
    let key = key.trim();
    if key.is_empty() {
        message.to_owned()
    } else {
        message.replace(key, "[已隐藏]")
    }
}

pub fn provider_failure_line(status: u16, detail: &str, key: &str) -> String {
    format!("模型请求失败（HTTP {status}）：{}", redact_secret(detail, key))
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct MigratedSecrets {
    pub answer: Option<String>,
    pub jev: Option<String>,
    pub changed: bool,
}

pub fn migrate_settings_value(root: &mut Value) -> MigratedSecrets {
    let mut secrets = MigratedSecrets::default();
    let Some(object) = root.as_object_mut() else {
        return secrets;
    };
    if let Some(answer) = object.get_mut("answer") {
        secrets.answer = migrate_model("answer", answer, &mut secrets.changed);
    }
    if let Some(jev) = object.get_mut("jev") {
        secrets.jev = migrate_model("jev", jev, &mut secrets.changed);
    }
    secrets
}

fn migrate_model(kind: &str, value: &mut Value, changed: &mut bool) -> Option<String> {
    let Some(object) = value.as_object_mut() else {
        return None;
    };
    let key = take_secret(object);
    if key.is_some() {
        *changed = true;
        object.insert("has_key".into(), Value::Bool(true));
    }
    if move_alias(object, "url", &["base_url", "baseUrl", "api_base", "apiBase", "endpoint"]) {
        *changed = true;
    }
    if move_alias(object, "model", &["model_name", "modelName", "model_id", "modelId"]) {
        *changed = true;
    }
    if let Some(models) = object.get("models").cloned() {
        if !models.is_array() {
            object.insert("models".into(), Value::Array(string_list(&models)));
            *changed = true;
        }
    } else if let Some(alias) = take_first(object, &["available_models", "availableModels"]) {
        object.insert("models".into(), Value::Array(string_list(&alias)));
        *changed = true;
    }
    if let Some(vendor) = object.remove("vendor").filter(|value| !value.as_str().unwrap_or("").trim().is_empty()) {
        if object.get("provider").and_then(Value::as_str).unwrap_or("").trim().is_empty() {
            object.insert("provider".into(), vendor);
        }
        *changed = true;
    }
    let provider = object.get("provider").and_then(Value::as_str).unwrap_or("").trim().to_owned();
    if provider.is_empty() {
        let url = object.get("url").and_then(Value::as_str).unwrap_or("");
        object.insert("provider".into(), Value::String(infer_provider(kind, url)));
        *changed = true;
    }
    let provider = object.get("provider").and_then(Value::as_str).unwrap_or("").trim().to_owned();
    let url = object.get("url").and_then(Value::as_str).unwrap_or("").to_owned();
    if let Some(next) = normalize_provider_url(&provider, &url) {
        object.insert("url".into(), Value::String(next));
        *changed = true;
    }
    key.filter(|value| !value.is_empty())
}

fn normalize_provider_url(provider: &str, url: &str) -> Option<String> {
    if provider != "anthropic" && !provider.is_empty() {
        return None;
    }
    let Ok(mut parsed) = reqwest::Url::parse(url.trim()) else { return None };
    if parsed.host_str() != Some("api.anthropic.com") { return None; }
    let path = parsed.path().trim_matches('/');
    if !path.is_empty() { return None; }
    parsed.set_path("/v1");
    parsed.set_query(None);
    parsed.set_fragment(None);
    let next = parsed.to_string();
    (next != url.trim()).then_some(next)
}

fn move_alias(object: &mut serde_json::Map<String, Value>, canonical: &str, aliases: &[&str]) -> bool {
    let alias = take_first(object, aliases);
    let missing = object.get(canonical).and_then(Value::as_str).unwrap_or("").trim().is_empty()
        && !object.get(canonical).is_some_and(|value| value.is_array() || value.is_number() || value.is_boolean());
    if missing {
        if object.get(canonical).is_some() {
            object.remove(canonical);
        }
        if let Some(alias) = alias {
            object.insert(canonical.into(), alias);
            return true;
        }
        return false;
    }
    alias.is_some()
}

fn take_secret(object: &mut serde_json::Map<String, Value>) -> Option<String> {
    let value = take_first(object, &["api_key", "apiKey", "api_token", "access_token", "secret", "key"])?;
    let text = match value {
        Value::String(text) => text,
        other => other.to_string(),
    };
    let text = text.trim().to_owned();
    (!text.is_empty()).then_some(text)
}

fn take_first(object: &mut serde_json::Map<String, Value>, names: &[&str]) -> Option<Value> {
    names.iter().find_map(|name| object.remove(*name))
}

fn string_list(value: &Value) -> Vec<Value> {
    let items: Vec<String> = match value {
        Value::Array(items) => items.iter().filter_map(|item| item.as_str().map(str::trim).filter(|item| !item.is_empty()).map(str::to_owned)).collect(),
        Value::String(text) => text.split([',', '\n']).map(str::trim).filter(|item| !item.is_empty()).map(str::to_owned).collect(),
        _ => Vec::new(),
    };
    let mut unique = Vec::new();
    for item in items {
        if !unique.contains(&item) {
            unique.push(item);
        }
    }
    unique.into_iter().map(Value::String).collect()
}

pub fn list_url(config: &ModelConfig) -> Result<String> {
    match request_style(config).as_str() {
        "anthropic" => origin_with_path(&config.url, "/v1/models"),
        "gemini" => origin_with_path(&config.url, "/v1beta/models"),
        "ollama" => origin_with_path(&config.url, "/api/tags"),
        _ => join_path(&config.url, "models"),
    }
}

pub fn native_test_url(config: &ModelConfig) -> Result<String> {
    let model = config.model.trim();
    if model.is_empty() {
        bail!("请先选择或填写模型名称");
    }
    match request_style(config).as_str() {
        "anthropic" => origin_with_path(&config.url, "/v1/messages"),
        "gemini" => origin_with_path(&config.url, &format!("/v1beta/models/{}:generateContent", encode_segment(model))),
        "ollama" => origin_with_path(&config.url, "/api/chat"),
        _ => bail!("该供应商使用 OpenAI 兼容接口"),
    }
}

fn origin_with_path(raw: &str, path: &str) -> Result<String> {
    let mut url = reqwest::Url::parse(raw.trim()).context("接口地址无效")?;
    url.set_path(path);
    url.set_query(None);
    url.set_fragment(None);
    Ok(url.to_string())
}

fn join_path(raw: &str, suffix: &str) -> Result<String> {
    let mut url = reqwest::Url::parse(raw.trim()).context("接口地址无效")?;
    let path = url.path().trim_end_matches('/');
    if path.ends_with(&format!("/{suffix}")) {
        return Ok(url.to_string());
    }
    url.set_path(&format!("{path}/{suffix}"));
    Ok(url.to_string())
}

fn encode_segment(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => encoded.push(byte as char),
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

pub async fn list_models(client: &Client, config: &ModelConfig, key: &str, timeout: u64) -> Result<Vec<String>> {
    let style = request_style(config);
    if style != "ollama" && key.trim().is_empty() {
        bail!("请先填写 API 密钥");
    }
    let url = list_url(config)?;
    let data = send_json(client, "GET", &url, key, &style, None, timeout).await?;
    let models = parse_model_ids(&style, &data);
    if models.is_empty() {
        bail!("供应商没有返回可用模型");
    }
    Ok(models)
}

pub async fn test_native(client: &Client, config: &ModelConfig, key: &str, timeout: u64) -> Result<String> {
    let style = request_style(config);
    let model = config.model.trim();
    if model.is_empty() {
        bail!("请先选择或填写模型名称");
    }
    let url = native_test_url(config)?;
    let body = match style.as_str() {
        "anthropic" => json!({"model": model, "max_tokens": 8, "messages": [{"role": "user", "content": "Reply with OK."}]}),
        "gemini" => json!({"contents": [{"parts": [{"text": "Reply with OK."}]}]}),
        "ollama" => json!({"model": model, "messages": [{"role": "user", "content": "Reply with OK."}], "stream": false}),
        _ => bail!("该供应商使用 OpenAI 兼容接口"),
    };
    let data = send_json(client, "POST", &url, key, &style, Some(body), timeout).await?;
    let ok = match style.as_str() {
        "anthropic" => data.get("content").and_then(Value::as_array).is_some_and(|items| !items.is_empty()) || data.get("type").and_then(Value::as_str) == Some("message"),
        "gemini" => data.get("candidates").and_then(Value::as_array).is_some_and(|items| !items.is_empty()),
        "ollama" => data.get("message").and_then(|message| message.get("content")).and_then(Value::as_str).is_some(),
        _ => false,
    };
    if !ok {
        bail!("接口连通，但未返回回答");
    }
    Ok("连接成功".into())
}

async fn send_json(
    client: &Client,
    method: &str,
    url: &str,
    key: &str,
    style: &str,
    body: Option<Value>,
    timeout: u64,
) -> Result<Value> {
    let mut request = match method {
        "GET" => client.get(url),
        _ => client.post(url),
    };
    request = request.timeout(std::time::Duration::from_secs(timeout.max(1)));
    if let Some(body) = body {
        request = request.json(&body);
    }
    request = apply_auth(request, style, key.trim())?;
    let response = request.send().await.map_err(|error| anyhow!(redact_secret(&error.to_string(), key)))?;
    let status = response.status();
    let text = response.text().await.map_err(|error| anyhow!(redact_secret(&error.to_string(), key)))?;
    if !status.is_success() {
        let line = provider_failure_line(status.as_u16(), &text.chars().take(500).collect::<String>(), key);
        eprintln!("{line}");
        bail!(line);
    }
    serde_json::from_str(&text).map_err(|error| anyhow!(redact_secret(&error.to_string(), key)))
}

fn apply_auth(request: reqwest::RequestBuilder, style: &str, key: &str) -> Result<reqwest::RequestBuilder> {
    if key.is_empty() {
        return Ok(request);
    }
    let header = |name: &str| {
        reqwest::header::HeaderValue::from_str(key)
            .map(|value| (name.to_owned(), value))
            .map_err(|_| anyhow!("API 密钥包含无法发送的字符"))
    };
    let (name, value) = match style {
        "anthropic" => header("x-api-key")?,
        "gemini" => header("x-goog-api-key")?,
        _ => {
            return Ok(request.bearer_auth(key));
        }
    };
    let mut request = request.header(name, value);
    if style == "anthropic" {
        request = request.header("anthropic-version", "2023-06-01");
    }
    Ok(request)
}

fn parse_model_ids(style: &str, data: &Value) -> Vec<String> {
    let mut models = Vec::new();
    let push = |models: &mut Vec<String>, id: &str| {
        let id = id.trim();
        if !id.is_empty() && !models.iter().any(|existing| existing == id) {
            models.push(id.to_owned());
        }
    };
    match style {
        "gemini" => {
            if let Some(items) = data.get("models").and_then(Value::as_array) {
                for item in items {
                    if let Some(name) = item.get("name").and_then(Value::as_str) {
                        push(&mut models, name.rsplit('/').next().unwrap_or(name));
                    }
                }
            }
        }
        "ollama" => {
            if let Some(items) = data.get("models").and_then(Value::as_array) {
                for item in items {
                    if let Some(name) = item.get("name").and_then(Value::as_str).or_else(|| item.get("model").and_then(Value::as_str)) {
                        push(&mut models, name);
                    }
                }
            }
        }
        _ => {
            if let Some(items) = data.get("data").and_then(Value::as_array) {
                for item in items {
                    if let Some(id) = item.get("id").and_then(Value::as_str) {
                        push(&mut models, id);
                    }
                }
            }
        }
    }
    models
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    #[test]
    fn catalog_covers_the_required_providers_and_default_urls() {
        let expected = [
            ("openai", "https://api.openai.com/v1", "openai"),
            ("anthropic", "https://api.anthropic.com/v1", "anthropic"),
            ("gemini", "https://generativelanguage.googleapis.com/v1beta/openai", "gemini"),
            ("deepseek", "https://api.deepseek.com", "openai"),
            ("dashscope", "https://dashscope.aliyuncs.com/compatible-mode/v1", "openai"),
            ("glm", "https://open.bigmodel.cn/api/paas/v4", "openai"),
            ("moonshot", "https://api.moonshot.cn/v1", "openai"),
            ("siliconflow", "https://api.siliconflow.cn/v1", "openai"),
            ("openrouter", "https://openrouter.ai/api/v1", "openai"),
            ("ollama", "http://127.0.0.1:11434/v1", "ollama"),
            ("custom", "", "openai"),
        ];
        for (id, url, api) in expected {
            let provider = provider(id).unwrap_or_else(|| panic!("missing {id}"));
            assert!(!provider.label.is_empty(), "{id}");
            assert_eq!(provider.base_url, url, "{id}");
            assert_eq!(provider.api, api, "{id}");
            if id != "custom" {
                assert!(!presets_for(id).is_empty(), "{id} should ship fallback models");
            }
        }
        assert!(provider("302").is_some(), "existing 302.AI configs stay selectable");
    }

    #[test]
    fn provider_inference_uses_host_and_keeps_local_proxies_custom() {
        assert_eq!(infer_provider("answer", "https://api.openai.com/v1"), "openai");
        assert_eq!(infer_provider("answer", "https://dashscope-intl.aliyuncs.com/compatible-mode/v1"), "dashscope");
        assert_eq!(infer_provider("answer", "http://127.0.0.1:11434/v1"), "ollama");
        assert_eq!(infer_provider("answer", "http://127.0.0.1:8000/v1"), "custom");
        assert_eq!(infer_provider("answer", "https://api.302.ai/v1"), "302");
        assert_eq!(infer_provider("jev", "https://api.zleap.com/v1"), "zleap");
        assert_eq!(infer_provider("jev", "https://api.typesafe.ai/v1"), "official");
        assert_eq!(infer_provider("jev", "https://openrouter.ai/api/alpha/decisions"), "openrouter");
        assert_eq!(infer_provider("jev", "https://api.openai.com/v1"), "custom");
    }

    #[test]
    fn legacy_model_settings_keep_fields_and_remove_embedded_secrets() {
        let mut value = json!({
            "theme": "dark",
            "retrieval": {"candidate_k": 70, "seed_k": 10},
            "model_profiles": {"gpt-4o": {"system_prompt": "保留", "mcp_servers": []}},
            "answer": {
                "provider": "",
                "base_url": "https://api.openai.com/v1",
                "model_name": "gpt-4o",
                "models": "gpt-4o, gpt-4o-mini\ngpt-4o",
                "api_key": "legacy-answer-key",
                "auth_mode": "bearer",
                "reasoning_effort": "low",
                "note": "keep-me"
            },
            "jev": {
                "url": "https://scorer.example/decisions",
                "model": "custom-jev",
                "key": "legacy-jev-key",
                "auth_mode": "bearer"
            }
        });
        let secrets = migrate_settings_value(&mut value);
        assert_eq!(secrets.answer.as_deref(), Some("legacy-answer-key"));
        assert_eq!(secrets.jev.as_deref(), Some("legacy-jev-key"));
        assert!(secrets.changed);
        assert_eq!(value["theme"], "dark");
        assert_eq!(value["retrieval"]["candidate_k"], 70);
        assert_eq!(value["model_profiles"]["gpt-4o"]["system_prompt"], "保留");
        assert_eq!(value["answer"]["url"], "https://api.openai.com/v1");
        assert_eq!(value["answer"]["model"], "gpt-4o");
        assert_eq!(value["answer"]["models"], json!(["gpt-4o", "gpt-4o-mini"]));
        assert_eq!(value["answer"]["provider"], "openai");
        assert_eq!(value["answer"]["has_key"], true);
        assert_eq!(value["answer"]["reasoning_effort"], "low");
        assert_eq!(value["answer"]["note"], "keep-me");
        assert!(value["answer"].get("api_key").is_none());
        assert!(value["answer"].get("base_url").is_none());
        assert_eq!(value["jev"]["provider"], "custom");
        assert_eq!(value["jev"]["model"], "custom-jev");
        assert!(value["jev"].get("key").is_none());
        assert!(!value.to_string().contains("legacy-answer-key"));
        assert!(!value.to_string().contains("legacy-jev-key"));
        let mut anthropic = json!({"answer": {"provider": "anthropic", "url": "https://api.anthropic.com", "model": "claude-sonnet-4-5", "models": ["claude-sonnet-4-5"]}});
        assert!(migrate_settings_value(&mut anthropic).changed);
        assert_eq!(anthropic["answer"]["url"], "https://api.anthropic.com/v1");
        assert_eq!(anthropic["answer"]["model"], "claude-sonnet-4-5");
        assert!(!migrate_settings_value(&mut anthropic).changed);
        let mut rooted = json!({"answer": {"url": "https://api.anthropic.com/", "model": "claude-sonnet-4-5"}});
        assert!(migrate_settings_value(&mut rooted).changed);
        assert_eq!(rooted["answer"]["url"], "https://api.anthropic.com/v1");
        assert_eq!(rooted["answer"]["provider"], "anthropic");
        let again = migrate_settings_value(&mut value);
        assert!(!again.changed);
        assert!(again.answer.is_none() && again.jev.is_none());
    }

    #[test]
    fn failure_lines_and_logs_redact_the_key() {
        let key = "sk-test-secret-value";
        let line = provider_failure_line(401, &format!("unauthorized {key} bearer {key}"), key);
        assert!(!line.contains(key));
        assert!(line.contains("[已隐藏]"));
        assert!(line.contains("HTTP 401"));
    }

    fn serve(status: u16, body: &str, expect_header: Option<(&'static str, String)>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let body = body.to_owned();
        std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
            let mut bytes = Vec::new();
            let mut part = [0; 4096];
            loop {
                let count = socket.read(&mut part).unwrap();
                if count == 0 {
                    break;
                }
                bytes.extend_from_slice(&part[..count]);
                if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            let header = String::from_utf8_lossy(&bytes).to_ascii_lowercase();
            if let Some((name, value)) = expect_header {
                assert!(header.contains(&format!("{name}: {}", value.to_ascii_lowercase())), "{header}");
            }
            write!(socket, "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        });
        format!("http://{address}")
    }

    #[tokio::test]
    async fn model_lists_follow_each_provider_api_and_failures_hide_the_key() {
        let key = "sk-unit-test-key";
        let cases = [
            ("openai", "/v1", "openai", json!({"data":[{"id":"remote-b"},{"id":"remote-a"},{"id":"remote-b"}]}), vec!["remote-b", "remote-a"]),
            ("anthropic", "", "anthropic", json!({"data":[{"id":"claude-test"}]}), vec!["claude-test"]),
            ("gemini", "/v1beta/openai", "gemini", json!({"models":[{"name":"models/gemini-test"}]}), vec!["gemini-test"]),
            ("ollama", "/v1", "ollama", json!({"models":[{"name":"llama-test"}]}), vec!["llama-test"]),
        ];
        for (id, suffix, style, body, expected) in cases {
            let url = serve(200, &body.to_string(), None);
            let config = ModelConfig { provider: id.into(), url: format!("{url}{suffix}"), model: "placeholder".into(), ..Default::default() };
            assert!(list_url(&config).unwrap().contains(match style {
                "anthropic" => "/v1/models",
                "gemini" => "/v1beta/models",
                "ollama" => "/api/tags",
                _ => "/models",
            }), "{id}");
            let models = list_models(&Client::new(), &config, if id == "ollama" { "" } else { key }, 5).await.unwrap();
            assert_eq!(models, expected, "{id}");
        }
        let url = serve(401, &format!("denied {key}"), None);
        let config = ModelConfig { provider: "openai".into(), url: url, ..Default::default() };
        let error = list_models(&Client::new(), &config, key, 5).await.unwrap_err().to_string();
        assert!(error.contains("HTTP 401"));
        assert!(!error.contains(key));
        assert!(error.contains("[已隐藏]"));
    }

    #[tokio::test]
    async fn native_connection_tests_succeed_and_failures_hide_the_key() {
        let key = "sk-connection-test";
        let anthropic = serve(200, r#"{"type":"message","content":[{"type":"text","text":"OK"}]}"#, Some(("x-api-key", key.into())));
        let config = ModelConfig { provider: "anthropic".into(), url: anthropic, model: "claude-test".into(), ..Default::default() };
        assert_eq!(test_native(&Client::new(), &config, key, 5).await.unwrap(), "连接成功");
        let gemini = serve(200, r#"{"candidates":[{"content":{"parts":[{"text":"OK"}]}}]}"#, Some(("x-goog-api-key", key.into())));
        let config = ModelConfig { provider: "gemini".into(), url: format!("{gemini}/v1beta/openai"), model: "gemini-test".into(), ..Default::default() };
        assert_eq!(test_native(&Client::new(), &config, key, 5).await.unwrap(), "连接成功");
        assert!(!native_test_url(&config).unwrap().contains(key));
        let ollama = serve(200, r#"{"message":{"role":"assistant","content":"OK"}}"#, None);
        let config = ModelConfig { provider: "ollama".into(), url: format!("{ollama}/v1"), model: "llama-test".into(), ..Default::default() };
        assert_eq!(test_native(&Client::new(), &config, "", 5).await.unwrap(), "连接成功");
        let failed = serve(403, &format!("nope {key}"), None);
        let config = ModelConfig { provider: "anthropic".into(), url: failed, model: "claude-test".into(), ..Default::default() };
        let error = test_native(&Client::new(), &config, key, 5).await.unwrap_err().to_string();
        assert!(error.contains("HTTP 403"));
        assert!(!error.contains(key));
    }
}
