//! `reclassify_expense`: move an expense to another account (a code in `ledger.toml`).
//! Side effect: the expense's current account changes. World files: `expenses.toml`,
//! `ledger.toml`.

use serde_json::json;

use super::list_expenses::account_label;
use super::{Args, ToolSpec, err, money};
use crate::world::World;

/// Spec.
pub fn spec() -> ToolSpec {
    ToolSpec {
        name: "reclassify_expense",
        description:
            "Move an expense to another account (4-digit code from the chart of accounts).".into(),
        parameters: json!({"type": "object",
            "properties": {
                "expense_id": {"type": "string"},
                "new_category": {"type": "string", "description": "4-digit account code."}
            },
            "required": ["expense_id", "new_category"]}),
        side_effect: true,
        needs: &["expenses", "ledger"],
    }
}

/// Change the account, or report that it is unchanged.
pub fn run(world: &mut World, args: &Args) -> Result<String, String> {
    let id = args.str("expense_id")?.trim().to_string();
    let code = args.str("new_category")?.trim().to_string();
    let Some(i) = world.expenses.iter().position(|e| e.def.id == id) else {
        return err(format!("no expense {id}"));
    };
    if world
        .ledger
        .as_ref()
        .and_then(|l| l.account(&code))
        .is_none()
    {
        return err(format!(
            "unknown account '{code}'. Use a 4-digit code from the chart of accounts."
        ));
    }
    let old = world.expenses[i].account.clone();
    let amount = world.expenses[i].def.amount;
    if old == code {
        return Ok(format!(
            "{id} already in {}; no change",
            account_label(world, &code)
        ));
    }
    world.expenses[i].account = code.clone();
    Ok(format!(
        "{id} reclassified: {} -> {} ({})",
        account_label(world, &old),
        account_label(world, &code),
        money(amount)
    ))
}
