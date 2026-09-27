//! `world/emails.toml`: the inbox. `[[email]]` id (integer or string), from, to, cc, date,
//! subject, body (markdown path), arms. Ids may repeat across arms, never within one.

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use super::{Email, RawId, body, check_date, parse_opt};
use crate::scenario::Files;
use crate::types::Arm;

/// An email as written (inbox and events share the shape; events have no `arms` on it).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawEmail {
    /// Email id.
    pub id: RawId,
    /// Sender, `Name <addr>` or `addr`.
    pub from: String,
    /// Recipients.
    pub to: Vec<String>,
    /// Copy recipients.
    #[serde(default)]
    pub cc: Vec<String>,
    /// `YYYY-MM-DDTHH:MM`.
    pub date: String,
    /// Subject line.
    pub subject: String,
    /// Markdown file with the body, relative to `world/`.
    pub body: String,
}

impl RawEmail {
    /// The email with its body read and its date checked.
    pub fn resolve(self, files: &Files) -> Result<Email> {
        let id = self.id.text();
        check_date(&self.date).with_context(|| format!("email {id}"))?;
        if self.to.is_empty() {
            bail!("email {id} has no recipients");
        }
        Ok(Email {
            body: body(files, &self.body).with_context(|| format!("email {id}"))?,
            id,
            from: self.from,
            to: self.to,
            cc: self.cc,
            date: self.date,
            subject: self.subject,
        })
    }
}

/// `[[email]]`: an email plus its arms.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawInboxEmail {
    /// Email id.
    id: RawId,
    /// Sender.
    from: String,
    /// Recipients.
    to: Vec<String>,
    /// Copy recipients.
    #[serde(default)]
    cc: Vec<String>,
    /// Date.
    date: String,
    /// Subject.
    subject: String,
    /// Body path.
    body: String,
    /// Arms it appears in.
    arms: Option<Vec<Arm>>,
}

/// The file.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EmailsFile {
    /// The inbox.
    #[serde(default)]
    email: Vec<RawInboxEmail>,
}

/// An inbox email with its arms.
#[derive(Debug, Clone, PartialEq)]
pub struct EmailDef {
    /// The email.
    pub email: Email,
    /// Arms it appears in (`None`: both).
    pub arms: Option<Vec<Arm>>,
}

/// Parse `world/emails.toml`, if present.
pub fn parse(files: &Files) -> Result<Option<Vec<EmailDef>>> {
    let Some(f) = parse_opt::<EmailsFile>(files, "emails")? else {
        return Ok(None);
    };
    f.email
        .into_iter()
        .map(|e| {
            let raw = RawEmail {
                id: e.id,
                from: e.from,
                to: e.to,
                cc: e.cc,
                date: e.date,
                subject: e.subject,
                body: e.body,
            };
            Ok(EmailDef {
                email: raw.resolve(files).context("in world/emails.toml")?,
                arms: e.arms,
            })
        })
        .collect::<Result<Vec<_>>>()
        .map(Some)
}
