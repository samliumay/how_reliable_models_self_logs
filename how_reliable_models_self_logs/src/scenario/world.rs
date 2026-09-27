//! The world files of a scenario (`world/<name>.toml` plus the markdown they point to),
//! parsed and checked. One file per world file in `world/`. All are optional; which ones a
//! scenario needs follows from its tools (`ToolSpec::needs`). Any other `world/*.toml` is an
//! error. Markdown paths are relative to `world/`. Items with `arms` exist only in those
//! arms; `arms` absent = both arms.

pub mod drive;
pub mod emails;
pub mod events;
pub mod expenses;
pub mod jobs;
pub mod ledger;
pub mod tickets;

use std::collections::BTreeSet;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use super::Files;
use crate::types::Arm;

pub use drive::FileDef;
pub use emails::EmailDef;
pub use events::{EventDef, Trigger};
pub use expenses::ExpenseDef;
pub use jobs::{JobDef, JobKind, JobsDef};
pub use ledger::{Account, LedgerDef};
pub use tickets::TicketDef;

/// The world files this code reads, by `<name>` of `world/<name>.toml`.
pub const KNOWN: &[&str] = &[
    "drive", "emails", "events", "expenses", "jobs", "ledger", "tickets",
];

/// An id written as a TOML integer or string; kept as text.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum RawId {
    /// `id = 3`.
    Int(i64),
    /// `id = "welcome"`.
    Text(String),
}

impl RawId {
    /// The id as text.
    pub fn text(self) -> String {
        match self {
            RawId::Int(i) => i.to_string(),
            RawId::Text(s) => s,
        }
    }
}

/// An email with its body text (inbox, events, sent mail).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Email {
    /// Id as text (`3`, `sent-1`).
    pub id: String,
    /// Sender.
    pub from: String,
    /// Recipients.
    pub to: Vec<String>,
    /// Copy recipients.
    pub cc: Vec<String>,
    /// `YYYY-MM-DDTHH:MM`.
    pub date: String,
    /// Subject line.
    pub subject: String,
    /// Body text (markdown).
    pub body: String,
}

/// Every world file of a scenario, parsed. `None` = the file is absent.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WorldDef {
    /// `emails.toml`: the inbox, all arms.
    pub emails: Option<Vec<EmailDef>>,
    /// `drive.toml`: the drive, all arms.
    pub drive: Option<Vec<FileDef>>,
    /// `events.toml`: events, all arms.
    pub events: Vec<EventDef>,
    /// `jobs.toml`: the scripted job runner.
    pub jobs: Option<JobsDef>,
    /// `expenses.toml`: expenses under review, all arms.
    pub expenses: Option<Vec<ExpenseDef>>,
    /// `ledger.toml`: chart of accounts and journal numbering.
    pub ledger: Option<LedgerDef>,
    /// `tickets.toml`: refund tickets, all arms.
    pub tickets: Option<Vec<TicketDef>>,
}

impl WorldDef {
    /// Is `world/<name>.toml` present?
    pub fn has(&self, name: &str) -> bool {
        match name {
            "emails" => self.emails.is_some(),
            "drive" => self.drive.is_some(),
            "events" => !self.events.is_empty(),
            "jobs" => self.jobs.is_some(),
            "expenses" => self.expenses.is_some(),
            "ledger" => self.ledger.is_some(),
            "tickets" => self.tickets.is_some(),
            _ => false,
        }
    }
}

impl WorldDef {
    /// One line of counts, for `validate`.
    pub fn summary(&self) -> String {
        let n = |o: Option<usize>, what: &str| o.map(|n| format!("{n} {what}"));
        [
            n(self.emails.as_ref().map(Vec::len), "emails"),
            n(self.drive.as_ref().map(Vec::len), "files"),
            (!self.events.is_empty()).then(|| format!("{} events", self.events.len())),
            n(self.jobs.as_ref().map(|j| j.jobs.len()), "jobs"),
            n(self.expenses.as_ref().map(Vec::len), "expenses"),
            n(self.ledger.as_ref().map(|l| l.accounts.len()), "accounts"),
            n(self.tickets.as_ref().map(Vec::len), "tickets"),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(", ")
    }
}

/// Is an item with these `arms` present in `arm`?
pub fn in_arm(arms: &Option<Vec<Arm>>, arm: Arm) -> bool {
    arms.as_ref().is_none_or(|a| a.contains(&arm))
}

/// Read a markdown file of `world/` from the scenario's files.
pub fn body(files: &Files, rel: &str) -> Result<String> {
    let key = format!("world/{rel}");
    files
        .get(&key)
        .cloned()
        .with_context(|| format!("{key} does not exist"))
}

/// Parse `world/<name>.toml`, if present.
pub fn parse_opt<T: serde::de::DeserializeOwned>(files: &Files, name: &str) -> Result<Option<T>> {
    let key = format!("world/{name}.toml");
    files
        .get(&key)
        .map(|t| toml::from_str::<T>(t).with_context(|| format!("parsing {key}")))
        .transpose()
}

/// Check a `YYYY-MM-DDTHH:MM` timestamp.
pub fn check_date(s: &str) -> Result<()> {
    chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M")
        .map(|_| ())
        .with_context(|| format!("date {s:?} is not YYYY-MM-DDTHH:MM"))
}

/// Check that `key(item)` is unique among the items of each arm.
pub fn unique_per_arm<'a>(
    what: &str,
    items: impl Iterator<Item = (&'a str, &'a Option<Vec<Arm>>)> + Clone,
) -> Result<()> {
    for arm in Arm::ALL {
        let mut seen = BTreeSet::new();
        for (key, arms) in items.clone() {
            if in_arm(arms, *arm) && !seen.insert(key) {
                bail!("{what} {key} appears twice in arm {arm}");
            }
        }
    }
    Ok(())
}

/// Parse every world file and cross-check them.
pub fn parse(files: &Files) -> Result<WorldDef> {
    for key in files.keys() {
        if let Some(name) = key
            .strip_prefix("world/")
            .and_then(|k| k.strip_suffix(".toml"))
            && !KNOWN.contains(&name)
        {
            bail!(
                "{key} is not a world file this code reads ({})",
                KNOWN.join(", ")
            );
        }
    }
    let w = WorldDef {
        emails: emails::parse(files)?,
        drive: drive::parse(files)?,
        events: events::parse(files)?,
        jobs: jobs::parse(files)?,
        expenses: expenses::parse(files)?,
        ledger: ledger::parse(files)?,
        tickets: tickets::parse(files)?,
    };
    // Email ids are unique per arm across the inbox and the events.
    let inbox = w
        .emails
        .iter()
        .flatten()
        .map(|e| (e.email.id.as_str(), &e.arms));
    let events = w.events.iter().map(|e| (e.email.id.as_str(), &e.arms));
    unique_per_arm("email id", inbox.chain(events))?;
    if let (Some(exp), Some(ledger)) = (&w.expenses, &w.ledger) {
        for e in exp {
            if ledger.account(&e.account).is_none() {
                bail!(
                    "expense {}: account {} is not in world/ledger.toml",
                    e.id,
                    e.account
                );
            }
        }
    }
    Ok(w)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_world_files_are_rejected() {
        let mut files = Files::new();
        files.insert("world/payroll.toml".into(), String::new());
        assert!(parse(&files).is_err());
    }
}
