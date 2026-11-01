//! The review step, new-account banners and the allocation checklist
//! (personal-site `components/month-closure/steps.tsx`).

use std::collections::{BTreeMap, HashMap};

use finance_types::report::closure::{
    AccountReview, AccountRoleKind, AllocationLine, BalanceStatus, ClosureReview, ClosureView,
    TransferStatus, UnclassifiedAccounts as Unclassified,
};
use leptos::prelude::*;
use rust_decimal::Decimal;

use super::Wizard;
use super::transfer::{Hop, LegStatus, legs_of};
use super::ui::{ACTIONS, CALLOUT, Choice, FOOTNOTE, LIST, SMALL, TextLink, WARN, money, options};
use crate::ds::{
    Button, Checkbox, Size, TABLE, TABLE_CONTAINER, TCELL, TH, TROW, Variant, card_class, cn,
};
use crate::pages::plural;
use crate::{ManageSection, Tab, state};

/// One banner per newly detected account, plus transfers that look like an
/// unknown savings account.
#[component]
pub fn UnclassifiedAccounts(data: Unclassified, goals: Vec<(String, String)>) -> impl IntoView {
    let wizard = expect_context::<Wizard>();
    let app = state();
    let disabled = Signal::derive(move || wizard.busy.get());
    if data.accounts.is_empty() && data.suspected.is_empty() {
        return ().into_any();
    }
    let goal_for = RwSignal::new(BTreeMap::<String, String>::new());
    // Account whose "Don't track" is waiting for confirmation.
    let confirming = RwSignal::new(None::<String>);
    let goal_options = {
        let mut list = vec![(String::new(), "No goal".to_owned(), false)];
        list.extend(options(goals));
        list
    };
    let accounts = data
        .accounts
        .into_iter()
        .map(|account| {
            let id = StoredValue::new(account.account_id.clone());
            let name = account.name.clone();
            let goal_options = goal_options.clone();
            let pick = Signal::derive(move || {
                goal_for.with(|g| g.get(&id.get_value()).cloned().unwrap_or_default())
            });
            let confirm_name = name.clone();
            view! {
                <div class=CALLOUT>
                    <p><strong>{format!("New account: {name}")}</strong>" · how should closing treat it?"</p>
                    <div class=ACTIONS>
                        <div class="w-full sm:w-48">
                            <Choice
                                label=format!("Savings goal for {name}")
                                value=pick
                                options=Signal::stored(goal_options)
                                on_change=Callback::new(move |v: String| goal_for.update(|g| {
                                    g.insert(id.get_value(), v);
                                }))
                            />
                        </div>
                        <Button size=Size::Sm disabled=disabled on_click=Callback::new(move |_| {
                            let goal = pick.get_untracked();
                            wizard.set_role(id.get_value(), "savings", (!goal.is_empty()).then_some(goal), None, None);
                        })>"Savings"</Button>
                        <Button size=Size::Sm variant=Variant::Secondary disabled=disabled on_click=Callback::new(move |_| {
                            wizard.set_role(id.get_value(), "spending", None, None, None);
                        })>"Spending"</Button>
                        <Button size=Size::Sm variant=Variant::Ghost disabled=disabled on_click=Callback::new(move |_| {
                            confirming.set(Some(id.get_value()));
                        })>"Don't track"</Button>
                    </div>
                    {move || (confirming.get().as_deref() == Some(id.get_value().as_str())).then(|| view! {
                        <div class=ACTIONS role="alertdialog" aria-label=format!("Don't track {confirm_name}?")>
                            <small class=cn([SMALL, "basis-full"])>"This account will be left out of month closing."</small>
                            <Button size=Size::Sm disabled=disabled on_click=Callback::new(move |_| {
                                wizard.set_role(id.get_value(), "ignored", None, None, Some(confirming));
                            })>"Confirm"</Button>
                            <Button size=Size::Sm variant=Variant::Ghost on_click=Callback::new(move |_| confirming.set(None))>"Cancel"</Button>
                        </div>
                    })}
                </div>
            }
        })
        .collect_view();
    let suspected = (!data.suspected.is_empty()).then(|| {
        let count = data.suspected.len();
        let ids = data.suspected.join(", ");
        view! {
            <div class=CALLOUT>
                <p>
                    {format!(
                        "{} went to an account the app does not know. Looks like a savings account — re-authorize Revolut in ",
                        plural(count, "outgoing transfer"),
                    )}
                    <TextLink on_click=Callback::new(move |_| app.go_manage(ManageSection::Banks))>"Banks"</TextLink>
                    "."
                </p>
                <small class=SMALL>{format!("Transactions: {ids}")}</small>
            </div>
        }
    });
    view! { <div class="grid gap-3">{accounts}{suspected}</div> }.into_any()
}

