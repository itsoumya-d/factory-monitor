//! Provider abstraction (docs/09 §1): one trait, three wire schemes
//! (OpenAI-compatible, Gemini, Anthropic), plus a scripted mock so every
//! downstream test runs hermetically.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::error::VlmError;
use crate::schema::JsonSchemaSpec;

/// One structured chat round-trip. `body` is the prompt payload (json),
/// `task` routes the reply on the provider side.
#[derive(Clone)]
pub struct VlmRequest {
    pub task: String,
    pub body: Value,
}

impl VlmRequest {
    pub fn new(task: impl Into<String>, body: Value) -> Self {
        Self {
            task: task.into(),
            body,
        }
    }

    /// Flattened user text sent on the wire.
    pub fn user_text(&self) -> Result<String, VlmError> {
        Ok(serde_json::to_string(&json!({
            "task": self.task,
            "payload": self.body,
        }))?)
    }

    /// Approximate wire size for cost accounting (docs/09 §6).
    pub fn wire_bytes(&self) -> usize {
        self.user_text().map_or(0, |t| t.len())
    }
}

#[async_trait]
pub trait VlmProvider: Send + Sync {
    /// Structured chat with JSON-schema constrained output. Implementations
    /// MUST reject answers failing `schema` (fail-closed, docs/09 §1).
    async fn chat_json(&self, req: VlmRequest, schema: &JsonSchemaSpec) -> Result<Value, VlmError>;

    fn name(&self) -> &str;
}

/// Minimal HTTP seam so every provider test is hermetic; the real impl
/// wraps `reqwest` with a per-call timeout (docs/09 §1: timeout 90 s).
#[async_trait]
pub trait HttpTransport: Send + Sync {
    async fn post_json(
        &self,
        url: &str,
        headers: &[(String, String)],
        body: &Value,
    ) -> Result<HttpResponse, String>;
}

pub struct HttpResponse {
    pub status: u16,
    pub body: String,
}

pub struct ReqwestTransport {
    timeout_secs: u64,
}

impl ReqwestTransport {
    pub const fn new(timeout_secs: u64) -> Self {
        Self { timeout_secs }
    }
}

#[async_trait]
impl HttpTransport for ReqwestTransport {
    async fn post_json(
        &self,
        url: &str,
        headers: &[(String, String)],
        body: &Value,
    ) -> Result<HttpResponse, String> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(self.timeout_secs))
            .build()
            .map_err(|e| e.to_string())?;
        let mut req = client.post(url).json(body);
        for (name, value) in headers {
            req = req.header(name, value);
        }
        let resp = req.send().await.map_err(|e| e.to_string())?;
        let status = resp.status().as_u16();
        let text = resp.text().await.map_err(|e| e.to_string())?;
        Ok(HttpResponse { status, body: text })
    }
}

/// Scripted provider for tests and offline simulation: pops planned
/// results in order, then falls back to a per-task canned map.
pub struct MockProvider {
    name: String,
    planned: Mutex<VecDeque<Result<Value, VlmError>>>,
    fallback: Mutex<HashMap<String, Value>>,
}

impl Default for MockProvider {
    fn default() -> Self {
        Self::new("mock")
    }
}

impl MockProvider {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            planned: Mutex::new(VecDeque::new()),
            fallback: Mutex::new(HashMap::new()),
        }
    }

    pub fn plan(&self, entries: Vec<Result<Value, VlmError>>) -> &Self {
        self.planned
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .extend(entries);
        self
    }

    pub fn respond_with(&self, task: impl Into<String>, value: Value) -> &Self {
        self.fallback
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(task.into(), value);
        self
    }
}

#[async_trait]
impl VlmProvider for MockProvider {
    async fn chat_json(&self, req: VlmRequest, schema: &JsonSchemaSpec) -> Result<Value, VlmError> {
        let next = self
            .planned
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pop_front();
        let value = match next {
            Some(front) => front?,
            None => self
                .fallback
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(&req.task)
                .cloned()
                .ok_or_else(|| VlmError::Schema(format!("mock has no answer for {}", req.task)))?,
        };
        schema.validate(&value).map_err(VlmError::Schema)?;
        Ok(value)
    }

