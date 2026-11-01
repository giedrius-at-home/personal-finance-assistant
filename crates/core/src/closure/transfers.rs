//! The bank transfers a split needs (Python `domain/month_closure.py`):
//! where each line's money has to move, the route through via accounts, and
//! ticking transfers off once the bank shows them.

use std::collections::{HashMap, HashSet};

use finance_types::gateway::{AccountRole, PotRecord};
use finance_types::report::closure::{
    AllocationLine, TransferCandidate, TransferHop, TransferStatus,
};
use jiff::Timestamp;
use jiff::tz::TimeZone;
use rust_decimal::Decimal;

use crate::model::{Kind, Tx};

/// `account → via account` from the saved roles.
pub fn via_routes(roles: &[AccountRole]) -> HashMap<String, String> {
    roles
        .iter()
        .filter_map(|r| Some((r.account_id.clone(), r.via_account_id.clone()?)))
        .collect()
}

/// Accounts a move passes through: out of `source` through its via account,
/// into `target` through its via account (each at most once).
pub fn transfer_path(source: &str, target: &str, via: &HashMap<String, String>) -> Vec<String> {
    let mut path = vec![source.to_owned()];
    for hop in [via.get(source), via.get(target)].into_iter().flatten() {
        if path.last() != Some(hop) && hop != target {
            path.push(hop.clone());
        }
    }
    path.push(target.to_owned());
    path
}

/// Split a pending transfer into hops when an end is reached via another account.
fn routed(item: AllocationLine, via: &HashMap<String, String>) -> AllocationLine {
    let Some(target) = item.to_account_id.as_deref() else {
        return item;
    };
    let source = item
        .transfer_source_account_id
        .as_deref()
        .unwrap_or(&item.from_account_id);
    let path = transfer_path(source, target, via);
    if path.len() <= 2 {
        return item;
    }
    let hops = path
        .windows(2)
        .map(|pair| TransferHop {
            from_account_id: pair[0].clone(),
            to_account_id: pair[1].clone(),
            amount: item.amount.abs(),
            transfer_status: TransferStatus::Pending,
            matched_transaction_ids: Vec::new(),
            manual: false,
            moved_amount: None,
        })
        .collect();
    AllocationLine {
        transfer_hops: hops,
        ..item
    }
}

/// Mark the lines whose money has to move to another account as pending
/// transfers: into a pot's account, out of it for a take, into the savings
/// account that holds a goal, or into the account a move names; routed
/// through via accounts.
pub fn with_transfer_targets(
    lines: Vec<AllocationLine>,
    roles: &[AccountRole],
    pots: &[PotRecord],
) -> Vec<AllocationLine> {
    let savings_by_goal: HashMap<&str, &str> = roles
        .iter()
        .filter(|r| r.role == "savings")
        .filter_map(|r| {
            Some((
                r.goal_id.as_deref().filter(|g| !g.is_empty())?,
                r.account_id.as_str(),
            ))
        })
        .collect();
    let via = via_routes(roles);
    let pot_accounts: HashMap<&str, Option<&str>> = pots
        .iter()
        .map(|p| (p.pot_id.as_str(), p.account_id.as_deref()))
        .collect();
    lines
        .into_iter()
        .map(|item| {
            if let Some(pot) = item.pot_id() {
                let located = pot_accounts.get(pot).copied().flatten();
                return match located {
                    Some(at) if at != item.from_account_id && !item.amount.is_zero() => {
                        if item.amount > Decimal::ZERO {
                            AllocationLine {
                                to_account_id: Some(at.to_owned()),
                                transfer_status: TransferStatus::Pending,
                                ..item
                            }
                        } else {
                            // A take moves money from the pot's account to this one.
                            AllocationLine {
                                to_account_id: Some(item.from_account_id.clone()),
                                transfer_source_account_id: Some(at.to_owned()),
                                transfer_status: TransferStatus::Pending,
                                ..item
                            }
                        }
                    }
                    _ => item,
                };
            }
            let target = item
                .target_account_id()
                .or_else(|| item.goal_id().and_then(|g| savings_by_goal.get(g).copied()));
            match target {
                Some(at) if at != item.from_account_id && item.amount > Decimal::ZERO => {
                    AllocationLine {
                        to_account_id: Some(at.to_owned()),
                        transfer_status: TransferStatus::Pending,
                        ..item
                    }
                }
                _ => item,
            }
        })
        .map(|item| {
            if item.transfer_status == TransferStatus::Pending {
                routed(item, &via)
            } else {
                item
            }
        })
        .collect()
}

