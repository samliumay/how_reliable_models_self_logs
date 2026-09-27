//! `world/events.toml`: `[[event]]` id, trigger (`{ after_turn = N }`, N from 1, or
//! `{ after_tool = "name" }`), email (an inbox email without arms), arms. An event fires once
//! and adds its email to the inbox.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use super::emails::RawEmail;
use super::{Email, RawId, parse_opt, unique_per_arm};
use crate::scenario::Files;
use crate::types::Arm;

/// When an event fires.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub enum Trigger {
    /// After the model's N-th turn (1-based), once its tool calls ran.
    AfterTurn(u32),
    /// After the first successful call of this tool.
    AfterTool(String),
}

/// `[[event]]` as written.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawEvent {
    /// Event id.
    id: RawId,
    /// When it fires.
    trigger: Trigger,
    /// The email it delivers.
    email: RawEmail,
    /// Arms it exists in.
    arms: Option<Vec<Arm>>,
}

/// The file.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EventsFile {
    /// The events.
    #[serde(default)]
    event: Vec<RawEvent>,
}

/// An event with its email.
#[derive(Debug, Clone, PartialEq)]
pub struct EventDef {
    /// Event id.
    pub id: String,
    /// When it fires.
    pub trigger: Trigger,
    /// The email it delivers.
    pub email: Email,
    /// Arms it exists in (`None`: both).
    pub arms: Option<Vec<Arm>>,
}

/// Parse `world/events.toml` (empty if absent).
pub fn parse(files: &Files) -> Result<Vec<EventDef>> {
    let Some(f) = parse_opt::<EventsFile>(files, "events")? else {
        return Ok(Vec::new());
    };
    let events = f
        .event
        .into_iter()
        .map(|ev| {
            let id = ev.id.text();
            if ev.trigger == Trigger::AfterTurn(0) {
                bail!("event {id}: after_turn counts from 1");
            }
            Ok(EventDef {
                email: ev
                    .email
                    .resolve(files)
                    .with_context(|| format!("event {id}"))
                    .context("in world/events.toml")?,
                id,
                trigger: ev.trigger,
                arms: ev.arms,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    unique_per_arm("event id", events.iter().map(|e| (e.id.as_str(), &e.arms)))?;
    Ok(events)
}
