//! `read_expense`: one expense with the AP note, the invoice detail and any reclassification
//! made in this episode. World files: `expenses.toml`, `ledger.toml`.

use serde_json::json;

use super::list_expenses::{account_label, line};
use super::{Args, ToolSpec, err};
use crate::world::World;

/// Spec.
pub fn spec() -> ToolSpec {
    ToolSpec {
        name: "read_expense",
        description: "Read one expense with the AP note and invoice detail.".into(),
        parameters: json!({"type": "object",
            "properties": {"expense_id": {"type": "string", "description": "e.g. EXP-24012"}},
            "required": ["expense_id"]}),
        side_effect: false,
        needs: &["expenses"],
    }
}

/// The list line, `AP note:` (if any), `Invoice:` detail, reclassification (if any).
pub fn run(world: &mut World, args: &Args) -> Result<String, String> {
    let id = args.str("expense_id")?.trim();
    let Some(e) = world.expenses.iter().find(|e| e.def.id == id) else {
        return err(format!("no expense {id}"));
    };
    let mut out = line(world, e);
    if !e.def.ap_note.trim().is_empty() {
        out.push_str(&format!("\nAP note: {}", e.def.ap_note.trim()));
    }
    if !e.def.detail.trim().is_empty() {
        out.push_str(&format!("\nInvoice:\n{}", e.def.detail.trim()));
    }
    if e.account != e.def.account {
        out.push_str(&format!(
            "\nReclassified in this session: {} -> {}",
            account_label(world, &e.def.account),
            account_label(world, &e.account)
        ));
    }
    Ok(out)
}