/// Re-plan transfers nobody has made yet after a via route changes; a line
/// with any matched or hand-marked part keeps the path it was made on.
pub fn reroute_pending(lines: &[AllocationLine], roles: &[AccountRole]) -> Vec<AllocationLine> {
    let via = via_routes(roles);
    lines
        .iter()
        .map(|item| {
            let untouched = item.transfer_status == TransferStatus::Pending
                && !item.manual
                && item
                    .transfer_hops
                    .iter()
                    .all(|h| h.transfer_status == TransferStatus::Pending);
            if untouched {
                routed(
                    AllocationLine {
                        transfer_hops: Vec::new(),
                        ..item.clone()
                    },
                    &via,
                )
            } else {
                item.clone()
            }
        })
        .collect()
}

type TransferKey = (String, Option<String>, Decimal, String);

fn key(t: &Tx) -> TransferKey {
    (
        t.account_id.clone(),
        t.counterparty_account_id.clone(),
        t.amount,
        t.currency.code.clone(),
    )
}

/// Own-account transfers booked on or after the settle day, oldest first.
/// `transactions` come in the Python order (by transaction id).
pub fn own_transfers(
    transactions: &[&Tx],
    settled_at: Timestamp,
    zone: &TimeZone,
) -> Vec<TransferCandidate> {
    // Paired debit and credit legs (up to days apart) are chunked into twos
    // and consumed as one bank transfer; every other leg is a transfer on its own.
    let settled_on = settled_at.to_zoned(zone.clone()).date();
    let mut paired: Vec<(TransferKey, Vec<&Tx>)> = Vec::new();
    let mut found: Vec<Vec<&Tx>> = Vec::new();
    for t in transactions
        .iter()
        .copied()
        .filter(|t| t.kind == Kind::Transfer)
    {
        if t.transfer_evidence == Some("paired_entries") {
            let k = key(t);
            match paired.iter_mut().find(|(g, _)| *g == k) {
                Some((_, legs)) => legs.push(t),
                None => paired.push((k, vec![t])),
            }
        } else {
            found.push(vec![t]);
        }
    }
    for (_, mut legs) in paired {
        // Both legs carry the same key once paired, so interleaved transfers
        // may chunk legs of different transfers together; the count of
        // consumable transfers, which is what matching needs, holds.
        legs.sort_by_key(|leg| leg.occurred_at.timestamp());
        found.extend(legs.chunks(2).map(<[&Tx]>::to_vec));
    }
    let mut result: Vec<(Timestamp, TransferCandidate)> = Vec::new();
    for legs in found {
        let Some(first) = legs.iter().copied().reduce(|a, b| {
            if b.occurred_at.timestamp() < a.occurred_at.timestamp() {
                b
            } else {
                a
            }
        }) else {
            continue;
        };
        let booked_on = first.occurred_at.timestamp().to_zoned(zone.clone()).date();
        if booked_on < settled_on {
            continue;
        }
        result.push((
            first.occurred_at.timestamp(),
            TransferCandidate {
                transaction_ids: legs.iter().map(|l| l.transaction_id.clone()).collect(),
                from_account_id: first.account_id.clone(),
                to_account_id: first.counterparty_account_id.clone(),
                amount: first.amount,
                currency: first.currency.clone(),
                booked_on: booked_on.to_string(),
                description: legs.iter().find_map(|l| l.description.clone()),
            },
        ));
    }
    result.sort_by_key(|(at, _)| *at);
    result.into_iter().map(|(_, c)| c).collect()
}

