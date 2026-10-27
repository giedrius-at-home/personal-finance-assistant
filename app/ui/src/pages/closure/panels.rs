//! The Accounts and Pots panels under the wizard (personal-site
//! `components/month-closure/accounts-panel.tsx` and `pots-panel.tsx`).

use std::collections::HashSet;

use finance_types::report::Currency;
use finance_types::report::closure::{
    AccountRoleKind, AccountRoleView, ClosureView, PotDraft, PotView, PotsView,
};
use leptos::prelude::*;
use leptos::task::spawn_local;
use rust_decimal::Decimal;

use super::Wizard;
use super::ui::{
    ACTIONS, Choice, FOOTNOTE, FieldBox, GROW, INLINE_FORM, LIST, SMALL, StepCard, TextBox, WARN,
    field_id, load_pots, money, month_label,
};
use crate::ds::{Button, EmptyState, Size, Variant};

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

fn is_empty(pot: &PotView) -> bool {
    pot.balances.values().all(Decimal::is_zero)
}

#[component]
fn PotRow(
    pot: PotView,
    data: Option<PotsView>,
    view: StoredValue<ClosureView>,
    locations: StoredValue<Vec<(String, String)>>,
) -> impl IntoView {
    let wizard = expect_context::<Wizard>();
    let disabled = Signal::derive(move || wizard.busy.get());
    let editing = RwSignal::new(false);
    let name = RwSignal::new(pot.name.clone());
    let account = RwSignal::new(pot.account_id.clone().unwrap_or_default());
    let general = pot.pot_id == "general";
    let where_ = match &pot.account_id {
        Some(id) => locations.with_value(|l| {
            l.iter()
                .find(|(lid, _)| lid == id)
                .map_or_else(|| id.clone(), |(_, label)| label.clone())
        }),
        None => "No account set".into(),
    };
    let recent: Vec<_> = data
        .map(|d| {
            d.entries
                .into_iter()
                .filter(|e| e.pot_id == pot.pot_id)
                .take(3)
                .collect()
        })
        .unwrap_or_default();
    let pot = StoredValue::new(pot);
    let save = move |change: Box<dyn FnOnce(&mut PotDraft)>, done: Option<RwSignal<bool>>| {
        let mut draft = pot.with_value(|p| PotDraft {
            pot_id: Some(p.pot_id.clone()),
            name: p.name.clone(),
            account_id: p.account_id.clone(),
            archived: p.archived,
        });
        change(&mut draft);
        wizard.save_pot(draft, done);
    };
    let (title, total) = pot.with_value(|p| (p.name.clone(), view.with_value(|v| balances(p, v))));
    let recent_view = (!recent.is_empty()).then(|| {
        let items = recent
            .into_iter()
            .map(|entry| {
                let unit = view.with_value(|v| unit_for(&entry.currency.code, v));
                let sign = if entry.amount > Decimal::ZERO { "+" } else { "" };
                let label = format!(
                    "{}{}",
                    month_label(entry.month.get(..10).unwrap_or(&entry.month), true),
                    entry.note.map(|n| format!(" · {n}")).unwrap_or_default()
                );
                view! {
                    <li class="flex justify-between gap-3">
                        <span>{label}</span>
                        <span class="tabular-nums">{format!("{sign}{}", money(entry.amount, &unit))}</span>
                    </li>
                }
            })
            .collect_view();
        view! { <ul class="grid gap-0.5 text-xs text-fg-muted" aria-label=format!("Recent {title} entries")>{items}</ul> }
    });
    let edit_form = move || {
        let name_id = field_id();
        let account_id = field_id();
        let mut opts = vec![(String::new(), "Not set".to_owned(), false)];
        locations.with_value(|l| {
            opts.extend(
                l.iter()
                    .map(|(id, label)| (id.clone(), label.clone(), false)),
            )
        });
        let title = pot.with_value(|p| p.name.clone());
        view! {
            <div class=INLINE_FORM role="group" aria-label=format!("Edit {title}")>
                <FieldBox label="Name".into_any() id=name_id.clone() class=GROW>
                    <TextBox id=name_id value=name on_input=Callback::new(move |v| name.set(v)) maxlength=128 />
                </FieldBox>
                <FieldBox label="Money sits on".into_any() id=account_id.clone() class=GROW>
                    <Choice id=account_id value=account options=Signal::stored(opts) on_change=Callback::new(move |v| account.set(v)) />
                </FieldBox>
                <Button
                    size=Size::Sm
                    disabled=Signal::derive(move || disabled.get() || name.with(|n| n.trim().is_empty()))
                    on_click=Callback::new(move |_| {
                        let new_name = name.get_untracked().trim().to_owned();
                        let place = account.get_untracked();
                        save(
                            Box::new(move |d| {
                                d.name = new_name;
                                d.account_id = (!place.is_empty()).then_some(place);
                            }),
                            Some(editing),
                        );
                    })
                >
                    "Save"
                </Button>
                <Button size=Size::Sm variant=Variant::Ghost on_click=Callback::new(move |_| {
                    editing.set(false);
                    pot.with_value(|p| {
                        name.set(p.name.clone());
                        account.set(p.account_id.clone().unwrap_or_default());
                    });
                })>"Cancel"</Button>
            </div>
        }
    };
    let (archived, empty) = pot.with_value(|p| (p.archived, is_empty(p)));
    let buttons = move || {
        view! {
            <div class=ACTIONS>
                <Button size=Size::Sm variant=Variant::Secondary disabled=disabled on_click=Callback::new(move |_| editing.set(true))>"Edit"</Button>
                {if archived {
                    Some(view! {
                        <Button size=Size::Sm variant=Variant::Ghost disabled=disabled on_click=Callback::new(move |_| {
                            save(Box::new(|d| d.archived = false), None);
                        })>"Restore"</Button>
                    }.into_any())
                } else if !general {
                    Some(view! {
                        <Button
                            size=Size::Sm
                            variant=Variant::Ghost
                            disabled=Signal::derive(move || disabled.get() || !empty)
                            title=if empty { String::new() } else { "Only an empty pot can be archived".to_owned() }
                            on_click=Callback::new(move |_| save(Box::new(|d| d.archived = true), None))
                        >
                            "Archive"
                        </Button>
                    }.into_any())
                } else {
                    None
                }}
            </div>
        }
    };
    view! {
        <li class="grid gap-2 py-3 text-sm">
            <div class="flex flex-wrap justify-between gap-x-4 gap-y-1">
                <span><strong>{title.clone()}</strong><small class=SMALL>{where_}</small></span>
                <strong class="tabular-nums">{total}</strong>
            </div>
            {recent_view}
            {move || if editing.get() { edit_form().into_any() } else { buttons().into_any() }}
        </li>
    }
}

