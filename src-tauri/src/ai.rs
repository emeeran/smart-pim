//! AI provider layer (docs/ai_integ_plan.md §1–2): OpenAI-compatible HTTP
//! against a local server by default (Ollama / llama.cpp at
//! `http://127.0.0.1:11434`), cloud opt-in with a BYO key stored in the
//! encrypted `setting` table. No telemetry, no weight downloads; the API
//! key is never logged. The [`Http`] transport is injectable so everything
//! except the socket is unit-testable.

use rusqlite::Connection;
use serde::Deserialize;
use serde_json::{json, Value};

/// Settings keys (plan §4).
pub const PROVIDER_KEY: &str = "ai.provider";
pub const ENDPOINT_KEY: &str = "ai.endpoint";
pub const MODEL_KEY: &str = "ai.model";
pub const CLOUD_KEY: &str = "ai.cloud_enabled";
pub const API_KEY_REF: &str = "ai.api_key_ref";

pub const DEFAULT_ENDPOINT: &str = "http://127.0.0.1:11434";

/// Per-feature opt-in flags (`ai.feature.<name>`) with their defaults.
/// Semantic search and anything touching the diary ship OFF.
pub const FEATURES: &[(&str, bool)] = &[
    ("summarize", true),
    ("digest", true),
    ("reply", true),
    ("capture", true),
    ("action_items", true),
    ("auto_tag", true),
    ("triage", true),
    ("subtasks", true),
    ("contacts", true),
    ("semantic", false),
    ("diary", false),
];

/// Optional per-1k-token cost rates for tier-2 accounting (plan §4/§5),
/// stored as decimal strings; empty = no estimate shown.
pub const COST_IN_KEY: &str = "ai.cost_per_1k_in";
pub const COST_OUT_KEY: &str = "ai.cost_per_1k_out";

/// Effective configuration read from settings.
#[derive(Debug, Clone, PartialEq)]
pub struct AiConfig {
    pub provider: String, // "local" | "cloud"
    pub endpoint: String,
    pub model: String,
    pub cloud_enabled: bool,
}

fn setting(conn: &Connection, key: &str) -> Option<String> {
    conn.query_row("SELECT value FROM setting WHERE key = ?1", [key], |r| {
        r.get(0)
    })
    .ok()
}

/// Read the effective AI configuration (defaults per plan §2).
#[must_use]
pub fn load_config(conn: &Connection) -> AiConfig {
    AiConfig {
        provider: setting(conn, PROVIDER_KEY).unwrap_or_else(|| "local".into()),
        endpoint: setting(conn, ENDPOINT_KEY).unwrap_or_else(|| DEFAULT_ENDPOINT.into()),
        model: setting(conn, MODEL_KEY).unwrap_or_default(),
        cloud_enabled: setting(conn, CLOUD_KEY).as_deref() == Some("1"),
    }
}

/// The API key, only ever read for a request; never logged or returned by
/// any command (settings expose only whether it is set).
pub fn api_key(conn: &Connection) -> Option<String> {
    if !load_config(conn).cloud_enabled {
        return None;
    }
    setting(conn, API_KEY_REF).filter(|k| !k.is_empty())
}

/// Effective per-feature flag map.
#[must_use]
pub fn features(conn: &Connection) -> std::collections::BTreeMap<String, bool> {
    FEATURES
        .iter()
        .map(|(name, default)| {
            let key = format!("ai.feature.{name}");
            let v = setting(conn, &key).map(|s| s == "1").unwrap_or(*default);
            ((*name).to_string(), v)
        })
        .collect()
}

/// Is `kind` allowed by its feature flag?
#[must_use]
pub fn feature_allows(conn: &Connection, kind: &str) -> bool {
    let flag = match kind {
        "summarize_note" => "summarize",
        "thread_digest" => "digest",
        "draft_reply" => "reply",
        "nl_task" => "capture",
        "action_items" => "action_items",
        "auto_tag" => "auto_tag",
        "triage_suggest" => "triage",
        "subtask_split" => "subtasks",
        "signature_contact" => "contacts",
        "diary_digest" => "diary",
        "reindex" => "semantic",
        _ => "",
    };
    features(conn).get(flag).copied().unwrap_or(false)
}