/// A routed line is done once every hop is.
fn line_status(item: AllocationLine) -> AllocationLine {
    if item.transfer_hops.is_empty() {
        return item;
    }
    let done = item
        .transfer_hops
        .iter()
        .all(|h| h.transfer_status == TransferStatus::Matched);
    AllocationLine {
        transfer_status: if done {
            TransferStatus::Matched
        } else {
            TransferStatus::Pending
        },
        ..item
    }
}

/// Tick off pending transfers (and hops) that one bank transfer matches
/// exactly; a transfer already used by any line or hop is never reused.
pub fn match_transfers(
    lines: &[AllocationLine],
    transactions: &[&Tx],
    settled_at: Timestamp,
    zone: &TimeZone,
) -> Vec<AllocationLine> {
    let mut available: HashMap<TransferKey, Vec<Vec<String>>> = HashMap::new();
    for c in own_transfers(transactions, settled_at, zone) {
        available
            .entry((
                c.from_account_id.clone(),
                c.to_account_id.clone(),
                c.amount,
                c.currency.code.clone(),
            ))
            .or_default()
            .push(c.transaction_ids);
    }
    let mut used: HashSet<String> = lines
        .iter()
        .flat_map(|l| {
            l.matched_transaction_ids.iter().chain(
                l.transfer_hops
                    .iter()
                    .flat_map(|h| h.matched_transaction_ids.iter()),
            )
        })
        .cloned()
        .collect();
    let mut take = |source: &str, target: Option<&str>, amount: Decimal, code: &str| {
        let options = available.get(&(
            source.to_owned(),
            target.map(str::to_owned),
            amount.abs(),
            code.to_owned(),
        ))?;
        let found = options
            .iter()
            .find(|ids| !ids.iter().any(|i| used.contains(i)))?
            .clone();
        used.extend(found.iter().cloned());
        Some(found)
    };
    lines
        .iter()
        .map(|item| {
            if item.transfer_status != TransferStatus::Pending {
                return item.clone();
            }
            let code = item.currency.code.as_str();
            if !item.transfer_hops.is_empty() {
                let hops = item
                    .transfer_hops
                    .iter()
                    .map(|hop| {
                        let found = (hop.transfer_status == TransferStatus::Pending)
                            .then(|| {
                                take(
                                    &hop.from_account_id,
                                    Some(&hop.to_account_id),
                                    hop.amount,
                                    code,
                                )
                            })
                            .flatten();
                        match found {
                            Some(ids) => TransferHop {
                                transfer_status: TransferStatus::Matched,
                                matched_transaction_ids: ids,
                                ..hop.clone()
                            },
                            None => hop.clone(),
                        }
                    })
                    .collect();
                return line_status(AllocationLine {
                    transfer_hops: hops,
                    ..item.clone()
                });
            }
            let source = item
                .transfer_source_account_id
                .as_deref()
                .unwrap_or(&item.from_account_id);
            match take(source, item.to_account_id.as_deref(), item.amount, code) {
                Some(ids) => AllocationLine {
                    transfer_status: TransferStatus::Matched,
                    matched_transaction_ids: ids,
                    ..item.clone()
                },
                None => item.clone(),
            }
        })
        .collect()
}

fn transfer_line<'a>(
    lines: &'a [AllocationLine],
    line_id: &str,
    hop: Option<usize>,
) -> Result<(usize, &'a AllocationLine), String> {
    let found = lines
        .iter()
        .enumerate()
        .find(|(_, l)| l.line_id == line_id)
        .filter(|(_, l)| l.transfer_status != TransferStatus::NotNeeded);
    let Some((index, item)) = found else {
        return Err("no transfer on that line".into());
    };
    if hop.is_none() && !item.transfer_hops.is_empty() {
        return Err("this transfer is routed; mark each hop".into());
    }
    if hop.is_some_and(|h| h >= item.transfer_hops.len()) {
        return Err("unknown hop".into());
    }
    Ok((index, item))
}

