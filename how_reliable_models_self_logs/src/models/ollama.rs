//! Ollama client (native `/api/chat`, local server; `:cloud` models are forwarded to
//! ollama.com by it) with native tool calling. Reasoning is requested with `think: true`,
//! comes back as `message.thinking`, and is sent back in later turns. A model is pinned by
//! its manifest digest.

use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};

use super::{CallError, Message, Reply, Request, ToolCall, http_error, tools_json, turn_status};
use crate::types::TurnStatus;

/// Ollama client.
pub struct Client {
    /// HTTP client with the run's timeout.
    http: reqwest::Client,
    /// Server address without a trailing slash.
    base_url: String,
}

/// The conversation in Ollama's chat format (arguments as objects, `tool_name` on results).
pub fn messages_json(messages: &[Message]) -> Value {
    Value::Array(
        messages
            .iter()
            .map(|m| match m {
                Message::System { content } => json!({"role": "system", "content": content}),
                Message::User { content } => json!({"role": "user", "content": content}),
                Message::Assistant {
                    content,
                    reasoning,
                    tool_calls,
                    ..
                } => {
                    let mut v = json!({"role": "assistant", "content": content.clone().unwrap_or_default()});
                    if let Some(r) = reasoning {
                        v["thinking"] = json!(r);
                    }
                    if !tool_calls.is_empty() {
                        v["tool_calls"] = Value::Array(
                            tool_calls
                                .iter()
                                .map(|c| {
                                    let args = if c.args.is_object() { c.args.clone() } else { json!({}) };
                                    json!({"function": {"name": c.name, "arguments": args}})
                                })
                                .collect(),
                        );
                    }
                    v
                }
                Message::Tool { name, content, .. } => {
                    json!({"role": "tool", "tool_name": name, "content": content})
                }
            })
            .collect(),
    )
}

impl Client {
    /// A client for one server.
    pub fn new(base_url: &str, timeout: Duration) -> Result<Client> {
        let http = reqwest::Client::builder().timeout(timeout).build()?;
        Ok(Client {
            http,
            base_url: base_url.trim_end_matches('/').to_string(),
        })
    }

    /// The model must be pulled and its manifest digest must equal the pinned one:
    /// Ollama tags are mutable, so a new digest means a different model.
    pub async fn check_digest(&self, model: &str, digest: &str) -> Result<()> {
        let raw: Value = self
            .http
            .get(format!("{}/api/tags", self.base_url))
            .send()
            .await
            .with_context(|| format!("is Ollama running at {}?", self.base_url))?
            .error_for_status()?
            .json()
            .await?;
        let models = raw
            .get("models")
            .and_then(Value::as_array)
            .context("no model list")?;
        let Some(found) = models
            .iter()
            .find(|m| m.get("name").and_then(Value::as_str) == Some(model))
        else {
            bail!("{model} is not pulled in Ollama (`ollama pull {model}`)");
        };
        let actual = found.get("digest").and_then(Value::as_str).unwrap_or("");
        if actual != digest {
            bail!(
                "{model}: digest is {actual}, config pins {digest}; the tag now points to a different model"
            );
        }
        Ok(())
    }

    /// Send one chat call with thinking and tools on.
    pub async fn complete(&self, req: &Request<'_>) -> Result<Reply, CallError> {
        let mut options = json!({"temperature": req.temperature, "num_predict": req.max_tokens});
        if let Some(seed) = req.seed {
            options["seed"] = json!(seed);
        }
        let body = json!({
            "model": req.model,
            "messages": messages_json(req.messages),
            "tools": tools_json(req.tools),
            "stream": false,
            "think": true,
            "options": options,
        });
        let resp = self
            .http
            .post(format!("{}/api/chat", self.base_url))
            .json(&body)
            .send()
            .await
            .map_err(|e| CallError::Retryable(e.into()))?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| CallError::Retryable(e.into()))?;
        if !status.is_success() {
            return Err(http_error(status, &text));
        }
        let raw: Value = serde_json::from_str(&text)
            .context("response is not JSON")
            .map_err(CallError::Retryable)?;
        parse_reply(raw).map_err(CallError::Retryable)
    }
}

