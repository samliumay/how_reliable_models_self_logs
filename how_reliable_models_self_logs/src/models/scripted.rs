//! Scripted backend: replays canned replies from a JSON file. For tests and dry runs only;
//! the runner refuses it outside the `smoke` phase. Reply `n` is returned for the model's
//! `n`-th turn (counted from the assistant messages already in the conversation), so one
//! script serves every episode, concurrently.
//!
//! ```json
//! {"replies": [
//!   {"reasoning": "...", "tool_calls": [{"name": "list_inbox", "args": {}}]},
//!   {"content": "Done.", "finish_reason": "stop"}
//! ]}
//! ```
//! `finish_reason` defaults to `tool_calls` when there are calls, else `stop`.

use std::path::Path;

use anyhow::{Context, Result, anyhow};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{CallError, Message, Reply, Request, ToolCall, turn_status};
use crate::util::hash::sha256_hex;

/// A canned tool call.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScriptedCall {
    /// Tool name.
    pub name: String,
    /// Arguments (any JSON; non-objects are sent as unparseable argument text).
    #[serde(default = "empty_object")]
    pub args: Value,
}

/// `{}`.
fn empty_object() -> Value {
    json!({})
}

/// A canned reply.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScriptedReply {
    /// Visible text.
    pub content: Option<String>,
    /// Reasoning trace.
    pub reasoning: Option<String>,
    /// Tool calls.
    #[serde(default)]
    pub tool_calls: Vec<ScriptedCall>,
    /// Finish reason (`stop`, `tool_calls`, `length`, `error`).
    pub finish_reason: Option<String>,
}

/// The script file.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScriptFile {
    /// Replies, one per model turn.
    replies: Vec<ScriptedReply>,
}

/// A loaded script.
#[derive(Debug, Clone)]
pub struct Script {
    /// Replies, one per model turn.
    pub replies: Vec<ScriptedReply>,
    /// sha256 of the file: the "pin" recorded with a run.
    pub sha256: String,
}

impl Script {
    /// Read a script file.
    pub fn load(path: &Path) -> Result<Script> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading script {}", path.display()))?;
        let f: ScriptFile = serde_json::from_str(&text)
            .with_context(|| format!("parsing script {}", path.display()))?;
        Ok(Script {
            replies: f.replies,
            sha256: sha256_hex(text.as_bytes()),
        })
    }

    /// A script from replies (tests).
    pub fn from_replies(replies: Vec<ScriptedReply>) -> Script {
        Script {
            replies,
            sha256: "inline".into(),
        }
    }

    /// The reply for the next turn; an exhausted script is a permanent error.
    pub fn complete(&self, req: &Request<'_>) -> Result<Reply, CallError> {
        let n = req
            .messages
            .iter()
            .filter(|m| matches!(m, Message::Assistant { .. }))
            .count();
        let r = self.replies.get(n).ok_or_else(|| {
            CallError::Permanent(anyhow!("script has no reply for turn {}", n + 1))
        })?;
        let finish = r.finish_reason.clone().unwrap_or_else(|| {
            if r.tool_calls.is_empty() {
                "stop".into()
            } else {
                "tool_calls".into()
            }
        });
        let status = turn_status(Some(&finish));
        let tool_calls = r
            .tool_calls
            .iter()
            .enumerate()
            .map(|(i, c)| match &c.args {
                Value::Object(_) => ToolCall {
                    id: format!("scripted_{n}_{i}"),
                    name: c.name.clone(),
                    args: c.args.clone(),
                    raw_args: None,
                },
                other => ToolCall::from_arg_text(
                    format!("scripted_{n}_{i}"),
                    c.name.clone(),
                    &match other {
                        Value::String(s) => s.clone(),
                        v => v.to_string(),
                    },
                ),
            })
            .collect();
        Ok(Reply {
            status,
            content: r.content.clone(),
            reasoning: r.reasoning.clone(),
            reasoning_details: None,
            tool_calls,
            error: match status {
                crate::types::TurnStatus::Failed => Some(format!("finish_reason {finish:?}")),
                crate::types::TurnStatus::Ok | crate::types::TurnStatus::Truncated => None,
            },
            finish_reason: Some(finish),
            served_by: Some("scripted".into()),
            api_model: Some("scripted".into()),
            prompt_tokens: Some(0),
            completion_tokens: Some(0),
            reasoning_tokens: None,
            cost_usd: Some(0.0),
            raw: json!({"scripted_turn": n}),
        })
    }
}