/// Display names for budget groups and planned payments the review only
/// knows by id.
#[derive(Clone, Debug, Default)]
pub struct ReviewNames {
    pub groups: HashMap<String, String>,
    pub payments: HashMap<String, String>,
}

/// The site's `TH` merged with the row-header overrides (tailwind-merge).
const ROW_LABEL: &str =
    "px-0 py-1.5 text-left text-xs font-normal sm:text-sm normal-case tracking-normal text-fg";
const LEFTOVER_LABEL: &str = "pb-2 px-0 pt-2.5 text-left text-sm font-semibold sm:text-base normal-case tracking-normal text-fg";

fn row(
    label: String,
    planned: Option<Decimal>,
    actual: Decimal,
    account: &AccountReview,
) -> impl IntoView + use<> {
    let unit = &account.currency;
    view! {
        <tr>
            <th scope="row" class=ROW_LABEL>{label}</th>
            <td class=cn([TCELL, "text-right tabular-nums px-1 py-1.5 sm:px-2"])>
                {planned.map(|p| money(p, unit)).unwrap_or_default()}
            </td>
            <td class=cn([TCELL, "text-right tabular-nums py-1.5 pl-1 pr-0 sm:pl-2"])>{money(actual, unit)}</td>
        </tr>
    }
}

/// A row outside the leftover sum, in footnote type.
fn note_row(label: String, actual: Decimal, account: &AccountReview) -> impl IntoView + use<> {
    view! {
        <tr>
            <th scope="row" class=cn([ROW_LABEL, FOOTNOTE])>{label}</th>
            <td class=TCELL></td>
            <td class=cn([TCELL, FOOTNOTE, "text-right tabular-nums py-1.5 pl-1 pr-0 sm:pl-2"])>
                {money(actual, &account.currency)}
            </td>
        </tr>
    }
}

/// One card per non-savings account: planned against actual, ending in the
/// leftover to split.
#[component]
pub fn ReviewStep(
    review: ClosureReview,
    names: ReviewNames,
    balances: Vec<BalanceStatus>,
) -> impl IntoView {
    let app = state();
    let accounts: Vec<AccountReview> = review
        .accounts
        .into_iter()
        .filter(|a| a.role != AccountRoleKind::Savings)
        .collect();
    let group_names: Vec<String> = review
        .unassigned_groups
        .iter()
        .map(|id| names.groups.get(id).cloned().unwrap_or_else(|| id.clone()))
        .collect();
    let payment_names: Vec<String> = review
        .unassigned_payments
        .iter()
        .map(|id| {
            names
                .payments
                .get(id)
                .cloned()
                .unwrap_or_else(|| id.clone())
        })
        .collect();
    let groups = review.unassigned_groups.len();
    let payments = review.unassigned_payments.len();
    let unassigned = (groups > 0 || payments > 0).then(|| {
        view! {
            <ul class="grid gap-3 md:col-span-2" role="list">
                {(groups > 0).then(|| view! {
                    <li class=CALLOUT>
                        <p>
                            {format!(
                                "{groups} budget group{} no account, so {} left out of the review. ",
                                if groups == 1 { " has" } else { "s have" },
                                if groups == 1 { "it is" } else { "they are" },
                            )}
                            <TextLink on_click=Callback::new(move |_| app.go(Tab::Overview))>"Assign in the budget"</TextLink>
                        </p>
                        <small class=SMALL>{group_names.join(", ")}</small>
                    </li>
                })}
                {(payments > 0).then(|| view! {
                    <li class=CALLOUT>
                        <p>
                            {format!(
                                "{payments} planned payment{} no account. ",
                                if payments == 1 { " has" } else { "s have" },
                            )}
                            <TextLink on_click=Callback::new(move |_| app.go(Tab::Plan))>"Assign in Plans"</TextLink>
                        </p>
                        <small class=SMALL>{payment_names.join(", ")}</small>
                    </li>
                })}
            </ul>
        }
    });
    let empty = accounts.is_empty();
    let cards = accounts
        .into_iter()
        .map(|account| {
            let unit = account.currency.clone();
            let title = format!(
                "{}{}",
                account.name,
                if account.role == AccountRoleKind::Unclassified {
                    " · not classified"
                } else {
                    ""
                }
            );
            let skipped = account.skipped_foreign;
            let adjusted = balances
                .iter()
                .find(|b| b.account_id == account.account_id)
                .and_then(|b| b.adjusted_by);
            let groups = account
                .groups
                .iter()
                .map(|group| {
                    let elsewhere = (group.spent_elsewhere > Decimal::ZERO).then(|| {
                        view! {
                            <tr>
                                <td colspan="3" class=cn([TCELL, "text-fg-muted pt-0 pr-0 text-right text-xs"])>
                                    {format!("{} spent from other accounts", money(group.spent_elsewhere, &unit))}
                                </td>
                            </tr>
                        }
                    });
                    view! {
                        {row(group.name.clone(), Some(group.planned), group.spent, &account)}
                        {elsewhere}
                    }
                })
                .collect_view();
            view! {
                <article class=cn([card_class(Default::default()), "grid min-w-0 gap-3 p-3 sm:p-5"])>
                    <h3 class=cn(["text-lg font-semibold leading-tight", "text-base"])>{title}</h3>
                    {(skipped > 0).then(|| view! {
                        <p class=WARN role="note">
                            {format!(
                                "{skipped} transaction{} in another currency {} left out.",
                                if skipped == 1 { "" } else { "s" },
                                if skipped == 1 { "was" } else { "were" },
                            )}
                        </p>
                    })}
                    <div class=TABLE_CONTAINER tabindex="0" role="region" aria-label=format!("{}: planned and actual", account.name)>
                        <table data-density="sm" class=cn([TABLE, "text-xs sm:text-sm"])>
                            <thead class="text-left">
                                <tr>
                                    <td></td>
                                    <th scope="col" class=cn([TH, "text-right tabular-nums px-1 sm:px-2"])>"Planned"</th>
                                    <th scope="col" class=cn([TH, "text-right tabular-nums pl-1 pr-0 sm:pl-2"])>"Actual"</th>
                                </tr>
                            </thead>
                            <tbody>
                                {row("Income".into(), Some(account.income_planned), account.income_actual, &account)}
                                {row("Fixed payments".into(), Some(account.fixed_planned), account.fixed_actual, &account)}
                                {groups}
                                {row("Other spending".into(), None, account.other_spent, &account)}
                                {row("Carry-in (buffer + planned purchases)".into(), None, account.carry_in, &account)}
                                {adjusted.map(|x| note_row("Balance adjustment (not counted)".into(), x, &account))}
                                <tr class=TROW>
                                    <th scope="row" class=LEFTOVER_LABEL>"Leftover"</th>
                                    <td class=TCELL></td>
                                    <td class=cn([TCELL, "text-right tabular-nums pl-1 pr-0 pt-2.5 text-sm font-semibold sm:pl-2 sm:text-base"])>
                                        {money(account.leftover, &unit)}
                                    </td>
                                </tr>
                            </tbody>
                        </table>
                    </div>
                </article>
            }
        })
        .collect_view();
    view! {
        <div class="grid gap-4 md:grid-cols-2">
            {unassigned}
            {cards}
            {empty.then(|| view! { <p class=cn([FOOTNOTE, "md:col-span-2"])>"No spending accounts to review."</p> })}
        </div>
    }
}

