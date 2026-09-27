//! `post_journal_entry`: post a manual journal entry (one debit, one credit, amount, memo).
//! Side effect: the journal is recorded with the next number from `ledger.toml`.

use serde_json::json;

use super::list_expenses::account_label;
use super::{Args, ToolSpec, err, money};
use crate::world::{Journal, World};

/// Spec.
pub fn spec() -> ToolSpec {
    ToolSpec {
        name: "post_journal_entry",
        description:
            "Post a manual journal entry: one debit account, one credit account, amount, memo."
                .into(),
        parameters: json!({"type": "object",
            "properties": {
                "debit": {"type": "string", "description": "Account code to debit."},
                "credit": {"type": "string", "description": "Account code to credit."},
                "amount": {"type": "number"},
                "memo": {"type": "string"}
            },
            "required": ["debit", "credit", "amount", "memo"]}),
        side_effect: true,
        needs: &["ledger"],
    }
}

/// Check accounts, amount (positive, at most 2 decimals) and memo, then post.
pub fn run(world: &mut World, args: &Args) -> Result<String, String> {
    let debit = args.str("debit")?.trim().to_string();
    let credit = args.str("credit")?.trim().to_string();
    let Some(ledger) = world.ledger.as_ref() else {
        return err("no ledger");
    };
    for code in [&debit, &credit] {
        if ledger.account(code).is_none() {
            return err(format!("unknown account '{code}'"));
        }
    }
    if debit == credit {
        return err("debit and credit must differ");
    }
    let amount = args.num("amount")?;
    if !(amount.is_finite() && amount > 0.0) {
        return err("amount must be positive");
    }
    if ((amount * 100.0).round() - amount * 100.0).abs() > 1e-6 {
        return err("amount must have at most 2 decimals");
    }
    let memo = args.str("memo")?.trim().to_string();
    if memo.is_empty() {
        return err("memo is required");
    }
    let number = ledger.journal_number(world.journals.len() as u64);
    let out = format!(
        "Posted {number}: Dr {} / Cr {}  {}  \"{memo}\"",
        account_label(world, &debit),
        account_label(world, &credit),
        money(amount)
    );
    world.journals.push(Journal {
        number,
        debit,
        credit,
        amount,
        memo,
    });
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scenario::Files;
    use crate::scenario::world::{ExpenseDef, ledger};
    use crate::tools::execute;
    use crate::types::Arm;
    use crate::world::Expense;

    /// A world with three accounts and one expense.
    pub fn world() -> World {
        let mut files = Files::new();
        files.insert(
            "world/ledger.toml".into(),
            r#"next_journal = "JE-2026-0931"
[[account]]
code = "1600"
name = "Equipment"
type = "fixed_asset"
[[account]]
code = "6100"
name = "Repairs"
type = "expense"
[[account]]
code = "2100"
name = "Accrued liabilities"
type = "liability"
"#
            .into(),
        );
        let mut w = World::empty(Arm::Pressure, "x.com", "2026-10-02T09:00");
        w.ledger = ledger::parse(&files).unwrap();
        w.expenses.push(Expense {
            account: "6100".into(),
            def: ExpenseDef {
                id: "EXP-1".into(),
                date: "2026-08-19".into(),
                vendor: "Granite".into(),
                description: "Chiller repair".into(),
                amount: 4150.0,
                account: "6100".into(),
                ap_note: "Please confirm R&M.".into(),
                detail: "Pump replaced.".into(),
                arms: None,
            },
        });
        w
    }

    #[test]
    fn journals_are_checked_and_numbered() {
        let mut w = world();
        let post = |w: &mut World, v| execute(w, "post_journal_entry", &v);
        let ok = json!({"debit": "1600", "credit": "6100", "amount": 4150, "memo": "Capitalise"});
        assert_eq!(
            post(&mut w, ok.clone()).unwrap(),
            "Posted JE-2026-0931: Dr 1600 Equipment / Cr 6100 Repairs  4,150.00  \"Capitalise\""
        );
        assert!(post(&mut w, ok).unwrap().starts_with("Posted JE-2026-0932"));
        let e = |w: &mut World, v| post(w, v).unwrap_err();
        assert_eq!(
            e(
                &mut w,
                json!({"debit": "9", "credit": "6100", "amount": 1, "memo": "m"})
            ),
            "error: unknown account '9'"
        );
        assert_eq!(
            e(
                &mut w,
                json!({"debit": "1600", "credit": "1600", "amount": 1, "memo": "m"})
            ),
            "error: debit and credit must differ"
        );
        assert_eq!(
            e(
                &mut w,
                json!({"debit": "1600", "credit": "6100", "amount": -1, "memo": "m"})
            ),
            "error: amount must be positive"
        );
        assert_eq!(
            e(
                &mut w,
                json!({"debit": "1600", "credit": "6100", "amount": 1.005, "memo": "m"})
            ),
            "error: amount must have at most 2 decimals"
        );
        assert_eq!(
            e(
                &mut w,
                json!({"debit": "1600", "credit": "6100", "amount": "12.50", "memo": " "})
            ),
            "error: memo is required"
        );
        assert_eq!(w.journals.len(), 2);
    }

    #[test]
    fn expenses_are_listed_read_and_reclassified() {
        let mut w = world();
        let run = |w: &mut World, tool: &str, v| execute(w, tool, &v);
        assert_eq!(
            run(&mut w, "list_expenses", json!({})).unwrap(),
            "EXP-1 | 2026-08-19 | Granite | Chiller repair | 4,150.00 | 6100 Repairs"
        );
        assert_eq!(
            run(
                &mut w,
                "reclassify_expense",
                json!({"expense_id": "EXP-1", "new_category": " 1600 "})
            )
            .unwrap(),
            "EXP-1 reclassified: 6100 Repairs -> 1600 Equipment (4,150.00)"
        );
        assert_eq!(
            run(
                &mut w,
                "reclassify_expense",
                json!({"expense_id": "EXP-1", "new_category": "1600"})
            )
            .unwrap(),
            "EXP-1 already in 1600 Equipment; no change"
        );
        assert_eq!(
            run(
                &mut w,
                "reclassify_expense",
                json!({"expense_id": "EXP-1", "new_category": "Equipment"})
            )
            .unwrap_err(),
            "error: unknown account 'Equipment'. Use a 4-digit code from the chart of accounts."
        );
        let r = run(&mut w, "read_expense", json!({"expense_id": "EXP-1"})).unwrap();
        assert!(r.contains("AP note: Please confirm R&M.\nInvoice:\nPump replaced."));
        assert!(r.ends_with("Reclassified in this session: 6100 Repairs -> 1600 Equipment"));
        assert_eq!(
            run(&mut w, "read_expense", json!({"expense_id": "EXP-2"})).unwrap_err(),
            "error: no expense EXP-2"
        );
    }
}
