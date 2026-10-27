//! Pair the two legs of a move between own accounts that the banks booked as
//! separate spending and income.

use std::collections::HashMap;
use std::sync::LazyLock;

use jiff::SignedDuration;
use regex::Regex;

use crate::model::{Account, Kind, Tx};
use crate::text::casefold;

/// Fail closed on ambiguous pairs; names or equal amounts alone are not enough.
pub fn resolve_own_transfers(transactions: Vec<Tx>, accounts: &[Account]) -> Vec<Tx> {
    static TOP_UP: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\b(?:top[ -]?up|card deposit)\b").expect("valid regex"));
    let names: HashMap<&str, String> = accounts
        .iter()
        .map(|a| {
            let base = a.name.split('(').next().unwrap_or("").trim();
            (a.account_id.as_str(), casefold(base))
        })
        .collect();
    let mut incoming: HashMap<(String, String), Vec<usize>> = HashMap::new();
    for (i, t) in transactions.iter().enumerate() {
        if t.kind == Kind::Inflow && names.contains_key(t.account_id.as_str()) {
            incoming
                .entry((t.currency.code.clone(), t.amount.normalize().to_string()))
                .or_default()
                .push(i);
        }
    }
    let mut candidates: Vec<(usize, Vec<usize>)> = Vec::new();
    let mut usage: HashMap<usize, u32> = HashMap::new();
    let window = SignedDuration::from_hours(72);
    for (d, debit) in transactions.iter().enumerate() {
        let Some(debit_name) = names.get(debit.account_id.as_str()) else {
            continue;
        };
        if debit.kind != Kind::Outflow {
            continue;
        }
        let text = casefold(debit.description.as_deref().unwrap_or(""));
        let mut matches = Vec::new();
        let key = (
            debit.currency.code.clone(),
            debit.amount.normalize().to_string(),
        );
        for &c in incoming.get(&key).map(Vec::as_slice).unwrap_or(&[]) {
            let credit = &transactions[c];
            let gap = credit
                .occurred_at
                .timestamp()
                .duration_since(debit.occurred_at.timestamp());
            if credit.account_id == debit.account_id || gap.abs() > window {
                continue;
            }
            let credit_name = &names[credit.account_id.as_str()];
            let credit_text = casefold(credit.description.as_deref().unwrap_or(""));
            let named = Regex::new(&format!(r"\b{}\b", regex::escape(credit_name)))
                .is_ok_and(|re| re.is_match(&text));
            let named_top_up = named && TOP_UP.is_match(&credit_text);
            let named_bank_transfer = text.starts_with(&format!("sent from {debit_name} to "));
            if named_top_up || named_bank_transfer {
                matches.push(c);
                *usage.entry(c).or_default() += 1;
            }
        }
        candidates.push((d, matches));
    }
    let mut updates: HashMap<usize, (String, String)> = HashMap::new();
    for (d, matches) in candidates {
        if matches.len() != 1 || usage.get(&matches[0]).copied() != Some(1) {
            continue;
        }
        let (debit, credit) = (&transactions[d], &transactions[matches[0]]);
        let pair = (debit.account_id.clone(), credit.account_id.clone());
        updates.insert(d, pair.clone());
        updates.insert(matches[0], pair);
    }
    transactions
        .into_iter()
        .enumerate()
        .map(|(i, mut t)| {
            if let Some((from, to)) = updates.remove(&i) {
                t.kind = Kind::Transfer;
                t.account_id = from;
                t.counterparty_account_id = Some(to);
                t.transfer_evidence = Some("paired_entries");
            }
            t
        })
        .collect()
}
