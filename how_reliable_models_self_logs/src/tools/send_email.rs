//! `send_email`: send an email. Side effect: recorded in the world's sent mail. No reply is
//! ever generated.

use serde_json::json;

use super::{Args, ToolSpec, err};
use crate::scenario::world::Email;
use crate::world::World;

/// Spec.
pub fn spec() -> ToolSpec {
    ToolSpec {
        name: "send_email",
        description: "Send an email.".into(),
        parameters: json!({"type": "object",
            "properties": {
                "to": {"type": "array", "items": {"type": "string"}, "description": "Recipient addresses."},
                "cc": {"type": "array", "items": {"type": "string"}, "description": "Copy recipients."},
                "subject": {"type": "string"},
                "body": {"type": "string"}
            },
            "required": ["to", "subject", "body"]}),
        side_effect: true,
        needs: &[],
    }
}

/// Check the addresses (each must contain `@`), then record the email.
pub fn run(world: &mut World, args: &Args) -> Result<String, String> {
    let to = args.str_list("to");
    let cc = args.str_list("cc");
    if to.is_empty() {
        return err("'to' must contain at least one address");
    }
    if let Some(bad) = to.iter().chain(&cc).find(|a| !a.contains('@')) {
        return err(format!("invalid address {bad}"));
    }
    let subject = args.str("subject")?.to_string();
    world.sent.push(Email {
        id: format!("sent-{}", world.sent.len() + 1),
        from: "me".into(),
        to: to.clone(),
        cc: cc.clone(),
        date: world.now.clone(),
        subject: subject.clone(),
        body: args.str("body")?.to_string(),
    });
    let cc = if cc.is_empty() {
        String::new()
    } else {
        format!(" (cc {})", cc.join(", "))
    };
    Ok(format!("Sent to {}{cc}: \"{subject}\"", to.join(", ")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::execute;
    use crate::types::Arm;

    #[test]
    fn sends_and_reports() {
        let mut w = World::empty(Arm::Pressure, "x.com", "2026-10-14T09:30");
        let out = execute(
            &mut w,
            "send_email",
            &json!({"to": ["a@x.com"], "cc": ["b@x.com"], "subject": "Hi", "body": "Text"}),
        )
        .unwrap();
        assert_eq!(out, "Sent to a@x.com (cc b@x.com): \"Hi\"");
        assert_eq!(w.sent[0].date, "2026-10-14T09:30");
    }

    #[test]
    fn nothing_is_sent_on_bad_input() {
        let mut w = World::empty(Arm::Pressure, "x.com", "2026-10-14T09:30");
        let e = |w: &mut World, v| execute(w, "send_email", &v).unwrap_err();
        assert_eq!(
            e(&mut w, json!({"to": [], "subject": "s", "body": "b"})),
            "error: 'to' must contain at least one address"
        );
        assert_eq!(
            e(
                &mut w,
                json!({"to": ["Marcus"], "subject": "s", "body": "b"})
            ),
            "error: invalid address Marcus"
        );
        assert!(
            execute(
                &mut w,
                "send_email",
                &json!({"to": ["a@x.com"], "body": "b"})
            )
            .is_err()
        );
        assert!(w.sent.is_empty());
    }
}
