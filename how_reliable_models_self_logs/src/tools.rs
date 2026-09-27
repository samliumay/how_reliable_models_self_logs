//! The tool registry: every tool a scenario can list (`benchmark/scenarios/TOOLS.md`), one
//! file each in `tools/`. Tools are pure functions over the episode's in-memory [`World`] and
//! the call's JSON arguments. A tool returns the text the model sees, or a tool error: a
//! string starting with `error:`, also shown to the model; the episode continues and the
//! world is unchanged. Every call is recorded by the agent loop as a [`SystemLogEntry`].
//!
//! Adding a tool: one file in `tools/` with `spec()` and `run()`, plus one line in
//! [`registry`] and one arm in [`execute`].

pub mod decide_refund;
pub mod delete_file;
pub mod list_expenses;
pub mod list_files;
pub mod list_inbox;
pub mod list_tickets;
pub mod log_close_note;
pub mod log_shift_summary;
pub mod post_journal_entry;
pub mod read_email;
pub mod read_expense;
pub mod read_file;
pub mod read_ticket;
pub mod reclassify_expense;
pub mod reply_to_customer;
pub mod run_job;
pub mod send_email;
pub mod write_file;
pub mod write_worklog;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::world::World;

/// A world file a tool reads: `world/<name>.toml`.
pub type WorldFile = &'static str;

/// What the model is told about a tool, and how the harness treats it.
#[derive(Debug, Clone)]
pub struct ToolSpec {
    /// Tool name, as the model calls it.
    pub name: &'static str,
    /// Description sent to the model (a scenario's `[[tools]]` entry may replace it).
    pub description: String,
    /// JSON schema of the arguments (`type = object`).
    pub parameters: Value,
    /// Does a call change the world outside the model (send, write, pay)? Side-effect calls
    /// are the actions the self-log is judged against.
    pub side_effect: bool,
    /// World files the tool needs (`emails`, `drive`, ...); a scenario that lists the tool
    /// must have them.
    pub needs: &'static [WorldFile],
}

/// One row of the system log: a tool call as the harness executed it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SystemLogEntry {
    /// Model turn (1-based).
    pub turn: u32,
    /// Position of the call in the episode (0-based, over all turns).
    pub seq: u32,
    /// The call id the API gave (or one the harness made up).
    pub call_id: String,
    /// Tool name as called (may be unknown to the registry).
    pub tool: String,
    /// Arguments; `null` if the model sent arguments that are not a JSON object.
    pub args: Value,
    /// The raw argument text, when it did not parse.
    pub raw_args: Option<String>,
    /// What the model was shown.
    pub result: String,
    /// The call failed (unknown tool, bad arguments, tool error).
    pub is_error: bool,
    /// The tool changes the world (from its spec; false for unknown tools).
    pub side_effect: bool,
}

/// Every tool, in name order.
pub fn registry() -> Vec<ToolSpec> {
    vec![
        decide_refund::spec(),
        delete_file::spec(),
        list_expenses::spec(),
        list_files::spec(),
        list_inbox::spec(),
        list_tickets::spec(),
        log_close_note::spec(),
        log_shift_summary::spec(),
        post_journal_entry::spec(),
        read_email::spec(),
        read_expense::spec(),
        read_file::spec(),
        read_ticket::spec(),
        reclassify_expense::spec(),
        reply_to_customer::spec(),
        run_job::spec(),
        send_email::spec(),
        write_file::spec(),
        write_worklog::spec(),
    ]
}

/// The spec of a registered tool.
pub fn spec(name: &str) -> Option<ToolSpec> {
    registry().into_iter().find(|s| s.name == name)
}

/// Run one call against the world. Unknown tools and bad arguments are tool errors.
pub fn execute(world: &mut World, name: &str, args: &Value) -> Result<String, String> {
    let Some(spec) = spec(name) else {
        let known: Vec<&str> = registry().iter().map(|s| s.name).collect();
        return err(format!(
            "unknown tool '{name}'. Tools: {}",
            known.join(", ")
        ));
    };
    let args = check_args(&spec.parameters, args)?;
    match name {
        "decide_refund" => decide_refund::run(world, &args),
        "delete_file" => delete_file::run(world, &args),
        "list_expenses" => list_expenses::run(world, &args),
        "list_files" => list_files::run(world, &args),
        "list_inbox" => list_inbox::run(world, &args),
        "list_tickets" => list_tickets::run(world, &args),
        "log_close_note" => log_close_note::run(world, &args),
        "log_shift_summary" => log_shift_summary::run(world, &args),
        "post_journal_entry" => post_journal_entry::run(world, &args),
        "read_email" => read_email::run(world, &args),
        "read_expense" => read_expense::run(world, &args),
        "read_file" => read_file::run(world, &args),
        "read_ticket" => read_ticket::run(world, &args),
        "reclassify_expense" => reclassify_expense::run(world, &args),
        "reply_to_customer" => reply_to_customer::run(world, &args),
        "run_job" => run_job::run(world, &args),
        "send_email" => send_email::run(world, &args),
        "write_file" => write_file::run(world, &args),
        "write_worklog" => write_worklog::run(world, &args),
        other => err(format!("tool '{other}' has no implementation")),
    }
}

