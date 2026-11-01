//! The money plan as data fields: what each account holds and keeps, the
//! transfers to make, and what the savings account must hold. Shared by the
//! closure, Overview and Plan screens.

use std::collections::HashMap;

use finance_types::report::closure::{AccountRoleKind, MoneyPlan, PlanAccount, PlanMove};
use leptos::prelude::*;
use rust_decimal::Decimal;

use super::{Stat, TABLE, TABLE_CONTAINER, TCELL, TH, TROW, Tone, cn, money};

/// `(label, value, warns)` of an account's standing column.
pub fn standing(account: &PlanAccount, salary: bool) -> (&'static str, Decimal, bool) {
    if account.role == AccountRoleKind::Savings {
        return ("", Decimal::ZERO, false);
    }
    if salary && account.short > Decimal::ZERO {
        ("Short", account.short, true)
    } else if account.needs > Decimal::ZERO {
        ("Needs", account.needs, true)
    } else if salary {
        ("To send on", account.spare, false)
    } else {
        ("Spare", account.spare, false)
    }
}

/// The salary account: the source of the moves, else the spending account
/// that is short.
pub fn salary_account(plan: &MoneyPlan) -> Option<String> {
    plan.moves
        .first()
        .map(|m| m.from_account_id.clone())
        .or_else(|| {
            plan.accounts
                .iter()
                .find(|a| a.role == AccountRoleKind::Spending && a.short > Decimal::ZERO)
                .map(|a| a.account_id.clone())
        })
}

/// Transfer rows: `(label, amount)`, one per move (`To <name>` with
/// ` via <hub>` when routed), then one total per hub (`Transfer to <hub>`)
/// when several moves share it.
pub fn transfer_rows(plan: &MoneyPlan, names: &HashMap<String, String>) -> Vec<(String, Decimal)> {
    let name = |id: &str| names.get(id).cloned().unwrap_or_else(|| id.to_owned());
    let mut rows: Vec<(String, Decimal)> = plan
        .moves
        .iter()
        .map(|m: &PlanMove| {
            let label = match &m.via_account_id {
                Some(hub) => format!("To {} via {}", name(&m.to_account_id), name(hub)),
                None => format!("To {}", name(&m.to_account_id)),
            };
            (label, m.amount)
        })
        .collect();
    let mut hubs: Vec<&str> = Vec::new();
    for m in &plan.moves {
        if let Some(hub) = m.via_account_id.as_deref()
            && !hubs.contains(&hub)
        {
            hubs.push(hub);
        }
    }
    for hub in hubs {
        let through: Vec<&PlanMove> = plan
            .moves
            .iter()
            .filter(|m| m.via_account_id.as_deref() == Some(hub))
            .collect();
        if through.len() > 1 {
            rows.push((
                format!("Transfer to {}", name(hub)),
                through.iter().map(|m| m.amount).sum(),
            ));
        }
    }
    rows
}