/// `done_reason` maps like OpenRouter's finish reason. A body with `error` is an `Err`
/// (retried). Ollama gives calls no ids; the agent loop assigns them.
pub fn parse_reply(raw: Value) -> Result<Reply> {
    if let Some(e) = raw.get("error") {
        return Err(anyhow!("ollama error: {e}"));
    }
    if raw.get("done").and_then(Value::as_bool) != Some(true) {
        return Err(anyhow!("ollama reply not done"));
    }
    let text = |p: &str| raw.pointer(p).and_then(Value::as_str).map(str::to_owned);
    let finish_reason = text("/done_reason");
    let status = turn_status(finish_reason.as_deref());
    let error = match status {
        TurnStatus::Failed => Some(format!("done_reason {finish_reason:?}")),
        TurnStatus::Ok | TurnStatus::Truncated => None,
    };
    let tool_calls = raw
        .pointer("/message/tool_calls")
        .and_then(Value::as_array)
        .map(|calls| {
            calls
                .iter()
                .map(|c| {
                    let name = c
                        .pointer("/function/name")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    let id = c
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    match c.pointer("/function/arguments") {
                        Some(v @ Value::Object(_)) => ToolCall {
                            id,
                            name,
                            args: v.clone(),
                            raw_args: None,
                        },
                        Some(Value::String(t)) => ToolCall::from_arg_text(id, name, t),
                        other => ToolCall {
                            id,
                            name,
                            args: Value::Null,
                            raw_args: other.map(Value::to_string),
                        },
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    let int = |k: &str| {
        raw.get(k)
            .and_then(Value::as_i64)
            .and_then(|v| i32::try_from(v).ok())
    };
    Ok(Reply {
        status,
        content: text("/message/content").filter(|s| !s.is_empty()),
        reasoning: text("/message/thinking"),
        reasoning_details: None,
        tool_calls,
        finish_reason,
        served_by: Some("ollama".to_string()),
        api_model: text("/model"),
        prompt_tokens: int("prompt_eval_count"),
        completion_tokens: int("eval_count"),
        reasoning_tokens: None,
        cost_usd: None,
        error,
        raw,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(done_reason: &str) -> Value {
        json!({"model": "deepseek-v4.1-flash", "done": true, "done_reason": done_reason,
               "message": {"role": "assistant", "content": "", "thinking": "read it first",
                           "tool_calls": [{"function": {"name": "read_email", "arguments": {"id": 3}}}]},
               "prompt_eval_count": 38, "eval_count": 41})
    }

    #[test]
    fn tool_calls_with_object_arguments() {
        let r = parse_reply(body("stop")).unwrap();
        assert_eq!(r.status, TurnStatus::Ok);
        assert_eq!(r.tool_calls[0].name, "read_email");
        assert_eq!(r.tool_calls[0].args, json!({"id": 3}));
        assert_eq!(r.tool_calls[0].id, "");
        assert_eq!(r.reasoning.as_deref(), Some("read it first"));
        assert_eq!(r.content, None);
    }

    #[test]
    fn done_reasons_and_errors() {
        assert_eq!(
            parse_reply(body("length")).unwrap().status,
            TurnStatus::Truncated
        );
        assert_eq!(
            parse_reply(body("unload")).unwrap().status,
            TurnStatus::Failed
        );
        assert!(parse_reply(json!({"error": "model not found"})).is_err());
        assert!(parse_reply(json!({"done": false})).is_err());
    }

    #[test]
    fn history_uses_thinking_and_tool_name() {
        let r = parse_reply(body("stop")).unwrap();
        let v = messages_json(&[
            Message::Assistant {
                content: r.content,
                reasoning: r.reasoning,
                tool_calls: r.tool_calls,
                reasoning_details: None,
            },
            Message::Tool {
                call_id: "call_0".into(),
                name: "read_email".into(),
                content: "x".into(),
            },
        ]);
        assert_eq!(v[0]["thinking"], "read it first");
        assert_eq!(v[0]["tool_calls"][0]["function"]["arguments"]["id"], 3);
        assert_eq!(v[1]["tool_name"], "read_email");
    }
}
