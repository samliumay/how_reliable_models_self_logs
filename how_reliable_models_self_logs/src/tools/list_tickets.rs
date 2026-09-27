//! `list_tickets`: every refund ticket with its status (`open`, or the decision made in this
//! episode). World file: `tickets.toml`.

use serde_json::json;

use super::{Args, ToolSpec, money};
use crate::types::RefundDecision;
use crate::world::{Ticket, World};

/// Spec.
pub fn spec() -> ToolSpec {
    ToolSpec {
        name: "list_tickets",
        description: "List open refund tickets (id, customer, subject, amount, status).".into(),
        parameters: json!({"type": "object", "properties": {}}),
        side_effect: false,
        needs: &["tickets"],
    }
}

/// `open`, `approved`, `denied` or `escalated`.
pub fn status(t: &Ticket) -> &'static str {
    match t.decision.as_ref().map(|(d, _)| *d) {
        None => "open",
        Some(RefundDecision::Approve) => "approved",
        Some(RefundDecision::Deny) => "denied",
        Some(RefundDecision::Escalate) => "escalated",
    }
}

/// `<id> | <customer> | <subject> | <amount> | <status>`, one line per ticket.
pub fn run(world: &mut World, _args: &Args) -> Result<String, String> {
    if world.tickets.is_empty() {
        return Ok("(no tickets)".into());
    }
    Ok(world
        .tickets
        .iter()
        .map(|t| {
            format!(
                "{} | {} | {} | {} | {}",
                t.def.id,
                t.def.customer,
                t.def.subject,
                money(t.def.amount),
                status(t)
            )
        })
        .collect::<Vec<_>>()
        .join("\n"))
}
