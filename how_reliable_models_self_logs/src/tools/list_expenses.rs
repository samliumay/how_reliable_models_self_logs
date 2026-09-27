//! `list_expenses`: every expense under review with its current account. World files:
//! `expenses.toml` (+ `ledger.toml` for account names).

use serde_json::json;

use super::{Args, ToolSpec, money};
use crate::world::{Expense, World};

/// Spec.
pub fn spec() -> ToolSpec {
    ToolSpec {
        name: "list_expenses",
        description:
            "List Q3 expenses flagged for review (id, date, vendor, description, amount, account)."
                .into(),
        parameters: json!({"type": "object", "properties": {}}),
        side_effect: false,
        needs: &["expenses"],
    }
}

/// `<code> <name>` of an account (code alone if the ledger does not know it).
pub fn account_label(world: &World, code: &str) -> String {
    match world.ledger.as_ref().and_then(|l| l.account(code)) {
        Some(a) => format!("{} {}", a.code, a.name),
        None => code.to_string(),
    }
}

/// `<id> | <date> | <vendor> | <description> | <amount> | <account code> <account name>`.
pub fn line(world: &World, e: &Expense) -> String {
    format!(
        "{} | {} | {} | {} | {} | {}",
        e.def.id,
        e.def.date,
        e.def.vendor,
        e.def.description,
        money(e.def.amount),
        account_label(world, &e.account)
    )
}

/// One line per expense.
pub fn run(world: &mut World, _args: &Args) -> Result<String, String> {
    if world.expenses.is_empty() {
        return Ok("(no expenses)".into());
    }
    Ok(world
        .expenses
        .iter()
        .map(|e| line(world, e))
        .collect::<Vec<_>>()
        .join("\n"))
}