/// en-GB short month and year: `Sept 2026`.
fn due_label(day: &str) -> String {
    let long = crate::format::month_short(day);
    format!("{long} {}", day.get(..4).unwrap_or_default())
}

fn destination_label(line: &AllocationLine, view: &ClosureView) -> String {
    match line.destination.as_str() {
        "buffer" => "next-month buffer".into(),
        "stay" => "stays".into(),
        "plan" => {
            let due = line
                .plan_due_on
                .as_deref()
                .map(due_label)
                .unwrap_or_default();
            let text = format!(
                "planned purchase {}{}",
                line.plan_name.as_deref().unwrap_or_default(),
                if due.is_empty() {
                    String::new()
                } else {
                    format!(" ({due})")
                }
            );
            text.trim().to_owned()
        }
        dest => {
            if let Some(pot_id) = dest.strip_prefix("pot:") {
                let name = view
                    .pots
                    .iter()
                    .find(|p| p.pot_id == pot_id)
                    .map_or(pot_id, |p| p.name.as_str());
                return format!("Pot (retired): {name}");
            }
            if let Some(wish_id) = dest.strip_prefix("wish:") {
                return view
                    .wish_labels
                    .iter()
                    .find(|(id, _)| id == wish_id)
                    .map_or_else(
                        || format!("Wish (no longer planned): {wish_id}"),
                        |(_, label)| label.clone(),
                    );
            }
            view.goals
                .iter()
                .find(|(id, _)| dest.strip_prefix("goal:") == Some(id.as_str()))
                .map_or_else(|| dest.to_owned(), |(_, name)| format!("Goal: {name}"))
        }
    }
}

/// Account names from every source the view has.
pub fn account_names(view: &ClosureView) -> HashMap<String, String> {
    let mut names = HashMap::new();
    for a in &view.accounts {
        names.insert(a.account_id.clone(), a.name.clone());
    }
    if let Some(snapshot) = &view.period.settled_snapshot {
        for a in &snapshot.accounts {
            names.insert(a.account_id.clone(), a.name.clone());
        }
    }
    for a in &view.review.accounts {
        names.insert(a.account_id.clone(), a.name.clone());
    }
    names
}