    fn name(&self) -> &str {
        &self.name
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderKind {
    OpenAi,
    Gemini,
    Anthropic,
}

/// Real cloud provider behind the URL-scheme abstraction (docs/09 §1).
/// The secret lives in memory only — never persisted (docs/13 §7).
pub struct CloudProvider {
    kind: ProviderKind,
    name: String,
    base_url: String,
    model: String,
    api_key: String,
    http: Arc<dyn HttpTransport>,
}

impl CloudProvider {
    pub fn new(
        kind: ProviderKind,
        name: impl Into<String>,
        base_url: impl Into<String>,
        model: impl Into<String>,
        api_key: impl Into<String>,
        http: Arc<dyn HttpTransport>,
    ) -> Result<Self, VlmError> {
        let api_key = api_key.into();
        if api_key.is_empty() {
            return Err(VlmError::Auth(name.into()));
        }
        Ok(Self {
            kind,
            name: name.into(),
            base_url: base_url.into(),
            model: model.into(),
            api_key,
            http,
        })
    }
}

fn endpoint(kind: ProviderKind, base_url: &str, model: &str) -> String {
    let base = base_url.trim_end_matches('/');
    match kind {
        ProviderKind::OpenAi => format!("{base}/chat/completions"),
        ProviderKind::Gemini => format!("{base}/models/{model}:generateContent"),
        ProviderKind::Anthropic => format!("{base}/messages"),
    }
}

fn wire_request(kind: ProviderKind, model: &str, user_text: &str) -> Value {
    match kind {
        ProviderKind::OpenAi => json!({
            "model": model,
            "messages": [{"role": "user", "content": user_text}],
            "response_format": {"type": "json_object"},
        }),
        ProviderKind::Gemini => json!({
            "contents": [{"parts": [{"text": user_text}]}],
            "generationConfig": {"responseMimeType": "application/json"},
        }),
        ProviderKind::Anthropic => json!({
            "model": model,
            "max_tokens": 4096,
            "messages": [{"role": "user", "content": user_text}],
        }),
    }
}

fn wire_headers(kind: ProviderKind, api_key: &str) -> Vec<(String, String)> {
    match kind {
        ProviderKind::OpenAi => vec![("Authorization".into(), format!("Bearer {api_key}"))],
        ProviderKind::Gemini => vec![("x-goog-api-key".into(), api_key.into())],
        ProviderKind::Anthropic => vec![
            ("x-api-key".into(), api_key.into()),
            ("anthropic-version".into(), "2023-06-01".into()),
        ],
    }
}

/// Walk a (str keys OR numeric array indices) path into the envelope.
fn pointer<'a>(body: &'a Value, path: &[&str]) -> Result<&'a Value, VlmError> {
    let mut node = body;
    for key in path {
        let next = if let Value::Array(arr) = node {
            arr.get(
                key.parse::<usize>()
                    .map_err(|_| VlmError::Schema(format!("non-numeric index {key}")))?, // on Arrays
            )
        } else {
            node.get(key)
        };
        node = next.ok_or_else(|| VlmError::Schema(format!("missing field {key}")))?;
    }
    Ok(node)
}

fn extract(kind: ProviderKind, envelope: &Value) -> Result<Value, VlmError> {
    let path: &[&str] = match kind {
        ProviderKind::OpenAi => &["choices", "0", "message", "content"],
        ProviderKind::Gemini => &["candidates", "0", "content", "parts", "0", "text"],
        ProviderKind::Anthropic => &["content", "0", "text"],
    };
    let text = pointer(envelope, path)?.as_str().ok_or(VlmError::Schema(
        "content/text envelope field is not a string".into(),
    ))?;
    Ok(serde_json::from_str(text)?)
}