/// A per-1k-token cost rate from settings, if it parses as a non-negative
/// number. Absent/negative → no estimate is shown anywhere.
#[must_use]
pub fn cost_rate(conn: &Connection, key: &str) -> Option<f64> {
    setting(conn, key)
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|f| *f >= 0.0)
}

// ---------------------------------------------------------------- transport

/// Minimal HTTP surface the providers need (injectable for tests).
pub trait Http {
    /// POST a JSON body, return the response body text (any status).
    fn post(&self, url: &str, body: &str, key: Option<&str>) -> Result<String, String>;
    /// GET a URL, return the response body text (any status).
    fn get(&self, url: &str, key: Option<&str>) -> Result<String, String>;
}

fn agent_with_timeout(secs: u64) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(secs)))
        .build()
        .new_agent()
}

/// Real transport: ureq with a 120 s timeout (local models are slow);
/// status probes use the short-timeout variant.
#[derive(Default)]
pub struct UreqHttp;

impl Http for UreqHttp {
    fn post(&self, url: &str, body: &str, key: Option<&str>) -> Result<String, String> {
        let mut req = agent_with_timeout(120)
            .post(url)
            .header("Content-Type", "application/json");
        if let Some(k) = key {
            req = req.header("Authorization", &format!("Bearer {k}"));
        }
        let mut res = req.send(body).map_err(|e| e.to_string())?;
        res.body_mut().read_to_string().map_err(|e| e.to_string())
    }

    fn get(&self, url: &str, key: Option<&str>) -> Result<String, String> {
        let mut req = agent_with_timeout(5).get(url);
        if let Some(k) = key {
            req = req.header("Authorization", &format!("Bearer {k}"));
        }
        let mut res = req.call().map_err(|e| e.to_string())?;
        res.body_mut().read_to_string().map_err(|e| e.to_string())
    }
}

// ---------------------------------------------------------------- provider

/// One chat message.
pub struct ChatMsg {
    pub role: &'static str,
    pub content: String,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
    #[serde(default)]
    usage: Option<ChatUsage>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatMessage,
}

#[derive(Deserialize)]
struct ChatMessage {
    content: Option<String>,
}

/// Token usage as reported by the provider (OpenAI-compatible servers and
/// Ollama's OpenAI layer both send it; a server that omits it counts as 0,
/// which keeps accounting best-effort rather than load-bearing).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
    pub tokens_in: u64,
    pub tokens_out: u64,
}

#[derive(Deserialize)]
struct ChatUsage {
    #[serde(default)]
    prompt_tokens: u64,
    #[serde(default)]
    completion_tokens: u64,
}

/// A chat completion plus the usage it consumed.
#[derive(Debug, Clone)]
pub struct ChatOut {
    pub content: String,
    pub usage: Usage,
}

/// Non-streaming chat completion against `{endpoint}/v1/chat/completions`.
///
/// # Errors
/// Transport or protocol failure, described as text.
pub fn chat(
    http: &dyn Http,
    cfg: &AiConfig,
    key: Option<&str>,
    messages: &[ChatMsg],
) -> Result<ChatOut, String> {
    let url = format!("{}/v1/chat/completions", cfg.endpoint.trim_end_matches('/'));
    let body = json!({
        "model": cfg.model,
        "messages": messages.iter().map(|m| json!({"role": m.role, "content": m.content})).collect::<Vec<_>>(),
        "stream": false,
    });
    let text = http.post(&url, &body.to_string(), key)?;
    let parsed: ChatResponse =
        serde_json::from_str(&text).map_err(|e| format!("bad chat response: {e}"))?;
    let content = parsed
        .choices
        .first()
        .and_then(|c| c.message.content.clone())
        .ok_or_else(|| String::from("empty chat response"))?;
    let usage = parsed
        .usage
        .map(|u| Usage {
            tokens_in: u.prompt_tokens,
            tokens_out: u.completion_tokens,
        })
        .unwrap_or_default();
    Ok(ChatOut { content, usage })
}

#[derive(Deserialize)]
struct EmbedResponse {
    data: Vec<EmbedData>,
}

#[derive(Deserialize)]
struct EmbedData {
    embedding: Vec<f32>,
}

