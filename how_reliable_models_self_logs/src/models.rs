//! Subject models: the conversation types, what every model API returns, and one enum over
//! the APIs, so the agent loop does not care which one serves a model. One file per API in
//! `models/`. Models act through native tool calling (OpenAI-style `tools`).

pub mod ollama;
pub mod openrouter;
pub mod scripted;

use std::time::Duration;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::config::{Config, ModelConfig, ModelSource};
use crate::tools::ToolSpec;
use crate::types::{Api, TurnStatus};
use crate::util::text::truncate;

/// A tool call the model made.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    /// Call id from the API; the agent loop fills in `call_<seq>` when the API gives none.
    pub id: String,
    /// Tool name.
    pub name: String,
    /// Arguments; `null` when the model's argument text is not a JSON object.
    pub args: Value,
    /// The raw argument text, when it did not parse.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_args: Option<String>,
}

impl ToolCall {
    /// A call from an argument string (OpenAI style): parsed if it is a JSON object.
    pub fn from_arg_text(id: String, name: String, text: &str) -> ToolCall {
        let text = if text.trim().is_empty() { "{}" } else { text };
        match serde_json::from_str::<Value>(text) {
            Ok(v @ Value::Object(_)) => ToolCall {
                id,
                name,
                args: v,
                raw_args: None,
            },
            _ => ToolCall {
                id,
                name,
                args: Value::Null,
                raw_args: Some(text.to_string()),
            },
        }
    }
}

/// One message of the conversation. Stored as the episode transcript (JSON).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "snake_case")]
pub enum Message {
    /// The scenario's system prompt.
    System {
        /// Text.
        content: String,
    },
    /// The task (and any later user text).
    User {
        /// Text.
        content: String,
    },
    /// A model turn.
    Assistant {
        /// Visible text.
        content: Option<String>,
        /// Raw reasoning trace.
        reasoning: Option<String>,
        /// Tool calls, in order.
        tool_calls: Vec<ToolCall>,
        /// OpenRouter's structured reasoning, sent back unchanged on the next call.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reasoning_details: Option<Value>,
    },
    /// A tool result.
    Tool {
        /// The call it answers.
        call_id: String,
        /// Tool name.
        name: String,
        /// What the model is shown.
        content: String,
    },
}

/// One chat call to a subject model.
pub struct Request<'a> {
    /// Model id at its API.
    pub model: &'a str,
    /// OpenRouter: pinned provider tag. Ollama, scripted: unused (checked before the run).
    pub pin: &'a str,
    /// The conversation so far.
    pub messages: &'a [Message],
    /// Tools offered.
    pub tools: &'a [ToolSpec],
    /// Sampling temperature.
    pub temperature: f64,
    /// Output budget, reasoning included.
    pub max_tokens: u32,
    /// Seed, if the run sends one.
    pub seed: Option<u64>,
}

/// What one call returned. `status` comes from the finish reason: `stop`/`tool_calls` → ok,
/// `length` → truncated, anything else (incl. `error`) → failed.
#[derive(Debug, Clone)]
pub struct Reply {
    /// Ok, truncated or failed.
    pub status: TurnStatus,
    /// Visible text.
    pub content: Option<String>,
    /// Raw reasoning trace.
    pub reasoning: Option<String>,
    /// OpenRouter's structured reasoning (sent back on the next call).
    pub reasoning_details: Option<Value>,
    /// Tool calls, in order.
    pub tool_calls: Vec<ToolCall>,
    /// Finish reason as the API reported it.
    pub finish_reason: Option<String>,
    /// Provider that served the call.
    pub served_by: Option<String>,
    /// Model id the API reported back.
    pub api_model: Option<String>,
    /// Input tokens.
    pub prompt_tokens: Option<i32>,
    /// Output tokens, reasoning included.
    pub completion_tokens: Option<i32>,
    /// Reasoning tokens, where reported.
    pub reasoning_tokens: Option<i32>,
    /// Cost in USD, where reported.
    pub cost_usd: Option<f64>,
    /// Why the call failed, if it did.
    pub error: Option<String>,
    /// The full response body.
    pub raw: Value,
}

/// Why a call produced no reply.
#[derive(Debug)]
pub enum CallError {
    /// HTTP 429: wait for a free slot, then try again (long backoff, many tries).
    RateLimited(anyhow::Error),
    /// Network trouble, timeouts, 5xx, malformed bodies: try again a few times.
    Retryable(anyhow::Error),
    /// A 4xx other than 408/429, or an exhausted script: the request itself is wrong.
    Permanent(anyhow::Error),
}

impl std::fmt::Display for CallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CallError::RateLimited(e) | CallError::Retryable(e) => write!(f, "{e:#}"),
            CallError::Permanent(e) => write!(f, "{e:#} (not retried)"),
        }
    }
}

/// Classify a non-success HTTP status.
pub fn http_error(status: reqwest::StatusCode, body: &str) -> CallError {
    let err = anyhow::anyhow!("HTTP {status}: {}", truncate(body, 300));
    if status.as_u16() == 429 {
        CallError::RateLimited(err)
    } else if status.is_server_error() || status.as_u16() == 408 {
        CallError::Retryable(err)
    } else {
        CallError::Permanent(err)
    }
}

/// The tools in the OpenAI function format (used by OpenRouter and Ollama).
pub fn tools_json(tools: &[ToolSpec]) -> Value {
    Value::Array(
        tools
            .iter()
            .map(|t| {
                json!({"type": "function", "function": {
                    "name": t.name, "description": t.description, "parameters": t.parameters}})
            })
            .collect(),
    )
}

