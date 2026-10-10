//! `TutorialProvider` — the LLM calling seam of the tutorial module
//! (D-0004 / D-0007).
//!
//! The first and currently only implementation,
//! [`OpenAiCompatProvider`], speaks the OpenAI-compatible
//! chat-completions protocol (`POST {base_url}/chat/completions`).
//! That one protocol covers OpenAI, DeepSeek, GLM, Kimi, ... and a local
//! `llama-server`, so switching provider is a config change, not a code
//! change.
//!
//! House rules mirrored from `crate::bot::api::ApiClient` (the MJOT
//! client this module deliberately imitates):
//! - one `reqwest::Client` per provider instance, held and reused —
//!   never one per request (each would pay a fresh TCP + TLS handshake);
//! - per-request timeout taken from `LlmConfig::effective_timeout`
//!   (already clamped there);
//! - bearer auth; **the key never appears in errors or logs** — error
//!   paths only carry the endpoint path and the server's status/body.

use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::config::LlmConfig;

/// One chat message. `role` is `"system"` / `"user"` / `"assistant"`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system".into(),
            content: content.into(),
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".into(),
            content: content.into(),
        }
    }
}

/// A provider-independent completion request. Field semantics follow the
/// OpenAI shape; providers that lack a knob simply ignore it.
#[derive(Debug, Clone, Default)]
pub struct ChatRequest {
    pub messages: Vec<ChatMessage>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
}

/// Token accounting, when the provider reports it (cost awareness).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChatUsage {
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
}

/// A provider-independent completion result.
#[derive(Debug, Clone, Default)]
pub struct ChatResponse {
    /// The assistant message content (no reasoning traces).
    pub content: String,
    /// Model id that actually served the request, when reported.
    pub model: Option<String>,
    pub usage: Option<ChatUsage>,
}

/// The swappable LLM seam. Implementations live behind this trait so the
/// review pipeline never names a vendor. The M1 pipeline calls
/// `complete` only after the upload-zone gate and the store cache
/// checked out.
// `async_trait` stamps a message-less `#[must_use]` on the boxed future
// it generates; the future is `must_use` already, which newer clippy
// (`double_must_use`) flags — nothing we can change from call sites.
#[allow(clippy::double_must_use)]
#[async_trait]
pub trait TutorialProvider: Send + Sync {
    /// Stable short name for logs / the stored review doc.
    fn name(&self) -> &'static str;

    /// Run one chat completion. Errors are for the caller to degrade on
    /// (numbers-only review) — never retry blindly: a slow or failing
    /// provider is the user's signal to check their config.
    async fn complete(&self, req: ChatRequest) -> Result<ChatResponse>;
}

/// OpenAI-compatible cloud client (D-0007's first provider). Built from
/// [`LlmConfig`]; not constructible while the config is incomplete —
/// callers gate on `TutorialConfig::outbound_allowed` first.
pub struct OpenAiCompatProvider {
    base: String,
    key: String,
    model: String,
    timeout: std::time::Duration,
    http: reqwest::Client,
}

impl OpenAiCompatProvider {
    /// Build a provider for a fully-configured [`LlmConfig`]. Fails only
    /// if the HTTP client cannot be built.
    pub fn new(cfg: &LlmConfig) -> Result<Self> {
        Ok(Self {
            base: normalize_base(&cfg.base_url),
            key: cfg.effective_api_key(),
            model: cfg.model.trim().to_string(),
            timeout: cfg.effective_timeout(),
            http: reqwest::Client::new(),
        })
    }
}

#[async_trait]
impl TutorialProvider for OpenAiCompatProvider {
    fn name(&self) -> &'static str {
        "openai-compat"
    }

    async fn complete(&self, req: ChatRequest) -> Result<ChatResponse> {
        let url = format!("{}/chat/completions", self.base);
        let body = OpenAiChatRequest {
            model: &self.model,
            messages: &req.messages,
            temperature: req.temperature,
            max_tokens: req.max_tokens,
        };
        let resp = self
            .http
            .post(&url)
            .timeout(self.timeout)
            .bearer_auth(&self.key)
            .json(&body)
            .send()
            .await
            .context("POST /chat/completions")?;
        if !resp.status().is_success() {
            let status = resp.status();
            // Server-authored body only; our request (which carries the
            // key) is never echoed into the error.
            let body = resp.text().await.unwrap_or_default();
            let snippet = truncate(&body, 300);
            bail!("chat completions failed: {status}: {snippet}");
        }
        let parsed: OpenAiChatResponse = resp
            .json()
            .await
            .context("parse /chat/completions response")?;
        let Some(choice) = parsed.choices.into_iter().next() else {
            bail!("chat completions returned no choices");
        };
        let content = choice.message.and_then(|m| m.content).unwrap_or_default();
        if content.trim().is_empty() {
            bail!("chat completions returned an empty message");
        }
        Ok(ChatResponse {
            content,
            model: parsed.model,
            usage: parsed.usage.map(|u| ChatUsage {
                prompt_tokens: u.prompt_tokens,
                completion_tokens: u.completion_tokens,
            }),
        })
    }
}

/// Tolerate trailing slashes / surrounding whitespace in the configured
/// base URL so `https://api.example.com/v1/` and `.../v1` behave alike.
fn normalize_base(base_url: &str) -> String {
    base_url.trim().trim_end_matches('/').to_string()
}

fn truncate(s: &str, max: usize) -> &str {
    match s.char_indices().nth(max) {
        Some((idx, _)) => &s[..idx],
        None => s,
    }
}

// --- wire types (serde mirrors of the OpenAI chat schema, subset) ---

#[derive(Serialize)]
struct OpenAiChatRequest<'a> {
    model: &'a str,
    messages: &'a [ChatMessage],
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
}

#[derive(Deserialize)]
struct OpenAiChatResponse {
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    choices: Vec<OpenAiChoice>,
    #[serde(default)]
    usage: Option<OpenAiUsage>,
}

#[derive(Deserialize)]
struct OpenAiChoice {
    #[serde(default)]
    message: Option<OpenAiMessage>,
}

#[derive(Deserialize)]
struct OpenAiMessage {
    #[serde(default)]
    content: Option<String>,
}

#[derive(Deserialize)]
struct OpenAiUsage {
    #[serde(default)]
    prompt_tokens: Option<u64>,
    #[serde(default)]
    completion_tokens: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The wire response must parse into the provider-independent shape
    /// even when optional bits (usage, model) are absent — providers
    /// vary in what they echo back. No network involved.
    #[test]
    fn parses_minimal_openai_response() {
        let raw = r#"{
            "choices": [ { "message": { "role": "assistant", "content": "你好" } } ]
        }"#;
        let parsed: OpenAiChatResponse = serde_json::from_str(raw).unwrap();
        let content = parsed
            .choices
            .into_iter()
            .next()
            .unwrap()
            .message
            .and_then(|m| m.content)
            .unwrap();
        assert_eq!(content, "你好");
        assert!(parsed.model.is_none());
        assert!(parsed.usage.is_none());
    }

    #[test]
    fn normalizes_base_url_slashes() {
        assert_eq!(
            normalize_base("  https://api.example.com/v1/ "),
            "https://api.example.com/v1"
        );
    }

    #[test]
    fn truncates_on_char_boundary() {
        assert_eq!(truncate("abcdef", 3), "abc");
        // Multibyte characters must not be split mid-codepoint.
        assert_eq!(truncate("你好世界", 2), "你好");
    }
}
