//! The money plan under the wizard: from the balances as they stand, what
//! each account keeps, the transfer to make and what savings must hold.

use std::collections::{BTreeMap, HashMap};

use finance_types::report::closure::{AccountRoleKind, MoneyPlan, PlanAccount, PlanMove};
use leptos::prelude::*;
use rust_decimal::Decimal;

use super::ui::{LIST, StepCard};
use crate::ds::{cn, money};

const MONTHS_EN: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// `1 Nov` from `YYYY-MM-DD`.
fn short_day(day: &str) -> String {
    match (
        day.get(8..10).and_then(|d| d.parse::<u32>().ok()),
        day.get(5..7).and_then(|m| m.parse::<usize>().ok()),
    ) {
        (Some(d), Some(m @ 1..=12)) => format!("{d} {}", MONTHS_EN[m - 1]),
        _ => day.to_owned(),
    }
}

/// `keep €1,020.00 (bills €1,018.90, planned €1.10)` or `nothing to keep`.
fn keep_text(account: &PlanAccount, plan: &MoneyPlan) -> String {
    if account.keep <= Decimal::ZERO {
        return "nothing to keep".to_owned();
    }
    let parts: Vec<String> = account
        .keep_parts
        .iter()
        .map(|(label, value)| format!("{} {}", label.to_lowercase(), money(*value, &plan.currency)))
        .collect();
    format!(
        "keep {} ({})",
        money(account.keep, &plan.currency),
        parts.join(", ")
    )
}

/// The account's verdict: short, needs a top-up, or fine with spare.
fn standing(account: &PlanAccount, plan: &MoneyPlan, salary: bool) -> (String, bool) {
    let unit = &plan.currency;
    if account.role == AccountRoleKind::Savings {
        return (String::new(), false);
    }
    if account.short > Decimal::ZERO && salary {
        (format!("short {}", money(account.short, unit)), true)
    } else if account.needs > Decimal::ZERO {
        (format!("needs {}", money(account.needs, unit)), true)
    } else if salary && account.spare > Decimal::ZERO {
        (format!("{} to send on", money(account.spare, unit)), false)
    } else if account.spare > Decimal::ZERO {
        (format!("fine, {} spare", money(account.spare, unit)), false)
    } else {
        ("fine".to_owned(), false)
    }
}

/// Moves grouped by the hub they go through (`None` = direct), in the
/// plan's order.
fn grouped(moves: &[PlanMove]) -> Vec<(Option<String>, Vec<PlanMove>)> {
    let mut order: Vec<Option<String>> = Vec::new();
    let mut by_via: BTreeMap<Option<String>, Vec<PlanMove>> = BTreeMap::new();
    for m in moves {
        if !by_via.contains_key(&m.via_account_id) {
            order.push(m.via_account_id.clone());
        }
        by_via
            .entry(m.via_account_id.clone())
            .or_default()
            .push(m.clone());
    }
    order
        .into_iter()
        .map(|via| {
            let list = by_via.remove(&via).unwrap_or_default();
            (via, list)
        })
        .collect()
}

/// The transfer lines: `Transfer €980.00 from ABN AMRO to Revolut: €180.00
/// → Revolut Groceries, €800.00 → Revolut Savings`.
pub fn move_lines(plan: &MoneyPlan, names: &HashMap<String, String>) -> Vec<String> {
    let name = |id: &str| names.get(id).cloned().unwrap_or_else(|| id.to_owned());
    grouped(&plan.moves)
        .into_iter()
        .map(|(via, list)| {
            let total: Decimal = list.iter().map(|m| m.amount).sum();
            let from = name(&list[0].from_account_id);
            let parts: Vec<String> = list
                .iter()
                .map(|m| {
                    format!(
                        "{} → {}",
                        money(m.amount, &plan.currency),
                        name(&m.to_account_id)
                    )
                })
                .collect();
            match via {
                Some(hub) => format!(
                    "Transfer {} from {from} to {}: {}",
                    money(total, &plan.currency),
                    name(&hub),
                    parts.join(", ")
                ),
                None if list.len() == 1 => format!(
                    "Transfer {} from {from} to {}",
                    money(total, &plan.currency),
                    name(&list[0].to_account_id)
                ),
                None => format!(
                    "Transfer {} from {from}: {}",
                    money(total, &plan.currency),
                    parts.join(", ")
                ),
            }
        })
        .collect()
}

/// `Revolut Savings: €1,200.84 (+€800.00 incoming). Keep €624.00 for wishes
/// already saved (…). Wiggle room €1,376.84.`
pub fn savings_lines(plan: &MoneyPlan) -> Option<(String, String, String, bool)> {
    let s = plan.savings.as_ref()?;
    let unit = &plan.currency;
    let balance = if s.incoming > Decimal::ZERO {
        format!(
            "{}: {} (+{} incoming)",
            s.name,
            money(s.balance, unit),
            money(s.incoming, unit)
        )
    } else {
        format!("{}: {}", s.name, money(s.balance, unit))
    };
    let keep = if s.wishes.is_empty() {
        "Nothing saved toward a wish yet.".to_owned()
    } else {
        let names: Vec<String> = s
            .wishes
            .iter()
            .map(|(n, v)| format!("{n} {}", money(*v, unit)))
            .collect();
        format!(
            "Keep {} for wishes already saved ({}).",
            money(s.must_keep, unit),
            names.join(", ")
        )
    };
    let negative = s.wiggle_room < Decimal::ZERO;
    let room = if negative {
        format!(
            "{} short of what the wishes need.",
            money(-s.wiggle_room, unit)
        )
    } else {
        format!("Wiggle room {}.", money(s.wiggle_room, unit))
    };
    Some((balance, keep, room, negative))
}