/// Savings pots: balances, where the money sits, recent entries, and edits.
#[component]
pub fn PotsPanel(view: ClosureView, locations: Vec<(String, String)>) -> impl IntoView {
    let wizard = expect_context::<Wizard>();
    let disabled = Signal::derive(move || wizard.busy.get());
    let data = RwSignal::new(None::<PotsView>);
    let failed = RwSignal::new(false);
    let new_name = RwSignal::new(String::new());
    let new_account = RwSignal::new(String::new());
    // Pot balances arrive with every closure view; reload the entries
    // whenever the view changes (this panel is rebuilt with it).
    spawn_local(async move {
        match load_pots().await {
            Ok(next) => {
                let _ = data.try_set(Some(next));
                let _ = failed.try_set(false);
            }
            Err(_) => {
                let _ = failed.try_set(true);
            }
        }
    });
    let view = StoredValue::new(view);
    let locations = StoredValue::new(locations);
    let new_name_id = field_id();
    let new_account_id = field_id();
    let mut location_options = vec![(String::new(), "Not set".to_owned(), false)];
    locations.with_value(|l| {
        location_options.extend(
            l.iter()
                .map(|(id, label)| (id.clone(), label.clone(), false)),
        );
    });
    let pots_now = move || -> (Option<PotsView>, Vec<PotView>) {
        let loaded = data.get();
        let pots = loaded
            .as_ref()
            .map(|d| d.pots.clone())
            .unwrap_or_else(|| view.with_value(|v| v.pots.clone()));
        (loaded, pots)
    };
    let row = move |loaded: Option<PotsView>| move |pot: PotView| view! { <PotRow pot=pot data=loaded.clone() view=view locations=locations /> };
    let body = move || {
        let (loaded, pots) = pots_now();
        let active: Vec<PotView> = pots.into_iter().filter(|p| !p.archived).collect();
        // Pots claim more than the account holds: the owner moved money
        // without recording it, or a pot is on the wrong account.
        let over = view.with_value(|v| {
            v.review
                .accounts
                .iter()
                .filter_map(|account| {
                    let claimed: Decimal = active
                        .iter()
                        .filter(|p| p.account_id.as_deref() == Some(account.account_id.as_str()))
                        .map(|p| {
                            p.balances
                                .get(&account.currency.code)
                                .copied()
                                .unwrap_or_default()
                        })
                        .sum();
                    (claimed > account.closing_balance).then(|| {
                        format!(
                            "Pots on {} add up to {}, more than its {} balance.",
                            account.name,
                            money(claimed, &account.currency),
                            money(account.closing_balance, &account.currency)
                        )
                    })
                })
                .collect::<Vec<_>>()
        });
        view! {
            {over.into_iter().map(|text| view! { <p class=WARN role="note">{text}</p> }).collect_view()}
            {if active.is_empty() {
                view! { <EmptyState class="p-4" title="No pots yet." /> }.into_any()
            } else {
                view! { <ul class=LIST>{active.into_iter().map(row(loaded)).collect_view()}</ul> }.into_any()
            }}
        }
    };
    let archived = move || {
        let (loaded, pots) = pots_now();
        let archived: Vec<PotView> = pots.into_iter().filter(|p| p.archived).collect();
        let count = archived.len();
        (count > 0).then(|| view! {
            <details class="border-t border-line pt-3 text-sm">
                <summary class="cursor-pointer font-medium">{format!("Archived pots ({count})")}</summary>
                <ul class=LIST>{archived.into_iter().map(row(loaded)).collect_view()}</ul>
            </details>
        })
    };
    view! {
        <StepCard
            title="Pots"
            description=Some("Money set aside inside your accounts. Pots fill and empty through the month's split.".to_owned())
        >
            {move || failed.get().then(|| view! { <p class=FOOTNOTE role="status">"Recent pot entries are unavailable right now."</p> })}
            {body}
            <div class=INLINE_FORM role="group" aria-label="New pot">
                <FieldBox label="New pot".into_any() id=new_name_id.clone() class=GROW>
                    <TextBox
                        id=new_name_id
                        value=new_name
                        on_input=Callback::new(move |v| new_name.set(v))
                        maxlength=128
                        placeholder="e.g. Holiday"
                    />
                </FieldBox>
                <FieldBox label="Money sits on".into_any() id=new_account_id.clone() class=GROW>
                    <Choice
                        id=new_account_id
                        value=new_account
                        options=Signal::stored(location_options)
                        on_change=Callback::new(move |v| new_account.set(v))
                    />
                </FieldBox>
                <Button
                    size=Size::Sm
                    disabled=Signal::derive(move || disabled.get() || new_name.with(|n| n.trim().is_empty()))
                    on_click=Callback::new(move |_| {
                        let place = new_account.get_untracked();
                        wizard.save_pot(
                            PotDraft {
                                pot_id: None,
                                name: new_name.get_untracked().trim().to_owned(),
                                account_id: (!place.is_empty()).then_some(place),
                                archived: false,
                            },
                            None,
                        );
                    })
                >
                    "Create pot"
                </Button>
            </div>
            {archived}
        </StepCard>
    }
}