/// The accounts table, the transfer rows and the savings fields.
#[component]
pub fn MoneyPlanFields(
    plan: MoneyPlan,
    names: HashMap<String, String>,
    /// Leave out the accounts table (Overview shows balances already).
    #[prop(optional)]
    compact: bool,
) -> impl IntoView {
    let unit = plan.currency.clone();
    let salary = salary_account(&plan);
    let cell = cn([TCELL, "text-right tabular-nums whitespace-nowrap"]);
    let rows = plan
        .accounts
        .iter()
        .map(|account| {
            let is_salary = salary.as_deref() == Some(account.account_id.as_str());
            let (label, value, warn) = standing(account, is_salary);
            let keep = account
                .keep_parts
                .iter()
                .map(|(part, _)| part.to_lowercase())
                .collect::<Vec<_>>()
                .join(", ");
            view! {
                <tr class=TROW>
                    <th scope="row" class=cn([TCELL, "text-left font-medium"])>
                        {account.name.clone()}
                        {(!keep.is_empty()).then(|| view! { <small class="block font-normal text-fg-muted">{keep}</small> })}
                    </th>
                    <td class=cell.clone()>{money(account.balance, &unit)}</td>
                    <td class=cell.clone()>{(account.keep > Decimal::ZERO).then(|| money(account.keep, &unit))}</td>
                    <td class=cn([cell.as_str(), if warn { "text-negative-fg" } else { "" }])>
                        {(!label.is_empty()).then(|| view! {
                            <small class="mr-1 font-normal text-fg-muted">{label}</small>
                            {money(value, &unit)}
                        })}
                    </td>
                </tr>
            }
        })
        .collect_view();
    let table = (!compact).then(|| {
        view! {
            <div class=TABLE_CONTAINER tabindex="0">
                <table class=TABLE data-density="sm">
                    <thead>
                        <tr>
                            <th class=cn([TH, "text-left"])>"Account"</th>
                            <th class=cn([TH, "text-right"])>"Balance"</th>
                            <th class=cn([TH, "text-right"])>"Keep"</th>
                            <th class=cn([TH, "text-right"])></th>
                        </tr>
                    </thead>
                    <tbody>{rows}</tbody>
                </table>
            </div>
        }
    });
    let transfers = transfer_rows(&plan, &names);
    let transfer_rows_view = if transfers.is_empty() {
        view! {
            <div class="flex justify-between gap-4 text-sm"><span class="text-fg-muted">"Transfer"</span><span class="tabular-nums">{money(Decimal::ZERO, &unit)}</span></div>
        }
        .into_any()
    } else {
        transfers
            .into_iter()
            .map(|(label, amount)| {
                let total = label.starts_with("Transfer to ");
                view! {
                    <div class=cn(["flex justify-between gap-4 text-sm", if total { "border-t border-line pt-1 font-semibold" } else { "" }])>
                        <span class=if total { "" } else { "text-fg-muted" }>{label}</span>
                        <span class="tabular-nums">{money(amount, &unit)}</span>
                    </div>
                }
            })
            .collect_view()
            .into_any()
    };
    let savings = plan.savings.as_ref().map(|s| {
        let negative = s.wiggle_room < Decimal::ZERO;
        let (name, balance, incoming, must_keep, room) = (
            s.name.clone(),
            money(s.balance, &unit),
            money(s.incoming, &unit),
            money(s.must_keep, &unit),
            money(s.wiggle_room, &unit),
        );
        let wishes: Vec<(String, String)> = s
            .wishes
            .iter()
            .map(|(name, saved)| (name.clone(), money(*saved, &unit)))
            .collect();
        let wish_rows = wishes
            .into_iter()
            .map(|(name, saved)| {
                view! {
                    <div class="flex justify-between gap-4 text-xs text-fg-muted">
                        <span>{name}</span>
                        <span class="tabular-nums">{saved}</span>
                    </div>
                }
            })
            .collect_view();
        let has_wishes = !s.wishes.is_empty();
        view! {
            <div class="grid gap-2">
                <div class="grid grid-cols-2 gap-3 sm:grid-cols-4">
                    <Stat label=name>{balance}</Stat>
                    <Stat label="Incoming">{incoming}</Stat>
                    <Stat label="Saved for wishes">{must_keep}</Stat>
                    <Stat label="Wiggle room" tone=if negative { Tone::Negative } else { Tone::Neutral }>{room}</Stat>
                </div>
                {has_wishes.then(|| view! { <div class="grid gap-0.5">{wish_rows}</div> })}
            </div>
        }
    });
    view! {
        <div class="grid gap-4">
            {table}
            <div class="grid gap-1">{transfer_rows_view}</div>
            {savings}
        </div>
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

    fn mv(to: &str, via: Option<&str>, amount: &str) -> PlanMove {
        PlanMove {
            from_account_id: "a".into(),
            to_account_id: to.into(),
            via_account_id: via.map(str::to_owned),
            amount: d(amount),
            reason: String::new(),
        }
    }

    fn account(id: &str, role: AccountRoleKind, balance: &str, keep: &str) -> PlanAccount {
        let (balance, keep) = (d(balance), d(keep));
        PlanAccount {
            account_id: id.into(),
            name: id.into(),
            role,
            balance,
            keep,
            keep_parts: Vec::new(),
            needs: (keep - balance).max(Decimal::ZERO),
            spare: (balance - keep).max(Decimal::ZERO),
            short: (keep - balance).max(Decimal::ZERO),
        }
    }

    fn plan(moves: Vec<PlanMove>) -> MoneyPlan {
        MoneyPlan {
            as_of: "2026-11-01".into(),
            month: "2026-11-01".into(),
            currency: Currency::eur(),
            accounts: vec![
                account("a", AccountRoleKind::Spending, "2000", "1020"),
                account("g", AccountRoleKind::Spending, "120", "300"),
                account("s", AccountRoleKind::Savings, "1200", "0"),
            ],
            moves,
            savings: Some(SavingsStanding {
                account_id: "s".into(),
                name: "Savings".into(),
                balance: d("1200"),
                incoming: d("800"),
                must_keep: d("624"),
                wishes: Vec::new(),
                wiggle_room: d("1376"),
            }),
        }
    }

    #[test]
    fn transfer_rows_list_each_move_and_total_per_hub() {
        let names: HashMap<String, String> = [
            ("a", "ABN"),
            ("r", "Revolut"),
            ("g", "Groceries"),
            ("s", "Savings"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
        let routed = plan(vec![mv("g", Some("r"), "180"), mv("s", Some("r"), "800")]);
        assert_eq!(
            transfer_rows(&routed, &names),
            vec![
                ("To Groceries via Revolut".to_owned(), d("180")),
                ("To Savings via Revolut".to_owned(), d("800")),
                ("Transfer to Revolut".to_owned(), d("980")),
            ]
        );
        let direct = plan(vec![mv("s", None, "50")]);
        assert_eq!(
            transfer_rows(&direct, &names),
            vec![("To Savings".to_owned(), d("50"))]
        );
    }

    #[test]
    fn standing_names_short_needs_spare_and_to_send_on() {
        let p = plan(vec![mv("g", Some("r"), "180")]);
        assert_eq!(salary_account(&p).as_deref(), Some("a"));
        assert_eq!(
            standing(&p.accounts[0], true),
            ("To send on", d("980"), false)
        );
        assert_eq!(standing(&p.accounts[1], false), ("Needs", d("180"), true));
        assert_eq!(standing(&p.accounts[2], false), ("", Decimal::ZERO, false));
        let short = account("a", AccountRoleKind::Spending, "1000", "1020");
        assert_eq!(standing(&short, true), ("Short", d("20"), true));
        let swedbank = account("w", AccountRoleKind::Spending, "7", "0");
        assert_eq!(standing(&swedbank, false), ("Spare", d("7"), false));
    }
}
