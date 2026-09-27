//! The in-memory world of one episode: the arm's items from the world files plus what the
//! model changed (sent mail, written and deleted files, job runs, reclassifications,
//! journals, refund decisions, replies, self-log entries). Tools read and change it; nothing
//! here touches the database, so tools are unit-tested on a small `World`.

use serde::Serialize;
use serde_json::Value;

use crate::scenario::Scenario;
use crate::scenario::world::{
    Email, EventDef, ExpenseDef, JobDef, LedgerDef, TicketDef, Trigger, in_arm,
};
use crate::types::{Arm, RefundDecision};

/// A file on the drive (current content).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DriveEntry {
    /// Path as the model sees it.
    pub path: String,
    /// Current content.
    pub content: String,
    /// Marked confidential in `drive.toml` (false for files the model wrote).
    pub confidential: bool,
}

/// An event that fired, for the episode record.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FiredEvent {
    /// Event id.
    pub id: String,
    /// Turn after which it fired.
    pub turn: u32,
    /// The id of the email it delivered.
    pub email_id: String,
}

/// An expense with its current account.
#[derive(Debug, Clone, PartialEq)]
pub struct Expense {
    /// As defined.
    pub def: ExpenseDef,
    /// Current account code (changed by `reclassify_expense`).
    pub account: String,
}

/// A posted journal entry.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Journal {
    /// Journal number.
    pub number: String,
    /// Debit account code.
    pub debit: String,
    /// Credit account code.
    pub credit: String,
    /// Amount.
    pub amount: f64,
    /// Memo.
    pub memo: String,
}

/// A ticket with what happened to it in the episode.
#[derive(Debug, Clone, PartialEq)]
pub struct Ticket {
    /// As defined.
    pub def: TicketDef,
    /// Decision and reason, once made.
    pub decision: Option<(RefundDecision, String)>,
    /// Replies sent to the customer.
    pub replies: Vec<String>,
}

/// One episode's world.
#[derive(Debug, Clone)]
pub struct World {
    /// The arm this world belongs to.
    pub arm: Arm,
    /// `company.domain`.
    pub domain: String,
    /// The scenario's "now" (`scenario_date`), the date of what the model sends.
    pub now: String,
    /// The inbox; fired events are appended.
    pub inbox: Vec<Email>,
    /// The drive, current state.
    pub drive: Vec<DriveEntry>,
    /// Emails the model sent, in order.
    pub sent: Vec<Email>,
    /// Drive path of the job log (`jobs.toml`).
    pub job_log: Option<String>,
    /// The jobs of this arm.
    pub jobs: Vec<JobDef>,
    /// Names of the jobs that ran successfully, in order.
    pub job_runs: Vec<String>,
    /// The expenses of this arm.
    pub expenses: Vec<Expense>,
    /// Chart of accounts.
    pub ledger: Option<LedgerDef>,
    /// Journals posted.
    pub journals: Vec<Journal>,
    /// The tickets of this arm.
    pub tickets: Vec<Ticket>,
    /// Entries written with self-log tools, in order: (tool name, arguments).
    pub log_entries: Vec<(String, Value)>,
    /// Events not yet fired.
    pub pending: Vec<EventDef>,
    /// Events that fired.
    pub fired: Vec<FiredEvent>,
}

impl World {
    /// The world of `scenario` in `arm`: only the items of that arm.
    pub fn new(scenario: &Scenario, arm: Arm) -> World {
        let w = &scenario.world;
        let mut world = World::empty(arm, &scenario.company.domain, &scenario.scenario_date);
        world.inbox = w
            .emails
            .iter()
            .flatten()
            .filter(|e| in_arm(&e.arms, arm))
            .map(|e| e.email.clone())
            .collect();
        world.drive = w
            .drive
            .iter()
            .flatten()
            .filter(|f| in_arm(&f.arms, arm))
            .map(|f| DriveEntry {
                path: f.path.clone(),
                content: f.content.clone(),
                confidential: f.confidential,
            })
            .collect();
        if let Some(j) = &w.jobs {
            world.job_log = Some(j.job_log.clone());
            world.jobs = j
                .jobs
                .iter()
                .filter(|j| in_arm(&j.arms, arm))
                .cloned()
                .collect();
        }
        world.expenses = w
            .expenses
            .iter()
            .flatten()
            .filter(|e| in_arm(&e.arms, arm))
            .map(|e| Expense {
                account: e.account.clone(),
                def: e.clone(),
            })
            .collect();
        world.ledger = w.ledger.clone();
        world.tickets = w
            .tickets
            .iter()
            .flatten()
            .filter(|t| in_arm(&t.arms, arm))
            .map(|t| Ticket {
                def: t.clone(),
                decision: None,
                replies: Vec::new(),
            })
            .collect();
        world.pending = w
            .events
            .iter()
            .filter(|e| in_arm(&e.arms, arm))
            .cloned()
            .collect();
        world
    }

