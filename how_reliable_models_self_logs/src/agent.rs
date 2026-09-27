//! The multi-turn loop of one episode: model ↔ tools over the episode's [`World`].
//!
//! Messages: the scenario's system prompt, then its task as the user message; the tool specs
//! go with every call (native tool calling). Each turn:
//! 1. call the model (retries with backoff; HTTP 429 waits longer);
//! 2. a truncated reply ends the episode (`truncated`) and its tool calls are not run;
//! 3. a reply without tool calls ends it (`finished`; its text is the final answer);
//! 4. otherwise each call runs in order: one system-log row per call, the result appended as
//!    a tool message, `after_tool` events fired; then `after_turn` events fire.
//!
//! After `max_turns` turns the episode ends as `max_turns`. A call with no usable reply after
//! all attempts ends it as `failed`; everything up to then is kept.

use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::Value;

use crate::models::{Backend, CallError, Message, Reply, Request, ToolCall};
use crate::scenario::Scenario;
use crate::tools::{self, SystemLogEntry};
use crate::types::{Api, Arm, EpisodeStatus, TurnStatus};
use crate::world::{FiredEvent, World};

/// How the subject model is called (from `[run]` and the run's pin).
pub struct Settings<'a> {
    /// Model id at its API.
    pub model: &'a str,
    /// Provider tag or digest.
    pub pin: &'a str,
    /// Sampling temperature.
    pub temperature: f64,
    /// Output budget per call.
    pub max_tokens: u32,
    /// Seed sent with every call of the episode.
    pub seed: Option<u64>,
    /// Retries of one call after the first attempt.
    pub max_retries: u32,
}

/// One model turn as stored in `turns`.
#[derive(Debug, Clone, Serialize)]
pub struct TurnRecord {
    /// 1-based.
    pub turn: u32,
    /// Ok, truncated or failed.
    pub status: TurnStatus,
    /// Visible text.
    pub content: Option<String>,
    /// Reasoning trace.
    pub reasoning: Option<String>,
    /// Tool calls the model made (not run if the turn was truncated).
    pub tool_calls: Vec<ToolCall>,
    /// Finish reason as reported.
    pub finish_reason: Option<String>,
    /// Provider that served the call.
    pub served_by: Option<String>,
    /// Model id the API reported.
    pub api_model: Option<String>,
    /// Input tokens.
    pub prompt_tokens: Option<i32>,
    /// Output tokens.
    pub completion_tokens: Option<i32>,
    /// Reasoning tokens.
    pub reasoning_tokens: Option<i32>,
    /// Cost in USD.
    pub cost_usd: Option<f64>,
    /// Calls made for this turn.
    pub attempts: u32,
    /// Wall time over all attempts.
    pub duration_ms: i64,
    /// Why the turn failed, if it did.
    pub error: Option<String>,
    /// Last response body.
    pub raw: Value,
}

/// A finished episode, ready to store.
#[derive(Debug, Clone)]
pub struct Episode {
    /// How it ended.
    pub status: EpisodeStatus,
    /// Every model turn.
    pub turns: Vec<TurnRecord>,
    /// The system log: every tool call the harness ran.
    pub system_log: Vec<SystemLogEntry>,
    /// The full conversation.
    pub transcript: Vec<Message>,
    /// Text of the last reply without tool calls.
    pub final_answer: Option<String>,
    /// Why it failed, if it did.
    pub error: Option<String>,
    /// Events that fired.
    pub fired: Vec<FiredEvent>,
    /// Wall time.
    pub duration: Duration,
}

impl Episode {
    /// Model calls over all turns.
    pub fn attempts(&self) -> u32 {
        self.turns.iter().map(|t| t.attempts).sum()
    }

    /// Sum of an optional per-turn number; `None` if no turn reported it.
    pub fn sum<T: std::iter::Sum<T> + Copy>(
        &self,
        f: impl Fn(&TurnRecord) -> Option<T>,
    ) -> Option<T> {
        let v: Vec<T> = self.turns.iter().filter_map(&f).collect();
        (!v.is_empty()).then(|| v.into_iter().sum())
    }
}

/// Most waits for a free slot after HTTP 429.
const RATE_LIMIT_WAITS: u32 = 20;
/// Longest wait after HTTP 429, in seconds.
const RATE_LIMIT_MAX_WAIT_S: u64 = 300;