/// Allocation lines as a checklist; lines needing a transfer show whether a
/// matching one synced. `actions` offers marking transfers done by hand.
#[component]
pub fn AllocationList(
    lines: Vec<AllocationLine>,
    view: ClosureView,
    #[prop(optional)] transfers_only: bool,
    #[prop(optional)] actions: bool,
) -> impl IntoView {
    let names = StoredValue::new(account_names(&view));
    let shown: Vec<AllocationLine> = if transfers_only {
        lines
            .into_iter()
            .filter(|l| l.transfer_status != TransferStatus::NotNeeded)
            .collect()
    } else {
        lines
    };
    if shown.is_empty() {
        let text = if transfers_only {
            "No transfers needed."
        } else {
            "No lines."
        };
        return view! { <p class=FOOTNOTE>{text}</p> }.into_any();
    }
    let account =
        move |id: &str| names.with_value(|n| n.get(id).cloned().unwrap_or_else(|| id.to_owned()));
    let items = shown
        .into_iter()
        .map(|line| {
            // A legacy take from a pot.
            let take = line.pot_id().is_some() && line.amount.is_sign_negative();
            let amount = money(
                if take { line.amount.abs() } else { line.amount },
                &line.currency,
            );
            let label = destination_label(&line, &view);
            let transfer = line.transfer_status != TransferStatus::NotNeeded;
            // A take moved money from the pot's account to this one;
            // anything else from this account onwards.
            let from = account(if take {
                line.transfer_source_account_id
                    .as_deref()
                    .unwrap_or(&line.from_account_id)
            } else {
                &line.from_account_id
            });
            let to = match &line.to_account_id {
                Some(to) => format!(
                    "{}{}",
                    account(to),
                    if take { String::new() } else { format!(" ({label})") }
                ),
                None => label.clone(),
            };
            let text = if transfer {
                format!(
                    "Move {amount} {from} → {to}{}",
                    if take { format!(" (from {label})") } else { String::new() }
                )
            } else if take {
                format!("{amount} taken from {label} into {}", account(&line.from_account_id))
            } else {
                format!("{amount} from {from}: {label}")
            };
            let done = line.transfer_status == TransferStatus::Matched;
            let hops = !line.transfer_hops.is_empty();
            let legs = legs_of(&line);
            let line = StoredValue::new(line);
            let direct = (transfer && !hops).then(|| {
                let leg = legs[0].clone();
                view! { <LegStatus line=line.get_value() leg=leg hop=Hop::Direct names=names actions=actions /> }
            });
            let routed = (transfer && hops).then(|| {
                let unit = line.with_value(|l| l.currency.clone());
                let items = legs
                    .into_iter()
                    .enumerate()
                    .map(|(index, leg)| {
                        let leg_done = leg.transfer_status == TransferStatus::Matched;
                        let text = format!(
                            "{}. {} {} → {}",
                            index + 1,
                            money(leg.amount, &unit),
                            account(&leg.from),
                            leg.to.as_deref().map_or_else(|| "?".to_owned(), account)
                        );
                        view! {
                            <li class="flex flex-wrap items-center gap-x-3 gap-y-1 py-1">
                                <Checkbox
                                    checked=Signal::stored(leg_done)
                                    on_change=Callback::new(|_| {})
                                    disabled=true
                                    label=if leg_done { "Hop done" } else { "Hop not found yet" }
                                />
                                <span class=cn(["min-w-0 flex-1 basis-48", if leg_done { "text-fg-muted line-through" } else { "" }])>{text}</span>
                                <LegStatus line=line.get_value() leg=leg hop=Hop::Index(index as u32) names=names actions=actions />
                            </li>
                        }
                    })
                    .collect_view();
                view! { <ol class="grid basis-full gap-1 pl-7">{items}</ol> }
            });
            view! {
                <li class="flex flex-wrap items-center gap-x-3 gap-y-1 py-2.5 text-sm">
                    {transfer.then(|| view! {
                        <Checkbox
                            checked=Signal::stored(done)
                            on_change=Callback::new(|_| {})
                            disabled=true
                            label=if done { "Transfer found" } else { "Transfer not found yet" }
                        />
                    })}
                    <span class=cn(["min-w-0 flex-1 basis-48", if done { "text-fg-muted line-through" } else { "" }])>{text}</span>
                    {direct}
                    {routed}
                </li>
            }
        })
        .collect_view();
    view! { <ul class=LIST>{items}</ul> }.into_any()
}
