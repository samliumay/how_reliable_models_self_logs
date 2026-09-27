//! `list_inbox`: the inbox (initial emails of the arm plus fired events), newest first.
//! Sent mail does not appear. World file: `emails.toml` (+ `events.toml`).

use serde_json::json;

use super::{Args, ToolSpec};
use crate::world::World;

/// Spec.
pub fn spec() -> ToolSpec {
    ToolSpec {
        name: "list_inbox",
        description: "List emails in the inbox (id, from, subject, date), newest first.".into(),
        parameters: json!({"type": "object", "properties": {}}),
        side_effect: false,
        needs: &["emails"],
    }
}

/// `#<id> | <date> | <from> | <subject>`, one line per email.
pub fn run(world: &mut World, _args: &Args) -> Result<String, String> {
    if world.inbox.is_empty() {
        return Ok("(inbox is empty)".into());
    }
    let mut emails: Vec<_> = world.inbox.iter().collect();
    // Stable: emails with the same date keep their scenario order.
    emails.sort_by(|a, b| b.date.cmp(&a.date));
    Ok(emails
        .iter()
        .map(|e| {
            format!(
                "#{} | {} | {} | {}",
                e.id,
                e.date.replace('T', " "),
                e.from,
                e.subject
            )
        })
        .collect::<Vec<_>>()
        .join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scenario::world::Email;
    use crate::tools::execute;
    use crate::types::Arm;

    #[test]
    fn newest_first() {
        let mut w = World::empty(Arm::Control, "x.com", "2026-10-14T09:30");
        for (id, date) in [("1", "2026-10-01T08:00"), ("2", "2026-10-14T08:00")] {
            w.inbox.push(Email {
                id: id.into(),
                from: "A <a@x.com>".into(),
                to: vec!["me@x.com".into()],
                cc: vec![],
                date: date.into(),
                subject: format!("s{id}"),
                body: String::new(),
            });
        }
        let out = execute(&mut w, "list_inbox", &json!({})).unwrap();
        assert_eq!(
            out,
            "#2 | 2026-10-14 08:00 | A <a@x.com> | s2\n#1 | 2026-10-01 08:00 | A <a@x.com> | s1"
        );
    }
}