/// A tool error: `error: <msg>`.
pub fn err<T>(msg: impl std::fmt::Display) -> Result<T, String> {
    Err(format!("error: {msg}"))
}

/// A drive path as matched: one leading `./` or `/` stripped.
pub fn norm_path(p: &str) -> &str {
    p.strip_prefix("./")
        .or_else(|| p.strip_prefix('/'))
        .unwrap_or(p)
}

/// Number of words (for self-log receipts).
pub fn words(s: &str) -> usize {
    s.split_whitespace().count()
}

/// An amount with two decimals and thousands separators: `7,200.00`.
pub fn money(x: f64) -> String {
    let s = format!("{:.2}", x.abs());
    let (int, frac) = s.split_once('.').unwrap_or((&s, "00"));
    let mut grouped = String::new();
    for (i, c) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(c);
    }
    format!("{}{grouped}.{frac}", if x < 0.0 { "-" } else { "" })
}

/// Store a self-log entry: `field` must not be empty. Returns `<what> saved (<n> words).`
pub fn save_log_entry(
    world: &mut World,
    tool: &str,
    args: &Args,
    field: &str,
    what: &str,
) -> Result<String, String> {
    let text = args.str(field)?;
    if text.trim().is_empty() {
        return err(format!("{field} must not be empty"));
    }
    let n = words(text);
    world.log_entries.push((tool.to_string(), args.json()));
    Ok(format!("{what} saved ({n} words)."))
}

/// Checked arguments of one call.
pub struct Args<'a>(&'a Map<String, Value>);

impl Args<'_> {
    /// A string argument (required by the schema, so present after the check).
    pub fn str(&self, name: &str) -> Result<&str, String> {
        self.opt_str(name)
            .ok_or_else(|| format!("error: missing argument '{name}'"))
    }

    /// An optional string argument.
    pub fn opt_str(&self, name: &str) -> Option<&str> {
        self.0.get(name).and_then(Value::as_str)
    }

    /// A string-list argument; an absent optional list is empty.
    pub fn str_list(&self, name: &str) -> Vec<String> {
        match self.0.get(name) {
            Some(Value::Array(items)) => items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect(),
            _ => Vec::new(),
        }
    }

    /// An id argument given as an integer or a string, as text.
    pub fn id(&self, name: &str) -> Result<String, String> {
        match self.0.get(name) {
            Some(Value::String(s)) => Ok(s.trim().to_string()),
            Some(Value::Number(n)) => Ok(n.to_string()),
            _ => Err(format!("error: missing argument '{name}'")),
        }
    }

    /// A number argument given as a number or a numeric string.
    pub fn num(&self, name: &str) -> Result<f64, String> {
        match self.0.get(name) {
            Some(Value::Number(n)) => n
                .as_f64()
                .ok_or_else(|| format!("error: '{name}' is not a number")),
            Some(Value::String(s)) => s
                .trim()
                .parse()
                .map_err(|_| format!("error: '{name}' is not a number")),
            _ => Err(format!("error: missing argument '{name}'")),
        }
    }

    /// The arguments as JSON (for records).
    pub fn json(&self) -> Value {
        Value::Object(self.0.clone())
    }
}

/// Does `v` have the JSON-schema type `ty`? Integers and numbers may also come as numeric
/// strings, since models often quote them.
fn has_type(v: &Value, ty: &str, items: Option<&Value>) -> bool {
    match ty {
        "string" => v.is_string(),
        "integer" => v.is_i64() || v.as_str().is_some_and(|s| s.trim().parse::<i64>().is_ok()),
        "number" => v.is_number() || v.as_str().is_some_and(|s| s.trim().parse::<f64>().is_ok()),
        "boolean" => v.is_boolean(),
        "array" => v.as_array().is_some_and(|a| {
            let item_ty = items.and_then(|i| i.get("type")).and_then(Value::as_str);
            a.iter()
                .all(|x| item_ty.is_none_or(|t| has_type(x, t, None)))
        }),
        "object" => v.is_object(),
        _ => true,
    }
}

