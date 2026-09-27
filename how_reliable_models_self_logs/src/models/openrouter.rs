//! OpenRouter chat client: native tool calling (OpenAI format), reasoning returned, provider
//! pinned with fallbacks off. The previous turns' `reasoning_details` are sent back
//! unchanged, as OpenRouter asks for tool-calling loops with reasoning models.

use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};

use super::{CallError, Message, Reply, Request, ToolCall, http_error, tools_json, turn_status};
use crate::types::TurnStatus;

/// Chat completions endpoint.
const URL: &str = "https://openrouter.ai/api/v1/chat/completions";

/// OpenRouter client.
pub struct Client {
    /// HTTP client with the run's timeout.
    http: reqwest::Client,
    /// `OPENROUTER_API_KEY`.
    api_key: String,
}

/// The conversation in OpenAI chat format.
pub fn messages_json(messages: &[Message]) -> Value {
    Value::Array(
        messages
            .iter()
            .map(|m| match m {
                Message::System { content } => json!({"role": "system", "content": content}),
                Message::User { content } => json!({"role": "user", "content": content}),
                Message::Assistant {
                    content,
                    tool_calls,
                    reasoning_details,
                    ..
                } => {
                    let mut v = json!({"role": "assistant", "content": content.clone().unwrap_or_default()});
                    if !tool_calls.is_empty() {
                        v["tool_calls"] = Value::Array(
                            tool_calls
                                .iter()
                                .map(|c| {
                                    let args = c
                                        .raw_args
                                        .clone()
                                        .unwrap_or_else(|| c.args.to_string());
                                    json!({"id": c.id, "type": "function",
                                           "function": {"name": c.name, "arguments": args}})
                                })
                                .collect(),
                        );
                    }
                    if let Some(d) = reasoning_details {
                        v["reasoning_details"] = d.clone();
                    }
                    v
                }
                Message::Tool {
                    call_id, content, ..
                } => json!({"role": "tool", "tool_call_id": call_id, "content": content}),
            })
            .collect(),
    )
}

impl Client {
    /// A client for one API key.
    pub fn new(api_key: String, timeout: Duration) -> Result<Client> {
        let http = reqwest::Client::builder().timeout(timeout).build()?;
        Ok(Client { http, api_key })
    }

    /// Check before a run that the pinned endpoint serves the model, allows `max_tokens`
    /// of output and supports tools; otherwise every call would be refused.
    pub async fn check_endpoint(&self, model: &str, provider: &str, max_tokens: u32) -> Result<()> {
        let url = format!("https://openrouter.ai/api/v1/models/{model}/endpoints");
        let raw: Value = self
            .http
            .get(url)
            .bearer_auth(&self.api_key)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let endpoints = raw
            .pointer("/data/endpoints")
            .and_then(Value::as_array)
            .context("no endpoint list")?;
        let cap = |e: &Value| e.get("max_completion_tokens").and_then(Value::as_u64);
        let tools = |e: &Value| {
            e.get("supported_parameters")
                .and_then(Value::as_array)
                .is_none_or(|p| p.iter().any(|x| x.as_str() == Some("tools")))
        };
        let fits: Vec<&str> = endpoints
            .iter()
            .filter(|e| cap(e).is_none_or(|c| c >= u64::from(max_tokens)) && tools(e))
            .filter_map(|e| e.get("tag").and_then(Value::as_str))
            .collect();
        match endpoints
            .iter()
            .find(|e| e.get("tag").and_then(Value::as_str) == Some(provider))
        {
            None => bail!(
                "{model}: provider {provider:?} does not serve it (endpoints that fit: {})",
                fits.join(", ")
            ),
            Some(e) if cap(e).is_some_and(|c| c < u64::from(max_tokens)) => bail!(
                "{model}: {provider} allows at most {} output tokens, config asks for {max_tokens} (endpoints that fit: {})",
                cap(e).unwrap_or(0),
                fits.join(", ")
            ),
            Some(e) if !tools(e) => bail!(
                "{model}: {provider} does not support tool calling (endpoints that fit: {})",
                fits.join(", ")
            ),
            Some(_) => Ok(()),
        }
    }

