//! `world/expenses.toml`: expenses flagged for review. `[[expense]]` id, date, vendor,
//! description, amount, account (a code in `ledger.toml`), ap_note, detail, arms.

use anyhow::{Result, bail};
use serde::Deserialize;

use super::{parse_opt, unique_per_arm};
use crate::scenario::Files;
use crate::types::Arm;

/// `[[expense]]` as written.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawExpense {
    /// Id, e.g. `EXP-24012`.
    id: String,
    /// `YYYY-MM-DD`.
    date: String,
    /// Vendor.
    vendor: String,
    /// Short description.
    description: String,
    /// Amount.
    amount: f64,
    /// Account code at the start.
    account: String,
    /// AP's note (may be empty).
    #[serde(default)]
    ap_note: String,
    /// Invoice detail.
    #[serde(default)]
    detail: String,
    /// Arms it appears in.
    arms: Option<Vec<Arm>>,
}

/// The file.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExpensesFile {
    /// The expenses.
    #[serde(default)]
    expense: Vec<RawExpense>,
}

/// An expense.
#[derive(Debug, Clone, PartialEq)]
pub struct ExpenseDef {
    /// Id.
    pub id: String,
    /// Date.
    pub date: String,
    /// Vendor.
    pub vendor: String,
    /// Description.
    pub description: String,
    /// Amount.
    pub amount: f64,
    /// Account code at the start of the episode.
    pub account: String,
    /// AP's note.
    pub ap_note: String,
    /// Invoice detail.
    pub detail: String,
    /// Arms it appears in (`None`: both).
    pub arms: Option<Vec<Arm>>,
}

/// Parse `world/expenses.toml`, if present.
pub fn parse(files: &Files) -> Result<Option<Vec<ExpenseDef>>> {
    let Some(f) = parse_opt::<ExpensesFile>(files, "expenses")? else {
        return Ok(None);
    };
    let out = f
        .expense
        .into_iter()
        .map(|e| {
            chrono::NaiveDate::parse_from_str(&e.date, "%Y-%m-%d").map_err(|_| {
                anyhow::anyhow!("expense {}: date {:?} is not YYYY-MM-DD", e.id, e.date)
            })?;
            if !(e.amount.is_finite() && e.amount > 0.0) {
                bail!("expense {}: amount must be positive", e.id);
            }
            Ok(ExpenseDef {
                id: e.id,
                date: e.date,
                vendor: e.vendor,
                description: e.description,
                amount: e.amount,
                account: e.account,
                ap_note: e.ap_note,
                detail: e.detail,
                arms: e.arms,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    unique_per_arm("expense", out.iter().map(|e| (e.id.as_str(), &e.arms)))?;
    Ok(Some(out))
}
