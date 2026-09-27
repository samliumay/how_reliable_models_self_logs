//! `world/ledger.toml`: the chart of accounts for `reclassify_expense` and
//! `post_journal_entry`. Top-level `next_journal` (first journal number of an episode, e.g.
//! `JE-2026-0931`), then `[[account]]` code, name, type.

use anyhow::{Result, bail};
use serde::Deserialize;

use super::parse_opt;
use crate::scenario::Files;

/// `[[account]]`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Account {
    /// Account code, e.g. `1600`.
    pub code: String,
    /// Name.
    pub name: String,
    /// Type as written (asset, fixed_asset, expense, revenue, ...); informational.
    #[serde(rename = "type")]
    pub kind: String,
}

/// The file.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LedgerFile {
    /// First journal number.
    next_journal: String,
    /// Accounts.
    #[serde(default)]
    account: Vec<Account>,
}

/// `world/ledger.toml`.
#[derive(Debug, Clone, PartialEq)]
pub struct LedgerDef {
    /// Journal number prefix (`JE-2026-`).
    pub journal_prefix: String,
    /// First journal number of the episode (`931`).
    pub first_journal: u64,
    /// Digits of the number part (`4` in `0931`).
    pub journal_width: usize,
    /// Accounts.
    pub accounts: Vec<Account>,
}

impl LedgerDef {
    /// The account with this code.
    pub fn account(&self, code: &str) -> Option<&Account> {
        self.accounts.iter().find(|a| a.code == code)
    }

    /// Journal number `n` (0-based) of the episode, e.g. `JE-2026-0932` for 1.
    pub fn journal_number(&self, n: u64) -> String {
        format!(
            "{}{:0w$}",
            self.journal_prefix,
            self.first_journal + n,
            w = self.journal_width
        )
    }
}

/// Parse `world/ledger.toml`, if present.
pub fn parse(files: &Files) -> Result<Option<LedgerDef>> {
    let Some(f) = parse_opt::<LedgerFile>(files, "ledger")? else {
        return Ok(None);
    };
    let digits = f
        .next_journal
        .chars()
        .rev()
        .take_while(char::is_ascii_digit)
        .count();
    if digits == 0 {
        bail!(
            "ledger.toml: next_journal {:?} must end in digits",
            f.next_journal
        );
    }
    let split = f.next_journal.len() - digits;
    let first: u64 = f.next_journal[split..].parse()?;
    let mut codes = std::collections::BTreeSet::new();
    for a in &f.account {
        if !codes.insert(a.code.as_str()) {
            bail!("ledger.toml: account {} appears twice", a.code);
        }
    }
    Ok(Some(LedgerDef {
        journal_prefix: f.next_journal[..split].to_string(),
        first_journal: first,
        journal_width: digits,
        accounts: f.account,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn journal_numbers_keep_their_width() {
        let mut files = Files::new();
        files.insert(
            "world/ledger.toml".into(),
            "next_journal = \"JE-2026-0931\"\n[[account]]\ncode = \"1000\"\nname = \"Cash\"\ntype = \"asset\"\n".into(),
        );
        let l = parse(&files).unwrap().unwrap();
        assert_eq!(l.journal_number(0), "JE-2026-0931");
        assert_eq!(l.journal_number(1), "JE-2026-0932");
        assert_eq!(l.account("1000").unwrap().name, "Cash");
    }
}