/// The owner marks a hop (or a direct transfer, `hop: None`) done with one
/// or more bank transfers between the right accounts, at any amount. One
/// bank transfer may cover several hops.
pub fn mark_transfer(
    lines: &[AllocationLine],
    line_id: &str,
    hop: Option<usize>,
    transaction_ids: &[String],
    candidates: &[TransferCandidate],
) -> Result<Vec<AllocationLine>, String> {
    let (index, item) = transfer_line(lines, line_id, hop)?;
    if transaction_ids.is_empty() {
        return Err("pick at least one transfer".into());
    }
    let (source, target) = match hop {
        None => (
            item.transfer_source_account_id
                .as_deref()
                .unwrap_or(&item.from_account_id),
            item.to_account_id.as_deref(),
        ),
        Some(h) => (
            item.transfer_hops[h].from_account_id.as_str(),
            Some(item.transfer_hops[h].to_account_id.as_str()),
        ),
    };
    let mut picked: Vec<&TransferCandidate> = Vec::new();
    for id in transaction_ids {
        let Some(found) = candidates.iter().find(|c| c.transaction_ids.contains(id)) else {
            return Err("unknown transfer or booked before the split".into());
        };
        if found.from_account_id != source
            || found.to_account_id.as_deref() != target
            || found.currency != item.currency
        {
            return Err("transfer is between other accounts".into());
        }
        if !picked
            .iter()
            .any(|p| p.transaction_ids == found.transaction_ids)
        {
            picked.push(found);
        }
    }
    let ids: Vec<String> = picked
        .iter()
        .flat_map(|c| c.transaction_ids.iter().cloned())
        .collect();
    let moved = picked
        .iter()
        .fold(Decimal::ZERO, |total, c| total + c.amount);
    let marked = match hop {
        None => AllocationLine {
            transfer_status: TransferStatus::Matched,
            matched_transaction_ids: ids,
            manual: true,
            moved_amount: Some(moved),
            ..item.clone()
        },
        Some(h) => {
            let mut hops = item.transfer_hops.clone();
            hops[h] = TransferHop {
                transfer_status: TransferStatus::Matched,
                matched_transaction_ids: ids,
                manual: true,
                moved_amount: Some(moved),
                ..hops[h].clone()
            };
            line_status(AllocationLine {
                transfer_hops: hops,
                ..item.clone()
            })
        }
    };
    let mut result = lines.to_vec();
    result[index] = marked;
    Ok(result)
}

/// Undo a manual mark; the transfer can then match (or be marked) again.
pub fn unmark_transfer(
    lines: &[AllocationLine],
    line_id: &str,
    hop: Option<usize>,
) -> Result<Vec<AllocationLine>, String> {
    let (index, item) = transfer_line(lines, line_id, hop)?;
    let manual = match hop {
        None => item.manual,
        Some(h) => item.transfer_hops[h].manual,
    };
    if !manual {
        return Err("only a transfer marked by hand can be unmarked".into());
    }
    let cleared = match hop {
        None => AllocationLine {
            transfer_status: TransferStatus::Pending,
            matched_transaction_ids: Vec::new(),
            manual: false,
            moved_amount: None,
            ..item.clone()
        },
        Some(h) => {
            let mut hops = item.transfer_hops.clone();
            hops[h] = TransferHop {
                transfer_status: TransferStatus::Pending,
                matched_transaction_ids: Vec::new(),
                manual: false,
                moved_amount: None,
                ..hops[h].clone()
            };
            line_status(AllocationLine {
                transfer_hops: hops,
                ..item.clone()
            })
        }
    };
    let mut result = lines.to_vec();
    result[index] = cleared;
    Ok(result)
}