/// Where the money should go now.
#[component]
pub fn MoneyPlanPanel(plan: MoneyPlan, names: HashMap<String, String>) -> impl IntoView {
    let unit = plan.currency.clone();
    let salary_id = plan
        .moves
        .first()
        .map(|m| m.from_account_id.clone())
        .or_else(|| {
            plan.accounts
                .iter()
                .find(|a| a.role == AccountRoleKind::Spending && a.short > Decimal::ZERO)
                .map(|a| a.account_id.clone())
        });
    let rows = plan
        .accounts
        .iter()
        .map(|account| {
            let salary = salary_id.as_deref() == Some(account.account_id.as_str());
            let (verdict, warn) = standing(account, &plan, salary);
            let keep = keep_text(account, &plan);
            view! {
                <li class="flex flex-wrap items-baseline justify-between gap-x-4 gap-y-1 py-2">
                    <span class="min-w-0">
                        <strong>{account.name.clone()}</strong>
                        <span class="text-fg-muted">{format!(" · {}", keep)}</span>
                    </span>
                    <span class="flex items-baseline gap-3 tabular-nums">
                        <span>{money(account.balance, &unit)}</span>
                        {(!verdict.is_empty()).then(|| view! {
                            <span class=cn(["text-xs", if warn { "text-negative-fg" } else { "text-fg-muted" }])>{verdict}</span>
                        })}
                    </span>
                </li>
            }
        })
        .collect_view();
    let transfers = move_lines(&plan, &names);
    let transfer_view = if transfers.is_empty() {
        view! { <p class="text-sm text-fg-muted">"Nothing to transfer."</p> }.into_any()
    } else {
        view! {
            <ul class="grid gap-1 text-sm">
                {transfers.into_iter().map(|t| view! { <li><strong>{t}</strong></li> }).collect_view()}
            </ul>
        }
        .into_any()
    };
    let savings = savings_lines(&plan).map(|(balance, keep, room, negative)| {
        view! {
            <p class="text-sm">
                <strong>{balance}</strong>
                " "{keep}" "
                <span class=if negative { "text-negative-fg" } else { "" }>{room}</span>
            </p>
        }
    });
    view! {
        <StepCard
            title="Where the money goes"
            description=Some(format!(
                "From the balances as of {}, for {}: what each account keeps, what to move on, and what savings must hold.",
                short_day(&plan.as_of),
                super::ui::month_label(&plan.month, false)
            ))
        >
            <ul class=LIST>{rows}</ul>
            {transfer_view}
            {savings}
        </StepCard>
    }
}

#[cfg(test)]
mod tests {
    use finance_types::report::Currency;
    use finance_types::report::closure::SavingsStanding;

    use super::*;

    fn d(text: &str) -> Decimal {
        text.parse().unwrap()
    }

    fn plan() -> MoneyPlan {
        MoneyPlan {
            as_of: "2026-11-01".into(),
            month: "2026-11-01".into(),
            currency: Currency::eur(),
            accounts: Vec::new(),
            moves: vec![
                PlanMove {
                    from_account_id: "a".into(),
                    to_account_id: "g".into(),
                    via_account_id: Some("r".into()),
                    amount: d("180"),
                    reason: "Top up".into(),
                },
                PlanMove {
                    from_account_id: "a".into(),
                    to_account_id: "s".into(),
                    via_account_id: Some("r".into()),
                    amount: d("800"),
                    reason: "Savings".into(),
                },
            ],
            savings: Some(SavingsStanding {
                account_id: "s".into(),
                name: "Revolut Savings".into(),
                balance: d("1200.84"),
                incoming: d("800"),
                must_keep: d("624"),
                wishes: vec![("Dinner".into(), d("300"))],
                wiggle_room: d("1376.84"),
            }),
        }
    }

    #[test]
    fn moves_group_by_hub() {
        let names: HashMap<String, String> = [
            ("a", "ABN AMRO"),
            ("r", "Revolut"),
            ("g", "Revolut Groceries"),
            ("s", "Revolut Savings"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
        assert_eq!(
            move_lines(&plan(), &names),
            vec![
                "Transfer €980.00 from ABN AMRO to Revolut: €180.00 → Revolut Groceries, €800.00 → Revolut Savings"
            ]
        );
    }

    #[test]
    fn savings_lines_name_the_wishes() {
        let (balance, keep, room, negative) = savings_lines(&plan()).unwrap();
        assert_eq!(balance, "Revolut Savings: €1,200.84 (+€800.00 incoming)");
        assert_eq!(
            keep,
            "Keep €624.00 for wishes already saved (Dinner €300.00)."
        );
        assert_eq!(room, "Wiggle room €1,376.84.");
        assert!(!negative);
    }
}
