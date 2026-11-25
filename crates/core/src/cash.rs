//! Cash overview: payday status and per-account record health (Python domain/cash_overview.py).

use std::collections::{BTreeMap, HashMap};

use finance_types::report::cash::{AccountHealth, CashOverview, PaydayStatus};
use finance_types::report::reporting::CurrencyWeek;
use finance_types::report::{Currency, UnitMoney};
use jiff::civil::Date;
use jiff::{Timestamp, ToSpan, Zoned};
use rust_decimal::Decimal;

use crate::model::{Account, Kind, Tx};
use crate::period::shift_month;
use crate::text::casefold;

/// Salary booked this many days before or after payday still counts for it.
const EARLY_DAYS: i64 = 7;
const LATE_DAYS: i64 = 3;

fn payday_in(month: Date, day: u8) -> Date {
    let day = i8::try_from(day).unwrap_or(31).min(month.days_in_month());
    month.with().day(day).build().unwrap_or(month)
}

/// The current pay period `[start, next payday)`. Payday is clamped to the
/// month's last day, and payday itself starts the new period.
pub fn payday_window(today: Date, day: u8) -> (Date, Date) {
    let day = day.clamp(1, 31);
    let this_month = today.first_of_month();
    let this = payday_in(this_month, day);
    if today >= this {
        (this, payday_in(shift_month(this_month, 1), day))
    } else {
        (payday_in(shift_month(this_month, -1), day), this)
    }
}