/// Check arguments against the tool's schema: an object, required keys present, no unknown
/// keys, types right. Errors are worded for the model.
pub fn check_args<'a>(schema: &Value, args: &'a Value) -> Result<Args<'a>, String> {
    let Some(obj) = args.as_object() else {
        return err("arguments must be a JSON object");
    };
    let props = schema
        .get("properties")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let required: Vec<&str> = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|r| r.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    for r in &required {
        if obj.get(*r).is_none_or(Value::is_null) {
            return err(format!("missing required argument '{r}'"));
        }
    }
    for (k, v) in obj {
        let Some(p) = props.get(k) else {
            let known: Vec<&str> = props.keys().map(String::as_str).collect();
            return err(format!(
                "unknown argument '{k}' (arguments: {})",
                known.join(", ")
            ));
        };
        if v.is_null() {
            continue;
        }
        if let Some(ty) = p.get("type").and_then(Value::as_str)
            && !has_type(v, ty, p.get("items"))
        {
            return err(format!("argument '{k}' must be of type {ty}"));
        }
    }
    Ok(Args(obj))
}

/// The argument signature of a schema in the scenario notation: `{to = "string[]",
/// cc = "string[]?"}` (`?` = optional).
pub fn signature(schema: &Value) -> BTreeMap<String, String> {
    let required: Vec<&str> = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|r| r.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let mut out = BTreeMap::new();
    if let Some(props) = schema.get("properties").and_then(Value::as_object) {
        for (name, p) in props {
            let ty = p.get("type").and_then(Value::as_str).unwrap_or("?");
            let mut s = match ty {
                "array" => format!(
                    "{}[]",
                    p.pointer("/items/type")
                        .and_then(Value::as_str)
                        .unwrap_or("?")
                ),
                other => other.to_string(),
            };
            if !required.contains(&name.as_str()) {
                s.push('?');
            }
            out.insert(name.clone(), s);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Arm;
    use serde_json::json;

    #[test]
    fn registry_names_are_unique_and_dispatch() {
        let specs = registry();
        let mut names: Vec<&str> = specs.iter().map(|s| s.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), specs.len());
        let mut w = World::empty(Arm::Control, "x.com", "2026-10-14T09:30");
        for s in &specs {
            let r = execute(&mut w, s.name, &json!({}));
            // Every registered name reaches its implementation (no "has no implementation").
            if let Err(e) = r {
                assert!(!e.contains("no implementation"), "{}: {e}", s.name);
            }
        }
    }

    #[test]
    fn arguments_are_checked_against_the_schema() {
        let schema = json!({"type": "object",
            "properties": {"to": {"type": "array", "items": {"type": "string"}},
                           "id": {"type": "integer"}},
            "required": ["to"]});
        assert!(check_args(&schema, &json!({"to": ["a@b.c"]})).is_ok());
        assert!(check_args(&schema, &json!({"to": ["a@b.c"], "id": "7"})).is_ok());
        assert!(check_args(&schema, &json!({})).is_err());
        assert!(check_args(&schema, &json!({"to": "a@b.c"})).is_err());
        assert!(check_args(&schema, &json!({"to": [1]})).is_err());
        assert!(check_args(&schema, &json!({"to": [], "bcc": []})).is_err());
        assert!(check_args(&schema, &json!("to")).is_err());
        let e = check_args(&schema, &json!({})).err().unwrap();
        assert_eq!(e, "error: missing required argument 'to'");
    }

    #[test]
    fn unknown_tool_is_a_tool_error() {
        let mut w = World::empty(Arm::Control, "x.com", "2026-10-14T09:30");
        let e = execute(&mut w, "delete_everything", &json!({})).unwrap_err();
        assert!(e.starts_with("error: unknown tool 'delete_everything'"));
    }

    #[test]
    fn registry_matches_tools_md() {
        let names: Vec<&str> = registry().iter().map(|s| s.name).collect();
        assert_eq!(names.len(), 19);
        let side: Vec<&str> = registry()
            .iter()
            .filter(|s| s.side_effect)
            .map(|s| s.name)
            .collect();
        assert_eq!(
            side,
            vec![
                "decide_refund",
                "delete_file",
                "log_close_note",
                "log_shift_summary",
                "post_journal_entry",
                "reclassify_expense",
                "reply_to_customer",
                "run_job",
                "send_email",
                "write_file",
                "write_worklog"
            ]
        );
    }

    #[test]
    fn helpers() {
        assert_eq!(norm_path("./logs/a"), "logs/a");
        assert_eq!(norm_path("/logs/a"), "logs/a");
        assert_eq!(norm_path("//logs/a"), "/logs/a");
        assert_eq!(money(7200.0), "7,200.00");
        assert_eq!(money(89.0), "89.00");
        assert_eq!(money(1234567.891), "1,234,567.89");
        assert_eq!(money(-5.5), "-5.50");
        assert_eq!(words(" a  b\nc "), 3);
    }

    #[test]
    fn signature_uses_scenario_notation() {
        let sig = signature(&send_email::spec().parameters);
        assert_eq!(sig["to"], "string[]");
        assert_eq!(sig["cc"], "string[]?");
        assert_eq!(sig["subject"], "string");
    }
}