    /// One HTTP call. A model-side failure comes back as `Ok` with status `failed`.
    pub async fn complete(&self, req: &Request<'_>) -> Result<Reply, CallError> {
        let mut body = json!({
            "model": req.model,
            "messages": messages_json(req.messages),
            "tools": tools_json(req.tools),
            "temperature": req.temperature,
            "max_tokens": req.max_tokens,
            "reasoning": {"enabled": true},
            "provider": {"order": [req.pin], "allow_fallbacks": false},
            "usage": {"include": true},
        });
        if let Some(seed) = req.seed {
            body["seed"] = json!(seed);
        }
        let resp = self
            .http
            .post(URL)
            .bearer_auth(&self.api_key)
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

/// Turn a response body into a `Reply`. A body with an `error` object and no choices is an
/// `Err` (retryable); a choice that finished with an error is a `failed` reply.
pub fn parse_reply(raw: Value) -> Result<Reply> {
    if raw.get("choices").is_none() {
        let msg = raw
            .pointer("/error/message")
            .and_then(Value::as_str)
            .unwrap_or("no choices in response");
        return Err(anyhow!("provider error: {msg}"));
    }
    let choice = raw.pointer("/choices/0").context("empty choices")?;
    let text = |p: &str| choice.pointer(p).and_then(Value::as_str).map(str::to_owned);
    let finish_reason = text("/finish_reason");
    let status = turn_status(finish_reason.as_deref());
    let error = match status {
        TurnStatus::Failed => Some(
            choice
                .pointer("/error/message")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .unwrap_or_else(|| format!("finish_reason {finish_reason:?}")),
        ),
        TurnStatus::Ok | TurnStatus::Truncated => None,
    };
    let tool_calls = choice
        .pointer("/message/tool_calls")
        .and_then(Value::as_array)
        .map(|calls| {
            calls
                .iter()
                .map(|c| {
                    let s = |p: &str| c.pointer(p).and_then(Value::as_str).unwrap_or("");
                    // Some providers send the arguments as an object instead of a string.
                    let args = match c.pointer("/function/arguments") {
                        Some(Value::String(t)) => t.clone(),
                        Some(v @ Value::Object(_)) => v.to_string(),
                        _ => String::new(),
                    };
                    ToolCall::from_arg_text(s("/id").into(), s("/function/name").into(), &args)
                })
                .collect()
        })
        .unwrap_or_default();
    let int = |p: &str| {
        raw.pointer(p)
            .and_then(Value::as_i64)
            .and_then(|v| i32::try_from(v).ok())
    };
    Ok(Reply {
        status,
        content: text("/message/content").filter(|s| !s.is_empty()),
        reasoning: text("/message/reasoning"),
        reasoning_details: choice
            .pointer("/message/reasoning_details")
            .filter(|v| !v.is_null())
            .cloned(),
        tool_calls,
        finish_reason,
        served_by: raw
            .get("provider")
            .and_then(Value::as_str)
            .map(str::to_owned),
        api_model: raw.get("model").and_then(Value::as_str).map(str::to_owned),
        prompt_tokens: int("/usage/prompt_tokens"),
        completion_tokens: int("/usage/completion_tokens"),
        reasoning_tokens: int("/usage/completion_tokens_details/reasoning_tokens"),
        cost_usd: raw.pointer("/usage/cost").and_then(Value::as_f64),
        error,
        raw,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(finish: &str) -> Value {
        json!({
            "model": "qwen/qwen3.8-27b", "provider": "DeepInfra",
            "choices": [{"finish_reason": finish,
                         "message": {"content": "", "reasoning": "I should check the inbox",
                                     "reasoning_details": [{"type": "reasoning.text", "text": "..."}],
                                     "tool_calls": [
                                        {"id": "call_a", "type": "function",
                                         "function": {"name": "read_email", "arguments": "{\"id\": 3}"}},
                                        {"id": "call_b", "type": "function",
                                         "function": {"name": "send_email", "arguments": "{\"to\": [\"a@b"}}
                                     ]}}],
            "usage": {"prompt_tokens": 10, "completion_tokens": 20, "cost": 0.001,
                      "completion_tokens_details": {"reasoning_tokens": 15}}
        })
    }

    #[test]
    fn tool_calls_are_parsed_with_raw_text_kept_when_broken() {
        let r = parse_reply(body("tool_calls")).unwrap();
        assert_eq!(r.status, TurnStatus::Ok);
        assert_eq!(r.content, None);
        assert_eq!(r.tool_calls.len(), 2);
        assert_eq!(r.tool_calls[0].id, "call_a");
        assert_eq!(r.tool_calls[0].args, json!({"id": 3}));
        assert!(r.tool_calls[1].args.is_null());
        assert!(r.tool_calls[1].raw_args.is_some());
        assert!(r.reasoning_details.is_some());
        assert_eq!(r.reasoning_tokens, Some(15));
    }

    #[test]
    fn finish_reasons() {
        assert_eq!(
            parse_reply(body("length")).unwrap().status,
            TurnStatus::Truncated
        );
        let failed = parse_reply(body("error")).unwrap();
        assert_eq!(failed.status, TurnStatus::Failed);
        assert!(failed.error.is_some());
        assert!(parse_reply(json!({"error": {"message": "rate limited"}})).is_err());
    }

    #[test]
    fn conversation_is_sent_in_openai_format() {
        let r = parse_reply(body("tool_calls")).unwrap();
        let msgs = vec![
            Message::User {
                content: "task".into(),
            },
            Message::Assistant {
                content: r.content,
                reasoning: r.reasoning,
                tool_calls: r.tool_calls,
                reasoning_details: r.reasoning_details,
            },
            Message::Tool {
                call_id: "call_a".into(),
                name: "read_email".into(),
                content: "hi".into(),
            },
        ];
        let v = messages_json(&msgs);
        assert_eq!(v[1]["tool_calls"][0]["function"]["arguments"], "{\"id\":3}");
        // Broken arguments go back exactly as the model wrote them.
        assert_eq!(
            v[1]["tool_calls"][1]["function"]["arguments"],
            "{\"to\": [\"a@b"
        );
        assert!(v[1]["reasoning_details"].is_array());
        assert_eq!(v[2]["tool_call_id"], "call_a");
    }
}