#[async_trait]
impl VlmProvider for CloudProvider {
    async fn chat_json(&self, req: VlmRequest, schema: &JsonSchemaSpec) -> Result<Value, VlmError> {
        let user_text = req.user_text()?;
        let url = endpoint(self.kind, &self.base_url, &self.model);
        let headers = wire_headers(self.kind, &self.api_key);
        let wire = wire_request(self.kind, &self.model, &user_text);
        let resp = self
            .http
            .post_json(&url, &headers, &wire)
            .await
            .map_err(VlmError::Http)?;
        if resp.status == 429 {
            return Err(VlmError::RateLimited {
                provider: self.name.clone(),
            });
        }
        if !(200..300).contains(&resp.status) {
            return Err(VlmError::Provider {
                provider: self.name.clone(),
                status: resp.status,
                body: resp.body.chars().take(400).collect(),
            });
        }
        let envelope: Value = serde_json::from_str(&resp.body)?;
        let value = extract(self.kind, &envelope)?;
        schema.validate(&value).map_err(VlmError::Schema)?;
        Ok(value)
    }

    fn name(&self) -> &str {
        &self.name
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use std::sync::Mutex;

    type SeenCalls = Mutex<Vec<(String, Vec<(String, String)>, Value)>>;

    struct ScriptedHttp {
        planned: Mutex<Vec<HttpResponse>>,
        seen: SeenCalls,
    }

    impl ScriptedHttp {
        fn new(responses: Vec<u16>) -> Self {
            // All canned bodies use the OpenAI envelope shape.
            let planned = responses
                .into_iter()
                .map(|status| HttpResponse {
                    status,
                    body: r#"{"choices":[{"message":{"content":"{\"ok\": true}"}}]}"#.to_string(),
                })
                .collect();
            Self {
                planned: Mutex::new(planned),
                seen: Mutex::new(Vec::new()),
            }
        }
        fn body_ok() -> String {
            r#"{"choices":[{"message":{"content":"{\"walkway_blocked\": false, \"detect_changes\":[], \"new_equipment_needs_roi\":[]}"}}]}"#.to_string()
        }
    }

    #[async_trait]
    impl HttpTransport for ScriptedHttp {
        async fn post_json(
            &self,
            url: &str,
            headers: &[(String, String)],
            body: &Value,
        ) -> Result<HttpResponse, String> {
            self.seen.lock().map_err(|e| e.to_string())?.push((
                url.into(),
                headers.to_vec(),
                body.clone(),
            ));
            let mut planned = self.planned.lock().map_err(|e| e.to_string())?;
            if planned.is_empty() {
                return Ok(HttpResponse {
                    status: 200,
                    body: Self::body_ok(),
                });
            }
            let mut resp = planned.remove(0);
            if resp.status == 200 {
                resp.body = Self::body_ok();
            }
            Ok(resp)
        }
    }

    fn open_ai_provider(scripts: Vec<u16>) -> (CloudProvider, Arc<ScriptedHttp>) {
        let http = Arc::new(ScriptedHttp::new(scripts));
        let provider = CloudProvider::new(
            ProviderKind::OpenAi,
            "openai-test",
            "https://api.example.com/v2",
            "org-vlm-1",
            "sk-test",
            http.clone(),
        )
        .unwrap_or_else(|e| panic!("provider: {e}"));
        (provider, http)
    }

    fn layout_req() -> VlmRequest {
        VlmRequest::new("factory_layout_diff", serde_json::json!({"x": 1}))
    }

    #[tokio::test]
    async fn cloud_provider_hits_openai_url_with_auth_and_validates() {
        let (provider, http) = open_ai_provider(vec![]);
        let schema = crate::schema::layout_diff_schema();
        let value = provider
            .chat_json(layout_req(), &schema)
            .await
            .unwrap_or_else(|e| panic!("call: {e}"));
        assert!(
            !value
                .get("walkway_blocked")
                .and_then(Value::as_bool)
                .unwrap_or(true)
        );
        let seen = http.seen.lock().expect("seen mutex");
        let (url, headers, body) = &seen[0];
        assert!(url.ends_with("/chat/completions"));
        assert!(
            headers
                .iter()
                .any(|(k, v)| k == "Authorization" && v.starts_with("Bearer sk-test"))
        );
        assert_eq!(body.get("model").and_then(Value::as_str), Some("org-vlm-1"));
        let user_text = body
            .get("messages")
            .and_then(Value::as_array)
            .unwrap_or_else(|| panic!("messages"))
            .first()
            .and_then(Value::as_object)
            .and_then(|m| m.get("content"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        assert!(user_text.contains("factory_layout_diff"));
    }

    #[tokio::test]
    async fn rate_limited_maps_to_retryable_error_then_recovers() {
        let (provider, _) = open_ai_provider(vec![429]);
        let schema = crate::schema::layout_diff_schema();
        let err = provider.chat_json(layout_req(), &schema).await.unwrap_err();
        assert!(err.is_retryable(), "429 must be retryable: {err}");
        // Next call gets the 200 fallback.
        assert!(provider.chat_json(layout_req(), &schema).await.is_ok());
    }

    #[tokio::test]
    async fn server_error_maps_to_retryable_provider_error() {
        let (provider, _) = open_ai_provider(vec![503]);
        let schema = crate::schema::layout_diff_schema();
        let err = provider.chat_json(layout_req(), &schema).await.unwrap_err();
        assert!(err.is_retryable(), "5xx must be retryable: {err}");
        assert!(matches!(err, VlmError::Provider { status: 503, .. }));
    }

    #[tokio::test]
    async fn schema_violation_fails_closed() {
        struct Bad200;
        #[async_trait]
        impl HttpTransport for Bad200 {
            async fn post_json(
                &self,
                _url: &str,
                _h: &[(String, String)],
                _b: &Value,
            ) -> Result<HttpResponse, String> {
                Ok(HttpResponse {
                    status: 200,
                    body: r#"{"choices":[{"message":{"content":"{\"nonsense\": 1}"}}]}"#
                        .to_string(),
                })
            }
        }
        let provider = CloudProvider::new(
            ProviderKind::OpenAi,
            "bad",
            "https://x",
            "m",
            "k",
            Arc::new(Bad200),
        )
        .unwrap_or_else(|e| panic!("provider: {e}"));
        let err = provider
            .chat_json(layout_req(), &crate::schema::layout_diff_schema())
            .await
            .unwrap_err();
        assert!(matches!(err, VlmError::Schema(_)), "schema enforced: {err}");
    }

    #[tokio::test]
    async fn empty_api_key_is_rejected_before_any_http() {
        match CloudProvider::new(
            ProviderKind::OpenAi,
            "nokey",
            "https://x",
            "m",
            "",
            Arc::new(ScriptedHttp::new(vec![])),
        ) {
            Err(e) => assert!(matches!(e, VlmError::Auth(_))),
            Ok(_) => panic!("empty key must be rejected"),
        }
    }

    #[tokio::test]
    async fn mock_provider_orders_scripted_results_then_falls_back() {
        let mock = MockProvider::new("scripted");
        mock.plan(vec![
            Err(VlmError::Http("net down".into())),
            Ok(serde_json::json!({"a": 1})),
        ]);
        let schema = JsonSchemaSpec::new("any", serde_json::json!({"type": "object"}));
        assert!(
            mock.chat_json(VlmRequest::new("t", json!({})), &schema)
                .await
                .is_err()
        );
        let v2 = mock
            .chat_json(VlmRequest::new("t", json!({})), &schema)
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(v2, json!({"a": 1}));
        mock.respond_with("root_cause_localization", json!({"planned": true}));
        let v3 = mock
            .chat_json(
                VlmRequest::new("root_cause_localization", json!({})),
                &schema,
            )
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(v3, json!({"planned": true}));
    }
}
