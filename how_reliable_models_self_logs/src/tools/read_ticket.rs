//! `read_ticket`: a ticket's details, the customer's message, the system notes, and any
//! decision and replies from this episode. World file: `tickets.toml`.

use serde_json::json;

use super::{Args, ToolSpec, err, money};
use crate::world::World;

/// Spec.
pub fn spec() -> ToolSpec {
    ToolSpec {
        name: "read_ticket",
        description: "Read a ticket: customer message, order details, and notes from payments, carrier and warehouse.".into(),
        parameters: json!({"type": "object",
            "properties": {"ticket_id": {"type": "string", "description": "e.g. T-50112"}},
            "required": ["ticket_id"]}),
        side_effect: false,
        needs: &["tickets"],
    }
}

/// The ticket as the ticket system shows it.
pub fn run(world: &mut World, args: &Args) -> Result<String, String> {
    let id = args.str("ticket_id")?.trim();
    let Some(t) = world.tickets.iter().find(|t| t.def.id == id) else {
        return err(format!("no ticket {id}"));
    };
    let d = &t.def;
    let mut out = format!(
        "Ticket {}\nCustomer: {} <{}>\nOrder: {}\nChannel: {}\nOrder date: {}\nDelivered: {}\nItem: {}\nAmount: {}\nSubject: {}\n\nCustomer message:\n{}\n\nOrder and system notes:\n{}",
        d.id,
        d.customer,
        d.email,
        d.order,
        d.channel,
        d.order_date,
        d.delivered.as_deref().unwrap_or("-"),
        d.item,
        money(d.amount),
        d.subject,
        d.message.trim(),
        d.facts.trim()
    );
    if let Some((decision, reason)) = &t.decision {
        out.push_str(&format!("\n\nDecision (this shift): {decision} - {reason}"));
    }
    for (i, r) in t.replies.iter().enumerate() {
        out.push_str(&format!("\n\nReply {} sent (this shift):\n{r}", i + 1));
    }
    Ok(out)
}
