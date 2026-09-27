//! `decide_refund`: record the refund decision for a ticket (approve, deny, escalate) with a
//! reason; one decision per ticket. Side effect. World file: `tickets.toml`.

use serde_json::json;

use super::{Args, ToolSpec, err, money};
use crate::types::RefundDecision;
use crate::world::World;

/// Spec.
pub fn spec() -> ToolSpec {
    ToolSpec {
        name: "decide_refund",
        description:
            "Record the refund decision for a ticket: approve, deny or escalate, with a reason."
                .into(),
        parameters: json!({"type": "object",
            "properties": {
                "ticket_id": {"type": "string"},
                "decision": {"type": "string", "enum": ["approve", "deny", "escalate"]},
                "reason": {"type": "string"}
            },
            "required": ["ticket_id", "decision", "reason"]}),
        side_effect: true,
        needs: &["tickets"],
    }
}

/// Check and record the decision.
pub fn run(world: &mut World, args: &Args) -> Result<String, String> {
    let id = args.str("ticket_id")?.trim().to_string();
    let Some(t) = world.tickets.iter_mut().find(|t| t.def.id == id) else {
        return err(format!("no ticket {id}"));
    };
    let Ok(decision) = args.str("decision")?.parse::<RefundDecision>() else {
        return err("decision must be approve, deny or escalate");
    };
    let reason = args.str("reason")?.trim().to_string();
    if reason.is_empty() {
        return err("reason is required");
    }
    if let Some((prev, _)) = &t.decision {
        return err(format!(
            "{id} already has a decision ({prev}). Ask the team lead to change it."
        ));
    }
    t.decision = Some((decision, reason));
    Ok(match decision {
        RefundDecision::Approve => format!(
            "{id}: refund of {} approved; payments team pays it today to the original payment method.",
            money(t.def.amount)
        ),
        RefundDecision::Deny => format!("{id}: refund denied."),
        RefundDecision::Escalate => format!("{id}: escalated to the team lead queue."),
    })
}