/// Sleep, except for the scripted backend (tests).
async fn wait(backend: &Backend, secs: u64) {
    match backend.api() {
        Api::Scripted => {}
        Api::OpenRouter | Api::Ollama => tokio::time::sleep(Duration::from_secs(secs)).await,
    }
}

/// One model call with retries. Errors and failed finishes are retried `max_retries` times
/// with exponential backoff; HTTP 429 waits 30 s doubling to 5 min and does not use up those
/// retries. Returns the last reply (if any), the error (if it never succeeded) and the
/// number of calls.
async fn call_with_retries(
    backend: &Backend,
    req: &Request<'_>,
    max_retries: u32,
) -> (Option<Reply>, Option<String>, u32) {
    let mut last_reply = None;
    let mut last_error: Option<String>;
    let (mut attempts, mut errors, mut waits) = (0u32, 0u32, 0u32);
    loop {
        attempts += 1;
        match backend.complete(req).await {
            Ok(reply) if reply.status != TurnStatus::Failed => {
                return (Some(reply), None, attempts);
            }
            Ok(reply) => {
                last_error = reply.error.clone().or(Some("failed finish".into()));
                last_reply = Some(reply);
            }
            Err(e @ CallError::Permanent(_)) => {
                last_error = Some(e.to_string());
                break;
            }
            Err(e @ CallError::RateLimited(_)) => {
                last_error = Some(e.to_string());
                if waits == RATE_LIMIT_WAITS {
                    break;
                }
                let secs = (30 * 2u64.pow(waits.min(4))).min(RATE_LIMIT_MAX_WAIT_S);
                waits += 1;
                wait(backend, secs).await;
                continue;
            }
            Err(e @ CallError::Retryable(_)) => last_error = Some(e.to_string()),
        }
        if errors == max_retries {
            break;
        }
        errors += 1;
        wait(backend, 2u64.pow(errors)).await;
    }
    (last_reply, last_error, attempts)
}

/// Run one episode of `scenario` in `arm`.
pub async fn run_episode(
    backend: &Backend,
    scenario: &Scenario,
    arm: Arm,
    s: &Settings<'_>,
) -> Episode {
    let start = Instant::now();
    let specs = scenario.tool_specs();
    let mut world = World::new(scenario, arm);
    let mut messages = vec![
        Message::System {
            content: scenario.system_prompt.clone(),
        },
        Message::User {
            content: scenario.task.clone(),
        },
    ];
    let mut turns = Vec::new();
    let mut log: Vec<SystemLogEntry> = Vec::new();
    let mut final_answer = None;
    let mut error = None;
    let mut status = EpisodeStatus::MaxTurns;

    for turn in 1..=scenario.max_turns {
        let t0 = Instant::now();
        let req = Request {
            model: s.model,
            pin: s.pin,
            messages: &messages,
            tools: &specs,
            temperature: s.temperature,
            max_tokens: s.max_tokens,
            seed: s.seed,
        };
        let (reply, err, attempts) = call_with_retries(backend, &req, s.max_retries).await;
        let record = |r: Option<&Reply>, err: Option<String>| TurnRecord {
            turn,
            status: r.map_or(TurnStatus::Failed, |r| {
                if err.is_some() {
                    TurnStatus::Failed
                } else {
                    r.status
                }
            }),
            content: r.and_then(|r| r.content.clone()),
            reasoning: r.and_then(|r| r.reasoning.clone()),
            tool_calls: r.map(|r| r.tool_calls.clone()).unwrap_or_default(),
            finish_reason: r.and_then(|r| r.finish_reason.clone()),
            served_by: r.and_then(|r| r.served_by.clone()),
            api_model: r.and_then(|r| r.api_model.clone()),
            prompt_tokens: r.and_then(|r| r.prompt_tokens),
            completion_tokens: r.and_then(|r| r.completion_tokens),
            reasoning_tokens: r.and_then(|r| r.reasoning_tokens),
            cost_usd: r.and_then(|r| r.cost_usd),
            attempts,
            duration_ms: t0.elapsed().as_millis() as i64,
            error: err,
            raw: r.map(|r| r.raw.clone()).unwrap_or(Value::Null),
        };
        let reply = match (reply, err) {
            (Some(reply), None) => reply,
            (reply, err) => {
                turns.push(record(reply.as_ref(), err.clone()));
                status = EpisodeStatus::Failed;
                error = Some(format!("turn {turn}: {}", err.unwrap_or_default()));
                break;
            }
        };
        // Give every call an id unique in the episode.
        let mut reply = reply;
        for (i, c) in reply.tool_calls.iter_mut().enumerate() {
            if c.id.is_empty() {
                c.id = format!("call_{}", log.len() + i);
            }
        }
        turns.push(record(Some(&reply), None));
        messages.push(Message::Assistant {
            content: reply.content.clone(),
            reasoning: reply.reasoning.clone(),
            tool_calls: reply.tool_calls.clone(),
            reasoning_details: reply.reasoning_details.clone(),
        });
        if reply.status == TurnStatus::Truncated {
            status = EpisodeStatus::Truncated;
            break;
        }
        if reply.tool_calls.is_empty() {
            status = EpisodeStatus::Finished;
            final_answer = reply.content.clone();
            break;
        }
        for call in &reply.tool_calls {
            let outcome = match (&call.raw_args, call.args.is_object()) {
                (Some(_), _) | (None, false) => {
                    Err("Error: arguments are not a valid JSON object".to_string())
                }
                (None, true) => tools::execute(&mut world, &call.name, &call.args),
            };
            let (result, is_error) = match outcome {
                Ok(r) => (r, false),
                Err(e) => (e, true),
            };
            log.push(SystemLogEntry {
                turn,
                seq: log.len() as u32,
                call_id: call.id.clone(),
                tool: call.name.clone(),
                args: call.args.clone(),
                raw_args: call.raw_args.clone(),
                result: result.clone(),
                is_error,
                side_effect: tools::spec(&call.name).is_some_and(|s| s.side_effect),
            });
            messages.push(Message::Tool {
                call_id: call.id.clone(),
                name: call.name.clone(),
                content: result,
            });
            if !is_error {
                world.after_tool(&call.name, turn);
            }
        }
        world.after_turn(turn);
    }

    Episode {
        status,
        turns,
        system_log: log,
        transcript: messages,
        final_answer,
        error,
        fired: world.fired,
        duration: start.elapsed(),
    }
}

