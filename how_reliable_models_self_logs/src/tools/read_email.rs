//! `read_email`: one inbox email by id (fired event emails included; unfired ones are not
//! found). World file: `emails.toml` (+ `events.toml`).

use serde_json::json;

use super::{Args, ToolSpec, err};
use crate::world::World;

/// Spec.
pub fn spec() -> ToolSpec {
    ToolSpec {
        name: "read_email",
        description: "Read one email by id.".into(),
        parameters: json!({"type": "object",
            "properties": {"id": {"type": "integer", "description": "Email id from list_inbox."}},
            "required": ["id"]}),
        side_effect: false,
        needs: &["emails"],
    }
}

/// `From:`, `To:`, `Cc:` (if any), `Date:`, `Subject:`, blank line, body.
pub fn run(world: &mut World, args: &Args) -> Result<String, String> {
    let id = args.id("id")?;
    let id = id.trim_start_matches('#');
    let Some(e) = world.inbox.iter().find(|e| e.id == id) else {
        return err(format!("no email with id {id}"));
    };
    let mut out = format!("From: {}\nTo: {}\n", e.from, e.to.join(", "));
    if !e.cc.is_empty() {
        out.push_str(&format!("Cc: {}\n", e.cc.join(", ")));
    }
    out.push_str(&format!(
        "Date: {}\nSubject: {}\n\n{}",
        e.date.replace('T', " "),
        e.subject,
        e.body
    ));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scenario::world::Email;
    use crate::tools::execute;
    use crate::types::Arm;

    #[test]
    fn reads_by_integer_or_string_id() {
        let mut w = World::empty(Arm::Control, "x.com", "2026-10-14T09:30");
        w.inbox.push(Email {
            id: "7".into(),
            from: "a@x.com".into(),
            to: vec!["me@x.com".into()],
            cc: vec!["c@x.com".into()],
            date: "2026-10-01T08:00".into(),
            subject: "Hello".into(),
            body: "Body text".into(),
        });
        let out = execute(&mut w, "read_email", &json!({"id": 7})).unwrap();
        assert!(
            out.contains("Cc: c@x.com\nDate: 2026-10-01 08:00") && out.ends_with("\n\nBody text")
        );
        assert!(execute(&mut w, "read_email", &json!({"id": "7"})).is_ok());
        assert_eq!(
            execute(&mut w, "read_email", &json!({"id": 8})).unwrap_err(),
            "error: no email with id 8"
        );
    }
}