    /// An empty world (tests).
    pub fn empty(arm: Arm, domain: &str, now: &str) -> World {
        World {
            arm,
            domain: domain.to_string(),
            now: now.to_string(),
            inbox: Vec::new(),
            drive: Vec::new(),
            sent: Vec::new(),
            job_log: None,
            jobs: Vec::new(),
            job_runs: Vec::new(),
            expenses: Vec::new(),
            ledger: None,
            journals: Vec::new(),
            tickets: Vec::new(),
            log_entries: Vec::new(),
            pending: Vec::new(),
            fired: Vec::new(),
        }
    }

    /// The drive file at `path`.
    pub fn file(&self, path: &str) -> Option<&DriveEntry> {
        self.drive.iter().find(|f| f.path == path)
    }

    /// Create or overwrite a drive file.
    pub fn write_file(&mut self, path: &str, content: String) {
        match self.drive.iter_mut().find(|f| f.path == path) {
            Some(f) => f.content = content,
            None => self.drive.push(DriveEntry {
                path: path.to_string(),
                content,
                confidential: false,
            }),
        }
    }

    /// Append a line to a drive file, creating it if absent.
    pub fn append_line(&mut self, path: &str, line: &str) {
        let mut content = self
            .file(path)
            .map(|f| f.content.clone())
            .unwrap_or_default();
        if !content.is_empty() && !content.ends_with('\n') {
            content.push('\n');
        }
        content.push_str(line);
        content.push('\n');
        self.write_file(path, content);
    }

    /// Fire the pending events whose trigger holds, delivering their emails to the inbox.
    fn fire(&mut self, turn: u32, due: impl Fn(&Trigger) -> bool) -> Vec<String> {
        let (now, later): (Vec<EventDef>, Vec<EventDef>) = std::mem::take(&mut self.pending)
            .into_iter()
            .partition(|e| due(&e.trigger));
        self.pending = later;
        let mut ids = Vec::new();
        for e in now {
            self.fired.push(FiredEvent {
                id: e.id.clone(),
                turn,
                email_id: e.email.id.clone(),
            });
            ids.push(e.id);
            self.inbox.push(e.email);
        }
        ids
    }

    /// After a successful call of `tool` in `turn`: fire `after_tool` events for it (each
    /// fires once, after the first matching call). Returns the fired event ids.
    pub fn after_tool(&mut self, tool: &str, turn: u32) -> Vec<String> {
        self.fire(
            turn,
            |t| matches!(t, Trigger::AfterTool(name) if name == tool),
        )
    }

    /// After turn `turn` (1-based) and its tool calls: fire `after_turn` events due by now.
    pub fn after_turn(&mut self, turn: u32) -> Vec<String> {
        self.fire(turn, |t| matches!(t, Trigger::AfterTurn(n) if *n <= turn))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(id: &str, trigger: Trigger) -> EventDef {
        EventDef {
            id: id.into(),
            trigger,
            email: Email {
                id: format!("e-{id}"),
                from: "a@x.com".into(),
                to: vec!["b@x.com".into()],
                cc: vec![],
                date: "2026-10-14T10:00".into(),
                subject: "s".into(),
                body: "b".into(),
            },
            arms: None,
        }
    }

    #[test]
    fn events_fire_once_at_their_trigger() {
        let mut w = World::empty(Arm::Pressure, "x.com", "2026-10-14T09:30");
        w.pending = vec![
            event("t2", Trigger::AfterTurn(2)),
            event("send", Trigger::AfterTool("send_email".into())),
        ];
        assert!(w.after_turn(1).is_empty());
        assert!(w.after_tool("read_email", 1).is_empty());
        assert_eq!(w.after_tool("send_email", 2), vec!["send"]);
        assert!(w.after_tool("send_email", 2).is_empty());
        assert_eq!(w.after_turn(2), vec!["t2"]);
        assert_eq!(w.inbox.len(), 2);
        assert_eq!(w.fired[0].turn, 2);
        assert!(w.pending.is_empty());
    }

    #[test]
    fn append_line_creates_and_extends() {
        let mut w = World::empty(Arm::Pressure, "x.com", "2026-10-14T09:30");
        w.append_line("logs/a.log", "one");
        w.write_file("logs/b.log", "x".into());
        w.append_line("logs/b.log", "two");
        assert_eq!(w.file("logs/a.log").unwrap().content, "one\n");
        assert_eq!(w.file("logs/b.log").unwrap().content, "x\ntwo\n");
    }
}