/// The conversation as plain text for judges: every message in order, with reasoning,
/// tool calls and tool results.
pub fn render_transcript(messages: &[Message]) -> String {
    let mut out = String::new();
    let mut turn = 0;
    for m in messages {
        match m {
            Message::System { content } => {
                out.push_str(&format!("=== SYSTEM PROMPT ===\n{content}\n\n"))
            }
            Message::User { content } => out.push_str(&format!("=== USER ===\n{content}\n\n")),
            Message::Assistant {
                content,
                reasoning,
                tool_calls,
                ..
            } => {
                turn += 1;
                out.push_str(&format!("=== ASSISTANT (turn {turn}) ===\n"));
                if let Some(r) = reasoning.as_deref().filter(|r| !r.trim().is_empty()) {
                    out.push_str(&format!("[reasoning]\n{r}\n[/reasoning]\n"));
                }
                if let Some(c) = content.as_deref().filter(|c| !c.trim().is_empty()) {
                    out.push_str(&format!("[message]\n{c}\n[/message]\n"));
                }
                for c in tool_calls {
                    let args = c.raw_args.clone().unwrap_or_else(|| c.args.to_string());
                    out.push_str(&format!("[tool call {} ({})] {args}\n", c.name, c.id));
                }
                out.push('\n');
            }
            Message::Tool {
                call_id,
                name,
                content,
            } => out.push_str(&format!(
                "=== TOOL RESULT {name} ({call_id}) ===\n{content}\n\n"
            )),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::scripted::{Script, ScriptedCall, ScriptedReply};
    use crate::scenario::{self, Scenario};
    use serde_json::json;
    use std::path::PathBuf;

    fn fixture() -> Scenario {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/scenarios/fixture_refund");
        scenario::load_dir(&dir).unwrap()
    }

    fn settings() -> Settings<'static> {
        Settings {
            model: "scripted",
            pin: "inline",
            temperature: 1.0,
            max_tokens: 100,
            seed: Some(0),
            max_retries: 2,
        }
    }

    fn call(name: &str, args: Value) -> ScriptedCall {
        ScriptedCall {
            name: name.into(),
            args,
        }
    }

    fn calls(v: Vec<ScriptedCall>) -> ScriptedReply {
        ScriptedReply {
            tool_calls: v,
            ..Default::default()
        }
    }

    fn text(t: &str) -> ScriptedReply {
        ScriptedReply {
            content: Some(t.into()),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn tool_call_then_result_then_final_text() {
        let s = fixture();
        let backend = Backend::Scripted(Script::from_replies(vec![
            calls(vec![
                call("list_inbox", json!({})),
                call("read_email", json!({"id": 1})),
            ]),
            calls(vec![
                call("nope", json!({})),
                call("read_email", json!("{bad")),
            ]),
            text("All done."),
        ]));
        let ep = run_episode(&backend, &s, Arm::Pressure, &settings()).await;
        assert_eq!(ep.status, EpisodeStatus::Finished);
        assert_eq!(ep.final_answer.as_deref(), Some("All done."));
        assert_eq!(ep.turns.len(), 3);
        assert_eq!(ep.system_log.len(), 4);
        let seqs: Vec<u32> = ep.system_log.iter().map(|e| e.seq).collect();
        assert_eq!(seqs, vec![0, 1, 2, 3]);
        assert!(!ep.system_log[1].is_error);
        assert!(ep.system_log[2].is_error, "unknown tool");
        assert!(ep.system_log[3].is_error, "bad JSON");
        assert_eq!(ep.system_log[3].raw_args.as_deref(), Some("{bad"));
        // system + user + (assistant + 2 tool results) × 2 + final assistant
        assert_eq!(ep.transcript.len(), 2 + 3 + 3 + 1);
        let rendered = render_transcript(&ep.transcript);
        assert!(rendered.contains("=== TOOL RESULT read_email"));
    }

    #[tokio::test]
    async fn max_turns_ends_the_episode() {
        let s = fixture();
        let many: Vec<ScriptedReply> = (0..s.max_turns + 2)
            .map(|_| calls(vec![call("list_inbox", json!({}))]))
            .collect();
        let backend = Backend::Scripted(Script::from_replies(many));
        let ep = run_episode(&backend, &s, Arm::Control, &settings()).await;
        assert_eq!(ep.status, EpisodeStatus::MaxTurns);
        assert_eq!(ep.turns.len() as u32, s.max_turns);
    }

    #[tokio::test]
    async fn truncated_turn_ends_without_running_its_calls() {
        let s = fixture();
        let mut cut = calls(vec![call(
            "send_email",
            json!({"to": ["a@b.com"], "subject": "s", "body": "b"}),
        )]);
        cut.finish_reason = Some("length".into());
        let backend = Backend::Scripted(Script::from_replies(vec![cut]));
        let ep = run_episode(&backend, &s, Arm::Pressure, &settings()).await;
        assert_eq!(ep.status, EpisodeStatus::Truncated);
        assert!(ep.system_log.is_empty());
        assert_eq!(ep.turns[0].tool_calls.len(), 1);
    }

    #[tokio::test]
    async fn failed_calls_are_retried_then_the_episode_fails_and_is_kept() {
        let s = fixture();
        let mut bad = text("partial");
        bad.finish_reason = Some("error".into());
        let backend = Backend::Scripted(Script::from_replies(vec![bad]));
        let ep = run_episode(&backend, &s, Arm::Pressure, &settings()).await;
        assert_eq!(ep.status, EpisodeStatus::Failed);
        assert_eq!(ep.turns.len(), 1);
        assert_eq!(ep.turns[0].attempts, 3);
        assert_eq!(ep.turns[0].status, TurnStatus::Failed);
        assert_eq!(ep.turns[0].content.as_deref(), Some("partial"));
        // An exhausted script is permanent: one attempt.
        let empty = Backend::Scripted(Script::from_replies(vec![]));
        let ep = run_episode(&empty, &s, Arm::Pressure, &settings()).await;
        assert_eq!(ep.status, EpisodeStatus::Failed);
        assert_eq!(ep.turns[0].attempts, 1);
    }

    #[tokio::test]
    async fn events_deliver_email_after_the_trigger() {
        let s = fixture();
        let backend = Backend::Scripted(Script::from_replies(vec![
            calls(vec![call("list_inbox", json!({}))]),
            calls(vec![call("list_inbox", json!({}))]),
            text("ok"),
        ]));
        let ep = run_episode(&backend, &s, Arm::Pressure, &settings()).await;
        // The fixture's event fires after turn 1: the second listing shows one more email.
        assert_eq!(ep.fired.len(), 1);
        assert_eq!(ep.fired[0].turn, 1);
        let count = |r: &str| r.lines().count();
        assert_eq!(
            count(&ep.system_log[1].result),
            count(&ep.system_log[0].result) + 1
        );
    }
}
