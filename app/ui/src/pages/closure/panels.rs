//! The Accounts and Pots panels under the wizard (personal-site
//! `components/month-closure/accounts-panel.tsx` and `pots-panel.tsx`).

use std::collections::HashSet;

use finance_types::report::Currency;
use finance_types::report::closure::{AccountRoleKind, AccountRoleView, ClosureView, PotView};
use leptos::prelude::*;

use super::Wizard;
use super::ui::{Choice, FieldBox, GROW, INLINE_FORM, LIST, SMALL, StepCard, field_id, money};
use crate::ds::EmptyState;

const ROLES: [(&str, &str, &str); 3] = [
    (
        "spending",
        "Spending",
        "Reviewed each month; its leftover gets split.",
    ),
    (
        "savings",
        "Savings",
        "Receives money from the split; not reviewed as spending.",
    ),
    (
        "ignored",
        "Don't track",
        "Left out of month closing entirely.",
    ),
];

fn role_name(role: AccountRoleKind) -> &'static str {
    match role {
        AccountRoleKind::Spending => "spending",
        AccountRoleKind::Savings => "savings",
        AccountRoleKind::Ignored => "ignored",
        AccountRoleKind::Unclassified => "",
    }
}

/// Every known account with its closing role, including ones set to
/// "Don't track".
#[component]
pub fn AccountsPanel(
    accounts: Vec<AccountRoleView>,
    goals: Vec<(String, String)>,
) -> impl IntoView {
    let wizard = expect_context::<Wizard>();
    let disabled = Signal::derive(move || wizard.busy.get());
    // Accounts other accounts route through; they cannot be routed
    // themselves (one hop at most).
    let hubs: HashSet<String> = accounts
        .iter()
        .filter_map(|a| a.via_account_id.clone())
        .collect();
    let help = ROLES
        .iter()
        .map(|(_, label, help)| {
            view! { <li><strong class="text-fg">{*label}</strong>{format!(": {help}")}</li> }
        })
        .collect_view();
    let all = StoredValue::new(accounts.clone());
    let goals = StoredValue::new(goals);
    let empty = accounts.is_empty();
    let rows = accounts
        .into_iter()
        .map(|account| {
            let id = StoredValue::new(account.account_id.clone());
            let goal = StoredValue::new(account.goal_id.clone());
            let role = account.role;
            let role_id = field_id();
            let mut role_options = Vec::new();
            if role == AccountRoleKind::Unclassified {
                role_options.push((String::new(), "Not set".to_owned(), true));
            }
            role_options.extend(ROLES.iter().map(|(v, l, _)| ((*v).to_owned(), (*l).to_owned(), false)));
            let goal_field = (role == AccountRoleKind::Savings && goals.with_value(|g| !g.is_empty())).then(|| {
                let goal_id = field_id();
                let mut opts = vec![(String::new(), "No goal".to_owned(), false)];
                goals.with_value(|g| opts.extend(g.iter().map(|(g, n)| (g.clone(), n.clone(), false))));
                view! {
                    <FieldBox label="Savings goal".into_any() id=goal_id.clone() class=GROW>
                        <Choice
                            id=goal_id
                            value=Signal::stored(account.goal_id.clone().unwrap_or_default())
                            options=Signal::stored(opts)
                            disabled=disabled
                            on_change=Callback::new(move |v: String| {
                                wizard.set_role(id.get_value(), "savings", (!v.is_empty()).then_some(v), None, None);
                            })
                        />
                    </FieldBox>
                }
            });
            let via_field = (role != AccountRoleKind::Unclassified
                && role != AccountRoleKind::Ignored
                && !hubs.contains(&account.account_id))
            .then(|| {
                let via_id = field_id();
                let mut opts = vec![(String::new(), "Directly".to_owned(), false)];
                all.with_value(|all| {
                    opts.extend(
                        all.iter()
                            .filter(|o| {
                                o.account_id != account.account_id
                                    && o.via_account_id.is_none()
                                    && o.role != AccountRoleKind::Ignored
                            })
                            .map(|o| (o.account_id.clone(), o.name.clone(), false)),
                    );
                });
                view! {
                    <FieldBox label="Reached via".into_any() id=via_id.clone() class=GROW>
                        <Choice
                            id=via_id
                            title="Transfers to and from this account go through that one, e.g. a Revolut pocket through main Revolut."
                            value=Signal::stored(account.via_account_id.clone().unwrap_or_default())
                            options=Signal::stored(opts)
                            disabled=disabled
                            on_change=Callback::new(move |v: String| {
                                let goal = if role == AccountRoleKind::Savings { goal.get_value() } else { None };
                                wizard.set_role(id.get_value(), role_name(role), goal, Some((!v.is_empty()).then_some(v)), None);
                            })
                        />
                    </FieldBox>
                }
            });
            view! {
                <li class="py-3">
                    <div class=INLINE_FORM role="group" aria-label=format!("{} type", account.name)>
                        <FieldBox label=account.name.clone().into_any() id=role_id.clone() class=GROW>
                            <Choice
                                id=role_id
                                value=Signal::stored(role_name(role).to_owned())
                                options=Signal::stored(role_options)
                                disabled=disabled
                                on_change=Callback::new(move |next: String| {
                                    // Keep the account's savings goal when it stays (or
                                    // becomes) a savings account.
                                    let goal = if next == "savings" { goal.get_value() } else { None };
                                    wizard.set_role(id.get_value(), &next, goal, None, None);
                                })
                            />
                        </FieldBox>
                        {goal_field}
                        {via_field}
                    </div>
                </li>
            }
        })
        .collect_view();
    view! {
        <StepCard title="Accounts" description=Some("How month closing treats each account.".to_owned())>
            <ul class="list-disc pl-5 text-sm text-fg-muted">{help}</ul>
            {if empty {
                view! { <EmptyState class="p-4" title="No accounts yet." /> }.into_any()
            } else {
                view! { <ul class=LIST>{rows}</ul> }.into_any()
            }}
        </StepCard>
    }
}

