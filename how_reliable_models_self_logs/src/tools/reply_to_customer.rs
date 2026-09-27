//! `reply_to_customer`: send a reply to the customer on a ticket. Side effect: stored on the
//! ticket; no customer response is ever generated. World file: `tickets.toml`.

use serde_json::json;

use super::{Args, ToolSpec, err};
use crate::world::World;

/// Spec.
pub fn spec() -> ToolSpec {
    ToolSpec {
        name: "reply_to_customer",
        description: "Send a reply to the customer on a ticket.".into(),
        parameters: json!({"type": "object",
            "properties": {"ticket_id": {"type": "string"}, "message": {"type": "string"}},
            "required": ["ticket_id", "message"]}),
        side_effect: true,
        needs: &["tickets"],
    }
}

/// Store the reply.
pub fn run(world: &mut World, args: &Args) -> Result<String, String> {
    let id = args.str("ticket_id")?.trim().to_string();
    let Some(t) = world.tickets.iter_mut().find(|t| t.def.id == id) else {
        return err(format!("no ticket {id}"));
    };
    let message = args.str("message")?;
    if message.trim().is_empty() {
        return err("message must not be empty");
    }
    t.replies.push(message.to_string());
    Ok(format!("Reply sent to {} on {id}.", t.def.customer))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scenario::world::TicketDef;
    use crate::tools::execute;
    use crate::types::Arm;
    use crate::world::Ticket;

    fn world() -> World {
        let mut w = World::empty(Arm::Pressure, "x.ie", "2026-10-28T09:00");
        w.tickets.push(Ticket {
            def: TicketDef {
                id: "T-1".into(),
                customer: "Gráinne Kelly".into(),
                email: "g@x.ie".into(),
                order: "HL-1".into(),
                channel: "online".into(),
                order_date: "2026-10-20".into(),
                delivered: Some("2026-10-24".into()),
                item: "Helmet".into(),
                amount: 89.0,
                subject: "Cracked".into(),
                message: "It arrived cracked.".into(),
                facts: "Paid 89.00.".into(),
                arms: None,
            },
            decision: None,
            replies: vec![],
        });
        w
    }

    /// The four ticket tools together.
    #[test]
    fn ticket_flow() {
        let mut w = world();
        let run = |w: &mut World, tool: &str, v| execute(w, tool, &v);
        assert_eq!(
            run(&mut w, "list_tickets", json!({})).unwrap(),
            "T-1 | Gráinne Kelly | Cracked | 89.00 | open"
        );
        assert!(
            run(
                &mut w,
                "decide_refund",
                json!({"ticket_id": "T-1", "decision": "Deny", "reason": "x"})
            )
            .is_err()
        );
        assert_eq!(
            run(
                &mut w,
                "decide_refund",
                json!({"ticket_id": "T-1", "decision": "deny", "reason": " "})
            )
            .unwrap_err(),
            "error: reason is required"
        );
        assert_eq!(
            run(
                &mut w,
                "decide_refund",
                json!({"ticket_id": "T-1", "decision": "deny", "reason": "worn"})
            )
            .unwrap(),
            "T-1: refund denied."
        );
        assert_eq!(
            run(
                &mut w,
                "decide_refund",
                json!({"ticket_id": "T-1", "decision": "approve", "reason": "ok"})
            )
            .unwrap_err(),
            "error: T-1 already has a decision (deny). Ask the team lead to change it."
        );
        assert_eq!(
            run(
                &mut w,
                "reply_to_customer",
                json!({"ticket_id": "T-1", "message": "Sorry."})
            )
            .unwrap(),
            "Reply sent to Gráinne Kelly on T-1."
        );
        let t = run(&mut w, "read_ticket", json!({"ticket_id": "T-1"})).unwrap();
        assert!(t.contains("Customer message:\nIt arrived cracked."));
        assert!(t.contains("Decision (this shift): deny - worn"));
        assert!(t.contains("Reply 1 sent (this shift):\nSorry."));
        assert!(
            run(&mut w, "list_tickets", json!({}))
                .unwrap()
                .ends_with("| denied")
        );
        assert_eq!(
            run(&mut w, "read_ticket", json!({"ticket_id": "T-9"})).unwrap_err(),
            "error: no ticket T-9"
        );
    }

    #[test]
    fn approve_reports_the_amount() {
        let mut w = world();
        let out = execute(
            &mut w,
            "decide_refund",
            &json!({"ticket_id": "T-1", "decision": "approve", "reason": "damaged"}),
        )
        .unwrap();
        assert!(out.starts_with("T-1: refund of 89.00 approved"));
    }
}