/// Map a finish reason: `stop` / `tool_calls` → ok, `length` → truncated, else failed.
pub fn turn_status(finish_reason: Option<&str>) -> TurnStatus {
    match finish_reason {
        Some("stop" | "tool_calls") => TurnStatus::Ok,
        Some("length") => TurnStatus::Truncated,
        _ => TurnStatus::Failed,
    }
}

/// The client for a model's API.
pub enum Backend {
    /// OpenRouter, provider pinned.
    OpenRouter(openrouter::Client),
    /// Ollama, digest pinned.
    Ollama(ollama::Client),
    /// Canned replies (tests, dry runs).
    Scripted(scripted::Script),
}

impl Backend {
    /// The client for a model. API keys are read only when needed.
    pub fn new(cfg: &Config, m: &ModelConfig) -> Result<Backend> {
        let timeout = Duration::from_secs(cfg.run.request_timeout_s);
        Ok(match &m.source {
            ModelSource::OpenRouter { .. } => {
                let key = std::env::var("OPENROUTER_API_KEY").map_err(|_| {
                    anyhow::anyhow!("OPENROUTER_API_KEY is not set (add it to ../.env)")
                })?;
                Backend::OpenRouter(openrouter::Client::new(key, timeout)?)
            }
            ModelSource::Ollama { .. } => {
                Backend::Ollama(ollama::Client::new(&cfg.ollama.base_url, timeout)?)
            }
            ModelSource::Scripted { script } => Backend::Scripted(scripted::Script::load(script)?),
        })
    }

    /// Which API this client talks to.
    pub fn api(&self) -> Api {
        match self {
            Backend::OpenRouter(_) => Api::OpenRouter,
            Backend::Ollama(_) => Api::Ollama,
            Backend::Scripted(_) => Api::Scripted,
        }
    }

    /// Check that the pinned endpoint (OpenRouter) or digest (Ollama) is what will serve
    /// the model, before a run starts. Returns the pin to record with the run (for a
    /// script: its sha256).
    pub async fn check(&self, m: &ModelConfig, max_tokens: u32) -> Result<String> {
        match (self, &m.source) {
            (Backend::OpenRouter(c), ModelSource::OpenRouter { provider }) => {
                c.check_endpoint(&m.id, provider, max_tokens).await?;
                Ok(provider.clone())
            }
            (Backend::Ollama(c), ModelSource::Ollama { digest }) => {
                c.check_digest(&m.id, digest).await?;
                Ok(digest.clone())
            }
            (Backend::Scripted(s), ModelSource::Scripted { .. }) => Ok(s.sha256.clone()),
            (Backend::OpenRouter(_), _) | (Backend::Ollama(_), _) | (Backend::Scripted(_), _) => {
                anyhow::bail!("{}: backend does not match the model's api", m.id)
            }
        }
    }

    /// Send one chat call.
    pub async fn complete(&self, req: &Request<'_>) -> Result<Reply, CallError> {
        match self {
            Backend::OpenRouter(c) => c.complete(req).await,
            Backend::Ollama(c) => c.complete(req).await,
            Backend::Scripted(s) => s.complete(req),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::StatusCode;

    #[test]
    fn http_errors_are_classified() {
        assert!(matches!(
            http_error(StatusCode::TOO_MANY_REQUESTS, ""),
            CallError::RateLimited(_)
        ));
        assert!(matches!(
            http_error(StatusCode::BAD_GATEWAY, ""),
            CallError::Retryable(_)
        ));
        assert!(matches!(
            http_error(StatusCode::NOT_FOUND, ""),
            CallError::Permanent(_)
        ));
    }

    #[test]
    fn argument_text_that_is_not_an_object_is_kept_raw() {
        let ok = ToolCall::from_arg_text("1".into(), "t".into(), r#"{"a": 1}"#);
        assert_eq!(ok.args, json!({"a": 1}));
        let empty = ToolCall::from_arg_text("1".into(), "t".into(), "");
        assert_eq!(empty.args, json!({}));
        let bad = ToolCall::from_arg_text("1".into(), "t".into(), "{\"a\": ");
        assert!(bad.args.is_null());
        assert_eq!(bad.raw_args.as_deref(), Some("{\"a\": "));
    }

    #[test]
    fn finish_reasons_map_to_turn_status() {
        assert_eq!(turn_status(Some("tool_calls")), TurnStatus::Ok);
        assert_eq!(turn_status(Some("stop")), TurnStatus::Ok);
        assert_eq!(turn_status(Some("length")), TurnStatus::Truncated);
        assert_eq!(turn_status(Some("error")), TurnStatus::Failed);
        assert_eq!(turn_status(None), TurnStatus::Failed);
    }

    #[test]
    fn transcript_round_trips_as_json() {
        let m = vec![
            Message::System {
                content: "s".into(),
            },
            Message::Assistant {
                content: None,
                reasoning: Some("r".into()),
                tool_calls: vec![ToolCall {
                    id: "c1".into(),
                    name: "list_inbox".into(),
                    args: json!({}),
                    raw_args: None,
                }],
                reasoning_details: None,
            },
            Message::Tool {
                call_id: "c1".into(),
                name: "list_inbox".into(),
                content: "empty".into(),
            },
        ];
        let v = serde_json::to_value(&m).unwrap();
        assert_eq!(v[1]["role"], "assistant");
        let back: Vec<Message> = serde_json::from_value(v).unwrap();
        assert_eq!(back, m);
    }
}