/// Cash flow since the last payday and what the ledger holds per account.
/// Own transfers never count as income or spending. `refreshed_at` is when
/// the ledger was last refreshed (the ledger keeps no per-account time).
pub fn cash_overview(
    accounts: &[Account],
    transactions: &[Tx],
    now: &Zoned,
    payday_day: Option<u8>,
    refreshed_at: Option<Timestamp>,
) -> CashOverview {
    let zone = now.time_zone().clone();
    let today = now.date();
    let local = |t: &Tx| t.occurred_at.with_time_zone(zone.clone()).date();
    let window = payday_day
        .filter(|d| (1..=31).contains(d))
        .map(|d| payday_window(today, d));
    let mut selected: Vec<&Account> = accounts.iter().collect();
    selected.sort_by(|a, b| a.account_id.cmp(&b.account_id));
    let mut by_account: HashMap<&str, Vec<&Tx>> = selected
        .iter()
        .map(|a| (a.account_id.as_str(), Vec::new()))
        .collect();

    #[derive(Default)]
    struct Flow {
        income: Decimal,
        spending: Decimal,
        transfers: u64,
    }
    let mut flows: BTreeMap<Currency, Flow> = BTreeMap::new();
    for t in transactions {
        if t.occurred_at.timestamp() > now.timestamp() {
            continue;
        }
        let mut touched = vec![t.account_id.as_str()];
        if t.kind == Kind::Transfer {
            let other = t.counterparty_account_id.as_deref().unwrap_or("");
            if other != t.account_id {
                touched.push(other);
            }
        }
        let mut relevant = false;
        for key in touched {
            if let Some(list) = by_account.get_mut(key) {
                list.push(t);
                relevant = true;
            }
        }
        if !relevant {
            continue;
        }
        let Some((start, _)) = window else { continue };
        if local(t) < start {
            continue;
        }
        let flow = flows.entry(t.currency.clone()).or_default();
        match t.kind {
            Kind::Transfer => flow.transfers += 1,
            Kind::Inflow => flow.income += t.amount,
            Kind::Refund => flow.spending -= t.amount,
            Kind::Outflow => flow.spending += t.amount,
        }
    }

    let health = selected
        .iter()
        .map(|a| {
            let records = &by_account[a.account_id.as_str()];
            let dates: Vec<Date> = records.iter().map(|t| local(t)).collect();
            let outflows: Vec<&&Tx> = records.iter().filter(|t| t.kind == Kind::Outflow).collect();
            AccountHealth {
                account_id: a.account_id.clone(),
                name: a.name.clone(),
                source_modified_at: refreshed_at.map(|t| t.to_string()),
                first_transaction_on: dates.iter().min().map(Date::to_string),
                last_transaction_on: dates.iter().max().map(Date::to_string),
                transaction_count: records.len() as u64,
                outflow_count: outflows.len() as u64,
                uncategorised_outflows: outflows
                    .iter()
                    .filter(|t| t.category.as_deref().is_none_or(str::is_empty))
                    .count() as u64,
            }
        })
        .collect();

    let payday_status = match (window, payday_day) {
        (Some((start, next_day)), Some(day)) => {
            let mut due_on = if (today - start).get_days() <= 3 {
                start
            } else {
                next_day
            };
            let evidence: Vec<&Tx> = transactions
                .iter()
                .filter(|t| {
                    t.kind == Kind::Inflow
                        && t.category
                            .as_deref()
                            .is_some_and(|c| casefold(c.trim()) == "salary")
                        && t.occurred_at.timestamp() <= now.timestamp()
                })
                .collect();
            let mut salary: Vec<&Tx> = Vec::new();
            for expected in [next_day, start] {
                let (from, to) = (
                    expected.checked_sub(EARLY_DAYS.days()).unwrap_or(expected),
                    expected.checked_add(LATE_DAYS.days()).unwrap_or(expected),
                );
                let matches: Vec<&Tx> = evidence
                    .iter()
                    .copied()
                    .filter(|t| (from..=to).contains(&local(t)))
                    .collect();
                if !matches.is_empty() {
                    due_on = expected;
                    salary = matches;
                    break;
                }
            }
            let mut amounts: BTreeMap<Currency, Decimal> = BTreeMap::new();
            for t in &salary {
                *amounts.entry(t.currency.clone()).or_default() += t.amount;
            }
            Some(PaydayStatus {
                due_on: due_on.to_string(),
                next_expected_on: if !salary.is_empty() && due_on == next_day {
                    payday_window(next_day, day).1
                } else {
                    next_day
                }
                .to_string(),
                received_on: salary.iter().map(|t| local(t)).max().map(|d| d.to_string()),
                received_amounts: amounts
                    .into_iter()
                    .map(|(currency, amount)| UnitMoney { amount, currency })
                    .collect(),
                transaction_count: salary.len() as u64,
            })
        }
        _ => None,
    };

    CashOverview {
        through_date: today.to_string(),
        period_start: window.map(|w| w.0.to_string()),
        next_payday: window.map(|w| w.1.to_string()),
        days_until_payday: window.map(|w| i64::from((w.1 - today).get_days())),
        payday_status,
        currencies: flows
            .into_iter()
            .map(|(currency, f)| CurrencyWeek {
                currency,
                income: f.income,
                spending: f.spending,
                net_cash_flow: f.income - f.spending,
                transfer_count: f.transfers,
                categories: Vec::new(),
            })
            .collect(),
        accounts: health,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    #[test]
    fn payday_boundaries() {
        let cases = [
            (date(2026, 9, 9), 25, date(2026, 8, 25), date(2026, 9, 25)),
            (date(2026, 9, 25), 25, date(2026, 9, 25), date(2026, 10, 25)),
            (date(2026, 3, 1), 31, date(2026, 2, 28), date(2026, 3, 31)),
            (date(2024, 2, 29), 31, date(2024, 2, 29), date(2024, 3, 31)),
            (date(2026, 1, 1), 25, date(2025, 12, 25), date(2026, 1, 25)),
        ];
        for (today, day, start, next) in cases {
            assert_eq!(payday_window(today, day), (start, next), "{today} {day}");
        }
    }

    fn tx(id: &str, kind: Kind, amount: &str, day: i8, category: Option<&str>) -> Tx {
        Tx {
            transaction_id: id.into(),
            ledger_id: 0,
            account_id: "a".into(),
            counterparty_account_id: (kind == Kind::Transfer).then(|| "b".into()),
            kind,
            amount: amount.parse().unwrap(),
            currency: Currency::eur(),
            occurred_at: date(2026, 9, day)
                .at(14, 0, 0, 0)
                .in_tz("Europe/Amsterdam")
                .unwrap(),
            purchased_on: None,
            description: Some("Synthetic Owner shop".into()),
            counterparty_name: None,
            group_id: None,
            category: category.map(str::to_owned),
            subscription_id: None,
            source_country: None,
            destination_country: None,
            transfer_evidence: None,
            repays: None,
        }
    }

    fn account(id: &str) -> Account {
        Account {
            account_id: id.into(),
            ledger_id: 1,
            name: format!("Synthetic {id}"),
            balance: UnitMoney {
                amount: Decimal::from(100),
                currency: Currency::eur(),
            },
            include_net_worth: true,
            active: true,
            iban: None,
        }
    }

    #[test]
    fn health_and_cycle_keep_inflows_and_separate_transfers() {
        let rows = vec![
            tx("salary", Kind::Inflow, "1000", 9, Some("Salary")),
            tx("spend", Kind::Outflow, "30", 9, Some("Groceries")),
            tx("unknown", Kind::Outflow, "10", 9, None),
            tx("move", Kind::Transfer, "50", 9, None),
            tx("old", Kind::Outflow, "99", 1, None),
        ];
        let now = date(2026, 9, 9)
            .at(14, 0, 0, 0)
            .in_tz("Europe/Amsterdam")
            .unwrap();
        let report = cash_overview(&[account("a"), account("b")], &rows, &now, Some(8), None);
        let eur = &report.currencies[0];
        assert_eq!(eur.income, Decimal::from(1000));
        assert_eq!(eur.spending, Decimal::from(40));
        assert_eq!(eur.transfer_count, 1);
        assert_eq!(report.accounts[0].transaction_count, 5);
        assert_eq!(report.accounts[0].uncategorised_outflows, 2);
        assert_eq!(report.accounts[1].transaction_count, 1);
        let status = report.payday_status.unwrap();
        assert_eq!(status.due_on, "2026-09-08");
        assert_eq!(status.received_on.as_deref(), Some("2026-09-09"));
        assert_eq!(status.next_expected_on, "2026-10-08");
    }
}