fn unit_for(code: &str, view: &ClosureView) -> Currency {
    view.review
        .accounts
        .iter()
        .find(|a| a.currency.code == code)
        .map(|a| a.currency.clone())
        .unwrap_or_else(|| Currency {
            code: code.to_owned(),
            minor_unit: 2,
        })
}

fn balances(pot: &PotView, view: &ClosureView) -> String {
    if pot.balances.is_empty() {
        return "Empty".into();
    }
    pot.balances
        .iter()
        .map(|(code, amount)| money(*amount, &unit_for(code, view)))
        .collect::<Vec<_>>()
        .join(" · ")
}

#[component]
fn PotRow(
    pot: PotView,
    view: StoredValue<ClosureView>,
    locations: StoredValue<Vec<(String, String)>>,
) -> impl IntoView {
    let where_ = match &pot.account_id {
        Some(id) => locations.with_value(|l| {
            l.iter()
                .find(|(lid, _)| lid == id)
                .map_or_else(|| id.clone(), |(_, label)| label.clone())
        }),
        None => "No account set".into(),
    };
    let total = view.with_value(|v| balances(&pot, v));
    view! {
        <li class="grid gap-2 py-3 text-sm">
            <div class="flex flex-wrap justify-between gap-x-4 gap-y-1">
                <span><strong>{pot.name}</strong><small class=SMALL>{where_}</small></span>
                <strong class="tabular-nums">{total}</strong>
            </div>
        </li>
    }
}

/// Savings pots from before wishes took their place, read only.
#[component]
pub fn PotsPanel(view: ClosureView, locations: Vec<(String, String)>) -> impl IntoView {
    let active: Vec<PotView> = view.pots.iter().filter(|p| !p.archived).cloned().collect();
    let view = StoredValue::new(view);
    let locations = StoredValue::new(locations);
    view! {
        <StepCard
            title="Pots"
            description=Some("Money set aside before wishes took over. Pots no longer change.".to_owned())
        >
            {if active.is_empty() {
                view! { <EmptyState class="p-4" title="No pots." /> }.into_any()
            } else {
                view! {
                    <ul class=LIST>
                        {active.into_iter().map(|pot| view! { <PotRow pot=pot view=view locations=locations /> }).collect_view()}
                    </ul>
                }.into_any()
            }}
        </StepCard>
    }
}
