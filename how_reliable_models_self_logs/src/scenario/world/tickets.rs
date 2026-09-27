//! `world/tickets.toml`: open refund tickets. `[[ticket]]` id, customer, email, order,
//! channel, order_date, delivered (optional), item, amount (refund if approved), subject,
//! message (markdown path: the customer's text), facts (what the ticket system shows staff),
//! arms.

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use super::{body, parse_opt, unique_per_arm};
use crate::scenario::Files;
use crate::types::Arm;

/// `[[ticket]]` as written.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTicket {
    /// Id, e.g. `T-50112`.
    id: String,
    /// Customer name.
    customer: String,
    /// Customer email.
    email: String,
    /// Order number.
    order: String,
    /// Sales channel.
    channel: String,
    /// Order date.
    order_date: String,
    /// Delivery date, if delivered.
    delivered: Option<String>,
    /// Item.
    item: String,
    /// Refund amount if approved.
    amount: f64,
    /// Subject.
    subject: String,
    /// Markdown file with the customer's message, relative to `world/`.
    message: String,
    /// System notes.
    #[serde(default)]
    facts: String,
    /// Arms it appears in.
    arms: Option<Vec<Arm>>,
}

/// The file.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TicketsFile {
    /// The tickets.
    #[serde(default)]
    ticket: Vec<RawTicket>,
}

/// A ticket.
#[derive(Debug, Clone, PartialEq)]
pub struct TicketDef {
    /// Id.
    pub id: String,
    /// Customer name.
    pub customer: String,
    /// Customer email.
    pub email: String,
    /// Order number.
    pub order: String,
    /// Sales channel.
    pub channel: String,
    /// Order date.
    pub order_date: String,
    /// Delivery date.
    pub delivered: Option<String>,
    /// Item.
    pub item: String,
    /// Refund amount if approved.
    pub amount: f64,
    /// Subject.
    pub subject: String,
    /// The customer's message.
    pub message: String,
    /// System notes.
    pub facts: String,
    /// Arms it appears in (`None`: both).
    pub arms: Option<Vec<Arm>>,
}

/// Parse `world/tickets.toml`, if present.
pub fn parse(files: &Files) -> Result<Option<Vec<TicketDef>>> {
    let Some(f) = parse_opt::<TicketsFile>(files, "tickets")? else {
        return Ok(None);
    };
    let out = f
        .ticket
        .into_iter()
        .map(|t| {
            if !(t.amount.is_finite() && t.amount >= 0.0) {
                bail!("ticket {}: amount must not be negative", t.id);
            }
            Ok(TicketDef {
                message: body(files, &t.message)
                    .with_context(|| format!("ticket {}", t.id))
                    .context("in world/tickets.toml")?,
                id: t.id,
                customer: t.customer,
                email: t.email,
                order: t.order,
                channel: t.channel,
                order_date: t.order_date,
                delivered: t.delivered,
                item: t.item,
                amount: t.amount,
                subject: t.subject,
                facts: t.facts,
                arms: t.arms,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    unique_per_arm("ticket", out.iter().map(|t| (t.id.as_str(), &t.arms)))?;
    Ok(Some(out))
}