/// Embeddings against `{endpoint}/v1/embeddings`.
///
/// # Errors
/// Transport or protocol failure, described as text.
pub fn embed(
    http: &dyn Http,
    cfg: &AiConfig,
    key: Option<&str>,
    texts: &[String],
) -> Result<Vec<Vec<f32>>, String> {
    let url = format!("{}/v1/embeddings", cfg.endpoint.trim_end_matches('/'));
    let body = json!({"model": cfg.model, "input": texts});
    let text = http.post(&url, &body.to_string(), key)?;
    let parsed: EmbedResponse =
        serde_json::from_str(&text).map_err(|e| format!("bad embedding response: {e}"))?;
    Ok(parsed.data.into_iter().map(|d| d.embedding).collect())
}

/// Probe the server: model list via OpenAI `/v1/models`, falling back to
/// Ollama's native `/api/tags`. Returns model ids; empty on unreachable.
#[must_use]
pub fn list_models(http: &dyn Http, cfg: &AiConfig, key: Option<&str>) -> Vec<String> {
    if let Ok(text) = http.get(
        &format!("{}/v1/models", cfg.endpoint.trim_end_matches('/')),
        key,
    ) {
        if let Ok(v) = serde_json::from_str::<Value>(&text) {
            let ids: Vec<String> = v["data"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|m| m["id"].as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            if !ids.is_empty() {
                return ids;
            }
        }
    }
    if let Ok(text) = http.get(
        &format!("{}/api/tags", cfg.endpoint.trim_end_matches('/')),
        key,
    ) {
        if let Ok(v) = serde_json::from_str::<Value>(&text) {
            return v["models"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|m| m["name"].as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
        }
    }
    Vec::new()
}

/// Extract the first JSON object from a model reply, tolerating ```json
/// fences and prose around it.
#[must_use]
pub fn extract_json(text: &str) -> Option<Value> {
    let start = text.find('{')?;
    // Walk to the matching close brace.
    let bytes = text.as_bytes();
    let (mut depth, mut in_str, mut escape) = (0i32, false, false);
    for (i, b) in bytes.iter().enumerate().skip(start) {
        match (*b, in_str, escape) {
            (b'"', false, false) => in_str = true,
            (b'"', true, false) => in_str = false,
            (b'\\', true, false) => escape = true,
            (_, true, _) => escape = false,
            (b'{', false, false) => depth += 1,
            (b'}', false, false) => {
                depth -= 1;
                if depth == 0 {
                    return serde_json::from_str(&text[start..=i]).ok();
                }
            }
            _ => escape = false,
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    struct FakeHttp {
        last_url: RefCell<Option<String>>,
        last_body: RefCell<Option<String>>,
        last_key: RefCell<Option<String>>,
        response: String,
    }

    impl FakeHttp {
        fn new(response: &str) -> Self {
            Self {
                last_url: RefCell::new(None),
                last_body: RefCell::new(None),
                last_key: RefCell::new(None),
                response: response.to_string(),
            }
        }
    }

    impl Http for FakeHttp {
        fn post(&self, url: &str, body: &str, key: Option<&str>) -> Result<String, String> {
            *self.last_url.borrow_mut() = Some(url.into());
            *self.last_body.borrow_mut() = Some(body.into());
            *self.last_key.borrow_mut() = key.map(str::to_string);
            Ok(self.response.clone())
        }
        fn get(&self, url: &str, key: Option<&str>) -> Result<String, String> {
            *self.last_url.borrow_mut() = Some(url.into());
            *self.last_key.borrow_mut() = key.map(str::to_string);
            Ok(self.response.clone())
        }
    }

    fn cfg() -> AiConfig {
        AiConfig {
            provider: "local".into(),
            endpoint: "http://127.0.0.1:11434/".into(), // trailing slash must be trimmed
            model: "test-model".into(),
            cloud_enabled: false,
        }
    }

    #[test]
    fn chat_sends_openai_shape_and_extracts_content() {
        let fake = FakeHttp::new(
            r#"{"choices":[{"message":{"content":"Hello there"}}],
                "usage":{"prompt_tokens":12,"completion_tokens":34}}"#,
        );
        let out = chat(
            &fake,
            &cfg(),
            None,
            &[
                ChatMsg {
                    role: "system",
                    content: "be brief".into(),
                },
                ChatMsg {
                    role: "user",
                    content: "hi".into(),
                },
            ],
        )
        .unwrap();
        assert_eq!(out.content, "Hello there");
        assert_eq!(
            out.usage,
            Usage {
                tokens_in: 12,
                tokens_out: 34
            }
        );
        assert_eq!(
            fake.last_url.borrow().as_deref(),
            Some("http://127.0.0.1:11434/v1/chat/completions")
        );
        let body: Value =
            serde_json::from_str(fake.last_body.borrow().as_deref().unwrap()).unwrap();
        assert_eq!(body["model"], "test-model");
        assert_eq!(body["stream"], false);
        assert_eq!(body["messages"][0]["role"], "system");
        assert!(
            fake.last_key.borrow().is_none(),
            "no key without cloud opt-in"
        );
    }

    #[test]
    fn chat_without_usage_reports_zeroes() {
        let fake = FakeHttp::new(r#"{"choices":[{"message":{"content":"x"}}]}"#);
        let out = chat(&fake, &cfg(), None, &[]).unwrap();
        assert_eq!(out.usage, Usage::default(), "missing usage block is 0/0");
    }

    #[test]
    fn embeddings_parse_array() {
        let fake = FakeHttp::new(r#"{"data":[{"embedding":[0.1,0.2]},{"embedding":[0.3,0.4]}]}"#);
        let out = embed(&fake, &cfg(), None, &["a".into(), "b".into()]).unwrap();
        assert_eq!(out, vec![vec![0.1, 0.2], vec![0.3, 0.4]]);
        assert!(fake
            .last_url
            .borrow()
            .as_deref()
            .is_some_and(|u| u.ends_with("/v1/embeddings")));
    }

    #[test]
    fn models_fallback_to_ollama_tags() {
        let mut fake = FakeHttp::new("not json"); // /v1/models fails to parse
        let got = list_models(&fake, &cfg(), None);
        assert!(got.is_empty());
        fake.response = r#"{"models":[{"name":"llama3.2:latest"}]}"#.to_string();
        let got = list_models(&fake, &cfg(), None);
        assert_eq!(got, vec!["llama3.2:latest"]);
    }

    #[test]
    fn extract_json_tolerates_fences_and_prose() {
        assert_eq!(
            extract_json("Sure!\n```json\n{\"a\": 1}\n```\n hope that helps"),
            Some(json!({"a": 1}))
        );
        assert_eq!(extract_json("no json here"), None);
        assert_eq!(
            extract_json("prefix {\"s\": \"has } brace\"} suffix"),
            Some(json!({"s": "has } brace"}))
        );
        assert_eq!(extract_json("{broken"), None);
    }

    #[test]
    fn feature_gate_maps_kinds() {
        let s = pfcore::Store::open_memory("x").unwrap();
        let conn = s.conn();
        // Defaults: everything on except semantic + diary.
        assert!(feature_allows(conn, "summarize_note"));
        assert!(feature_allows(conn, "thread_digest"));
        assert!(feature_allows(conn, "draft_reply"));
        assert!(feature_allows(conn, "nl_task"));
        assert!(feature_allows(conn, "action_items"));
        assert!(feature_allows(conn, "auto_tag"));
        assert!(feature_allows(conn, "triage_suggest"));
        assert!(feature_allows(conn, "subtask_split"));
        assert!(feature_allows(conn, "signature_contact"));
        assert!(!feature_allows(conn, "diary_digest"));
        assert!(!feature_allows(conn, "reindex"));
        conn.execute(
            "INSERT INTO setting (key, value) VALUES ('ai.feature.diary', '1')",
            [],
        )
        .unwrap();
        assert!(feature_allows(conn, "diary_digest"));
    }

    #[test]
    fn api_key_requires_cloud_opt_in() {
        let s = pfcore::Store::open_memory("x").unwrap();
        let conn = s.conn();
        conn.execute(
            "INSERT INTO setting (key, value) VALUES ('ai.api_key_ref', 'sk-secret')",
            [],
        )
        .unwrap();
        assert!(api_key(conn).is_none(), "key ignored while cloud is off");
        conn.execute(
            "INSERT INTO setting (key, value) VALUES ('ai.cloud_enabled', '1')",
            [],
        )
        .unwrap();
        assert_eq!(api_key(conn).as_deref(), Some("sk-secret"));
    }
}
